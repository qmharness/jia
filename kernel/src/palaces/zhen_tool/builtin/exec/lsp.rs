use crate::error::ToolError;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::palaces::qian_permission::PathOp;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::CeremoniesIntent;
use crate::stems::action::ExecContext;

/// 震三宫 · LSP — semantic code navigation (go-to-def / references / hover /
/// document symbols / call hierarchy). Read-only (戊仪 Wu ceremony), routes to
/// 震三 (Zhen) palace. GeJu evaluates as Direct.
///
/// Spawns a long-lived language server per language (rust-analyzer, clangd, …)
/// and speaks JSON-RPC 2.0 over stdio. The manager is process-global and
/// serialized by a Mutex; LSP operations are not concurrency-safe (they share
/// server state via didOpen).
pub struct LspTool {
    manager: Arc<LspManager>,
}

impl Default for LspTool {
    fn default() -> Self {
        Self::new()
    }
}

impl LspTool {
    pub fn new() -> Self {
        Self::with_manager(Arc::new(LspManager::new()))
    }

    /// N6: share one LspManager with the fs edit tools (post-edit
    /// diagnostics) so a single server pool serves both — 工具单例不动,
    /// worktree 重建(仅换 ExecContext)时 LSP 不重启。
    pub fn with_manager(manager: Arc<LspManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl BaseTool for LspTool {
    fn name(&self) -> &str {
        "lsp"
    }

    fn description(&self) -> String {
        "Semantic code navigation via the Language Server Protocol. \
         Operations: definition, references, hover, document_symbol, \
         incoming_calls, outgoing_calls. Positions are 0-based (line, character). \
         Returns file:line:col locations or symbol info. Requires a language \
         server installed (rust-analyzer for Rust, clangd for C/C++)."
            .to_string()
    }

    fn category(&self) -> &str {
        "file"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Wu
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    fn accesses(&self, _input: &Value) -> crate::palaces::zhen_tool::base::ToolAccesses {
        // U1 appendix red line: this tool holds session-level mutable state
        // (LspManager — long-lived language servers, didOpen mutations), so
        // it must NOT declare parallelizable access. Keep the All barrier
        // even though operations are read-only (Wu).
        crate::palaces::zhen_tool::base::ToolAccesses::all()
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["definition", "references", "hover", "document_symbol", "incoming_calls", "outgoing_calls"],
                    "description": "Navigation operation"
                },
                "file": {"type": "string", "description": "File path (relative to project root or absolute)"},
                "line": {"type": "integer", "description": "0-based line number"},
                "character": {"type": "integer", "description": "0-based character offset"}
            },
            "required": ["operation", "file", "line", "character"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        let operation = input["operation"]
            .as_str()
            .ok_or("Missing 'operation' parameter")?
            .to_string();
        let file = input["file"]
            .as_str()
            .ok_or("Missing 'file' parameter")?
            .to_string();
        let line = input["line"].as_u64().ok_or("Missing 'line' parameter")? as u32;
        let character = input["character"]
            .as_u64()
            .ok_or("Missing 'character' parameter")? as u32;

        // Sandbox the file path
        let path = ctx.permissions.verify_path(&file, PathOp::Read)?;
        let lang = LanguageKind::from_path(&path)
            .ok_or_else(|| format!("no language server for file: {}", path.display()))?;

        let manager = self.manager.clone();
        // LSP JSON-RPC is blocking IO — run off the async runtime.
        let inner = tokio::task::spawn_blocking(move || {
            manager.run_operation(&path, lang, &operation, line, character)
        })
        .await
        .map_err(|e| ToolError::exec(self.name(), format!("LSP task join error: {e}")))?;
        Ok(inner?)
    }
}

// ── Language detection ─────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum LanguageKind {
    Rust,
    Cpp,
}

impl LanguageKind {
    fn from_path(p: &Path) -> Option<Self> {
        match p.extension().and_then(|e| e.to_str())? {
            "rs" => Some(Self::Rust),
            "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "hh" => Some(Self::Cpp),
            _ => None,
        }
    }

    fn language_id(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Cpp => "cpp",
        }
    }

    /// Server command to spawn for this language. Returns None if not installed.
    fn server_command(self) -> Option<Vec<&'static str>> {
        let (cmd, args): (&str, &[&str]) = match self {
            Self::Rust => ("rust-analyzer", &[]),
            Self::Cpp => ("clangd", &["--background-index=false"][..]),
        };
        if which_exists(cmd) {
            let mut v = vec![cmd];
            v.extend_from_slice(args);
            Some(v)
        } else {
            None
        }
    }
}

