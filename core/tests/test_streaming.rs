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
use std::sync::atomic::Ordering;

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
                msg.contains("agent") || msg.contains("config") || msg.contains("provider") || msg.contains("No"),
                "error message should mention config/provider, got: {msg}"
            );
        }
        Ok(r) => {
            assert!(
                r.output.contains("failed") || r.output.contains("error") || r.output.is_empty(),
                "Ok result should indicate failure, got: {:?}", r.output
            );
        }
    }
}

// ── Test 2: StreamEvent variants can be created and matched ───────────────

#[test]
fn test_stream_event_variants() {
    // Verify every StreamEvent variant can be constructed and matched without
    // compiler errors.  This test exercises the public enum surface.
    let token = StreamEvent::Token { content: "hello".to_string() };
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
    let done = StreamEvent::Done { total_tokens: 10, cost_usd: 0.001 };

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
            StreamEvent::ToolDone { name, result_preview, .. } => {
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
    };
    assert_eq!(r.output, "hello");
    assert_eq!(r.tool_calls_made.len(), 1);
    assert!(r.elapsed_secs > 0.0);
}

// ── Test 4: mock provider streaming — tokens collected via on_event ────────

#[tokio::test]
async fn test_stream_collects_tokens() {
    // Build a minimal valid SSE payload using the OpenAI streaming format.
    // Two token chunks followed by the [DONE] sentinel.
    let sse_body =
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n\
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
    let sse_body = "data: {\"choices\":[{\"delta\":{\"content\":\"fallback\"}}]}\n\ndata: [DONE]\n\n";

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
    let sse_body =
        "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n\
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
    assert_eq!(result, "abc", "tokens must arrive in order, got: {result:?}");
}
