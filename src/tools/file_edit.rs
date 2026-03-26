use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolResult, SecurityConfig};
use super::file_read::FileSnapshots;

/// File edit tool — precise string replacement within a file
pub struct FileEditTool {
    security: SecurityConfig,
    snapshots: Option<FileSnapshots>,
}

impl FileEditTool {
    pub fn new(security: SecurityConfig) -> Self {
        Self { security, snapshots: None }
    }

    /// Create with shared FileSnapshots for TOCTOU validation (used by Task 7)
    pub fn new_with_snapshots(security: SecurityConfig, snapshots: FileSnapshots) -> Self {
        Self { security, snapshots: Some(snapshots) }
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

        // TOCTOU protection: check if file was modified externally since last read
        if let Some(ref snapshots) = self.snapshots {
            let snaps = snapshots.lock().unwrap();
            if let Some(snapshot) = snaps.get(&file_path) {
                if let Ok(current_meta) = std::fs::metadata(&file_path) {
                    if let Ok(current_mtime) = current_meta.modified() {
                        let current_size = current_meta.len();
                        let mtime_differs = current_mtime != snapshot.mtime;
                        let size_differs = current_size != snapshot.size;

                        // Primary: mtime check
                        // Fallback: if read was recent (<3s), also check size (exFAT 2s mtime granularity)
                        let recently_read = snapshot.read_at.elapsed().as_secs() < 3;
                        let conflict = mtime_differs || (recently_read && size_differs);

                        if conflict {
                            return Ok(ToolResult {
                                success: false,
                                output: "File was modified externally since last read. Please re-read the file before editing.".to_string(),
                            });
                        }
                    }
                }
            }
            drop(snaps); // Release lock before write
        }

        let count = content.matches(old_text).count();
        if count == 0 {
            return Ok(ToolResult {
                success: false,
                output: format!("old_text not found in file '{}'", path),
            });
        }

        let new_content = content.replacen(old_text, new_text, 1);

        match std::fs::write(&file_path, &new_content) {
            Ok(_) => {
                // Update snapshot with new mtime/size after successful edit
                if let Some(ref snapshots) = self.snapshots {
                    if let Ok(new_meta) = std::fs::metadata(&file_path) {
                        if let Ok(new_mtime) = new_meta.modified() {
                            use crate::tools::file_read::FileSnapshot;
                            snapshots.lock().unwrap().insert(file_path.clone(), FileSnapshot {
                                mtime: new_mtime,
                                size: new_meta.len(),
                                read_at: std::time::Instant::now(),
                            });
                        }
                    }
                }

                Ok(ToolResult {
                    success: true,
                    output: format!(
                        "Replaced 1 occurrence in '{}' ({} total matches, {} bytes → {} bytes)",
                        path, count, content.len(), new_content.len()
                    ),
                })
            }
            Err(e) => Ok(ToolResult { success: false, output: format!("Write error: {}", e) }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::file_read::{FileSnapshot, FileSnapshots};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    fn make_tool_with_snapshots(suffix: &str) -> (FileEditTool, std::path::PathBuf, FileSnapshots) {
        let dir = std::env::temp_dir().join(format!("clawtex_test_fe_{}", suffix));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let snapshots: FileSnapshots = Arc::new(Mutex::new(HashMap::new()));
        let security = SecurityConfig {
            workspace_dir: dir.to_string_lossy().to_string(),
            workspace_only: true,
            allowed_commands: vec![],
            ..Default::default()
        };
        (FileEditTool::new_with_snapshots(security, snapshots.clone()), dir, snapshots)
    }

    #[tokio::test]
    async fn test_edit_after_read_succeeds() {
        let (tool, dir, snapshots) = make_tool_with_snapshots("edit_ok");
        let file = dir.join("test.txt");
        std::fs::write(&file, "hello world").unwrap();

        // Simulate a read by recording a snapshot
        let meta = std::fs::metadata(&file).unwrap();
        let canonical = file.canonicalize().unwrap();
        snapshots.lock().unwrap().insert(canonical, FileSnapshot {
            mtime: meta.modified().unwrap(),
            size: meta.len(),
            read_at: Instant::now(),
        });

        let result = tool.execute(json!({
            "path": "test.txt",
            "old_text": "hello",
            "new_text": "goodbye"
        })).await.unwrap();
        assert!(result.success, "Edit should succeed after read: {}", result.output);
        assert!(std::fs::read_to_string(&file).unwrap().contains("goodbye"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_edit_after_external_modify_fails() {
        let (tool, dir, snapshots) = make_tool_with_snapshots("edit_conflict");
        let file = dir.join("conflict.txt");
        std::fs::write(&file, "original content").unwrap();

        // Record snapshot with current state
        let canonical = file.canonicalize().unwrap();
        let meta = std::fs::metadata(&file).unwrap();
        snapshots.lock().unwrap().insert(canonical, FileSnapshot {
            mtime: meta.modified().unwrap(),
            size: meta.len(),
            read_at: Instant::now(),
        });

        // External modification (changes size, which triggers the exFAT fallback detection)
        std::fs::write(&file, "externally modified content that is much longer now").unwrap();

        let result = tool.execute(json!({
            "path": "conflict.txt",
            "old_text": "original",
            "new_text": "replaced"
        })).await.unwrap();
        assert!(!result.success, "Edit should fail after external modify");
        assert!(result.output.contains("modified externally") || result.output.contains("modified since"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_edit_without_prior_read_succeeds() {
        let (tool, dir, _snapshots) = make_tool_with_snapshots("no_read");
        let file = dir.join("noread.txt");
        std::fs::write(&file, "hello world").unwrap();

        // No snapshot recorded — should allow edit (backwards compatible)
        let result = tool.execute(json!({
            "path": "noread.txt",
            "old_text": "hello",
            "new_text": "goodbye"
        })).await.unwrap();
        assert!(result.success, "Edit without prior read should succeed: {}", result.output);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_edit_updates_snapshot_after_write() {
        let (tool, dir, snapshots) = make_tool_with_snapshots("update_snap");
        let file = dir.join("update.txt");
        std::fs::write(&file, "hello world").unwrap();

        // Record initial snapshot
        let canonical = file.canonicalize().unwrap();
        let meta = std::fs::metadata(&file).unwrap();
        let original_size = meta.len();
        snapshots.lock().unwrap().insert(canonical.clone(), FileSnapshot {
            mtime: meta.modified().unwrap(),
            size: original_size,
            read_at: Instant::now(),
        });

        let result = tool.execute(json!({
            "path": "update.txt",
            "old_text": "hello",
            "new_text": "goodbye"
        })).await.unwrap();
        assert!(result.success);

        // Snapshot should be updated with new size
        let snaps = snapshots.lock().unwrap();
        let new_snap = snaps.get(&canonical).unwrap();
        assert_ne!(new_snap.size, original_size, "Snapshot size should be updated after edit");

        let _ = std::fs::remove_dir_all(&dir);
    }

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
