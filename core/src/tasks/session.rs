//! JSONL session persistence (claw-code pattern) + Session Repair (OpenFang pattern).
//!
//! Each long-running task owns a single append-only JSONL file at
//! `<root>/sessions/<workspace_id>/<task_id>.jsonl`. Lines are `SessionEntry`
//! values (User / Assistant / ToolCall / ToolResult / System).
//!
//! On resume the reader replays the file and runs Session Repair: any
//! `ToolCall` whose `call_id` lacks a matching `ToolResult` gets a synthetic
//! result injected so the LLM isn't fed a half-completed turn.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use pm_types::SessionEntry;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use uuid::Uuid;

const SYNTHETIC_RESULT_NOTE: &str =
    "(execution interrupted — synthetic result injected by Session Repair)";

/// Resolves the on-disk path for a workspace + task pair, rooted at
/// `~/.spectyn-mesh/sessions/` (or `SPECTYN_SESSIONS_DIR` when set, for tests).
pub fn session_path(workspace_id: &str, task_id: Uuid) -> PathBuf {
    sessions_root()
        .join(workspace_id)
        .join(format!("{}.jsonl", task_id))
}

/// Same as `session_path` but rooted under a caller-supplied directory. Used
/// by tests to keep them hermetic without touching process env vars.
pub fn session_path_at(root: &Path, workspace_id: &str, task_id: Uuid) -> PathBuf {
    root.join(workspace_id).join(format!("{}.jsonl", task_id))
}

fn sessions_root() -> PathBuf {
    if let Ok(p) = std::env::var("SPECTYN_SESSIONS_DIR") {
        return PathBuf::from(p);
    }
    crate::cli_config::spectyn_data_dir()
        .unwrap_or_else(|_| PathBuf::from(".").join(".spectyn-mesh"))
        .join("sessions")
}

/// Append-only JSONL session writer. Cheap to clone (`Arc` inside).
#[derive(Clone)]
pub struct SessionWriter {
    workspace_id: String,
    task_id: Uuid,
    path: PathBuf,
    file: Arc<Mutex<tokio::fs::File>>,
}

impl SessionWriter {
    pub async fn open(workspace_id: &str, task_id: Uuid) -> Result<Self> {
        Self::open_at(&sessions_root(), workspace_id, task_id).await
    }

    pub async fn open_at(root: &Path, workspace_id: &str, task_id: Uuid) -> Result<Self> {
        let path = session_path_at(root, workspace_id, task_id);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .with_context(|| format!("open {}", path.display()))?;
        Ok(Self {
            workspace_id: workspace_id.to_string(),
            task_id,
            path,
            file: Arc::new(Mutex::new(file)),
        })
    }

    pub async fn append(&self, entry: SessionEntry) -> Result<()> {
        let line = serde_json::to_string(&entry).with_context(|| "serialize SessionEntry")?;
        let mut file = self.file.lock().await;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        // fsync via flush-equivalent — we don't fdatasync every line for cost.
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    pub fn task_id(&self) -> Uuid {
        self.task_id
    }
}

/// Outcome of loading + repairing a session log.
#[derive(Debug, Clone)]
pub struct RepairedSession {
    /// Entries after repair (any synthetic ToolResult injected at the right slot).
    pub entries: Vec<SessionEntry>,
    /// Number of orphan ToolCalls that received a synthetic ToolResult.
    pub repaired_count: usize,
}

/// Read the entire session JSONL into memory and run Session Repair on it.
/// Missing files yield an empty session, not an error — that's the normal path
/// for a brand-new task.
pub async fn load_and_repair(workspace_id: &str, task_id: Uuid) -> Result<RepairedSession> {
    load_and_repair_at(&sessions_root(), workspace_id, task_id).await
}

pub async fn load_and_repair_at(
    root: &Path,
    workspace_id: &str,
    task_id: Uuid,
) -> Result<RepairedSession> {
    let path = session_path_at(root, workspace_id, task_id);
    let raw = match tokio::fs::read_to_string(&path).await {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RepairedSession {
                entries: vec![],
                repaired_count: 0,
            });
        }
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };

    let mut entries: Vec<SessionEntry> = Vec::new();
    for (lineno, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<SessionEntry>(line) {
            Ok(e) => entries.push(e),
            Err(e) => {
                tracing::warn!(
                    "session {} line {}: skipping unparsable entry: {}",
                    path.display(),
                    lineno + 1,
                    e
                );
            }
        }
    }

    Ok(repair(entries))
}

