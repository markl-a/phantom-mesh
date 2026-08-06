// CUJ-05 · delete-all with broker wipe — integration test.
//
// 對應 [`docs/cuj/05-export-and-uninstall.md`] Happy path B (export + delete)
// 的 broker-side leg. Verifies that `spectyn data delete --all --yes
// --include-broker` correctly:
//   1. Sends `DELETE /vault/wipe` to the broker URL stored in
//      `~/.spectyn-mesh/broker.json` (auth::load).
//   2. Sends a `VaultWipeRequest` JSON body with `scope="all"` + reason.
//      The broker validates `scope` so this is the request shape contract.
//   3. Handles the 202 accepted response (wipe_id + eta_complete_ts) and
//      surfaces the wipe_id to stdout for support follow-up.
//   4. Treats a 5xx as a failure mode (caller sees stderr + non-zero exit)
//      so a transient broker outage doesn't silently swallow the wipe ask.
//
// **Why a real wiremock**: the broker DELETE invariant is "no plaintext
// ever leaves the client", and the only way to assert that is to read what
// actually went out over the wire. A mock that records the request body
// lets us assert `Content-Type: application/json` + the exact
// `VaultWipeRequest` shape. A stubbed function couldn't catch a future
// regression where someone refactors the client to send the wrong body.
//
// VERIFIES (CUJ-05 Happy path B step 3 + degraded server-down):
//   - `MAC-CUJ05-DEL-002` from docs/test-cases/mac.md v2

use spectyn_mesh::broker_vault_wire::{VaultWipeRequest, VaultWipeResponse};
use spectyn_mesh::cli_config::wipe_broker_vault_now;
use tempfile::TempDir;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Build a fake broker auth state on disk under a temp HOME so the call
/// finds `auth::load()` populated without touching the dev's real
/// `~/.spectyn-mesh/auth.json`.
fn install_fake_auth(home: &std::path::Path, broker_url: &str) {
    let pm = home.join(".spectyn-mesh");
    std::fs::create_dir_all(&pm).expect("mkdir spectyn-mesh");
    // AuthState has several non-#[serde(default)] fields (provider, email,
    // device_id, created_at_ms, last_login_ms); supplying just broker_token
    // + broker_url silently fails to deserialize so auth::load() returns
    // None, which the broker fn surfaces as "not logged in". Include the
    // required spine so the fixture is honoured.
    let auth_json = serde_json::json!({
        "provider": "google",
        "email": "test@phantommesh.io",
        "display_name": "Test",
        "sub": "test-sub-001",
        "avatar_url": null,
        "device_id": "test-device-uuid",
        "created_at_ms": 1_700_000_000_000_i64,
        "last_login_ms": 1_700_000_000_000_i64,
        "broker_token": "test-token-abc",
        "broker_url": broker_url,
    });
    std::fs::write(pm.join("auth.json"), auth_json.to_string()).expect("write auth.json");
}

/// Single sequential #[tokio::test] covering all 3 scenarios.
///
/// **Why one big test instead of three separate ones**: `auth::load()`
/// reads `$HOME/.spectyn-mesh/auth.json`, and `dirs::home_dir()` resolves
/// `$HOME` from the *process-global* env. With three #[tokio::test]
/// functions, Rust's test harness runs them in parallel by default — the
/// `set_var("HOME", ...)` calls race each other and one test's tempdir
/// gets clobbered by another's, surfacing as spurious "not logged in"
/// failures. We could pull in `serial_test = "3"` as a dev-dep to gate
/// each test with `#[serial]`, but a single sequential test is zero new
/// dependencies and just as expressive — each `// Phase N:` block is the
/// old test, run in a known order.
#[tokio::test]
async fn cuj05_include_broker_all_scenarios_sequential() {
    // ───────────────────────────────────────────────────────────────────
    // Phase 1: happy path — 202 from broker, wire-shape contract honoured.
    // ───────────────────────────────────────────────────────────────────
    let home = TempDir::new().expect("tempdir-happy");
    std::env::set_var("HOME", home.path());

    let mock = MockServer::start().await;
    install_fake_auth(home.path(), &mock.uri());

    let canned = VaultWipeResponse {
        wipe_id: "wipe_test_abc123".to_string(),
        eta_complete_ts: 9_999_999_999_000,
    };
    Mock::given(method("DELETE"))
        .and(path("/vault/wipe"))
        .and(header("authorization", "Bearer test-token-abc"))
        .and(header("content-type", "application/json"))
        .respond_with(ResponseTemplate::new(202).set_body_json(&canned))
        .mount(&mock)
        .await;

    let resp = wipe_broker_vault_now("all", Some("test-driven".to_string()))
        .await
        .expect("wipe should succeed against 202 mock");
    assert_eq!(resp.wipe_id, "wipe_test_abc123");
    assert_eq!(resp.eta_complete_ts, 9_999_999_999_000);

    let received = &mock.received_requests().await.expect("mock has requests")[0];
    let req_body: VaultWipeRequest =
        serde_json::from_slice(&received.body).expect("body is VaultWipeRequest JSON");
    assert_eq!(req_body.scope, "all", "scope must be 'all' for full wipe");
    assert!(
        req_body.reason.is_some(),
        "audit-log reason should be non-None"
    );

    drop(mock);

    // ───────────────────────────────────────────────────────────────────
    // Phase 2: broker 5xx surfaces as Err, not silent success.
    // ───────────────────────────────────────────────────────────────────
    let home2 = TempDir::new().expect("tempdir-5xx");
    std::env::set_var("HOME", home2.path());

    let mock = MockServer::start().await;
    install_fake_auth(home2.path(), &mock.uri());

    Mock::given(method("DELETE"))
        .and(path("/vault/wipe"))
        .respond_with(ResponseTemplate::new(503).set_body_string("upstream busy"))
        .mount(&mock)
        .await;

    let result = wipe_broker_vault_now("all", None).await;
    let err = result.expect_err("5xx should propagate as Err, not silent success");
    let msg = err.to_string();
    assert!(
        msg.contains("503") || msg.contains("HTTP"),
        "error should mention status code; got: {}",
        msg
    );

    drop(mock);

    // ───────────────────────────────────────────────────────────────────
    // Phase 3: no `spectyn login` yet → friendly Err, not panic.
    // ───────────────────────────────────────────────────────────────────
    let home3 = TempDir::new().expect("tempdir-no-auth");
    std::env::set_var("HOME", home3.path());
    // Intentionally do NOT install fake auth.

    let result = wipe_broker_vault_now("all", None).await;
    let err = result.expect_err("no auth → Err, not silent success");
    let msg = err.to_string();
    assert!(
        msg.contains("login") || msg.contains("token") || msg.contains("not logged in"),
        "error should mention login state; got: {}",
        msg
    );
}
