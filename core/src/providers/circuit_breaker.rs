//! P0-5 — deterministic provider circuit breaker.
//!
//! Closed → (N consecutive transient failures) → Open → (cooldown elapsed)
//! → HalfOpen → (probe success) → Closed, or (probe failure) → Open.
//! Time is read through `crate::clock::Clock`, so every transition is
//! assertable with a `MockClock` — no real sleeps, no wall-clock flake.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::clock::Clock;

/// The three circuit-breaker states for one provider slug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    /// Normal operation — requests flow; transient failures accumulate.
    Closed,
    /// Tripped — the provider is skipped until the cooldown elapses. Carries
    /// the unix-ms instant the breaker opened so half-open timing is exact.
    Open { opened_at_ms: u64 },
    /// One probe is allowed through. Success → Closed; failure → Open again.
    HalfOpen,
}

/// Tunables. Defaults: open after 3 consecutive transient failures, 60s cooldown.
#[derive(Debug, Clone, Copy)]
pub struct BreakerConfig {
    /// Consecutive transient failures that move Closed → Open. Must be >= 1.
    pub failure_threshold: u32,
    /// Milliseconds in Open before a probe is allowed (Open → HalfOpen).
    pub cooldown_ms: u64,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self { failure_threshold: 3, cooldown_ms: 60_000 }
    }
}

use crate::providers::traits::ProviderError;

/// What the failover loop should DO about one failure, independent of the
/// breaker's open/closed bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// Transient (network / rate-limit / overload). Retry the same provider
    /// per its retry budget; counts toward opening the breaker.
    Retry,
    /// Permanent for THIS provider (auth / model-not-found / context-too-long /
    /// unknown). Skip to the next chain slot; does NOT count toward opening
    /// (a bad key isn't a reason to circuit-break a healthy upstream).
    Failover,
    /// Caller-fatal — abort the whole call, no failover. Reserved for inputs
    /// no provider can serve (none today; kept so the match is exhaustive and
    /// future variants have a home).
    Abort,
}

impl FailureKind {
    /// Only transient (`Retry`) failures count toward tripping the breaker.
    pub fn counts_toward_open(self) -> bool {
        matches!(self, FailureKind::Retry)
    }
}

/// Map a `ProviderError` (the existing 6-variant catalog) to a failover
/// decision. Pure — this is the single place the breaker reads error meaning,
/// so retry/failover/abort policy can't drift across call sites.
pub fn classify_failure(err: &ProviderError) -> FailureKind {
    match err {
        ProviderError::NetworkError | ProviderError::RateLimit => FailureKind::Retry,
        ProviderError::AuthError
        | ProviderError::ModelNotFound
        | ProviderError::ContextTooLong
        | ProviderError::Unknown(_) => FailureKind::Failover,
    }
}

/// Per-slug breaker bookkeeping.
#[derive(Debug, Clone, Copy)]
struct BreakerEntry {
    state: BreakerState,
    /// Consecutive transient failures since the last success / since open.
    consecutive_failures: u32,
}

impl Default for BreakerEntry {
    fn default() -> Self {
        Self { state: BreakerState::Closed, consecutive_failures: 0 }
    }
}

/// Deterministic per-process circuit breaker over provider slugs. All time
/// reads go through the passed `&dyn Clock`, so tests drive transitions with a
/// `MockClock`. Interior-mutable (`Mutex`) so the failover loop can share `&self`.
#[derive(Debug)]
pub struct CircuitBreaker {
    config: BreakerConfig,
    entries: Mutex<HashMap<String, BreakerEntry>>,
}

impl CircuitBreaker {
    /// Build a breaker with the given config. `failure_threshold` is clamped to
    /// at least 1 so a misconfigured 0 can never open on a spectyn 0th failure.
    pub fn new(config: BreakerConfig) -> Self {
        let mut config = config;
        if config.failure_threshold == 0 {
            config.failure_threshold = 1;
        }
        Self { config, entries: Mutex::new(HashMap::new()) }
    }

    /// Resolve the *effective* state for `slug` at the clock's "now",
    /// transitioning Open → HalfOpen if the cooldown has elapsed. Mutates the
    /// stored entry on that transition so a HalfOpen probe is granted exactly
    /// once. Returns `Closed` for an unseen slug.
    fn effective_state(&self, slug: &str, clock: &dyn Clock) -> BreakerState {
        let mut guard = self.entries.lock().unwrap();
        let entry = guard.entry(slug.to_string()).or_default();
        if let BreakerState::Open { opened_at_ms } = entry.state {
            let now = clock.now_ms();
            if now.saturating_sub(opened_at_ms) >= self.config.cooldown_ms {
                entry.state = BreakerState::HalfOpen;
            }
        }
        entry.state
    }

    /// Public, read-with-transition view of the state (used by tests + telemetry).
    pub fn state(&self, slug: &str, clock: &dyn Clock) -> BreakerState {
        self.effective_state(slug, clock)
    }

