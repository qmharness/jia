use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::types::Message;
use crate::types::Role;

pub mod bots;

/// Input from any channel, carrying messages and source metadata.
#[derive(Debug, Clone)]
pub struct ChannelInput {
    pub messages: Vec<Message>,
    pub source: ChannelSource,
    /// Optional reply channel — when set, the IO consumer sends the
    /// Agent's response text back through this sender.
    pub reply_tx: Option<mpsc::UnboundedSender<OutboundReply>>,
}

/// Agent response routed back to a bot/platform adapter.
#[derive(Debug, Clone)]
pub struct OutboundReply {
    pub text: String,
}

/// Identifies which channel the input came from.
#[derive(Debug, Clone)]
pub enum ChannelSource {
    Stdin,
    FileWatch { path: String },
    Webhook { endpoint: String },
    Api,
}

/// 坎一宫 — I/O Channel Manager
///
/// Provides a shared `mpsc` pipe for channel inputs (stdin, file watch, webhook)
/// to feed into the agent loop. The receiver end is consumed by whichever component
/// runs the agent (gateway SSE handler, CLI REPL, etc.).
pub struct ChannelManager {
    tx: mpsc::UnboundedSender<ChannelInput>,
}

impl ChannelManager {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<ChannelInput>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { tx }, rx)
    }

    /// Clone the sender half for sharing across channels.
    pub fn sender(&self) -> mpsc::UnboundedSender<ChannelInput> {
        self.tx.clone()
    }

    /// Push input directly into the channel (used by webhook handler, REPL, etc.).
    pub fn push(&self, input: ChannelInput) {
        let _ = self.tx.send(input);
    }

    /// Spawn a stdin channel: reads lines asynchronously, sends each as a user message.
    ///
    /// Returns the JoinHandle. The task exits when stdin is closed (EOF / Ctrl-D).
    pub fn spawn_stdin(&self) -> tokio::task::JoinHandle<()> {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let reader = tokio::io::BufReader::new(tokio::io::stdin());
            let mut lines = reader.lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        if line.trim().is_empty() {
                            continue;
                        }
                        let _ = tx.send(ChannelInput {
                            messages: vec![Message::text(Role::User, line)],
                            source: ChannelSource::Stdin,
                            reply_tx: None,
                        });
                    }
                    Ok(None) => {
                        tracing::info!("ChannelManager: stdin closed");
                        break;
                    }
                    Err(e) => {
                        tracing::warn!("ChannelManager: stdin error: {e}");
                        break;
                    }
                }
            }
        })
    }

    /// Spawn a file watch channel using the `notify` crate.
    ///
    /// Watches the given paths for modifications. On each file change event,
    /// sends a brief message describing what changed. Debounces rapid events.
    pub fn spawn_file_watch(&self, paths: Vec<PathBuf>) -> tokio::task::JoinHandle<()> {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            use notify::{Config, Event, EventKind, RecursiveMode, Watcher};
            use std::time::Duration;

            let (event_tx, mut event_rx) = mpsc::unbounded_channel::<notify::Result<Event>>();

            let mut watcher = match notify::recommended_watcher(move |res| {
                let _ = event_tx.send(res);
            }) {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!("FileWatch: failed to create watcher: {e}");
                    return;
                }
            };

            if let Err(e) =
                watcher.configure(Config::default().with_poll_interval(Duration::from_secs(2)))
            {
                tracing::warn!("FileWatch: configure error: {e}");
            }

            for path in &paths {
                let mode = RecursiveMode::NonRecursive;
                if let Err(e) = watcher.watch(path, mode) {
                    tracing::warn!("FileWatch: failed to watch {}: {e}", path.display());
                    return;
                }
            }

            tracing::info!("FileWatch: watching {} path(s)", paths.len());

            // Debounce: collect events over a window, then emit one message
            let mut pending: Vec<String> = Vec::new();
            let debounce = Duration::from_millis(500);

            loop {
                match tokio::time::timeout(debounce, event_rx.recv()).await {
                    Ok(Some(Ok(event))) => {
                        for p in &event.paths {
                            let kind = match event.kind {
                                EventKind::Create(_) => "created",
                                EventKind::Modify(_) => "modified",
                                EventKind::Remove(_) => "removed",
                                _ => "changed",
                            };
                            pending.push(format!("{} ({})", p.display(), kind));
                        }
                        // If more events arrive quickly, batch them
                        continue;
                    }
                    _ => {
                        // Timeout or error — flush pending
                        if !pending.is_empty() {
                            pending.sort();
                            pending.dedup();
                            let msg = format!(
                                "File watch: {}. You may want to review the changes.",
                                pending.join(", "),
                            );
                            let _ = tx.send(ChannelInput {
                                messages: vec![Message::text(Role::User, msg)],
                                source: ChannelSource::FileWatch {
                                    path: paths
                                        .first()
                                        .map(|p| p.display().to_string())
                                        .unwrap_or_default(),
                                },
                                reply_tx: None,
                            });
                            pending.clear();
                        }
                    }
                }
            }
        })
    }
}

impl Default for ChannelManager {
    fn default() -> Self {
        let (cm, _rx) = Self::new();
        // Drop rx — channels that need it should use `new()` and keep their own rx
        drop(_rx);
        cm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_manager_new_creates_sender_receiver() {
        let (cm, mut rx) = ChannelManager::new();
        assert!(rx.try_recv().is_err()); // empty

        cm.push(ChannelInput {
            messages: vec![Message::text(Role::User, "hello")],
            source: ChannelSource::Api,
            reply_tx: None,
        });

        let input = rx.try_recv().unwrap();
        assert_eq!(input.messages.len(), 1);
        assert_eq!(input.messages[0].content, "hello");
        assert!(matches!(input.source, ChannelSource::Api));
    }

    #[test]
    fn channel_manager_sender_cloneable() {
        let (cm, mut rx) = ChannelManager::new();
        let tx2 = cm.sender();
        let _ = tx2.send(ChannelInput {
            messages: vec![Message::text(Role::User, "from clone")],
            source: ChannelSource::Api,
            reply_tx: None,
        });
        assert!(rx.try_recv().is_ok());
    }
}
