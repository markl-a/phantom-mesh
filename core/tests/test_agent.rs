use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use phantom_mesh::{
    agent::AgentRuntime,
    config::{AgentEntry, AgentsConfig, ProviderEntry},
};

// ── Mock server helpers ───────────────────────────────────────────────────────

/// Shared state that hands out canned JSON responses in sequence.
#[derive(Clone)]
struct MockState {
    responses: Arc<Vec<Value>>,
    call_count: Arc<AtomicUsize>,
}

async fn mock_handler(State(state): State<MockState>) -> impl IntoResponse {
    let idx = state.call_count.fetch_add(1, Ordering::SeqCst);
    let body = state
        .responses
        .get(idx)
        .cloned()
        .unwrap_or_else(|| json!({"error": "no more canned responses"}));
    axum::Json(body)
}

/// Spin up a mock HTTP server on a random port.
/// Returns the base URL, e.g. `"http://127.0.0.1:PORT"`.
/// The server runs in a background task for the lifetime of the test; it is
/// automatically dropped when the `tokio::test` runtime shuts down.
async fn start_mock_server(responses: Vec<Value>) -> String {
    let state = MockState {
        responses: Arc::new(responses),
        call_count: Arc::new(AtomicUsize::new(0)),
    };

    let app = Router::new()
        .route("/v1/chat/completions", post(mock_handler))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock server");
    let addr = listener.local_addr().expect("local addr");

    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("mock server error");
    });

    format!("http://127.0.0.1:{}", addr.port())
}

/// Build a minimal `AgentsConfig` wired up to a mock server.
fn config_with_mock(base_url: &str, tools: Vec<String>) -> AgentsConfig {
    let mut providers = std::collections::HashMap::new();
    providers.insert(
        "mock".to_string(),
        ProviderEntry {
            provider_type: "openai_compat".to_string(),
            url: Some(base_url.to_string()),
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
            tools,
            instructions: String::new(),
        },
    );

    AgentsConfig {
        providers,
        agent,
        ..Default::default()
    }
}

// ── 1. No valid providers → error mentions "All providers failed" ─────────────

#[tokio::test]
async fn test_agent_no_config() {
    let runtime = AgentRuntime::new(AgentsConfig::default());
    // Default config has no real API key so call_with_fallback returns Err.
    // We accept either Ok(result-with-error-text) or Err(error) — both are
    // valid "graceful failure" signals; we just verify the message is useful.
    let msg = match runtime.run("master", "hello", &[], None).await {
        Ok(r) => r.output,
        Err(e) => e.to_string(),
    };
    assert!(
        msg.contains("All providers failed")
            || msg.contains("No agent configuration")
            || msg.contains("no output"),
        "unexpected fallback message: {:?}",
        msg
    );
}

// ── 2. Single round — plain text response ────────────────────────────────────

#[tokio::test]
async fn test_agent_single_round_mock() {
    let canned = json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "Hello, world!",
                "tool_calls": null
            }
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5}
    });

    let base_url = start_mock_server(vec![canned]).await;
    let config = config_with_mock(&base_url, vec![]);
    let runtime = AgentRuntime::new(config);

    let result = runtime
        .run("master", "say hello", &[], None)
        .await
        .expect("run should not return Err");

    assert_eq!(
        result.output, "Hello, world!",
        "unexpected output: {:?}",
        result.output
    );
    assert!(result.tool_calls_made.is_empty(), "expected no tool calls");
}

// ── 3. Tool-call round then final text response ───────────────────────────────

#[tokio::test]
async fn test_agent_tool_call_round_mock() {
    // Round 1: the assistant wants to call `shell`
    let round1 = json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "c1",
                    "function": {
                        "name": "shell",
                        "arguments": "{\"command\":\"echo hi\"}"
                    }
                }]
            }
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5}
    });

    // Round 2: final plain-text answer
    let round2 = json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "Done.",
                "tool_calls": null
            }
        }],
        "usage": {"prompt_tokens": 15, "completion_tokens": 3}
    });

    let base_url = start_mock_server(vec![round1, round2]).await;
    let config = config_with_mock(&base_url, vec!["shell".to_string()]);
    let runtime = AgentRuntime::new(config);

    let result = runtime
        .run("master", "run echo hi", &[], None)
        .await
        .expect("run should not return Err");

    assert_eq!(
        result.output, "Done.",
        "unexpected final output: {:?}",
        result.output
    );
    assert_eq!(
        result.tool_calls_made.len(),
        1,
        "expected exactly 1 tool call recorded, got: {:?}",
        result.tool_calls_made
    );

    let recorded = &result.tool_calls_made[0];
    assert_eq!(
        recorded["tool"].as_str(),
        Some("shell"),
        "expected tool name 'shell', got: {:?}",
        recorded["tool"]
    );
}

