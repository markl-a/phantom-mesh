// Wave H1.1 — Tauri command surface for SPEC-28 onboarding FSM (wire layer).
//
// Wraps `phantom_mesh::onboarding_wire` so the React onboarding flow can drive
// the GUI D1–D5 state machine through Tauri's `invoke` channel.
//
// `onboarding_advance` now runs the REAL per-edge side-effects (login+identity
// mint, detached `phantom serve` + mDNS advertise, provider detection +
// ranking) via `onboarding_wire::advance_with_effects` — the same functions the
// shipped `phantom` CLI onboarding (a7c5701f) uses. The remaining fns
// (`compute_ttfr` / `start_demo_relay_handoff`) are still `unimplemented!()`
// (TTFR telemetry + SPEC-52 demo relay = Stage 2); we use
// `std::panic::catch_unwind` to translate those panics into a stable
// wire-error string instead of crashing the worker.

use std::panic::{catch_unwind, AssertUnwindSafe};

use phantom_mesh::onboarding_wire::{
    self, DemoRelayHandoff, OnboardingContext, OnboardingError, OnboardingState,
    OnboardingStateSnapshot, TTFRMetric,
};

const NOT_YET_WIRED: &str =
    "onboarding.not_yet_wired: SPEC-28 Stage 3 deferred — core function still unimplemented";

fn err_string(e: OnboardingError) -> String {
    e.to_string()
}

fn run_or_unimplemented<T>(f: impl FnOnce() -> Result<T, OnboardingError>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(err_string(e)),
        Err(_) => Err(NOT_YET_WIRED.to_string()),
    }
}

/// Forward one onboarding step, running the real per-edge side-effects (D1–D5).
///
/// The GUI patches `ctx` with the OAuth login result (`identityProvider` /
/// `identitySub`) from the broker login BEFORE calling this on the
/// `fresh_install` edge, so `login` is `None` here and the side-effect reads
/// the already-folded values from `ctx`. Returns ONLY the next state on the
/// wire (matching the prior contract); the GUI re-reads derived context fields
/// — identity fingerprint, cluster hash, provider slug — via their dedicated
/// status commands (`identity_status`, etc.) when it needs to display them.
#[tauri::command]
pub async fn onboarding_advance(
    snapshot: OnboardingStateSnapshot,
    ctx: OnboardingContext,
) -> Result<OnboardingState, String> {
    match onboarding_wire::advance_with_effects(&snapshot, &ctx, None).await {
        Ok(outcome) => Ok(outcome.next_state),
        Err(e) => Err(err_string(e)),
    }
}

#[tauri::command]
pub async fn onboarding_rollback(
    snapshot: OnboardingStateSnapshot,
) -> Result<OnboardingState, String> {
    run_or_unimplemented(|| onboarding_wire::rollback(&snapshot))
}

#[tauri::command]
pub async fn onboarding_compute_ttfr(
    install_at_ms: u64,
    first_reply_at_ms: u64,
) -> Result<TTFRMetric, String> {
    run_or_unimplemented(|| onboarding_wire::compute_ttfr(install_at_ms, first_reply_at_ms))
}

#[tauri::command]
pub async fn onboarding_should_fallback_to_demo_relay(
    ctx: OnboardingContext,
) -> Result<bool, String> {
    Ok(onboarding_wire::should_fallback_to_demo_relay(&ctx))
}

#[tauri::command]
pub async fn onboarding_start_demo_relay_handoff() -> Result<DemoRelayHandoff, String> {
    run_or_unimplemented(onboarding_wire::start_demo_relay_handoff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn should_fallback_true_when_no_cluster_no_provider() {
        let ctx = OnboardingContext::default();
        assert_eq!(onboarding_should_fallback_to_demo_relay(ctx).await, Ok(true));
    }

    #[tokio::test]
    async fn should_fallback_false_when_provider_set() {
        let ctx = OnboardingContext {
            cluster_id_hash: None,
            identity_fingerprint: None,
            provider_slug: Some("groq".to_string()),
            demo_relay_used: false,
            identity_provider: None,
            identity_sub: None,
        };
        assert_eq!(onboarding_should_fallback_to_demo_relay(ctx).await, Ok(false));
    }

    #[tokio::test]
    async fn should_fallback_false_when_cluster_joined() {
        let ctx = OnboardingContext {
            cluster_id_hash: Some("abc123".to_string()),
            identity_fingerprint: None,
            provider_slug: None,
            demo_relay_used: false,
            identity_provider: None,
            identity_sub: None,
        };
        assert_eq!(onboarding_should_fallback_to_demo_relay(ctx).await, Ok(false));
    }

    #[tokio::test]
    async fn advance_fresh_install_without_login_is_refused() {
        // D1 (login-first): advancing from FreshInstall with no OAuth login in
        // the context is refused with the IdentityCreationFailed wire error —
        // the side-effect is now wired (not the old not_yet_wired panic).
        let snap = OnboardingStateSnapshot {
            current_state: OnboardingState::FreshInstall,
            entered_at_ms: 0,
            retry_count: 0,
            last_error: None,
        };
        let ctx = OnboardingContext::default();
        let err = onboarding_advance(snap, ctx).await.unwrap_err();
        assert!(
            err.contains("identity_creation_failed"),
            "expected login-first refusal, got {err}"
        );
    }

    #[tokio::test]
    async fn rollback_joined_cluster_returns_created_identity() {
        // Rollback is a pure FSM move (no side-effect) — JoinedCluster rolls
        // back to CreatedIdentity (the one sanctioned cancel edge).
        let snap = OnboardingStateSnapshot {
            current_state: OnboardingState::JoinedCluster,
            entered_at_ms: 0,
            retry_count: 0,
            last_error: None,
        };
        let next = onboarding_rollback(snap).await.expect("rollback ok");
        assert_eq!(next, OnboardingState::CreatedIdentity);
    }

    #[tokio::test]
    async fn compute_ttfr_returns_not_yet_wired_error_when_core_unimplemented() {
        let err = onboarding_compute_ttfr(1000, 5000).await.unwrap_err();
        assert!(err.starts_with("onboarding.not_yet_wired"), "got {err}");
    }

    #[tokio::test]
    async fn start_demo_relay_handoff_returns_not_yet_wired_error_when_core_unimplemented() {
        let err = onboarding_start_demo_relay_handoff().await.unwrap_err();
        assert!(err.starts_with("onboarding.not_yet_wired"), "got {err}");
    }
}
