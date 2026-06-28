// ── Gemini Provider (Google Generative Language API) ──────────

use std::pin::Pin;

use futures::Stream;
use reqwest::Client;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::stems::action::ToolSchema;

use tokio_util::sync::CancellationToken;

use crate::palaces::zhong_core::{LlmProvider, StreamChunk};
use crate::types::{Message, Role};

pub struct GeminiProvider {
    client: Client,
    api_key: String,
    api_base: String,
    model: String,
    max_tokens: u32,
}

impl GeminiProvider {
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

/// Build the Gemini API request body from messages.
fn build_gemini_body(messages: &[Message], max_tokens: u32, tools: Option<&[ToolSchema]>) -> Value {
    let mut contents: Vec<Value> = Vec::new();
    let mut system_instruction: Option<Value> = None;

    for msg in messages {
        match msg.role {
            Role::System => {
                system_instruction = Some(serde_json::json!({
                    "parts": [{"text": msg.content}]
                }));
            }
            Role::User => {
                contents.push(serde_json::json!({
                    "role": "user",
                    "parts": [{"text": msg.content}]
                }));
            }
            Role::Assistant => {
                contents.push(serde_json::json!({
                    "role": "model",
                    "parts": [{"text": msg.content}]
                }));
            }
        }
    }

    let mut body = serde_json::json!({
        "contents": contents,
        "generationConfig": {
            "maxOutputTokens": max_tokens,
        },
    });
    if let Some(si) = system_instruction {
        body["systemInstruction"] = si;
    }
    if let Some(tools) = tools {
        let declarations: Vec<Value> = tools.iter().map(|t| serde_json::json!({
            "name": t.name,
            "description": t.description,
            "parameters": t.parameters,
        })).collect();
        body["tools"] = serde_json::json!([{"functionDeclarations": declarations}]);
    }
    body
}

impl LlmProvider for GeminiProvider {
    fn infer_stream(
        &self,
        messages: Vec<Message>,
        tools: Option<&[ToolSchema]>,
        cancel_token: Option<CancellationToken>,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, String>> + Send>> {
        let (tx, rx) = mpsc::unbounded_channel();

        let body = build_gemini_body(&messages, self.max_tokens, tools);
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let base = self.api_base.trim_end_matches('/').to_string();
        let model = self.model.clone();
        let url = format!("{base}/models/{model}:streamGenerateContent?alt=sse");

        tokio::spawn(async move {
            super::run_or_cancel(cancel_token, async {
                let resp = match client
                    .post(&url)
                    .header("x-goog-api-key", &api_key)
                    .json(&body)
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = tx.send(Err(format!("Network error: {e}")));
                        return;
                    }
                };

                let status = resp.status().as_u16();
                if !resp.status().is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    let err = super::classify_http_error(status, &body);
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
                            let _ = tx.send(Err(format!("Stream error: {e}")));
                            return;
                        }
                        Ok(None) => break,
                        Err(_elapsed) => {
                            let _ = tx
                                .send(Err("LLM stream stalled — no data received for 30s".into()));
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

                        let Ok(event) = serde_json::from_str::<Value>(data) else {
                            tracing::debug!(?data, "Gemini SSE: failed to parse event");
                            continue;
                        };

                        if let Some(err) = event["error"].as_object() {
                            let msg = err["message"].as_str().unwrap_or("Unknown Gemini error");
                            let _ = tx.send(Err(format!("Provider error: {msg}")));
                            return;
                        }

                        // Parse usage metadata
                        if let Some(um) = event["usageMetadata"].as_object() {
                            input_tokens = um
                                .get("promptTokenCount")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            output_tokens = um
                                .get("candidatesTokenCount")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                        }

                        // Extract delta: candidates[0].content.parts
                        if let Some(parts) = event["candidates"]
                            .as_array()
                            .and_then(|c| c.first())
                            .and_then(|c| c["content"]["parts"].as_array())
                        {
                            for part in parts {
                                if let Some(text) = part["text"].as_str() {
                                    let _ = tx.send(Ok(StreamChunk::Delta(text.to_string())));
                                }
                                if let Some(fc) = part["functionCall"].as_object() {
                                    let id = uuid::Uuid::new_v4().to_string();
                                    let name = fc["name"].as_str().unwrap_or("").to_string();
                                    let args = fc["args"].to_string();
                                    let _ = tx.send(Ok(StreamChunk::NativeToolCall {
                                        id,
                                        name,
                                        arguments: args,
                                    }));
                                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_body_no_system() {
        let msgs = vec![
            Message::text(Role::User, "hello"),
            Message::text(Role::Assistant, "hi there"),
        ];
        let body = build_gemini_body(&msgs, 1024, None);
        let contents = body["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 2);
        assert_eq!(contents[0]["role"].as_str().unwrap(), "user");
        assert_eq!(contents[1]["role"].as_str().unwrap(), "model");
        assert!(body.get("systemInstruction").is_none());
    }

    #[test]
    fn test_build_body_with_system() {
        let msgs = vec![
            Message::text(Role::System, "you are helpful"),
            Message::text(Role::User, "hello"),
        ];
        let body = build_gemini_body(&msgs, 1024, None);
        let contents = body["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 1); // system excluded from contents
        assert_eq!(contents[0]["role"].as_str().unwrap(), "user");
        let si = body["systemInstruction"].as_object().unwrap();
        let parts = si["parts"].as_array().unwrap();
        assert_eq!(parts[0]["text"].as_str().unwrap(), "you are helpful");
    }

    #[test]
    fn test_build_body_max_tokens() {
        let body = build_gemini_body(&[], 2048, None);
        assert_eq!(
            body["generationConfig"]["maxOutputTokens"]
                .as_u64()
                .unwrap(),
            2048
        );
    }
}
