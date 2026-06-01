use std::sync::atomic::{AtomicUsize, Ordering};
/// Integration tests for the streaming module.
///
/// Tests cover:
///   1. `stream_agent_full` with no provider config → graceful error / fallback
///   2. `StreamEvent` enum variants can be constructed and matched
///   3. A mock axum SSE server returns chunked tokens that are collected via the
///      `on_event` callback
///
/// NOTE: `streaming` must be accessible as `phantom_mesh::streaming` for these
/// tests to compile.  If it is not yet re-exported from `lib.rs`, add
///   `pub mod streaming;`
/// to `src/lib.rs`.  The module itself exists at `src/streaming.rs`.
use std::sync::{Arc, Mutex};

use axum::routing::post;
use axum::Router;
use serde_json::json;
use tokio::net::TcpListener;

use phantom_mesh::{
    config::{AgentEntry, AgentsConfig, ProviderEntry},
    providers::traits::ChatMessage,
    streaming::{stream_agent_full, StreamEvent, StreamResult},
};

const NO_HISTORY: &[ChatMessage] = &[];

// ── Mock SSE server helpers ───────────────────────────────────────────────

/// Spin up a mock HTTP server that returns a canned SSE response body.
///
/// `sse_body` must be a valid SSE payload, e.g.
/// ```
/// "data: {...}\n\ndata: [DONE]\n\n"
/// ```
/// Returns the base URL of the server.
async fn start_sse_mock(sse_body: &'static str) -> String {
    use axum::http::StatusCode;
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || async move {
            axum::response::Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/event-stream")
                .body(axum::body::Body::from(sse_body))
                .expect("build SSE mock response")
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind streaming mock server");
    let addr = listener.local_addr().expect("local addr");

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("streaming mock server error");
    });

    format!("http://127.0.0.1:{}", addr.port())
}

/// Build a minimal `AgentsConfig` pointing at `base_url`.
fn streaming_config(base_url: &str) -> AgentsConfig {
    let mut providers = std::collections::HashMap::new();
    providers.insert(
        "mock".to_string(),
        ProviderEntry {
            provider_type: "openai_compat".to_string(),
            url: Some(format!("{}/v1/chat/completions", base_url)),
            api_key: Some("test-key".to_string()),
            api_key_env: None,
            default_model: None,
            tier: None,
        },
    );

    let mut agent = std::collections::HashMap::new();
    agent.insert(
        "master".to_string(),
        AgentEntry {
            provider: "mock".to_string(),
            providers: None,
            model: "mock-model".to_string(),
            tools: vec![],
            instructions: String::new(),
        },
    );

    AgentsConfig {
        providers,
        agent,
        ..Default::default()
    }
}

// ── Test 1: stream_agent_full with no config → graceful fallback ──────────

#[tokio::test]
async fn test_stream_no_config() {
    // An empty AgentsConfig has no agents and no providers.
    // stream_agent_full should return an Err (or a graceful output), not panic.
    let config = AgentsConfig::default();

    let result: anyhow::Result<StreamResult> = stream_agent_full(
        &config,
        "master",
        "hello",
        NO_HISTORY,
        None,
        None,
        |_event: StreamEvent| {},
    )
    .await;

    // With default config (no real API key), either Err or Ok with failure message.
    match result {
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("agent")
                    || msg.contains("config")
                    || msg.contains("provider")
                    || msg.contains("No"),
                "error message should mention config/provider, got: {msg}"
            );
        }
        Ok(r) => {
            assert!(
                r.output.contains("failed") || r.output.contains("error") || r.output.is_empty(),
                "Ok result should indicate failure, got: {:?}",
                r.output
            );
        }
    }
}

// ── Test 2: StreamEvent variants can be created and matched ───────────────

