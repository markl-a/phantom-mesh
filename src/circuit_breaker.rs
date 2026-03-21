// circuit_breaker.rs — Provider Circuit Breaker pattern for LLM providers.
//
// When a provider fails repeatedly, it gets "tripped" (Open state) and
// requests are blocked until it recovers via a HalfOpen probing phase.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// CircuitState
// ---------------------------------------------------------------------------

/// The three states of a circuit breaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation — requests flow through.
    Closed,
    /// Tripped — requests are blocked until `open_duration_secs` elapses.
    Open,
    /// Probing — a limited number of requests are allowed through to test
    /// whether the provider has recovered.
    HalfOpen,
}

impl CircuitState {
    /// Return a lowercase string representation suitable for serialization.
    pub fn as_str(&self) -> &'static str {
        match self {
            CircuitState::Closed => "closed",
            CircuitState::Open => "open",
            CircuitState::HalfOpen => "half_open",
        }
    }
}

// ---------------------------------------------------------------------------
// BreakerConfig
// ---------------------------------------------------------------------------

/// Configuration knobs for a circuit breaker.
#[derive(Debug, Clone)]
pub struct BreakerConfig {
    /// How many consecutive failures are required to trip the breaker (Closed -> Open).
    pub failure_threshold: u32,
    /// How many seconds the breaker stays Open before transitioning to HalfOpen.
    pub open_duration_secs: u64,
    /// How many consecutive successes are needed in HalfOpen to close the breaker.
    pub half_open_success_needed: u32,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            open_duration_secs: 60,
            half_open_success_needed: 2,
        }
    }
}

// ---------------------------------------------------------------------------
// BreakerState (internal per-provider state)
// ---------------------------------------------------------------------------

/// Internal mutable state tracked per provider.
#[derive(Debug)]
struct BreakerState {
    state: CircuitState,
    failure_count: u32,
    /// Consecutive successes counted while in HalfOpen.
    success_count: u32,
    last_failure_at: Option<Instant>,
    last_state_change: Instant,
    /// Lifetime counter of how many times this provider has been tripped.
    total_trips: u64,
}

impl BreakerState {
    fn new() -> Self {
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            success_count: 0,
            last_failure_at: None,
            last_state_change: Instant::now(),
            total_trips: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// CircuitStatus (serializable snapshot for API)
// ---------------------------------------------------------------------------

/// A point-in-time snapshot of a single provider's circuit breaker, suitable
/// for returning from an HTTP status endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct CircuitStatus {
    pub provider: String,
    pub state: String,
    pub failure_count: u32,
    pub total_trips: u64,
    pub time_in_state_secs: u64,
}

// ---------------------------------------------------------------------------
// ProviderCircuitBreaker
// ---------------------------------------------------------------------------

/// Thread-safe circuit breaker that manages per-provider states.
///
/// Uses `std::sync::Mutex` (not `tokio::sync::Mutex`) because all critical
/// sections are fast, in-memory map look-ups with no `.await` points.
pub struct ProviderCircuitBreaker {
    states: Mutex<HashMap<String, BreakerState>>,
    config: BreakerConfig,
}

impl ProviderCircuitBreaker {
    /// Create a new circuit breaker with the given configuration.
    pub fn new(config: BreakerConfig) -> Self {
        Self {
            states: Mutex::new(HashMap::new()),
            config,
        }
    }

    // -- record_success -------------------------------------------------------

    /// Record a successful request to `provider`.
    ///
    /// * **Closed** — resets `failure_count` to 0.
    /// * **HalfOpen** — increments `success_count`; transitions to Closed when
    ///   `half_open_success_needed` consecutive successes are reached.
    /// * **Open** — no-op (requests should not be reaching here).
    pub fn record_success(&self, provider: &str) {
        let mut states = self.states.lock().expect("circuit_breaker lock poisoned");
        let entry = states
            .entry(provider.to_string())
            .or_insert_with(BreakerState::new);

        match entry.state {
            CircuitState::Closed => {
                entry.failure_count = 0;
                debug!(provider, "circuit breaker: success recorded (Closed)");
            }
            CircuitState::HalfOpen => {
                entry.success_count += 1;
                debug!(
                    provider,
                    success_count = entry.success_count,
                    needed = self.config.half_open_success_needed,
                    "circuit breaker: success recorded (HalfOpen)"
                );
                if entry.success_count >= self.config.half_open_success_needed {
                    info!(provider, "circuit breaker: HalfOpen -> Closed (recovered)");
                    entry.state = CircuitState::Closed;
                    entry.failure_count = 0;
                    entry.success_count = 0;
                    entry.last_state_change = Instant::now();
                }
            }
            CircuitState::Open => {
                // Shouldn't happen — callers should check is_available first.
                debug!(provider, "circuit breaker: success recorded while Open (ignored)");
            }
        }
    }

