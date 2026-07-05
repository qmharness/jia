use std::sync::Arc;
// ── MCP stdio Connection ──────────────────────────────────────

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc, oneshot};

use super::protocol::*;
use crate::palaces::kun_config::McpServerConfig;

struct PendingRequest {
    tx: oneshot::Sender<Result<Value, String>>,
}

/// A managed stdio connection to one MCP server process.
pub struct McpConnection {
    name: String,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, PendingRequest>>>,
    send_tx: mpsc::UnboundedSender<String>,
    child: Mutex<Option<Child>>,
    server_info: ServerInfo,
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

        let (send_tx, mut send_rx) = mpsc::unbounded_channel::<String>();
        let pending: Arc<Mutex<HashMap<u64, PendingRequest>>> =
            Arc::new(Mutex::new(HashMap::new()));

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
        let reader_pending = pending.clone();
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        tokio::spawn(async move {
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
                        let mut guard = reader_pending.lock().await;
                        if let Some(pr) = guard.remove(&id) {
                            let _ = pr.tx.send(Ok(result));
                        }
                    }
                    JsonRpcResponse::Err { id, error, .. } => {
                        let mut guard = reader_pending.lock().await;
                        if let Some(pr) = guard.remove(&id) {
                            let _ = pr.tx.send(Err(error.message));
                        }
                    }
                    JsonRpcResponse::Notification { .. } => {}
                }
            }
        });

        // ── Initialize handshake ─────────────────────────
        let next_id = AtomicU64::new(1);
        let init_params = serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": CLIENT_NAME,
                "version": CLIENT_VERSION,
            }
        });
        let init_resp = rpc_request(
            &next_id,
            &pending,
            &send_tx,
            METHOD_INITIALIZE,
            Some(init_params),
        )
        .await?;

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
        })
    }

    pub async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value, String> {
        rpc_request(&self.next_id, &self.pending, &self.send_tx, method, params).await
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
        let result = self.send_request(METHOD_TOOLS_CALL, Some(params)).await?;
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

async fn rpc_request(
    next_id: &AtomicU64,
    pending: &Arc<Mutex<HashMap<u64, PendingRequest>>>,
    send_tx: &mpsc::UnboundedSender<String>,
    method: &str,
    params: Option<Value>,
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
    send_tx
        .send(req)
        .map_err(|e| format!("Connection closed: {e}"))?;
    tokio::time::timeout(std::time::Duration::from_secs(60), rx)
        .await
        .map_err(|_| "MCP request timed out after 60s".to_string())?
        .map_err(|_| "Request cancelled".to_string())?
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
