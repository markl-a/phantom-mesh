//! Lightweight cross-node sync index.
//!
//! The [`SyncIndex`] is an in-memory structure maintained by the Coordinator.
//! It aggregates conversation and task summaries received via heartbeat data
//! from all cluster nodes.  It is **not** persisted — it is rebuilt from
//! heartbeats on each coordinator startup.

use serde::{Deserialize, Serialize};
use std::sync::RwLock;

/// Summary of a conversation for cross-node discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub chat_id: String,
    pub node_id: String,
    pub last_message_at: u64,
    pub message_count: usize,
    /// First user message used as a short title.
    pub title: Option<String>,
}

/// Summary of a task for cross-node discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSummary {
    pub task_id: String,
    pub node_id: String,
    pub title: String,
    pub status: String,
    pub updated_at: u64,
}

/// In-memory index aggregated from node heartbeats.
///
/// Thread-safe via [`RwLock`]; reads are non-blocking when no write is
/// in progress.
#[derive(Debug, Default)]
pub struct SyncIndex {
    pub conversations: RwLock<Vec<ConversationSummary>>,
    pub tasks: RwLock<Vec<TaskSummary>>,
}

impl SyncIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace all entries for `node_id` with fresh data from a heartbeat.
    pub fn update_from_node(
        &self,
        node_id: &str,
        conversations: Vec<ConversationSummary>,
        tasks: Vec<TaskSummary>,
    ) {
        // -- conversations --
        {
            let mut convs = self.conversations.write().unwrap();
            convs.retain(|c| c.node_id != node_id);
            convs.extend(conversations);
        }
        // -- tasks --
        {
            let mut ts = self.tasks.write().unwrap();
            ts.retain(|t| t.node_id != node_id);
            ts.extend(tasks);
        }
    }

    /// Find which node owns a conversation by `chat_id`.
    pub fn find_conversation(&self, chat_id: &str) -> Option<ConversationSummary> {
        let convs = self.conversations.read().unwrap();
        convs.iter().find(|c| c.chat_id == chat_id).cloned()
    }

    /// List every conversation across all nodes.
    pub fn all_conversations(&self) -> Vec<ConversationSummary> {
        self.conversations.read().unwrap().clone()
    }

    /// List every task across all nodes.
    pub fn all_tasks(&self) -> Vec<TaskSummary> {
        self.tasks.read().unwrap().clone()
    }
}

// ── Unit tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(chat_id: &str, node_id: &str, ts: u64) -> ConversationSummary {
        ConversationSummary {
            chat_id: chat_id.to_string(),
            node_id: node_id.to_string(),
            last_message_at: ts,
            message_count: 1,
            title: Some(format!("conv-{}", chat_id)),
        }
    }

    fn task(task_id: &str, node_id: &str, ts: u64) -> TaskSummary {
        TaskSummary {
            task_id: task_id.to_string(),
            node_id: node_id.to_string(),
            title: format!("task-{}", task_id),
            status: "pending".to_string(),
            updated_at: ts,
        }
    }

    #[test]
    fn update_and_find() {
        let idx = SyncIndex::new();
        idx.update_from_node("node-a", vec![conv("c1", "node-a", 100)], vec![]);
        idx.update_from_node("node-b", vec![conv("c2", "node-b", 200)], vec![]);

        let found = idx.find_conversation("c1").unwrap();
        assert_eq!(found.node_id, "node-a");

        let found = idx.find_conversation("c2").unwrap();
        assert_eq!(found.node_id, "node-b");

        assert!(idx.find_conversation("c999").is_none());
    }

    #[test]
    fn all_conversations_from_multiple_nodes() {
        let idx = SyncIndex::new();
        idx.update_from_node(
            "node-a",
            vec![conv("c1", "node-a", 10), conv("c2", "node-a", 20)],
            vec![],
        );
        idx.update_from_node("node-b", vec![conv("c3", "node-b", 30)], vec![]);
        idx.update_from_node("node-c", vec![conv("c4", "node-c", 40)], vec![]);

        let all = idx.all_conversations();
        assert_eq!(all.len(), 4);
    }

    #[test]
    fn update_replaces_old_data() {
        let idx = SyncIndex::new();
        idx.update_from_node("node-a", vec![conv("c1", "node-a", 10)], vec![]);
        assert_eq!(idx.all_conversations().len(), 1);

        // Second update from same node replaces
        idx.update_from_node(
            "node-a",
            vec![conv("c1", "node-a", 20), conv("c5", "node-a", 30)],
            vec![],
        );
        assert_eq!(idx.all_conversations().len(), 2);
    }

    #[test]
    fn all_tasks_from_multiple_nodes() {
        let idx = SyncIndex::new();
        idx.update_from_node("node-a", vec![], vec![task("t1", "node-a", 100)]);
        idx.update_from_node(
            "node-b",
            vec![],
            vec![task("t2", "node-b", 200), task("t3", "node-b", 300)],
        );

        let tasks = idx.all_tasks();
        assert_eq!(tasks.len(), 3);
    }
}