    // -- record_failure -------------------------------------------------------

    /// Record a failed request to `provider`.
    ///
    /// * **Closed** — increments `failure_count`; trips to Open when threshold
    ///   is reached.
    /// * **HalfOpen** — immediately trips back to Open (single failure).
    /// * **Open** — no-op.
    pub fn record_failure(&self, provider: &str) {
        let mut states = self.states.lock().expect("circuit_breaker lock poisoned");
        let entry = states
            .entry(provider.to_string())
            .or_insert_with(BreakerState::new);

        match entry.state {
            CircuitState::Closed => {
                entry.failure_count += 1;
                entry.last_failure_at = Some(Instant::now());
                debug!(
                    provider,
                    failure_count = entry.failure_count,
                    threshold = self.config.failure_threshold,
                    "circuit breaker: failure recorded (Closed)"
                );
                if entry.failure_count >= self.config.failure_threshold {
                    warn!(
                        provider,
                        failures = entry.failure_count,
                        "circuit breaker: Closed -> Open (tripped)"
                    );
                    entry.state = CircuitState::Open;
                    entry.last_state_change = Instant::now();
                    entry.total_trips += 1;
                }
            }
            CircuitState::HalfOpen => {
                warn!(provider, "circuit breaker: HalfOpen -> Open (probe failed)");
                entry.state = CircuitState::Open;
                entry.failure_count += 1;
                entry.last_failure_at = Some(Instant::now());
                entry.success_count = 0;
                entry.last_state_change = Instant::now();
                entry.total_trips += 1;
            }
            CircuitState::Open => {
                // Already open; just update the failure timestamp.
                entry.last_failure_at = Some(Instant::now());
                debug!(provider, "circuit breaker: failure recorded while Open (already tripped)");
            }
        }
    }

    // -- is_available ---------------------------------------------------------

    /// Check whether `provider` is available for requests.
    ///
    /// * **Closed** — `true`.
    /// * **Open** — if `open_duration_secs` has elapsed since the state change,
    ///   auto-transitions to HalfOpen and returns `true`; otherwise `false`.
    /// * **HalfOpen** — `true` (probing is allowed).
    pub fn is_available(&self, provider: &str) -> bool {
        let mut states = self.states.lock().expect("circuit_breaker lock poisoned");
        let entry = match states.get_mut(provider) {
            Some(e) => e,
            None => return true, // Never seen => Closed by default.
        };

        match entry.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                let elapsed = entry.last_state_change.elapsed().as_secs();
                if elapsed >= self.config.open_duration_secs {
                    info!(
                        provider,
                        elapsed_secs = elapsed,
                        "circuit breaker: Open -> HalfOpen (timeout elapsed)"
                    );
                    entry.state = CircuitState::HalfOpen;
                    entry.success_count = 0;
                    entry.last_state_change = Instant::now();
                    true
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => true,
        }
    }

    // -- status ---------------------------------------------------------------

    /// Return a snapshot of every tracked provider's circuit breaker status.
    pub fn status(&self) -> HashMap<String, CircuitStatus> {
        let states = self.states.lock().expect("circuit_breaker lock poisoned");
        states
            .iter()
            .map(|(provider, bs)| {
                let cs = CircuitStatus {
                    provider: provider.clone(),
                    state: bs.state.as_str().to_string(),
                    failure_count: bs.failure_count,
                    total_trips: bs.total_trips,
                    time_in_state_secs: bs.last_state_change.elapsed().as_secs(),
                };
                (provider.clone(), cs)
            })
            .collect()
    }

    // -- reset ----------------------------------------------------------------

