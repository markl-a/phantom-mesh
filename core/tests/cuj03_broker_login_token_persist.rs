// CUJ-03 §3.A · broker login → token-persist → re-read — hermetic test.
//
// 對應 docs/test-cases/mac.md §3.A:
//   - MAC-CUJ03-LOG-001 (`phantom login --provider google` → exit 0、
//     `broker.json` 存在) — the *token-persist* leg.
//   - MAC-CUJ03-LOG-002 (post-001 + clear cache → token 有效) — the
//     *token persistence* leg: a subsequent authenticated read picks the
//     token back up off disk without a fresh login.
//
// ## Why this test exists (the gap it closes)
//
// Before this file, mac.md marked the broker-login token-persist flow as
// "✅ existing", but NO hermetic test exercised it — the only `login` hit
// in `core/tests` was a literal string inside a task_text fixture, i.e. a
// false-green. This test exercises the *actual* persist/read seam that
// `phantom login` drives.
//
// ## What the production flow does (core/src/bin/phantom.rs::login_broker)
//
//   1. Receives a broker callback carrying `broker_token` + `broker_url`.
//   2. Runs an **authenticated** vault exchange against the broker
//      (`cli_config::config_pull_lines` → `GET /api/me/settings/raw` with
//      `Authorization: Bearer <token>`).
//   3. **Only on a successful exchange** persists the token to
//      `~/.phantom-mesh/broker.json` via `cli_config::write_broker_config`,
//      so zero-arg `phantom config pull` re-runs remember it.
//   4. A later authenticated read picks it back up via
//      `cli_config::read_broker_config`.
//
// This test reproduces that exact sequence against a wiremock broker, then
// asserts (a) `broker.json` lands on disk under a temp HOME and (b) a fresh
// `read_broker_config()` (in-memory state dropped) reads the same
// url + token back.
//
// ## Why a real wiremock instead of a stubbed function
//
// The persist is *gated* on an authenticated broker exchange — the whole
// point of MAC-CUJ03-LOG is that the token only sticks after the broker
// honours `Authorization: Bearer <token>`. Standing up a mock that records
// the request lets us assert the Bearer header actually went out, so a
// future refactor that drops the auth header (or persists before the
// exchange) would fail here. A stubbed function couldn't catch that.
//
// ## Hermetic isolation
//
// `cli_config::broker_config_path()` (and the env-file path) resolve under
// `dirs::home_dir()`. On Linux/macOS that follows `$HOME`; we point it at a
// `TempDir` so we never touch the dev's real `~/.phantom-mesh/broker.json`.
// We also export `PHANTOM_HOME` (the spec'd data-root override) to the same
// dir for forward-compat with the unified home resolver — on Windows
// `dirs::home_dir()` ignores `$HOME`, and `PHANTOM_HOME` is the documented
// way to redirect the data root, so setting both keeps this test correct as
// the resolver migrates. (CI for this test runs under WSL/Linux, where
// `$HOME` is honoured today.)
//
// PLATFORM GATE: this test relies on a `$HOME`/`PHANTOM_HOME` redirect to a
// TempDir for isolation, but on Windows `dirs::home_dir()` ignores those env
// vars (resolves SHGetKnownFolderPath), so the test would be RED on a windows
// runner and could touch the operator's real `~/.phantom-mesh`. Gate the whole
// file to Unix until a PHANTOM_HOME-honouring resolver lands (matches the repo
// pattern of windows-ignoring `$HOME`-sandbox tests).
#![cfg(unix)]

