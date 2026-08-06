use spectyn_mesh::{cost::CostTracker, providers::traits::ChatMessage, session::ConversationStore};
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn user_msg(content: &str) -> ChatMessage {
    ChatMessage {
        role: "user".into(),
        content: content.into(),
        tool_calls: None,
    }
}

fn asst_msg(content: &str) -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: content.into(),
        tool_calls: None,
    }
}

// ---------------------------------------------------------------------------
// ConversationStore tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_session_empty_history() {
    let store = ConversationStore::new();
    let history = store
        .get_history("unknown_chat_id_that_does_not_exist")
        .await;
    assert!(
        history.is_empty(),
        "expected empty history for unknown chat id"
    );
}

#[tokio::test]
async fn test_session_append_persists() {
    let dir = tempdir().unwrap();
    let store = ConversationStore::new_with_dir(dir.path().to_path_buf());

    store
        .append("persist-chat", user_msg("hello"), asst_msg("world"))
        .await;

    // Create a brand-new store pointing at the same directory — no shared cache.
    let store2 = ConversationStore::new_with_dir(dir.path().to_path_buf());
    let history = store2.get_history("persist-chat").await;

    assert_eq!(history.len(), 2, "expected 2 messages loaded from disk");
    assert_eq!(history[0].role, "user");
    assert_eq!(history[0].content, "hello");
    assert_eq!(history[1].role, "assistant");
    assert_eq!(history[1].content, "world");
}

#[tokio::test]
async fn test_session_multiple_chats() {
    let dir = tempdir().unwrap();
    let store = ConversationStore::new_with_dir(dir.path().to_path_buf());

    store
        .append("chat1", user_msg("chat1-user"), asst_msg("chat1-asst"))
        .await;
    store
        .append("chat2", user_msg("chat2-user"), asst_msg("chat2-asst"))
        .await;

    let h1 = store.get_history("chat1").await;
    let h2 = store.get_history("chat2").await;

    assert_eq!(h1.len(), 2);
    assert!(h1.iter().all(|m| m.content.starts_with("chat1")));

    assert_eq!(h2.len(), 2);
    assert!(h2.iter().all(|m| m.content.starts_with("chat2")));
}

#[tokio::test]
async fn test_session_concurrent_appends() {
    use std::sync::Arc;
    use tokio::task::JoinSet;

    let dir = tempdir().unwrap();
    let store = Arc::new(ConversationStore::new_with_dir(dir.path().to_path_buf()));

    let mut set = JoinSet::new();
    for i in 0..10 {
        let s = store.clone();
        set.spawn(async move {
            s.append(
                "concurrent-chat",
                user_msg(&format!("user-{i}")),
                asst_msg(&format!("asst-{i}")),
            )
            .await;
        });
    }
    while let Some(res) = set.join_next().await {
        res.expect("task panicked");
    }

    let history = store.get_history("concurrent-chat").await;
    assert_eq!(
        history.len(),
        20,
        "expected 20 messages (10 user + 10 assistant), got {}",
        history.len()
    );
}

#[tokio::test]
async fn test_session_list() {
    let dir = tempdir().unwrap();
    let store = ConversationStore::new_with_dir(dir.path().to_path_buf());

    store
        .append("alpha", user_msg("hi"), asst_msg("hello"))
        .await;
    store.append("beta", user_msg("hey"), asst_msg("yo")).await;

    let list = store.list().await;
    assert!(
        list.contains(&"alpha".to_string()),
        "list should contain 'alpha'"
    );
    assert!(
        list.contains(&"beta".to_string()),
        "list should contain 'beta'"
    );
}

#[tokio::test]
async fn test_session_evict() {
    let dir = tempdir().unwrap();
    let store = ConversationStore::new_with_dir(dir.path().to_path_buf());

    store
        .append("evict-chat", user_msg("keep me"), asst_msg("I persist"))
        .await;

    // Evict from in-memory cache.
    store.evict("evict-chat").await;

    // get_history should reload from disk transparently.
    let history = store.get_history("evict-chat").await;
    assert_eq!(
        history.len(),
        2,
        "expected history reloaded from disk after eviction"
    );
    assert_eq!(history[0].content, "keep me");
    assert_eq!(history[1].content, "I persist");
}

