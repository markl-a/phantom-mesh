use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;
use tracing::{debug, info, warn};

use super::traits::*;

/// Error classification for retry decisions
#[derive(Debug, Clone, PartialEq)]
pub enum ErrorClass {
    /// Non-retryable client errors (400, 401, 403, 404, etc.)
    NonRetryable,
    /// Rate limited (429) — retry after backoff
    RateLimited,
    /// Transient server errors (5xx, timeout, network) — retry with backoff
    Transient,
}

/// Classify an error for retry decisions
pub fn classify_error(err: &anyhow::Error) -> ErrorClass {
    let msg = err.to_string().to_lowercase();
    if msg.contains("429") || msg.contains("rate limit") || msg.contains("too many requests")
        || msg.contains("usage limit") || msg.contains("quota") || msg.contains("exceeded your")
    {
        ErrorClass::RateLimited
    } else if msg.contains("401") || msg.contains("403") || msg.contains("404")
        || msg.contains("400") || msg.contains("invalid")
    {
        ErrorClass::NonRetryable
    } else {
        // 5xx, timeouts, connection errors, etc.
        ErrorClass::Transient
    }
}

/// Circuit breaker state
#[derive(Debug)]
struct CircuitBreaker {
    /// Whether the circuit is open (broken)
    open: AtomicBool,
    /// Number of consecutive failures
    failures: AtomicU32,
    /// Timestamp (as millis since epoch) when circuit opened
    opened_at: AtomicU64,
    /// Number of failures before opening circuit
    threshold: u32,
    /// How long the circuit stays open before half-open (allow one probe)
    recovery_time: Duration,
}

impl CircuitBreaker {
    fn new(threshold: u32, recovery_time: Duration) -> Self {
        Self {
            open: AtomicBool::new(false),
            failures: AtomicU32::new(0),
            opened_at: AtomicU64::new(0),
            threshold,
            recovery_time,
        }
    }

    /// Check if the circuit allows requests
    fn allow_request(&self) -> bool {
        if !self.open.load(Ordering::Relaxed) {
            return true;
        }
        // Check if recovery time has passed (half-open)
        let opened = self.opened_at.load(Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        now.saturating_sub(opened) >= self.recovery_time.as_millis() as u64
    }

    fn record_success(&self) {
        self.failures.store(0, Ordering::Relaxed);
        self.open.store(false, Ordering::Relaxed);
    }

    fn record_failure(&self) {
        let count = self.failures.fetch_add(1, Ordering::Relaxed) + 1;
        if count >= self.threshold {
            self.open.store(true, Ordering::Relaxed);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            self.opened_at.store(now, Ordering::Relaxed);
            debug!("Circuit breaker opened after {} failures", count);
        }
    }

    #[allow(dead_code)]
    fn is_open(&self) -> bool {
        self.open.load(Ordering::Relaxed)
    }
}

/// A provider with a fallback chain.
/// Wraps multiple providers with automatic failover, exponential backoff, and circuit breakers.
pub struct ReliableProvider {
    /// Provider chain in priority order
    chain: Vec<Box<dyn Provider>>,
    /// Circuit breakers, one per provider
    breakers: Vec<CircuitBreaker>,
    /// Max retries per provider
    max_retries: u32,
    /// Base backoff duration
    base_backoff: Duration,
}

impl ReliableProvider {
    pub fn new(chain: Vec<Box<dyn Provider>>) -> Self {
        let breakers = chain.iter().map(|_| CircuitBreaker::new(3, Duration::from_secs(60))).collect();
        Self {
            chain,
            breakers,
            max_retries: 2,
            base_backoff: Duration::from_millis(500),
        }
    }

    pub fn with_config(
        chain: Vec<Box<dyn Provider>>,
        max_retries: u32,
        base_backoff: Duration,
        breaker_threshold: u32,
        breaker_recovery: Duration,
    ) -> Self {
        let breakers = chain.iter()
            .map(|_| CircuitBreaker::new(breaker_threshold, breaker_recovery))
            .collect();
        Self {
            chain,
            breakers,
            max_retries,
            base_backoff,
        }
    }

