//! Integration tests for [T1] — PhantomAgentDispatcher (real AgentRuntime
//! invocation via a mock OpenAI-compatible HTTP server).
//!
//! See docs/superpowers/plans/2026-05-15-track-t1-telegram-dispatch.md.

#![cfg(feature = "experimental-openclaw-telegram")]

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
    openclaw::telegram::{OpenclawDispatcher, OpenclawTelegramBot, OpenclawTelegramConfig},
    openclaw::telegram_agent_dispatcher::PhantomAgentDispatcher,
};

// ── Mock LLM server (mirrors core/tests/test_agent.rs helpers) ─────────────

#[derive(Clone)]
struct MockState {
    responses: Arc<Vec<Value>>,
    call_count: Arc<AtomicUsize>,
    seen_messages: Arc<tokio::sync::Mutex<Vec<Value>>>,
}

async fn mock_handler(
    State(state): State<MockState>,
    axum::Json(body): axum::Json<Value>,
) -> impl IntoResponse {
    // Record the `messages` array the runtime sent so per-chat-history
    // tests can inspect what the LLM saw.
    if let Some(msgs) = body.get("messages").cloned() {
        state.seen_messages.lock().await.push(msgs);
    }
    let idx = state.call_count.fetch_add(1, Ordering::SeqCst);
    let resp = state
        .responses
        .get(idx)
        .cloned()
        .unwrap_or_else(|| json!({"error": "no more canned responses"}));
    axum::Json(resp)
}

async fn start_mock(responses: Vec<Value>) -> (String, Arc<tokio::sync::Mutex<Vec<Value>>>) {
    let seen = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let state = MockState {
        responses: Arc::new(responses),
        call_count: Arc::new(AtomicUsize::new(0)),
        seen_messages: seen.clone(),
    };
    let app = Router::new()
        .route("/v1/chat/completions", post(mock_handler))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://127.0.0.1:{}", addr.port()), seen)
}

fn config_pointing_at(base_url: &str) -> AgentsConfig {
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
    let mut agents = std::collections::HashMap::new();
    agents.insert(
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
        agent: agents,
        ..Default::default()
    }
}

fn canned_reply(text: &str) -> Value {
    json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": text,
                "tool_calls": null
            }
        }],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1}
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────

/// Single message → dispatcher invokes runtime → returns the LLM output.
#[tokio::test]
async fn dispatch_round_trips_through_agent_runtime() {
    let (base_url, _seen) = start_mock(vec![canned_reply("pong from agent")]).await;
    let runtime = Arc::new(AgentRuntime::new(config_pointing_at(&base_url)));
    let dispatcher = PhantomAgentDispatcher::new(runtime, "master".into());

    let reply = dispatcher
        .dispatch_with_chat(42, "ping".into())
        .await
        .expect("dispatch should succeed");

    assert_eq!(reply, "pong from agent");
}

/// Two messages on the same chat_id must result in turn 2's LLM call
/// receiving turn 1's user + assistant messages in the `messages` array.
#[tokio::test]
async fn same_chat_id_carries_history_into_second_turn() {
    let (base_url, seen) = start_mock(vec![
        canned_reply("first reply"),
        canned_reply("second reply"),
    ])
    .await;
    let runtime = Arc::new(AgentRuntime::new(config_pointing_at(&base_url)));
    let dispatcher = PhantomAgentDispatcher::new(runtime, "master".into());

    let r1 = dispatcher
        .dispatch_with_chat(7777, "turn 1".into())
        .await
        .unwrap();
    assert_eq!(r1, "first reply");

    let r2 = dispatcher
        .dispatch_with_chat(7777, "turn 2".into())
        .await
        .unwrap();
    assert_eq!(r2, "second reply");

    // Inspect the messages array sent to the mock LLM on the SECOND call.
    let seen = seen.lock().await;
    assert_eq!(seen.len(), 2, "mock should have received exactly 2 calls");
    let second_call_messages = seen[1].as_array().expect("messages must be an array");

    // The second call's messages must include the turn-1 user+assistant
    // exchange. We don't assert exact ordering relative to system
    // messages (the runtime may prepend a system prompt); we just
    // assert presence of the two content strings.
    let contents: Vec<String> = second_call_messages
        .iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()).map(String::from))
        .collect();
    assert!(
        contents.iter().any(|c| c.contains("turn 1")),
        "turn-1 user text missing from turn-2 history: {:?}",
        contents
    );
    assert!(
        contents.iter().any(|c| c.contains("first reply")),
        "turn-1 assistant reply missing from turn-2 history: {:?}",
        contents
    );
    assert!(
        contents.iter().any(|c| c.contains("turn 2")),
        "turn-2 user text missing: {:?}",
        contents
    );

    // Also assert via the inspector that the dispatcher's internal
    // map holds the full 4-message trail.
    let stored = dispatcher.history_for(7777).await;
    assert_eq!(stored.len(), 4, "expected 2 turns × 2 messages = 4 entries");
    assert_eq!(stored[0].role, "user");
    assert_eq!(stored[0].content, "turn 1");
    assert_eq!(stored[1].role, "assistant");
    assert_eq!(stored[1].content, "first reply");
    assert_eq!(stored[2].role, "user");
    assert_eq!(stored[2].content, "turn 2");
    assert_eq!(stored[3].role, "assistant");
    assert_eq!(stored[3].content, "second reply");
}

