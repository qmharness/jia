use std::sync::Arc;
// ── Socket Connection ─────────────────────────────────────
//
// Unix socket connection to the Jia daemon via ~/.jia/rin.sock.
// Reads JSON-line protocol, handling both StreamEvent-tagged
// messages and bare cron_notification events.
//
// Uses tokio::io::split to separate read and write halves.
// Write half is Arc<Mutex<>> for concurrent access from
// send() and spawned tasks. Read half is owned by the
// reader task exclusively.

use std::io;
use std::path::Path;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::{Mutex, mpsc};

/// Events received from the daemon via the socket.
#[derive(Debug, Clone)]
pub enum SocketEvent {
    /// A standard StreamEvent (delta, tool_call, tool_result, etc.)
    Stream(StreamEvent),
    /// A cron job notification (bare JSON, not wrapped in StreamEvent)
    CronNotification {
        job_name: String,
        prompt: String,
        response: String,
        timestamp: i64,
    },
    /// Sessions list response
    SessionsList(Vec<Value>),
    /// Session history loaded
    SessionHistory {
        session_id: String,
        entries: Vec<Value>,
    },
    /// Confirmation resolved
    ConfirmResolved { id: String, resolved: bool },
    /// Answer resolved
    AnswerResolved { id: String, resolved: bool },
    /// Daemon model info (provider + model name).
    ModelInfo { provider: String, model: String },
    /// Project resolution result from "hello".
    WorkspaceResolved {
        workspace_id: String,
        approved: bool,
    },
}

/// Parsed StreamEvent — mirrors `kernel::types::StreamEvent` but implements
/// manual parsing from JSON Value since the original type is Serialize-only.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    Delta {
        content: String,
    },
    Session {
        session_id: String,
    },
    ToolCall {
        tool: String,
        input: Value,
    },
    ToolResult {
        tool: String,
        output: String,
        error: Option<String>,
        geju: Option<String>,
        execution_mode: Option<String>,
    },
    ConfirmationRequest {
        id: String,
        tool: String,
        reason: String,
        timeout_secs: u64,
        token: String,
    },
    UserQuestion {
        id: String,
        question: String,
        timeout_secs: u64,
        token: String,
        options: Option<Vec<String>>,
    },
    Done,
    StreamEnd,
    ToolBatchStart,
    Error {
        message: String,
    },
    /// P3 · interaction mode changed (谋划态 toggle).
    InteractionModeChanged {
        planning: bool,
    },
    ContextPressure,
    Compacting,
    /// S2: LLM stream failed mid-flight and is being retried — the TUI rolls
    /// the partial assistant bubble back to the stream-start anchor.
    Retrying {
        attempt: u32,
    },
}

impl StreamEvent {
    fn from_value(value: &Value) -> Option<Self> {
        let msg_type = value["type"].as_str()?;
        match msg_type {
            "delta" => Some(StreamEvent::Delta {
                content: value["content"].as_str()?.to_string(),
            }),
            "session" => Some(StreamEvent::Session {
                session_id: value["session_id"].as_str()?.to_string(),
            }),
            "tool_call" => Some(StreamEvent::ToolCall {
                tool: value["tool"].as_str()?.to_string(),
                input: value["input"].clone(),
            }),
            "tool_result" => Some(StreamEvent::ToolResult {
                tool: value["tool"].as_str()?.to_string(),
                output: value["output"].as_str().unwrap_or("").to_string(),
                error: value["error"].as_str().map(String::from),
                geju: value["geju"].as_str().map(String::from),
                execution_mode: value["execution_mode"].as_str().map(String::from),
            }),
            "confirm_request" => Some(StreamEvent::ConfirmationRequest {
                id: value["id"].as_str()?.to_string(),
                tool: value["tool"].as_str()?.to_string(),
                reason: value["reason"].as_str()?.to_string(),
                timeout_secs: value["timeout_secs"].as_u64()?,
                token: value["token"].as_str()?.to_string(),
            }),
            "user_question" => Some(StreamEvent::UserQuestion {
                id: value["id"].as_str()?.to_string(),
                question: value["question"].as_str()?.to_string(),
                timeout_secs: value["timeout_secs"].as_u64()?,
                token: value["token"].as_str()?.to_string(),
                options: value["options"].as_array().map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                }),
            }),
            "done" => Some(StreamEvent::Done),
            "stream_end" => Some(StreamEvent::StreamEnd),
            "tool_batch_start" => Some(StreamEvent::ToolBatchStart),
            "error" => Some(StreamEvent::Error {
                message: value["message"]
                    .as_str()
                    .unwrap_or("unknown error")
                    .to_string(),
            }),
            "interaction_mode_changed" => Some(StreamEvent::InteractionModeChanged {
                planning: value["planning"].as_bool().unwrap_or(false),
            }),
            "context_pressure" => Some(StreamEvent::ContextPressure),
            "compacting" => Some(StreamEvent::Compacting),
            // Unknown/missing attempt is tolerated (default 0) — the TUI only
            // needs the signal, not the count.
            "retrying" => Some(StreamEvent::Retrying {
                attempt: value["attempt"].as_u64().unwrap_or(0) as u32,
            }),
            _ => None,
        }
    }
}