    /// Manually reset a provider's circuit breaker to Closed.
    pub fn reset(&self, provider: &str) {
        let mut states = self.states.lock().expect("circuit_breaker lock poisoned");
        if let Some(entry) = states.get_mut(provider) {
            info!(provider, "circuit breaker: manually reset to Closed");
            entry.state = CircuitState::Closed;
            entry.failure_count = 0;
            entry.success_count = 0;
            entry.last_state_change = Instant::now();
            // Intentionally preserve total_trips and last_failure_at.
        }
    }
}

impl Default for ProviderCircuitBreaker {
    fn default() -> Self {
        Self::new(BreakerConfig::default())
    }
}

// ---------------------------------------------------------------------------
// PluginModule adapter
// ---------------------------------------------------------------------------

use crate::app_context::AppContext;
use crate::health_check::HealthStatus;
use crate::plugin_bus::PluginModule;
use async_trait::async_trait;
use std::sync::Arc;

/// Wraps ProviderCircuitBreaker as a PluginModule.
///
/// On init, creates the circuit breaker and registers `Arc<ProviderCircuitBreaker>`
/// in AppContext for ProviderRouter and other consumers.
pub struct CircuitBreakerPlugin {
    config: BreakerConfig,
    breaker: std::sync::RwLock<Option<Arc<ProviderCircuitBreaker>>>,
}

impl CircuitBreakerPlugin {
    pub fn new(config: BreakerConfig) -> Self {
        Self {
            config,
            breaker: std::sync::RwLock::new(None),
        }
    }
}

#[async_trait]
impl PluginModule for CircuitBreakerPlugin {
    fn id(&self) -> &str { "circuit-breaker" }
    fn version(&self) -> &str { env!("CARGO_PKG_VERSION") }
    fn capabilities(&self) -> Vec<String> { vec!["provider-reliability".into()] }
    async fn init(&self, ctx: &AppContext) -> anyhow::Result<()> {
        let breaker = Arc::new(ProviderCircuitBreaker::new(self.config.clone()));
        ctx.register(breaker.clone());
        *self.breaker.write().expect("lock poisoned") = Some(breaker);
        Ok(())
    }
    async fn shutdown(&self) -> anyhow::Result<()> {
        *self.breaker.write().expect("lock poisoned") = None;
        Ok(())
    }
    fn health(&self) -> HealthStatus {
        match self.breaker.read().expect("lock poisoned").as_ref() {
            Some(_) => HealthStatus::Healthy,
            None => HealthStatus::Unhealthy,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    /// Helper: create a breaker with very short open duration for tests.
    fn fast_breaker() -> ProviderCircuitBreaker {
        ProviderCircuitBreaker::new(BreakerConfig {
            failure_threshold: 3,
            open_duration_secs: 1, // 1 second for fast tests
            half_open_success_needed: 2,
        })
    }

    #[test]
    fn test_initial_state_closed() {
        let cb = fast_breaker();
        // A provider we've never interacted with should be available.
        assert!(cb.is_available("ollama"));

        // Record one success to create the entry, then verify via status.
        cb.record_success("ollama");
        let snap = cb.status();
        let s = snap.get("ollama").expect("ollama should exist in status");
        assert_eq!(s.state, "closed");
        assert_eq!(s.failure_count, 0);
        assert_eq!(s.total_trips, 0);
    }

    #[test]
    fn test_trip_after_threshold() {
        let cb = fast_breaker();

        // 2 failures should NOT trip (threshold = 3).
        cb.record_failure("openai");
        cb.record_failure("openai");
        assert!(cb.is_available("openai"));

        // 3rd failure trips the breaker.
        cb.record_failure("openai");
        assert!(!cb.is_available("openai"));

        let snap = cb.status();
        let s = snap.get("openai").unwrap();
        assert_eq!(s.state, "open");
        assert_eq!(s.failure_count, 3);
        assert_eq!(s.total_trips, 1);
    }

    #[test]
    fn test_auto_recovery_to_half_open() {
        let cb = fast_breaker();

        // Trip the breaker.
        for _ in 0..3 {
            cb.record_failure("anthropic");
        }
        assert!(!cb.is_available("anthropic"));

        // Wait for open_duration_secs (1s) to elapse.
        thread::sleep(Duration::from_millis(1100));

        // Now is_available should auto-transition to HalfOpen.
        assert!(cb.is_available("anthropic"));

        let snap = cb.status();
        let s = snap.get("anthropic").unwrap();
        assert_eq!(s.state, "half_open");
    }

    #[test]
    fn test_half_open_to_closed() {
        let cb = fast_breaker();

        // Trip and wait for HalfOpen.
        for _ in 0..3 {
            cb.record_failure("gemini");
        }
        thread::sleep(Duration::from_millis(1100));
        assert!(cb.is_available("gemini")); // transitions to HalfOpen

        // One success is not enough (need 2).
        cb.record_success("gemini");
        let snap = cb.status();
        assert_eq!(snap.get("gemini").unwrap().state, "half_open");

        // Second success should close it.
        cb.record_success("gemini");
        let snap = cb.status();
        let s = snap.get("gemini").unwrap();
        assert_eq!(s.state, "closed");
        assert_eq!(s.failure_count, 0);
    }

    #[test]
    fn test_half_open_immediate_trip_on_failure() {
        let cb = fast_breaker();

        // Trip and wait for HalfOpen.
        for _ in 0..3 {
            cb.record_failure("groq");
        }
        thread::sleep(Duration::from_millis(1100));
        assert!(cb.is_available("groq")); // -> HalfOpen

        // One success, then a failure — should trip immediately.
        cb.record_success("groq");
        cb.record_failure("groq");

        assert!(!cb.is_available("groq"));

        let snap = cb.status();
        let s = snap.get("groq").unwrap();
        assert_eq!(s.state, "open");
        // total_trips should be 2 (first trip + HalfOpen re-trip).
        assert_eq!(s.total_trips, 2);
    }

    #[test]
    fn test_is_available_states() {
        let cb = fast_breaker();

        // Unknown provider => available (implicit Closed).
        assert!(cb.is_available("never_seen"));

        // Closed => available.
        cb.record_success("test_prov");
        assert!(cb.is_available("test_prov"));

        // Trip it => Open => not available.
        for _ in 0..3 {
            cb.record_failure("test_prov");
        }
        assert!(!cb.is_available("test_prov"));

        // Wait for HalfOpen => available.
        thread::sleep(Duration::from_millis(1100));
        assert!(cb.is_available("test_prov"));
    }

    #[test]
    fn test_reset() {
        let cb = fast_breaker();

        // Trip the breaker.
        for _ in 0..3 {
            cb.record_failure("reset_me");
        }
        assert!(!cb.is_available("reset_me"));

        // Manual reset.
        cb.reset("reset_me");
        assert!(cb.is_available("reset_me"));

        let snap = cb.status();
        let s = snap.get("reset_me").unwrap();
        assert_eq!(s.state, "closed");
        assert_eq!(s.failure_count, 0);
        // total_trips is preserved.
        assert_eq!(s.total_trips, 1);
    }

    #[test]
    fn test_multiple_providers() {
        let cb = fast_breaker();

        // Trip provider A.
        for _ in 0..3 {
            cb.record_failure("provider_a");
        }

        // Provider B should be unaffected.
        cb.record_success("provider_b");
        assert!(!cb.is_available("provider_a"));
        assert!(cb.is_available("provider_b"));

        let snap = cb.status();
        assert_eq!(snap.get("provider_a").unwrap().state, "open");
        assert_eq!(snap.get("provider_b").unwrap().state, "closed");
    }

    #[test]
    fn test_status_snapshot() {
        let cb = fast_breaker();

        // Create entries for several providers.
        cb.record_success("alpha");
        cb.record_failure("beta");

        for _ in 0..3 {
            cb.record_failure("gamma");
        }

        let snap = cb.status();
        assert_eq!(snap.len(), 3);

        assert_eq!(snap.get("alpha").unwrap().state, "closed");
        assert_eq!(snap.get("alpha").unwrap().failure_count, 0);

        assert_eq!(snap.get("beta").unwrap().state, "closed");
        assert_eq!(snap.get("beta").unwrap().failure_count, 1);

        assert_eq!(snap.get("gamma").unwrap().state, "open");
        assert_eq!(snap.get("gamma").unwrap().failure_count, 3);
        assert_eq!(snap.get("gamma").unwrap().total_trips, 1);

        // time_in_state_secs should be a small non-negative number.
        for (_, cs) in &snap {
            // Just verify it doesn't panic and is reasonable.
            assert!(cs.time_in_state_secs < 10);
        }
    }

    #[tokio::test]
    async fn test_circuit_breaker_plugin_lifecycle() {
        use crate::app_context::AppContext;
        use crate::health_check::HealthStatus;
        use crate::plugin_bus::PluginModule;

        let plugin = CircuitBreakerPlugin::new(BreakerConfig::default());
        let ctx = AppContext::new();

        assert_eq!(plugin.id(), "circuit-breaker");
        assert_eq!(plugin.health(), HealthStatus::Unhealthy); // Not initialized yet

        plugin.init(&ctx).await.unwrap();

        let breaker = ctx.get::<ProviderCircuitBreaker>().unwrap();
        assert!(breaker.is_available("test-provider"));
        assert_eq!(plugin.health(), HealthStatus::Healthy);

        plugin.shutdown().await.unwrap();
        assert_eq!(plugin.health(), HealthStatus::Unhealthy);
    }
}
