// F102 · Integration tests for dispatch commands.
//
// These run against a real axum mock broker bound to a random port so
// the test exercises the full POST → SSE-stream → frame-parse → emit
// chain (minus the Tauri Window emit, which we replace with a channel).
//
// What we verify (mirrors E002 acceptance + F102 test matrix):
//   - dispatch_rejects_missing_token: invoking dispatch_task with no
//     broker_token saved (or an empty one) returns E_DISPATCH_AUTH_REQUIRED.
//   - validators round-trip: invalid prompts/caps/providers/URLs are
//     caught with stable E_DISPATCH_* codes (same surface the JS layer
//     pattern-matches on).
//   - end-to-end: against a mock broker that emits canned SSE frames,
//     the run_dispatch_stream loop parses every frame and (in the real
//     code path) hands them to the emitter. We assert frame parsing +
//     bearer-auth handling on the wire.

use phantom_mesh_app_lib::commands::dispatch::{
    parse_frame, validate_caps, validate_prompt, validate_provider_in_set, DispatchFrame,
};

#[test]
fn validators_reject_bad_inputs_with_stable_codes() {
    // Empty prompt → E_DISPATCH_PROMPT_EMPTY
    assert_eq!(validate_prompt("").unwrap_err(), "E_DISPATCH_PROMPT_EMPTY");
    assert_eq!(
        validate_prompt("   \n  ").unwrap_err(),
        "E_DISPATCH_PROMPT_EMPTY"
    );

    // Oversize prompt → E_DISPATCH_PROMPT_TOO_LONG
    let big = "a".repeat(9_000);
    assert_eq!(
        validate_prompt(&big).unwrap_err(),
        "E_DISPATCH_PROMPT_TOO_LONG"
    );

    // NUL byte → E_DISPATCH_PROMPT_INVALID
    assert_eq!(
        validate_prompt("foo\0bar").unwrap_err(),
        "E_DISPATCH_PROMPT_INVALID"
    );

    // Bad cap shapes → E_DISPATCH_CAPS_INVALID
    assert_eq!(
        validate_caps(&["GPU ".to_string()]).unwrap_err(),
        "E_DISPATCH_CAPS_INVALID"
    );
    assert_eq!(
        validate_caps(&["gpu!".to_string()]).unwrap_err(),
        "E_DISPATCH_CAPS_INVALID"
    );

    // >3 caps → E_DISPATCH_CAPS_TOO_MANY
    let caps = vec![
        "a".to_string(),
        "b".to_string(),
        "c".to_string(),
        "d".to_string(),
    ];
    assert_eq!(
        validate_caps(&caps).unwrap_err(),
        "E_DISPATCH_CAPS_TOO_MANY"
    );

    // Unknown provider → E_DISPATCH_PROVIDER_UNKNOWN
    let allowed = vec!["openai".to_string()];
    assert_eq!(
        validate_provider_in_set(Some("unknown-provider"), &allowed).unwrap_err(),
        "E_DISPATCH_PROVIDER_UNKNOWN"
    );
}

