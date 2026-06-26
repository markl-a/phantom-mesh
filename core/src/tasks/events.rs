//! Append-only per-task event store (S0 lane F1 — the FlightRecorder spine).
//!
//! Every TaskQueue lifecycle transition is durably appended here, keyed by
//! `task_id`, so a task's history can later be replayed/exported in order.
//! This is the durable spine that replaces the stubbed `// TODO (P6): emit
//! DomainEvent` seams in [`super::state::TaskQueue`].
//!
//! ## Append-only invariant
//! Rows are only ever INSERTed — there is no UPDATE or DELETE path. Ordering
//! is established by an AUTOINCREMENT `seq` rowid (NOT by `timestamp_ms`), so
//! events keep a stable, monotonic order even when two events land in the same
//! millisecond. `events_for` returns rows ordered by `seq`.
//!
//! ## Scope (v1) and S1 follow-up
//! This lane records only TaskQueue *lifecycle* transitions (created / started
//! / completed / failed / cancelled / interrupted). It deliberately does NOT
//! capture per-step agent activity. The S1 follow-up is to emit
//! `tool_start` / `tool_done` events around each tool invocation and an
//! `approval` event for each approval-gate decision, wired into the live agent
//! loop / streaming path. `TaskEventKind` is `#[non_exhaustive]` and the table
//! schema is forward-compatible (free-form `kind` TEXT + nullable `detail`),
//! so adding those kinds in S1 needs no migration.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Kind of a recorded task event. Maps 1:1 onto the TaskQueue lifecycle
/// transitions for v1. `#[non_exhaustive]` because S1 will add tool/approval
/// kinds (see module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskEventKind {
    Created,
    Started,
    Completed,
    Failed,
    Cancelled,
    /// Crash-recovery sweep marked an in-flight task as interrupted.
    Interrupted,
    /// A high-risk [`ExecutionContract`](crate::execution_contract::ExecutionContract)
    /// was raised and is awaiting an operator decision (detail = contract JSON).
    ApprovalRequested,
    /// The operator approved a pending contract (detail = decision JSON).
    Approved,
    /// The operator denied a pending contract (detail = decision JSON).
    Denied,
}

impl TaskEventKind {
    /// Stable on-disk discriminator. Keep these strings stable — they are the
    /// wire/replay format. Adding variants is fine; renaming is a migration.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
            Self::ApprovalRequested => "approval_requested",
            Self::Approved => "approved",
            Self::Denied => "denied",
        }
    }

    // Inherent `from_str -> Option<Self>` mirrors `TaskStatus::from_str`; not
    // the fallible `std::str::FromStr` trait (parsing an unknown kind is a
    // None, not an error we propagate).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "created" => Some(Self::Created),
            "started" => Some(Self::Started),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "interrupted" => Some(Self::Interrupted),
            "approval_requested" => Some(Self::ApprovalRequested),
            "approved" => Some(Self::Approved),
            "denied" => Some(Self::Denied),
            _ => None,
        }
    }
}

/// One appended event in a task's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEvent {
    /// Monotonic append sequence (table rowid). Establishes total order.
    pub seq: i64,
    pub task_id: Uuid,
    pub kind: TaskEventKind,
    /// Wall-clock append time, milliseconds since the Unix epoch. Monotonic
    /// within a task in practice but ordering is authoritative via `seq`.
    pub timestamp_ms: i64,
    /// Optional human/structured detail (e.g. a failure reason). Free-form.
    pub detail: Option<String>,
}

/// Append-only event store, sharing the same SQLite connection as
/// [`super::store::TaskStore`] so task rows and their events live in one DB.
#[derive(Clone)]
pub struct EventStore {
    conn: Arc<Mutex<Connection>>,
}