// ---------------------------------------------------------------------------
// CostTracker tests
// ---------------------------------------------------------------------------

/// Build a fresh CostTracker whose backing file lives in a temp dir so that
/// tests are isolated from ~/.spectyn-mesh/costs.json and from each other.
fn fresh_tracker(dir: &tempfile::TempDir) -> CostTracker {
    // CostTracker::new() reads HOME to pick its path. Point HOME at the
    // tempdir so the backing file is isolated per-test.
    std::env::set_var("HOME", dir.path());
    CostTracker::new()
}

#[tokio::test]
async fn test_cost_starts_zero() {
    let dir = tempdir().unwrap();
    let tracker = fresh_tracker(&dir);
    let summary = tracker.summary().await;

    assert_eq!(
        summary["total_usd"].as_f64().unwrap(),
        0.0,
        "fresh CostTracker should start at $0"
    );
    assert_eq!(
        summary["requests"].as_u64().unwrap(),
        0,
        "fresh CostTracker should have 0 requests"
    );
}

#[tokio::test]
async fn test_cost_record_claude_sonnet() {
    let dir = tempdir().unwrap();
    let tracker = fresh_tracker(&dir);

    // 1_000_000 prompt tokens @ $3.0/MTok  = $3.00
    // 500_000 completion tokens @ $15.0/MTok = $7.50
    // Total expected: $10.50
    tracker.record("claude-sonnet-4", 1_000_000, 500_000).await;

    let summary = tracker.summary().await;
    let total = summary["total_usd"].as_f64().unwrap();
    assert!(
        (total - 10.50).abs() < 0.01,
        "expected $10.50 for claude-sonnet-4, got ${total:.4}"
    );
}

#[tokio::test]
async fn test_cost_record_unknown_model() {
    let dir = tempdir().unwrap();
    let tracker = fresh_tracker(&dir);

    // Fallback price: $1.0/MTok prompt, $5.0/MTok completion
    // 1M prompt  = $1.00
    // 1M completion = $5.00
    // Total expected: $6.00
    tracker
        .record("unknown-model-xyz", 1_000_000, 1_000_000)
        .await;

    let summary = tracker.summary().await;
    let total = summary["total_usd"].as_f64().unwrap();
    assert!(
        (total - 6.00).abs() < 0.01,
        "expected $6.00 for unknown model fallback, got ${total:.4}"
    );
}

#[tokio::test]
async fn test_cost_accumulates() {
    let dir = tempdir().unwrap();
    let tracker = fresh_tracker(&dir);

    // First call: claude-sonnet-4 — 1M prompt ($3.0) + 500K completion ($7.5) = $10.50
    tracker.record("claude-sonnet-4", 1_000_000, 500_000).await;
    // Second call: unknown model — 1M prompt ($1.0) + 1M completion ($5.0) = $6.00
    tracker
        .record("unknown-model-xyz", 1_000_000, 1_000_000)
        .await;

    let summary = tracker.summary().await;
    let total = summary["total_usd"].as_f64().unwrap();
    assert!(
        (total - 16.50).abs() < 0.01,
        "expected accumulated total of $16.50, got ${total:.4}"
    );
}

#[tokio::test]
async fn test_cost_requests_count() {
    let dir = tempdir().unwrap();
    let tracker = fresh_tracker(&dir);

    tracker.record("claude-sonnet-4", 100, 100).await;
    tracker.record("claude-sonnet-4", 100, 100).await;
    tracker.record("claude-sonnet-4", 100, 100).await;

    let summary = tracker.summary().await;
    assert_eq!(
        summary["requests"].as_u64().unwrap(),
        3,
        "expected request count of 3"
    );
}
