//! Distributed State Management — task state tracking, file transfer registry,
//! and state sync protocol for hub-worker coordination.
//!
//! Provides three core subsystems:
//! 1. **DistributedTaskTracker** — tracks task lifecycle (Pending → Running → Completed/Failed)
//!    with in-memory HashMap + optional SQLite persistence.
//! 2. **FileTransferRegistry** — tracks files that need syncing between cluster nodes.
//! 3. **StateSyncMessage** — serializable messages for state sync over HTTP transport.

use anyhow::{anyhow, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

// ── Task State ──────────────────────────────────────────────────────────────

/// Lifecycle state of a distributed task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskState {
    /// Task is queued but not yet picked up by a worker.
    Pending,
    /// Task is actively being executed by a worker.
    Running {
        worker: String,
        started_at: String,
    },
    /// Task finished successfully.
    Completed {
        worker: String,
        duration_secs: f64,
        result_summary: String,
    },
    /// Task failed with an error.
    Failed {
        worker: String,
        error: String,
    },
}

impl TaskState {
    /// Return a short status string (for filtering).
    pub fn status_name(&self) -> &str {
        match self {
            TaskState::Pending => "pending",
            TaskState::Running { .. } => "running",
            TaskState::Completed { .. } => "completed",
            TaskState::Failed { .. } => "failed",
        }
    }

    /// Extract the worker name, if any.
    pub fn worker(&self) -> Option<&str> {
        match self {
            TaskState::Pending => None,
            TaskState::Running { worker, .. } => Some(worker),
            TaskState::Completed { worker, .. } => Some(worker),
            TaskState::Failed { worker, .. } => Some(worker),
        }
    }
}

// ── DistributedTaskTracker ──────────────────────────────────────────────────

/// Tracks task states across the cluster.
///
/// Uses an in-memory HashMap for fast lookups, with optional SQLite persistence
/// so that state survives daemon restarts.
pub struct DistributedTaskTracker {
    /// In-memory state map (task_id → TaskState).
    states: Mutex<HashMap<String, TaskState>>,
    /// Optional SQLite connection for persistence.
    db: Option<Mutex<Connection>>,
}

impl DistributedTaskTracker {
    /// Create an in-memory-only tracker (no persistence).
    pub fn new_memory() -> Self {
        Self {
            states: Mutex::new(HashMap::new()),
            db: None,
        }
    }

    /// Create a tracker with SQLite persistence at `db_path`.
    pub async fn new_persistent(db_path: &str) -> Result<Self> {
        let path = db_path.to_string();
        let (conn, states) = tokio::task::spawn_blocking(move || -> Result<(Connection, HashMap<String, TaskState>)> {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }

            let conn = Connection::open(&path)?;
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;")?;

            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS task_states (
                    task_id      TEXT PRIMARY KEY,
                    status       TEXT NOT NULL,
                    worker       TEXT,
                    started_at   TEXT,
                    duration_secs REAL,
                    result_summary TEXT,
                    error        TEXT,
                    updated_at   TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_task_status ON task_states(status);
                CREATE INDEX IF NOT EXISTS idx_task_worker ON task_states(worker);",
            )?;

            // Load existing rows into memory
            let mut states = HashMap::new();
            {
                let mut stmt = conn.prepare(
                    "SELECT task_id, status, worker, started_at, duration_secs, result_summary, error
                     FROM task_states",
                )?;
                let rows = stmt.query_map([], |row| {
                    let task_id: String = row.get(0)?;
                    let status: String = row.get(1)?;
                    let worker: Option<String> = row.get(2)?;
                    let started_at: Option<String> = row.get(3)?;
                    let duration_secs: Option<f64> = row.get(4)?;
                    let result_summary: Option<String> = row.get(5)?;
                    let error: Option<String> = row.get(6)?;

                    let state = match status.as_str() {
                        "pending" => TaskState::Pending,
                        "running" => TaskState::Running {
                            worker: worker.unwrap_or_default(),
                            started_at: started_at.unwrap_or_default(),
                        },
                        "completed" => TaskState::Completed {
                            worker: worker.unwrap_or_default(),
                            duration_secs: duration_secs.unwrap_or(0.0),
                            result_summary: result_summary.unwrap_or_default(),
                        },
                        "failed" => TaskState::Failed {
                            worker: worker.unwrap_or_default(),
                            error: error.unwrap_or_default(),
                        },
                        _ => TaskState::Pending,
                    };
                    Ok((task_id, state))
                })?;

                for row in rows {
                    let (id, state) = row?;
                    states.insert(id, state);
                }
            }

            Ok((conn, states))
        }).await.map_err(|e| anyhow::anyhow!("spawn_blocking join error: {}", e))??;

