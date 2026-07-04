use crate::error::ProviderError;
use std::pin::Pin;

use futures::Stream;
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
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>>;

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
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>> {
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
pub(crate) fn build_anthropic_content(msg: &Message) -> serde_json::Value {
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

/// Categorize an HTTP status into a typed ProviderError.
pub(crate) fn classify_http_error(status: u16, body: &str) -> crate::error::ProviderError {
    match status {
        429 => crate::error::ProviderError::RateLimited { body: body.to_string() },
        401 | 403 => crate::error::ProviderError::AuthFailed { status },
        500..=599 => {
            crate::error::ProviderError::ServerError { status, body: body.to_string() }
        }
        400..=499 => crate::error::ProviderError::ClientError { status, body: body.to_string() },
        _ => crate::error::ProviderError::Provider(format!("HTTP {status}: {body}")),
    }
}


// ── Anthropic ──────────────────────────────────────────────
mod anthropic;
pub use anthropic::AnthropicProvider;

// ── OpenAI ────────────────────────────────────────────────
mod openai;
pub use openai::OpenAIProvider;

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
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>> {
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
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>> {
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
    use crate::error::ProviderError;
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
        ) -> Pin<Box<dyn Stream<Item = Result<super::StreamChunk, ProviderError>> + Send>> {
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
    use super::anthropic::build_anthropic_system_body;
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
