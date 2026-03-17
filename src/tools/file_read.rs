// FileReadTool — read files within workspace
// Security: workspace_only enforcement

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::info;

use super::{SecurityConfig, Tool, ToolResult};

pub struct FileReadTool {
    security: SecurityConfig,
}

impl FileReadTool {
    pub fn new(security: SecurityConfig) -> Self {
        Self { security }
    }
}

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str { "file_read" }

    fn preflight(&self, args: &Value) -> Result<()> {
        let raw_path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        if raw_path.is_empty() {
            return Err(anyhow::anyhow!("Preflight: missing 'path' parameter"));
        }
        // Resolve the path and check existence
        let expanded = super::expand_home(raw_path);
        let ws = self.security.workspace_path();
        let normalized = super::normalize_llm_path(&expanded, &ws);
        let full_path = if std::path::Path::new(&normalized).is_absolute() {
            std::path::PathBuf::from(&normalized)
        } else {
            ws.join(&normalized)
        };
        if !full_path.exists() {
            return Err(anyhow::anyhow!("Preflight: file does not exist: {}", full_path.display()));
        }
        Ok(())
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Path can be relative to workspace or absolute. Paths with ~/ are auto-expanded."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path. Relative paths resolve from workspace. Absolute paths (C:/...) or ~/ paths also work. Examples: 'output/result.txt', 'C:/Users/m4932/.clawtex/workspace/report.md'"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let raw_path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if raw_path.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Error: empty path".to_string(),
            });
        }

        // ACI poka-yoke: normalize LLM-provided paths
        let workspace = self.security.workspace_path();
        let path = super::normalize_llm_path(raw_path, &workspace);

        // Resolve full path — support both absolute and relative paths
        let full_path = if path.starts_with('/') || path.contains(":\\") || path.contains(":/") {
            std::path::PathBuf::from(&path)
        } else {
            workspace.join(&path)
        };

        // Security: prevent path traversal
        let canonical = match full_path.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Error: file not found: {}", path),
                });
            }
        };

        if self.security.workspace_only && !self.security.is_allowed_path(&canonical) {
            return Ok(ToolResult {
                success: false,
                output: format!("Error: path '{}' is outside workspace and allowed paths", path),
            });
        }

        info!("Reading file: {}", canonical.display());

        match tokio::fs::read_to_string(&canonical).await {
            Ok(content) => {
                // Truncate very long files (safe for multi-byte UTF-8)
                let truncated = if content.len() > 8000 {
                    let end = {
                        let mut i = 8000;
                        while i > 0 && !content.is_char_boundary(i) { i -= 1; }
                        i
                    };
                    format!(
                        "{}...\n(truncated, {} bytes total)",
                        &content[..end],
                        content.len()
                    )
                } else {
                    content
                };

                Ok(ToolResult {
                    success: true,
                    output: truncated,
                })
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: format!("Error reading file: {}", e),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_tool(suffix: &str) -> (FileReadTool, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("clawtex_test_fr_{}", suffix));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        let security = SecurityConfig {
            workspace_dir: dir.to_string_lossy().to_string(),
            workspace_only: true,
            allowed_commands: vec![],
            ..Default::default()
        };
        (FileReadTool::new(security), dir)
    }

    #[tokio::test]
    async fn test_read_existing_file() {
        let (tool, dir) = make_tool("read");
        fs::write(dir.join("test.txt"), "hello world").unwrap();

        let result = tool.execute(json!({"path": "test.txt"})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("hello world"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_read_nonexistent() {
        let (tool, dir) = make_tool("nope");
        let result = tool.execute(json!({"path": "nope.txt"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("not found"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_read_empty_path() {
        let (tool, _dir) = make_tool("empty");
        let result = tool.execute(json!({"path": ""})).await.unwrap();
        assert!(!result.success);
    }
}