/// Client messages sent to the daemon.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type")]
pub enum ClientMsg {
    #[serde(rename = "hello")]
    Hello { cwd: String },
    #[serde(rename = "agent")]
    Agent {
        messages: Vec<kernel::types::Message>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        /// Current working directory for project detection.
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        /// Project ID from .jia/config.toml (if exists).
        #[serde(skip_serializing_if = "Option::is_none")]
        workspace_id: Option<String>,
    },
    #[serde(rename = "cancel")]
    Cancel { session_id: String },
    #[serde(rename = "confirm")]
    Confirm {
        id: String,
        token: String,
        approved: bool,
    },
    #[serde(rename = "answer")]
    Answer {
        id: String,
        token: String,
        answer: String,
    },
    #[serde(rename = "sessions")]
    Sessions,
    #[serde(rename = "load_session")]
    #[allow(dead_code)]
    LoadSession { session_id: String },
    /// P3 · user-triggered plan-mode toggle (/plan, /plan-end).
    #[serde(rename = "set_mode")]
    SetInteractionMode {
        session_id: Option<String>,
        planning: bool,
    },
}

/// A connection to the Jia daemon.
///
/// Write half is shared via `Arc<Mutex<OwnedWriteHalf>>` so multiple
/// tasks can write concurrently (send + spawned response senders).
#[derive(Clone)]
pub struct Connection {
    writer: Arc<Mutex<OwnedWriteHalf>>,
}

impl Connection {
    /// Connect to the rin socket and spawn the reader.
    ///
    /// Returns (connection, receiver) — the receiver gets all incoming
    /// `SocketEvent`s from the daemon.
    pub async fn connect(
        sock_path: &Path,
    ) -> io::Result<(Self, mpsc::UnboundedReceiver<SocketEvent>)> {
        let stream = UnixStream::connect(sock_path).await?;
        let (reader, writer) = stream.into_split();

        let (tx, rx) = mpsc::unbounded_channel::<SocketEvent>();

        // Spawn reader task — uses BufReader directly on the read half
        tokio::spawn(async move {
            let mut buf_reader = BufReader::new(reader);
            let mut line = String::new();

            loop {
                line.clear();
                match buf_reader.read_line(&mut line).await {
                    Ok(0) => break, // EOF
                    Ok(_) => {}
                    Err(e) => {
                        tracing::debug!("TUI socket read error: {e}");
                        break;
                    }
                }

                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let value: Value = match serde_json::from_str(trimmed) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let msg_type = value["type"].as_str().unwrap_or("");

                let event = match msg_type {
                    "cron_notification" => SocketEvent::CronNotification {
                        job_name: value["job_name"].as_str().unwrap_or("").to_string(),
                        prompt: value["prompt"].as_str().unwrap_or("").to_string(),
                        response: value["response"].as_str().unwrap_or("").to_string(),
                        timestamp: value["timestamp"].as_i64().unwrap_or(0),
                    },
                    "sessions" => {
                        let sessions = value["sessions"].as_array().cloned().unwrap_or_default();
                        SocketEvent::SessionsList(sessions)
                    }
                    "session_history" => SocketEvent::SessionHistory {
                        session_id: value["session_id"].as_str().unwrap_or("").to_string(),
                        entries: value["entries"].as_array().cloned().unwrap_or_default(),
                    },
                    "confirm_resolved" => SocketEvent::ConfirmResolved {
                        id: value["id"].as_str().unwrap_or("").to_string(),
                        resolved: value["resolved"].as_bool().unwrap_or(false),
                    },
                    "answer_resolved" => SocketEvent::AnswerResolved {
                        id: value["id"].as_str().unwrap_or("").to_string(),
                        resolved: value["resolved"].as_bool().unwrap_or(false),
                    },
                    "model_info" => SocketEvent::ModelInfo {
                        provider: value["provider"].as_str().unwrap_or("").to_string(),
                        model: value["model"].as_str().unwrap_or("").to_string(),
                    },
                    "workspace_resolved" => SocketEvent::WorkspaceResolved {
                        workspace_id: value["workspace_id"].as_str().unwrap_or("").to_string(),
                        approved: value["approved"].as_bool().unwrap_or(false),
                    },
                    // All StreamEvent types
                    _ => match StreamEvent::from_value(&value) {
                        Some(se) => SocketEvent::Stream(se),
                        None => {
                            tracing::debug!("TUI: unknown message type: {msg_type}");
                            continue;
                        }
                    },
                };

                if tx.send(event).is_err() {
                    break; // Receiver dropped
                }
            }
        });

        let conn = Self {
            writer: Arc::new(Mutex::new(writer)),
        };

        Ok((conn, rx))
    }

