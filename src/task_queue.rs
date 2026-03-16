use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use uuid::Uuid;

use crate::LlmRouter;

/// Task status values
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    Running,
    Done,
    Failed,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "pending"),
            TaskStatus::Running => write!(f, "running"),
            TaskStatus::Done => write!(f, "done"),
            TaskStatus::Failed => write!(f, "failed"),
        }
    }
}

/// Task priority levels (lower number = higher priority)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum TaskPriority {
    Critical = 0,
    High = 1,
    Normal = 2,
    Low = 3,
}

impl Default for TaskPriority {
    fn default() -> Self { TaskPriority::Normal }
}

impl std::fmt::Display for TaskPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskPriority::Critical => write!(f, "critical"),
            TaskPriority::High => write!(f, "high"),
            TaskPriority::Normal => write!(f, "normal"),
            TaskPriority::Low => write!(f, "low"),
        }
    }
}

impl TaskPriority {
    pub fn from_str(s: &str) -> Self {
        match s {
            "critical" | "0" => TaskPriority::Critical,
            "high" | "1" => TaskPriority::High,
            "low" | "3" => TaskPriority::Low,
            _ => TaskPriority::Normal,
        }
    }

    pub fn as_i32(&self) -> i32 {
        *self as i32
    }
}

/// A task record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub task_id: String,
    pub title: String,
    pub prompt: String,
    pub status: TaskStatus,
    pub result: Option<String>,
    pub strategy_used: Option<String>,
    pub feedback_score: Option<f64>,
    pub priority: TaskPriority,
    pub idempotency_key: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// SQLite-backed task queue (WAL mode for concurrent access)
pub struct TaskQueue {
    conn: Mutex<Connection>,
}

impl TaskQueue {
    /// Open (or create) the task queue database
    pub async fn new(db_path: &str) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = std::path::Path::new(db_path).parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;

        // Enable WAL for concurrent reads
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

