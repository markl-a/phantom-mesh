// Wave H1.1 — Tauri command surface for SPEC-28 onboarding FSM (wire layer).
//
// Wraps `phantom_mesh::onboarding_wire` so the React onboarding flow (H2.1)
// can drive the 6-state machine through Tauri's `invoke` channel. Four of the
// five core fns are `unimplemented!()` pending SPEC-28 Stage 3 (FSM body /
// OTEL emit / reqwest seam); we use `std::panic::catch_unwind` to translate
// those panics into a stable wire-error string instead of crashing the worker.

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

#[tauri::command]
pub async fn onboarding_advance(
    snapshot: OnboardingStateSnapshot,
    ctx: OnboardingContext,
) -> Result<OnboardingState, String> {
    run_or_unimplemented(|| onboarding_wire::advance(&snapshot, &ctx))
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
        };
        assert_eq!(onboarding_should_fallback_to_demo_relay(ctx).await, Ok(false));
    }

    #[tokio::test]
    async fn advance_returns_not_yet_wired_error_when_core_unimplemented() {
        let snap = OnboardingStateSnapshot {
            current_state: OnboardingState::FreshInstall,
            entered_at_ms: 0,
            retry_count: 0,
            last_error: None,
        };
        let ctx = OnboardingContext::default();
        let err = onboarding_advance(snap, ctx).await.unwrap_err();
        assert!(err.starts_with("onboarding.not_yet_wired"), "got {err}");
    }

    #[tokio::test]
    async fn rollback_returns_not_yet_wired_error_when_core_unimplemented() {
        let snap = OnboardingStateSnapshot {
            current_state: OnboardingState::JoinedCluster,
            entered_at_ms: 0,
            retry_count: 0,
            last_error: None,
        };
        let err = onboarding_rollback(snap).await.unwrap_err();
        assert!(err.starts_with("onboarding.not_yet_wired"), "got {err}");
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
