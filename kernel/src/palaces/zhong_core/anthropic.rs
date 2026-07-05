// ── Anthropic Provider ───────────────────────────────────────

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

use super::{
    LlmProvider, StreamChunk, SystemPrompt, build_anthropic_content, classify_http_error,
    run_or_cancel,
};

// ── Anthropic ──────────────────────────────────────────────

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    api_base: String,
    model: String,
    max_tokens: u32,
}

impl AnthropicProvider {
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

impl LlmProvider for AnthropicProvider {
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
                    "content": build_anthropic_content(m),
                })
            }).collect::<Vec<_>>(),
        });
        if let Some(tools) = tools {
            body["tools"] = serde_json::Value::Array(
                tools
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "name": t.name,
                            "description": t.description,
                            "input_schema": t.parameters,
                        })
                    })
                    .collect::<Vec<_>>(),
            );
        }
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let url = format!("{}/messages", self.api_base.trim_end_matches('/'));

        tokio::spawn(async move {
            run_or_cancel(cancel_token, async {
                let resp = match client
                    .post(&url)
                    .header("x-api-key", &api_key)
                    .header("anthropic-version", "2023-06-01")
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
                let mut tool_use_state: std::collections::HashMap<usize, (String, String, String)> =
                    std::collections::HashMap::new();

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

                        if let Ok(event) = serde_json::from_str::<Value>(data) {
                            match event["type"].as_str() {
                                Some("message_start") => {
                                    if let Some(u) =
                                        event["message"]["usage"]["input_tokens"].as_u64()
                                    {
                                        input_tokens = u;
                                    }
                                }
                                Some("content_block_start") => {
                                    if event["content_block"]["type"].as_str() == Some("tool_use") {
                                        let idx = event["index"].as_u64().unwrap_or(0) as usize;
                                        let id = event["content_block"]["id"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string();
                                        let name = event["content_block"]["name"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string();
                                        tool_use_state.insert(idx, (id, name, String::new()));
                                    }
                                }
                                Some("content_block_delta") => {
                                    match event["delta"]["type"].as_str() {
                                        Some("text_delta") => {
                                            if let Some(text) = event["delta"]["text"].as_str() {
                                                let _ = tx
                                                    .send(Ok(StreamChunk::Delta(text.to_string())));
                                            }
                                        }
                                        Some("input_json_delta") => {
                                            let idx = event["index"].as_u64().unwrap_or(0) as usize;
                                            if let Some(entry) = tool_use_state.get_mut(&idx)
                                                && let Some(partial) =
                                                    event["delta"]["partial_json"].as_str()
                                            {
                                                entry.2.push_str(partial);
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                Some("content_block_stop") => {
                                    let idx = event["index"].as_u64().unwrap_or(0) as usize;
                                    if let Some((id, name, args)) = tool_use_state.remove(&idx) {
                                        let _ = tx.send(Ok(StreamChunk::NativeToolCall {
                                            id,
                                            name,
                                            arguments: args,
                                        }));
                                    }
                                }
                                Some("message_delta") => {
                                    if let Some(u) = event["usage"]["output_tokens"].as_u64() {
                                        output_tokens = u;
                                    }
                                }
                                Some("error") => {
                                    let msg = event["error"]["message"]
                                        .as_str()
                                        .unwrap_or("Unknown Anthropic error");
                                    let _ = tx.send(Err(ProviderError::Provider(msg.to_string())));
                                    return;
                                }
                                _ => {}
                            }
                        } else {
                            tracing::debug!(?data, "Anthropic SSE: failed to parse event");
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

    /// Anthropic supports prompt caching via `cache_control` on content blocks.
    ///
    /// The system prompt is sent as a top-level `system` array (not a message):
    ///   [ {text: stable, cache_control: ephemeral}, {text: dynamic} ]
    /// so the stable prefix (人设 + tools + always-on skills) is cached across
    /// turns, while the dynamic tail (memory/profile) is not. Conversation
    /// messages carry a second breakpoint on the last pre-rollback message so
    /// the compacted-history prefix is cached between compactions.
    fn supports_caching(&self) -> bool {
        true
    }

    fn infer_stream_with_system(
        &self,
        messages: Vec<Message>,
        system: SystemPrompt,
        tools: Option<&[ToolSchema]>,
        cancel_token: Option<CancellationToken>,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>> {
        let body =
            build_anthropic_system_body(&self.model, self.max_tokens, &messages, &system, tools);

        // Reuse the streaming pipeline: spawn the same SSE reader by feeding
        // the pre-built body through the shared request/stream path.
        let (tx, rx) = mpsc::unbounded_channel();
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let url = format!("{}/messages", self.api_base.trim_end_matches('/'));

        tokio::spawn(async move {
            run_or_cancel(cancel_token, async {
                let resp = match client
                    .post(&url)
                    .header("x-api-key", &api_key)
                    .header("anthropic-version", "2023-06-01")
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
                stream_anthropic_response(resp, tx).await;
            })
            .await;
        });

        Box::pin(UnboundedReceiverStream::new(rx))
    }
}

/// Build the Anthropic `/messages` request body with a top-level `system`
/// array carrying a `cache_control: ephemeral` breakpoint at the end of the
/// stable segment. Extracted for unit testing (no network needed).
pub(crate) fn build_anthropic_system_body(
    model: &str,
    max_tokens: u32,
    messages: &[Message],
    system: &SystemPrompt,
    tools: Option<&[ToolSchema]>,
) -> Value {
    // Build top-level system content blocks with a cache breakpoint at the
    // end of the stable segment.
    let mut system_blocks: Vec<Value> = Vec::new();
    if !system.stable.is_empty() {
        system_blocks.push(serde_json::json!({
            "type": "text",
            "text": system.stable,
            "cache_control": { "type": "ephemeral" }
        }));
    }
    if !system.dynamic.is_empty() {
        system_blocks.push(serde_json::json!({
            "type": "text",
            "text": system.dynamic
        }));
    }

    let anthropic_messages: Vec<Value> = messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "role": m.role.to_api_str(),
                "content": build_anthropic_content(m),
            })
        })
        .collect();

    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "stream": true,
        "messages": anthropic_messages,
    });
    if !system_blocks.is_empty() {
        body["system"] = Value::Array(system_blocks);
    }
    if let Some(tools) = tools {
        body["tools"] = serde_json::Value::Array(
            tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.parameters,
                    })
                })
                .collect::<Vec<_>>(),
        );
    }
    body
}

/// Shared Anthropic SSE response reader: parses the byte stream and forwards
/// deltas/usage/errors to `tx`. Extracted so both the cached and uncached
/// paths use identical streaming logic.
async fn stream_anthropic_response(
    resp: reqwest::Response,
    tx: mpsc::UnboundedSender<Result<StreamChunk, ProviderError>>,
) {
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
    let mut cache_read: u64 = 0;
    let mut cache_creation: u64 = 0;
    let mut tool_use_state: std::collections::HashMap<usize, (String, String, String)> =
        std::collections::HashMap::new(); // index → (id, name, partial_json)

    loop {
        let chunk = match futures::StreamExt::next(&mut byte_stream).await {
            Some(Ok(bytes)) => bytes,
            Some(Err(e)) => {
                let _ = tx.send(Err(ProviderError::Stream(e.to_string())));
                return;
            }
            None => break,
        };

        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buffer.find('\n') {
            let line = buffer[..pos].trim().to_string();
            buffer = buffer[pos + 1..].to_string();

            if line.is_empty() || !line.starts_with("data: ") {
                continue;
            }
            let data = &line[6..];

            if let Ok(event) = serde_json::from_str::<Value>(data) {
                match event["type"].as_str() {
                    Some("message_start") => {
                        if let Some(u) = event["message"]["usage"]["input_tokens"].as_u64() {
                            input_tokens = u;
                        }
                        if let Some(c) =
                            event["message"]["usage"]["cache_read_input_tokens"].as_u64()
                        {
                            cache_read = c;
                        }
                        if let Some(c) =
                            event["message"]["usage"]["cache_creation_input_tokens"].as_u64()
                        {
                            cache_creation = c;
                        }
                    }
                    Some("content_block_start") => {
                        let cb = &event["content_block"];
                        if cb["type"].as_str() == Some("tool_use") {
                            let idx = event["index"].as_u64().unwrap_or(0) as usize;
                            let id = cb["id"].as_str().unwrap_or("").to_string();
                            let name = cb["name"].as_str().unwrap_or("").to_string();
                            tool_use_state.insert(idx, (id, name, String::new()));
                        }
                    }
                    Some("content_block_delta") => match event["delta"]["type"].as_str() {
                        Some("text_delta") => {
                            if let Some(text) = event["delta"]["text"].as_str() {
                                let _ = tx.send(Ok(StreamChunk::Delta(text.to_string())));
                            }
                        }
                        Some("input_json_delta") => {
                            let idx = event["index"].as_u64().unwrap_or(0) as usize;
                            if let Some(entry) = tool_use_state.get_mut(&idx)
                                && let Some(partial) = event["delta"]["partial_json"].as_str()
                            {
                                entry.2.push_str(partial);
                            }
                        }
                        _ => {}
                    },
                    Some("content_block_stop") => {
                        let idx = event["index"].as_u64().unwrap_or(0) as usize;
                        if let Some((id, name, args)) = tool_use_state.remove(&idx) {
                            let _ = tx.send(Ok(StreamChunk::NativeToolCall {
                                id,
                                name,
                                arguments: args,
                            }));
                        }
                    }
                    Some("message_delta") => {
                        if let Some(u) = event["usage"]["output_tokens"].as_u64() {
                            output_tokens = u;
                        }
                    }
                    Some("error") => {
                        let msg = event["error"]["message"]
                            .as_str()
                            .unwrap_or("Unknown Anthropic error");
                        let _ = tx.send(Err(ProviderError::Provider(msg.to_string())));
                        return;
                    }
                    _ => {}
                }
            } else {
                tracing::debug!(?data, "Anthropic SSE: failed to parse event");
            }
        }
    }
    // Emit a CacheHit usage signal when caching is active so the 神盘 can
    // observe hit rate (P2 telemetry). Note: input_tokens from the API already
    // includes cache tokens (cache_read + cache_creation), so we emit a separate
    // CacheHit chunk for telemetry rather than inflating the Usage chunk.
    if cache_read > 0 || cache_creation > 0 {
        let _ = tx.send(Ok(StreamChunk::Usage {
            input_tokens,
            output_tokens,
        }));
        let _ = tx.send(Ok(StreamChunk::CacheHit {
            input_tokens,
            cache_read,
            cache_creation,
        }));
    } else if input_tokens > 0 || output_tokens > 0 {
        let _ = tx.send(Ok(StreamChunk::Usage {
            input_tokens,
            output_tokens,
        }));
    }
}
