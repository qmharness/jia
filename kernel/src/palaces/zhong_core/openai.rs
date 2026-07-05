// ── OpenAI-compatible Provider ──────────────────────────────

use std::pin::Pin;

use futures::Stream;
use reqwest::Client;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_util::sync::CancellationToken;

use crate::error::ProviderError;
use crate::stems::action::ToolSchema;
use crate::types::Message;

use super::{LlmProvider, StreamChunk, build_openai_content, classify_http_error, run_or_cancel};

pub struct OpenAIProvider {
    client: Client,
    api_key: String,
    api_base: String,
    model: String,
    max_tokens: u32,
}

impl OpenAIProvider {
    pub fn new(api_key: String, api_base: String, model: String, max_tokens: u32) -> Self {
        Self {
            client: Client::new(),
            api_key,
            api_base,
            model,
            max_tokens,
        }
    }
}

impl LlmProvider for OpenAIProvider {
    fn infer_stream(
        &self,
        messages: Vec<Message>,
        tools: Option<&[ToolSchema]>,
        cancel_token: Option<CancellationToken>,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>> {
        let (tx, rx) = mpsc::unbounded_channel();

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "stream": true,
            "messages": messages.iter().map(|m| {
                serde_json::json!({
                    "role": m.role.to_api_str(),
                    "content": build_openai_content(m),
                })
            }).collect::<Vec<_>>(),
        });
        if let Some(tools) = tools {
            body["tools"] = serde_json::Value::Array(
                tools
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "type": "function",
                            "function": {
                                "name": t.name,
                                "description": t.description,
                                "parameters": t.parameters,
                            }
                        })
                    })
                    .collect::<Vec<_>>(),
            );
        }
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let url = format!("{}/chat/completions", self.api_base.trim_end_matches('/'));

        tokio::spawn(async move {
            run_or_cancel(cancel_token, async {
                // Track streaming tool call fragments (index → accumulated state).
                let mut tc_state: std::collections::HashMap<usize, (String, String, String)> =
                    std::collections::HashMap::new(); // index → (id, name, args_json)
                let resp = match client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", &api_key))
                    .json(&body)
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = tx.send(Err(ProviderError::Network(e.to_string())));
                        return;
                    }
                };

                let status = resp.status().as_u16();
                if !resp.status().is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    let err = classify_http_error(status, &body);
                    let _ = tx.send(Err(err));
                    return;
                }

                let mut byte_stream = resp.bytes_stream();
                let mut buffer = String::new();
                let mut input_tokens: u64 = 0;
                let mut output_tokens: u64 = 0;

                loop {
                    let chunk = match tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        futures::StreamExt::next(&mut byte_stream),
                    )
                    .await
                    {
                        Ok(Some(Ok(bytes))) => bytes,
                        Ok(Some(Err(e))) => {
                            let _ = tx.send(Err(ProviderError::Stream(e.to_string())));
                            return;
                        }
                        Ok(None) => break,
                        Err(_elapsed) => {
                            let _ = tx.send(Err(ProviderError::StreamStalled));
                            return;
                        }
                    };

                    buffer.push_str(&String::from_utf8_lossy(&chunk));
                    while let Some(pos) = buffer.find('\n') {
                        let line = buffer[..pos].trim().to_string();
                        buffer = buffer[pos + 1..].to_string();

                        if line.is_empty() || !line.starts_with("data: ") {
                            continue;
                        }
                        let data = &line[6..];
                        if data == "[DONE]" {
                            continue;
                        }

                        if let Ok(event) = serde_json::from_str::<Value>(data) {
                            if let Some(err) = event["error"].as_object() {
                                let msg = err["message"].as_str().unwrap_or("Unknown OpenAI error");
                                let _ = tx.send(Err(ProviderError::Provider(msg.to_string())));
                                return;
                            }
                            if let Some(choice) =
                                event["choices"].as_array().and_then(|c| c.first())
                            {
                                if let Some(text) = choice["delta"]["content"].as_str() {
                                    let _ = tx.send(Ok(StreamChunk::Delta(text.to_string())));
                                }
                                // Parse streaming tool_calls (native tools API)
                                if let Some(tc_arr) = choice["delta"]["tool_calls"].as_array() {
                                    for tc in tc_arr {
                                        let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                                        let entry = tc_state.entry(idx).or_insert_with(|| {
                                            (String::new(), String::new(), String::new())
                                        });
                                        if let Some(id) = tc["id"].as_str() {
                                            entry.0 = id.to_string();
                                        }
                                        if let Some(n) = tc["function"]["name"].as_str() {
                                            entry.1 = n.to_string();
                                        }
                                        if let Some(a) = tc["function"]["arguments"].as_str() {
                                            entry.2.push_str(a);
                                        }
                                    }
                                }
                                // When finish_reason appears, emit completed tool calls
                                if let Some(reason) = choice["finish_reason"].as_str()
                                    && reason == "tool_calls"
                                    && !tc_state.is_empty()
                                {
                                    // Sort by index and emit
                                    let mut items: Vec<_> = tc_state.drain().collect();
                                    items.sort_by_key(|(k, _)| *k);
                                    for (_, (id, name, args)) in items {
                                        let _ = tx.send(Ok(StreamChunk::NativeToolCall {
                                            id,
                                            name,
                                            arguments: args,
                                        }));
                                    }
                                }
                            }
                            // Parse usage from final chunk (finish_reason == "stop")
                            if let Some(usage) = event["usage"].as_object() {
                                input_tokens = usage
                                    .get("prompt_tokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                output_tokens = usage
                                    .get("completion_tokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                            }
                        }
                    }
                }
                if input_tokens > 0 || output_tokens > 0 {
                    let _ = tx.send(Ok(StreamChunk::Usage {
                        input_tokens,
                        output_tokens,
                    }));
                }
            })
            .await;
        });

        Box::pin(UnboundedReceiverStream::new(rx))
    }
}