        Ok(Self {
            states: Mutex::new(states),
            db: Some(Mutex::new(conn)),
        })
    }

    /// Update (or insert) the state for a task.
    pub fn update_state(&self, task_id: &str, state: TaskState) -> Result<()> {
        // Update in-memory
        {
            let mut map = self.states.lock().unwrap();
            map.insert(task_id.to_string(), state.clone());
        }

        // Persist to SQLite if available
        if let Some(db) = &self.db {
            let conn = db.lock().unwrap();
            let now = Utc::now().to_rfc3339();

            let (status, worker, started_at, duration_secs, result_summary, error) = match &state {
                TaskState::Pending => ("pending", None, None, None, None, None),
                TaskState::Running { worker, started_at } => {
                    ("running", Some(worker.as_str()), Some(started_at.as_str()), None, None, None)
                }
                TaskState::Completed { worker, duration_secs, result_summary } => {
                    ("completed", Some(worker.as_str()), None, Some(*duration_secs), Some(result_summary.as_str()), None)
                }
                TaskState::Failed { worker, error } => {
                    ("failed", Some(worker.as_str()), None, None, None, Some(error.as_str()))
                }
            };

            conn.execute(
                "INSERT OR REPLACE INTO task_states
                 (task_id, status, worker, started_at, duration_secs, result_summary, error, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![task_id, status, worker, started_at, duration_secs, result_summary, error, now],
            )?;
        }

        Ok(())
    }

    /// Get the current state of a task.
    pub fn get_state(&self, task_id: &str) -> Option<TaskState> {
        let map = self.states.lock().unwrap();
        map.get(task_id).cloned()
    }

    /// List all tasks assigned to a specific worker.
    pub fn list_by_worker(&self, worker: &str) -> Vec<(String, TaskState)> {
        let map = self.states.lock().unwrap();
        map.iter()
            .filter(|(_, state)| state.worker() == Some(worker))
            .map(|(id, state)| (id.clone(), state.clone()))
            .collect()
    }

    /// List all tasks matching a status string ("pending", "running", "completed", "failed").
    pub fn list_by_status(&self, status: &str) -> Vec<(String, TaskState)> {
        let map = self.states.lock().unwrap();
        map.iter()
            .filter(|(_, state)| state.status_name() == status)
            .map(|(id, state)| (id.clone(), state.clone()))
            .collect()
    }

    /// Remove a task from tracking (cleanup after completion).
    pub fn remove(&self, task_id: &str) -> Result<bool> {
        let existed = {
            let mut map = self.states.lock().unwrap();
            map.remove(task_id).is_some()
        };

        if let Some(db) = &self.db {
            let conn = db.lock().unwrap();
            conn.execute("DELETE FROM task_states WHERE task_id = ?1", params![task_id])?;
        }

        Ok(existed)
    }

    /// Count tasks by status.
    pub fn count_by_status(&self) -> HashMap<String, usize> {
        let map = self.states.lock().unwrap();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for state in map.values() {
            *counts.entry(state.status_name().to_string()).or_insert(0) += 1;
        }
        counts
    }

    /// Total number of tracked tasks.
    pub fn total_count(&self) -> usize {
        let map = self.states.lock().unwrap();
        map.len()
    }
}

// ── File Transfer Registry ──────────────────────────────────────────────────

/// A pending or completed file transfer between cluster nodes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileTransfer {
    pub transfer_id: String,
    pub source_worker: String,
    pub path: String,
    pub target_worker: String,
    pub registered_at: String,
    pub transferred_at: Option<String>,
}

