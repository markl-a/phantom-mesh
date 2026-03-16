use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::memory::{MemoryStore, MemoryCategory};
use super::{Tool, ToolResult};

/// Memory store tool — allows agent to save information to semantic memory
pub struct MemoryStoreTool {
    memory: Arc<MemoryStore>,
}

impl MemoryStoreTool {
    pub fn new(memory: Arc<MemoryStore>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for MemoryStoreTool {
    fn name(&self) -> &str { "memory_store" }
    fn description(&self) -> &str {
        "Store information in long-term semantic memory. Useful for remembering facts, user preferences, or important context."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Short key/label for the memory" },
                "content": { "type": "string", "description": "The information to store" },
                "category": {
                    "type": "string",
                    "enum": ["core", "conversation", "task_result"],
                    "description": "Category of the memory (default: core). Use 'core' for facts/preferences, 'conversation' for dialog context, 'task_result' for task outputs."
                }
            },
            "required": ["key", "content"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let key = args.get("key").and_then(|v| v.as_str()).unwrap_or("");
        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
        if key.is_empty() || content.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing 'key' or 'content'".into() });
        }
        let category = match args.get("category").and_then(|v| v.as_str()).unwrap_or("core") {
            "core" => MemoryCategory::Core,
            "conversation" => MemoryCategory::Conversation,
            "task_result" => MemoryCategory::TaskResult,
            other => MemoryCategory::Custom(other.to_string()),
        };

        match self.memory.store(key, content, category, None).await {
            Ok(id) => Ok(ToolResult {
                success: true,
                output: format!("Stored memory '{}' (id={})", key, id),
            }),
            Err(e) => Ok(ToolResult { success: false, output: format!("Store failed: {}", e) }),
        }
    }
}

/// Memory recall tool — allows agent to search semantic memory
pub struct MemoryRecallTool {
    memory: Arc<MemoryStore>,
}

impl MemoryRecallTool {
    pub fn new(memory: Arc<MemoryStore>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for MemoryRecallTool {
    fn name(&self) -> &str { "memory_recall" }
    fn description(&self) -> &str {
        "Search long-term semantic memory for relevant information. Returns matching memories."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "limit": { "type": "integer", "description": "Max results (default: 5)" }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        if query.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing 'query'".into() });
        }
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

        match self.memory.recall(query, limit, None).await {
            Ok(results) => {
                if results.is_empty() {
                    return Ok(ToolResult { success: true, output: "No matching memories found.".into() });
                }
                let formatted: Vec<String> = results.iter().map(|entry| {
                    format!("[{}] ({}) {}: {}", entry.id, entry.category, entry.key, entry.content)
                }).collect();
                Ok(ToolResult {
                    success: true,
                    output: format!("Found {} memories:\n{}", results.len(), formatted.join("\n")),
                })
            }
            Err(e) => Ok(ToolResult { success: false, output: format!("Search failed: {}", e) }),
        }
    }
}

/// Memory forget tool — allows agent to delete specific memories
pub struct MemoryForgetTool {
    memory: Arc<MemoryStore>,
}

impl MemoryForgetTool {
    pub fn new(memory: Arc<MemoryStore>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for MemoryForgetTool {
    fn name(&self) -> &str { "memory_forget" }
    fn description(&self) -> &str {
        "Delete a memory by its key. Use memory_recall first to find the key."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Memory key to delete" }
            },
            "required": ["key"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let key = args.get("key").and_then(|v| v.as_str()).unwrap_or("");
        if key.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing 'key'".into() });
        }

        match self.memory.forget(key).await {
            Ok(deleted) => {
                if deleted {
                    Ok(ToolResult { success: true, output: format!("Deleted memory '{}'", key) })
                } else {
                    Ok(ToolResult { success: false, output: format!("Memory '{}' not found", key) })
                }
            }
            Err(e) => Ok(ToolResult { success: false, output: format!("Delete failed: {}", e) }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryConfig;

    fn make_memory() -> (Arc<MemoryStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_mem.db");
        let mem = Arc::new(MemoryStore::sqlite(db_path.to_str().unwrap(), MemoryConfig::default()).unwrap());
        (mem, dir) // keep dir alive so the temp file persists
    }

    #[tokio::test]
    async fn test_memory_store_missing_content() {
        let (mem, _dir) = make_memory();
        let tool = MemoryStoreTool::new(mem);
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_memory_recall_missing_query() {
        let (mem, _dir) = make_memory();
        let tool = MemoryRecallTool::new(mem);
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_memory_forget_missing_key() {
        let (mem, _dir) = make_memory();
        let tool = MemoryForgetTool::new(mem);
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_memory_store_and_recall() {
        let (mem, _dir) = make_memory();
        let store_tool = MemoryStoreTool::new(mem.clone());
        let recall_tool = MemoryRecallTool::new(mem.clone());

        // Store
        let result = store_tool.execute(json!({
            "key": "dark_mode_pref",
            "content": "The user prefers dark mode",
            "category": "core"
        })).await.unwrap();
        assert!(result.success, "Store failed: {}", result.output);

        // Recall
        let result = recall_tool.execute(json!({"query": "dark mode"})).await.unwrap();
        assert!(result.success, "Recall failed: {}", result.output);
        assert!(result.output.contains("dark mode"), "Recall output: {}", result.output);
    }
}
