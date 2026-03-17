//! Task Preemption — allows P0 tasks to preempt lower-priority running tasks.
//!
//! When a Critical/High (P0-P1) task arrives and only lower-priority workers are
//! busy, this module identifies which running P2-P3 task can be preempted,
//! checkpoints its state to SQLite, and moves it back to the queue.

use anyhow::{anyhow, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

// ── Data Structures ──────────────────────────────────────────────────────────

/// A currently running task on a worker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunningTask {
    pub task_id: String,
    pub priority: u8,
    pub worker_name: String,
    pub started_at: String,
    pub checkpoint_data: Option<String>,
}

/// Plan describing which task to preempt and why
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreemptionPlan {
    pub target_task_id: String,
    pub target_worker: String,
    pub target_priority: u8,
    pub reason: String,
    pub incoming_priority: u8,
}

/// Record of a preempted task stored in SQLite
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreemptedRecord {
    pub task_id: String,
    pub original_priority: u8,
    pub worker_name: String,
    pub checkpoint_data: Option<String>,
    pub preempted_at: String,
    pub restored_at: Option<String>,
}

/// Result of a preemption check
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PreemptionDecision {
    /// No preemption needed or possible
    None,
    /// A task can be preempted
    Preempt(PreemptionPlan),
}

// ── PreemptionManager ────────────────────────────────────────────────────────

/// Manages task preemption with SQLite-backed checkpoint storage.
pub struct PreemptionManager {
    conn: Mutex<Connection>,
}

impl PreemptionManager {
    /// Create a new PreemptionManager, creating the `preempted_tasks` table if needed.
    pub fn new(db_path: &str) -> Result<Self> {
        if let Some(parent) = std::path::Path::new(db_path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS preempted_tasks (
                task_id          TEXT PRIMARY KEY,
                original_priority INTEGER NOT NULL,
                worker_name      TEXT NOT NULL,
                checkpoint_data  TEXT,
                preempted_at     TEXT NOT NULL,
                restored_at      TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_preempted_restored ON preempted_tasks(restored_at);",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Check whether an incoming task should preempt any running task.
    ///
    /// Rules:
    /// - Only P0 (priority 0) and P1 (priority 1) tasks can trigger preemption.
    /// - Only P2 (priority 2) and P3 (priority 3) tasks can be preempted.
    /// - Among eligible targets, pick the lowest priority (highest number).
    ///   Ties are broken by longest running (earliest started_at).
    /// - Never preempt P0 or P1 tasks.
    pub fn check_preemption(
        &self,
        incoming_priority: u8,
        running_tasks: &[RunningTask],
    ) -> Option<PreemptionPlan> {
        // Only P0-P1 can trigger preemption
        if incoming_priority > 1 {
            return None;
        }

        // Find preemptable tasks (P2-P3 only)
        let mut candidates: Vec<&RunningTask> = running_tasks
            .iter()
            .filter(|t| t.priority >= 2 && t.priority <= 3)
            .collect();

        if candidates.is_empty() {
            return None;
        }

        // Sort: lowest priority first (highest number), then earliest started_at
        candidates.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.started_at.cmp(&b.started_at))
        });

        let target = candidates[0];

        Some(PreemptionPlan {
            target_task_id: target.task_id.clone(),
            target_worker: target.worker_name.clone(),
            target_priority: target.priority,
            reason: format!(
                "P{} incoming preempts P{} task '{}' on worker '{}'",
                incoming_priority, target.priority, target.task_id, target.worker_name
            ),
            incoming_priority,
        })
    }

    /// Execute a preemption: checkpoint the target task into SQLite.
    ///
    /// The caller is responsible for actually stopping the task on the worker
    /// and re-queuing it. This method persists the checkpoint state.
    pub fn preempt(&self, plan: &PreemptionPlan, checkpoint_data: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        conn.execute(
            "INSERT OR REPLACE INTO preempted_tasks
             (task_id, original_priority, worker_name, checkpoint_data, preempted_at, restored_at)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
            params![
                plan.target_task_id,
                plan.target_priority,
                plan.target_worker,
                checkpoint_data,
                now,
            ],
        )?;

        Ok(())
    }

    /// Mark a preempted task as restored (it has been re-queued and picked up).
    pub fn restore(&self, task_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        let updated = conn.execute(
            "UPDATE preempted_tasks SET restored_at = ?1 WHERE task_id = ?2 AND restored_at IS NULL",
            params![now, task_id],
        )?;

        if updated == 0 {
            return Err(anyhow!(
                "No unrestored preempted task found with id '{}'",
                task_id
            ));
        }

        Ok(())
    }

