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
        let dir = crate::cli_config::phantom_data_dir()?;
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

    /// Shared connection handle, so sibling stores (e.g. the append-only
    /// [`super::events::EventStore`]) can live in the same SQLite DB without a
    /// second file. Returns a clone of the `Arc`; the underlying connection is
    /// unchanged.
    pub fn conn(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.conn)
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
        // Additive migration (DISPATCH-MESH-DURABILITY gap-a): the `tasks`
        // table historically had no column for an agent's output text, so the
        // durable async-dispatch job store had nowhere to persist a completed
        // job's result. Add a nullable `output TEXT` column. Guarded by a
        // `PRAGMA table_info` check so it is a no-op on already-migrated DBs
        // (SQLite `ADD COLUMN` errors if the column exists) and non-destructive
        // on old rows (existing rows simply get NULL output).
        if !column_exists(conn, "tasks", "output")? {
            conn.execute_batch("ALTER TABLE tasks ADD COLUMN output TEXT;")?;
        }
        // Additive migration: the governance `ExecutionContract.id` that parked a
        // task in `AwaitingApproval`, so a desktop/phone client can correlate the
        // task row with its pending approval card. Same idempotent `PRAGMA
        // table_info` guard as `output`; old rows get NULL approval_id.
        if !column_exists(conn, "tasks", "approval_id")? {
            conn.execute_batch("ALTER TABLE tasks ADD COLUMN approval_id TEXT;")?;
        }
        // S0 lane F1: additive, append-only `task_events` table living in the
        // same DB. This does NOT alter the `tasks` schema. Runs on the raw
        // connection here (no lock) so the sibling EventStore is ready before
        // the Mutex wrap.
        super::events::EventStore::init_schema(conn)?;
        Ok(())
    }

    pub async fn insert(&self, task: &TaskRecord) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO tasks (
                 task_id, workspace_id, session_id, agent_name, prompt, status,
                 created_at, started_at, finished_at, parent_task_id, assigned_node,
                 cost_usd, turns, error, output, approval_id, trace_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
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
                task.output,
                task.approval_id,
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

    /// Persist an agent's output text for a finished async dispatch job
    /// (DISPATCH-MESH-DURABILITY gap-a). Does not change `status` — callers
    /// transition separately (see `TaskQueue::record_result`).
    pub async fn set_output(&self, task_id: Uuid, output: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE tasks SET output = ?1 WHERE task_id = ?2",
            params![output, task_id.to_string()],
        )?;
        Ok(())
    }

    /// Correlate a pending governance approval onto a task by persisting the
    /// `ExecutionContract.id` (the per-action approval id the escalator matches a
    /// phone reply against) into `approval_id`. Called when a task enters
    /// `AwaitingApproval` because the governor raised an approval, so the `/tasks`
    /// rows let a desktop/phone client map the awaiting-approval task to its
    /// pending approval card. Does not change `status` — the caller transitions
    /// separately. Idempotent: a later call overwrites with the latest contract.
    pub async fn set_approval_id(&self, task_id: Uuid, approval_id: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE tasks SET approval_id = ?1 WHERE task_id = ?2",
            params![approval_id, task_id.to_string()],
        )?;
        Ok(())
    }
}

/// Return true if `table` has a column named `column`. Used to make the
/// `output` migration in [`TaskStore::init_schema`] idempotent. The table name
/// is a compile-time literal here (`tasks`), so the `PRAGMA` format is not an
/// injection surface.
fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

const SELECT_BASE: &str = "SELECT task_id, workspace_id, session_id, agent_name, prompt, status, \
     created_at, started_at, finished_at, parent_task_id, assigned_node, cost_usd, turns, error, \
     output, approval_id, trace_id FROM tasks";