fn which_exists(cmd: &str) -> bool {
    // Require a successful (exit 0) --version: a broken rustup proxy exits
    // non-zero with "unknown binary" and must NOT be treated as installed.
    std::process::Command::new(cmd)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

// ── LSP manager ────────────────────────────────────────────

struct LspServerHandle {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
    initialized: bool,
    /// Server advertised `diagnosticProvider` (LSP 3.17 pull diagnostics).
    pull_diagnostics: bool,
}

pub struct LspManager {
    servers: Mutex<HashMap<LanguageKind, LspServerHandle>>,
    /// N6: upper bound on every diagnostics wait inside `fetch_diagnostics`
    /// (initialize handshake, pull response, publishDiagnostics loop). The
    /// blocking call self-terminates at this deadline, so no `spawn_blocking`
    /// thread outlives it holding the manager lock.
    diag_wait: Duration,
}

/// N6 · diagnostics wait budget (matches the caller-side wall-clock timeout).
const DIAGNOSTICS_WAIT_TIMEOUT: Duration = Duration::from_secs(3);

impl Default for LspManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LspManager {
    pub fn new() -> Self {
        Self {
            servers: Mutex::new(HashMap::new()),
            diag_wait: DIAGNOSTICS_WAIT_TIMEOUT,
        }
    }

    fn run_operation(
        &self,
        path: &Path,
        lang: LanguageKind,
        operation: &str,
        line: u32,
        character: u32,
    ) -> Result<String, String> {
        let uri = path_to_uri(path);
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;

        let pos = json!({ "line": line, "character": character });
        let text_document = json!({ "uri": uri });

        // Acquire (or spawn) the server for this language, then ensure init +
        // didOpen. The lock is held for the whole op — LSP ops are sequential.
        let mut servers = self
            .servers
            .lock()
            .map_err(|e| format!("lock error: {e}"))?;
        if let std::collections::hash_map::Entry::Vacant(e) = servers.entry(lang) {
            let handle = spawn_server(lang)?;
            e.insert(handle);
        }
        let handle = servers.get_mut(&lang).expect("just inserted");
        if !handle.initialized {
            initialize(handle, path)?;
            handle.initialized = true;
        }
        did_open(handle, &uri, lang.language_id(), &text)?;

        let result: Value = match operation {
            "definition" => request(
                handle,
                "textDocument/definition",
                json!({ "textDocument": text_document, "position": pos }),
            )?,
            "references" => request(
                handle,
                "textDocument/references",
                json!({
                    "textDocument": text_document,
                    "position": pos,
                    "context": { "includeDeclaration": true }
                }),
            )?,
            "hover" => request(
                handle,
                "textDocument/hover",
                json!({ "textDocument": text_document, "position": pos }),
            )?,
            "document_symbol" => request(
                handle,
                "textDocument/documentSymbol",
                json!({ "textDocument": text_document }),
            )?,
            "incoming_calls" | "outgoing_calls" => {
                let items = request(
                    handle,
                    "textDocument/prepareCallHierarchy",
                    json!({ "textDocument": text_document, "position": pos }),
                )?;
                let item = items
                    .get(0)
                    .ok_or("no call hierarchy item at position")?
                    .clone();
                let method = if operation == "incoming_calls" {
                    "callHierarchy/incomingCalls"
                } else {
                    "callHierarchy/outgoingCalls"
                };
                request(handle, method, json!({ "item": item }))?
            }
            other => return Err(format!("unknown operation: {other}")),
        };

        Ok(format_result(&result))
    }

    /// N6 · fetch diagnostics for one file. Pull (`textDocument/diagnostic`,
    /// LSP 3.17) when the server advertised `diagnosticProvider`; otherwise
    /// fall back to waiting for a `publishDiagnostics` notification (bounded
    /// by a message budget). Every blocking read below carries a shared
    /// deadline (`self.diag_wait`), so a silent server can never park this
    /// call (and the manager lock it holds) forever — the blocking thread
    /// self-terminates instead of relying on the caller's wall-clock timeout.
    /// Blocking; run inside `spawn_blocking`.
    fn fetch_diagnostics(&self, path: &Path, lang: LanguageKind) -> Result<Vec<Value>, String> {
        let deadline = Instant::now() + self.diag_wait;
        let uri = path_to_uri(path);
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;

        let mut servers = self
            .servers
            .lock()
            .map_err(|e| format!("lock error: {e}"))?;
        if let std::collections::hash_map::Entry::Vacant(e) = servers.entry(lang) {
            let handle = spawn_server(lang)?;
            e.insert(handle);
        }
        let handle = servers.get_mut(&lang).expect("just inserted");
        if !handle.initialized {
            initialize_with_deadline(handle, path, Some(deadline))?;
            handle.initialized = true;
        }
        did_open(handle, &uri, lang.language_id(), &text)?;

        if handle.pull_diagnostics {
            let result = request_with_deadline(
                handle,
                "textDocument/diagnostic",
                json!({ "textDocument": { "uri": uri } }),
                Some(deadline),
            )?;
            // FullDocumentDiagnosticReport: { "kind": "full", "items": [...] }
            return Ok(result
                .get("items")
                .and_then(|i| i.as_array())
                .cloned()
                .unwrap_or_default());
        }

        // publishDiagnostics fallback: read messages until the notification
        // for our URI arrives. Bounded by a message budget AND the shared
        // read deadline — a quiet server returns a timeout error instead of
        // spinning or blocking forever.
        for _ in 0..200 {
            let msg = read_message(handle, Some(deadline))?;
            if msg.get("method").and_then(|m| m.as_str())
                == Some("textDocument/publishDiagnostics")
            {
                let params = msg.get("params").cloned().unwrap_or(Value::Null);
                if params.get("uri").and_then(|u| u.as_str()) == Some(uri.as_str()) {
                    return Ok(params
                        .get("diagnostics")
                        .and_then(|d| d.as_array())
                        .cloned()
                        .unwrap_or_default());
                }
            }
        }
        Err("no publishDiagnostics received".into())
    }
}

// ── N6 · 编辑后主动诊断 (post-edit diagnostics) ─────────────

/// fs 编辑工具(patch_file/write_file)写入成功后经此接口拉取诊断摘要。
/// 同步阻塞接口;调用方(见 `append_post_edit_diagnostics`)负责
/// `spawn_blocking` + 超时。诊断是读——lsp 保持戊仪只读。
pub trait EditDiagnostics: Send + Sync {
    /// `None`: 无 error/warning、语言未接入 LSP、或拉取失败(静默降级)。
    /// `Some`: 已格式化的摘要,直接附加到工具结果尾部。
    fn post_edit_summary(&self, path: &Path) -> Option<String>;
}

impl EditDiagnostics for LspManager {
    fn post_edit_summary(&self, path: &Path) -> Option<String> {
        let lang = LanguageKind::from_path(path)?;
        match self.fetch_diagnostics(path, lang) {
            Ok(items) => format_diagnostics_summary(path, &items),
            Err(e) => {
                tracing::debug!(path = %path.display(), error = %e, "post-edit diagnostics failed");
                None
            }
        }
    }
}

/// Count errors/warnings and format the summary appended to edit results.
/// Returns None when there are no errors or warnings (无诊断不附加).
fn format_diagnostics_summary(path: &Path, items: &[Value]) -> Option<String> {
    let mut errors = 0usize;
    let mut warnings = 0usize;
    let mut samples: Vec<String> = Vec::new();
    for d in items {
        // LSP DiagnosticSeverity: 1=Error 2=Warning 3=Info 4=Hint
        let severity = d.get("severity").and_then(|s| s.as_i64()).unwrap_or(1);
        if severity != 1 && severity != 2 {
            continue;
        }
        if severity == 1 {
            errors += 1;
        } else {
            warnings += 1;
        }
        if samples.len() < 3 {
            let line = d
                .pointer("/range/start/line")
                .and_then(|l| l.as_i64())
                .unwrap_or(0)
                + 1;
            let msg = d
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("?")
                .lines()
                .next()
                .unwrap_or("?");
            samples.push(format!("{line}: {msg}"));
        }
    }
    if errors == 0 && warnings == 0 {
        return None;
    }
    let mut parts = Vec::new();
    if errors > 0 {
        parts.push(format!("{} error{}", errors, if errors > 1 { "s" } else { "" }));
    }
    if warnings > 0 {
        parts.push(format!(
            "{} warning{}",
            warnings,
            if warnings > 1 { "s" } else { "" }
        ));
    }
    Some(format!(
        "\n[LSP 诊断: {} — {}: {}]",
        parts.join(", "),
        path.display(),
        samples.join("; ")
    ))
}

/// N6 · 编辑成功后调用:拉取诊断并附加摘要。3s 超时;超时/失败/未覆盖
/// 一律静默降级(仅 debug log),不阻塞主流程。
///
/// 诊断等待自身有界:LspManager 内每次阻塞读都带 deadline(见
/// `DIAGNOSTICS_WAIT_TIMEOUT`),server 永不发声时阻塞调用在预算内自行
/// 返回,不遗留持锁的 spawn_blocking 线程;此处 3s wall-clock 超时只是
/// 冗余保险(对非 LspManager 的 EditDiagnostics 实现仍有效)。
pub async fn append_post_edit_diagnostics(
    result: &mut String,
    diagnostics: &Option<Arc<dyn EditDiagnostics>>,
    path: &Path,
) {
    let Some(d) = diagnostics else { return };
    let d = d.clone();
    let p = path.to_path_buf();
    let task = tokio::task::spawn_blocking(move || d.post_edit_summary(&p));
    match tokio::time::timeout(std::time::Duration::from_secs(3), task).await {
        Ok(Ok(Some(summary))) => result.push_str(&summary),
        Ok(Ok(None)) => {}
        Ok(Err(e)) => tracing::debug!("post-edit diagnostics join error: {e}"),
        Err(_) => tracing::debug!(path = %path.display(), "post-edit diagnostics timed out"),
    }
}

fn spawn_server(lang: LanguageKind) -> Result<LspServerHandle, String> {
    let cmd = lang
        .server_command()
        .ok_or_else(|| format!("no language server installed for {:?}", lang))?;
    let (program, args) = cmd.split_first().ok_or("empty server command")?;

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to spawn {program}: {e}"))?;

    let stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    Ok(LspServerHandle {
        child,
        stdin,
        stdout: BufReader::new(stdout),
        next_id: 1,
        initialized: false,
        pull_diagnostics: false,
    })
}

fn initialize(handle: &mut LspServerHandle, root: &Path) -> Result<(), String> {
    initialize_with_deadline(handle, root, None)
}

/// `deadline` bounds the blocking wait for the initialize response; `None`
/// keeps the historical unbounded behavior (navigation path).
fn initialize_with_deadline(
    handle: &mut LspServerHandle,
    root: &Path,
    deadline: Option<Instant>,
) -> Result<(), String> {
    let root_uri = path_to_uri(root.parent().unwrap_or(root));
    let init: Value = request_with_deadline(
        handle,
        "initialize",
        json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "definition": { "linkSupport": false },
                    "references": {},
                    "hover": { "contentFormat": ["markdown", "plaintext"] },
                    "documentSymbol": { "hierarchicalDocumentSymbolSupport": false },
                    "callHierarchy": { "dynamicRegistration": false },
                    "publishDiagnostics": {},
                    "diagnostic": { "dynamicRegistration": false }
                }
            }
        }),
        deadline,
    )?;
    // N6: record pull-diagnostics support (LSP 3.17 `diagnosticProvider`).
    handle.pull_diagnostics = init
        .pointer("/capabilities/diagnosticProvider")
        .is_some_and(|v| !v.is_null());
    // Send initialized notification (no response expected)
    notify(handle, "initialized", json!({}))?;
    Ok(())
}

