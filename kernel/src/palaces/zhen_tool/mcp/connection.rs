use std::sync::Arc;
// ── MCP stdio Connection ──────────────────────────────────────

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc, oneshot};

use super::protocol::*;
use crate::palaces::kun_config::McpServerConfig;

struct PendingRequest {
    tx: oneshot::Sender<Result<Value, String>>,
}

/// In-flight JSON-RPC requests awaiting a response, shared with the reader task.
type PendingMap = Arc<Mutex<HashMap<u64, PendingRequest>>>;

/// A managed stdio connection to one MCP server process.
pub struct McpConnection {
    name: String,
    next_id: AtomicU64,
    pending: PendingMap,
    send_tx: mpsc::UnboundedSender<String>,
    child: Mutex<Option<Child>>,
    server_info: ServerInfo,
    /// Per-request timeout (from `McpServerConfig::timeout_secs`).
    timeout: Duration,
    /// Set by the reader task when the server closes the connection; further
    /// requests fail fast instead of queueing behind a dead pipe.
    closed: Arc<AtomicBool>,
}

/// Wrap a command for OS-level sandbox execution (block network).
fn isolate_command(command: &str, args: &[String]) -> Result<(String, Vec<String>), String> {
    #[cfg(target_os = "macos")]
    {
        let mut full_args = vec![
            "-n".to_string(),
            "no-network".to_string(),
            command.to_string(),
        ];
        full_args.extend(args.iter().cloned());
        Ok(("sandbox-exec".to_string(), full_args))
    }
    #[cfg(target_os = "linux")]
    {
        let mut full_args = vec!["-n".to_string(), command.to_string()];
        full_args.extend(args.iter().cloned());
        Ok(("unshare".to_string(), full_args))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err("isolated MCP servers are not supported on this platform. Set isolated=false in config.".into())
    }
}

impl McpConnection {
    /// Spawn an MCP server subprocess, perform initialize handshake.
    pub async fn connect(config: &McpServerConfig) -> Result<Self, String> {
        let (cmd_binary, cmd_args) = if config.isolated {
            isolate_command(&config.command, &config.args)?
        } else {
            (config.command.clone(), config.args.clone())
        };

        let mut cmd = Command::new(&cmd_binary);
        cmd.args(&cmd_args);
        cmd.kill_on_drop(true);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::inherit());

        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        tracing::warn!(
            "MCP spawn{}: {} {} (env: {:?})",
            if config.isolated { " [isolated]" } else { "" },
            cmd_binary,
            cmd_args.join(" "),
            config.env.keys().collect::<Vec<_>>(),
        );
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn '{}': {e}", config.command))?;
        let stdin = child.stdin.take().ok_or("no stdin pipe")?;
        let stdout = child.stdout.take().ok_or("no stdout pipe")?;

        let timeout = Duration::from_secs(config.timeout_secs);
        let (send_tx, mut send_rx) = mpsc::unbounded_channel::<String>();
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(AtomicBool::new(false));

        // ── writer task ──────────────────────────────────
        let mut stdin_writer = tokio::io::BufWriter::new(stdin);
        tokio::spawn(async move {
            while let Some(line) = send_rx.recv().await {
                if stdin_writer.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if stdin_writer.write_all(b"\n").await.is_err() {
                    break;
                }
                if stdin_writer.flush().await.is_err() {
                    break;
                }
            }
        });

        // ── reader task ──────────────────────────────────
        let reader = BufReader::new(stdout);
        tokio::spawn(run_reader(
            reader.lines(),
            pending.clone(),
            closed.clone(),
        ));