    /// Try each provider in the chain with retries and circuit breaking
    async fn try_chain<F, T>(&self, operation: F) -> Result<T>
    where
        F: Fn(&dyn Provider) -> Pin<Box<dyn std::future::Future<Output = Result<T>> + Send + '_>>,
        T: Send,
    {
        let mut last_error = anyhow!("No providers in fallback chain");

        for (idx, provider) in self.chain.iter().enumerate() {
            let breaker = &self.breakers[idx];

            if !breaker.allow_request() {
                debug!("Provider '{}' circuit breaker open, skipping", provider.name());
                continue;
            }

            for attempt in 0..=self.max_retries {
                if attempt > 0 {
                    let backoff = self.base_backoff * 2u32.pow(attempt - 1);
                    debug!("Retry {} for '{}' after {:?}", attempt, provider.name(), backoff);
                    tokio::time::sleep(backoff).await;
                }

                match operation(provider.as_ref()).await {
                    Ok(result) => {
                        breaker.record_success();
                        if idx > 0 {
                            info!("Fallback to '{}' succeeded", provider.name());
                        }
                        return Ok(result);
                    }
                    Err(e) => {
                        let class = classify_error(&e);
                        warn!(
                            "Provider '{}' attempt {} failed ({:?}): {}",
                            provider.name(), attempt, class, e
                        );

                        match class {
                            ErrorClass::NonRetryable => {
                                // Don't retry, don't open circuit breaker
                                last_error = e;
                                break; // Try next provider
                            }
                            ErrorClass::RateLimited => {
                                breaker.record_failure();
                                last_error = e;
                                if attempt == self.max_retries {
                                    break; // Try next provider
                                }
                                // Extra backoff for rate limits
                                tokio::time::sleep(Duration::from_secs(2)).await;
                            }
                            ErrorClass::Transient => {
                                breaker.record_failure();
                                last_error = e;
                                if attempt == self.max_retries {
                                    break; // Try next provider
                                }
                            }
                        }
                    }
                }
            }
        }

        Err(last_error)
    }
}

#[async_trait]
impl Provider for ReliableProvider {
    fn name(&self) -> &str {
        "reliable"
    }

    fn default_model(&self) -> &str {
        self.chain.first().map(|p| p.default_model()).unwrap_or("unknown")
    }

    fn capabilities(&self) -> ProviderCapabilities {
        // Merge capabilities from all providers
        let mut caps = ProviderCapabilities::default();
        for p in &self.chain {
            let c = p.capabilities();
            caps.streaming = caps.streaming || c.streaming;
            caps.native_tools = caps.native_tools || c.native_tools;
            caps.vision = caps.vision || c.vision;
        }
        caps
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<ChatResponse> {
        let messages = messages.to_vec();
        let tools = tools.to_vec();
        let model = model.to_string();

        self.try_chain(|p: &dyn Provider| {
            let msgs = messages.clone();
            let tls = tools.clone();
            let mdl = model.clone();
            Box::pin(async move { p.chat(&msgs, &tls, &mdl).await })
        }).await
    }

    async fn stream_chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<Pin<Box<dyn futures_util::Stream<Item = Result<StreamChunk>> + Send>>> {
        let messages = messages.to_vec();
        let tools = tools.to_vec();
        let model = model.to_string();

        self.try_chain(|p: &dyn Provider| {
            let msgs = messages.clone();
            let tls = tools.clone();
            let mdl = model.clone();
            Box::pin(async move { p.stream_chat(&msgs, &tls, &mdl).await })
        }).await
    }

    async fn is_alive(&self) -> bool {
        for (idx, provider) in self.chain.iter().enumerate() {
            if self.breakers[idx].allow_request() && provider.is_alive().await {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_rate_limit() {
        let err = anyhow!("HTTP 429 Too Many Requests");
        assert_eq!(classify_error(&err), ErrorClass::RateLimited);
    }

    #[test]
    fn test_classify_rate_limit_message() {
        let err = anyhow!("rate limit exceeded");
        assert_eq!(classify_error(&err), ErrorClass::RateLimited);
    }

    #[test]
    fn test_classify_non_retryable() {
        let err = anyhow!("HTTP 401 Unauthorized");
        assert_eq!(classify_error(&err), ErrorClass::NonRetryable);
    }

    #[test]
    fn test_classify_non_retryable_404() {
        let err = anyhow!("HTTP 404 Not Found");
        assert_eq!(classify_error(&err), ErrorClass::NonRetryable);
    }

    #[test]
    fn test_classify_transient() {
        let err = anyhow!("connection refused");
        assert_eq!(classify_error(&err), ErrorClass::Transient);
    }

    #[test]
    fn test_classify_timeout() {
        let err = anyhow!("request timeout");
        assert_eq!(classify_error(&err), ErrorClass::Transient);
    }

    #[test]
    fn test_circuit_breaker_initial() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(60));
        assert!(cb.allow_request());
        assert!(!cb.is_open());
    }

    #[test]
    fn test_circuit_breaker_opens_after_threshold() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(60));
        cb.record_failure();
        cb.record_failure();
        assert!(!cb.is_open());
        cb.record_failure();
        assert!(cb.is_open());
    }

    #[test]
    fn test_circuit_breaker_resets_on_success() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(60));
        cb.record_failure();
        cb.record_failure();
        cb.record_success();
        assert!(!cb.is_open());
        assert_eq!(cb.failures.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_reliable_provider_name() {
        let chain: Vec<Box<dyn Provider>> = vec![];
        let rp = ReliableProvider::new(chain);
        assert_eq!(rp.name(), "reliable");
    }

    #[test]
    fn test_reliable_provider_default_model_empty() {
        let chain: Vec<Box<dyn Provider>> = vec![];
        let rp = ReliableProvider::new(chain);
        assert_eq!(rp.default_model(), "unknown");
    }
}