    /// Get the checkpoint data for a preempted task (if it exists and hasn't been restored).
    pub fn get_checkpoint(&self, task_id: &str) -> Result<Option<PreemptedRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT task_id, original_priority, worker_name, checkpoint_data, preempted_at, restored_at
             FROM preempted_tasks WHERE task_id = ?1",
        )?;

        let record = stmt
            .query_row(params![task_id], |row| {
                Ok(PreemptedRecord {
                    task_id: row.get(0)?,
                    original_priority: row.get::<_, i32>(1)? as u8,
                    worker_name: row.get(2)?,
                    checkpoint_data: row.get(3)?,
                    preempted_at: row.get(4)?,
                    restored_at: row.get(5)?,
                })
            })
            .ok();

        Ok(record)
    }

    /// List all currently preempted (unrestored) tasks.
    pub fn pending_restorations(&self) -> Result<Vec<PreemptedRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT task_id, original_priority, worker_name, checkpoint_data, preempted_at, restored_at
             FROM preempted_tasks WHERE restored_at IS NULL ORDER BY preempted_at ASC",
        )?;

        let records = stmt
            .query_map([], |row| {
                Ok(PreemptedRecord {
                    task_id: row.get(0)?,
                    original_priority: row.get::<_, i32>(1)? as u8,
                    worker_name: row.get(2)?,
                    checkpoint_data: row.get(3)?,
                    preempted_at: row.get(4)?,
                    restored_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(records)
    }

    /// Get full preemption history (including restored tasks).
    pub fn history(&self, limit: i64) -> Result<Vec<PreemptedRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT task_id, original_priority, worker_name, checkpoint_data, preempted_at, restored_at
             FROM preempted_tasks ORDER BY preempted_at DESC LIMIT ?1",
        )?;

        let records = stmt
            .query_map(params![limit], |row| {
                Ok(PreemptedRecord {
                    task_id: row.get(0)?,
                    original_priority: row.get::<_, i32>(1)? as u8,
                    worker_name: row.get(2)?,
                    checkpoint_data: row.get(3)?,
                    preempted_at: row.get(4)?,
                    restored_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(records)
    }

    /// Count of currently preempted (unrestored) tasks.
    pub fn preempted_count(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM preempted_tasks WHERE restored_at IS NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn make_manager() -> PreemptionManager {
        let tmp = NamedTempFile::new().unwrap();
        PreemptionManager::new(tmp.path().to_str().unwrap()).unwrap()
    }

    fn running(task_id: &str, priority: u8, worker: &str, started: &str) -> RunningTask {
        RunningTask {
            task_id: task_id.to_string(),
            priority,
            worker_name: worker.to_string(),
            started_at: started.to_string(),
            checkpoint_data: None,
        }
    }

    #[test]
    fn test_p0_preempts_p3() {
        let mgr = make_manager();
        let tasks = vec![running("t1", 3, "acer", "2026-01-01T00:00:00Z")];
        let plan = mgr.check_preemption(0, &tasks);
        assert!(plan.is_some());
        let plan = plan.unwrap();
        assert_eq!(plan.target_task_id, "t1");
        assert_eq!(plan.target_priority, 3);
        assert_eq!(plan.incoming_priority, 0);
    }

    #[test]
    fn test_p0_preempts_p2() {
        let mgr = make_manager();
        let tasks = vec![running("t1", 2, "z13", "2026-01-01T00:00:00Z")];
        let plan = mgr.check_preemption(0, &tasks);
        assert!(plan.is_some());
        assert_eq!(plan.unwrap().target_task_id, "t1");
    }

    #[test]
    fn test_p1_preempts_p2() {
        let mgr = make_manager();
        let tasks = vec![running("t1", 2, "m1-mac", "2026-01-01T00:00:00Z")];
        let plan = mgr.check_preemption(1, &tasks);
        assert!(plan.is_some());
        assert_eq!(plan.unwrap().target_task_id, "t1");
    }

    #[test]
    fn test_p2_cannot_preempt() {
        let mgr = make_manager();
        let tasks = vec![running("t1", 3, "acer", "2026-01-01T00:00:00Z")];
        let plan = mgr.check_preemption(2, &tasks);
        assert!(plan.is_none());
    }

    #[test]
    fn test_p3_cannot_preempt() {
        let mgr = make_manager();
        let tasks = vec![running("t1", 3, "acer", "2026-01-01T00:00:00Z")];
        let plan = mgr.check_preemption(3, &tasks);
        assert!(plan.is_none());
    }

    #[test]
    fn test_never_preempt_p0_or_p1() {
        let mgr = make_manager();
        let tasks = vec![
            running("t0", 0, "z13", "2026-01-01T00:00:00Z"),
            running("t1", 1, "m1-mac", "2026-01-01T00:00:00Z"),
        ];
        let plan = mgr.check_preemption(0, &tasks);
        assert!(plan.is_none(), "Must never preempt P0 or P1 tasks");
    }

    #[test]
    fn test_prefers_lowest_priority_target() {
        let mgr = make_manager();
        let tasks = vec![
            running("t-p2", 2, "z13", "2026-01-01T00:00:00Z"),
            running("t-p3", 3, "acer", "2026-01-01T00:00:00Z"),
        ];
        let plan = mgr.check_preemption(0, &tasks).unwrap();
        assert_eq!(plan.target_task_id, "t-p3", "Should preempt P3 before P2");
    }

    #[test]
    fn test_tiebreak_by_earliest_started() {
        let mgr = make_manager();
        let tasks = vec![
            running("t-new", 3, "z13", "2026-01-01T01:00:00Z"),
            running("t-old", 3, "acer", "2026-01-01T00:00:00Z"),
        ];
        let plan = mgr.check_preemption(0, &tasks).unwrap();
        assert_eq!(
            plan.target_task_id, "t-old",
            "Among same priority, preempt the longest-running task"
        );
    }

    #[test]
    fn test_no_preemptable_tasks() {
        let mgr = make_manager();
        let tasks: Vec<RunningTask> = vec![];
        let plan = mgr.check_preemption(0, &tasks);
        assert!(plan.is_none());
    }

    #[test]
    fn test_preempt_and_restore_lifecycle() {
        let mgr = make_manager();

        let plan = PreemptionPlan {
            target_task_id: "task-abc".to_string(),
            target_worker: "acer".to_string(),
            target_priority: 3,
            reason: "P0 preempts P3".to_string(),
            incoming_priority: 0,
        };

        // Preempt with checkpoint
        mgr.preempt(&plan, Some(r#"{"step":3,"partial":"hello"}"#))
            .unwrap();

        // Verify checkpoint exists
        let record = mgr.get_checkpoint("task-abc").unwrap().unwrap();
        assert_eq!(record.original_priority, 3);
        assert_eq!(record.worker_name, "acer");
        assert!(record.checkpoint_data.is_some());
        assert!(record.restored_at.is_none());

        // Count pending
        assert_eq!(mgr.preempted_count().unwrap(), 1);
        assert_eq!(mgr.pending_restorations().unwrap().len(), 1);

        // Restore
        mgr.restore("task-abc").unwrap();

        // After restore
        let record = mgr.get_checkpoint("task-abc").unwrap().unwrap();
        assert!(record.restored_at.is_some());
        assert_eq!(mgr.preempted_count().unwrap(), 0);
    }

    #[test]
    fn test_restore_nonexistent_fails() {
        let mgr = make_manager();
        let result = mgr.restore("no-such-task");
        assert!(result.is_err());
    }

    #[test]
    fn test_double_restore_fails() {
        let mgr = make_manager();
        let plan = PreemptionPlan {
            target_task_id: "task-xyz".to_string(),
            target_worker: "z13".to_string(),
            target_priority: 2,
            reason: "test".to_string(),
            incoming_priority: 0,
        };
        mgr.preempt(&plan, None).unwrap();
        mgr.restore("task-xyz").unwrap();

        // Second restore should fail (already restored)
        let result = mgr.restore("task-xyz");
        assert!(result.is_err());
    }

    #[test]
    fn test_preempt_without_checkpoint_data() {
        let mgr = make_manager();
        let plan = PreemptionPlan {
            target_task_id: "task-nochk".to_string(),
            target_worker: "m1-mac".to_string(),
            target_priority: 3,
            reason: "test".to_string(),
            incoming_priority: 1,
        };
        mgr.preempt(&plan, None).unwrap();

        let record = mgr.get_checkpoint("task-nochk").unwrap().unwrap();
        assert!(record.checkpoint_data.is_none());
        assert_eq!(record.original_priority, 3);
    }

    #[test]
    fn test_history_returns_all() {
        let mgr = make_manager();

        for i in 0..5 {
            let plan = PreemptionPlan {
                target_task_id: format!("task-{}", i),
                target_worker: "acer".to_string(),
                target_priority: 3,
                reason: "test".to_string(),
                incoming_priority: 0,
            };
            mgr.preempt(&plan, Some(&format!("checkpoint-{}", i)))
                .unwrap();
        }
        // Restore some
        mgr.restore("task-0").unwrap();
        mgr.restore("task-1").unwrap();

        let history = mgr.history(10).unwrap();
        assert_eq!(history.len(), 5);

        let pending = mgr.pending_restorations().unwrap();
        assert_eq!(pending.len(), 3);
    }

    #[test]
    fn test_mixed_priorities_only_preempts_eligible() {
        let mgr = make_manager();
        let tasks = vec![
            running("t-p0", 0, "z13", "2026-01-01T00:00:00Z"),
            running("t-p1", 1, "m1-mac", "2026-01-01T00:00:00Z"),
            running("t-p2", 2, "ayaneo", "2026-01-01T00:00:00Z"),
            running("t-p3", 3, "acer", "2026-01-01T00:00:00Z"),
        ];
        let plan = mgr.check_preemption(0, &tasks).unwrap();
        // Should preempt P3 (lowest priority among eligible)
        assert_eq!(plan.target_task_id, "t-p3");
        // P0 and P1 should never be targets
        assert!(plan.target_priority >= 2);
    }
}
