// FileWriteTool — write files within workspace
// Security: workspace_only enforcement, creates parent dirs

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::info;

use super::{SecurityConfig, Tool, ToolResult};

pub struct FileWriteTool {
    security: SecurityConfig,
}

impl FileWriteTool {
    pub fn new(security: SecurityConfig) -> Self {
        Self { security }
    }
}

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str { "file_write" }

    fn description(&self) -> &str {
        "Write content to a file in the workspace directory. Creates parent directories if needed. Paths with ~/ are auto-expanded."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path. Relative paths resolve from workspace. Absolute paths work if within workspace. Examples: 'output/result.txt', 'evolve_suggestions/2026-03-12.md'"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let raw_path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if raw_path.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Error: empty path".to_string(),
            });
        }

        // Reject obvious path traversal attempts
        if raw_path.contains("..") {
            return Ok(ToolResult {
                success: false,
                output: "Error: path traversal not allowed".to_string(),
            });
        }

        // ACI poka-yoke: normalize LLM-provided paths
        let workspace = self.security.workspace_path();
        let path = super::normalize_llm_path(raw_path, &workspace);

        let full_path = if path.starts_with('/') || path.contains(":\\") || path.contains(":/") {
            std::path::PathBuf::from(&path)
        } else {
            workspace.join(&path)
        };

        // Security: ensure resolved path stays within workspace or allowed paths
        if self.security.workspace_only {
            let ws_str = workspace.to_string_lossy().to_string();
            let full_str = full_path.to_string_lossy().to_string();
            let in_workspace = full_str.starts_with(&ws_str);
            let in_allowed = self.security.allowed_paths.iter().any(|p| full_str.starts_with(p));
            if !in_workspace && !in_allowed {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Error: path '{}' is outside workspace and allowed paths", path),
                });
            }
        }

        // Create parent directories
        if let Some(parent) = full_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Error creating directories: {}", e),
                });
            }
        }

        info!("Writing file: {} ({} bytes)", full_path.display(), content.len());

        match tokio::fs::write(&full_path, content).await {
            Ok(()) => Ok(ToolResult {
                success: true,
                output: format!("Written {} bytes to {}", content.len(), path),
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: format!("Error writing file: {}", e),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_tool(suffix: &str) -> (FileWriteTool, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("phantom_mesh_test_fw_{}", suffix));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        let security = SecurityConfig {
            workspace_dir: dir.to_string_lossy().to_string(),
            workspace_only: true,
            allowed_commands: vec![],
            ..Default::default()
        };
        (FileWriteTool::new(security), dir)
    }

    #[tokio::test]
    async fn test_write_file() {
        let (tool, dir) = make_tool("write");
        let result = tool.execute(json!({
            "path": "out.txt",
            "content": "hello world"
        })).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("11 bytes"));

        let content = fs::read_to_string(dir.join("out.txt")).unwrap();
        assert_eq!(content, "hello world");

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_write_creates_dirs() {
        let (tool, dir) = make_tool("mkdir");
        let result = tool.execute(json!({
            "path": "sub/dir/file.txt",
            "content": "nested"
        })).await.unwrap();
        assert!(result.success);
        assert!(dir.join("sub/dir/file.txt").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_write_rejects_traversal() {
        let (tool, dir) = make_tool("trav");
        let result = tool.execute(json!({
            "path": "../escape.txt",
            "content": "nope"
        })).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("traversal"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_write_empty_path() {
        let (tool, _dir) = make_tool("empty");
        let result = tool.execute(json!({
            "path": "",
            "content": "nope"
        })).await.unwrap();
        assert!(!result.success);
    }
}