fn did_open(
    handle: &mut LspServerHandle,
    uri: &str,
    language_id: &str,
    text: &str,
) -> Result<(), String> {
    notify(
        handle,
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "languageId": language_id,
                "version": 1,
                "text": text
            }
        }),
    )
}

// ── JSON-RPC framing ───────────────────────────────────────

fn write_message(handle: &mut LspServerHandle, msg: &Value) -> Result<(), String> {
    let body = serde_json::to_string(msg).map_err(|e| format!("serialize: {e}"))?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    handle
        .stdin
        .write_all(header.as_bytes())
        .and_then(|_| handle.stdin.write_all(body.as_bytes()))
        .and_then(|_| handle.stdin.flush())
        .map_err(|e| format!("write to server: {e}"))
}

/// Block until at least one byte can be read, or the deadline expires.
/// Bytes already sitting in the BufReader buffer count as readable (polling
/// the fd alone would miss them). Deadline is enforced at message granularity:
/// a server that dribbles a partial line/body can still block one read past
/// the deadline — accepted residual; the target scenario (silent server) is
/// fully bounded.
fn wait_readable(reader: &BufReader<ChildStdout>, deadline: Option<Instant>) -> Result<(), String> {
    if !reader.buffer().is_empty() {
        return Ok(());
    }
    let Some(deadline) = deadline else {
        return Ok(());
    };
    wait_fd_readable(reader, deadline)
}

