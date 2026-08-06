//! apex-④ PHONE-APPROVAL round-trip integration test (REAL HTTP seam, in-process).
//!
//! The drive-loop tests (`apex4_*.rs`) cover the approve/deny/stop decision flow
//! with a MOCK escalator. This file closes the remaining gap: the REAL serve HTTP
//! seam that a phone app actually talks to —
//!   * `POST /rpc/approvals/list` — surfaces the high-risk approvals a governed
//!     run is blocked on (apex-④ decision cards), reading the real pending store.
//!   * `POST /rpc/inbox`          — the channel the phone POSTs its decision on
//!     (`{topic: approval_id, text: "approve"/"deny"/"stop"}`).
//!
//! Both endpoints are exercised through the REAL `spectyn_mesh::serve::router`
//! via `tower::ServiceExt::oneshot` (no network socket), with a genuine
//! `X-Cluster-Auth` HMAC the handlers verify. The decision *resolution* — turning
//! the inbox reply into an `ApprovalDecision` and clearing the pending card — is
//! the production `PhoneEscalator::await_decision` logic, not a stub.
//!
//! What this PROVES: pending high-risk action -> surfaces on `/rpc/approvals/list`
//! -> operator decision submitted on `/rpc/inbox` -> real escalation logic
//! releases (approve) or blocks (deny) the action and removes it from the
//! HTTP-visible store. The test fails if any link breaks (auth, list projection,
//! inbox persistence, decision parsing, or pending-card removal) — it is not
//! vacuous (the deny test even sets the timeout fallback to ApproveOnce, so a
//! decision that failed to parse would time out to ApproveOnce and trip the
//! `== Deny` assertion).
//!
//! What it leaves UNPROVEN at the HTTP layer: there is no separate
//! `/rpc/approvals/decision` route — by design decisions ride `/rpc/inbox`, and a
//! live governed run polls the inbox via `PhoneEscalator`. This test drives that
//! escalator directly rather than through a spawned `spectyn serve` process, so
//! the socket/process lifecycle (bind, TLS, the run-loop thread that calls
//! `await_decision`) is out of scope; everything from the HTTP handler down is
//! the real code path. TEST-ONLY — no production code is modified.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use spectyn_mesh::execution_contract::{ApprovalDecision, RiskLevel};
use spectyn_mesh::governed_run::escalation::{Escalator, PhoneEscalator};
use spectyn_mesh::mesh::{ClusterConfig, ClusterManager};
use spectyn_mesh::notifications::NotificationDispatcher;
use spectyn_mesh::pending_approvals::{list_pending, write_pending, PendingCard};
use spectyn_mesh::AppState;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tempfile::tempdir;
use tokio::runtime::Builder;
use tower::ServiceExt;
use uuid::Uuid;

/// Both handlers resolve their store path from `$HOME` (via `resolve_home_dir`),
/// which is process-global, so the two tests serialise on one lock while each
/// points `HOME` at its own tempdir.
fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// An `AppState` whose cluster manager has a known secret, so the test can mint
/// the matching body-HMAC `X-Cluster-Auth` token the `/rpc/*` handlers require.
fn app_state_with_secret(secret: &str) -> Arc<AppState> {
    let mut st = AppState::new();
    let mut cfg = ClusterConfig::default();
    cfg.cluster_secret = Some(secret.to_string());
    st.cluster_manager = ClusterManager::new(cfg);
    Arc::new(st)
}

/// HMAC-sign-and-POST one request against a fresh router built from `state`.
/// `token` must be `make_auth_token(body)` over the exact `body` bytes sent.
async fn post(state: Arc<AppState>, uri: &str, body: &str, token: String) -> (StatusCode, Value) {
    let app = spectyn_mesh::serve::router(state);
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("X-Cluster-Auth", token)
        .body(Body::from(body.to_string()))
        .expect("build request");
    let resp = app.oneshot(request).await.expect("axum oneshot");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("read body");
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(json!({}));
    (status, v)
}

/// True if the `/rpc/approvals/list` response carries a pending card matching
/// `approval_id` with the expected tool + risk fields.
fn pending_contains(body: &Value, approval_id: &str, tool: &str, risk: &str) -> bool {
    body["pending"].as_array().is_some_and(|cards| {
        cards.iter().any(|c| {
            c["approval_id"] == approval_id && c["tool"] == tool && c["risk"] == risk
        })
    })
}

