//! T7 security audit (codex 2026-05-15) — regression + positive tests.
//!
//! Covers:
//!   * /rpc/message — HMAC enforcement (HIGH)
//!   * /api/chat    — HMAC enforcement (HIGH)
//!   * /rpc/task/assign — fail-closed when cluster_secret empty (HIGH)
//!   * tools::file::safe_path — workspace boundary check (HIGH)
//!   * tools::patch::apply — workspace boundary check (HIGH)
//!   * bin/spectyn redact_argv — credential elision (MEDIUM)
//!
//! NOTE: tests that mutate `SPECTYN_ALLOW_EMPTY_CLUSTER_SECRET` /
//! `SPECTYN_EXTRA_ALLOWED_ROOTS` serialise on `env_guard()`. Env vars are
//! process-global, so any test that reads or writes them must take the lock.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use spectyn_mesh::mesh::{ClusterConfig, ClusterManager};
use spectyn_mesh::AppState;
use serde_json::json;
use std::sync::{Arc, Mutex, OnceLock};
use tower::ServiceExt;

/// Serialise env-mutating tests in this file. Cargo test runs threads in
/// parallel by default; env vars are process-global so any test that reads
/// or sets `SPECTYN_ALLOW_EMPTY_CLUSTER_SECRET` / `SPECTYN_EXTRA_ALLOWED_ROOTS`
/// must hold this lock.
fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

fn app_state_with_secret(secret: &str) -> Arc<AppState> {
    let mut st = AppState::new();
    let mut cfg = ClusterConfig::default();
    cfg.cluster_secret = if secret.is_empty() {
        None
    } else {
        Some(secret.into())
    };
    st.cluster_manager = ClusterManager::new(cfg);
    Arc::new(st)
}

// ── /rpc/message ────────────────────────────────────────────────────────────

#[tokio::test]
async fn rpc_message_rejects_request_without_hmac_when_secret_set() {
    let _g = env_guard();
    std::env::remove_var("SPECTYN_ALLOW_EMPTY_CLUSTER_SECRET");
    let state = app_state_with_secret("topsecret");
    let app = spectyn_mesh::serve::router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/rpc/message")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"message":"hello","agent":"master"}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "missing X-Cluster-Auth must yield 401 when cluster_secret is set"
    );
}

#[tokio::test]
async fn rpc_message_rejects_when_secret_empty_and_no_override() {
    let _g = env_guard();
    std::env::remove_var("SPECTYN_ALLOW_EMPTY_CLUSTER_SECRET");
    let state = app_state_with_secret("");
    let app = spectyn_mesh::serve::router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/rpc/message")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"message":"hello"}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "empty cluster_secret must fail closed (403)"
    );
}

#[tokio::test]
async fn rpc_message_accepts_valid_hmac() {
    let _g = env_guard();
    std::env::remove_var("SPECTYN_ALLOW_EMPTY_CLUSTER_SECRET");
    let state = app_state_with_secret("topsecret");
    let app = spectyn_mesh::serve::router(state.clone());

    let body = r#"{"message":"hello","agent":"master"}"#;
    let token = state.cluster_manager.make_auth_token(body);

    let req = Request::builder()
        .method("POST")
        .uri("/rpc/message")
        .header("content-type", "application/json")
        .header("X-Cluster-Auth", token)
        .body(Body::from(body))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // Auth passes — downstream may return 200 (output) or 200 (error)
    // depending on whether the test env has an LLM configured; what
    // matters is NOT 401/403.
    assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_ne!(resp.status(), StatusCode::FORBIDDEN);
}