/// Tracks files that need to be synced between cluster nodes.
///
/// Uses SQLite for persistence so transfer state survives restarts.
pub struct FileTransferRegistry {
    conn: Mutex<Connection>,
}

impl FileTransferRegistry {
    /// Create a new registry with SQLite backing at `db_path`.
    pub async fn new(db_path: &str) -> Result<Self> {
        let path = db_path.to_string();
        let conn = tokio::task::spawn_blocking(move || -> Result<Connection> {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }

            let conn = Connection::open(&path)?;
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;")?;

            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS file_transfers (
                    transfer_id    TEXT PRIMARY KEY,
                    source_worker  TEXT NOT NULL,
                    path           TEXT NOT NULL,
                    target_worker  TEXT NOT NULL,
                    registered_at  TEXT NOT NULL,
                    transferred_at TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_ft_target ON file_transfers(target_worker);
                CREATE INDEX IF NOT EXISTS idx_ft_pending ON file_transfers(transferred_at);",
            )?;

            Ok(conn)
        }).await.map_err(|e| anyhow::anyhow!("spawn_blocking join error: {}", e))??;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Register a file that needs to be transferred from `source_worker` to each of `target_workers`.
    ///
    /// Creates one transfer record per target worker. Returns the generated transfer IDs.
    pub fn register_file(
        &self,
        source_worker: &str,
        path: &str,
        target_workers: Vec<String>,
    ) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        let mut ids = Vec::new();

        for target in &target_workers {
            let transfer_id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO file_transfers (transfer_id, source_worker, path, target_worker, registered_at, transferred_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
                params![transfer_id, source_worker, path, target, now],
            )?;
            ids.push(transfer_id);
        }

