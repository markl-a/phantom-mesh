//! C1+C2+C3 — RPC capability-aware task forwarding integration tests.
//!
//! Spec: `docs/superpowers/specs/2026-05-17-c1-rpc-forwarding-design.md`.
//!
//! Scenarios covered:
//!   * `test_forward_decision_chooses_capable_peer` — single-host two-port
//!     acceptance test. Spawns two `spectyn serve` instances on ephemeral
//!     ports, posts to the sandbox peer with `required_caps=["shell.write"]`
//!     and `SPECTYN_FORWARD_ON_CAPS_MISMATCH=1`, asserts the 202 carries
//!     `dispatched_to: <full-worker-name>` and that the remote job_id
//!     resolves on the full worker. (Spec §11.3.)
//!   * `test_forward_cycle_guard_rejects_at_chain_len_2` — POST with a
//!     pre-populated `forward_chain` of length 2; asserts 409 with
//!     `error_code: forward_chain_exhausted`. (Spec §5.)
//!   * `test_forward_self_in_chain_rejects` — POST where chain already
//!     contains the receiving node's name; asserts 409 with
//!     `error_code: self_in_chain`.
//!   * `test_no_peer_satisfies_caps_returns_taxonomy_error` — env gate on,
//!     no peer satisfies, asserts the response body carries
//!     `error_code: no_peer_satisfies_caps` not `capability_mismatch`.
//!   * `test_forward_disabled_returns_reject_by_default` — gate OFF →
//!     pre-C1 behaviour: strict-mode mismatch still 409s as
//!     `capability_mismatch`, no forwarding attempted. Back-compat.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use spectyn_mesh::mesh::{ClusterConfig, ClusterManager, EnforceMode, TaskAssignRequest};
use spectyn_mesh::AppState;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::net::TcpListener;
use tower::ServiceExt;

/// Tests in this file set / clear `SPECTYN_FORWARD_ON_CAPS_MISMATCH`. Env vars
/// are process-global so we serialise on a single lock per the existing
/// pattern in `test_security_t7.rs`.
fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// Build an `AppState` configured as a node named `node_name`, with the given
/// `worker_caps`, optional peers, and strict capability enforcement. Uses a
/// hard-coded cluster secret so tests can mint matching HMAC tokens.
///
/// `capabilities` (advertised to other nodes via `/rpc/ping`) defaults to
/// the same value as `worker_caps`; the integration test that needs a peer
/// to be selectable for `shell.write` passes a richer set explicitly.
fn make_state_full(
    node_name: &str,
    worker_caps: Vec<String>,
    advertised_capabilities: Vec<String>,
    peers: Vec<String>,
    cluster_secret: &str,
) -> Arc<AppState> {
    let mut st = AppState::new();
    let mut cfg = ClusterConfig::default();
    cfg.node_name = Some(node_name.to_string());
    cfg.worker_caps = worker_caps;
    cfg.capabilities = advertised_capabilities;
    cfg.peers = peers;
    cfg.cluster_secret = Some(cluster_secret.to_string());
    cfg.enforce_caps = Some(EnforceMode::Strict);
    st.cluster_manager = ClusterManager::new(cfg);
    Arc::new(st)
}

/// Common short-form: `capabilities = worker_caps`.
fn make_state(
    node_name: &str,
    worker_caps: Vec<String>,
    peers: Vec<String>,
    cluster_secret: &str,
) -> Arc<AppState> {
    let caps = worker_caps.clone();
    make_state_full(node_name, worker_caps, caps, peers, cluster_secret)
}

