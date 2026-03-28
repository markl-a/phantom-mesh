//! Cross-node query helpers for conversations, tasks, and memory.
//!
//! These functions format RPC requests to query data from other nodes.

use serde_json::{json, Value};

/// Build an RPC request to list conversations on a remote node.
pub fn list_remote_conversations_request() -> (String, Value) {
    ("rpc.conversations.list".to_string(), json!({}))
}

/// Build an RPC request to get a specific conversation from a remote node.
pub fn get_remote_conversation_request(chat_id: &str) -> (String, Value) {
    ("rpc.conversations.get".to_string(), json!({ "chat_id": chat_id }))
}

/// Build an RPC request to search memory on a remote node.
pub fn search_remote_memory_request(query: &str, limit: usize) -> (String, Value) {
    ("rpc.memory.search".to_string(), json!({ "query": query, "limit": limit }))
}

/// Build an RPC request to list tasks on a remote node.
pub fn list_remote_tasks_request(status_filter: Option<&str>) -> (String, Value) {
    let mut params = json!({});
    if let Some(status) = status_filter {
        params["status"] = json!(status);
    }
    ("rpc.tasks.list".to_string(), params)
}

/// Build an RPC request to dispatch a task to a remote node.
pub fn dispatch_remote_task_request(title: &str, prompt: &str, priority: &str) -> (String, Value) {
    ("rpc.tasks.dispatch".to_string(), json!({
        "title": title,
        "prompt": prompt,
        "priority": priority,
    }))
}
