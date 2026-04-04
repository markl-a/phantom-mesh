// ConversationStore — per-chat conversation memory
// Layer 1: in-memory VecDeque buffer (fast, max 20 messages)
// Layer 2: SQLite persistence (survives daemon restarts)

use anyhow::Result;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::llm_router::ChatMessage;

const MAX_MESSAGES: usize = 10; // 5 turns (user + assistant) — smaller to avoid context pollution
const STALE_SECS: u64 = 3600;  // Clean up after 1 hour idle

struct Session {
    messages: VecDeque<ChatMessage>,
    updated_at: Instant,
}

pub struct ConversationStore {
    buffers: RwLock<HashMap<String, Session>>,
    db_path: String,
}

impl ConversationStore {
    /// Create store and load recent sessions from SQLite
    pub async fn new(db_path: &str) -> Result<Self> {
        // Create table (blocking, only at startup)
        let db = db_path.to_string();
        let loaded = tokio::task::spawn_blocking({
            let db = db.clone();
            move || -> Result<HashMap<String, Session>> {
                let conn = rusqlite::Connection::open(&db)?;
                conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS sessions (
                        chat_id     TEXT PRIMARY KEY,
                        messages    TEXT NOT NULL,
                        updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
                    );"
                )?;

                // Load sessions from last 7 days
                let mut stmt = conn.prepare(
                    "SELECT chat_id, messages FROM sessions
                     WHERE datetime(updated_at) > datetime('now', '-7 days')"
                )?;

                let mut buffers = HashMap::new();
                let rows = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                    ))
                })?;

                for row in rows {
                    let (chat_id, json) = row?;
                    if let Ok(msgs) = serde_json::from_str::<Vec<ChatMessage>>(&json) {
                        let mut deque = VecDeque::from(msgs);
                        while deque.len() > MAX_MESSAGES {
                            deque.pop_front();
                        }
                        buffers.insert(chat_id, Session {
                            messages: deque,
                            updated_at: Instant::now(),
                        });
                    }
                }

                Ok(buffers)
            }
        }).await??;

        let count = loaded.len();
        if count > 0 {
            info!("Loaded {} conversation sessions from DB", count);
        }

        Ok(Self {
            buffers: RwLock::new(loaded),
            db_path: db,
        })
    }

    /// Get conversation history for a chat (cloned, safe to use)
    pub async fn get_history(&self, chat_id: &str) -> Vec<ChatMessage> {
        let buffers = self.buffers.read().await;
        buffers.get(chat_id)
            .map(|s| s.messages.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Append a user + assistant turn, then persist to SQLite
    pub async fn append(&self, chat_id: &str, user_msg: ChatMessage, assistant_msg: ChatMessage) {
        let messages_json = {
            let mut buffers = self.buffers.write().await;
            let session = buffers.entry(chat_id.to_string()).or_insert_with(|| Session {
                messages: VecDeque::new(),
                updated_at: Instant::now(),
            });

            session.messages.push_back(user_msg);
            session.messages.push_back(assistant_msg);
            session.updated_at = Instant::now();

            // Trim oldest
            while session.messages.len() > MAX_MESSAGES {
                session.messages.pop_front();
            }

            debug!("Chat {} now has {} messages in buffer", chat_id, session.messages.len());

            // Serialize for DB write
            let msgs: Vec<&ChatMessage> = session.messages.iter().collect();
            serde_json::to_string(&msgs).unwrap_or_default()
        };
        // RwLock released here

        // Persist async (best-effort, don't block the handler)
        let db_path = self.db_path.clone();
        let cid = chat_id.to_string();
        tokio::task::spawn_blocking(move || {
            if let Ok(conn) = rusqlite::Connection::open(&db_path) {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO sessions (chat_id, messages, updated_at)
                     VALUES (?1, ?2, datetime('now'))",
                    rusqlite::params![cid, messages_json],
                );
            }
        });
    }

    /// Clear conversation for a chat
    pub async fn clear(&self, chat_id: &str) {
        {
            let mut buffers = self.buffers.write().await;
            buffers.remove(chat_id);
        }

        let db_path = self.db_path.clone();
        let cid = chat_id.to_string();
        tokio::task::spawn_blocking(move || {
            if let Ok(conn) = rusqlite::Connection::open(&db_path) {
                let _ = conn.execute(
                    "DELETE FROM sessions WHERE chat_id = ?1",
                    rusqlite::params![cid],
                );
            }
        });

        info!("Cleared conversation for chat {}", chat_id);
    }

    /// Get message count for a chat
    pub async fn message_count(&self, chat_id: &str) -> usize {
        let buffers = self.buffers.read().await;
        buffers.get(chat_id).map(|s| s.messages.len()).unwrap_or(0)
    }

    /// Get number of active conversation sessions
    pub async fn active_count(&self) -> usize {
        self.buffers.read().await.len()
    }

    /// Clean up stale in-memory sessions (called periodically)
    pub async fn cleanup_stale(&self) -> usize {
        let mut buffers = self.buffers.write().await;
        let before = buffers.len();
        let cutoff = Instant::now() - std::time::Duration::from_secs(STALE_SECS);
        buffers.retain(|_, s| s.updated_at > cutoff);
        let removed = before - buffers.len();
        if removed > 0 {
            debug!("Cleaned up {} stale conversation sessions", removed);
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Helper: create a ConversationStore backed by a tempfile DB.
    /// Returns (store, TempDir) — keep TempDir alive for the test duration.
    async fn make_store() -> (ConversationStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let db = dir.path().join("test_conv.db");
        let store = ConversationStore::new(db.to_str().unwrap())
            .await
            .expect("failed to create ConversationStore");
        (store, dir)
    }

    // ---------------------------------------------------------------
    // 1. Create a new session
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_create_new_session() {
        let (store, _dir) = make_store().await;

        // Appending to a chat_id implicitly creates a session
        store.append("new_session", make_msg("user", "hello"), make_msg("assistant", "hi")).await;

        assert_eq!(store.active_count().await, 1);
        assert_eq!(store.message_count("new_session").await, 2);
    }

    // ---------------------------------------------------------------
    // 2. Add messages to session
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_add_messages_to_session() {
        let (store, _dir) = make_store().await;

        store.append("chat1", make_msg("user", "hello"), make_msg("assistant", "hi")).await;
        store.append("chat1", make_msg("user", "how?"), make_msg("assistant", "fine")).await;
        store.append("chat1", make_msg("user", "bye"), make_msg("assistant", "later")).await;

        assert_eq!(store.message_count("chat1").await, 6);
        let history = store.get_history("chat1").await;
        assert_eq!(history.len(), 6);
    }

    // ---------------------------------------------------------------
    // 3. Retrieve message history
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_retrieve_message_history() {
        let (store, _dir) = make_store().await;

        store.append("chat1", make_msg("user", "hello"), make_msg("assistant", "hi")).await;

        let history = store.get_history("chat1").await;
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[0].content, "hello");
        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[1].content, "hi");
    }

    // ---------------------------------------------------------------
    // 4. Session isolation
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_session_isolation() {
        let (store, _dir) = make_store().await;

        store.append("session_A", make_msg("user", "alpha"), make_msg("assistant", "a_reply")).await;
        store.append("session_B", make_msg("user", "beta"), make_msg("assistant", "b_reply")).await;

        let hist_a = store.get_history("session_A").await;
        let hist_b = store.get_history("session_B").await;

        // A's messages must not leak into B and vice versa
        assert_eq!(hist_a.len(), 2);
        assert_eq!(hist_b.len(), 2);
        assert_eq!(hist_a[0].content, "alpha");
        assert_eq!(hist_b[0].content, "beta");

        // Clearing A must not affect B
        store.clear("session_A").await;
        assert!(store.get_history("session_A").await.is_empty());
        assert_eq!(store.get_history("session_B").await.len(), 2);
    }

    // ---------------------------------------------------------------
    // 5. Prune old sessions (by age)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_prune_stale_sessions() {
        let (store, _dir) = make_store().await;

        store.append("chat_old", make_msg("user", "old"), make_msg("assistant", "old")).await;
        store.append("chat_fresh", make_msg("user", "fresh"), make_msg("assistant", "fresh")).await;

        // Manually age "chat_old" by setting updated_at to the past
        {
            let mut buffers = store.buffers.write().await;
            if let Some(session) = buffers.get_mut("chat_old") {
                session.updated_at = Instant::now() - std::time::Duration::from_secs(STALE_SECS + 60);
            }
        }

        let removed = store.cleanup_stale().await;
        assert_eq!(removed, 1);
        assert_eq!(store.active_count().await, 1);

        // chat_old should be gone, chat_fresh should remain
        assert!(store.get_history("chat_old").await.is_empty());
        assert_eq!(store.get_history("chat_fresh").await.len(), 2);
    }

    // ---------------------------------------------------------------
    // 6. Message ordering (oldest first)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_message_ordering_oldest_first() {
        let (store, _dir) = make_store().await;

        store.append("chat1", make_msg("user", "first"), make_msg("assistant", "first_reply")).await;
        store.append("chat1", make_msg("user", "second"), make_msg("assistant", "second_reply")).await;
        store.append("chat1", make_msg("user", "third"), make_msg("assistant", "third_reply")).await;

        let history = store.get_history("chat1").await;
        let contents: Vec<&str> = history.iter().map(|m| m.content.as_str()).collect();
        assert_eq!(
            contents,
            vec!["first", "first_reply", "second", "second_reply", "third", "third_reply"]
        );
    }

    // ---------------------------------------------------------------
    // 7. Message limit enforcement (MAX_MESSAGES)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_message_limit_enforcement() {
        let (store, _dir) = make_store().await;

        // Add 8 turns = 16 messages; MAX_MESSAGES = 10 so oldest are trimmed
        for i in 0..8 {
            store.append(
                "chat1",
                make_msg("user", &format!("msg_{}", i)),
                make_msg("assistant", &format!("reply_{}", i)),
            ).await;
        }

        let history = store.get_history("chat1").await;
        assert_eq!(history.len(), MAX_MESSAGES);
        // With 16 pushed and 10 kept, we lose the first 6 messages (turns 0-2).
        // Remaining: msg_3, reply_3, msg_4, reply_4, ..., msg_7, reply_7
        assert_eq!(history[0].content, "msg_3");
        assert_eq!(history[1].content, "reply_3");
        assert_eq!(history[history.len() - 1].content, "reply_7");
    }

    // ---------------------------------------------------------------
    // 8. Empty session returns empty history
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_empty_session_returns_empty_history() {
        let (store, _dir) = make_store().await;

        let history = store.get_history("nonexistent_chat_id").await;
        assert!(history.is_empty());
        assert_eq!(store.message_count("nonexistent_chat_id").await, 0);
    }

    // ---------------------------------------------------------------
    // 9. Non-existent session handling
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_nonexistent_session_operations() {
        let (store, _dir) = make_store().await;

        // get_history on missing session returns empty vec
        assert!(store.get_history("ghost").await.is_empty());

        // message_count on missing session returns 0
        assert_eq!(store.message_count("ghost").await, 0);

        // clear on missing session should not panic
        store.clear("ghost").await;

        // active_count should still be 0
        assert_eq!(store.active_count().await, 0);
    }

    // ---------------------------------------------------------------
    // 10. Session metadata — roles are preserved correctly
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_session_metadata_roles_preserved() {
        let (store, _dir) = make_store().await;

        let user_msg = ChatMessage {
            role: "user".to_string(),
            content: "test content".to_string(),
            tool_calls: None,
            tool_call_id: Some("tc_001".to_string()),
        };
        let assistant_msg = ChatMessage {
            role: "assistant".to_string(),
            content: "response".to_string(),
            tool_calls: None,
            tool_call_id: None,
        };

        store.append("meta_chat", user_msg, assistant_msg).await;

        let history = store.get_history("meta_chat").await;
        assert_eq!(history[0].role, "user");
        assert_eq!(history[0].tool_call_id, Some("tc_001".to_string()));
        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[1].tool_call_id, None);
    }

    // ---------------------------------------------------------------
    // 11. Large message handling
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_large_message_handling() {
        let (store, _dir) = make_store().await;

        // Create a 100 KB message
        let large_content = "x".repeat(100_000);
        store.append(
            "big_chat",
            make_msg("user", &large_content),
            make_msg("assistant", "ack"),
        ).await;

        let history = store.get_history("big_chat").await;
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content.len(), 100_000);
        assert_eq!(history[1].content, "ack");
    }

    // ---------------------------------------------------------------
    // 12. Multiple concurrent sessions
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_multiple_concurrent_sessions() {
        let (store, _dir) = make_store().await;
        let store = Arc::new(store);

        // Spawn 20 concurrent tasks each writing to a different session
        let mut handles = Vec::new();
        for i in 0..20 {
            let s = Arc::clone(&store);
            handles.push(tokio::spawn(async move {
                let chat_id = format!("concurrent_{}", i);
                s.append(
                    &chat_id,
                    make_msg("user", &format!("hello from {}", i)),
                    make_msg("assistant", &format!("hi {}", i)),
                ).await;
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        assert_eq!(store.active_count().await, 20);

        // Verify each session has exactly 2 messages with correct content
        for i in 0..20 {
            let chat_id = format!("concurrent_{}", i);
            let history = store.get_history(&chat_id).await;
            assert_eq!(history.len(), 2, "session {} should have 2 messages", i);
            assert_eq!(history[0].content, format!("hello from {}", i));
            assert_eq!(history[1].content, format!("hi {}", i));
        }
    }

    // ---------------------------------------------------------------
    // 13. Delete session
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_delete_session() {
        let (store, _dir) = make_store().await;

        store.append("doomed", make_msg("user", "soon gone"), make_msg("assistant", "bye")).await;
        store.append("survivor", make_msg("user", "still here"), make_msg("assistant", "yes")).await;

        assert_eq!(store.active_count().await, 2);

        store.clear("doomed").await;

        assert_eq!(store.active_count().await, 1);
        assert!(store.get_history("doomed").await.is_empty());
        assert_eq!(store.message_count("doomed").await, 0);

        // Survivor is unaffected
        assert_eq!(store.get_history("survivor").await.len(), 2);
    }

    // ---------------------------------------------------------------
    // 14. Session count / stats
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_session_count_stats() {
        let (store, _dir) = make_store().await;

        assert_eq!(store.active_count().await, 0);

        store.append("s1", make_msg("user", "a"), make_msg("assistant", "b")).await;
        assert_eq!(store.active_count().await, 1);
        assert_eq!(store.message_count("s1").await, 2);

        store.append("s2", make_msg("user", "c"), make_msg("assistant", "d")).await;
        store.append("s2", make_msg("user", "e"), make_msg("assistant", "f")).await;
        assert_eq!(store.active_count().await, 2);
        assert_eq!(store.message_count("s2").await, 4);

        store.clear("s1").await;
        assert_eq!(store.active_count().await, 1);
        assert_eq!(store.message_count("s1").await, 0);
    }

    // ---------------------------------------------------------------
    // 15. Persistence across instances (survives restart)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_persistence_across_instances() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let db_path = dir.path().join("persist.db");
        let db_str = db_path.to_str().unwrap();

        // Instance 1: write data
        {
            let store = ConversationStore::new(db_str).await.unwrap();
            store.append("chat1", make_msg("user", "remember me"), make_msg("assistant", "ok")).await;
            // Wait for spawn_blocking DB write to complete
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        // Instance 2: read data (simulates daemon restart)
        {
            let store = ConversationStore::new(db_str).await.unwrap();
            let history = store.get_history("chat1").await;
            assert_eq!(history.len(), 2);
            assert_eq!(history[0].content, "remember me");
            assert_eq!(history[1].content, "ok");
        }
    }

    // ---------------------------------------------------------------
    // 16. Clear then re-append (session reuse)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_clear_then_reappend() {
        let (store, _dir) = make_store().await;

        store.append("chat1", make_msg("user", "before"), make_msg("assistant", "before_r")).await;
        assert_eq!(store.message_count("chat1").await, 2);

        store.clear("chat1").await;
        assert_eq!(store.message_count("chat1").await, 0);

        store.append("chat1", make_msg("user", "after"), make_msg("assistant", "after_r")).await;
        let history = store.get_history("chat1").await;
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content, "after");
        assert_eq!(history[1].content, "after_r");
    }

    // ---------------------------------------------------------------
    // 17. Cleanup stale does not remove fresh sessions
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_cleanup_stale_preserves_fresh() {
        let (store, _dir) = make_store().await;

        store.append("fresh1", make_msg("user", "hi"), make_msg("assistant", "hey")).await;
        store.append("fresh2", make_msg("user", "yo"), make_msg("assistant", "sup")).await;

        let removed = store.cleanup_stale().await;
        assert_eq!(removed, 0);
        assert_eq!(store.active_count().await, 2);
    }

    // ---------------------------------------------------------------
    // 18. Trimming preserves newest messages
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_trim_preserves_newest() {
        let (store, _dir) = make_store().await;

        // Fill exactly to MAX_MESSAGES (5 turns = 10 messages)
        for i in 0..5 {
            store.append(
                "chat1",
                make_msg("user", &format!("u{}", i)),
                make_msg("assistant", &format!("a{}", i)),
            ).await;
        }
        assert_eq!(store.message_count("chat1").await, MAX_MESSAGES);

        // Add one more turn — should evict the oldest 2 messages
        store.append("chat1", make_msg("user", "u5"), make_msg("assistant", "a5")).await;

        let history = store.get_history("chat1").await;
        assert_eq!(history.len(), MAX_MESSAGES);
        // The newest message must always be the last one appended
        assert_eq!(history[history.len() - 1].content, "a5");
        assert_eq!(history[history.len() - 2].content, "u5");
        // The oldest surviving message should be u1 (u0 and a0 evicted)
        assert_eq!(history[0].content, "u1");
    }

    // ---------------------------------------------------------------
    // 19. Persistence respects MAX_MESSAGES on reload
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_persistence_trims_on_reload() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let db_path = dir.path().join("trim_reload.db");
        let db_str = db_path.to_str().unwrap();

        // Instance 1: write exactly MAX_MESSAGES
        {
            let store = ConversationStore::new(db_str).await.unwrap();
            for i in 0..5 {
                store.append(
                    "chat1",
                    make_msg("user", &format!("u{}", i)),
                    make_msg("assistant", &format!("a{}", i)),
                ).await;
            }
            assert_eq!(store.message_count("chat1").await, MAX_MESSAGES);
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        // Instance 2: reload — should still have MAX_MESSAGES
        {
            let store = ConversationStore::new(db_str).await.unwrap();
            let history = store.get_history("chat1").await;
            assert!(history.len() <= MAX_MESSAGES);
            assert!(!history.is_empty());
        }
    }

    // ---------------------------------------------------------------
    // 20. Concurrent writes to the SAME session
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_concurrent_writes_same_session() {
        let (store, _dir) = make_store().await;
        let store = Arc::new(store);

        // Spawn 5 concurrent tasks all writing to the same session
        let mut handles = Vec::new();
        for i in 0..5 {
            let s = Arc::clone(&store);
            handles.push(tokio::spawn(async move {
                s.append(
                    "shared_chat",
                    make_msg("user", &format!("concurrent_u{}", i)),
                    make_msg("assistant", &format!("concurrent_a{}", i)),
                ).await;
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        // 5 appends x 2 messages = 10, which is exactly MAX_MESSAGES
        let count = store.message_count("shared_chat").await;
        assert!(count <= MAX_MESSAGES, "count {} should be <= MAX_MESSAGES {}", count, MAX_MESSAGES);
        assert!(count > 0, "should have at least some messages");

        // History should be consistent (no panics, no corruption)
        let history = store.get_history("shared_chat").await;
        assert_eq!(history.len(), count);
    }

    // ---------------------------------------------------------------
    // 21. Unicode and special character messages
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_unicode_messages() {
        let (store, _dir) = make_store().await;

        let unicode_content = "Hello \u{1F600} \u{4F60}\u{597D} \u{0410}\u{043B}\u{043B}\u{043E} \u{3053}\u{3093}\u{306B}\u{3061}\u{306F}";
        store.append(
            "unicode_chat",
            make_msg("user", unicode_content),
            make_msg("assistant", "\u{1F44D} acknowledged"),
        ).await;

        let history = store.get_history("unicode_chat").await;
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content, unicode_content);
        assert!(history[1].content.contains("acknowledged"));
    }

    // ---------------------------------------------------------------
    // 22. Tool call metadata is preserved in history
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_tool_call_metadata_preserved() {
        let (store, _dir) = make_store().await;

        let user_msg = make_msg("user", "search for cats");
        let assistant_msg = ChatMessage {
            role: "assistant".to_string(),
            content: "I found some results".to_string(),
            tool_calls: Some(vec![crate::llm_router::ToolCall {
                id: Some("call_123".to_string()),
                function: crate::llm_router::ToolCallFunction {
                    name: "web_search".to_string(),
                    arguments: serde_json::json!({"query": "cats"}),
                },
            }]),
            tool_call_id: None,
        };

        store.append("tool_chat", user_msg, assistant_msg).await;

        let history = store.get_history("tool_chat").await;
        assert_eq!(history.len(), 2);
        let tc = history[1].tool_calls.as_ref().expect("tool_calls should be Some");
        assert_eq!(tc.len(), 1);
        assert_eq!(tc[0].id, Some("call_123".to_string()));
        assert_eq!(tc[0].function.name, "web_search");
    }

    // ---------------------------------------------------------------
    // 23. Persistence of cleared session is gone on reload
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_persistence_clear_survives_reload() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let db_path = dir.path().join("clear_persist.db");
        let db_str = db_path.to_str().unwrap();

        // Instance 1: write then clear
        {
            let store = ConversationStore::new(db_str).await.unwrap();
            store.append("chat1", make_msg("user", "ephemeral"), make_msg("assistant", "ok")).await;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            store.clear("chat1").await;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        // Instance 2: should not find the cleared session
        {
            let store = ConversationStore::new(db_str).await.unwrap();
            let history = store.get_history("chat1").await;
            assert!(history.is_empty(), "cleared session should not survive reload");
        }
    }

    // ---------------------------------------------------------------
    // 24. Empty content messages
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_empty_content_messages() {
        let (store, _dir) = make_store().await;

        store.append("chat1", make_msg("user", ""), make_msg("assistant", "")).await;

        let history = store.get_history("chat1").await;
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content, "");
        assert_eq!(history[1].content, "");
    }
}