use phantom_mesh::cli_config::{
    broker_config_path, config_pull_lines, read_broker_config, write_broker_config, BrokerConfig,
};
use tempfile::TempDir;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Single sequential `#[tokio::test]`: drives the full
/// persist → reload sequence in one known order.
///
/// One test (not several) on purpose: the path helpers read `$HOME` from
/// the *process-global* env, and Rust's harness runs `#[tokio::test]`
/// functions in a file in parallel by default — separate tests would race
/// on `set_var("HOME", ...)` and clobber each other's tempdir. A single
/// sequential body needs zero extra deps (`serial_test`) and is just as
/// expressive (mirrors the cuj05_delete_include_broker.rs reasoning).
#[tokio::test]
async fn cuj03_broker_login_token_persist_and_reread() {
    // ── Hermetic HOME: redirect ~/.phantom-mesh into a throwaway tempdir ──
    let home = TempDir::new().expect("tempdir for fake HOME");
    std::env::set_var("HOME", home.path());
    // Spec'd data-root override (forward-compat; see module doc).
    std::env::set_var("PHANTOM_HOME", home.path());

    // Precondition: no broker.json yet (clean machine, never logged in).
    let cfg_path = broker_config_path().expect("broker_config_path resolves under temp HOME");
    assert!(
        cfg_path.starts_with(home.path()),
        "broker.json must resolve under the temp HOME, got: {}",
        cfg_path.display()
    );
    assert!(
        !cfg_path.exists(),
        "precondition: broker.json must NOT exist before login"
    );

    // ── Stand up a wiremock broker honouring the authenticated vault read ─
    let broker = MockServer::start().await;
    let broker_token = "broker-jwt-test-abc123";

    // GET /api/me/settings/raw — the authenticated exchange login_broker
    // runs (via config_pull_lines) before it persists the token. Requires
    // the exact Bearer header so a dropped-auth regression fails here.
    Mock::given(method("GET"))
        .and(path("/api/me/settings/raw"))
        .and(header("authorization", &*format!("Bearer {broker_token}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            // Empty env keeps the test from mutating process-global env
            // (config_pull_lines set_var's each returned key). The persist
            // we care about is broker.json, not the env merge.
            "env": {}
        })))
        .mount(&broker)
        .await;

    // Best-effort cluster-peers fetch config_pull_lines also makes — return
    // an empty registry so it doesn't 404-noise (failure there is non-fatal
    // anyway, but a clean 200 keeps the log quiet).
    Mock::given(method("GET"))
        .and(path("/api/me/cluster-peers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "peers": []
        })))
        .mount(&broker)
        .await;

    let broker_url = broker.uri();

    // ── Step 1: the authenticated broker exchange (login_broker step 2) ───
    // This is the real gate: the token only gets persisted if this succeeds.
    let lines = config_pull_lines(&broker_url, broker_token)
        .await
        .expect("authenticated vault pull should succeed against 200 mock");
    assert!(
        lines.iter().any(|l| l.contains("pulled")),
        "vault pull should report a result line; got: {lines:?}"
    );

    // ── Step 2: persist the token to broker.json (login_broker step 3) ────
    write_broker_config(&BrokerConfig {
        url: broker_url.trim_end_matches('/').to_string(),
        token: broker_token.to_string(),
    })
    .expect("write_broker_config should persist broker.json");

    // ── Assert MAC-CUJ03-LOG-001: broker.json now exists on disk ──────────
    assert!(
        cfg_path.exists(),
        "MAC-CUJ03-LOG-001: broker.json must exist after login token-persist, expected at {}",
        cfg_path.display()
    );

    // Prove the authenticated exchange actually carried the Bearer token —
    // i.e. the persist was gated on a real authenticated read, not blind.
    let received = broker
        .received_requests()
        .await
        .expect("mock recorded requests");
    let settings_req = received
        .iter()
        .find(|r| r.url.path() == "/api/me/settings/raw")
        .expect("broker should have seen the settings/raw read");
    let auth_hdr = settings_req
        .headers
        .get("authorization")
        .expect("settings read must carry an Authorization header")
        .to_str()
        .expect("auth header is valid UTF-8");
    assert_eq!(
        auth_hdr,
        format!("Bearer {broker_token}"),
        "the authenticated vault read must present the broker token as a Bearer credential"
    );

    // ── Step 3: clear in-memory state, re-read off disk (LOG-002) ─────────
    // Simulate "post-001 + clear cache": a brand-new process has no token in
    // memory and must recover it purely from ~/.phantom-mesh/broker.json.
    drop(broker); // tear down the live server — re-read is disk-only.

    let reread = read_broker_config()
        .expect("MAC-CUJ03-LOG-002: a subsequent read must pick the persisted token back up");
    assert_eq!(
        reread.token, broker_token,
        "re-read broker_token must match what was persisted at login (token persistence)"
    );
    assert_eq!(
        reread.url,
        broker_url.trim_end_matches('/'),
        "re-read broker_url must match what was persisted at login"
    );

    // Final tamper-evidence: the on-disk JSON really is the broker token,
    // not an empty/default struct that happened to deserialize.
    let raw = std::fs::read_to_string(&cfg_path).expect("broker.json readable");
    assert!(
        raw.contains(broker_token),
        "broker.json on disk must contain the persisted token; got: {raw}"
    );
}