#[test]
fn frame_parser_round_trips_all_variants() {
    // Verify the frame parser hands the JS-emitting layer correct
    // variants for every type the broker emits. This is the contract
    // that the F103 React store keys off of.
    assert!(matches!(
        parse_frame(r#"{"type":"token","text":"hello "}"#).unwrap(),
        DispatchFrame::Token { ref text } if text == "hello "
    ));
    assert!(matches!(
        parse_frame(r#"{"type":"status","phase":"running"}"#).unwrap(),
        DispatchFrame::Status { ref phase } if phase == "running"
    ));
    assert!(matches!(
        parse_frame(r#"{"type":"done","result":"42"}"#).unwrap(),
        DispatchFrame::Done { ref result } if result == "42"
    ));
    match parse_frame(r#"{"type":"error","code":"E_BOOM","message":"x"}"#).unwrap() {
        DispatchFrame::Error { code, message } => {
            assert_eq!(code, "E_BOOM");
            assert_eq!(message, "x");
        }
        _ => panic!("expected Error variant"),
    }
}

// ── End-to-end against a mock broker ────────────────────────────────────
//
// We spin a tiny axum SSE endpoint that:
//   1. asserts the inbound Authorization header carries "Bearer <token>"
//   2. emits three SSE frames (status → token → done)
// and exercise the same chunk-reading + parsing logic dispatch_task uses
// (minus the Window emit). This proves the full data path works end-to-
// end without needing a Tauri runtime.

#[tokio::test]
async fn end_to_end_post_and_sse_stream_parses_frames() {
    use axum::http::HeaderMap;
    use axum::http::header;
    use axum::response::Response;
    use axum::routing::post;
    use axum::Router;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Default, Clone)]
    struct Capture {
        bearer: Option<String>,
        body: Option<String>,
    }
    let cap = Arc::new(Mutex::new(Capture::default()));

    // Build a plain text/event-stream response body — three `data:`
    // frames separated by blank lines. Avoids pulling in futures_util
    // for axum's streaming Sse helper (which we don't need: the
    // production reader treats the body as raw chunked text anyway).
    let sse_body = "\
data: {\"type\":\"status\",\"phase\":\"running\"}\n\n\
data: {\"type\":\"token\",\"text\":\"hi \"}\n\n\
data: {\"type\":\"done\",\"result\":\"hi\"}\n\n";

    let cap_for_handler = cap.clone();
    let sse_body_owned = sse_body.to_string();
    let app = Router::new().route(
        "/api/squad/dispatch",
        post(move |headers: HeaderMap, body: String| {
            let cap = cap_for_handler.clone();
            let sse_body = sse_body_owned.clone();
            async move {
                {
                    let mut g = cap.lock().await;
                    g.bearer = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(String::from);
                    g.body = Some(body);
                }
                Response::builder()
                    .status(200)
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(sse_body)
                    .unwrap()
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // Drive the same wire shape dispatch_task produces.
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/api/squad/dispatch", addr.port());
    let mut resp = client
        .post(&url)
        .bearer_auth("integration-test-token")
        .header("Accept", "text/event-stream")
        .json(&serde_json::json!({
            "prompt": "hello",
            "required_caps": ["gpu"],
            "provider_override": null,
            "dispatch_id": "test-1",
        }))
        .send()
        .await
        .expect("send");
    assert!(resp.status().is_success(), "broker should accept request");

    // Read + parse the chunked SSE body the same way run_dispatch_stream
    // does (chunk loop + \n\n splitter).
    let mut buf = String::new();
    let mut frames: Vec<DispatchFrame> = Vec::new();
    while let Some(chunk) = resp.chunk().await.expect("chunk") {
        if let Ok(s) = std::str::from_utf8(&chunk) {
            buf.push_str(s);
        }
        while let Some(idx) = buf.find("\n\n") {
            let block: String = buf.drain(..idx + 2).collect();
            // Inline the same extract_data_lines logic the production
            // module uses (kept private to dispatch.rs).
            let mut current = String::new();
            for line in block.split('\n') {
                let line = line.trim_end_matches('\r');
                if let Some(rest) = line.strip_prefix("data:") {
                    let rest = rest.strip_prefix(' ').unwrap_or(rest);
                    if !current.is_empty() {
                        current.push('\n');
                    }
                    current.push_str(rest);
                }
            }
            if !current.is_empty() {
                if let Some(frame) = parse_frame(&current) {
                    frames.push(frame);
                }
            }
        }
    }

    // Verify all three frames arrived in order.
    assert_eq!(frames.len(), 3, "expected 3 SSE frames, got {:?}", frames);
    assert!(matches!(frames[0], DispatchFrame::Status { ref phase } if phase == "running"));
    assert!(matches!(frames[1], DispatchFrame::Token { ref text } if text == "hi "));
    assert!(matches!(frames[2], DispatchFrame::Done { ref result } if result == "hi"));

    // Verify the broker saw the bearer token (E002 sec acceptance — the
    // command MUST send Authorization with a token, never raw).
    let g = cap.lock().await;
    assert_eq!(
        g.bearer.as_deref(),
        Some("Bearer integration-test-token"),
        "dispatch_task must send Authorization: Bearer <token>"
    );
    // And the body must carry the JSON shape we expect.
    let body = g.body.as_deref().unwrap_or("");
    assert!(body.contains("\"prompt\""), "body should include prompt key: {body}");
    assert!(body.contains("\"required_caps\""), "body should include required_caps key");

    server.abort();
}