    /// May a request be sent to `slug` right now? `true` for Closed and HalfOpen
    /// (the HalfOpen probe), `false` for Open (still in cooldown).
    pub fn allow(&self, slug: &str, clock: &dyn Clock) -> bool {
        !matches!(self.effective_state(slug, clock), BreakerState::Open { .. })
    }

    /// Record one failure. Only `FailureKind::Retry` (transient) advances the
    /// consecutive count / trips the breaker. A failure while HalfOpen
    /// immediately re-opens (the probe failed). Non-counting kinds are a no-op
    /// against the breaker (the failover loop still advances the chain).
    pub fn on_failure(&self, slug: &str, kind: FailureKind, clock: &dyn Clock) {
        // Settle Open→HalfOpen first so a probe failure re-opens correctly.
        let _ = self.effective_state(slug, clock);
        if !kind.counts_toward_open() {
            return;
        }
        let mut guard = self.entries.lock().unwrap();
        let entry = guard.entry(slug.to_string()).or_default();
        match entry.state {
            BreakerState::HalfOpen => {
                // Probe failed — straight back to Open, restart the cooldown.
                entry.state = BreakerState::Open { opened_at_ms: clock.now_ms() };
                entry.consecutive_failures = self.config.failure_threshold;
            }
            BreakerState::Closed | BreakerState::Open { .. } => {
                entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
                if entry.consecutive_failures >= self.config.failure_threshold {
                    entry.state = BreakerState::Open { opened_at_ms: clock.now_ms() };
                }
            }
        }
    }

    /// Record a success: closes the breaker and clears the failure count.
    /// (A success from a HalfOpen probe is what closes it; a success while
    /// Closed just zeroes the running count.)
    pub fn on_success(&self, slug: &str, clock: &dyn Clock) {
        let _ = self.effective_state(slug, clock);
        let mut guard = self.entries.lock().unwrap();
        let entry = guard.entry(slug.to_string()).or_default();
        entry.state = BreakerState::Closed;
        entry.consecutive_failures = 0;
    }

