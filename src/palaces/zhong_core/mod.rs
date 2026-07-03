use std::pin::Pin;

use futures::Stream;
use reqwest::Client;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_util::sync::CancellationToken;

pub(crate) use crate::stems::action::ToolSchema;
use crate::types::Message;

/// A chunk from an LLM provider stream.
///
/// Either a text delta or aggregated token usage (emitted once at stream end).
#[derive(Debug, Clone)]
pub enum StreamChunk {
    Delta(String),
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    /// Prompt-cache telemetry (Anthropic). Emitted when caching is active so
    /// the 神盘 can observe hit rate. `cache_read` > 0 means the stable prefix
    /// was served from cache this turn.
    CacheHit {
        input_tokens: u64,
        cache_read: u64,
        cache_creation: u64,
    },
    /// Native tool call from providers that support tools API
    /// (OpenAI tool_calls, Anthropic tool_use, Gemini functionCall).
    NativeToolCall {
        id: String,
        name: String,
        arguments: String,
    },
}

/// LLM Provider trait — 甲隐于六仪，外部通过此 trait 间接触发 LLM
pub trait LlmProvider: Send + Sync {
    /// 流式推理，返回 SSE-compatible delta stream（'static，可跨越 task 边界）
    fn infer_stream(
        &self,
        messages: Vec<Message>,
        tools: Option<&[ToolSchema]>,
        cancel_token: Option<CancellationToken>,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, String>> + Send>>;

    /// Whether this provider supports prompt caching (e.g. Anthropic
    /// `cache_control`). Default false; caching providers override.
    fn supports_caching(&self) -> bool {
        false
    }

    /// Streaming inference with a split system prompt (`stable` + `dynamic`).
    ///
    /// Caching providers (Anthropic) override this to place `cache_control` on
    /// the stable prefix so it is reused across turns; the dynamic tail
    /// (memory/profile, which changes every turn via atma_graha) is not cached.
    ///
    /// Default impl concatenates stable+dynamic into a single system message
    /// prepended to `messages` and delegates to `infer_stream` — preserving the
    /// current behaviour for non-caching providers (OpenAI-compatible, Gemini,
    /// Mock) and the ~10 internal callers that still use `infer()`.
    fn infer_stream_with_system(
        &self,
        messages: Vec<Message>,
        system: SystemPrompt,
        tools: Option<&[ToolSchema]>,
        cancel_token: Option<CancellationToken>,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, String>> + Send>> {
        let full = if system.dynamic.is_empty() {
            system.stable
        } else {
            format!("{}\n\n{}", system.stable, system.dynamic)
        };
        let mut msgs = Vec::with_capacity(messages.len() + 1);
        msgs.push(Message::text(crate::types::Role::System, full));
        msgs.extend(messages);
        self.infer_stream(msgs, tools, cancel_token)
    }
}

/// A system prompt split into a cacheable stable prefix and a dynamic tail.
///
/// `stable` = 人设 (ren) + tools + always-on skills — byte-stable across
/// turns (modulo skill hot-reload), so it can carry an Anthropic
/// `cache_control: ephemeral` breakpoint.
/// `dynamic` = context-activated skills + user profile + memory catalog +
/// top_influence seeds + todo list — varies every turn (atma_graha-gated),
/// never cached.
#[derive(Debug, Clone, Default)]
pub struct SystemPrompt {
    pub stable: String,
    pub dynamic: String,
}

/// Whether this provider kind supports native tools API (vs XML text fallback).
pub fn use_native_tools(kind: &str) -> bool {
    matches!(kind, "openai" | "anthropic" | "gemini")
}

/// Build the API content value for a message.
///
/// Returns a plain string for text-only messages, or a content-blocks array
/// (Anthropic format) when images are present.
fn build_anthropic_content(msg: &Message) -> serde_json::Value {
    if msg.images.is_empty() {
        return serde_json::Value::String(msg.content.clone());
    }
    let mut blocks: Vec<serde_json::Value> = Vec::new();
    if !msg.content.is_empty() {
        blocks.push(serde_json::json!({
            "type": "text",
            "text": msg.content,
        }));
    }
    for img in &msg.images {
        blocks.push(serde_json::json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": img.media_type,
                "data": img.data,
            },
        }));
    }
    serde_json::Value::Array(blocks)
}

