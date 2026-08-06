// Integration test: `spectyn sessions` must degrade gracefully on a
// machine that is NOT logged in to the cross-mesh broker. Before this
// change, `sessions_lines()` hard-required auth.json and bailed with
// "not logged in" / HTTP 401. Now it tries the LOCAL loopback plane
// (`GET /api/sessions`, no auth) first and only consults the broker for
// the cross-mesh view — so a local-only run returns Ok with the local
// view instead of an error.
//
// These path/auth helpers read process-global env (HOME / SPECTYN_HOME)
// and the macOS/Linux home resolver, so this is gated to unix (matches
// the repo pattern of the other $HOME-sandbox tests, e.g.
// cuj03_broker_login_token_persist.rs).
#![cfg(unix)]

use spectyn_mesh::cli_config::{local_sessions_lines, render_local_sessions, sessions_lines};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Local plane reachable + has sessions → local view is rendered (no auth
/// header required on the request). Proves the local-first fetch works
/// against the real `/api/sessions` array shape.
#[tokio::test]
async fn local_plane_with_sessions_renders_local_view_no_auth() {
    let server = MockServer::start().await;

    // The local serve endpoint returns a bare JSON ARRAY (see
    // serve::api_sessions) and requires NO Authorization header — mounting
    // without a header matcher proves we don't send/need one.
    Mock::given(method("GET"))
        .and(path("/api/sessions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "id": "sess-aaa", "size_bytes": 1234, "modified": 100, "message_count": 7 },
            { "id": "sess-bbb", "size_bytes": 88, "modified": 50, "message_count": 1 }
        ])))
        .mount(&server)
        .await;

    let lines = local_sessions_lines(&server.uri())
        .await
        .expect("local plane fetch must succeed without auth");

    assert!(
        lines.first().map(|l| l.contains("local session")).unwrap_or(false),
        "expected a local-sessions header, got: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("sess-aaa")),
        "expected the session id in the rendered output, got: {lines:?}"
    );
}

/// Local plane reachable but empty → friendly "no active sessions" line,
/// still Ok (not an error / 401).
#[tokio::test]
async fn local_plane_empty_degrades_to_notice() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/sessions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let lines = local_sessions_lines(&server.uri())
        .await
        .expect("empty local plane must still be Ok");
    assert_eq!(lines, vec!["no active sessions on this machine".to_string()]);
}

/// THE GATE: not logged in (no auth.json) AND no local serve running.
/// `sessions_lines()` must return Ok with a friendly degraded view — NOT
/// an HTTP 401 / "not logged in" error.
#[tokio::test]
async fn sessions_lines_not_logged_in_degrades_to_ok() {
    // Hermetic HOME so auth::load() finds NO auth.json (never logged in).
    let home = TempDir::new().expect("tempdir for fake HOME");
    std::env::set_var("HOME", home.path());
    std::env::set_var("SPECTYN_HOME", home.path());

    // No `spectyn serve` is bound in-test, so the loopback fetch will fail
    // (connection refused) and the broker path will fail (no auth). The
    // command must still return Ok with the degraded notice.
    let result = sessions_lines().await;

    let lines = result.expect(
        "sessions_lines must DEGRADE to Ok when not logged in + no local serve, not error",
    );
    let joined = lines.join("\n");
    assert!(
        !joined.contains("401"),
        "must not surface an HTTP 401, got:\n{joined}"
    );
    assert!(
        !joined.to_lowercase().contains("not logged in"),
        "must not hard-error on missing auth, got:\n{joined}"
    );
    assert!(
        joined.contains("no active sessions"),
        "expected the degraded notice, got:\n{joined}"
    );
}

/// Pure render: array shape → header + one line per session.
#[test]
fn render_local_sessions_shapes_lines() {
    let empty = render_local_sessions(&[]);
    assert_eq!(empty, vec!["no active sessions on this machine".to_string()]);

    let one = render_local_sessions(&[serde_json::json!({
        "id": "abc", "size_bytes": 10, "modified": 1, "message_count": 3
    })]);
    assert!(one[0].contains("1 local session"), "got {one:?}");
    assert!(one[1].contains("abc"), "got {one:?}");
    assert!(one[1].contains("3 msgs"), "got {one:?}");
}