    /// Access the underlying writer (for raw writes outside ClientMsg).
    pub fn writer(&self) -> &Arc<Mutex<OwnedWriteHalf>> {
        &self.writer
    }

    /// Send a message to the daemon.
    pub async fn send(&self, msg: &ClientMsg) -> io::Result<()> {
        let json = serde_json::to_string(msg).unwrap_or_default();
        let mut writer = self.writer.lock().await;
        writer.write_all(json.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        Ok(())
    }

    /// Request session history from the daemon (Ctrl+R in TUI).
    pub async fn load_session(&self, session_id: &str) -> io::Result<()> {
        self.send(&ClientMsg::LoadSession {
            session_id: session_id.to_string(),
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_interaction_mode_changed() {
        let v: Value = serde_json::json!({
            "type": "interaction_mode_changed",
            "planning": true
        });
        match StreamEvent::from_value(&v) {
            Some(StreamEvent::InteractionModeChanged { planning }) => assert!(planning),
            other => panic!("expected InteractionModeChanged, got {other:?}"),
        }

        let v2: Value = serde_json::json!({
            "type": "interaction_mode_changed",
            "planning": false
        });
        match StreamEvent::from_value(&v2) {
            Some(StreamEvent::InteractionModeChanged { planning }) => assert!(!planning),
            other => panic!("expected InteractionModeChanged, got {other:?}"),
        }
    }

    #[test]
    fn parse_user_question_with_options() {
        let v: Value = serde_json::json!({
            "type": "user_question",
            "id": "q1",
            "question": "What to eat?",
            "timeout_secs": 120,
            "token": "tok1",
            "options": ["Pizza", "Sushi", "Salad"]
        });
        match StreamEvent::from_value(&v) {
            Some(StreamEvent::UserQuestion {
                id,
                question,
                timeout_secs,
                token,
                options,
            }) => {
                assert_eq!(id, "q1");
                assert_eq!(question, "What to eat?");
                assert_eq!(timeout_secs, 120);
                assert_eq!(token, "tok1");
                assert_eq!(
                    options,
                    Some(vec!["Pizza".into(), "Sushi".into(), "Salad".into()])
                );
            }
            other => panic!("expected UserQuestion with options, got {other:?}"),
        }
    }

    #[test]
    fn parse_user_question_without_options() {
        let v: Value = serde_json::json!({
            "type": "user_question",
            "id": "q2",
            "question": "How are you?",
            "timeout_secs": 60,
            "token": "tok2"
        });
        match StreamEvent::from_value(&v) {
            Some(StreamEvent::UserQuestion {
                id,
                question,
                timeout_secs,
                token,
                options,
            }) => {
                assert_eq!(id, "q2");
                assert_eq!(question, "How are you?");
                assert_eq!(timeout_secs, 60);
                assert_eq!(token, "tok2");
                assert!(options.is_none(), "options should be None when not present");
            }
            other => panic!("expected UserQuestion without options, got {other:?}"),
        }
    }

    #[test]
    fn parse_user_question_empty_options() {
        let v: Value = serde_json::json!({
            "type": "user_question",
            "id": "q3",
            "question": "Pick one:",
            "timeout_secs": 30,
            "token": "tok3",
            "options": []
        });
        match StreamEvent::from_value(&v) {
            Some(StreamEvent::UserQuestion { options, .. }) => {
                assert_eq!(options, Some(vec![]));
            }
            other => panic!("expected UserQuestion with empty options, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod s2_tests {
    use super::*;

    /// S2: daemon 的 {"type":"retrying","attempt":N} 必须解析为 Retrying。
    #[test]
    fn parse_retrying() {
        let v: Value = serde_json::json!({
            "type": "retrying",
            "attempt": 2
        });
        match StreamEvent::from_value(&v) {
            Some(StreamEvent::Retrying { attempt }) => assert_eq!(attempt, 2),
            other => panic!("expected Retrying, got {other:?}"),
        }
    }

    /// S2 兼容:缺少 attempt 字段时容忍(默认 0),TUI 只需要截断信号。
    #[test]
    fn parse_retrying_missing_attempt_tolerated() {
        let v: Value = serde_json::json!({ "type": "retrying" });
        match StreamEvent::from_value(&v) {
            Some(StreamEvent::Retrying { attempt }) => assert_eq!(attempt, 0),
            other => panic!("expected Retrying, got {other:?}"),
        }
    }
}
