//! ProviderRouter — multi-provider failover with circuit breakers.
//!
//! Implements `LlmProvider` so it slots transparently into `JiaCore`
//! without changing the 甲隐于六仪 contract. Internally maintains a
//! priority-ordered provider list with per-provider circuit breakers.

use std::pin::Pin;
use std::sync::Mutex;

use futures::Stream;
use tokio_util::sync::CancellationToken;

use crate::error::ProviderError;
use crate::stems::action::ToolSchema;
use crate::types::Message;

use super::breaker::CircuitBreaker;
use super::{LlmProvider, StreamChunk, SystemPrompt};

/// A priority-ordered list of providers with failover and circuit breakers.
pub(crate) struct ProviderRouter {
    /// (priority, provider) pairs sorted by priority ascending.
    providers: Vec<(u32, Box<dyn LlmProvider>)>,
    /// Per-provider circuit breakers.
    breakers: Mutex<Vec<CircuitBreaker>>,
    /// Index of the currently active provider.
    active: Mutex<usize>,
}

impl ProviderRouter {
    pub fn new(mut providers: Vec<(u32, Box<dyn LlmProvider>)>) -> Self {
        providers.sort_by_key(|(p, _)| *p);
        let n = providers.len();
        Self {
            providers,
            breakers: Mutex::new((0..n).map(|_| CircuitBreaker::new(3, 30)).collect()),
            active: Mutex::new(0),
        }
    }

    /// Record a successful request against the current provider.
    pub(crate) fn record_success(&self) {
        if let Ok(mut breakers) = self.breakers.lock() {
            if let Ok(active) = self.active.lock() {
                if *active < breakers.len() {
                    breakers[*active].record_success();
                }
            }
        }
    }

    /// Record a failure and try to switch to the next healthy provider.
    /// Returns true if a new provider was selected, false if all are exhausted.
    pub(crate) fn try_failover(&self) -> bool {
        let mut breakers = self.breakers.lock().unwrap();
        let mut active = self.active.lock().unwrap();
        let now = std::time::Instant::now();

        // Mark current provider as failed
        if *active < breakers.len() {
            breakers[*active].record_failure();
        }

        // Try next healthy provider
        let start = *active;
        for offset in 1..=self.providers.len() {
            let idx = (start + offset) % self.providers.len();
            if !breakers[idx].is_open(now) {
                *active = idx;
                return true;
            }
        }
        false // all exhausted
    }
}

impl LlmProvider for ProviderRouter {
    fn infer_stream(
        &self,
        messages: Vec<Message>,
        tools: Option<&[ToolSchema]>,
        cancel_token: Option<CancellationToken>,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>> {
        let active = *self.active.lock().unwrap();
        self.providers[active]
            .1
            .infer_stream(messages, tools, cancel_token)
    }

    fn infer_stream_with_system(
        &self,
        messages: Vec<Message>,
        system: SystemPrompt,
        tools: Option<&[ToolSchema]>,
        cancel_token: Option<CancellationToken>,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>> {
        let active = *self.active.lock().unwrap();
        self.providers[active]
            .1
            .infer_stream_with_system(messages, system, tools, cancel_token)
    }

    fn supports_caching(&self) -> bool {
        let active = *self.active.lock().unwrap();
        self.providers[active].1.supports_caching()
    }

    fn as_router(&self) -> Option<&crate::palaces::zhong_core::router::ProviderRouter> {
        Some(self)
    }
}
