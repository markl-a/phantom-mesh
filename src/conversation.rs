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

    fn make_msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[tokio::test]
    async fn test_append_and_get() {
        let dir = std::env::temp_dir().join("clawtex_test_conv_1");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("test.db");
        let store = ConversationStore::new(db.to_str().unwrap()).await.unwrap();

        store.append("chat1", make_msg("user", "hello"), make_msg("assistant", "hi")).await;

        let history = store.get_history("chat1").await;
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content, "hello");
        assert_eq!(history[1].content, "hi");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_clear() {
        let dir = std::env::temp_dir().join("clawtex_test_conv_2");
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("test.db");
        let store = ConversationStore::new(db.to_str().unwrap()).await.unwrap();

        store.append("chat1", make_msg("user", "hello"), make_msg("assistant", "hi")).await;
        store.clear("chat1").await;

        let history = store.get_history("chat1").await;
        assert_eq!(history.len(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_max_messages_trim() {
        let dir = std::env::temp_dir().join("clawtex_test_conv_3");
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("test.db");
        let store = ConversationStore::new(db.to_str().unwrap()).await.unwrap();

        // Add 8 turns = 16 messages, should trim to 10
        for i in 0..8 {
            store.append(
                "chat1",
                make_msg("user", &format!("msg {}", i)),
                make_msg("assistant", &format!("reply {}", i)),
            ).await;
        }

        let history = store.get_history("chat1").await;
        assert_eq!(history.len(), MAX_MESSAGES);
        // Oldest should have been trimmed — first message should be msg 3
        assert_eq!(history[0].content, "msg 3");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_persistence_across_instances() {
        let dir = std::env::temp_dir().join("clawtex_test_conv_4");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join("test.db");
        let db_str = db_path.to_str().unwrap();

        // Instance 1: write data
        {
            let store = ConversationStore::new(db_str).await.unwrap();
            store.append("chat1", make_msg("user", "remember me"), make_msg("assistant", "ok")).await;
            // Wait for spawn_blocking to complete
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        // Instance 2: read data (simulates daemon restart)
        {
            let store = ConversationStore::new(db_str).await.unwrap();
            let history = store.get_history("chat1").await;
            assert_eq!(history.len(), 2);
            assert_eq!(history[0].content, "remember me");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_separate_chats() {
        let dir = std::env::temp_dir().join("clawtex_test_conv_5");
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("test.db");
        let store = ConversationStore::new(db.to_str().unwrap()).await.unwrap();

        store.append("chat_a", make_msg("user", "a msg"), make_msg("assistant", "a reply")).await;
        store.append("chat_b", make_msg("user", "b msg"), make_msg("assistant", "b reply")).await;

        let a = store.get_history("chat_a").await;
        let b = store.get_history("chat_b").await;
        assert_eq!(a[0].content, "a msg");
        assert_eq!(b[0].content, "b msg");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ===== Additional tests =====

    #[tokio::test]
    async fn test_get_history_nonexistent_chat() {
        let dir = std::env::temp_dir().join("clawtex_test_conv_6");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("test.db");
        let store = ConversationStore::new(db.to_str().unwrap()).await.unwrap();

        let history = store.get_history("does_not_exist").await;
        assert!(history.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_message_count() {
        let dir = std::env::temp_dir().join("clawtex_test_conv_7");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("test.db");
        let store = ConversationStore::new(db.to_str().unwrap()).await.unwrap();

        assert_eq!(store.message_count("chat1").await, 0);

        store.append("chat1", make_msg("user", "hi"), make_msg("assistant", "hello")).await;
        assert_eq!(store.message_count("chat1").await, 2);

        store.append("chat1", make_msg("user", "how are you"), make_msg("assistant", "fine")).await;
        assert_eq!(store.message_count("chat1").await, 4);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_active_count() {
        let dir = std::env::temp_dir().join("clawtex_test_conv_8");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("test.db");
        let store = ConversationStore::new(db.to_str().unwrap()).await.unwrap();

        assert_eq!(store.active_count().await, 0);

        store.append("chat_a", make_msg("user", "a"), make_msg("assistant", "a")).await;
        store.append("chat_b", make_msg("user", "b"), make_msg("assistant", "b")).await;
        store.append("chat_c", make_msg("user", "c"), make_msg("assistant", "c")).await;

        assert_eq!(store.active_count().await, 3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_clear_does_not_affect_other_chats() {
        let dir = std::env::temp_dir().join("clawtex_test_conv_9");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("test.db");
        let store = ConversationStore::new(db.to_str().unwrap()).await.unwrap();

        store.append("chat_a", make_msg("user", "a"), make_msg("assistant", "a")).await;
        store.append("chat_b", make_msg("user", "b"), make_msg("assistant", "b")).await;

        store.clear("chat_a").await;

        let a = store.get_history("chat_a").await;
        let b = store.get_history("chat_b").await;
        assert_eq!(a.len(), 0);
        assert_eq!(b.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_cleanup_stale_removes_old() {
        let dir = std::env::temp_dir().join("clawtex_test_conv_10");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("test.db");
        let store = ConversationStore::new(db.to_str().unwrap()).await.unwrap();

        store.append("chat1", make_msg("user", "old"), make_msg("assistant", "old")).await;

        // Fresh session should not be cleaned up
        let removed = store.cleanup_stale().await;
        assert_eq!(removed, 0);
        assert_eq!(store.active_count().await, 1);
    }

    #[tokio::test]
    async fn test_append_preserves_order() {
        let dir = std::env::temp_dir().join("clawtex_test_conv_11");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("test.db");
        let store = ConversationStore::new(db.to_str().unwrap()).await.unwrap();

        store.append("chat1", make_msg("user", "first"), make_msg("assistant", "first reply")).await;
        store.append("chat1", make_msg("user", "second"), make_msg("assistant", "second reply")).await;

        let history = store.get_history("chat1").await;
        assert_eq!(history.len(), 4);
        assert_eq!(history[0].content, "first");
        assert_eq!(history[1].content, "first reply");
        assert_eq!(history[2].content, "second");
        assert_eq!(history[3].content, "second reply");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_clear_then_append() {
        let dir = std::env::temp_dir().join("clawtex_test_conv_12");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("test.db");
        let store = ConversationStore::new(db.to_str().unwrap()).await.unwrap();

        store.append("chat1", make_msg("user", "before"), make_msg("assistant", "before")).await;
        store.clear("chat1").await;
        store.append("chat1", make_msg("user", "after"), make_msg("assistant", "after")).await;

        let history = store.get_history("chat1").await;
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content, "after");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