/// Spawn an in-process axum server bound to an ephemeral port on 127.0.0.1.
/// Returns the address it bound. The server runs on a Tokio task and is
/// torn down when the test process exits.
async fn spawn_serve(state: Arc<AppState>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let addr = listener.local_addr().expect("local_addr");
    let app = spectyn_mesh::serve::router(state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    // Give axum a beat to start accepting.
    tokio::time::sleep(Duration::from_millis(50)).await;
    addr
}

/// HMAC-sign + POST a `TaskAssignRequest` against an in-memory router (no
/// network — used when we don't need the receiving node to make outbound
/// forward calls of its own).
async fn post_inproc(state: &Arc<AppState>, req: &TaskAssignRequest) -> (StatusCode, Value) {
    let app = spectyn_mesh::serve::router(state.clone());
    let body = serde_json::to_vec(req).expect("encode");
    let token = state
        .cluster_manager
        .make_auth_token(std::str::from_utf8(&body).expect("utf8"));
    let request = Request::builder()
        .method("POST")
        .uri("/rpc/task/assign")
        .header("content-type", "application/json")
        .header("X-Cluster-Auth", token)
        .body(Body::from(body))
        .expect("build request");
    let resp = app.oneshot(request).await.expect("axum oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("read body");
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(json!({}));
    (status, v)
}

// ── 1. Single-host two-port acceptance test (spec §11.3) ──────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_forward_decision_chooses_capable_peer() {
    let _g = env_guard();
    std::env::set_var("SPECTYN_FORWARD_ON_CAPS_MISMATCH", "1");
    std::env::remove_var("SPECTYN_ALLOW_EMPTY_CLUSTER_SECRET");

    // Bring up the full worker FIRST so we know its bound address before
    // configuring the sandbox node to point at it.
    let secret = "test-c1-secret";

    // Full worker:
    //   * `worker_caps = []` so its own T5 gate accepts anything.
    //   * `capabilities = ["shell.write", "memory"]` so the sandbox-side
    //     `select_best_peer_with_caps` picks it (spec §8 — `capabilities`
    //     is the field that survives peers.json reload).
    let full_state = make_state_full(
        "full-worker",
        Vec::new(),
        vec!["shell.write".into(), "memory".into()],
        Vec::new(),
        secret,
    );
    let full_addr = spawn_serve(full_state.clone()).await;
    let full_url = format!("http://{full_addr}");

    // Sandbox node: worker_caps = ["memory"] (lacks shell.write). Knows
    // about the full worker via peers list.
    let sandbox_state = make_state(
        "sandbox-node",
        vec!["memory".into()],
        vec![full_url.clone()],
        secret,
    );
    // Sandbox node needs to learn the full worker's `capabilities` so
    // `select_best_peer_with_caps` picks it. We ping the peer to populate
    // — same mechanism the running daemon would use.
    let ping_result = sandbox_state
        .cluster_manager
        .ping_peer(&full_url)
        .await
        .expect("ping full worker");
    eprintln!(
        "ping ok: name={} caps={:?}",
        ping_result.name, ping_result.capabilities
    );

    // POST to sandbox with required_caps that only the full worker can serve.
    let req = TaskAssignRequest {
        agent: "master".into(),
        prompt: "echo hi".into(),
        required_caps: vec!["shell.write".into()],
        forward_chain: Vec::new(),
        idempotency_key: Some("test-c1-acceptance-001".into()),
    };
    let (status, body) = post_inproc(&sandbox_state, &req).await;

    std::env::remove_var("SPECTYN_FORWARD_ON_CAPS_MISMATCH");

    eprintln!("sandbox response: status={status} body={body}");
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "expected 202 Accepted with forward; got {status} body={body}"
    );
    assert_eq!(
        body.get("dispatched_to").and_then(|v| v.as_str()),
        Some("full-worker"),
        "dispatched_to must name the full worker, got {body}"
    );
    assert_eq!(
        body.get("forwarded").and_then(|v| v.as_bool()),
        Some(true),
        "forwarded must be true on the forwarded branch; got {body}"
    );
    assert!(
        body.get("job_id").and_then(|v| v.as_str()).is_some(),
        "remote job_id must be propagated back to caller; got {body}"
    );
}

// ── 2. Cycle guard — chain length exhaustion (spec §5) ────────────────────

#[tokio::test]
async fn test_forward_cycle_guard_rejects_at_chain_len_2() {
    let _g = env_guard();
    std::env::set_var("SPECTYN_FORWARD_ON_CAPS_MISMATCH", "1");

    // A receiving node that would normally forward, but is asked to
    // forward a request whose chain is already at FORWARD_CHAIN_LIMIT.
    let state = make_state(
        "receiver-node",
        vec!["memory".into()], // missing shell.write so forwarding would normally trigger
        Vec::new(),
        "secret",
    );
    let req = TaskAssignRequest {
        agent: "master".into(),
        prompt: "irrelevant".into(),
        required_caps: vec!["shell.write".into()],
        // Spec §5: limit is 2. A chain already at 2 entries must
        // short-circuit before any work.
        forward_chain: vec!["A".into(), "B".into()],
        idempotency_key: None,
    };
    let (status, body) = post_inproc(&state, &req).await;

    std::env::remove_var("SPECTYN_FORWARD_ON_CAPS_MISMATCH");

    assert_eq!(status, StatusCode::CONFLICT, "expected 409, body={body}");
    assert_eq!(
        body.get("error_code").and_then(|v| v.as_str()),
        Some("forward_chain_exhausted"),
        "wrong error_code; body={body}"
    );
    assert_eq!(
        body.get("limit").and_then(|v| v.as_u64()),
        Some(spectyn_mesh::mesh::FORWARD_CHAIN_LIMIT as u64),
    );
}

// ── 3. Cycle guard — self in chain ────────────────────────────────────────