impl EventStore {
    /// Construct over a shared connection (typically `TaskStore`'s). The
    /// `task_events` table is created by [`EventStore::init_schema`], which the
    /// task store runs against the raw connection at open time (before the
    /// `Mutex` wrap), so this constructor performs no locking and is safe to
    /// call from inside an async runtime (e.g. `TaskQueue::new`).
    pub fn from_conn(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Create the append-only `task_events` table + ordering index. This is an
    /// *additive* table only — it never touches the `tasks` schema. Idempotent
    /// (`IF NOT EXISTS`), so safe to re-run on reopen. Invoked from
    /// `TaskStore::init_schema` on the raw connection.
    pub(crate) fn init_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS task_events (
                 seq          INTEGER PRIMARY KEY AUTOINCREMENT,
                 task_id      TEXT NOT NULL,
                 kind         TEXT NOT NULL,
                 timestamp_ms INTEGER NOT NULL,
                 detail       TEXT
             );
             CREATE INDEX IF NOT EXISTS task_events_task_seq
                 ON task_events(task_id, seq);",
        )
        .context("init task_events schema")?;
        Ok(())
    }

    /// Append a single event for `task_id`. Append-only: this is the only write
    /// path and it never updates or deletes. Returns the assigned `seq`.
    pub async fn append(
        &self,
        task_id: Uuid,
        kind: TaskEventKind,
        detail: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().await;
        let ts = now_millis();
        conn.execute(
            "INSERT INTO task_events (task_id, kind, timestamp_ms, detail)
             VALUES (?1, ?2, ?3, ?4)",
            params![task_id.to_string(), kind.as_str(), ts, detail],
        )
        .context("append task_event")?;
        Ok(conn.last_insert_rowid())
    }

    /// Return all events for `task_id` ordered by append sequence (`seq`),
    /// i.e. in the exact order they were recorded.
    pub async fn events_for(&self, task_id: Uuid) -> Result<Vec<TaskEvent>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare_cached(
            "SELECT seq, task_id, kind, timestamp_ms, detail
             FROM task_events WHERE task_id = ?1 ORDER BY seq ASC",
        )?;
        let rows = stmt
            .query_map(params![task_id.to_string()], row_to_event)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        // Forward-compatible: rows whose `kind` this binary doesn't recognise
        // (an event written by a newer binary) are skipped, not fatal — one
        // unknown row must never break a whole task's replay/approval history.
        Ok(rows.into_iter().flatten().collect())
    }
}

/// Returns `Ok(None)` for a row whose `kind` string is unrecognised (forward-
/// compat skip), `Ok(Some(event))` for a known kind, and `Err` only on a real
/// decode failure (e.g. a malformed task_id).
fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<TaskEvent>> {
    let seq: i64 = row.get(0)?;
    let task_id: String = row.get(1)?;
    let kind_str: String = row.get(2)?;
    let timestamp_ms: i64 = row.get(3)?;
    let detail: Option<String> = row.get(4)?;

    let task_id = Uuid::parse_str(&task_id).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let kind = match TaskEventKind::from_str(&kind_str) {
        Some(k) => k,
        None => {
            tracing::debug!("skipping task_event with unknown kind: {kind_str}");
            return Ok(None);
        }
    };

    Ok(Some(TaskEvent {
        seq,
        task_id,
        kind,
        timestamp_ms,
        detail,
    }))
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
    use rusqlite::Connection;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn mk_store() -> EventStore {
        let raw = Connection::open_in_memory().unwrap();
        EventStore::init_schema(&raw).unwrap();
        EventStore::from_conn(Arc::new(Mutex::new(raw)))
    }

    #[tokio::test]
    async fn append_then_events_for_in_order() {
        let store = mk_store();
        let id = Uuid::new_v4();
        store.append(id, TaskEventKind::Created, None).await.unwrap();
        store.append(id, TaskEventKind::Started, None).await.unwrap();
        store
            .append(id, TaskEventKind::Completed, Some("ok"))
            .await
            .unwrap();

        let evs = store.events_for(id).await.unwrap();
        let kinds: Vec<_> = evs.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TaskEventKind::Created,
                TaskEventKind::Started,
                TaskEventKind::Completed
            ]
        );
        assert_eq!(evs[2].detail.as_deref(), Some("ok"));
        // seq is strictly increasing.
        assert!(evs[0].seq < evs[1].seq && evs[1].seq < evs[2].seq);
        // timestamps are non-decreasing (monotonic wall clock).
        assert!(evs[0].timestamp_ms <= evs[1].timestamp_ms);
        assert!(evs[1].timestamp_ms <= evs[2].timestamp_ms);
    }

    #[tokio::test]
    async fn events_isolated_per_task() {
        let store = mk_store();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        store.append(a, TaskEventKind::Created, None).await.unwrap();
        store.append(b, TaskEventKind::Created, None).await.unwrap();
        store.append(a, TaskEventKind::Cancelled, None).await.unwrap();

        assert_eq!(store.events_for(a).await.unwrap().len(), 2);
        assert_eq!(store.events_for(b).await.unwrap().len(), 1);
        assert_eq!(store.events_for(Uuid::new_v4()).await.unwrap().len(), 0);
    }
}
