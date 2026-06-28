use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::palaces::qian_permission::{PathOp, PermissionMatrix};
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::intent::{CeremoniesIntent, ReadAction};

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
    permissions: Arc<PermissionMatrix>,
}

impl LspTool {
    pub fn new(permissions: Arc<PermissionMatrix>) -> Self {
        Self {
            manager: Arc::new(LspManager::new()),
            permissions,
        }
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
        CeremoniesIntent::Wu(ReadAction {
            target: String::new(),
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        false
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

    async fn execute(&self, input: Value) -> Result<String, String> {
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
        let path = self.permissions.verify_path(&file, PathOp::Read)?;
        let lang = LanguageKind::from_path(&path)
            .ok_or_else(|| format!("no language server for file: {}", path.display()))?;

        let manager = self.manager.clone();
        // LSP JSON-RPC is blocking IO — run off the async runtime.
        tokio::task::spawn_blocking(move || {
            manager.run_operation(&path, lang, &operation, line, character)
        })
        .await
        .map_err(|e| format!("LSP task join error: {e}"))?
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
}

pub struct LspManager {
    servers: Mutex<HashMap<LanguageKind, LspServerHandle>>,
}

impl Default for LspManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LspManager {
    pub fn new() -> Self {
        Self {
            servers: Mutex::new(HashMap::new()),
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
    })
}

fn initialize(handle: &mut LspServerHandle, root: &Path) -> Result<(), String> {
    let root_uri = path_to_uri(root.parent().unwrap_or(root));
    let _init: Value = request(
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
                    "callHierarchy": { "dynamicRegistration": false }
                }
            }
        }),
    )?;
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

fn read_message(handle: &mut LspServerHandle) -> Result<Value, String> {
    let mut content_length: Option<usize> = None;
    loop {
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
    handle
        .stdout
        .read_exact(&mut buf)
        .map_err(|e| format!("read body: {e}"))?;
    serde_json::from_slice(&buf).map_err(|e| format!("parse body: {e}"))
}

fn request(handle: &mut LspServerHandle, method: &str, params: Value) -> Result<Value, String> {
    let id = handle.next_id;
    handle.next_id += 1;
    let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    write_message(handle, &msg)?;
    // Read until we get the response matching `id` (skip notifications/server requests).
    loop {
        let resp = read_message(handle)?;
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

    #[tokio::test]
    async fn lsp_missing_params() {
        let tool = LspTool::new(Arc::new(PermissionMatrix::default()));
        let result = tool.execute(json!({})).await;
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

        let tool = LspTool::new(Arc::new(PermissionMatrix::default()));
        let result = tool
            .execute(json!({
                "operation": "document_symbol",
                "file": file.to_string_lossy(),
                "line": 0,
                "character": 0
            }))
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
        let tool = LspTool::new(Arc::new(PermissionMatrix::default()));
        let result = tool
            .execute(json!({
                "operation": "hover",
                "file": "src/palaces/zhen_tool/builtin/grep.rs",
                "line": 0,
                "character": 0
            }))
            .await;
        if LanguageKind::Rust.server_command().is_some() {
            assert!(result.is_ok(), "hover failed: {:?}", result.err());
        } else {
            assert!(result.is_err(), "expected error when server missing");
            assert!(result.unwrap_err().contains("no language server"));
        }
    }
}