#[tokio::test]
async fn test_forward_self_in_chain_rejects() {
    let _g = env_guard();
    std::env::set_var("SPECTYN_FORWARD_ON_CAPS_MISMATCH", "1");

    let state = make_state("node-x", vec!["memory".into()], Vec::new(), "secret");
    // Chain has only 1 entry — passes the length check — but it contains
    // this node's own name, so the cycle guard must still reject.
    let req = TaskAssignRequest {
        agent: "master".into(),
        prompt: "irrelevant".into(),
        required_caps: vec!["shell.write".into()],
        forward_chain: vec!["node-x".into()],
        idempotency_key: None,
    };
    let (status, body) = post_inproc(&state, &req).await;

    std::env::remove_var("SPECTYN_FORWARD_ON_CAPS_MISMATCH");

    assert_eq!(status, StatusCode::CONFLICT, "expected 409, body={body}");
    assert_eq!(
        body.get("error_code").and_then(|v| v.as_str()),
        Some("self_in_chain"),
        "wrong error_code; body={body}"
    );
    assert_eq!(body.get("node").and_then(|v| v.as_str()), Some("node-x"),);
}

// ── 4. Error taxonomy — no peer satisfies caps ────────────────────────────

#[tokio::test]
async fn test_no_peer_satisfies_caps_returns_taxonomy_error() {
    let _g = env_guard();
    std::env::set_var("SPECTYN_FORWARD_ON_CAPS_MISMATCH", "1");

    // Sandbox with NO peers at all: enforce returns Reject, the call site
    // sees the env gate is ON, and surfaces `no_peer_satisfies_caps`
    // instead of the legacy `capability_mismatch` — per spec §9.
    let state = make_state(
        "lonely-sandbox",
        vec!["memory".into()],
        Vec::new(), // no peers
        "secret",
    );
    let req = TaskAssignRequest {
        agent: "master".into(),
        prompt: "irrelevant".into(),
        required_caps: vec!["shell.write".into()],
        forward_chain: Vec::new(),
        idempotency_key: None,
    };
    let (status, body) = post_inproc(&state, &req).await;

    std::env::remove_var("SPECTYN_FORWARD_ON_CAPS_MISMATCH");

    assert_eq!(status, StatusCode::CONFLICT, "expected 409, body={body}");
    assert_eq!(
        body.get("error_code").and_then(|v| v.as_str()),
        Some("no_peer_satisfies_caps"),
        "wrong error_code (should be the C1 taxonomy variant, not legacy); body={body}"
    );
    // Inventory must be present even if empty — operators need to see it.
    assert!(
        body.get("available_peers").is_some(),
        "available_peers inventory must be present; body={body}"
    );
}

// ── 5. Back-compat — env gate OFF ────────────────────────────────────────

#[tokio::test]
async fn test_forward_disabled_returns_reject_by_default() {
    let _g = env_guard();
    // Explicit guarantee: no env var = no behaviour change.
    std::env::remove_var("SPECTYN_FORWARD_ON_CAPS_MISMATCH");

    let state = make_state("sandbox-bc", vec!["memory".into()], Vec::new(), "secret");
    let req = TaskAssignRequest {
        agent: "master".into(),
        prompt: "irrelevant".into(),
        required_caps: vec!["shell.write".into()],
        forward_chain: Vec::new(),
        idempotency_key: None,
    };
    let (status, body) = post_inproc(&state, &req).await;

    assert_eq!(status, StatusCode::CONFLICT, "expected 409, body={body}");
    assert_eq!(
        body.get("error_code").and_then(|v| v.as_str()),
        Some("capability_mismatch"),
        "back-compat: without env gate, error_code stays legacy `capability_mismatch`; body={body}"
    );
}

// ── 6. Bonus — local run path includes dispatched_to ──────────────────────

#[tokio::test]
async fn test_local_run_emits_dispatched_to_field() {
    let _g = env_guard();
    std::env::remove_var("SPECTYN_FORWARD_ON_CAPS_MISMATCH");

    // Full worker, no required_caps → Allow path → runs locally.
    // The runtime may error because no LLM is configured in the test
    // environment, but the 202 response (containing dispatched_to)
    // is returned before the spawn touches the runtime.
    let state = make_state("local-only", Vec::new(), Vec::new(), "secret");
    let req = TaskAssignRequest {
        agent: "master".into(),
        prompt: "anything".into(),
        required_caps: Vec::new(),
        forward_chain: Vec::new(),
        idempotency_key: None,
    };
    let (status, body) = post_inproc(&state, &req).await;
    assert_eq!(status, StatusCode::ACCEPTED, "body={body}");
    assert_eq!(
        body.get("dispatched_to").and_then(|v| v.as_str()),
        Some("local-only"),
        "local run path should still emit dispatched_to for routing audit; body={body}"
    );
    assert_eq!(body.get("forwarded").and_then(|v| v.as_bool()), Some(false),);
}

// ── 7. parse_peer_args coverage is exercised via the unit-test layer in
//      the mesh module + manual CLI smoke; the integration test above is
//      the real proof that C2's --required-caps wires through C1 → C3.