// ── 4. Compaction — tested indirectly through the agent loop ──────────────────
//
// `compact_if_needed` is a private free function in `agent.rs`.  We exercise it
// by constructing a history whose total character count exceeds 240 000 chars
// (≈ 60 000 tokens at 4 chars/token).  After the single-round mock run the
// message list passed to the LLM must still fit within the budget, which proves
// the compaction path ran.
//
// We verify this indirectly: if compaction were broken the agent would either
// panic or the mock server would see a request body whose "messages" array
// represents far more tokens than the budget.  Instead we just confirm the
// agent completes successfully and produces the expected output.

#[tokio::test]
async fn test_compaction_triggered() {
    let canned = json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "Compaction OK.",
                "tool_calls": null
            }
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 3}
    });

    // Use an Arc<AtomicUsize> to capture how many tokens the mock server saw.
    let received_chars = Arc::new(AtomicUsize::new(0));
    let received_chars_clone = received_chars.clone();

    // Build a custom mock server that records the request body size before
    // handing back the canned response.
    let responses: Arc<Vec<Value>> = Arc::new(vec![canned]);
    let call_count = Arc::new(AtomicUsize::new(0));

    let app = {
        #[derive(Clone)]
        struct S {
            responses: Arc<Vec<Value>>,
            call_count: Arc<AtomicUsize>,
            received_chars: Arc<AtomicUsize>,
        }

        async fn handler(State(s): State<S>, body: axum::body::Bytes) -> impl IntoResponse {
            s.received_chars.fetch_add(body.len(), Ordering::SeqCst);
            let idx = s.call_count.fetch_add(1, Ordering::SeqCst);
            let resp = s
                .responses
                .get(idx)
                .cloned()
                .unwrap_or_else(|| json!({"error": "no more responses"}));
            axum::Json(resp)
        }

        Router::new()
            .route("/v1/chat/completions", post(handler))
            .with_state(S {
                responses,
                call_count,
                received_chars: received_chars_clone,
            })
    };

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind compaction mock");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("compaction mock error");
    });
    let base_url = format!("http://127.0.0.1:{}", addr.port());

    // Craft a history whose total content far exceeds 240 000 chars.
    // Each ChatMessage has a 40 001-char content; 6 of them = 240 006 chars
    // which crosses the 60 000-token (≈ 240 000 char) threshold.
    let big_content = "x".repeat(40_001);
    let fat_history: Vec<phantom_mesh::providers::traits::ChatMessage> = (0..6)
        .map(|i| phantom_mesh::providers::traits::ChatMessage {
            role: if i % 2 == 0 {
                "user".to_string()
            } else {
                "assistant".to_string()
            },
            content: big_content.clone(),
            tool_calls: None,
        })
        .collect();

    let mut config = config_with_mock(&base_url, vec![]);
    // Override budget to 60 000 tokens (≈ 240 000 chars) so the fat history
    // triggers compaction, matching the test design.
    config.token_budget = 60_000;
    let runtime = AgentRuntime::new(config);

    let result = runtime
        .run("master", "ping", &fat_history, None)
        .await
        .expect("run should not return Err with fat history");

    assert_eq!(
        result.output, "Compaction OK.",
        "unexpected output after compaction: {:?}",
        result.output
    );

    // The request body sent to the mock server must be substantially smaller
    // than the raw 240 000-char history (compaction dropped older messages).
    // We allow a generous upper bound — the key invariant is that the agent
    // didn't just forward the full un-compacted context.
    let sent = received_chars.load(Ordering::SeqCst);
    // 240 000 chars of history ÷ 4 = 60 000 tokens; budget is 60 000 tokens.
    // After compaction the sent body should be well under 240 000 chars of
    // message content.  We use 220 000 as a conservative ceiling.
    assert!(
        sent < 220_000,
        "expected compaction to reduce request size below 220 000 chars, got {} bytes",
        sent
    );
}