        // ── Initialize handshake (bounded by the same timeout) ──
        let next_id = AtomicU64::new(1);
        let init_params = serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": CLIENT_NAME,
                "version": CLIENT_VERSION,
            }
        });
        let init_resp = match rpc_request(
            &next_id,
            &pending,
            &send_tx,
            METHOD_INITIALIZE,
            Some(init_params),
            timeout,
        )
        .await
        {
            Ok(resp) => resp,
            Err(e) => {
                let _ = child.start_kill();
                fail_all_pending(&pending, "MCP connection closed during initialize").await;
                return Err(format!(
                    "MCP server '{}' initialize failed: {e}",
                    config.name
                ));
            }
        };

        let init_result: InitializeResult = serde_json::from_value(init_resp)
            .map_err(|e| format!("Bad initialize response: {e}"))?;

        rpc_notification(&send_tx, METHOD_INITIALIZED, None).await;

        Ok(Self {
            name: config.name.clone(),
            next_id,
            pending,
            send_tx,
            child: Mutex::new(Some(child)),
            server_info: init_result.server_info,
            timeout,
            closed,
        })
    }

    pub async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value, String> {
        if self.closed.load(Ordering::SeqCst) {
            return Err("MCP connection is closed".to_string());
        }
        rpc_request(
            &self.next_id,
            &self.pending,
            &self.send_tx,
            method,
            params,
            self.timeout,
        )
        .await
    }

    pub async fn send_notification(&self, method: &str, params: Option<Value>) {
        rpc_notification(&self.send_tx, method, params).await
    }

    pub async fn list_tools(&self) -> Result<Vec<McpToolDef>, String> {
        let result = self.send_request(METHOD_TOOLS_LIST, None).await?;
        let list: ToolsListResult =
            serde_json::from_value(result).map_err(|e| format!("Bad tools/list response: {e}"))?;
        Ok(list.tools)
    }

    pub async fn call_tool(&self, name: &str, arguments: Option<Value>) -> Result<String, String> {
        let params = serde_json::to_value(ToolsCallParams {
            name: name.into(),
            arguments,
        })
        .map_err(|e| format!("Serialize error: {e}"))?;
        let result = self
            .send_request(METHOD_TOOLS_CALL, Some(params))
            .await
            .map_err(|e| format!("MCP server '{}' tool '{}': {e}", self.name, name))?;
        let call_result: ToolsCallResult =
            serde_json::from_value(result).map_err(|e| format!("Bad tools/call response: {e}"))?;

        let texts: Vec<String> = call_result
            .content
            .iter()
            .filter(|b| b.content_type == CONTENT_TYPE_TEXT)
            .map(|b| b.text.clone())
            .collect();
        Ok(texts.join("\n"))
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn server_info(&self) -> &ServerInfo {
        &self.server_info
    }
}

impl Drop for McpConnection {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.try_lock().ok().and_then(|mut g| g.take()) {
            let _ = child.start_kill();
        }
    }
}

// ── Free functions (shared between connect handshake and McpConnection) ──

/// Reader loop: dispatch responses to pending requests. On EOF or read error
/// the server is gone — mark the connection closed and fail every in-flight
/// request so no caller hangs forever (TOOL-C3).
async fn run_reader<R: AsyncBufRead + Unpin>(
    mut lines: Lines<R>,
    pending: PendingMap,
    closed: Arc<AtomicBool>,
) {
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let resp: JsonRpcResponse = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(_) => continue,
        };
        match resp {
            JsonRpcResponse::Ok { id, result, .. } => {
                let mut guard = pending.lock().await;
                if let Some(pr) = guard.remove(&id) {
                    let _ = pr.tx.send(Ok(result));
                }
            }
            JsonRpcResponse::Err { id, error, .. } => {
                let mut guard = pending.lock().await;
                if let Some(pr) = guard.remove(&id) {
                    let _ = pr.tx.send(Err(error.message));
                }
            }
            JsonRpcResponse::Notification { .. } => {}
        }
    }
    closed.store(true, Ordering::SeqCst);
    fail_all_pending(&pending, "MCP connection closed by server").await;
}

/// Complete every outstanding request with `reason` and empty the map.
async fn fail_all_pending(pending: &PendingMap, reason: &str) {
    let mut guard = pending.lock().await;
    for (_, pr) in guard.drain() {
        let _ = pr.tx.send(Err(reason.to_string()));
    }
}

async fn rpc_request(
    next_id: &AtomicU64,
    pending: &PendingMap,
    send_tx: &mpsc::UnboundedSender<String>,
    method: &str,
    params: Option<Value>,
    timeout: Duration,
) -> Result<Value, String> {
    let id = next_id.fetch_add(1, Ordering::SeqCst);
    let req = serde_json::to_string(&JsonRpcRequest {
        jsonrpc: JSONRPC_VERSION.into(),
        id,
        method: method.into(),
        params,
    })
    .map_err(|e| format!("Serialize error: {e}"))?;

    let (tx, rx) = oneshot::channel();
    pending.lock().await.insert(id, PendingRequest { tx });
    if let Err(e) = send_tx.send(req) {
        pending.lock().await.remove(&id);
        return Err(format!("Connection closed: {e}"));
    }
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(res)) => res,
        Ok(Err(_)) => Err("MCP request cancelled: connection dropped".to_string()),
        Err(_) => {
            // Timed out — drop the pending entry so a late response is ignored
            // and the map cannot grow unboundedly (TOOL-C2).
            pending.lock().await.remove(&id);
            Err(format!(
                "MCP request '{method}' timed out after {}s",
                timeout.as_secs()
            ))
        }
    }
}