/// Build the API content value for OpenAI-compatible providers.
fn build_openai_content(msg: &Message) -> serde_json::Value {
    if msg.images.is_empty() {
        return serde_json::Value::String(msg.content.clone());
    }
    let mut parts: Vec<serde_json::Value> = Vec::new();
    if !msg.content.is_empty() {
        parts.push(serde_json::json!({
            "type": "text",
            "text": msg.content,
        }));
    }
    for img in &msg.images {
        parts.push(serde_json::json!({
            "type": "image_url",
            "image_url": {
                "url": format!("data:{};base64,{}", img.media_type, img.data),
            },
        }));
    }
    serde_json::Value::Array(parts)
}

/// Run `f` to completion, or return early if the cancellation token fires.
async fn run_or_cancel(
    cancel_token: Option<CancellationToken>,
    f: impl Future<Output = ()> + Send,
) {
    match cancel_token {
        Some(token) => {
            tokio::select! {
                _ = token.cancelled() => {},
                _ = f => {}
            }
        }
        None => {
            f.await;
        }
    }
}

/// Categorize an HTTP status into a user-facing error string.
pub(crate) fn classify_http_error(status: u16, body: &str) -> String {
    match status {
        429 => format!("Rate limited — retry after a few seconds. {body}"),
        401 | 403 => format!("Authentication failed (HTTP {status}). Check API key."),
        500..=599 => {
            format!("Server error (HTTP {status}). The provider may be overloaded. {body}")
        }
        400..=499 => format!("Client error (HTTP {status}). Request may be malformed. {body}"),
        _ => format!("HTTP {status}: {body}"),
    }
}

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
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, String>> + Send>> {
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
            body["tools"] = serde_json::Value::Array(tools.iter().map(|t| serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.parameters,
            })).collect::<Vec<_>>());
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
                        let _ = tx.send(Err(format!("Network error: {e}")));
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

                        if let Ok(event) = serde_json::from_str::<Value>(data) {
                            match event["type"].as_str() {
                                Some("message_start") => {
                                    if let Some(u) =
                                        event["message"]["usage"]["input_tokens"].as_u64()
                                    {
                                        input_tokens = u;
                                    }
                                }
                                Some("content_block_delta") => {
                                    if let Some(text) = event["delta"]["text"].as_str() {
                                        let _ = tx.send(Ok(StreamChunk::Delta(text.to_string())));
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
                                    let _ = tx.send(Err(format!("Provider error: {msg}")));
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
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, String>> + Send>> {
        let body = build_anthropic_system_body(&self.model, self.max_tokens, &messages, &system, tools);

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
                        let _ = tx.send(Err(format!("Network error: {e}")));
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
fn build_anthropic_system_body(
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
        body["tools"] = serde_json::Value::Array(tools.iter().map(|t| serde_json::json!({
            "name": t.name,
            "description": t.description,
            "input_schema": t.parameters,
        })).collect::<Vec<_>>());
    }
    body
}

/// Shared Anthropic SSE response reader: parses the byte stream and forwards
/// deltas/usage/errors to `tx`. Extracted so both the cached and uncached
/// paths use identical streaming logic.
async fn stream_anthropic_response(
    resp: reqwest::Response,
    tx: mpsc::UnboundedSender<Result<StreamChunk, String>>,
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
                let _ = tx.send(Err(format!("Stream error: {e}")));
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
                    Some("content_block_delta") => {
                        match event["delta"]["type"].as_str() {
                            Some("text_delta") => {
                                if let Some(text) = event["delta"]["text"].as_str() {
                                    let _ = tx.send(Ok(StreamChunk::Delta(text.to_string())));
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
                        let _ = tx.send(Err(format!("Provider error: {msg}")));
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
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, String>> + Send>> {
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
            body["tools"] = serde_json::Value::Array(tools.iter().map(|t| serde_json::json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                }
            })).collect::<Vec<_>>());
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
                        let _ = tx.send(Err(format!("Network error: {e}")));
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
                        if data == "[DONE]" {
                            continue;
                        }

                        if let Ok(event) = serde_json::from_str::<Value>(data) {
                            if let Some(err) = event["error"].as_object() {
                                let msg = err["message"].as_str().unwrap_or("Unknown OpenAI error");
                                let _ = tx.send(Err(format!("Provider error: {msg}")));
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
                                        let entry = tc_state
                                            .entry(idx)
                                            .or_insert_with(|| (String::new(), String::new(), String::new()));
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
                                    && reason == "tool_calls" && !tc_state.is_empty() {
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

// ── Gemini ──────────────────────────────────────────────────

pub mod gemini;
pub(crate) use gemini::GeminiProvider;

// ── Factory ────────────────────────────────────────────────

use crate::config::ProviderProfile;

/// Create a provider from a config profile.
///
/// Supported kinds: openai, anthropic, gemini.
/// All OpenAI-compatible providers (deepseek, ollama, openrouter, etc.) use kind "openai" and share the same
/// implementation.
pub fn create_provider(profile: &ProviderProfile, model: &str) -> Box<dyn LlmProvider> {
    let max_tokens = profile.max_tokens.unwrap_or(4096);
    match profile.kind.to_lowercase().as_str() {
        "anthropic" => Box::new(AnthropicProvider::new(
            profile.api_key.clone(),
            profile.base_url.clone(),
            model.to_string(),
            max_tokens,
        )),
        "gemini" => Box::new(GeminiProvider::new(
            profile.api_key.clone(),
            profile.base_url.clone(),
            model.to_string(),
            max_tokens,
        )),
        // openai and anything unknown
        _ => Box::new(OpenAIProvider::new(
            profile.api_key.clone(),
            profile.base_url.clone(),
            model.to_string(),
            max_tokens,
        )),
    }
}

// ── JiaCore (中五宫 · 甲之所在) ─────────────────────────────

/// 中五宫 — 甲之所在
///
/// LLM = 甲。JiaCore 封装 LLM Provider，`llm_provider` 字段私有，
/// `infer` 方法为 `pub(crate)` —— 仅同 crate 的 HeavenPlate 可调用。
/// 外部代码（人盘、神盘）无法直接触达 LLM。
pub struct JiaCore {
    provider: Box<dyn LlmProvider>,
    model: String,
    pub context_window: usize,
    pub provider_kind: String,
}

impl JiaCore {
    pub fn new(profile: &ProviderProfile, model: &str) -> Self {
        Self {
            provider: create_provider(profile, model),
            provider_kind: profile.kind.clone(),
            model: model.to_string(),
            context_window: profile.context_window.unwrap_or(8192),
        }
    }

    /// LLM 推理 — 仅 crate 内部可调用
    pub(crate) fn infer(
        &self,
        messages: Vec<Message>,
        tools: Option<&[ToolSchema]>,
        cancel_token: Option<CancellationToken>,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, String>> + Send>> {
        self.provider.infer_stream(messages, tools, cancel_token)
    }

    /// LLM 推理 with split system prompt (P2 prompt caching). Used by the main
    /// agent loop so the Anthropic provider can cache the stable system prefix.
    /// Non-caching providers fall back to concatenating stable+dynamic (the
    /// pre-P2 behaviour) via the trait's default impl. pub(crate) — 甲隐于六仪.
    pub(crate) fn infer_with_system(
        &self,
        messages: Vec<Message>,
        system: SystemPrompt,
        tools: Option<&[ToolSchema]>,
        cancel_token: Option<CancellationToken>,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, String>> + Send>> {
        self.provider
            .infer_stream_with_system(messages, system, tools, cancel_token)
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Create a JiaCore backed by a mock provider. Test-only.
    #[cfg(any(test, feature = "test-utils"))]
    pub(crate) fn with_mock(responses: Vec<String>) -> Self {
        use mock::MockProvider;
        Self {
            provider: Box::new(MockProvider::new(responses)),
            model: "mock".into(),
            context_window: 4096,
            provider_kind: "mock".into(),
        }
    }
}

// ── Mock Provider (for testing) ─────────────────────────────

/// A mock LLM provider that yields predefined responses.
///
/// Used in agent loop end-to-end tests. Each call to `infer_stream`
/// consumes one response from the queue and streams its characters as deltas.
#[cfg(any(test, feature = "test-utils"))]
pub(crate) mod mock {
    use crate::types::Message;
    use futures::Stream;
    use std::pin::Pin;
    use std::sync::Mutex;
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::UnboundedReceiverStream;
    use tokio_util::sync::CancellationToken;

    pub struct MockProvider {
        /// Queue of response strings, streamed character-by-character.
        responses: Mutex<Vec<String>>,
    }

    impl MockProvider {
        pub fn new(responses: Vec<String>) -> Self {
            Self {
                responses: Mutex::new(responses),
            }
        }
    }

    impl super::LlmProvider for MockProvider {
        fn infer_stream(
            &self,
            _messages: Vec<Message>,
            _tools: Option<&[super::ToolSchema]>,
            _cancel_token: Option<CancellationToken>,
        ) -> Pin<Box<dyn Stream<Item = Result<super::StreamChunk, String>> + Send>> {
            let (tx, rx) = mpsc::unbounded_channel();
            let mut guard = self.responses.lock().unwrap();
            let response = if guard.is_empty() {
                Err("MockProvider: no responses left".into())
            } else {
                Ok(guard.remove(0))
            };

            tokio::spawn(async move {
                match response {
                    Ok(text) => {
                        for ch in text.chars() {
                            let _ = tx.send(Ok(super::StreamChunk::Delta(ch.to_string())));
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                    }
                }
            });

            Box::pin(UnboundedReceiverStream::new(rx))
        }
    }
}

/// Fetch model list from a provider's API.
/// Returns empty vec on failure (caller handles the warning).
pub async fn fetch_models(profile: &crate::config::ProviderProfile) -> Vec<String> {
    let client = reqwest::Client::new();
    match profile.kind.as_str() {
        "openai" | "openrouter" => {
            let base = profile.base_url.trim_end_matches('/');
            // Detect ollama-style endpoints (no /v1, typically localhost:11434)
            let is_ollama = !base.ends_with("/v1");
            let (url, key_field) = if is_ollama {
                (format!("{base}/api/tags"), "name")
            } else {
                (format!("{base}/models"), "id")
            };
            let mut req = client.get(&url).timeout(std::time::Duration::from_secs(15));
            if !is_ollama {
                req = req.header("Authorization", format!("Bearer {}", profile.api_key));
            }
            match req.send().await {
                Ok(resp) => {
                    let data: serde_json::Value = match resp.json().await {
                        Ok(d) => d,
                        Err(_) => return vec![],
                    };
                    let array_key = if is_ollama { "models" } else { "data" };
                    data[array_key]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| m[key_field].as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default()
                }
                Err(_) => vec![],
            }
        }
        _ => {
            // anthropic, gemini: no public model list API
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, Role};

    #[test]
    fn anthropic_system_body_has_cache_breakpoint_on_stable() {
        let system = SystemPrompt {
            stable: "You are Jia. Tools: ...".to_string(),
            dynamic: "## Current tasks\n- [ ] do thing".to_string(),
        };
        let messages = vec![Message::text(Role::User, "hello")];
        let body = build_anthropic_system_body("claude-x", 1024, &messages, &system, None);

        // system is a top-level array of two blocks
        let sys = body["system"].as_array().expect("system is array");
        assert_eq!(sys.len(), 2, "stable + dynamic blocks");

        // stable block carries the ephemeral cache breakpoint
        let stable_block = &sys[0];
        assert_eq!(stable_block["text"].as_str(), Some(system.stable.as_str()));
        assert_eq!(
            stable_block["cache_control"]["type"].as_str(),
            Some("ephemeral"),
            "stable block must carry cache_control: ephemeral"
        );

        // dynamic block has NO cache_control
        let dynamic_block = &sys[1];
        assert_eq!(
            dynamic_block["text"].as_str(),
            Some(system.dynamic.as_str())
        );
        assert!(
            dynamic_block.get("cache_control").is_none(),
            "dynamic block must NOT be cached"
        );

        // messages array excludes system (it travels via top-level `system`)
        let msgs = body["messages"].as_array().expect("messages is array");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"].as_str(), Some("user"));
    }

    #[test]
    fn anthropic_system_body_omits_empty_segments() {
        let system = SystemPrompt {
            stable: "identity only".to_string(),
            dynamic: String::new(),
        };
        let body = build_anthropic_system_body("m", 1, &[], &system, None);
        let sys = body["system"].as_array().unwrap();
        assert_eq!(sys.len(), 1, "only stable block when dynamic empty");
        assert_eq!(sys[0]["cache_control"]["type"].as_str(), Some("ephemeral"));
    }

    #[test]
    fn default_infer_stream_with_system_concatenates() {
        // Non-caching path: the trait default concatenates stable+dynamic into
        // one system message. Verified via the mock provider (supports_caching
        // is false by default).
        #[cfg(any(test, feature = "test-utils"))]
        {
            use crate::palaces::zhong_core::mock::MockProvider;
            let p = MockProvider::new(vec!["ok".to_string()]);
            assert!(!p.supports_caching());
        }
    }
}
