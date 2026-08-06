//! Durable SQLite work-queue with atomic CAS claim. The single coordination point.
use crate::fleet::types::{BacklogTask, TaskState};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct QueueRow {
    pub task_id: String,
    pub repo: String,
    pub slug: String,
    pub state: TaskState,
}

pub struct FleetQueue {
    conn: Arc<Mutex<Connection>>,
}

impl FleetQueue {
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open_default() -> Result<Self> {
        let dir = crate::cli_config::spectyn_data_dir()?;
        std::fs::create_dir_all(&dir)?;
        let conn = Connection::open(dir.join("fleet.db"))?;
        Self::init(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn init(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS fleet_tasks (
                task_id       TEXT PRIMARY KEY,
                repo          TEXT NOT NULL,
                slug          TEXT NOT NULL,
                component     TEXT NOT NULL,
                acceptance    TEXT NOT NULL DEFAULT '',
                caps          TEXT NOT NULL DEFAULT '',
                max_files     INTEGER NOT NULL DEFAULT 0,
                state         TEXT NOT NULL DEFAULT 'pending',
                claimed_by    TEXT,
                lease_until   INTEGER,
                changes_round INTEGER NOT NULL DEFAULT 0,
                park_reason   TEXT
            );",
        )?;
        // Forward-compat: add columns if an older fleet.db predates them.
        // A duplicate-column error is expected on an up-to-date schema and is ignored.
        for stmt in [
            "ALTER TABLE fleet_tasks ADD COLUMN acceptance TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE fleet_tasks ADD COLUMN caps TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE fleet_tasks ADD COLUMN max_files INTEGER NOT NULL DEFAULT 0",
        ] {
            let _ = conn.execute(stmt, []);
        }
        Ok(())
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    /// Insert a new task, OR refresh the backlog metadata of an existing **pending** task
    /// (so an edited backlog file reaches a not-yet-started task, and a row migrated from an
    /// older schema gets backfilled). In-flight rows (claimed/executing/gating/landing) and
    /// terminal rows are left untouched — their state, claim ownership, and metadata are frozen.
    /// Returns true iff the task was newly inserted (false if it already existed).
    pub async fn upsert(&self, t: &BacklogTask) -> Result<bool> {
        use rusqlite::OptionalExtension;
        let conn = self.conn.lock().await;
        // caps stored newline-joined (a cap tag never contains a newline).
        let caps = t.caps.join("\n");
        let existed = conn
            .query_row(
                "SELECT 1 FROM fleet_tasks WHERE task_id=?1",
                params![t.task_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        conn.execute(
            "INSERT INTO fleet_tasks
               (task_id, repo, slug, component, acceptance, caps, max_files, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending')
             ON CONFLICT(task_id) DO UPDATE SET
                 repo=excluded.repo,
                 slug=excluded.slug,
                 component=excluded.component,
                 acceptance=excluded.acceptance,
                 caps=excluded.caps,
                 max_files=excluded.max_files
               WHERE fleet_tasks.state='pending'",
            params![
                t.task_id,
                t.repo,
                t.slug,
                t.component,
                t.acceptance,
                caps,
                t.max_files
            ],
        )?;
        Ok(!existed)
    }

    /// Fetch the full task by id (None if absent). Reconstructs `caps` from the
    /// newline-joined column so the live executor receives a COMPLETE `BacklogTask`.
    pub async fn get_task(&self, task_id: &str) -> Result<Option<BacklogTask>> {
        use rusqlite::OptionalExtension;
        let conn = self.conn.lock().await;
        let row = conn
            .query_row(
                "SELECT task_id, repo, slug, component, acceptance, caps, max_files
                 FROM fleet_tasks WHERE task_id=?1",
                params![task_id],
                |r| {
                    let caps_s: String = r.get(5)?;
                    Ok(BacklogTask {
                        task_id: r.get(0)?,
                        repo: r.get(1)?,
                        slug: r.get(2)?,
                        component: r.get(3)?,
                        acceptance: r.get(4)?,
                        caps: if caps_s.is_empty() {
                            Vec::new()
                        } else {
                            caps_s.split('\n').map(|s| s.to_string()).collect()
                        },
                        max_files: r.get(6)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Atomic claim: succeeds only if the row is still `pending`. Returns true on win.
    pub async fn claim(&self, task_id: &str, worker: &str, lease_secs: i64) -> Result<bool> {
        let conn = self.conn.lock().await;
        let n = conn.execute(
            "UPDATE fleet_tasks SET state='claimed', claimed_by=?2, lease_until=?3
             WHERE task_id=?1 AND state='pending'",
            params![task_id, worker, Self::now() + lease_secs],
        )?;
        Ok(n == 1)
    }

    /// Advance state only if `worker` still owns the claim. Returns true iff owned & updated.
    pub async fn set_state(&self, task_id: &str, worker: &str, state: TaskState) -> Result<bool> {
        let conn = self.conn.lock().await;
        let n = conn.execute(
            "UPDATE fleet_tasks SET state=?3 WHERE task_id=?1 AND claimed_by=?2",
            params![task_id, worker, state.as_str()],
        )?;
        Ok(n == 1)
    }

    pub async fn complete(&self, task_id: &str, worker: &str, terminal: TaskState) -> Result<bool> {
        self.set_state(task_id, worker, terminal).await
    }

    pub async fn release(&self, task_id: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE fleet_tasks SET state='pending', claimed_by=NULL, lease_until=NULL
             WHERE task_id=?1",
            params![task_id],
        )?;
        Ok(())
    }

    pub async fn park(&self, task_id: &str, worker: &str, reason: &str) -> Result<bool> {
        let conn = self.conn.lock().await;
        let n = conn.execute(
            "UPDATE fleet_tasks SET state='parked', park_reason=?3 WHERE task_id=?1 AND claimed_by=?2",
            params![task_id, worker, reason],
        )?;
        Ok(n == 1)
    }

    /// Returns Some(new_round) if owned, None if the worker lost ownership.
    pub async fn bump_changes_round(&self, task_id: &str, worker: &str) -> Result<Option<u32>> {
        let conn = self.conn.lock().await;
        let n = conn.execute(
            "UPDATE fleet_tasks SET changes_round = changes_round + 1 WHERE task_id=?1 AND claimed_by=?2",
            params![task_id, worker],
        )?;
        if n != 1 {
            return Ok(None);
        }
        let r: u32 = conn.query_row(
            "SELECT changes_round FROM fleet_tasks WHERE task_id=?1",
            params![task_id],
            |row| row.get(0),
        )?;
        Ok(Some(r))
    }

    /// Return expired-lease rows in ANY non-terminal, worker-held state to `pending`.
    /// Returns the count reaped.
    ///
    /// Broadened beyond `claimed`: `process_one` transitions to `executing` almost
    /// immediately, so `claimed` is a vanishing window — a worker that crashes during
    /// `executing`/`gating`/`landing` (where the vendor CLI burns minutes) would otherwise
    /// strand its task forever (never reaped, never re-picked). This is safe because the
    /// `claimed_by` fence (see `set_state`/`park`/`complete`) makes a revived stale worker's
    /// mutations no-op, and the single-worker driver reaps only between tasks.
    ///
    /// NOTE: callers must set the claim `lease_secs` to exceed the max task duration so a
    /// still-live in-flight task is not reaped by a concurrent worker (relevant only in the
    /// future multi-worker path; the single-worker driver never reaps a live task).
    pub async fn reap_expired(&self) -> Result<usize> {
        let conn = self.conn.lock().await;
        let n = conn.execute(
            "UPDATE fleet_tasks SET state='pending', claimed_by=NULL, lease_until=NULL
             WHERE state IN ('claimed','executing','gating','landing')
               AND lease_until IS NOT NULL AND lease_until <= ?1",
            params![Self::now()],
        )?;
        Ok(n)
    }

    pub async fn list(&self, state: TaskState) -> Result<Vec<QueueRow>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT task_id, repo, slug, state FROM fleet_tasks WHERE state=?1 ORDER BY slug",
        )?;
        let rows = stmt
            .query_map(params![state.as_str()], |row| {
                Ok(QueueRow {
                    task_id: row.get(0)?,
                    repo: row.get(1)?,
                    slug: row.get(2)?,
                    state: TaskState::from_str(&row.get::<_, String>(3)?)
                        .unwrap_or(TaskState::Pending),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Snapshot of every non-terminal row (for the scheduler).
    pub async fn active_snapshot(&self) -> Result<Vec<QueueRow>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT task_id, repo, slug, state FROM fleet_tasks
             WHERE state NOT IN ('landed','staged','parked') ORDER BY slug",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(QueueRow {
                    task_id: row.get(0)?,
                    repo: row.get(1)?,
                    slug: row.get(2)?,
                    state: TaskState::from_str(&row.get::<_, String>(3)?)
                        .unwrap_or(TaskState::Pending),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::types::BacklogTask;

    fn task(repo: &str, slug: &str) -> BacklogTask {
        BacklogTask {
            task_id: crate::fleet::backlog::task_id(repo, slug),
            repo: repo.into(),
            slug: slug.into(),
            component: "c".into(),
            acceptance: "a".into(),
            caps: vec![],
            max_files: 3,
        }
    }

    #[tokio::test]
    async fn upsert_is_idempotent() {
        let q = FleetQueue::open_in_memory().unwrap();
        let first = q.upsert(&task("spectyn-quant", "x")).await.unwrap();
        let second = q.upsert(&task("spectyn-quant", "x")).await.unwrap();
        assert!(first, "first upsert inserts a new row");
        assert!(!second, "second upsert is a no-op insert (already present)");
        assert_eq!(q.list(TaskState::Pending).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn upsert_refreshes_pending_metadata_but_freezes_inflight() {
        let q = FleetQueue::open_in_memory().unwrap();
        let mut t = task("spectyn-quant", "x"); // component "c", max_files 3
        assert!(q.upsert(&t).await.unwrap());

        // Re-ingest with edited fields while the task is still PENDING -> metadata refreshed.
        t.component = "updated".into();
        t.max_files = 9;
        assert!(
            !q.upsert(&t).await.unwrap(),
            "re-ingest of an existing task is not a new insert"
        );
        let got = q.get_task(&t.task_id).await.unwrap().unwrap();
        assert_eq!(got.component, "updated", "pending metadata is refreshed");
        assert_eq!(got.max_files, 9);

        // Claim it, then re-ingest with new fields -> in-flight metadata must be FROZEN.
        assert!(q.claim(&t.task_id, "w1", 60).await.unwrap());
        t.component = "should-not-apply".into();
        q.upsert(&t).await.unwrap();
        let got2 = q.get_task(&t.task_id).await.unwrap().unwrap();
        assert_eq!(
            got2.component, "updated",
            "in-flight task metadata is frozen (claim/state preserved)"
        );
    }

    #[tokio::test]
    async fn upsert_then_get_task_roundtrips_all_fields() {
        let q = FleetQueue::open_in_memory().unwrap();
        let t = BacklogTask {
            task_id: crate::fleet::backlog::task_id("spectyn-quant", "sma"),
            repo: "spectyn-quant".into(),
            slug: "sma".into(),
            component: "add SMA".into(),
            acceptance: "sma() returns mean".into(),
            caps: vec!["quant".into(), "math".into()],
            max_files: 4,
        };
        q.upsert(&t).await.unwrap();
        let got = q.get_task(&t.task_id).await.unwrap().expect("task present");
        assert_eq!(got, t, "all fields round-trip through SQLite");
        assert!(q.get_task("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn claim_is_atomic_exactly_one_winner() {
        let q = FleetQueue::open_in_memory().unwrap();
        q.upsert(&task("spectyn-quant", "x")).await.unwrap();
        let id = task("spectyn-quant", "x").task_id;
        let first = q.claim(&id, "w1", 60).await.unwrap();
        let second = q.claim(&id, "w2", 60).await.unwrap();
        assert!(first, "first claim wins");
        assert!(!second, "second claim must fail (already claimed)");
    }

    #[tokio::test]
    async fn reap_expired_returns_lease_to_pending() {
        let q = FleetQueue::open_in_memory().unwrap();
        q.upsert(&task("spectyn-quant", "x")).await.unwrap();
        let id = task("spectyn-quant", "x").task_id;
        q.claim(&id, "w1", 0).await.unwrap(); // lease_secs=0 -> already expired
        let reaped = q.reap_expired().await.unwrap();
        assert_eq!(reaped, 1);
        assert_eq!(q.list(TaskState::Pending).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn reap_recovers_a_crashed_in_flight_task() {
        let q = FleetQueue::open_in_memory().unwrap();
        q.upsert(&task("spectyn-quant", "x")).await.unwrap();
        let id = task("spectyn-quant", "x").task_id;
        q.claim(&id, "w1", 0).await.unwrap(); // lease = now (already expired)
                                              // worker advances into the long-running window, then "crashes":
        assert!(q.set_state(&id, "w1", TaskState::Executing).await.unwrap());
        let reaped = q.reap_expired().await.unwrap();
        assert_eq!(
            reaped, 1,
            "an executing task with expired lease must be reaped"
        );
        assert_eq!(q.list(TaskState::Pending).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn park_and_complete_set_terminal_states() {
        let q = FleetQueue::open_in_memory().unwrap();
        q.upsert(&task("a", "1")).await.unwrap();
        q.upsert(&task("b", "2")).await.unwrap();
        let a = crate::fleet::backlog::task_id("a", "1");
        let b = crate::fleet::backlog::task_id("b", "2");
        // The fence requires the worker to own the claim before mutating.
        q.claim(&a, "w1", 60).await.unwrap();
        q.claim(&b, "w1", 60).await.unwrap();
        assert!(q.park(&a, "w1", "max changes").await.unwrap());
        assert!(q.complete(&b, "w1", TaskState::Landed).await.unwrap());
        assert_eq!(q.list(TaskState::Parked).await.unwrap().len(), 1);
        assert_eq!(q.list(TaskState::Landed).await.unwrap().len(), 1);
        // A non-owner worker cannot mutate the fenced row.
        assert!(
            !q.park(&a, "wrong-worker", "x").await.unwrap(),
            "non-owner must not be able to park the row"
        );
    }

    #[tokio::test]
    async fn stale_worker_cannot_mutate_after_reclaim() {
        let q = FleetQueue::open_in_memory().unwrap();
        q.upsert(&task("spectyn-quant", "x")).await.unwrap();
        let id = task("spectyn-quant", "x").task_id;
        q.claim(&id, "w1", 0).await.unwrap(); // w1 claims, lease already expired (0s)
        q.reap_expired().await.unwrap(); // back to pending
        q.claim(&id, "w2", 60).await.unwrap(); // w2 re-claims
                                               // stale w1 must NOT be able to advance the row now:
        assert!(
            !q.set_state(&id, "w1", TaskState::Executing).await.unwrap(),
            "stale w1 fenced out"
        );
        assert!(
            q.set_state(&id, "w2", TaskState::Executing).await.unwrap(),
            "owner w2 can advance"
        );
    }
}