        Ok(ids)
    }

    /// Get all pending (not yet transferred) file transfers for a specific worker.
    pub fn pending_transfers(&self, worker: &str) -> Result<Vec<FileTransfer>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT transfer_id, source_worker, path, target_worker, registered_at, transferred_at
             FROM file_transfers
             WHERE target_worker = ?1 AND transferred_at IS NULL
             ORDER BY registered_at ASC",
        )?;

        let transfers = stmt
            .query_map(params![worker], |row| {
                Ok(FileTransfer {
                    transfer_id: row.get(0)?,
                    source_worker: row.get(1)?,
                    path: row.get(2)?,
                    target_worker: row.get(3)?,
                    registered_at: row.get(4)?,
                    transferred_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(transfers)
    }

    /// Mark a transfer as completed.
    pub fn mark_transferred(&self, transfer_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        let updated = conn.execute(
            "UPDATE file_transfers SET transferred_at = ?1 WHERE transfer_id = ?2 AND transferred_at IS NULL",
            params![now, transfer_id],
        )?;

        if updated == 0 {
            return Err(anyhow!(
                "No pending transfer found with id '{}'",
                transfer_id
            ));
        }

        Ok(())
    }

    /// Get a specific transfer by ID.
    pub fn get_transfer(&self, transfer_id: &str) -> Result<Option<FileTransfer>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT transfer_id, source_worker, path, target_worker, registered_at, transferred_at
             FROM file_transfers WHERE transfer_id = ?1",
        )?;

        let transfer = stmt
            .query_row(params![transfer_id], |row| {
                Ok(FileTransfer {
                    transfer_id: row.get(0)?,
                    source_worker: row.get(1)?,
                    path: row.get(2)?,
                    target_worker: row.get(3)?,
                    registered_at: row.get(4)?,
                    transferred_at: row.get(5)?,
                })
            })
            .ok();

        Ok(transfer)
    }

    /// Count pending transfers across all workers.
    pub fn pending_count(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM file_transfers WHERE transferred_at IS NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Get transfer history (including completed), newest first.
    pub fn history(&self, limit: i64) -> Result<Vec<FileTransfer>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT transfer_id, source_worker, path, target_worker, registered_at, transferred_at
             FROM file_transfers ORDER BY registered_at DESC LIMIT ?1",
        )?;

        let transfers = stmt
            .query_map(params![limit], |row| {
                Ok(FileTransfer {
                    transfer_id: row.get(0)?,
                    source_worker: row.get(1)?,
                    path: row.get(2)?,
                    target_worker: row.get(3)?,
                    registered_at: row.get(4)?,
                    transferred_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(transfers)
    }
}

// ── State Sync Protocol ─────────────────────────────────────────────────────

/// Messages exchanged between hub and workers for state synchronization.
///
/// Designed for JSON serialization over HTTP transport (POST /cluster/sync).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum StateSyncMessage {
    /// Hub → Worker or Worker → Hub: a task state has changed.
    StateUpdate {
        task_id: String,
        state: TaskState,
        timestamp: String,
    },

    /// Hub → Worker: a file is available for download from `source_worker`.
    FileAvailable {
        transfer_id: String,
        source_worker: String,
        path: String,
        size_bytes: Option<u64>,
        timestamp: String,
    },

    /// Hub ↔ Worker: heartbeat acknowledgement with optional state digest.
    HeartbeatAck {
        worker: String,
        active_tasks: u32,
        pending_transfers: u32,
        timestamp: String,
    },
}

impl StateSyncMessage {
    /// Create a StateUpdate message for the current time.
    pub fn state_update(task_id: &str, state: TaskState) -> Self {
        StateSyncMessage::StateUpdate {
            task_id: task_id.to_string(),
            state,
            timestamp: Utc::now().to_rfc3339(),
        }
    }

    /// Create a FileAvailable message.
    pub fn file_available(
        transfer_id: &str,
        source_worker: &str,
        path: &str,
        size_bytes: Option<u64>,
    ) -> Self {
        StateSyncMessage::FileAvailable {
            transfer_id: transfer_id.to_string(),
            source_worker: source_worker.to_string(),
            path: path.to_string(),
            size_bytes,
            timestamp: Utc::now().to_rfc3339(),
        }
    }

    /// Create a HeartbeatAck message.
    pub fn heartbeat_ack(worker: &str, active_tasks: u32, pending_transfers: u32) -> Self {
        StateSyncMessage::HeartbeatAck {
            worker: worker.to_string(),
            active_tasks,
            pending_transfers,
            timestamp: Utc::now().to_rfc3339(),
        }
    }

    /// Get the message type as a string.
    pub fn message_type(&self) -> &str {
        match self {
            StateSyncMessage::StateUpdate { .. } => "state_update",
            StateSyncMessage::FileAvailable { .. } => "file_available",
            StateSyncMessage::HeartbeatAck { .. } => "heartbeat_ack",
        }
    }

    /// Get the timestamp from any message variant.
    pub fn timestamp(&self) -> &str {
        match self {
            StateSyncMessage::StateUpdate { timestamp, .. } => timestamp,
            StateSyncMessage::FileAvailable { timestamp, .. } => timestamp,
            StateSyncMessage::HeartbeatAck { timestamp, .. } => timestamp,
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn tmp_path() -> String {
        NamedTempFile::new()
            .unwrap()
            .path()
            .to_str()
            .unwrap()
            .to_string()
    }

    // ── TaskState unit tests ────────────────────────────────────────────────

    #[test]
    fn test_task_state_status_names() {
        assert_eq!(TaskState::Pending.status_name(), "pending");
        assert_eq!(
            TaskState::Running {
                worker: "z13".into(),
                started_at: "now".into()
            }
            .status_name(),
            "running"
        );
        assert_eq!(
            TaskState::Completed {
                worker: "m1".into(),
                duration_secs: 1.0,
                result_summary: "ok".into()
            }
            .status_name(),
            "completed"
        );
        assert_eq!(
            TaskState::Failed {
                worker: "acer".into(),
                error: "boom".into()
            }
            .status_name(),
            "failed"
        );
    }

    #[test]
    fn test_task_state_worker_extraction() {
        assert_eq!(TaskState::Pending.worker(), None);
        assert_eq!(
            TaskState::Running {
                worker: "z13".into(),
                started_at: "t".into()
            }
            .worker(),
            Some("z13")
        );
        assert_eq!(
            TaskState::Completed {
                worker: "m1".into(),
                duration_secs: 2.0,
                result_summary: "done".into()
            }
            .worker(),
            Some("m1")
        );
        assert_eq!(
            TaskState::Failed {
                worker: "acer".into(),
                error: "err".into()
            }
            .worker(),
            Some("acer")
        );
    }

    // ── DistributedTaskTracker — in-memory tests ────────────────────────────

    #[test]
    fn test_tracker_memory_update_and_get() {
        let tracker = DistributedTaskTracker::new_memory();

        tracker
            .update_state("task-1", TaskState::Pending)
            .unwrap();
        assert_eq!(tracker.get_state("task-1"), Some(TaskState::Pending));

        tracker
            .update_state(
                "task-1",
                TaskState::Running {
                    worker: "z13".into(),
                    started_at: "2026-03-18T00:00:00Z".into(),
                },
            )
            .unwrap();

        let state = tracker.get_state("task-1").unwrap();
        assert_eq!(state.status_name(), "running");
        assert_eq!(state.worker(), Some("z13"));
    }

    #[test]
    fn test_tracker_get_nonexistent() {
        let tracker = DistributedTaskTracker::new_memory();
        assert_eq!(tracker.get_state("no-such-task"), None);
    }

    #[test]
    fn test_tracker_list_by_status() {
        let tracker = DistributedTaskTracker::new_memory();

        tracker
            .update_state("t1", TaskState::Pending)
            .unwrap();
        tracker
            .update_state("t2", TaskState::Pending)
            .unwrap();
        tracker
            .update_state(
                "t3",
                TaskState::Running {
                    worker: "z13".into(),
                    started_at: "now".into(),
                },
            )
            .unwrap();
        tracker
            .update_state(
                "t4",
                TaskState::Completed {
                    worker: "m1".into(),
                    duration_secs: 5.0,
                    result_summary: "ok".into(),
                },
            )
            .unwrap();

        let pending = tracker.list_by_status("pending");
        assert_eq!(pending.len(), 2);

        let running = tracker.list_by_status("running");
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].0, "t3");

        let completed = tracker.list_by_status("completed");
        assert_eq!(completed.len(), 1);

        let failed = tracker.list_by_status("failed");
        assert_eq!(failed.len(), 0);
    }

    #[test]
    fn test_tracker_list_by_worker() {
        let tracker = DistributedTaskTracker::new_memory();

        tracker
            .update_state(
                "t1",
                TaskState::Running {
                    worker: "z13".into(),
                    started_at: "now".into(),
                },
            )
            .unwrap();
        tracker
            .update_state(
                "t2",
                TaskState::Running {
                    worker: "m1".into(),
                    started_at: "now".into(),
                },
            )
            .unwrap();
        tracker
            .update_state(
                "t3",
                TaskState::Completed {
                    worker: "z13".into(),
                    duration_secs: 3.0,
                    result_summary: "done".into(),
                },
            )
            .unwrap();
        tracker
            .update_state("t4", TaskState::Pending)
            .unwrap();

        let z13_tasks = tracker.list_by_worker("z13");
        assert_eq!(z13_tasks.len(), 2);

        let m1_tasks = tracker.list_by_worker("m1");
        assert_eq!(m1_tasks.len(), 1);

        let acer_tasks = tracker.list_by_worker("acer");
        assert_eq!(acer_tasks.len(), 0);
    }

    #[test]
    fn test_tracker_remove() {
        let tracker = DistributedTaskTracker::new_memory();

        tracker.update_state("t1", TaskState::Pending).unwrap();
        assert!(tracker.remove("t1").unwrap());
        assert_eq!(tracker.get_state("t1"), None);

        // Remove nonexistent returns false
        assert!(!tracker.remove("no-such").unwrap());
    }

    #[test]
    fn test_tracker_count_by_status() {
        let tracker = DistributedTaskTracker::new_memory();

        tracker.update_state("t1", TaskState::Pending).unwrap();
        tracker.update_state("t2", TaskState::Pending).unwrap();
        tracker
            .update_state(
                "t3",
                TaskState::Failed {
                    worker: "z13".into(),
                    error: "timeout".into(),
                },
            )
            .unwrap();

        let counts = tracker.count_by_status();
        assert_eq!(counts.get("pending"), Some(&2));
        assert_eq!(counts.get("failed"), Some(&1));
        assert_eq!(counts.get("running"), None);
    }

    #[test]
    fn test_tracker_total_count() {
        let tracker = DistributedTaskTracker::new_memory();
        assert_eq!(tracker.total_count(), 0);

        tracker.update_state("t1", TaskState::Pending).unwrap();
        tracker.update_state("t2", TaskState::Pending).unwrap();
        assert_eq!(tracker.total_count(), 2);

        tracker.remove("t1").unwrap();
        assert_eq!(tracker.total_count(), 1);
    }

    #[test]
    fn test_tracker_state_transitions() {
        let tracker = DistributedTaskTracker::new_memory();

        // Full lifecycle: Pending → Running → Completed
        tracker.update_state("task-lifecycle", TaskState::Pending).unwrap();
        assert_eq!(tracker.get_state("task-lifecycle").unwrap().status_name(), "pending");

        tracker
            .update_state(
                "task-lifecycle",
                TaskState::Running {
                    worker: "z13".into(),
                    started_at: "2026-03-18T00:00:00Z".into(),
                },
            )
            .unwrap();
        assert_eq!(tracker.get_state("task-lifecycle").unwrap().status_name(), "running");

        tracker
            .update_state(
                "task-lifecycle",
                TaskState::Completed {
                    worker: "z13".into(),
                    duration_secs: 12.5,
                    result_summary: "Generated 500 words".into(),
                },
            )
            .unwrap();
        let final_state = tracker.get_state("task-lifecycle").unwrap();
        assert_eq!(final_state.status_name(), "completed");
        if let TaskState::Completed { duration_secs, result_summary, .. } = final_state {
            assert!((duration_secs - 12.5).abs() < f64::EPSILON);
            assert_eq!(result_summary, "Generated 500 words");
        } else {
            panic!("Expected Completed state");
        }
    }

    // ── DistributedTaskTracker — persistent tests ───────────────────────────

    #[tokio::test]
    async fn test_tracker_persistent_roundtrip() {
        let path = tmp_path();

        // Create tracker and insert data
        {
            let tracker = DistributedTaskTracker::new_persistent(&path).await.unwrap();
            tracker.update_state("p1", TaskState::Pending).unwrap();
            tracker
                .update_state(
                    "p2",
                    TaskState::Running {
                        worker: "m1".into(),
                        started_at: "2026-03-18T10:00:00Z".into(),
                    },
                )
                .unwrap();
            tracker
                .update_state(
                    "p3",
                    TaskState::Completed {
                        worker: "acer".into(),
                        duration_secs: 8.3,
                        result_summary: "done".into(),
                    },
                )
                .unwrap();
            tracker
                .update_state(
                    "p4",
                    TaskState::Failed {
                        worker: "z13".into(),
                        error: "out of memory".into(),
                    },
                )
                .unwrap();
        }

        // Re-open and verify data was loaded from SQLite
        {
            let tracker = DistributedTaskTracker::new_persistent(&path).await.unwrap();
            assert_eq!(tracker.total_count(), 4);
            assert_eq!(tracker.get_state("p1").unwrap().status_name(), "pending");
            assert_eq!(tracker.get_state("p2").unwrap().status_name(), "running");
            assert_eq!(tracker.get_state("p3").unwrap().status_name(), "completed");

            let failed = tracker.get_state("p4").unwrap();
            if let TaskState::Failed { error, .. } = failed {
                assert_eq!(error, "out of memory");
            } else {
                panic!("Expected Failed state");
            }
        }
    }

    #[tokio::test]
    async fn test_tracker_persistent_remove() {
        let path = tmp_path();

        {
            let tracker = DistributedTaskTracker::new_persistent(&path).await.unwrap();
            tracker.update_state("rm1", TaskState::Pending).unwrap();
            tracker.update_state("rm2", TaskState::Pending).unwrap();
            tracker.remove("rm1").unwrap();
        }

        // Re-open: rm1 should be gone, rm2 should remain
        {
            let tracker = DistributedTaskTracker::new_persistent(&path).await.unwrap();
            assert_eq!(tracker.get_state("rm1"), None);
            assert_eq!(tracker.get_state("rm2").unwrap().status_name(), "pending");
        }
    }

    // ── FileTransferRegistry tests ──────────────────────────────────────────

    #[tokio::test]
    async fn test_file_registry_register_and_pending() {
        let path = tmp_path();
        let registry = FileTransferRegistry::new(&path).await.unwrap();

        let ids = registry
            .register_file("z13", "/workspace/output.pdf", vec!["m1".into(), "acer".into()])
            .unwrap();
        assert_eq!(ids.len(), 2);

        let m1_pending = registry.pending_transfers("m1").unwrap();
        assert_eq!(m1_pending.len(), 1);
        assert_eq!(m1_pending[0].source_worker, "z13");
        assert_eq!(m1_pending[0].path, "/workspace/output.pdf");

        let acer_pending = registry.pending_transfers("acer").unwrap();
        assert_eq!(acer_pending.len(), 1);

        // No pending for z13 (it's the source, not a target)
        let z13_pending = registry.pending_transfers("z13").unwrap();
        assert_eq!(z13_pending.len(), 0);
    }

    #[tokio::test]
    async fn test_file_registry_mark_transferred() {
        let path = tmp_path();
        let registry = FileTransferRegistry::new(&path).await.unwrap();

        let ids = registry
            .register_file("z13", "/data/model.bin", vec!["m1".into()])
            .unwrap();
        let tid = &ids[0];

        assert_eq!(registry.pending_count().unwrap(), 1);

        registry.mark_transferred(tid).unwrap();

        assert_eq!(registry.pending_count().unwrap(), 0);
        assert_eq!(registry.pending_transfers("m1").unwrap().len(), 0);

        // Verify it's completed
        let transfer = registry.get_transfer(tid).unwrap().unwrap();
        assert!(transfer.transferred_at.is_some());
    }

    #[tokio::test]
    async fn test_file_registry_mark_transferred_nonexistent() {
        let path = tmp_path();
        let registry = FileTransferRegistry::new(&path).await.unwrap();

        let result = registry.mark_transferred("no-such-id");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_file_registry_double_mark_fails() {
        let path = tmp_path();
        let registry = FileTransferRegistry::new(&path).await.unwrap();

        let ids = registry
            .register_file("z13", "/tmp/file.txt", vec!["acer".into()])
            .unwrap();
        let tid = &ids[0];

        registry.mark_transferred(tid).unwrap();
        let result = registry.mark_transferred(tid);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_file_registry_history() {
        let path = tmp_path();
        let registry = FileTransferRegistry::new(&path).await.unwrap();

        registry
            .register_file("z13", "/a.txt", vec!["m1".into()])
            .unwrap();
        registry
            .register_file("m1", "/b.txt", vec!["acer".into()])
            .unwrap();
        registry
            .register_file("z13", "/c.txt", vec!["m1".into(), "acer".into()])
            .unwrap();

        let history = registry.history(10).unwrap();
        assert_eq!(history.len(), 4); // 1 + 1 + 2
    }

    #[tokio::test]
    async fn test_file_registry_pending_count() {
        let path = tmp_path();
        let registry = FileTransferRegistry::new(&path).await.unwrap();

        let ids1 = registry
            .register_file("z13", "/x.txt", vec!["m1".into(), "acer".into()])
            .unwrap();
        assert_eq!(registry.pending_count().unwrap(), 2);

        registry.mark_transferred(&ids1[0]).unwrap();
        assert_eq!(registry.pending_count().unwrap(), 1);

        registry.mark_transferred(&ids1[1]).unwrap();
        assert_eq!(registry.pending_count().unwrap(), 0);
    }

    // ── StateSyncMessage tests ──────────────────────────────────────────────

    #[test]
    fn test_sync_message_state_update_serde() {
        let msg = StateSyncMessage::state_update(
            "task-42",
            TaskState::Running {
                worker: "z13".into(),
                started_at: "2026-03-18T12:00:00Z".into(),
            },
        );

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"StateUpdate\""));
        assert!(json.contains("task-42"));

        let deserialized: StateSyncMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg.message_type(), deserialized.message_type());
        if let StateSyncMessage::StateUpdate { task_id, state, .. } = &deserialized {
            assert_eq!(task_id, "task-42");
            assert_eq!(state.status_name(), "running");
        } else {
            panic!("Expected StateUpdate");
        }
    }

    #[test]
    fn test_sync_message_file_available_serde() {
        let msg = StateSyncMessage::file_available("xfer-1", "z13", "/workspace/report.pdf", Some(1048576));

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"FileAvailable\""));

        let deserialized: StateSyncMessage = serde_json::from_str(&json).unwrap();
        if let StateSyncMessage::FileAvailable {
            transfer_id,
            source_worker,
            path,
            size_bytes,
            ..
        } = &deserialized
        {
            assert_eq!(transfer_id, "xfer-1");
            assert_eq!(source_worker, "z13");
            assert_eq!(path, "/workspace/report.pdf");
            assert_eq!(*size_bytes, Some(1048576));
        } else {
            panic!("Expected FileAvailable");
        }
    }

    #[test]
    fn test_sync_message_heartbeat_ack_serde() {
        let msg = StateSyncMessage::heartbeat_ack("m1-mac", 3, 2);

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"HeartbeatAck\""));

        let deserialized: StateSyncMessage = serde_json::from_str(&json).unwrap();
        if let StateSyncMessage::HeartbeatAck {
            worker,
            active_tasks,
            pending_transfers,
            ..
        } = &deserialized
        {
            assert_eq!(worker, "m1-mac");
            assert_eq!(*active_tasks, 3);
            assert_eq!(*pending_transfers, 2);
        } else {
            panic!("Expected HeartbeatAck");
        }
    }

    #[test]
    fn test_sync_message_type_method() {
        let update = StateSyncMessage::state_update("t", TaskState::Pending);
        assert_eq!(update.message_type(), "state_update");

        let file = StateSyncMessage::file_available("f", "z13", "/a", None);
        assert_eq!(file.message_type(), "file_available");

        let hb = StateSyncMessage::heartbeat_ack("w", 0, 0);
        assert_eq!(hb.message_type(), "heartbeat_ack");
    }

    #[test]
    fn test_sync_message_timestamp_method() {
        let msg = StateSyncMessage::state_update("t", TaskState::Pending);
        // Timestamp should be a valid RFC 3339 string
        let ts = msg.timestamp();
        assert!(ts.contains("T"), "Timestamp should be RFC 3339 format");
    }

    #[test]
    fn test_sync_message_file_available_no_size() {
        let msg = StateSyncMessage::file_available("xfer-2", "acer", "/tmp/data.csv", None);
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: StateSyncMessage = serde_json::from_str(&json).unwrap();
        if let StateSyncMessage::FileAvailable { size_bytes, .. } = deserialized {
            assert_eq!(size_bytes, None);
        } else {
            panic!("Expected FileAvailable");
        }
    }

    #[test]
    fn test_task_state_serde_roundtrip() {
        let states = vec![
            TaskState::Pending,
            TaskState::Running {
                worker: "z13".into(),
                started_at: "2026-03-18T00:00:00Z".into(),
            },
            TaskState::Completed {
                worker: "m1".into(),
                duration_secs: 42.5,
                result_summary: "All good".into(),
            },
            TaskState::Failed {
                worker: "acer".into(),
                error: "connection refused".into(),
            },
        ];

        for state in &states {
            let json = serde_json::to_string(state).unwrap();
            let deserialized: TaskState = serde_json::from_str(&json).unwrap();
            assert_eq!(state, &deserialized);
        }
    }
}
