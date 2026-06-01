use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use pm_types::{TaskRecord, TaskStatus};
use rusqlite::{params, Connection};
use tokio::sync::Mutex;
use uuid::Uuid;

/// SQLite-backed task record store.
#[derive(Clone)]
pub struct TaskStore {
    conn: Arc<Mutex<Connection>>,
}

impl TaskStore {
    /// Open (or create) the store at `~/.phantom-mesh/phantom.db`.
    pub fn open_default() -> Result<Self> {
        let dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".phantom-mesh");
        std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        Self::open_at(dir.join("phantom.db"))
    }

    pub fn open_at(db_path: PathBuf) -> Result<Self> {
        let conn = Connection::open(&db_path)
            .with_context(|| format!("open sqlite at {}", db_path.display()))?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Construct from an already-open connection (shared with other stores).
    pub fn from_conn(conn: Arc<Mutex<Connection>>) -> Result<Self> {
        {
            let guard = conn.blocking_lock();
            Self::init_schema(&guard)?;
        }
        Ok(Self { conn })
    }

    fn init_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tasks (
                 task_id          TEXT PRIMARY KEY,
                 workspace_id     TEXT NOT NULL,
                 session_id       TEXT NOT NULL,
                 agent_name       TEXT NOT NULL,
                 prompt           TEXT NOT NULL,
                 status           TEXT NOT NULL,
                 created_at       INTEGER NOT NULL,
                 started_at       INTEGER,
                 finished_at      INTEGER,
                 parent_task_id   TEXT,
                 assigned_node    TEXT,
                 cost_usd         REAL NOT NULL DEFAULT 0,
                 turns            INTEGER NOT NULL DEFAULT 0,
                 error            TEXT,
                 trace_id         TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS tasks_ws_status
                 ON tasks(workspace_id, status, created_at DESC);
             CREATE INDEX IF NOT EXISTS tasks_session_id
                 ON tasks(session_id);
             CREATE INDEX IF NOT EXISTS tasks_trace_id
                 ON tasks(trace_id);",
        )?;
        Ok(())
    }

    pub async fn insert(&self, task: &TaskRecord) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO tasks (
                 task_id, workspace_id, session_id, agent_name, prompt, status,
                 created_at, started_at, finished_at, parent_task_id, assigned_node,
                 cost_usd, turns, error, trace_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                task.task_id.to_string(),
                task.workspace_id,
                task.session_id,
                task.agent_name,
                task.prompt,
                task.status.as_str(),
                task.created_at,
                task.started_at,
                task.finished_at,
                task.parent_task_id.map(|u| u.to_string()),
                task.assigned_node.clone(),
                task.cost_usd,
                task.turns as i64,
                task.error,
                task.trace_id.to_string(),
            ],
        )?;
        Ok(())
    }

    pub async fn get(&self, task_id: Uuid) -> Result<Option<TaskRecord>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare_cached(SELECT_FULL)?;
        let mut rows = stmt.query(params![task_id.to_string()])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_task(row)?))
        } else {
            Ok(None)
        }
    }

    pub async fn list(
        &self,
        workspace_id: Option<&str>,
        status_filter: Option<TaskStatus>,
        limit: usize,
    ) -> Result<Vec<TaskRecord>> {
        let conn = self.conn.lock().await;
        let sql = match (workspace_id, status_filter.is_some()) {
            (Some(_), true) => format!(
                "{} WHERE workspace_id = ?1 AND status = ?2 ORDER BY created_at DESC LIMIT ?3",
                SELECT_BASE
            ),
            (Some(_), false) => format!(
                "{} WHERE workspace_id = ?1 ORDER BY created_at DESC LIMIT ?2",
                SELECT_BASE
            ),
            (None, true) => format!(
                "{} WHERE status = ?1 ORDER BY created_at DESC LIMIT ?2",
                SELECT_BASE
            ),
            (None, false) => format!("{} ORDER BY created_at DESC LIMIT ?1", SELECT_BASE),
        };
        let mut stmt = conn.prepare(&sql)?;
        let limit_i = limit as i64;
        let rows: Vec<TaskRecord> = match (workspace_id, status_filter) {
            (Some(ws), Some(s)) => stmt
                .query_map(params![ws, s.as_str(), limit_i], row_to_task)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            (Some(ws), None) => stmt
                .query_map(params![ws, limit_i], row_to_task)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            (None, Some(s)) => stmt
                .query_map(params![s.as_str(), limit_i], row_to_task)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            (None, None) => stmt
                .query_map(params![limit_i], row_to_task)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        };
        Ok(rows)
    }

    pub async fn update_status(
        &self,
        task_id: Uuid,
        status: TaskStatus,
        error: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().await;
        let now = now_millis();
        match status {
            TaskStatus::Running => {
                conn.execute(
                    "UPDATE tasks SET status = ?1, started_at = COALESCE(started_at, ?2) WHERE task_id = ?3",
                    params![status.as_str(), now, task_id.to_string()],
                )?;
            }
            s if s.is_terminal() => {
                conn.execute(
                    "UPDATE tasks SET status = ?1, finished_at = ?2, error = ?3 WHERE task_id = ?4",
                    params![status.as_str(), now, error, task_id.to_string()],
                )?;
            }
            _ => {
                conn.execute(
                    "UPDATE tasks SET status = ?1 WHERE task_id = ?2",
                    params![status.as_str(), task_id.to_string()],
                )?;
            }
        }
        Ok(())
    }

    pub async fn record_turn(&self, task_id: Uuid, cost_delta: f64) -> Result<()> {
        self.record_progress(task_id, 1, cost_delta).await
    }

    /// Bulk increment `turns` by `turns_delta` and `cost_usd` by `cost_delta` in
    /// a single UPDATE — used when the agent loop reports aggregate progress
    /// after returning.
    pub async fn record_progress(
        &self,
        task_id: Uuid,
        turns_delta: u32,
        cost_delta: f64,
    ) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE tasks SET turns = turns + ?1, cost_usd = cost_usd + ?2 WHERE task_id = ?3",
            params![turns_delta as i64, cost_delta, task_id.to_string()],
        )?;
        Ok(())
    }
}

