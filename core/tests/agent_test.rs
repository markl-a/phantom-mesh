/// Integration tests for AgentRuntime and ConversationStore/CostTracker.
/// These tests use no real API calls — they exercise local state only.

use phantom_mesh::{
    agent::AgentRuntime,
    config::AgentsConfig,
    cost::CostTracker,
    session::ConversationStore,
    providers::traits::ChatMessage,
};

// ---------------------------------------------------------------------------
// 1. AgentRuntime creation
// ---------------------------------------------------------------------------

#[test]
fn test_agent_runtime_creation() {
    let rt = AgentRuntime::default();
    // Verify the config arc is accessible and has a default provider map.
    let cfg = rt.config();
    // Default config has no providers — just verify it doesn't panic and returns.
    let _ = cfg.providers.len();
    let _ = cfg.agent.len();
}

#[test]
fn test_agent_runtime_from_config() {
    let config = AgentsConfig::default();
    let rt = AgentRuntime::new(config);
    let cfg = rt.config();
    // Default config pre-populates a "master" agent entry pointing to "anthropic".
    assert!(cfg.agent.contains_key("master"), "default config should have a 'master' agent entry");
    assert!(cfg.providers.contains_key("anthropic"), "default config should have an 'anthropic' provider");
}

// ---------------------------------------------------------------------------
// 2. ConversationStore — storage and retrieval
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_session_storage_and_retrieval() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConversationStore::new_with_dir(dir.path().to_path_buf());

    let session_id = "test_session_123";
    let user_msg = ChatMessage {
        role: "user".into(),
        content: "hello".into(),
        tool_calls: None,
    };
    let asst_msg = ChatMessage {
        role: "assistant".into(),
        content: "world".into(),
        tool_calls: None,
    };
    store.append(session_id, user_msg, asst_msg).await;

    let history = store.get_history(session_id).await;
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].content, "hello");
    assert_eq!(history[1].content, "world");
}

// ---------------------------------------------------------------------------
// 3. Session list
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_session_list() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConversationStore::new_with_dir(dir.path().to_path_buf());

    for i in 0..3 {
        let user = ChatMessage { role: "user".into(), content: format!("msg {}", i), tool_calls: None };
        let asst = ChatMessage { role: "assistant".into(), content: format!("reply {}", i), tool_calls: None };
        store.append(&format!("session_{}", i), user, asst).await;
    }

    let list = store.list().await;
    assert_eq!(list.len(), 3, "should have three sessions");
    // IDs are sorted alphabetically
    assert!(list.contains(&"session_0".to_string()));
    assert!(list.contains(&"session_1".to_string()));
    assert!(list.contains(&"session_2".to_string()));
}

// ---------------------------------------------------------------------------
// 4. Session eviction
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_session_evict() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConversationStore::new_with_dir(dir.path().to_path_buf());

    let session_id = "evict_me";
    let user = ChatMessage { role: "user".into(), content: "soon gone".into(), tool_calls: None };
    let asst = ChatMessage { role: "assistant".into(), content: "bye".into(), tool_calls: None };
    store.append(session_id, user, asst).await;

    // Confirm messages are in cache.
    let before = store.get_history(session_id).await;
    assert_eq!(before.len(), 2);

    // Evict removes from in-memory cache but NOT from disk.
    store.evict(session_id).await;

    // A fresh get_history should reload from disk and still return 2 messages.
    let after = store.get_history(session_id).await;
    assert_eq!(after.len(), 2, "disk-backed messages survive in-memory eviction");
}

// ---------------------------------------------------------------------------
// 5. CostTracker
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_cost_tracker() {
    let tracker = CostTracker::new();

    // Snapshot request count before recording to handle pre-existing persisted data.
    let summary_before = tracker.summary().await;
    let requests_before = summary_before["requests"].as_u64().unwrap_or(0);

    // Ensure session cost starts clean for this test.
    tracker.reset_session().await;

    // Record some usage — model strings that match known pricing tiers.
    tracker.record("gpt-4o", 1_000, 500).await;
    tracker.record("claude-sonnet-4-5", 2_000, 1_000).await;

    let summary = tracker.summary().await;

    // Lifetime requests should have grown by exactly 2.
    let requests_after = summary["requests"].as_u64().unwrap_or(0);
    assert_eq!(requests_after, requests_before + 2, "should have recorded exactly 2 new requests");

    // Session cost should be > 0.
    let session_cost = tracker.session_cost().await;
    assert!(session_cost > 0.0, "session cost should be positive after recording tokens");

    // Last request cost should be > 0.
    let last = tracker.last_request_cost().await;
    assert!(last > 0.0, "last request cost should be positive");

    // Reset session and verify session counters clear.
    tracker.reset_session().await;
    let session_cost_after_reset = tracker.session_cost().await;
    assert_eq!(session_cost_after_reset, 0.0);
}