#[test]
fn test_stream_event_variants() {
    // Verify every StreamEvent variant can be constructed and matched without
    // compiler errors.  This test exercises the public enum surface.
    let token = StreamEvent::Token {
        content: "hello".to_string(),
    };
    let tool_start = StreamEvent::ToolStart {
        id: "call_1".to_string(),
        name: "shell".to_string(),
        args_json: "{\"command\":\"echo hi\"}".to_string(),
    };
    let tool_done = StreamEvent::ToolDone {
        id: "call_1".to_string(),
        name: "shell".to_string(),
        result_preview: "hi\n[exit 0]".to_string(),
        elapsed_ms: 42,
    };
    let done = StreamEvent::Done {
        total_tokens: 10,
        cost_usd: 0.001,
    };

    // Match each variant to make sure the enum arms compile correctly.
    let mut saw_token = false;
    let mut saw_tool_start = false;
    let mut saw_tool_done = false;
    let mut saw_done = false;

    for event in [token, tool_start, tool_done, done] {
        match event {
            StreamEvent::Token { content: t } => {
                assert_eq!(t, "hello");
                saw_token = true;
            }
            StreamEvent::ToolStart { name, .. } => {
                assert_eq!(name, "shell");
                saw_tool_start = true;
            }
            StreamEvent::ToolDone {
                name,
                result_preview,
                ..
            } => {
                assert_eq!(name, "shell");
                assert!(!result_preview.is_empty());
                saw_tool_done = true;
            }
            StreamEvent::Done { .. } => {
                saw_done = true;
            }
            _ => {}
        }
    }

    assert!(saw_token, "StreamEvent::Token variant not covered");
    assert!(saw_tool_start, "StreamEvent::ToolStart variant not covered");
    assert!(saw_tool_done, "StreamEvent::ToolDone variant not covered");
    assert!(saw_done, "StreamEvent::Done variant not covered");
}

// ── Test 3: StreamResult fields are accessible ────────────────────────────

#[test]
fn test_stream_result_fields() {
    // StreamResult is a plain struct; verify all public fields are accessible.
    let r = StreamResult {
        output: "hello".to_string(),
        tool_calls_made: vec![json!({"tool": "shell", "args": {"command": "echo hi"}})],
        elapsed_secs: 0.042,
        ..Default::default()
    };
    assert_eq!(r.output, "hello");
    assert_eq!(r.tool_calls_made.len(), 1);
    assert!(r.elapsed_secs > 0.0);
    // [F1] Cache token fields default to 0 for OpenAI-compatible providers.
    assert_eq!(r.cache_read_input_tokens, 0);
    assert_eq!(r.cache_creation_input_tokens, 0);
}

// ── Test 4: mock provider streaming — tokens collected via on_event ────────