const SELECT_FULL: &str = "SELECT task_id, workspace_id, session_id, agent_name, prompt, status, \
     created_at, started_at, finished_at, parent_task_id, assigned_node, cost_usd, turns, error, \
     output, approval_id, trace_id FROM tasks WHERE task_id = ?1";

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
    let output: Option<String> = row.get(14)?;
    let approval_id: Option<String> = row.get(15)?;
    let trace_id: String = row.get(16)?;

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
        output,
        approval_id,
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
    async fn output_column_round_trips() {
        // gap-a: a completed async dispatch job persists its output text.
        let dir = tempdir().unwrap();
        let store = TaskStore::open_at(dir.path().join("t.db")).unwrap();

        let t = mk_task("ws1");
        store.insert(&t).await.unwrap();
        // Fresh row has no output yet.
        assert_eq!(store.get(t.task_id).await.unwrap().unwrap().output, None);

        store.set_output(t.task_id, "the agent result").await.unwrap();
        let got = store.get(t.task_id).await.unwrap().unwrap();
        assert_eq!(got.output.as_deref(), Some("the agent result"));
    }

    #[tokio::test]
    async fn output_migration_is_idempotent_on_reopen() {
        // Re-opening the same DB file must not fail on the additive `output`
        // migration (PRAGMA table_info guard) and must preserve prior data.
        let dir = tempdir().unwrap();
        let db = dir.path().join("t.db");
        let t = {
            let store = TaskStore::open_at(db.clone()).unwrap();
            let t = mk_task("ws1");
            store.insert(&t).await.unwrap();
            store.set_output(t.task_id, "persisted").await.unwrap();
            t
        };
        // Reopen — init_schema runs again; ADD COLUMN must be skipped.
        let reopened = TaskStore::open_at(db).unwrap();
        let got = reopened.get(t.task_id).await.unwrap().unwrap();
        assert_eq!(got.output.as_deref(), Some("persisted"));
    }

    #[tokio::test]
    async fn approval_id_round_trips_and_flows_through_tasks_list_json() {
        // The REAL fix for the batch-3 fake-green: prove that an awaiting-approval
        // task's governance contract id (1) is persisted by the PRODUCTION setter
        // `set_approval_id`, (2) survives the store round-trip, and (3) appears in
        // the exact JSON shape `tasks_list` emits — `json!({ "tasks": <list> })` —
        // so a desktop client can correlate the task with its pending approval.
        let dir = tempdir().unwrap();
        let store = TaskStore::open_at(dir.path().join("t.db")).unwrap();

        let t = mk_task("ws1");
        store.insert(&t).await.unwrap();
        // Fresh row carries no approval correlation.
        assert_eq!(store.get(t.task_id).await.unwrap().unwrap().approval_id, None);

        // The contract id is what the governor mints (ExecutionContract::new) and
        // what the escalator correlates a phone reply against; set it via the REAL
        // setter — NOT by mutating the struct field directly in the test.
        let contract_id = "contract-7f3c-awaiting";
        store
            .set_approval_id(t.task_id, contract_id)
            .await
            .unwrap();

        // Serialize the way `tasks_list` does: a list of TaskRecord under "tasks".
        let listed = store.list(Some("ws1"), None, 100).await.unwrap();
        let body = serde_json::json!({ "tasks": listed });
        let approval_in_json = body["tasks"][0]["approval_id"].as_str();
        assert_eq!(
            approval_in_json,
            Some(contract_id),
            "the /tasks JSON must carry the approval_id set via the production setter"
        );

        // Control: a normal task (no setter call) has NO approval_id key in the
        // emitted JSON (skip_serializing_if), so a client makes no false match.
        let t2 = mk_task("ws2");
        store.insert(&t2).await.unwrap();
        let listed2 = store.list(Some("ws2"), None, 100).await.unwrap();
        let body2 = serde_json::json!({ "tasks": listed2 });
        assert!(
            body2["tasks"][0].get("approval_id").is_none(),
            "a task with no governance approval must emit no approval_id key"
        );
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