async fn rpc_notification(
    send_tx: &mpsc::UnboundedSender<String>,
    method: &str,
    params: Option<Value>,
) {
    let notif = serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "method": method,
        "params": params.unwrap_or(Value::Null),
    });
    if let Ok(s) = serde_json::to_string(&notif) {
        let _ = send_tx.send(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_pending() -> PendingMap {
        Arc::new(Mutex::new(HashMap::new()))
    }

    /// TOOL-C2: a request whose server never answers must return a timeout
    /// error (bounded wait), and its pending entry must be cleaned up.
    #[tokio::test]
    async fn request_timeout_returns_error_and_cleans_pending() {
        let next_id = AtomicU64::new(1);
        let pending = new_pending();
        let (send_tx, mut send_rx) = mpsc::unbounded_channel::<String>();
        // Drain outbound frames so the writer side stays open but never answers.
        let drainer = tokio::spawn(async move { while send_rx.recv().await.is_some() {} });

        let err = rpc_request(
            &next_id,
            &pending,
            &send_tx,
            METHOD_TOOLS_CALL,
            None,
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();

        assert!(err.contains("timed out"), "err: {err}");
        assert!(err.contains(METHOD_TOOLS_CALL), "err names the method: {err}");
        assert!(
            pending.lock().await.is_empty(),
            "pending entry must be removed on timeout"
        );
        drop(send_tx);
        drainer.await.unwrap();
    }

    /// TOOL-C3: when the reader task sees EOF (server exited), in-flight
    /// requests must immediately fail with a connection-closed error.
    #[tokio::test]
    async fn reader_exit_fails_pending_with_connection_closed() {
        let (client, server) = tokio::io::duplex(1024);
        let pending = new_pending();
        let closed = Arc::new(AtomicBool::new(false));
        let reader = tokio::spawn(run_reader(
            BufReader::new(client).lines(),
            pending.clone(),
            closed.clone(),
        ));

        // A call is in flight, awaiting its response.
        let (tx, rx) = oneshot::channel();
        pending.lock().await.insert(1, PendingRequest { tx });

        // Server dies without answering.
        drop(server);

        reader.await.unwrap();
        let err = rx.await.unwrap().unwrap_err();
        assert!(err.contains("connection closed"), "err: {err}");
        assert!(closed.load(Ordering::SeqCst), "closed flag set");
        assert!(pending.lock().await.is_empty(), "pending drained");
    }

    /// Reader dispatches a response line to the matching pending request.
    #[tokio::test]
    async fn reader_dispatches_response_to_pending() {
        let (client, mut server) = tokio::io::duplex(1024);
        let pending = new_pending();
        let closed = Arc::new(AtomicBool::new(false));
        let reader = tokio::spawn(run_reader(
            BufReader::new(client).lines(),
            pending.clone(),
            closed,
        ));

        let (tx, rx) = oneshot::channel();
        pending.lock().await.insert(7, PendingRequest { tx });
        server
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n")
            .await
            .unwrap();

        let res = tokio::time::timeout(Duration::from_secs(1), rx)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(res["ok"], Value::Bool(true));

        drop(server);
        reader.await.unwrap();
    }

    /// Handshake timeout: the initialize request uses the same bounded
    /// rpc_request, so a silent server fails connect instead of hanging.
    #[tokio::test]
    async fn initialize_handshake_times_out_when_server_silent() {
        let next_id = AtomicU64::new(1);
        let pending = new_pending();
        let (send_tx, mut send_rx) = mpsc::unbounded_channel::<String>();
        let drainer = tokio::spawn(async move { while send_rx.recv().await.is_some() {} });

        let err = rpc_request(
            &next_id,
            &pending,
            &send_tx,
            METHOD_INITIALIZE,
            None,
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();

        assert!(err.contains("timed out"), "err: {err}");
        assert!(err.contains(METHOD_INITIALIZE), "err names the method: {err}");
        drop(send_tx);
        drainer.await.unwrap();
    }
}
