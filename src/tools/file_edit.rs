use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolResult, SecurityConfig};

/// File edit tool — precise string replacement within a file
pub struct FileEditTool {
    security: SecurityConfig,
}

impl FileEditTool {
    pub fn new(security: SecurityConfig) -> Self {
        Self { security }
    }

    fn validate_path(&self, path: &str) -> Result<std::path::PathBuf> {
        let workspace = self.security.workspace_path();
        let full_path = if path.starts_with('/') || path.contains(":\\") {
            std::path::PathBuf::from(path)
        } else {
            workspace.join(path)
        };

        // Resolve symlinks and normalize
        let canonical = full_path.canonicalize().unwrap_or(full_path.clone());

        if self.security.workspace_only && !self.security.is_allowed_path(&canonical) {
            anyhow::bail!("Path '{}' is outside workspace and allowed paths", path);
        }

        // Block path traversal
        if path.contains("..") {
            anyhow::bail!("Path traversal not allowed");
        }

        Ok(canonical)
    }
}

#[async_trait]
impl Tool for FileEditTool {
    fn name(&self) -> &str {
        "file_edit"
    }

    fn description(&self) -> &str {
        "Replace exact text in a file. Finds old_text and replaces with new_text."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit (relative to workspace)"
                },
                "old_text": {
                    "type": "string",
                    "description": "Exact text to find and replace"
                },
                "new_text": {
                    "type": "string",
                    "description": "Text to replace old_text with"
                }
            },
            "required": ["path", "old_text", "new_text"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let old_text = args.get("old_text").and_then(|v| v.as_str()).unwrap_or("");
        let new_text = args.get("new_text").and_then(|v| v.as_str()).unwrap_or("");

        if path.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing 'path' argument".into() });
        }
        if old_text.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing 'old_text' argument".into() });
        }

        let file_path = match self.validate_path(path) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult { success: false, output: format!("Invalid path: {}", e) }),
        };

        if !file_path.exists() {
            return Ok(ToolResult { success: false, output: format!("File not found: {}", path) });
        }

        let content = match std::fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult { success: false, output: format!("Read error: {}", e) }),
        };

        let count = content.matches(old_text).count();
        if count == 0 {
            return Ok(ToolResult {
                success: false,
                output: format!("old_text not found in file '{}'", path),
            });
        }

        let new_content = content.replacen(old_text, new_text, 1);

        match std::fs::write(&file_path, &new_content) {
            Ok(_) => Ok(ToolResult {
                success: true,
                output: format!(
                    "Replaced 1 occurrence in '{}' ({} total matches, {} bytes → {} bytes)",
                    path, count, content.len(), new_content.len()
                ),
            }),
            Err(e) => Ok(ToolResult { success: false, output: format!("Write error: {}", e) }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_file_edit_missing_path() {
        let tool = FileEditTool::new(SecurityConfig::default());
        let result = tool.execute(json!({"old_text": "a", "new_text": "b"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing 'path'"));
    }

    #[tokio::test]
    async fn test_file_edit_missing_old_text() {
        let tool = FileEditTool::new(SecurityConfig::default());
        let result = tool.execute(json!({"path": "test.txt", "new_text": "b"})).await.unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_file_edit_nonexistent() {
        let tool = FileEditTool::new(SecurityConfig { workspace_only: false, ..Default::default() });
        let result = tool.execute(json!({
            "path": "/tmp/clawtex_nonexistent_test_file.txt",
            "old_text": "a",
            "new_text": "b"
        })).await.unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_file_edit_rejects_traversal() {
        let tool = FileEditTool::new(SecurityConfig::default());
        let result = tool.execute(json!({
            "path": "../../etc/passwd",
            "old_text": "a",
            "new_text": "b"
        })).await.unwrap();
        assert!(!result.success);
    }
}