/// Pure repair function — exposed for unit tests.
pub fn repair(entries: Vec<SessionEntry>) -> RepairedSession {
    // Walk the entries in order, tracking open ToolCall ids. When we hit the
    // end (or a User/Assistant breakpoint that effectively starts a new turn
    // — rare but possible with crash mid-tool), inject synthetic ToolResults
    // for any unresolved ids.
    let mut out: Vec<SessionEntry> = Vec::with_capacity(entries.len());
    let mut open_calls: Vec<(String, i64)> = Vec::new(); // (call_id, ts_of_call)
    let mut repaired_count = 0;

    for e in entries.into_iter() {
        match &e {
            SessionEntry::ToolCall {
                call_id, timestamp, ..
            } => {
                open_calls.push((call_id.clone(), *timestamp));
            }
            SessionEntry::ToolResult { call_id, .. } => {
                open_calls.retain(|(id, _)| id != call_id);
            }
            _ => {}
        }
        out.push(e);
    }

    // For any remaining open ToolCalls, synthesise a result so the LLM isn't
    // fed a half-completed turn.
    if !open_calls.is_empty() {
        let now = now_millis();
        for (call_id, _) in open_calls.drain(..) {
            out.push(SessionEntry::ToolResult {
                call_id,
                output: SYNTHETIC_RESULT_NOTE.to_string(),
                synthetic: true,
                timestamp: now,
            });
            repaired_count += 1;
        }
    }

    RepairedSession {
        entries: out,
        repaired_count,
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn repair_no_op_when_balanced() {
        let entries = vec![
            SessionEntry::User {
                content: "hi".into(),
                timestamp: 1,
            },
            SessionEntry::ToolCall {
                call_id: "c1".into(),
                name: "shell".into(),
                args: json!({}),
                timestamp: 2,
            },
            SessionEntry::ToolResult {
                call_id: "c1".into(),
                output: "ok".into(),
                synthetic: false,
                timestamp: 3,
            },
            SessionEntry::Assistant {
                content: "done".into(),
                timestamp: 4,
            },
        ];
        let out = repair(entries.clone());
        assert_eq!(out.entries.len(), 4);
        assert_eq!(out.repaired_count, 0);
    }

    #[test]
    fn repair_injects_synthetic_for_orphan_tool_call() {
        let entries = vec![
            SessionEntry::User {
                content: "do thing".into(),
                timestamp: 1,
            },
            SessionEntry::ToolCall {
                call_id: "c1".into(),
                name: "shell".into(),
                args: json!({"command": "wc -l"}),
                timestamp: 2,
            },
            // Crash here — no ToolResult.
        ];
        let out = repair(entries);
        assert_eq!(out.entries.len(), 3);
        assert_eq!(out.repaired_count, 1);
        let last = out.entries.last().unwrap();
        match last {
            SessionEntry::ToolResult {
                call_id,
                synthetic,
                output,
                ..
            } => {
                assert_eq!(call_id, "c1");
                assert!(*synthetic);
                assert!(output.contains("interrupted"));
            }
            _ => panic!("expected synthetic ToolResult"),
        }
    }

    #[test]
    fn repair_handles_multiple_orphans() {
        let entries = vec![
            SessionEntry::ToolCall {
                call_id: "a".into(),
                name: "x".into(),
                args: json!({}),
                timestamp: 1,
            },
            SessionEntry::ToolCall {
                call_id: "b".into(),
                name: "y".into(),
                args: json!({}),
                timestamp: 2,
            },
            // Both orphan
        ];
        let out = repair(entries);
        assert_eq!(out.repaired_count, 2);
        assert_eq!(out.entries.len(), 4);
    }

    #[test]
    fn repair_doesnt_double_close() {
        // ToolResult arrives but for a different call_id — the original orphan
        // should still be closed by repair.
        let entries = vec![
            SessionEntry::ToolCall {
                call_id: "a".into(),
                name: "x".into(),
                args: json!({}),
                timestamp: 1,
            },
            SessionEntry::ToolResult {
                call_id: "b".into(),
                output: "stray".into(),
                synthetic: false,
                timestamp: 2,
            },
        ];
        let out = repair(entries);
        // a still orphan, b is unmatched closer — keep both, plus synthetic for a.
        assert_eq!(out.repaired_count, 1);
        assert!(out.entries.iter().any(|e| matches!(e, SessionEntry::ToolResult { call_id, synthetic: true, .. } if call_id == "a")));
    }

    #[tokio::test]
    async fn writer_appends_jsonl_lines() {
        let dir = tempfile::tempdir().unwrap();
        let task_id = Uuid::new_v4();
        let writer = SessionWriter::open_at(dir.path(), "ws1", task_id)
            .await
            .unwrap();

        writer
            .append(SessionEntry::User {
                content: "a".into(),
                timestamp: 1,
            })
            .await
            .unwrap();
        writer
            .append(SessionEntry::Assistant {
                content: "b".into(),
                timestamp: 2,
            })
            .await
            .unwrap();
        drop(writer);

        let path = session_path_at(dir.path(), "ws1", task_id);
        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(contents.lines().count(), 2);

        let loaded = load_and_repair_at(dir.path(), "ws1", task_id)
            .await
            .unwrap();
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.repaired_count, 0);
    }

    #[tokio::test]
    async fn load_missing_session_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_and_repair_at(dir.path(), "nosuch", Uuid::new_v4())
            .await
            .unwrap();
        assert!(loaded.entries.is_empty());
    }

    #[tokio::test]
    async fn writer_then_repair_handles_orphan() {
        let dir = tempfile::tempdir().unwrap();
        let task_id = Uuid::new_v4();
        let w = SessionWriter::open_at(dir.path(), "ws1", task_id)
            .await
            .unwrap();
        w.append(SessionEntry::User {
            content: "do".into(),
            timestamp: 1,
        })
        .await
        .unwrap();
        w.append(SessionEntry::ToolCall {
            call_id: "c1".into(),
            name: "shell".into(),
            args: json!({"cmd": "x"}),
            timestamp: 2,
        })
        .await
        .unwrap();
        // Crash before ToolResult.
        drop(w);

        let r = load_and_repair_at(dir.path(), "ws1", task_id)
            .await
            .unwrap();
        assert_eq!(r.repaired_count, 1);
        assert!(matches!(
            r.entries.last().unwrap(),
            SessionEntry::ToolResult {
                synthetic: true,
                ..
            }
        ));
    }
}
