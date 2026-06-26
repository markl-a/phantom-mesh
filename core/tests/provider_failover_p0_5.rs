//! P0-5 — deterministic provider failover, end-to-end through the public
//! circuit-breaker API. Hermetic: a fake provider (`FlakyProvider`) fails a
//! fixed number of times then succeeds; time is a `MockClock`. No real network.

use std::cell::Cell;

use phantom_mesh::clock::MockClock;
use phantom_mesh::providers::circuit_breaker::{
    classify_failure, BreakerConfig, BreakerState, CircuitBreaker, FailureKind,
};
use phantom_mesh::providers::traits::ProviderError;

/// A provider-class fixture: fails `fail_k` times (with `err`), then succeeds.
struct FlakyProvider {
    fail_k: u32,
    seen: Cell<u32>,
    err: ProviderError,
}

impl FlakyProvider {
    fn new(fail_k: u32, err: ProviderError) -> Self {
        Self { fail_k, seen: Cell::new(0), err }
    }
    /// One call. `Ok(())` on success; `Err(cloned err)` while still failing.
    fn call(&self) -> Result<(), ProviderError> {
        let n = self.seen.get();
        self.seen.set(n + 1);
        if n < self.fail_k {
            Err(self.err.clone())
        } else {
            Ok(())
        }
    }
}

/// Drive one slug through the breaker the way the chain walk does: skip if not
/// allowed, attempt, classify the error, record failure/success. Returns the
/// final `Result` of the attempt (or `None` if the breaker blocked it).
fn drive_once(
    cb: &CircuitBreaker,
    slug: &str,
    provider: &FlakyProvider,
    clock: &MockClock,
) -> Option<Result<(), ProviderError>> {
    if !cb.allow(slug, clock) {
        return None; // breaker open → would fail over to next slug
    }
    let res = provider.call();
    match &res {
        Ok(()) => cb.on_success(slug, clock),
        Err(e) => cb.on_failure(slug, classify_failure(e), clock),
    }
    Some(res)
}

#[test]
fn breaker_opens_after_n_transient_failures_then_blocks() {
    let cb = CircuitBreaker::new(BreakerConfig { failure_threshold: 3, cooldown_ms: 60_000 });
    let clock = MockClock::new(0);
    // Always-failing transient provider.
    let p = FlakyProvider::new(u32::MAX, ProviderError::NetworkError);

    // 3 transient failures trip the breaker.
    for _ in 0..3 {
        let r = drive_once(&cb, "groq", &p, &clock);
        assert!(matches!(r, Some(Err(ProviderError::NetworkError))));
    }
    assert!(matches!(cb.state("groq", &clock), BreakerState::Open { .. }));
    // 4th attempt is BLOCKED by the open breaker (failover would happen here).
    assert!(drive_once(&cb, "groq", &p, &clock).is_none());
}

#[test]
fn fixture_fails_k_then_succeeds_recovers_via_half_open() {
    let cb = CircuitBreaker::new(BreakerConfig { failure_threshold: 3, cooldown_ms: 60_000 });
    let clock = MockClock::new(0);
    // Fails exactly 3 times (trips breaker), then would succeed.
    let p = FlakyProvider::new(3, ProviderError::NetworkError);

    for _ in 0..3 {
        let _ = drive_once(&cb, "groq", &p, &clock);
    }
    assert!(matches!(cb.state("groq", &clock), BreakerState::Open { .. }));
    // During cooldown the provider is skipped (even though it WOULD now succeed).
    assert!(drive_once(&cb, "groq", &p, &clock).is_none());
    // After cooldown, the half-open probe goes through and succeeds → Closed.
    clock.advance_ms(60_000);
    let r = drive_once(&cb, "groq", &p, &clock);
    assert!(matches!(r, Some(Ok(()))), "half-open probe should succeed");
    assert_eq!(cb.state("groq", &clock), BreakerState::Closed);
}

#[test]
fn permanent_auth_error_fails_over_without_opening_breaker() {
    let cb = CircuitBreaker::new(BreakerConfig { failure_threshold: 3, cooldown_ms: 60_000 });
    let clock = MockClock::new(0);
    let p = FlakyProvider::new(u32::MAX, ProviderError::AuthError);
    // 5 auth failures: failover decision, but breaker stays Closed + allowed.
    for _ in 0..5 {
        let r = drive_once(&cb, "groq", &p, &clock);
        assert!(matches!(r, Some(Err(ProviderError::AuthError))));
        assert_eq!(classify_failure(&ProviderError::AuthError), FailureKind::Failover);
    }
    assert_eq!(cb.state("groq", &clock), BreakerState::Closed);
    assert!(cb.allow("groq", &clock));
}

/// Simulate the chain walk's terminal contract: when EVERY slug in the chain is
/// either open (breaker) or failing, the walk exhausts. We model the walk's
/// skip/attempt/advance loop and assert it ends in "exhausted" (no Ok).
fn walk(
    cb: &CircuitBreaker,
    chain: &[(&str, &FlakyProvider)],
    clock: &MockClock,
) -> Result<&'static str, &'static str> {
    let mut any_attempted = false;
    for (slug, provider) in chain {
        if !cb.allow(slug, clock) {
            continue; // breaker open → skip to next slug
        }
        any_attempted = true;
        match provider.call() {
            Ok(()) => {
                cb.on_success(slug, clock);
                return Ok("ok");
            }
            Err(e) => cb.on_failure(slug, classify_failure(&e), clock),
        }
    }
    let _ = any_attempted;
    Err("fallback_exhausted")
}

#[test]
fn all_providers_failing_yields_fallback_exhausted() {
    let cb = CircuitBreaker::new(BreakerConfig { failure_threshold: 3, cooldown_ms: 60_000 });
    let clock = MockClock::new(0);
    let a = FlakyProvider::new(u32::MAX, ProviderError::NetworkError);
    let b = FlakyProvider::new(u32::MAX, ProviderError::RateLimit);
    let chain = [("groq", &a), ("openai", &b)];
    // Every pass over the 2-provider chain fails both → terminal exhaustion.
    let result = walk(&cb, &chain, &clock);
    assert_eq!(result, Err("fallback_exhausted"));
}

#[test]
fn open_breaker_skips_slug_and_still_exhausts_when_all_down() {
    let cb = CircuitBreaker::new(BreakerConfig { failure_threshold: 1, cooldown_ms: 60_000 });
    let clock = MockClock::new(0);
    let a = FlakyProvider::new(u32::MAX, ProviderError::NetworkError);
    let b = FlakyProvider::new(u32::MAX, ProviderError::NetworkError);
    let chain = [("groq", &a), ("openai", &b)];
    // First walk trips both breakers (threshold 1).
    assert_eq!(walk(&cb, &chain, &clock), Err("fallback_exhausted"));
    assert!(matches!(cb.state("groq", &clock), BreakerState::Open { .. }));
    assert!(matches!(cb.state("openai", &clock), BreakerState::Open { .. }));
    // Second walk skips BOTH (open) and still exhausts — no provider attempted.
    assert_eq!(walk(&cb, &chain, &clock), Err("fallback_exhausted"));
}