const SELECT_BASE: &str = "SELECT task_id, workspace_id, session_id, agent_name, prompt, status, \
     created_at, started_at, finished_at, parent_task_id, assigned_node, cost_usd, turns, error, \
     trace_id FROM tasks";

const SELECT_FULL: &str = "SELECT task_id, workspace_id, session_id, agent_name, prompt, status, \
     created_at, started_at, finished_at, parent_task_id, assigned_node, cost_usd, turns, error, \
     trace_id FROM tasks WHERE task_id = ?1";

fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRecord> {
    let task_id: String = row.get(0)?;
    let workspace_id: String = row.get(1)?;
    let session_id: String = row.get(2)?;
    let agent_name: String = row.get(3)?;
    let prompt: String = row.get(4)?;
    let status_str: String = row.get(5)?;
    let created_at: i64 = row.get(6)?;
    let started_at: Option<i64> = row.get(7)?;
    let finished_at: Option<i64> = row.get(8)?;
    let parent_task_id: Option<String> = row.get(9)?;
    let assigned_node: Option<String> = row.get(10)?;
    let cost_usd: f64 = row.get(11)?;
    let turns: i64 = row.get(12)?;
    let error: Option<String> = row.get(13)?;
    let trace_id: String = row.get(14)?;

    let status = TaskStatus::from_str(&status_str).unwrap_or_else(|| {
        tracing::warn!(status_str = %status_str, task_id = %task_id, "TaskStatus::from_str unknown variant — coerced to Failed");
        TaskStatus::Failed
    });
    let task_id = Uuid::parse_str(&task_id).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let trace_id = Uuid::parse_str(&trace_id).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let parent_task_id = match parent_task_id {
        Some(s) => Some(Uuid::parse_str(&s).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?),
        None => None,
    };
    // NodeId is a type alias for String; already in the right shape.

    Ok(TaskRecord {
        task_id,
        workspace_id,
        session_id,
        agent_name,
        prompt,
        status,
        created_at,
        started_at,
        finished_at,
        parent_task_id,
        assigned_node,
        cost_usd,
        turns: turns as u32,
        error,
        trace_id,
    })
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[allow(dead_code)]
pub(crate) fn _touch_path(p: &Path) -> &Path {
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn mk_task(ws: &str) -> TaskRecord {
        TaskRecord::new(ws.into(), "master".into(), "hello".into())
    }

    #[tokio::test]
    async fn insert_and_get() {
        let dir = tempdir().unwrap();
        let store = TaskStore::open_at(dir.path().join("t.db")).unwrap();

        let t = mk_task("ws1");
        store.insert(&t).await.unwrap();

        let got = store.get(t.task_id).await.unwrap().unwrap();
        assert_eq!(got.task_id, t.task_id);
        assert_eq!(got.status, TaskStatus::Pending);
    }

    #[tokio::test]
    async fn status_transitions_persist_timestamps() {
        let dir = tempdir().unwrap();
        let store = TaskStore::open_at(dir.path().join("t.db")).unwrap();

        let t = mk_task("ws1");
        store.insert(&t).await.unwrap();

        store
            .update_status(t.task_id, TaskStatus::Running, None)
            .await
            .unwrap();
        let got = store.get(t.task_id).await.unwrap().unwrap();
        assert_eq!(got.status, TaskStatus::Running);
        assert!(got.started_at.is_some());
        assert!(got.finished_at.is_none());

        store
            .update_status(t.task_id, TaskStatus::Completed, None)
            .await
            .unwrap();
        let got = store.get(t.task_id).await.unwrap().unwrap();
        assert_eq!(got.status, TaskStatus::Completed);
        assert!(got.finished_at.is_some());
    }

    #[tokio::test]
    async fn list_filters_by_workspace_and_status() {
        let dir = tempdir().unwrap();
        let store = TaskStore::open_at(dir.path().join("t.db")).unwrap();

        let t1 = mk_task("ws1");
        let t2 = mk_task("ws1");
        let t3 = mk_task("ws2");
        store.insert(&t1).await.unwrap();
        store.insert(&t2).await.unwrap();
        store.insert(&t3).await.unwrap();
        store
            .update_status(t2.task_id, TaskStatus::Failed, Some("boom"))
            .await
            .unwrap();

        let ws1_all = store.list(Some("ws1"), None, 100).await.unwrap();
        assert_eq!(ws1_all.len(), 2);

        let ws1_failed = store
            .list(Some("ws1"), Some(TaskStatus::Failed), 100)
            .await
            .unwrap();
        assert_eq!(ws1_failed.len(), 1);
        assert_eq!(ws1_failed[0].error.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn record_turn_accumulates() {
        let dir = tempdir().unwrap();
        let store = TaskStore::open_at(dir.path().join("t.db")).unwrap();

        let t = mk_task("ws1");
        store.insert(&t).await.unwrap();
        store.record_turn(t.task_id, 0.01).await.unwrap();
        store.record_turn(t.task_id, 0.02).await.unwrap();

        let got = store.get(t.task_id).await.unwrap().unwrap();
        assert_eq!(got.turns, 2);
        assert!((got.cost_usd - 0.03).abs() < 1e-9);
    }

    #[tokio::test]
    async fn record_progress_bulk() {
        let dir = tempdir().unwrap();
        let store = TaskStore::open_at(dir.path().join("t.db")).unwrap();

        let t = mk_task("ws1");
        store.insert(&t).await.unwrap();
        // Simulate an agent loop reporting back 7 turns and $0.042 cost.
        store.record_progress(t.task_id, 7, 0.042).await.unwrap();

        let got = store.get(t.task_id).await.unwrap().unwrap();
        assert_eq!(got.turns, 7);
        assert!((got.cost_usd - 0.042).abs() < 1e-9);
    }
}