#[test]
fn phone_approval_roundtrip_approve_releases_action() {
    let _g = env_guard();
    let tmp = tempdir().expect("tempdir must be created");
    std::env::set_var("HOME", tmp.path());

    let rt = Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime must be created");

    let state = app_state_with_secret("phone-rt-secret");
    let approval_id = format!("contract-{}", Uuid::new_v4());
    let task_id = Uuid::new_v4();

    // A governed run is blocked awaiting this high-risk approval.
    write_pending(
        tmp.path(),
        &PendingCard {
            approval_id: approval_id.clone(),
            task_id: task_id.to_string(),
            tool: "Bash".to_string(),
            risk: "execute_high".to_string(),
            reason: "pre-action approval".to_string(),
            created_ms: 1000,
        },
    )
    .expect("seed pending approval must be written");

    // (b) The phone lists what is awaiting a decision — REAL list HTTP handler.
    let list_token = state.cluster_manager.make_auth_token("");
    let (status, body) = rt.block_on(post(state.clone(), "/rpc/approvals/list", "", list_token));
    assert_eq!(
        status,
        StatusCode::OK,
        "approval list endpoint must return 200 before decision"
    );
    assert!(
        pending_contains(&body, &approval_id, "Bash", "execute_high"),
        "approval list must contain the seeded high-risk Bash approval"
    );

    // (c) The phone submits APPROVE — REAL decision-submission HTTP handler.
    let inbox_body = json!({ "from": "phone", "text": "approve", "topic": approval_id }).to_string();
    let inbox_token = state.cluster_manager.make_auth_token(&inbox_body);
    let (status, body) = rt.block_on(post(state.clone(), "/rpc/inbox", &inbox_body, inbox_token));
    assert_eq!(
        status,
        StatusCode::OK,
        "inbox endpoint must return 200 for approve decision"
    );
    assert_eq!(
        body["queued"], true,
        "inbox endpoint must acknowledge the approve decision as queued"
    );

    // The REAL release logic: the governed run's escalator reads the inbox reply
    // the HTTP handler wrote, parses the decision, and clears the pending card.
    let mut escalator = PhoneEscalator::new(
        tmp.path().to_path_buf(),
        NotificationDispatcher::new(),
        rt.handle().clone(),
        task_id,
        "default",
        Duration::from_millis(20),
        Duration::from_secs(5),
        ApprovalDecision::Deny,
    );
    let decision = escalator.await_decision(&approval_id, "Bash", RiskLevel::ExecuteHigh);
    assert_eq!(
        decision,
        ApprovalDecision::ApproveOnce,
        "real escalation logic must map phone text 'approve' to ApproveOnce"
    );

    // (d) The action is released and the pending card is gone from the HTTP store.
    let list_token = state.cluster_manager.make_auth_token("");
    let (status, body) = rt.block_on(post(state.clone(), "/rpc/approvals/list", "", list_token));
    assert_eq!(
        status,
        StatusCode::OK,
        "approval list endpoint must return 200 after approve resolution"
    );
    assert!(
        !pending_contains(&body, &approval_id, "Bash", "execute_high"),
        "resolved approve decision must clear the pending approval from the HTTP-visible store"
    );
    assert!(
        !list_pending(tmp.path())
            .expect("pending approvals must remain readable after approve")
            .iter()
            .any(|card| card.approval_id == approval_id),
        "resolved approve decision must remove the pending card from disk"
    );
}

#[test]
fn phone_approval_roundtrip_deny_blocks_action() {
    let _g = env_guard();
    let tmp = tempdir().expect("tempdir must be created");
    std::env::set_var("HOME", tmp.path());

    let rt = Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime must be created");

    let state = app_state_with_secret("phone-rt-secret");
    let approval_id = format!("contract-{}", Uuid::new_v4());
    let task_id = Uuid::new_v4();

    write_pending(
        tmp.path(),
        &PendingCard {
            approval_id: approval_id.clone(),
            task_id: task_id.to_string(),
            tool: "Bash".to_string(),
            risk: "execute_high".to_string(),
            reason: "pre-action approval".to_string(),
            created_ms: 1000,
        },
    )
    .expect("seed pending approval must be written");

    let list_token = state.cluster_manager.make_auth_token("");
    let (status, body) = rt.block_on(post(state.clone(), "/rpc/approvals/list", "", list_token));
    assert_eq!(
        status,
        StatusCode::OK,
        "approval list endpoint must return 200 before deny decision"
    );
    assert!(
        pending_contains(&body, &approval_id, "Bash", "execute_high"),
        "approval list must contain the seeded high-risk Bash approval before deny"
    );

    let inbox_body = json!({ "from": "phone", "text": "deny", "topic": approval_id }).to_string();
    let inbox_token = state.cluster_manager.make_auth_token(&inbox_body);
    let (status, body) = rt.block_on(post(state.clone(), "/rpc/inbox", &inbox_body, inbox_token));
    assert_eq!(
        status,
        StatusCode::OK,
        "inbox endpoint must return 200 for deny decision"
    );
    assert_eq!(
        body["queued"], true,
        "inbox endpoint must acknowledge the deny decision as queued"
    );

    // Fallback is ApproveOnce on purpose: if 'deny' failed to parse/correlate the
    // escalator would TIME OUT to ApproveOnce, so the `== Deny` assertion below is
    // a genuine check of the decision logic, never a no-op.
    let mut escalator = PhoneEscalator::new(
        tmp.path().to_path_buf(),
        NotificationDispatcher::new(),
        rt.handle().clone(),
        task_id,
        "default",
        Duration::from_millis(20),
        Duration::from_secs(5),
        ApprovalDecision::ApproveOnce,
    );
    let decision = escalator.await_decision(&approval_id, "Bash", RiskLevel::ExecuteHigh);
    assert_eq!(
        decision,
        ApprovalDecision::Deny,
        "real escalation logic must map phone text 'deny' to Deny (action blocked)"
    );

    let list_token = state.cluster_manager.make_auth_token("");
    let (status, body) = rt.block_on(post(state.clone(), "/rpc/approvals/list", "", list_token));
    assert_eq!(
        status,
        StatusCode::OK,
        "approval list endpoint must return 200 after deny resolution"
    );
    assert!(
        !pending_contains(&body, &approval_id, "Bash", "execute_high"),
        "resolved deny decision must clear the pending approval from the HTTP-visible store"
    );
    assert!(
        !list_pending(tmp.path())
            .expect("pending approvals must remain readable after deny")
            .iter()
            .any(|card| card.approval_id == approval_id),
        "resolved deny decision must remove the pending card from disk"
    );
}
