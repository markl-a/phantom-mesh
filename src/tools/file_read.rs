// FileReadTool — read files within workspace
// Security: workspace_only enforcement

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};
use tracing::info;

use super::{SecurityConfig, Tool, ToolResult};

/// Snapshot of a file's state at the time it was read
#[derive(Debug, Clone)]
pub struct FileSnapshot {
    pub mtime: SystemTime,
    pub size: u64,
    pub read_at: Instant,
}

/// Shared snapshot map for TOCTOU protection between file_read and file_edit
pub type FileSnapshots = Arc<Mutex<HashMap<PathBuf, FileSnapshot>>>;

pub struct FileReadTool {
    security: SecurityConfig,
    snapshots: Option<FileSnapshots>,
}

impl FileReadTool {
    pub fn new(security: SecurityConfig) -> Self {
        Self { security, snapshots: None }
    }

    pub fn new_with_snapshots(security: SecurityConfig, snapshots: FileSnapshots) -> Self {
        Self { security, snapshots: Some(snapshots) }
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
                    "description": "File path. Relative paths resolve from workspace. Absolute paths (C:/...) or ~/ paths also work. Examples: 'output/result.txt', '~/.clawtex/workspace/report.md'"
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

                // Record file snapshot for TOCTOU protection
                if let Some(ref snapshots) = self.snapshots {
                    if let Ok(meta) = std::fs::metadata(&canonical) {
                        if let Ok(mtime) = meta.modified() {
                            let snap = FileSnapshot {
                                mtime,
                                size: meta.len(),
                                read_at: Instant::now(),
                            };
                            snapshots.lock().unwrap().insert(canonical.clone(), snap);
                        }
                    }
                }

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

    #[test]
    fn test_file_snapshot_records_on_read() {
        use std::sync::{Arc, Mutex};
        use std::collections::HashMap;

        let dir = std::env::temp_dir().join("clawtex_test_fr_snap");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);

        let snapshots: FileSnapshots = Arc::new(Mutex::new(HashMap::new()));
        let security = SecurityConfig {
            workspace_dir: dir.to_string_lossy().to_string(),
            workspace_only: true,
            allowed_commands: vec![],
            ..Default::default()
        };
        let tool = FileReadTool::new_with_snapshots(security, snapshots.clone());

        std::fs::write(dir.join("snap_test.txt"), "content").unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(tool.execute(json!({"path": "snap_test.txt"}))).unwrap();
        assert!(result.success);

        let snaps = snapshots.lock().unwrap();
        assert_eq!(snaps.len(), 1);
        // Verify the snapshot has correct size
        let snap = snaps.values().next().unwrap();
        assert_eq!(snap.size, 7); // "content" = 7 bytes

        let _ = std::fs::remove_dir_all(&dir);
    }
}