/// Two messages on DIFFERENT chat_ids must NOT share history.
/// Chat B's turn-2 LLM call must NOT include chat A's prior messages.
#[tokio::test]
async fn different_chat_ids_are_isolated() {
    let (base_url, seen) = start_mock(vec![
        canned_reply("reply to chat A"),
        canned_reply("reply to chat B"),
    ])
    .await;
    let runtime = Arc::new(AgentRuntime::new(config_pointing_at(&base_url)));
    let dispatcher = PhantomAgentDispatcher::new(runtime, "master".into());

    let chat_a: i64 = 1001;
    let chat_b: i64 = 1002;

    let _ = dispatcher
        .dispatch_with_chat(chat_a, "secret from A".into())
        .await
        .unwrap();
    let _ = dispatcher
        .dispatch_with_chat(chat_b, "innocent from B".into())
        .await
        .unwrap();

    let seen = seen.lock().await;
    assert_eq!(seen.len(), 2);

    // The SECOND LLM call (for chat B) MUST NOT contain "secret from A"
    // in any message content.
    let chat_b_call = &seen[1];
    let chat_b_msgs = chat_b_call.as_array().expect("messages must be an array");
    let contents: Vec<String> = chat_b_msgs
        .iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()).map(String::from))
        .collect();
    for c in &contents {
        assert!(
            !c.contains("secret from A"),
            "chat B leaked chat A history: {:?}",
            contents
        );
        assert!(
            !c.contains("reply to chat A"),
            "chat B leaked chat A assistant reply: {:?}",
            contents
        );
    }

    // Sanity: each chat sees only its own 2 messages.
    assert_eq!(dispatcher.history_for(chat_a).await.len(), 2);
    assert_eq!(dispatcher.history_for(chat_b).await.len(), 2);
}

/// When the underlying agent runtime errors (no providers configured →
/// `All providers failed` from the mock returning HTTP errors / empty),
/// the dispatcher returns Err and OpenclawTelegramBot::handle_text must
/// translate that to the user-visible generic reply — WITHOUT leaking
/// the internal error text.
#[tokio::test]
async fn agent_error_becomes_user_visible_generic_reply() {
    // Use AgentsConfig::default() (no API key resolvable for anthropic
    // provider) AND ask for an agent name that does not exist → the
    // runtime returns Err with "No agent configuration" type message.
    let runtime = Arc::new(AgentRuntime::new(AgentsConfig::default()));
    let dispatcher = Arc::new(PhantomAgentDispatcher::new(
        runtime,
        "this-agent-does-not-exist".into(),
    ));

    let bot = OpenclawTelegramBot::new(
        OpenclawTelegramConfig {
            bot_token: "x".into(),
            allowed_user_ids: vec![],
        },
        dispatcher.clone(),
    );

    let reply = bot
        .handle_text(/* user_id */ 1, /* chat_id */ 2002, "ping".into())
        .await
        .expect("handle_text always returns Some when user is allowed");

    // 1. User sees the generic error reply.
    assert!(
        reply.contains("internal error"),
        "expected generic error, got: {:?}",
        reply
    );
    // 2. The internal error text must NOT be leaked. Common internal
    //    fragments that should never reach the user:
    for forbidden in &[
        "All providers failed",
        "this-agent-does-not-exist",
        "No agent configuration",
        "thread 'main' panicked",
    ] {
        assert!(
            !reply.contains(forbidden),
            "internal detail {:?} leaked to user reply: {:?}",
            forbidden,
            reply
        );
    }
}

/// History is bounded: with limit=4, after 3 turns (6 messages would
/// accumulate) the oldest user+assistant pair is evicted, leaving the
/// last 4 messages = 2 turns.
#[tokio::test]
async fn history_is_bounded_by_limit() {
    let (base_url, _seen) = start_mock(vec![
        canned_reply("r1"),
        canned_reply("r2"),
        canned_reply("r3"),
    ])
    .await;
    let runtime = Arc::new(AgentRuntime::new(config_pointing_at(&base_url)));
    // limit = 4 → keep last 2 user/assistant turns
    let dispatcher = PhantomAgentDispatcher::new_with_limit(runtime, "master".into(), 4);

    let chat_id = 9999_i64;
    dispatcher
        .dispatch_with_chat(chat_id, "msg 1".into())
        .await
        .unwrap();
    dispatcher
        .dispatch_with_chat(chat_id, "msg 2".into())
        .await
        .unwrap();
    dispatcher
        .dispatch_with_chat(chat_id, "msg 3".into())
        .await
        .unwrap();

    let stored = dispatcher.history_for(chat_id).await;
    assert_eq!(
        stored.len(),
        4,
        "history should be bounded to 4 messages, got {}",
        stored.len()
    );

    // The oldest pair ("msg 1" + "r1") must have been evicted; the
    // newest two turns must remain.
    let contents: Vec<&str> = stored.iter().map(|m| m.content.as_str()).collect();
    assert!(
        !contents.contains(&"msg 1"),
        "oldest user msg not evicted: {:?}",
        contents
    );
    assert!(
        !contents.contains(&"r1"),
        "oldest assistant msg not evicted: {:?}",
        contents
    );
    assert!(
        contents.contains(&"msg 2"),
        "expected msg 2 in: {:?}",
        contents
    );
    assert!(
        contents.contains(&"msg 3"),
        "expected msg 3 in: {:?}",
        contents
    );
}
