//! Backward-compatible LlmRouter wrapper over ProviderRouter.
//! This module re-exports all types from `providers::traits` and delegates
//! all functionality to `ProviderRouter`. Will be removed in Sprint 4.

use anyhow::Result;
use serde_json::Value;
use std::pin::Pin;

// Re-export all types from providers for backward compatibility
pub use crate::providers::traits::{
    ProviderConfig, RouteHint, LlmResponse,
    ChatMessage, ToolCall, ToolCallFunction, TokenUsage, ChatResponse,
    StreamChunk,
};
use std::sync::Arc;
use crate::circuit_breaker::ProviderCircuitBreaker;
use crate::providers::ProviderRouter;
use crate::providers::rotation::ProviderRotation;
use crate::trajectory::TrajectoryLogger;

/// LLM Router — thin wrapper over ProviderRouter for backward compatibility.
/// All methods delegate to the inner ProviderRouter.
pub struct LlmRouter {
    inner: ProviderRouter,
    circuit_breaker: Option<Arc<ProviderCircuitBreaker>>,
    trajectory_logger: Option<Arc<TrajectoryLogger>>,
}

impl LlmRouter {
    /// Create a new LlmRouter, loading config from the given TOML path.
    pub fn new(config_path: &str) -> Result<Self> {
        let inner = ProviderRouter::new(config_path)?;
        Ok(Self { inner, circuit_breaker: None, trajectory_logger: None })
    }

    /// Create LlmRouter from a pre-built ProviderRouter.
    /// Used by test harnesses for programmatic setup without a TOML config file.
    pub fn from_router(router: ProviderRouter) -> Self {
        Self {
            inner: router,
            circuit_breaker: None,
            trajectory_logger: None,
        }
    }

    /// Mutable access to the inner ProviderRouter.
    /// Allows registering providers after construction.
    pub fn inner_mut(&mut self) -> &mut ProviderRouter {
        &mut self.inner
    }

    /// Attach a circuit breaker to the router.
    pub fn set_circuit_breaker(&mut self, cb: Arc<ProviderCircuitBreaker>) {
        self.circuit_breaker = Some(cb);
    }

    /// Get circuit breaker reference (if attached).
    pub fn circuit_breaker(&self) -> Option<&Arc<ProviderCircuitBreaker>> {
        self.circuit_breaker.as_ref()
    }

    /// Attach a trajectory logger to the router for smart routing.
    pub fn set_trajectory_logger(&mut self, tl: Arc<TrajectoryLogger>) {
        self.trajectory_logger = Some(tl);
    }