// ── /api/chat ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn api_chat_rejects_request_without_hmac_when_secret_set() {
    let _g = env_guard();
    std::env::remove_var("SPECTYN_ALLOW_EMPTY_CLUSTER_SECRET");
    let state = app_state_with_secret("topsecret");
    let app = spectyn_mesh::serve::router(state);

    let mut req = Request::builder()
        .method("POST")
        .uri("/api/chat")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"prompt":"hello"}"#))
        .unwrap();
    // api_chat extracts ConnectInfo<SocketAddr> BEFORE the auth gate. `oneshot`
    // does not run into_make_service_with_connect_info, so without an injected
    // ConnectInfo the extractor 500s (MissingExtension) before auth runs. Use a
    // NON-loopback addr so require_cluster_auth_local_ui does not exempt it and
    // falls through to the strict require_cluster_auth (the remote/gated path).
    req.extensions_mut().insert(axum::extract::ConnectInfo(
        std::net::SocketAddr::from(([203, 0, 113, 1], 9)),
    ));

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "missing X-Cluster-Auth must yield 401 on /api/chat when cluster_secret is set"
    );
}

#[tokio::test]
async fn api_chat_rejects_when_secret_empty_and_no_override() {
    let _g = env_guard();
    std::env::remove_var("SPECTYN_ALLOW_EMPTY_CLUSTER_SECRET");
    let state = app_state_with_secret("");
    let app = spectyn_mesh::serve::router(state);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/chat")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"prompt":"hello"}"#))
        .unwrap();
    // Non-loopback peer → require_cluster_auth_local_ui falls through to the
    // strict gate, which fail-closes (403) on empty cluster_secret w/o override.
    req.extensions_mut().insert(axum::extract::ConnectInfo(
        std::net::SocketAddr::from(([203, 0, 113, 1], 9)),
    ));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "empty cluster_secret must fail closed on /api/chat (403)"
    );
}

#[tokio::test]
async fn api_chat_accepts_loopback_when_override_set() {
    let _g = env_guard();
    std::env::set_var("SPECTYN_ALLOW_EMPTY_CLUSTER_SECRET", "1");
    let state = app_state_with_secret("");
    let app = spectyn_mesh::serve::router(state);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/chat")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"prompt":"hello"}"#))
        .unwrap();
    // Loopback peer: this test exercises the loopback-exempt path, so inject a
    // 127.0.0.1 ConnectInfo (without it the handler 500s before auth and the
    // assert_ne! checks below would pass vacuously).
    req.extensions_mut().insert(axum::extract::ConnectInfo(
        std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
    ));
    let resp = app.oneshot(req).await.unwrap();
    std::env::remove_var("SPECTYN_ALLOW_EMPTY_CLUSTER_SECRET");
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "override should restore legacy access"
    );
    assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── /rpc/task/assign ────────────────────────────────────────────────────────

#[tokio::test]
async fn rpc_task_assign_rejects_when_secret_empty() {
    let _g = env_guard();
    std::env::remove_var("SPECTYN_ALLOW_EMPTY_CLUSTER_SECRET");
    let state = app_state_with_secret("");
    let app = spectyn_mesh::serve::router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/rpc/task/assign")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"agent":"master","prompt":"hello"}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "/rpc/task/assign must fail closed when cluster_secret is empty"
    );
}