    /// Test/telemetry helper: forget a slug entirely (back to pristine Closed).
    pub fn reset(&self, slug: &str) {
        self.entries.lock().unwrap().remove(slug);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_match_brief() {
        let c = BreakerConfig::default();
        assert_eq!(c.failure_threshold, 3);
        assert_eq!(c.cooldown_ms, 60_000);
    }

    #[test]
    fn classify_transient_errors_count_toward_breaker() {
        // Network + rate-limit + overload are transient: retry the same provider,
        // and count toward opening the breaker.
        assert_eq!(classify_failure(&ProviderError::NetworkError), FailureKind::Retry);
        assert_eq!(classify_failure(&ProviderError::RateLimit), FailureKind::Retry);
    }

    #[test]
    fn classify_permanent_provider_errors_failover_without_retry() {
        // Bad key / wrong model / oversized context won't fix by retrying THIS
        // provider — fail straight over to the next chain slot.
        assert_eq!(classify_failure(&ProviderError::AuthError), FailureKind::Failover);
        assert_eq!(classify_failure(&ProviderError::ModelNotFound), FailureKind::Failover);
        assert_eq!(classify_failure(&ProviderError::ContextTooLong), FailureKind::Failover);
    }

    #[test]
    fn classify_unknown_is_failover() {
        // Unclassified upstream errors: don't hammer one provider — fail over.
        assert_eq!(
            classify_failure(&ProviderError::Unknown("boom".into())),
            FailureKind::Failover
        );
    }

    #[test]
    fn failure_kind_counts_toward_open_only_for_retry() {
        assert!(FailureKind::Retry.counts_toward_open());
        assert!(!FailureKind::Failover.counts_toward_open());
        assert!(!FailureKind::Abort.counts_toward_open());
    }

    use crate::clock::MockClock;

    fn breaker() -> CircuitBreaker {
        CircuitBreaker::new(BreakerConfig { failure_threshold: 3, cooldown_ms: 60_000 })
    }

    #[test]
    fn unseen_slug_is_allowed_and_closed() {
        let cb = breaker();
        let clock = MockClock::new(1_000);
        assert!(cb.allow("groq", &clock));
        assert_eq!(cb.state("groq", &clock), BreakerState::Closed);
    }

    #[test]
    fn opens_after_n_consecutive_transient_failures() {
        let cb = breaker();
        let clock = MockClock::new(1_000);
        // 2 failures: still closed + allowed.
        cb.on_failure("groq", FailureKind::Retry, &clock);
        cb.on_failure("groq", FailureKind::Retry, &clock);
        assert!(cb.allow("groq", &clock), "below threshold stays allowed");
        assert_eq!(cb.state("groq", &clock), BreakerState::Closed);
        // 3rd failure trips it.
        cb.on_failure("groq", FailureKind::Retry, &clock);
        assert!(!cb.allow("groq", &clock), "threshold reached → Open → blocked");
        assert!(matches!(cb.state("groq", &clock), BreakerState::Open { .. }));
    }

    #[test]
    fn failover_kind_failures_do_not_open_breaker() {
        let cb = breaker();
        let clock = MockClock::new(1_000);
        // 5 permanent (auth) failures must NOT trip the breaker — a bad key is
        // not a reason to circuit-break a healthy upstream.
        for _ in 0..5 {
            cb.on_failure("groq", FailureKind::Failover, &clock);
        }
        assert!(cb.allow("groq", &clock));
        assert_eq!(cb.state("groq", &clock), BreakerState::Closed);
    }

    #[test]
    fn success_resets_consecutive_count() {
        let cb = breaker();
        let clock = MockClock::new(1_000);
        cb.on_failure("groq", FailureKind::Retry, &clock);
        cb.on_failure("groq", FailureKind::Retry, &clock);
        cb.on_success("groq", &clock);            // reset
        cb.on_failure("groq", FailureKind::Retry, &clock);
        cb.on_failure("groq", FailureKind::Retry, &clock);
        assert!(cb.allow("groq", &clock), "count was reset; 2 < 3");
        assert_eq!(cb.state("groq", &clock), BreakerState::Closed);
    }

    #[test]
    fn open_transitions_to_half_open_after_cooldown() {
        let cb = breaker();                       // threshold 3, cooldown 60_000ms
        let clock = MockClock::new(1_000);
        for _ in 0..3 { cb.on_failure("groq", FailureKind::Retry, &clock); }
        assert!(matches!(cb.state("groq", &clock), BreakerState::Open { .. }));
        assert!(!cb.allow("groq", &clock));

        // 59.999s later: still Open (cooldown not yet elapsed).
        clock.advance_ms(59_999);
        assert!(!cb.allow("groq", &clock), "still inside cooldown");
        assert!(matches!(cb.state("groq", &clock), BreakerState::Open { .. }));

        // Cross the 60s boundary: one probe allowed → HalfOpen.
        clock.advance_ms(1);                       // total 60_000ms elapsed
        assert!(cb.allow("groq", &clock), "cooldown elapsed → probe allowed");
        assert_eq!(cb.state("groq", &clock), BreakerState::HalfOpen);
    }

    #[test]
    fn half_open_probe_success_closes_breaker() {
        let cb = breaker();
        let clock = MockClock::new(1_000);
        for _ in 0..3 { cb.on_failure("groq", FailureKind::Retry, &clock); }
        clock.advance_ms(60_000);
        assert_eq!(cb.state("groq", &clock), BreakerState::HalfOpen);
        cb.on_success("groq", &clock);             // probe succeeded
        assert_eq!(cb.state("groq", &clock), BreakerState::Closed);
        assert!(cb.allow("groq", &clock));
    }

    #[test]
    fn half_open_probe_failure_reopens_and_restarts_cooldown() {
        let cb = breaker();
        let clock = MockClock::new(1_000);
        for _ in 0..3 { cb.on_failure("groq", FailureKind::Retry, &clock); }
        clock.advance_ms(60_000);
        assert_eq!(cb.state("groq", &clock), BreakerState::HalfOpen);
        // Probe fails → back to Open; cooldown restarts from NOW (t=61_000).
        cb.on_failure("groq", FailureKind::Retry, &clock);
        assert!(!cb.allow("groq", &clock));
        // 59.999s after the re-open is still Open.
        clock.advance_ms(59_999);
        assert!(!cb.allow("groq", &clock), "second cooldown still running");
        // Crossing the new 60s boundary half-opens again.
        clock.advance_ms(1);
        assert_eq!(cb.state("groq", &clock), BreakerState::HalfOpen);
    }

    use crate::providers::traits::classify_error;

    #[test]
    fn http_status_to_failover_decision_table() {
        // (status, body) → classify_error → classify_failure → expected decision.
        let cases: &[(u16, &str, FailureKind)] = &[
            (429, "Too Many Requests",            FailureKind::Retry),    // rate limit
            (0,   "",                             FailureKind::Retry),    // network
            (401, "Unauthorized",                 FailureKind::Failover), // auth
            (403, "Forbidden",                    FailureKind::Failover), // auth
            (404, r#"{"error":"model not found"}"#, FailureKind::Failover), // model not found
            (400, "context length exceeded the limit", FailureKind::Failover), // context too long
            (500, "internal server error",        FailureKind::Failover), // unknown 5xx → failover
            (404, "page not found",               FailureKind::Failover), // generic 404 → unknown → failover
        ];
        for (status, body, want) in cases {
            let err = classify_error(*status, body);
            let got = classify_failure(&err);
            assert_eq!(
                got, *want,
                "HTTP {status} ({body:?}) classified {err:?} → {got:?}, expected {want:?}"
            );
        }
    }

    #[test]
    fn only_retry_kind_increments_the_breaker_no_others() {
        // Belt-and-suspenders: assert non-Retry kinds are inert against the count
        // through the public on_failure path (not just counts_toward_open()).
        let cb = breaker();
        let clock = MockClock::new(0);
        for kind in [FailureKind::Failover, FailureKind::Abort] {
            let slug = format!("__inert_{:?}__", kind);
            for _ in 0..10 { cb.on_failure(&slug, kind, &clock); }
            assert!(cb.allow(&slug, &clock), "{kind:?} must never open the breaker");
        }
    }
}