#[cfg(unix)]
fn wait_fd_readable(reader: &BufReader<ChildStdout>, deadline: Instant) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    let fd = reader.get_ref().as_raw_fd();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("timed out waiting for language server output".into());
        }
        let ms = remaining.as_millis().clamp(1, i32::MAX as u128) as i32;
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pfd` is a valid out-pointer; fd stays open (owned by the
        // handle, which outlives this call).
        let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
        if rc > 0 {
            // Readable, or error/hang-up — let the following read report it.
            return Ok(());
        }
        if rc == 0 {
            return Err("timed out waiting for language server output".into());
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(format!("poll on server stdout: {err}"));
        }
    }
}

/// Non-unix fallback: no fd polling — reads stay unbounded (previous
/// behavior), the caller-side wall-clock timeout still applies.
#[cfg(not(unix))]
fn wait_fd_readable(_reader: &BufReader<ChildStdout>, _deadline: Instant) -> Result<(), String> {
    Ok(())
}

fn read_message(handle: &mut LspServerHandle, deadline: Option<Instant>) -> Result<Value, String> {
    let mut content_length: Option<usize> = None;
    loop {
        wait_readable(&handle.stdout, deadline)?;
        let mut line = String::new();
        let n = handle
            .stdout
            .read_line(&mut line)
            .map_err(|e| format!("read header: {e}"))?;
        if n == 0 {
            return Err("server closed connection".into());
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            content_length = rest.trim().parse().ok();
        }
    }
    let len = content_length.ok_or("missing Content-Length")?;
    let mut buf = vec![0u8; len];
    wait_readable(&handle.stdout, deadline)?;
    handle
        .stdout
        .read_exact(&mut buf)
        .map_err(|e| format!("read body: {e}"))?;
    serde_json::from_slice(&buf).map_err(|e| format!("parse body: {e}"))
}

fn request(handle: &mut LspServerHandle, method: &str, params: Value) -> Result<Value, String> {
    request_with_deadline(handle, method, params, None)
}

/// `deadline` bounds every blocking read while awaiting the response; `None`
/// keeps the historical unbounded behavior (navigation path).
fn request_with_deadline(
    handle: &mut LspServerHandle,
    method: &str,
    params: Value,
    deadline: Option<Instant>,
) -> Result<Value, String> {
    let id = handle.next_id;
    handle.next_id += 1;
    let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    write_message(handle, &msg)?;
    // Read until we get the response matching `id` (skip notifications/server requests).
    loop {
        let resp = read_message(handle, deadline)?;
        if resp.get("id") == Some(&Value::from(id)) {
            if let Some(err) = resp.get("error") {
                return Err(format!("LSP error on {method}: {err}"));
            }
            return Ok(resp.get("result").cloned().unwrap_or(Value::Null));
        }
        // Otherwise it's a notification or unrelated message — ignore.
    }
}

fn notify(handle: &mut LspServerHandle, method: &str, params: Value) -> Result<(), String> {
    let msg = json!({ "jsonrpc": "2.0", "method": method, "params": params });
    write_message(handle, &msg)
}

// ── Formatting ─────────────────────────────────────────────

fn format_result(result: &Value) -> String {
    // result may be Null, a single location, or an array of locations.
    if result.is_null() {
        return "No results.".to_string();
    }
    let arr = if result.is_array() {
        result.as_array().unwrap().clone()
    } else {
        vec![result.clone()]
    };
    if arr.is_empty() {
        return "No results.".to_string();
    }
    let lines: Vec<String> = arr.iter().map(format_location_or_symbol).collect();
    lines.join("\n")
}

fn format_location_or_symbol(v: &Value) -> String {
    // documentSymbol items have name/kind/range
    if let Some(name) = v.get("name").and_then(|n| n.as_str()) {
        let kind = v
            .get("kind")
            .and_then(|k| k.as_i64())
            .map(symbol_kind_name)
            .unwrap_or("symbol");
        let loc = format_range(v);
        return format!("{kind} {name}  {loc}");
    }
    // call hierarchy results: {from: {...}} / {to: {...}}
    if let Some(item) = v.get("from").or_else(|| v.get("to")) {
        let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("?");
        return format!("call {name}  {}", format_range(item));
    }
    format_range(v)
}

fn format_range(v: &Value) -> String {
    // A location/symbol may carry its range directly or under "location".
    let range_holder = v.get("location").unwrap_or(v);
    let uri = range_holder
        .get("uri")
        .and_then(|u| u.as_str())
        .map(uri_to_path)
        .unwrap_or_default();
    let (start_l, start_c) = range_holder
        .get("range")
        .and_then(|r| r.get("start"))
        .and_then(|s| Some((s.get("line")?.as_i64()?, s.get("character")?.as_i64()?)))
        .map(|(l, c)| (l + 1, c + 1))
        .unwrap_or((0, 0));
    if uri.is_empty() {
        String::new()
    } else {
        format!("{}:{}:{}", uri, start_l, start_c)
    }
}

fn symbol_kind_name(k: i64) -> &'static str {
    // LSP SymbolKind subset
    match k {
        1 => "module",
        2 => "class",
        3 => "method",
        4 => "property",
        5 => "field",
        6 => "constructor",
        9 => "enum",
        10 => "interface",
        11 => "function",
        12 => "variable",
        13 => "constant",
        24 => "struct",
        25 => "event",
        26 => "operator",
        _ => "symbol",
    }
}

fn path_to_uri(p: &Path) -> String {
    let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    format!("file://{}", abs.display())
}

fn uri_to_path(uri: &str) -> String {
    uri.strip_prefix("file://").unwrap_or(uri).to_string()
}

impl Drop for LspServerHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    fn test_ctx() -> crate::stems::action::ExecContext {
        use crate::palaces::qian_permission::PermissionMatrix;
        use std::sync::Arc;
        crate::stems::action::ExecContext::new(Arc::new(PermissionMatrix::default()))
    }

    use super::*;

    #[test]
    fn language_detection() {
        assert_eq!(
            LanguageKind::from_path(std::path::Path::new("a.rs")),
            Some(LanguageKind::Rust)
        );
        assert_eq!(
            LanguageKind::from_path(std::path::Path::new("b.cpp")),
            Some(LanguageKind::Cpp)
        );
        assert_eq!(LanguageKind::from_path(std::path::Path::new("c.txt")), None);
    }

    #[test]
    fn path_uri_roundtrip() {
        let uri = path_to_uri(std::path::Path::new("Cargo.toml"));
        assert!(uri.starts_with("file://"));
        assert!(uri_to_path(&uri).ends_with("Cargo.toml"));
    }

    #[test]
    fn format_null_result() {
        assert_eq!(format_result(&Value::Null), "No results.");
        assert_eq!(format_result(&json!([])), "No results.");
    }

    // ── N6 · 诊断摘要格式化 ─────────────────────────────────

    #[test]
    fn diagnostics_summary_counts_and_samples() {
        let items = json!([
            { "severity": 1, "range": { "start": { "line": 41, "character": 4 } }, "message": "missing semicolon\nmore detail" },
            { "severity": 1, "range": { "start": { "line": 9, "character": 0 } }, "message": "unused import" },
            { "severity": 2, "range": { "start": { "line": 2, "character": 0 } }, "message": "dead code" },
            { "severity": 3, "range": { "start": { "line": 0, "character": 0 } }, "message": "info ignored" }
        ]);
        let out = format_diagnostics_summary(
            std::path::Path::new("src/main.rs"),
            items.as_array().unwrap(),
        )
        .unwrap();
        assert!(out.contains("2 errors, 1 warning"), "got: {out}");
        assert!(out.contains("src/main.rs"), "got: {out}");
        assert!(out.contains("42: missing semicolon"), "1-based line + first line only, got: {out}");
        assert!(!out.contains("info ignored"), "info severity skipped, got: {out}");
    }

    #[test]
    fn diagnostics_summary_empty_or_no_errors_is_none() {
        assert_eq!(
            format_diagnostics_summary(std::path::Path::new("a.rs"), &[]),
            None
        );
        let infos = json!([{ "severity": 3, "range": { "start": { "line": 0 } }, "message": "hint" }]);
        assert_eq!(
            format_diagnostics_summary(std::path::Path::new("a.rs"), infos.as_array().unwrap()),
            None
        );
    }

    #[tokio::test]
    async fn append_diagnostics_none_handle_is_noop() {
        let mut s = "base".to_string();
        append_post_edit_diagnostics(&mut s, &None, std::path::Path::new("a.rs")).await;
        assert_eq!(s, "base");
    }

    #[tokio::test]
    async fn lsp_missing_params() {
        let tool = LspTool::new();
        let result = tool.execute(json!({}), &test_ctx()).await;
        assert!(result.is_err());
    }

    /// End-to-end smoke test against clangd on a temp .c file. Ignored by
    /// default (spawns a server, slow). Run: `cargo test --lib lsp -- --ignored`.
    #[tokio::test]
    #[ignore = "requires clangd LSP server installed"]
    async fn lsp_clangd_document_symbol() {
        if LanguageKind::Cpp.server_command().is_none() {
            eprintln!("skipping: clangd not installed");
            return;
        }
        let dir = tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        let file = dir.path().join("smoke.c");
        std::fs::write(
            &file,
            "struct Point { int x; int y; };\nint add(int a, int b) { return a + b; }\n",
        )
        .unwrap();

        let tool = LspTool::new();
        let result = tool
            .execute(
                json!({
                    "operation": "document_symbol",
                    "file": file.to_string_lossy(),
                    "line": 0,
                    "character": 0
                }),
                &test_ctx(),
            )
            .await;
        assert!(result.is_ok(), "document_symbol failed: {:?}", result.err());
        let out = result.unwrap();
        eprintln!("clangd document_symbol: {out}");
    }

    /// Tests that LSP gracefully errors when rust-analyzer is not installed.
    #[tokio::test]
    async fn lsp_rust_analyzer_skips_when_unavailable() {
        // If rust-analyzer isn't installed, the tool returns a clear error
        // rather than spawning a broken proxy.
        let tool = LspTool::new();
        let result = tool
            .execute(
                json!({
                    "operation": "hover",
                    "file": "src/palaces/zhen_tool/builtin/fs/grep.rs",
                    "line": 0,
                    "character": 0
                }),
                &test_ctx(),
            )
            .await;
        if LanguageKind::Rust.server_command().is_some() {
            assert!(result.is_ok(), "hover failed: {:?}", result.err());
        } else {
            assert!(result.is_err(), "expected error when server missing");
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("no language server")
            );
        }
    }

    // ── N6 · 诊断等待有界性(超时路径不泄漏阻塞线程)────────────

    /// Minimal mock language server (bash): speaks LSP framing, answers
    /// `textDocument/diagnostic` with a pull report and replies to
    /// `textDocument/didOpen` with a `publishDiagnostics` notification.
    const MOCK_LSP_SERVER: &str = r#"
len=0
while IFS= read -r line; do
  line="${line%$'\r'}"
  if [ -n "$line" ]; then
    case "$line" in
      Content-Length:*) len="${line#Content-Length: }";;
    esac
    continue
  fi
  body="$(dd bs=1 count="$len" 2>/dev/null)"
  case "$body" in
    *textDocument/didOpen*)
      uri="$(printf '%s' "$body" | sed -n 's/.*"uri":"\([^"]*\)".*/\1/p')"
      notif='{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":"'"$uri"'","diagnostics":[{"severity":1,"range":{"start":{"line":3,"character":1}},"message":"fallback err"}]}}'
      printf 'Content-Length: %d\r\n\r\n%s' "${#notif}" "$notif"
      ;;
    *textDocument/diagnostic*)
      id="$(printf '%s' "$body" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')"
      resp='{"jsonrpc":"2.0","id":'"${id:-1}"',"result":{"kind":"full","items":[{"severity":1,"range":{"start":{"line":0,"character":0}},"message":"mock error"}]}}'
      printf 'Content-Length: %d\r\n\r\n%s' "${#resp}" "$resp"
      ;;
  esac
  len=0
done
"#;

    fn handle_from_child(mut child: Child, pull_diagnostics: bool) -> LspServerHandle {
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        LspServerHandle {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            initialized: true, // skip the handshake; tests drive fetch directly
            pull_diagnostics,
        }
    }

    /// A "server" that reads stdin forever and never writes a byte back.
    fn spawn_silent_handle(pull_diagnostics: bool) -> LspServerHandle {
        let child = Command::new("bash")
            .arg("-c")
            .arg("while read -r l; do :; done")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        handle_from_child(child, pull_diagnostics)
    }

    fn spawn_mock_handle(pull_diagnostics: bool) -> LspServerHandle {
        let child = Command::new("bash")
            .arg("-c")
            .arg(MOCK_LSP_SERVER)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        handle_from_child(child, pull_diagnostics)
    }

    fn manager_with(handle: LspServerHandle, diag_wait: Duration) -> LspManager {
        LspManager {
            servers: Mutex::new(HashMap::from([(LanguageKind::Rust, handle)])),
            diag_wait,
        }
    }

    fn temp_rs(dir: &tempfile::TempDir) -> std::path::PathBuf {
        let f = dir.path().join("probe.rs");
        std::fs::write(&f, "fn main() {}\n").unwrap();
        f
    }

    #[test]
    fn silent_server_fallback_wait_is_bounded() {
        let dir = tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        let file = temp_rs(&dir);
        let manager = manager_with(spawn_silent_handle(false), Duration::from_millis(300));

        let start = Instant::now();
        let res = manager.fetch_diagnostics(&file, LanguageKind::Rust);
        let elapsed = start.elapsed();

        assert!(res.is_err(), "silent server must not yield diagnostics");
        assert!(
            elapsed < Duration::from_secs(2),
            "wait must self-terminate at the deadline, took {elapsed:?}"
        );
        // The call returned — the manager lock is not parked by a leaked wait.
        let start = Instant::now();
        let res = manager.fetch_diagnostics(&file, LanguageKind::Rust);
        assert!(res.is_err());
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "manager must stay usable after a timed-out wait"
        );
    }

    #[test]
    fn silent_server_pull_wait_is_bounded() {
        let dir = tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        let file = temp_rs(&dir);
        let manager = manager_with(spawn_silent_handle(true), Duration::from_millis(300));

        let start = Instant::now();
        let res = manager.fetch_diagnostics(&file, LanguageKind::Rust);
        let elapsed = start.elapsed();

        assert!(res.is_err(), "silent server must not yield diagnostics");
        assert!(
            elapsed < Duration::from_secs(2),
            "pull wait must self-terminate at the deadline, took {elapsed:?}"
        );
    }

    /// End-to-end: `append_post_edit_diagnostics` over a silent server returns
    /// promptly because the blocking task finishes on its own (inner bound
    /// 300ms << outer 3s timeout) — no abandoned `spawn_blocking` thread.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn append_diagnostics_silent_server_leaves_no_blocking_task() {
        let dir = tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        let file = temp_rs(&dir);
        let diagnostics: Option<Arc<dyn EditDiagnostics>> = Some(Arc::new(manager_with(
            spawn_silent_handle(false),
            Duration::from_millis(300),
        )));

        let mut s = "base".to_string();
        let start = Instant::now();
        append_post_edit_diagnostics(&mut s, &diagnostics, &file).await;
        let elapsed = start.elapsed();

        assert_eq!(s, "base", "silent server degrades to no summary");
        assert!(
            elapsed < Duration::from_secs(2),
            "returned via the self-bounded wait, not the 3s outer timeout: {elapsed:?}"
        );
    }

    /// Pull path regression: mock server answers `textDocument/diagnostic`.
    #[test]
    fn pull_diagnostics_mock_server_roundtrip() {
        let dir = tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        let file = temp_rs(&dir);
        let manager = manager_with(spawn_mock_handle(true), Duration::from_secs(5));

        let items = manager
            .fetch_diagnostics(&file, LanguageKind::Rust)
            .expect("mock server must answer the pull request");
        assert_eq!(items.len(), 1, "got: {items:?}");
        assert_eq!(items[0]["message"], "mock error");
    }

    /// Fallback path regression: mock server publishes diagnostics on didOpen.
    #[test]
    fn publish_diagnostics_fallback_mock_server_roundtrip() {
        let dir = tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        let file = temp_rs(&dir);
        let manager = manager_with(spawn_mock_handle(false), Duration::from_secs(5));

        let items = manager
            .fetch_diagnostics(&file, LanguageKind::Rust)
            .expect("mock server must publish diagnostics");
        assert_eq!(items.len(), 1, "got: {items:?}");
        assert_eq!(items[0]["message"], "fallback err");
    }
}