        // Create table if not exists (with new columns for fresh DBs)
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tasks (
                task_id        TEXT PRIMARY KEY,
                title          TEXT NOT NULL,
                prompt         TEXT NOT NULL,
                status         TEXT NOT NULL DEFAULT 'pending',
                result         TEXT,
                strategy_used  TEXT,
                feedback_score REAL,
                priority       INTEGER NOT NULL DEFAULT 2,
                idempotency_key TEXT,
                created_at     TEXT NOT NULL,
                updated_at     TEXT NOT NULL
            );",
        )?;

        // Migrate existing tables: add columns if missing
        let has_priority: bool = conn.prepare("SELECT priority FROM tasks LIMIT 0").is_ok();
        if !has_priority {
            let _ = conn.execute_batch(
                "ALTER TABLE tasks ADD COLUMN priority INTEGER NOT NULL DEFAULT 2;
                 ALTER TABLE tasks ADD COLUMN idempotency_key TEXT;"
            );
        }

        // Create indexes (safe to run after migration)
        let _ = conn.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_idempotency ON tasks(idempotency_key) WHERE idempotency_key IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_tasks_priority_status ON tasks(priority, status);"
        );

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Add a new task, return its task_id
    pub async fn add(&self, title: &str, prompt: &str) -> Result<String> {
        self.add_with_options(title, prompt, TaskPriority::Normal, None).await
    }

    /// Add a task with priority and optional idempotency key.
    /// If idempotency_key is provided and a task with the same key already exists,
    /// returns the existing task_id instead of creating a duplicate.
    pub async fn add_with_options(
        &self,
        title: &str,
        prompt: &str,
        priority: TaskPriority,
        idempotency_key: Option<&str>,
    ) -> Result<String> {
        let conn = self.conn.lock().unwrap();

        // Check idempotency: return existing task_id if key already exists
        if let Some(key) = idempotency_key {
            let mut stmt = conn.prepare(
                "SELECT task_id FROM tasks WHERE idempotency_key = ?1"
            )?;
            if let Ok(existing_id) = stmt.query_row(params![key], |row| row.get::<_, String>(0)) {
                return Ok(existing_id);
            }
        }

        let task_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO tasks (task_id, title, prompt, status, priority, idempotency_key, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6, ?6)",
            params![task_id, title, prompt, priority.as_i32(), idempotency_key, now],
        )?;
        Ok(task_id)
    }

    /// Run a task using the LLM router, updating status in DB
    pub async fn run(&self, task_id: &str, router: &LlmRouter) -> Result<String> {
        let prompt = {
            let conn = self.conn.lock().unwrap();
            let mut stmt =
                conn.prepare("SELECT prompt FROM tasks WHERE task_id = ?1")?;
            stmt.query_row(params![task_id], |row| row.get::<_, String>(0))?
        };

        // Mark running
        self.set_status(task_id, TaskStatus::Running, None, None)?;

        // Run LLM
        match router.route(&prompt, "auto").await {
            Ok(result) => {
                self.set_status(task_id, TaskStatus::Done, Some(&result), Some("ollama"))?;
                Ok(result)
            }
            Err(e) => {
                let err_str = e.to_string();
                self.set_status(task_id, TaskStatus::Failed, Some(&err_str), None)?;
                Err(e)
            }
        }
    }

    /// Get recent task history, ordered by priority (highest first) then creation time
    pub async fn history(&self, limit: i64) -> Result<Vec<Task>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT task_id, title, prompt, status, result, strategy_used,
                    feedback_score, priority, idempotency_key, created_at, updated_at
             FROM tasks ORDER BY priority ASC, created_at DESC LIMIT ?1",
        )?;

        let tasks = stmt
            .query_map(params![limit], |row| {
                let status_str: String = row.get(3)?;
                let status = match status_str.as_str() {
                    "running" => TaskStatus::Running,
                    "done" => TaskStatus::Done,
                    "failed" => TaskStatus::Failed,
                    _ => TaskStatus::Pending,
                };
                let priority_val: i32 = row.get(7)?;
                let priority = match priority_val {
                    0 => TaskPriority::Critical,
                    1 => TaskPriority::High,
                    3 => TaskPriority::Low,
                    _ => TaskPriority::Normal,
                };
                Ok(Task {
                    task_id: row.get(0)?,
                    title: row.get(1)?,
                    prompt: row.get(2)?,
                    status,
                    result: row.get(4)?,
                    strategy_used: row.get(5)?,
                    feedback_score: row.get(6)?,
                    priority,
                    idempotency_key: row.get(8)?,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(tasks)
    }

    /// Get next pending task by priority (highest priority first)
    pub async fn next_pending(&self) -> Result<Option<Task>> {
        let tasks = self.history(1).await?;
        Ok(tasks.into_iter().find(|t| t.status == TaskStatus::Pending))
    }

    pub fn set_status(
        &self,
        task_id: &str,
        status: TaskStatus,
        result: Option<&str>,
        strategy: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE tasks SET status=?1, result=?2, strategy_used=?3, updated_at=?4
             WHERE task_id=?5",
            params![status.to_string(), result, strategy, now, task_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn make_queue() -> TaskQueue {
        TaskQueue::new(":memory:").await.unwrap()
    }

    #[tokio::test]
    async fn test_add_task() {
        let q = make_queue().await;
        let id = q.add("test task", "hello world").await.unwrap();
        assert!(!id.is_empty());
        assert_eq!(id.len(), 36); // UUID length
    }

    #[tokio::test]
    async fn test_history_empty() {
        let q = make_queue().await;
        let h = q.history(10).await.unwrap();
        assert!(h.is_empty());
    }

    #[tokio::test]
    async fn test_history_after_add() {
        let q = make_queue().await;
        q.add("task 1", "prompt 1").await.unwrap();
        q.add("task 2", "prompt 2").await.unwrap();
        let h = q.history(10).await.unwrap();
        assert_eq!(h.len(), 2);
    }

    #[tokio::test]
    async fn test_task_status_default_pending() {
        let q = make_queue().await;
        let id = q.add("pending task", "prompt").await.unwrap();
        let h = q.history(1).await.unwrap();
        assert_eq!(h[0].task_id, id);
        assert_eq!(h[0].status, TaskStatus::Pending);
        assert_eq!(h[0].priority, TaskPriority::Normal);
    }

    #[tokio::test]
    async fn test_set_status_to_failed() {
        let q = make_queue().await;
        let id = q.add("fail task", "prompt").await.unwrap();
        q.set_status(&id, TaskStatus::Failed, Some("boom"), None).unwrap();
        let h = q.history(1).await.unwrap();
        assert_eq!(h[0].status, TaskStatus::Failed);
        assert_eq!(h[0].result.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn test_priority_ordering() {
        let q = make_queue().await;
        q.add_with_options("low task", "p", TaskPriority::Low, None).await.unwrap();
        q.add_with_options("critical task", "p", TaskPriority::Critical, None).await.unwrap();
        q.add_with_options("normal task", "p", TaskPriority::Normal, None).await.unwrap();
        let h = q.history(10).await.unwrap();
        assert_eq!(h[0].priority, TaskPriority::Critical);
        assert_eq!(h[1].priority, TaskPriority::Normal);
        assert_eq!(h[2].priority, TaskPriority::Low);
    }

    #[tokio::test]
    async fn test_idempotency_key_dedup() {
        let q = make_queue().await;
        let id1 = q.add_with_options("task", "p", TaskPriority::Normal, Some("key-1")).await.unwrap();
        let id2 = q.add_with_options("task dup", "p2", TaskPriority::High, Some("key-1")).await.unwrap();
        // Same idempotency key should return same task_id
        assert_eq!(id1, id2);
        let h = q.history(10).await.unwrap();
        assert_eq!(h.len(), 1);
    }

    #[tokio::test]
    async fn test_idempotency_key_different() {
        let q = make_queue().await;
        let id1 = q.add_with_options("task a", "p", TaskPriority::Normal, Some("key-a")).await.unwrap();
        let id2 = q.add_with_options("task b", "p", TaskPriority::Normal, Some("key-b")).await.unwrap();
        assert_ne!(id1, id2);
        let h = q.history(10).await.unwrap();
        assert_eq!(h.len(), 2);
    }

    #[tokio::test]
    async fn test_next_pending() {
        let q = make_queue().await;
        let id = q.add_with_options("urgent", "p", TaskPriority::High, None).await.unwrap();
        q.add_with_options("normal", "p", TaskPriority::Normal, None).await.unwrap();
        let next = q.next_pending().await.unwrap().unwrap();
        assert_eq!(next.task_id, id);
        assert_eq!(next.priority, TaskPriority::High);
    }
}
