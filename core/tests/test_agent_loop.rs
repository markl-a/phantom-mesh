/// Integration tests for the AgentRuntime agent loop.
///
/// Each test spins up a minimal mock HTTP server that returns canned JSON
/// responses in the OpenAI chat-completions format, then drives
/// `AgentRuntime::run` (or `run_tracked`) against it.
///
/// Tests:
///   1. `test_agent_single_turn_no_tools` — simple answer, no tool calls
///   2. `test_agent_tool_call_then_answer` — one tool-call round then final answer
///   3. `test_agent_cost_tracking` — usage tokens are recorded in CostTracker
use std::sync::{Arc, Mutex};

use axum::routing::post;
use axum::Router;
use serde_json::json;
use tokio::net::TcpListener;

use phantom_mesh::{
    config::{AgentEntry, AgentsConfig, ProviderEntry},
    AgentRuntime, CostTracker,
};

// ── Mock server helpers ───────────────────────────────────────────────────────

/// Spin up a mock HTTP server that serves a *sequence* of JSON response bodies
/// in order (one per incoming request).  After the list is exhausted every
/// subsequent request receives the last body again.
///
/// Returns the server base URL, e.g. `"http://127.0.0.1:54321"`.
async fn start_json_mock(responses: Vec<serde_json::Value>) -> String {
    let responses = Arc::new(Mutex::new(responses));
    let call_idx = Arc::new(Mutex::new(0usize));

    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || {
            let responses = responses.clone();
            let call_idx = call_idx.clone();
            async move {
                let mut idx = call_idx.lock().unwrap();
                let responses = responses.lock().unwrap();
                let body = if *idx < responses.len() {
                    responses[*idx].clone()
                } else {
                    responses.last().cloned().unwrap_or_else(|| json!({}))
                };
                *idx += 1;
                axum::Json(body)
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock server");
    let addr = listener.local_addr().expect("local addr");

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("mock server error");
    });

    format!("http://127.0.0.1:{}", addr.port())
}

/// Build a minimal `AgentsConfig` whose single provider points at `base_url`.
///
/// `model` is used for the agent; pass a non-free model name to get non-zero
/// cost when testing `CostTracker`.
fn make_config(base_url: &str, model: &str, tools: Vec<String>) -> AgentsConfig {
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
            model: model.to_string(),
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

// ── Test 1: single turn, no tool calls ───────────────────────────────────────

#[tokio::test]
async fn test_agent_single_turn_no_tools() {
    // The mock server returns a plain assistant message — no tool_calls field.
    let response = json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "Hello from the mock LLM!"
            },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 5 }
    });

    let base_url = start_json_mock(vec![response]).await;
    let config = make_config(&base_url, "mock-model", vec![]);
    let runtime = AgentRuntime::new(config);

    let result = runtime
        .run("master", "Say hello", &[], None)
        .await
        .expect("run should succeed");

    assert_eq!(
        result.output, "Hello from the mock LLM!",
        "output should contain the canned assistant message, got: {:?}",
        result.output
    );
    assert_eq!(
        result.tool_calls_made.len(),
        0,
        "no tool calls should be recorded for a plain response"
    );
}

// ── Test 2: one tool-call round then final answer ─────────────────────────────

#[tokio::test]
async fn test_agent_tool_call_then_answer() {
    // First response: the LLM wants to call the `shell` tool.
    let tool_call_response = json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_001",
                    "type": "function",
                    "function": {
                        "name": "shell",
                        "arguments": "{\"command\":\"echo hello\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 20, "completion_tokens": 10 }
    });

    // Second response: the final answer after the tool result is fed back.
    let final_response = json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "The command output was: hello"
            },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 30, "completion_tokens": 15 }
    });

    let base_url = start_json_mock(vec![tool_call_response, final_response]).await;
    // Include "shell" in the tools list so the agent config sends tool schemas.
    let config = make_config(&base_url, "mock-model", vec!["shell".to_string()]);
    let runtime = AgentRuntime::new(config);

    let result = runtime
        .run("master", "Run echo hello", &[], None)
        .await
        .expect("run should succeed");

    assert_eq!(
        result.tool_calls_made.len(),
        1,
        "exactly one tool call should be recorded"
    );
    assert_eq!(
        result.tool_calls_made[0]["tool"].as_str().unwrap_or(""),
        "shell",
        "recorded tool name should be 'shell'"
    );
    assert!(
        result.output.contains("hello"),
        "final output should contain the final answer text, got: {:?}",
        result.output
    );
}

// ── Test 3: cost tracking ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_agent_cost_tracking() {
    // Use a model with a known non-zero price so we can assert cost increases.
    // `gpt-4o` maps to (2.5 input, 10.0 output) USD per 1M tokens.
    // With 100 prompt tokens and 50 completion tokens:
    //   cost = (100/1e6)*2.5 + (50/1e6)*10.0 = 0.00025 + 0.0005 = 0.00075 USD
    let response = json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "Cost tracking response."
            },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 100, "completion_tokens": 50 }
    });

    let base_url = start_json_mock(vec![response]).await;
    // Use "gpt-4o" as model name so cost is non-zero.
    let config = make_config(&base_url, "gpt-4o", vec![]);
    let runtime = AgentRuntime::new(config);

    let tracker = CostTracker::new();

    // Snapshot total_usd before the call (the tracker may load existing data from disk).
    let before_usd = tracker.summary().await["total_usd"]
        .as_f64()
        .expect("total_usd should be a number before the call");

    runtime
        .run_tracked("master", "Track my cost", &[], None, &tracker)
        .await
        .expect("run_tracked should succeed");

    let summary = tracker.summary().await;

    let after_usd = summary["total_usd"]
        .as_f64()
        .expect("total_usd should be a number after the call");

    assert!(
        after_usd > before_usd,
        "total_usd should increase after a tracked call (before={before_usd}, after={after_usd})"
    );
}