#[tokio::test]
async fn rpc_task_assign_rejects_bad_hmac_when_secret_set() {
    let _g = env_guard();
    std::env::remove_var("SPECTYN_ALLOW_EMPTY_CLUSTER_SECRET");
    let state = app_state_with_secret("topsecret");
    let app = spectyn_mesh::serve::router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/rpc/task/assign")
        .header("content-type", "application/json")
        .header("X-Cluster-Auth", "deadbeef")
        .body(Body::from(r#"{"agent":"master","prompt":"hello"}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rpc_task_assign_accepts_valid_hmac() {
    let _g = env_guard();
    std::env::remove_var("SPECTYN_ALLOW_EMPTY_CLUSTER_SECRET");
    // Hermetic isolation (sprint6): `/rpc/task/assign` runs a file-backed
    // at-most-once dedup (default `~/.spectyn-mesh/idempotency.jsonl`) BEFORE the
    // 202 spawn. Without a fresh ledger this test passes the first time, then
    // every re-run sees `(master, hello)` as a Duplicate and gets 200 (deduped)
    // instead of 202 — which made it fail under the SPEC-60 V8 ship-gate collector
    // (the collector re-runs it). Point the ledger at a unique temp file so a
    // valid-HMAC NEW task always takes the 202 path. (`SPECTYN_IDEMPOTENCY_STORE`
    // is the documented test override; env mutation is serialised by env_guard.)
    let idem_dir = tempfile::tempdir().unwrap();
    std::env::set_var("SPECTYN_IDEMPOTENCY_STORE", idem_dir.path().join("idempotency.jsonl"));
    let state = app_state_with_secret("topsecret");
    let app = spectyn_mesh::serve::router(state.clone());

    let body = r#"{"agent":"master","prompt":"hello"}"#;
    let token = state.cluster_manager.make_auth_token(body);

    let req = Request::builder()
        .method("POST")
        .uri("/rpc/task/assign")
        .header("content-type", "application/json")
        .header("X-Cluster-Auth", token)
        .body(Body::from(body))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // 202 ACCEPTED is the documented success code (handler spawns task).
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    std::env::remove_var("SPECTYN_IDEMPOTENCY_STORE");
}

// ── tools::file::safe_path workspace boundary ───────────────────────────────

use spectyn_mesh::tools::file as ph_file;
use tempfile::tempdir;

#[test]
fn safe_path_rejects_etc_passwd_when_outside_allowed_roots() {
    let _g = env_guard();
    std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
    // Use an OS-specific outside path that is guaranteed not to be inside CWD
    // or $HOME/.spectyn-mesh on a normal dev box.
    let outside = if cfg!(windows) {
        "C:\\Windows\\System32\\drivers\\etc\\hosts"
    } else {
        "/etc/passwd"
    };
    let r = ph_file::safe_path(outside);
    assert!(r.is_err(), "{outside} must be rejected by safe_path");
    let err = r.unwrap_err().to_string();
    assert!(
        err.to_lowercase().contains("outside") || err.to_lowercase().contains("workspace"),
        "error should mention workspace boundary, got: {err}"
    );
}

#[test]
fn safe_path_rejects_dotdot_escape() {
    let _g = env_guard();
    std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
    // `../../../../../../etc/passwd` from any reasonable CWD lands at /etc/passwd.
    let r = ph_file::safe_path("../../../../../../etc/passwd");
    if let Ok(p) = &r {
        assert_ne!(
            p,
            std::path::Path::new("/etc/passwd"),
            "safe_path resolved to /etc/passwd via .. — boundary check failed"
        );
    }
    // Either Err, or a resolved path inside CWD (whichever the implementation chooses).
}

#[test]
fn safe_path_accepts_path_inside_cwd() {
    let _g = env_guard();
    std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
    // Cargo.toml exists in the crate root which IS the CWD during cargo test.
    let r = ph_file::safe_path("Cargo.toml");
    assert!(
        r.is_ok(),
        "Cargo.toml inside CWD must be accepted, got: {r:?}"
    );
}

#[test]
fn safe_path_accepts_path_inside_tempdir_when_listed_as_extra_root() {
    let _g = env_guard();
    let tmp = tempdir().unwrap();
    let target = tmp.path().join("inside.txt");
    std::fs::write(&target, "ok").unwrap();

    // Add the tempdir as an extra allowed root.
    let prev = std::env::var("SPECTYN_EXTRA_ALLOWED_ROOTS").ok();
    std::env::set_var(
        "SPECTYN_EXTRA_ALLOWED_ROOTS",
        tmp.path().to_string_lossy().to_string(),
    );
    let r = ph_file::safe_path(target.to_str().unwrap());
    if let Some(v) = prev {
        std::env::set_var("SPECTYN_EXTRA_ALLOWED_ROOTS", v);
    } else {
        std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
    }

    assert!(
        r.is_ok(),
        "tempdir path with extra-allowed-roots must succeed, got {r:?}"
    );
}

#[test]
fn safe_path_rejects_absolute_outside_tempdir() {
    let _g = env_guard();
    let tmp = tempdir().unwrap();
    // Allow only tmp; then ask for something definitively outside CWD,
    // ~/.spectyn-mesh, AND that tempdir.
    std::env::set_var(
        "SPECTYN_EXTRA_ALLOWED_ROOTS",
        tmp.path().to_string_lossy().to_string(),
    );
    let outside = if cfg!(windows) {
        "C:\\Windows\\System32\\drivers\\etc\\hosts"
    } else {
        "/etc/hostname"
    };
    let r = ph_file::safe_path(outside);
    std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
    assert!(
        r.is_err(),
        "{outside} must be rejected with only tempdir allowed"
    );
}

// ── tools::patch::apply workspace boundary ──────────────────────────────────

use spectyn_mesh::tools::patch as ph_patch;

#[tokio::test]
async fn patch_apply_rejects_absolute_path_outside_workspace() {
    let _g = env_guard();
    std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
    let bad_target = if cfg!(windows) {
        "C:/Windows/System32/drivers/etc/hosts"
    } else {
        "/etc/passwd"
    };
    let bad_patch = format!(
        "--- a{p}\n+++ {p}\n@@ -1,1 +1,1 @@\n-pwned\n+pwned\n",
        p = bad_target
    );
    let r = ph_patch::apply(&json!({ "patch": bad_patch, "dry_run": false })).await;
    let rl = r.to_lowercase();
    // T7 boundary check must produce an explicit "workspace"/"outside" reject,
    // NOT a vague "Error reading ..." filesystem error.
    assert!(
        rl.contains("outside") || rl.contains("workspace") || rl.contains("rejected"),
        "patch with absolute {bad_target} target must be rejected by workspace-boundary check, got: {r}"
    );
    // And the real /etc/passwd must not have been touched (best-effort).
    if std::path::Path::new("/etc/passwd").exists() {
        let pw = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
        assert!(
            !pw.contains("pwned"),
            "/etc/passwd was modified — boundary check absent in patch::apply!"
        );
    }
}

#[tokio::test]
async fn patch_apply_rejects_dotdot_traversal() {
    let _g = env_guard();
    std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
    let bad_patch = "\
--- a/../../../../../../etc/hostname
+++ b/../../../../../../etc/hostname
@@ -1,1 +1,1 @@
-x
+y
";
    let r = ph_patch::apply(&json!({ "patch": bad_patch, "dry_run": false })).await;
    let rl = r.to_lowercase();
    assert!(
        rl.contains("outside") || rl.contains("workspace") || rl.contains("rejected"),
        "patch with .. traversal target must be rejected by workspace-boundary check, got: {r}"
    );
}

#[tokio::test]
async fn patch_apply_accepts_path_inside_workspace() {
    let _g = env_guard();
    let dir = tempdir().unwrap();
    let path = dir.path().join("hello.txt");
    std::fs::write(&path, "foo\n").unwrap();

    // Allow this tempdir for the duration of the test.
    let prev = std::env::var("SPECTYN_EXTRA_ALLOWED_ROOTS").ok();
    std::env::set_var(
        "SPECTYN_EXTRA_ALLOWED_ROOTS",
        dir.path().to_string_lossy().to_string(),
    );

    let ok_patch = "\
--- a/hello.txt
+++ b/hello.txt
@@ -1,1 +1,1 @@
-foo
+bar
";
    let r = ph_patch::apply(&json!({
        "patch": ok_patch,
        "base_dir": dir.path().to_str().unwrap(),
        "dry_run": false,
    }))
    .await;

    if let Some(v) = prev {
        std::env::set_var("SPECTYN_EXTRA_ALLOWED_ROOTS", v);
    } else {
        std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
    }

    assert!(
        r.contains("Patched") || r.contains("1 hunk"),
        "in-workspace patch must succeed, got: {r}"
    );
    let updated = std::fs::read_to_string(&path).unwrap();
    assert_eq!(updated, "bar\n");
}