    /// Choose the best provider based on trajectory quality data.
    /// Returns the provider name that has the best quality/cost ratio
    /// above the given quality threshold. Falls back to the default
    /// provider if insufficient data exists.
    pub async fn smart_route(&self, default_provider: &str, quality_threshold: f64) -> String {
        // If no trajectory logger, just return default
        let logger = match &self.trajectory_logger {
            Some(tl) => tl,
            None => return default_provider.to_string(),
        };

        // Get quality stats
        let stats = match logger.quality_stats() {
            Ok(s) => s,
            Err(_) => return default_provider.to_string(),
        };

        // Filter: must have enough data (>= 5 runs) and meet quality threshold
        let mut candidates: Vec<_> = stats
            .iter()
            .filter(|s| {
                s.total_runs >= 5
                    && s.avg_quality >= quality_threshold
                    && s.success_rate >= 0.7
            })
            .collect();

        if candidates.is_empty() {
            return default_provider.to_string();
        }

        // Check circuit breaker — exclude tripped providers
        if let Some(ref cb) = self.circuit_breaker {
            candidates.retain(|s| cb.is_available(&s.provider));
        }

        if candidates.is_empty() {
            return default_provider.to_string();
        }

        // Sort by quality/cost ratio (higher is better).
        // quality_score is 1-5, cost is in USD.
        // We want high quality and low cost.
        candidates.sort_by(|a, b| {
            let ratio_a = if a.avg_cost > 0.0 {
                a.avg_quality / a.avg_cost
            } else {
                a.avg_quality * 1000.0
            };
            let ratio_b = if b.avg_cost > 0.0 {
                b.avg_quality / b.avg_cost
            } else {
                b.avg_quality * 1000.0
            };
            ratio_b
                .partial_cmp(&ratio_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        candidates[0].provider.clone()
    }

    /// Route a prompt to the specified provider (or auto-detect).
    pub async fn route(&self, prompt: &str, provider: &str) -> Result<String> {
        // Check circuit breaker
        if let Some(ref cb) = self.circuit_breaker {
            if !cb.is_available(provider) {
                let alt = self.inner.provider_names().into_iter()
                    .find(|p| p != provider && cb.is_available(p));
                if let Some(alt_provider) = alt {
                    tracing::info!("Circuit breaker: '{}' is open, routing to '{}'", provider, alt_provider);
                    let result = self.inner.route(prompt, &alt_provider).await;
                    match &result {
                        Ok(_) => cb.record_success(&alt_provider),
                        Err(_) => cb.record_failure(&alt_provider),
                    }
                    return result;
                }
                tracing::warn!("Circuit breaker: '{}' is open but no alternatives, trying anyway", provider);
            }
        }

        let result = self.inner.route(prompt, provider).await;

        // Record success/failure
        if let Some(ref cb) = self.circuit_breaker {
            match &result {
                Ok(_) => cb.record_success(provider),
                Err(_) => cb.record_failure(provider),
            }
        }

        result
    }

    /// Chat with tools support.
    /// Automatically strips `<think>...</think>` tags from response content
    /// (emitted by Qwen, DeepSeek, and other reasoning models).
    /// Checks circuit breaker before calling the provider; routes to an
    /// alternative provider if the requested one is circuit-broken.
    pub async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        provider: &str,
    ) -> Result<ChatResponse> {
        // Check circuit breaker
        if let Some(ref cb) = self.circuit_breaker {
            if !cb.is_available(provider) {
                // Provider is circuit-broken, try to find alternative
                let alt = self.inner.provider_names().into_iter()
                    .find(|p| p != provider && cb.is_available(p));
                if let Some(alt_provider) = alt {
                    tracing::info!("Circuit breaker: '{}' is open, routing to '{}'", provider, alt_provider);
                    let result = self.inner.chat_with_tools(messages, tools, &alt_provider).await;
                    match &result {
                        Ok(_) => cb.record_success(&alt_provider),
                        Err(_) => cb.record_failure(&alt_provider),
                    }
                    let mut resp = result?;
                    resp.message.content = crate::think_filter::strip_think_tags(&resp.message.content);
                    return Ok(resp);
                }
                // No alternative available, try the original provider anyway
                tracing::warn!("Circuit breaker: '{}' is open but no alternatives, trying anyway", provider);
            }
        }

        let result = self.inner.chat_with_tools(messages, tools, provider).await;

        // Record success/failure
        if let Some(ref cb) = self.circuit_breaker {
            match &result {
                Ok(_) => cb.record_success(provider),
                Err(_) => cb.record_failure(provider),
            }
        }

        let mut resp = result?;
        resp.message.content = crate::think_filter::strip_think_tags(&resp.message.content);
        Ok(resp)
    }

    /// Streaming chat — returns a stream of chunks.
    /// Checks circuit breaker before calling the provider; routes to an
    /// alternative provider if the requested one is circuit-broken.
    pub async fn stream_chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        provider: &str,
    ) -> Result<Pin<Box<dyn futures_util::Stream<Item = Result<StreamChunk>> + Send>>> {
        // Check circuit breaker
        if let Some(ref cb) = self.circuit_breaker {
            if !cb.is_available(provider) {
                let alt = self.inner.provider_names().into_iter()
                    .find(|p| p != provider && cb.is_available(p));
                if let Some(alt_provider) = alt {
                    tracing::info!("Circuit breaker: '{}' is open, routing to '{}' (stream)", provider, alt_provider);
                    // Note: success/failure recording for streams happens at chunk level,
                    // but we record the initial connection success/failure here.
                    let result = self.inner.stream_chat(messages, tools, &alt_provider).await;
                    match &result {
                        Ok(_) => cb.record_success(&alt_provider),
                        Err(_) => cb.record_failure(&alt_provider),
                    }
                    return result;
                }
                tracing::warn!("Circuit breaker: '{}' is open but no alternatives, trying anyway (stream)", provider);
            }
        }

        let result = self.inner.stream_chat(messages, tools, provider).await;

        // Record success/failure for the stream connection
        if let Some(ref cb) = self.circuit_breaker {
            match &result {
                Ok(_) => cb.record_success(provider),
                Err(_) => cb.record_failure(provider),
            }
        }

        result
    }

    /// Check if a named provider is alive
    pub async fn is_alive(&self, name: &str) -> bool {
        self.inner.is_alive(name).await
    }

    /// Check if any provider is alive
    pub async fn any_alive(&self) -> bool {
        self.inner.any_alive().await
    }

    /// Check if a provider exists
    pub fn has_provider(&self, name: &str) -> bool {
        self.inner.has_provider(name)
    }

    /// Get all provider names
    pub fn provider_names(&self) -> Vec<String> {
        self.inner.provider_names()
    }

    /// Access the inner ProviderRouter
    pub fn inner(&self) -> &ProviderRouter {
        &self.inner
    }

    /// Attach a rotation engine to the inner ProviderRouter.
    /// Must be called before wrapping in Arc.
    pub fn set_rotation(&mut self, rotation: Arc<ProviderRotation>) {
        self.inner.set_rotation(rotation);
    }

    /// Get rotation engine reference (if attached).
    pub fn rotation(&self) -> Option<&Arc<ProviderRotation>> {
        self.inner.rotation()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ProviderRouter;
    use crate::providers::mock::MockProvider;

    #[test]
    fn test_default_providers_loaded() {
        let router = LlmRouter::new("/nonexistent/path.toml").unwrap();
        assert!(router.has_provider("ollama"));
        assert!(router.has_provider("lmstudio"));
        assert!(router.has_provider("lemonade"));
    }

    #[test]
    fn test_provider_names() {
        let router = LlmRouter::new("/nonexistent/path.toml").unwrap();
        let names = router.provider_names();
        assert!(names.contains(&"ollama".to_string()));
    }

    #[test]
    fn test_unknown_provider_not_found() {
        let router = LlmRouter::new("/nonexistent/path.toml").unwrap();
        assert!(!router.has_provider("nonexistent"));
    }

    #[test]
    fn test_from_router_creates_llm_router() {
        let mut pr = ProviderRouter::empty();
        pr.register_provider("mock", Box::new(MockProvider::fixed("ok")));
        let router = LlmRouter::from_router(pr);
        assert!(router.has_provider("mock"));
    }

    #[test]
    fn test_inner_mut_allows_provider_registration() {
        let pr = ProviderRouter::empty();
        let mut router = LlmRouter::from_router(pr);
        router.inner_mut().register_provider("mock", Box::new(MockProvider::echo()));
        assert!(router.has_provider("mock"));
    }

    #[tokio::test]
    async fn test_from_router_chat_works() {
        let mut pr = ProviderRouter::empty();
        pr.register_provider("mock", Box::new(MockProvider::fixed("e2e response")));
        let router = LlmRouter::from_router(pr);

        let messages = vec![ChatMessage {
            role: "user".into(),
            content: "test".into(),
            tool_calls: None,
            tool_call_id: None,
        }];
        let result = router.chat_with_tools(&messages, &[], "mock").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().message.content, "e2e response");
    }
}
