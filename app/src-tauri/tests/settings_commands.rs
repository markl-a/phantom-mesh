// F105 · Integration tests for the mobile-settings command surface.
//
// These exercise the public validator helpers from the library crate so
// the wire contract (stable E_SETTINGS_* error codes + value semantics)
// is enumerable from a single place that mirrors the JS-side invoke
// boundary. Mirrors the F102 dispatch_commands.rs layout — three round-
// trip cases, one per new command surface.
//
// Scope-out: we do NOT drive the `#[tauri::command]` functions directly
// here. They mutate ~/.spectyn-mesh/{auth,agents.toml} on disk and the
// per-process TEST_PATH_OVERRIDE / AuthSnapshot machinery they rely on
// is gated behind cfg(test) inside the library. Those behaviours are
// covered by the `mobile_settings::tests` unit suite. This integration
// surface verifies the pieces JS sees — the validators + redactor — so
// a refactor that drifts the error codes is caught here, not at e2e.

use spectyn_mesh_app_lib::commands::mobile_settings::{
    redact_token, validate_heartbeat_secs, validate_peer_url,
    DEFAULT_HEARTBEAT_SECS, MAX_HEARTBEAT_SECS, MIN_HEARTBEAT_SECS,
};

#[test]
fn heartbeat_validator_round_trips_stable_codes() {
    // Inside the range → Ok(()).
    assert!(validate_heartbeat_secs(MIN_HEARTBEAT_SECS).is_ok());
    assert!(validate_heartbeat_secs(DEFAULT_HEARTBEAT_SECS).is_ok());
    assert!(validate_heartbeat_secs(MAX_HEARTBEAT_SECS).is_ok());

    // Outside → stable E_SETTINGS_HEARTBEAT_OUT_OF_RANGE.
    let err = validate_heartbeat_secs(0).unwrap_err();
    assert_eq!(err, "E_SETTINGS_HEARTBEAT_OUT_OF_RANGE");
    let err = validate_heartbeat_secs(MIN_HEARTBEAT_SECS - 1).unwrap_err();
    assert_eq!(err, "E_SETTINGS_HEARTBEAT_OUT_OF_RANGE");
    let err = validate_heartbeat_secs(MAX_HEARTBEAT_SECS + 1).unwrap_err();
    assert_eq!(err, "E_SETTINGS_HEARTBEAT_OUT_OF_RANGE");
}

#[test]
fn peer_validator_round_trips_normalised_url_or_stable_code() {
    // Allow-listed shapes → trimmed + slash-stripped URL.
    assert_eq!(
        validate_peer_url("http://localhost:7878").unwrap(),
        "http://localhost:7878"
    );
    assert_eq!(
        validate_peer_url("  http://localhost:7878/  ").unwrap(),
        "http://localhost:7878",
        "URL normaliser must strip trailing slash + whitespace"
    );
    assert!(validate_peer_url("https://phantommesh.io").is_ok());
    assert!(validate_peer_url("http://oracle.tail.ts.net:7878").is_ok());

    // Empty → distinct code so the UI can surface a "URL required" hint
    // separate from the daemon-allowlist reject.
    let err = validate_peer_url("").unwrap_err();
    assert_eq!(err, "E_SETTINGS_PEER_URL_EMPTY");
    let err = validate_peer_url("   \n ").unwrap_err();
    assert_eq!(err, "E_SETTINGS_PEER_URL_EMPTY");

    // V8-HIGH-2 allow-list rejects.
    for bad in [
        "http://evil.example.com/",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "localhost:7878",
        "http://user:pass@localhost/",
    ] {
        let err = validate_peer_url(bad).unwrap_err();
        assert!(
            err.starts_with("E_SETTINGS_PEER_URL_INVALID"),
            "expected E_SETTINGS_PEER_URL_INVALID for {bad:?}, got: {err}"
        );
    }
}

#[test]
fn redact_token_never_leaks_secret_prefix() {
    // Empty → empty. Lets the JS layer key "no broker logged in" off the
    // string length without a separate field.
    assert_eq!(redact_token(""), "");

    // Realistic broker-token shape (43-char base64 url-safe, no pad).
    let raw = "abcdefghijklmnopqrstuvwxyz0123456789ABCDxyz";
    let red = redact_token(raw);
    // Exact shape: 8 mask asterisks + last 4 chars of the raw token.
    assert_eq!(red, "********Dxyz",
        "redaction must surface only the last 4 chars, got: {red}");
    // Earlier chars do NOT leak.
    assert!(!red.contains("abcd"));
    assert!(!red.contains("efgh"));
    assert!(!red.contains("0123"));
    // 8 mask asterisks regardless of input length.
    assert!(red.starts_with("********"));
    assert_eq!(red.chars().take_while(|c| *c == '*').count(), 8);
}