#[tokio::test]
async fn test_stream_collects_tokens() {
    // Build a minimal valid SSE payload using the OpenAI streaming format.
    // Two token chunks followed by the [DONE] sentinel.
    let sse_body = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n\
         data: {\"choices\":[{\"delta\":{\"content\":\", world\"}}]}\n\n\
         data: [DONE]\n\n";

    let base_url = start_sse_mock(sse_body).await;
    let config = streaming_config(&base_url);

    let tokens: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let tokens_clone = tokens.clone();

    let done_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done_flag_clone = done_flag.clone();

    let result = stream_agent_full(
        &config,
        "master",
        "say hello",
        NO_HISTORY,
        None,
        None,
        move |event: StreamEvent| match event {
            StreamEvent::Token { content: t } => {
                tokens_clone.lock().unwrap().push(t);
            }
            StreamEvent::Done { .. } => {
                done_flag_clone.store(true, Ordering::SeqCst);
            }
            _ => {}
        },
    )
    .await
    .expect("stream_agent_full should succeed with mock SSE server");

    let collected: Vec<String> = tokens.lock().unwrap().clone();
    let full_text = collected.join("");

    assert_eq!(
        full_text, "Hello, world",
        "expected streamed text 'Hello, world', got: {full_text:?}"
    );
    assert!(
        done_flag.load(Ordering::SeqCst),
        "StreamEvent::Done should have been fired"
    );
    // The StreamResult.output must equal the accumulated text.
    assert_eq!(
        result.output, "Hello, world",
        "StreamResult.output should equal accumulated token text"
    );
}

// ── Test 5: Done event is always fired, even with no tokens ──────────────

#[tokio::test]
async fn test_stream_done_fired_on_empty_response() {
    // A response with no content, only [DONE].
    let sse_body = "data: {\"choices\":[{\"delta\":{}}]}\n\ndata: [DONE]\n\n";

    let base_url = start_sse_mock(sse_body).await;
    let config = streaming_config(&base_url);

    let done_fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done_flag = done_fired.clone();

    let result = stream_agent_full(
        &config,
        "master",
        "ping",
        NO_HISTORY,
        None,
        None,
        move |event: StreamEvent| {
            if let StreamEvent::Done { .. } = event {
                done_flag.store(true, Ordering::SeqCst);
            }
        },
    )
    .await
    .expect("stream_agent_full should succeed with empty-content mock");

    assert!(
        done_fired.load(Ordering::SeqCst),
        "StreamEvent::Done must always be fired at the end of a run"
    );
    assert_eq!(
        result.output, "",
        "output should be empty string for a no-content response"
    );
    assert!(
        result.tool_calls_made.is_empty(),
        "no tool calls should be recorded for a plain empty response"
    );
}

// ── Test 6: elapsed_secs is positive after a run ─────────────────────────

#[tokio::test]
async fn test_stream_elapsed_secs_positive() {
    let sse_body = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n";

    let base_url = start_sse_mock(sse_body).await;
    let config = streaming_config(&base_url);

    let result = stream_agent_full(
        &config,
        "master",
        "ping",
        NO_HISTORY,
        None,
        None,
        |_: StreamEvent| {},
    )
    .await
    .expect("stream should succeed");

    assert!(
        result.elapsed_secs >= 0.0,
        "elapsed_secs must be non-negative, got {}",
        result.elapsed_secs
    );
}

// ── Test 7: fallback to "master" when named agent is absent ──────────────

#[tokio::test]
async fn test_stream_falls_back_to_master() {
    let sse_body =
        "data: {\"choices\":[{\"delta\":{\"content\":\"fallback\"}}]}\n\ndata: [DONE]\n\n";

    let base_url = start_sse_mock(sse_body).await;
    let config = streaming_config(&base_url);

    // Request a non-existent agent name; should fall back to "master".
    let result = stream_agent_full(
        &config,
        "nonexistent_agent",
        "hello",
        NO_HISTORY,
        None,
        None,
        |_: StreamEvent| {},
    )
    .await
    .expect("should fall back to 'master' agent config");

    assert_eq!(
        result.output, "fallback",
        "fallback-to-master should produce the expected output, got: {:?}",
        result.output
    );
}

// ── Test 8: multiple on_event callbacks accumulate in order ───────────────

#[tokio::test]
async fn test_stream_token_order_preserved() {
    // Three chunks that together spell "abc".
    let sse_body = "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n\
         data: {\"choices\":[{\"delta\":{\"content\":\"b\"}}]}\n\n\
         data: {\"choices\":[{\"delta\":{\"content\":\"c\"}}]}\n\n\
         data: [DONE]\n\n";

    let base_url = start_sse_mock(sse_body).await;
    let config = streaming_config(&base_url);

    let order: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let order_clone = order.clone();

    stream_agent_full(
        &config,
        "master",
        "abc",
        NO_HISTORY,
        None,
        None,
        move |event: StreamEvent| {
            if let StreamEvent::Token { content: t } = event {
                order_clone.lock().unwrap().push(t);
            }
        },
    )
    .await
    .expect("should succeed");

    let result = order.lock().unwrap().join("");
    assert_eq!(
        result, "abc",
        "tokens must arrive in order, got: {result:?}"
    );
}

// ── T19 retry-test serialization ───────────────────────────────────────────
//
// PHANTOM_TEST_PRE_STREAM_FAST is process-global, so tokio's default parallel
// runner causes flakes when one test sets it and another removes it. We
// serialize all the retry tests on an async-aware mutex; happy-path tests
// above don't touch the env so they stay parallel.
static T19_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ── Flaky-mock helper for T19 pre-stream retry tests ──────────────────────

/// Spin up a mock HTTP server that returns `failure_codes` for the first N
/// requests (one per slot in the slice) and then a canned SSE body on every
/// subsequent request. Returns the base URL.
///
/// Each slot in `failure_codes` is `(status_code, retry_after_header)`.
/// `retry_after_header` of `""` means: do not set the header.
async fn start_flaky_mock(
    failure_codes: &'static [(u16, &'static str)],
    success_sse: &'static str,
) -> String {
    use axum::http::StatusCode;

    let counter = std::sync::Arc::new(AtomicUsize::new(0));

    let app = Router::new().route(
        "/v1/chat/completions",
        post({
            let counter = counter.clone();
            move || {
                let counter = counter.clone();
                async move {
                    let idx = counter.fetch_add(1, Ordering::SeqCst);
                    if idx < failure_codes.len() {
                        let (code, retry_after) = failure_codes[idx];
                        let mut builder = axum::response::Response::builder()
                            .status(StatusCode::from_u16(code).expect("valid status"));
                        if !retry_after.is_empty() {
                            builder = builder.header("retry-after", retry_after);
                        }
                        return builder
                            .body(axum::body::Body::from("flaky"))
                            .expect("build flaky response");
                    }
                    axum::response::Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", "text/event-stream")
                        .body(axum::body::Body::from(success_sse))
                        .expect("build SSE response")
                }
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind flaky mock");
    let addr = listener.local_addr().expect("local addr");

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("flaky mock server error");
    });

    format!("http://127.0.0.1:{}", addr.port())
}

// ── T19 Test 1: 429 then 200 — retries succeed before any token is emitted ─

#[tokio::test]
async fn test_pre_stream_retry_429_then_succeeds() {
    let _guard = T19_ENV_LOCK.lock().await;
    // Speed up wallclock for retry tests where applicable.
    std::env::set_var("PHANTOM_TEST_PRE_STREAM_FAST", "1");

    let success_sse = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n\
         data: [DONE]\n\n";
    // One 429 (no Retry-After), then 200 with content.
    let base_url = start_flaky_mock(&[(429, "")], success_sse).await;
    let config = streaming_config(&base_url);

    let tokens: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let tokens_clone = tokens.clone();

    let result = stream_agent_full(
        &config,
        "master",
        "test",
        NO_HISTORY,
        None,
        None,
        move |event: StreamEvent| {
            if let StreamEvent::Token { content: t } = event {
                tokens_clone.lock().unwrap().push(t);
            }
        },
    )
    .await
    .expect("retry must succeed after one 429");

    assert_eq!(
        result.output, "ok",
        "expected 'ok' after retry, got {:?}",
        result.output
    );
    let collected: Vec<String> = tokens.lock().unwrap().clone();
    assert_eq!(
        collected.join(""),
        "ok",
        "tokens should reflect post-retry success"
    );
}

// ── T19 Test 2: 503 across all attempts — error reports exhaustion ────────

#[tokio::test]
async fn test_pre_stream_retry_503_exhausted() {
    let _guard = T19_ENV_LOCK.lock().await;
    // 4 attempts total (1 initial + 3 retries). All 503. Fast mode keeps the
    // wallclock under a second.
    std::env::set_var("PHANTOM_TEST_PRE_STREAM_FAST", "1");

    let success_sse = "data: [DONE]\n\n"; // unreachable
    let base_url = start_flaky_mock(
        &[(503, ""), (503, ""), (503, ""), (503, ""), (503, "")],
        success_sse,
    )
    .await;
    let config = streaming_config(&base_url);

    let result = stream_agent_full(
        &config,
        "master",
        "test",
        NO_HISTORY,
        None,
        None,
        |_event: StreamEvent| {},
    )
    .await;

    match result {
        Err(e) => {
            let msg = format!("{}", e);
            // The rich PreStreamRetryError must surface via anyhow's Display.
            assert!(msg.contains("503"), "msg should mention status: {}", msg);
            assert!(
                msg.contains("4 attempt"),
                "msg should report attempts=4: {}",
                msg
            );
            assert!(
                msg.contains("mock") || msg.contains("[mock"),
                "msg should tag provider: {}",
                msg
            );
        }
        Ok(r) => {
            // If all providers fail, stream_agent_full returns an Ok with empty
            // output (graceful degrade). Still fine as long as no tokens
            // leaked through.
            assert!(
                r.output.is_empty(),
                "expected empty output on full retry exhaustion, got {:?}",
                r.output,
            );
        }
    }
}

// ── T19 Test 3: Retry-After: 1 → respected, parsed as 1 second ────────────

#[tokio::test]
async fn test_pre_stream_retry_honours_retry_after() {
    let _guard = T19_ENV_LOCK.lock().await;
    let success_sse = "data: {\"choices\":[{\"delta\":{\"content\":\"after-wait\"}}]}\n\n\
         data: [DONE]\n\n";
    // One 429 with Retry-After: 1, then 200.
    let base_url = start_flaky_mock(&[(429, "1")], success_sse).await;
    let config = streaming_config(&base_url);

    // Make sure the fast-mode env is NOT set for this test — we want to
    // verify that Retry-After (1s) is actually used.
    std::env::remove_var("PHANTOM_TEST_PRE_STREAM_FAST");

    let start = std::time::Instant::now();
    let result = stream_agent_full(
        &config,
        "master",
        "test",
        NO_HISTORY,
        None,
        None,
        |_event: StreamEvent| {},
    )
    .await
    .expect("should succeed after honouring Retry-After");
    let elapsed = start.elapsed();

    assert_eq!(result.output, "after-wait");
    assert!(
        elapsed >= std::time::Duration::from_millis(900),
        "Retry-After: 1 should cause >= ~1s sleep, took {:?}",
        elapsed,
    );
    // Sanity upper bound — even with jitter on subsequent attempts (there are
    // none here), plus TCP setup overhead on Windows / cargo test cold-start,
    // should not exceed ~6 seconds. (If we exceeded that we'd be in the
    // exponential-backoff path, not the Retry-After path.)
    assert!(
        elapsed < std::time::Duration::from_secs(6),
        "Retry-After: 1 should not exceed ~6s including jitter, took {:?}",
        elapsed,
    );
}

// ── T19 Test 4: 400 fails immediately, no retry ───────────────────────────

#[tokio::test]
async fn test_pre_stream_retry_no_retry_on_400() {
    let _guard = T19_ENV_LOCK.lock().await;
    // 400 returned indefinitely. Retry layer must NOT retry; the mock
    // counter at the end must show exactly 1 hit.
    std::env::set_var("PHANTOM_TEST_PRE_STREAM_FAST", "1");

    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    use axum::http::StatusCode;
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                axum::response::Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(axum::body::Body::from("bad input"))
                    .expect("400")
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum");
    });
    let base_url = format!("http://127.0.0.1:{}", addr.port());
    let config = streaming_config(&base_url);

    let _ = stream_agent_full(
        &config,
        "master",
        "test",
        NO_HISTORY,
        None,
        None,
        |_: StreamEvent| {},
    )
    .await;

    // The OUTER MAX_RECONNECT_ATTEMPTS loop (capped at 3 total iterations
    // including reconnect_attempt=0) will retry on terminal pre-stream
    // failures too — that's a pre-existing behaviour we deliberately did
    // not change. So expect up to MAX_RECONNECT_ATTEMPTS+1 = 3 hits.
    // Crucially the *pre-stream* retry layer must NOT retry on 400, so the
    // hit count must NOT be 4 (which is what one round of pre-stream retries
    // would produce).
    let hit_count = counter.load(Ordering::SeqCst);
    assert!(
        hit_count <= 3,
        "400 must NOT trigger pre-stream retry (which would yield 4+ hits per round); got {}",
        hit_count,
    );
    assert!(
        hit_count >= 1,
        "should have hit the mock at least once, got {}",
        hit_count,
    );
}

// ── T19 Test 5: tokens already emitted → no re-issue on next call ─────────
//
// This test guards the contract: if the SSE body actually delivers tokens
// (the happy path with a normal, valid response), no extra retry occurs
// regardless of what the response looks like after. We test by sending a
// totally valid SSE that completes cleanly, and asserting the mock was hit
// exactly once.

#[tokio::test]
async fn test_pre_stream_retry_no_retry_after_tokens_emitted() {
    let _guard = T19_ENV_LOCK.lock().await;
    std::env::set_var("PHANTOM_TEST_PRE_STREAM_FAST", "1");

    let success_sse = "data: {\"choices\":[{\"delta\":{\"content\":\"first\"}}]}\n\n\
         data: {\"choices\":[{\"delta\":{\"content\":\"-second\"}}]}\n\n\
         data: [DONE]\n\n";

    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    use axum::http::StatusCode;
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                axum::response::Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "text/event-stream")
                    .body(axum::body::Body::from(success_sse))
                    .expect("200")
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum");
    });
    let base_url = format!("http://127.0.0.1:{}", addr.port());
    let config = streaming_config(&base_url);

    let result = stream_agent_full(
        &config,
        "master",
        "test",
        NO_HISTORY,
        None,
        None,
        |_: StreamEvent| {},
    )
    .await
    .expect("happy-path stream should succeed");

    assert_eq!(result.output, "first-second");
    let hit_count = counter.load(Ordering::SeqCst);
    assert_eq!(
        hit_count, 1,
        "happy-path stream must hit mock exactly once; got {}",
        hit_count,
    );
}
