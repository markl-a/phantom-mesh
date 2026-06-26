use anyhow::{anyhow, Result};
use pm_types::{TaskRecord, TaskStatus};
use uuid::Uuid;

use super::events::{EventStore, TaskEvent, TaskEventKind};
use super::store::TaskStore;

/// Facade over the task store that enforces legal state transitions and
/// appends a durable lifecycle event to the append-only [`EventStore`] for
/// each transition (S0 lane F1 — the FlightRecorder spine).
///
/// Scope: v1 records only TaskQueue *lifecycle* transitions. The S1 follow-up
/// is per-step `tool_start`/`tool_done` + approval-gate events wired into the
/// live agent loop — see `events.rs` module docs.
#[derive(Clone)]
pub struct TaskQueue {
    store: TaskStore,
    events: EventStore,
}

impl TaskQueue {
    /// Build a queue whose lifecycle events are appended to an [`EventStore`]
    /// backed by the *same* SQLite connection as `store`. The `task_events`
    /// table is created by `TaskStore::init_schema`, so this performs no DB I/O
    /// or locking and is safe to call from an async runtime.
    pub fn new(store: TaskStore) -> Self {
        let events = EventStore::from_conn(store.conn());
        Self { store, events }
    }

    pub fn store(&self) -> &TaskStore {
        &self.store
    }

    /// Access the append-only event store (read side: `events_for`).
    pub fn events(&self) -> &EventStore {
        &self.events
    }

    /// All lifecycle events recorded for `task_id`, in append order.
    pub async fn events_for(&self, task_id: Uuid) -> Result<Vec<TaskEvent>> {
        self.events.events_for(task_id).await
    }

    /// Runner enforcement point (deny-until-approved): consult the gate for a
    /// contracted high-risk action on `task_id`. Durably raises the contract the
    /// first time it is seen and returns the decision — `Allow` (proceed),
    /// `Deny` (refuse), or `NeedsApproval` (the caller parks the task in
    /// [`TaskStatus::AwaitingApproval`] and routes an approval card). Low-risk
    /// contracts are auto-allowed without touching the ledger.
    pub async fn enforce_contract(
        &self,
        task_id: Uuid,
        contract: &crate::execution_contract::ExecutionContract,
        now_ms: i64,
    ) -> Result<crate::tasks::approvals::GateOutcome> {
        crate::tasks::approvals::enforce(&self.events, task_id, contract, now_ms).await
    }

    /// Correlate the governance approval that parked `task_id` in
    /// [`TaskStatus::AwaitingApproval`] onto its task row, by persisting the
    /// pending `ExecutionContract.id`. A client listing `/tasks` then maps the
    /// awaiting-approval task to its pending approval card via this id (the same
    /// id the escalator matches an operator's phone reply against), instead of
    /// falling back to `task_id`. Call alongside the transition to
    /// `AwaitingApproval`; it does not itself change status.
    pub async fn set_approval_id(&self, task_id: Uuid, approval_id: &str) -> Result<()> {
        self.store.set_approval_id(task_id, approval_id).await
    }

    /// Contracts on `task_id` still awaiting an operator decision.
    pub async fn pending_approvals(
        &self,
        task_id: Uuid,
    ) -> Result<Vec<crate::execution_contract::ExecutionContract>> {
        crate::tasks::approvals::pending_for(&self.events, task_id).await
    }

    /// Load this task's durably-approved contracts into the sync `contract_gate`
    /// snapshot — call BEFORE running an agent loop for `task_id` so already-
    /// approved actions pass the gate without re-blocking. Returns the count.
    pub async fn load_gate_snapshot(&self, task_id: Uuid) -> Result<usize> {
        crate::contract_gate::load_approved(&self.events, task_id).await
    }

    /// Durably raise every contract the gate blocked during the run — call AFTER
    /// the agent loop so `phantom task approvals` shows what needs the operator.
    /// Returns the number newly raised.
    pub async fn flush_gate_pending(&self, task_id: Uuid) -> Result<usize> {
        crate::contract_gate::flush_pending(&self.events, task_id).await
    }

    /// Map a (terminal or running) status to its lifecycle event kind. Returns
    /// `None` for transitions we do not record as a distinct event (e.g.
    /// Pending / AwaitingApproval, which are covered by Created).
    fn kind_for_status(status: TaskStatus) -> Option<TaskEventKind> {
        match status {
            TaskStatus::Running => Some(TaskEventKind::Started),
            TaskStatus::Completed => Some(TaskEventKind::Completed),
            TaskStatus::Failed => Some(TaskEventKind::Failed),
            TaskStatus::Cancelled => Some(TaskEventKind::Cancelled),
            TaskStatus::Pending | TaskStatus::AwaitingApproval => None,
        }
    }

    /// Create a new Pending task and persist it.
    pub async fn create(
        &self,
        workspace_id: &str,
        agent_name: &str,
        prompt: &str,
    ) -> Result<TaskRecord> {
        let task = TaskRecord::new(
            workspace_id.to_string(),
            agent_name.to_string(),
            prompt.to_string(),
        );
        self.store.insert(&task).await?;
        self.events
            .append(task.task_id, TaskEventKind::Created, None)
            .await?;
        Ok(task)
    }

    /// Create a new Pending task with a caller-supplied `task_id` and persist it.
    ///
    /// The durable async-dispatch path (DISPATCH-MESH-DURABILITY gap-a) mints
    /// the `job_id` up front (it is recorded in the at-most-once idempotency
    /// ledger before the row is created), so the durable row's `task_id` MUST
    /// equal that `job_id` — otherwise a deduped `/rpc/task/assign` would return
    /// a `job_id` that `/rpc/task/status` cannot resolve. `create` mints its own
    /// UUID, so this variant exists for that exact need.
    pub async fn create_with_id(
        &self,
        task_id: Uuid,
        workspace_id: &str,
        agent_name: &str,
        prompt: &str,
    ) -> Result<TaskRecord> {
        let mut task = TaskRecord::new(
            workspace_id.to_string(),
            agent_name.to_string(),
            prompt.to_string(),
        );
        task.task_id = task_id;
        self.store.insert(&task).await?;
        self.events
            .append(task.task_id, TaskEventKind::Created, None)
            .await?;
        Ok(task)
    }

    /// Move a task to a new state, enforcing legal transitions. Terminal states
    /// are absorbing — any further transition attempt returns an error.
    pub async fn transition(
        &self,
        task_id: Uuid,
        next: TaskStatus,
        error: Option<&str>,
    ) -> Result<TaskRecord> {
        let current = self
            .store
            .get(task_id)
            .await?
            .ok_or_else(|| anyhow!("task {} not found", task_id))?;

        if !is_legal_transition(current.status, next) {
            return Err(anyhow!(
                "illegal transition: {:?} -> {:?} for task {}",
                current.status,
                next,
                task_id
            ));
        }

        self.store.update_status(task_id, next, error).await?;
        // Append the matching lifecycle event (Started / Completed / Failed /
        // Cancelled). Pending/AwaitingApproval do not produce a distinct event.
        if let Some(kind) = Self::kind_for_status(next) {
            self.events.append(task_id, kind, error).await?;
        }
        Ok(self
            .store
            .get(task_id)
            .await?
            .ok_or_else(|| anyhow!("task vanished after transition"))?)
    }

    /// Record a terminal result for a running async dispatch job in one call:
    /// persist `output` (if any) then transition to `status`
    /// (DISPATCH-MESH-DURABILITY gap-a). Output is written BEFORE the status
    /// flip so a `/rpc/task/status` poll that observes `Completed` always sees
    /// the output too (never a done-but-empty window).
    pub async fn record_result(
        &self,
        task_id: Uuid,
        status: TaskStatus,
        output: Option<&str>,
        error: Option<&str>,
    ) -> Result<TaskRecord> {
        if let Some(o) = output {
            self.store.set_output(task_id, o).await?;
        }
        self.transition(task_id, status, error).await
    }

    /// Increment the turn counter and cost for a running task. Does not change state.
    pub async fn record_turn(&self, task_id: Uuid, cost_delta: f64) -> Result<()> {
        self.store.record_turn(task_id, cost_delta).await
    }

    /// Bulk-record progress (turns + cost) from a completed agent run.
    pub async fn record_progress(
        &self,
        task_id: Uuid,
        turns_delta: u32,
        cost_delta: f64,
    ) -> Result<()> {
        self.store
            .record_progress(task_id, turns_delta, cost_delta)
            .await
    }

    pub async fn get(&self, task_id: Uuid) -> Result<Option<TaskRecord>> {
        self.store.get(task_id).await
    }

    pub async fn list(
        &self,
        workspace_id: Option<&str>,
        status: Option<TaskStatus>,
        limit: usize,
    ) -> Result<Vec<TaskRecord>> {
        self.store.list(workspace_id, status, limit).await
    }

    /// Mark every Pending / Running / AwaitingApproval task as Failed with an
    /// "interrupted" reason. Called once at daemon startup so a crashed task
    /// stops looking like it's still in flight.
    ///
    /// Returns the number of tasks that were transitioned.
    pub async fn mark_interrupted(&self) -> Result<usize> {
        let mut count = 0;
        for status in [
            TaskStatus::Pending,
            TaskStatus::AwaitingApproval,
            TaskStatus::Running,
        ] {
            let stale = self.store.list(None, Some(status), 10_000).await?;
            for t in stale {
                match self
                    .store
                    .update_status(
                        t.task_id,
                        TaskStatus::Failed,
                        Some("interrupted: daemon restart"),
                    )
                    .await
                {
                    Ok(_) => {
                        count += 1;
                        // Bypasses `transition`, so emit the lifecycle event
                        // here. Recorded as `Interrupted` (a distinct kind from
                        // a normal Failed) even though the row's status becomes
                        // Failed, so replay can tell a crash-sweep apart from a
                        // genuine task failure.
                        if let Err(e) = self
                            .events
                            .append(
                                t.task_id,
                                TaskEventKind::Interrupted,
                                Some("interrupted: daemon restart"),
                            )
                            .await
                        {
                            tracing::warn!(task_id = %t.task_id, "mark_interrupted: event append failed: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(task_id = %t.task_id, "mark_interrupted: update_status failed (count not incremented): {}", e)
                    }
                }
            }
        }
        Ok(count)
    }
}

fn is_legal_transition(from: TaskStatus, to: TaskStatus) -> bool {
    use TaskStatus::*;
    match (from, to) {
        // Can always cancel (except from terminal)
        (s, Cancelled) if !s.is_terminal() => true,
        // Pending -> Running (direct) or AwaitingApproval
        (Pending, Running) => true,
        (Pending, AwaitingApproval) => true,
        // AwaitingApproval -> Running (approved / RESUMED) or Failed (rejected)
        (AwaitingApproval, Running) => true,
        (AwaitingApproval, Failed) => true,
        // Running -> Completed / Failed, or PARKED back to AwaitingApproval by a
        // phone STOP (apex-④ off-switch): a running task is paused, not killed,
        // so it can be RESUMED (AwaitingApproval -> Running) later.
        (Running, Completed) => true,
        (Running, Failed) => true,
        (Running, AwaitingApproval) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    async fn setup() -> (TaskQueue, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let store = TaskStore::open_at(dir.path().join("t.db")).unwrap();
        (TaskQueue::new(store), dir)
    }

    #[tokio::test]
    async fn happy_path_pending_running_completed() {
        let (q, _d) = setup().await;
        let t = q.create("ws1", "master", "hi").await.unwrap();
        assert_eq!(t.status, TaskStatus::Pending);

        let running = q
            .transition(t.task_id, TaskStatus::Running, None)
            .await
            .unwrap();
        assert_eq!(running.status, TaskStatus::Running);
        assert!(running.started_at.is_some());

        let done = q
            .transition(t.task_id, TaskStatus::Completed, None)
            .await
            .unwrap();
        assert_eq!(done.status, TaskStatus::Completed);
        assert!(done.finished_at.is_some());
    }

    #[tokio::test]
    async fn cannot_resurrect_terminal_task() {
        let (q, _d) = setup().await;
        let t = q.create("ws1", "master", "hi").await.unwrap();
        q.transition(t.task_id, TaskStatus::Running, None)
            .await
            .unwrap();
        q.transition(t.task_id, TaskStatus::Completed, None)
            .await
            .unwrap();

        let bad = q.transition(t.task_id, TaskStatus::Running, None).await;
        assert!(bad.is_err());
    }

    #[tokio::test]
    async fn awaiting_approval_path() {
        let (q, _d) = setup().await;
        let t = q.create("ws1", "master", "risky").await.unwrap();
        q.transition(t.task_id, TaskStatus::AwaitingApproval, None)
            .await
            .unwrap();

        // approved → running
        q.transition(t.task_id, TaskStatus::Running, None)
            .await
            .unwrap();
        q.transition(t.task_id, TaskStatus::Completed, None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn rejected_approval_goes_to_failed() {
        let (q, _d) = setup().await;
        let t = q.create("ws1", "master", "risky").await.unwrap();
        q.transition(t.task_id, TaskStatus::AwaitingApproval, None)
            .await
            .unwrap();
        q.transition(t.task_id, TaskStatus::Failed, Some("rejected by user"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cancel_from_any_nonterminal_state() {
        let (q, _d) = setup().await;
        let t = q.create("ws1", "master", "hi").await.unwrap();
        q.transition(t.task_id, TaskStatus::Cancelled, None)
            .await
            .unwrap();

        let t2 = q.create("ws1", "master", "hi2").await.unwrap();
        q.transition(t2.task_id, TaskStatus::Running, None)
            .await
            .unwrap();
        q.transition(t2.task_id, TaskStatus::Cancelled, None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn mark_interrupted_sweeps_running_and_pending() {
        let (q, _d) = setup().await;
        let a = q.create("ws1", "master", "running").await.unwrap();
        let _b = q.create("ws1", "master", "still pending").await.unwrap();
        let c = q.create("ws1", "master", "completed before").await.unwrap();
        q.transition(a.task_id, TaskStatus::Running, None)
            .await
            .unwrap();
        q.transition(c.task_id, TaskStatus::Running, None)
            .await
            .unwrap();
        q.transition(c.task_id, TaskStatus::Completed, None)
            .await
            .unwrap();

        let n = q.mark_interrupted().await.unwrap();
        assert_eq!(n, 2); // a and b were nonterminal

        let after = q.list(Some("ws1"), None, 100).await.unwrap();
        let by_id: std::collections::HashMap<_, _> =
            after.iter().map(|t| (t.task_id, t.status)).collect();
        assert_eq!(by_id[&a.task_id], TaskStatus::Failed);
        assert_eq!(by_id[&c.task_id], TaskStatus::Completed); // untouched

        // Should be idempotent — second sweep is a no-op.
        let n2 = q.mark_interrupted().await.unwrap();
        assert_eq!(n2, 0);
    }

    #[tokio::test]
    async fn create_with_id_uses_supplied_uuid() {
        let (q, _d) = setup().await;
        let id = Uuid::new_v4();
        let t = q
            .create_with_id(id, "node-a", "master", "do x")
            .await
            .unwrap();
        assert_eq!(t.task_id, id);
        assert_eq!(t.status, TaskStatus::Pending);
        let got = q.get(id).await.unwrap().unwrap();
        assert_eq!(got.task_id, id);
        assert_eq!(got.agent_name, "master");
    }

    #[tokio::test]
    async fn record_result_persists_output_and_completed() {
        let (q, _d) = setup().await;
        let t = q.create("ws1", "master", "hi").await.unwrap();
        q.transition(t.task_id, TaskStatus::Running, None)
            .await
            .unwrap();
        let done = q
            .record_result(t.task_id, TaskStatus::Completed, Some("RESULT"), None)
            .await
            .unwrap();
        assert_eq!(done.status, TaskStatus::Completed);
        assert_eq!(done.output.as_deref(), Some("RESULT"));
        assert!(done.finished_at.is_some());
    }

    #[tokio::test]
    async fn record_result_failed_keeps_error_no_output() {
        let (q, _d) = setup().await;
        let t = q.create("ws1", "master", "hi").await.unwrap();
        q.transition(t.task_id, TaskStatus::Running, None)
            .await
            .unwrap();
        let failed = q
            .record_result(t.task_id, TaskStatus::Failed, None, Some("boom"))
            .await
            .unwrap();
        assert_eq!(failed.status, TaskStatus::Failed);
        assert_eq!(failed.error.as_deref(), Some("boom"));
        assert_eq!(failed.output, None);
    }

    #[tokio::test]
    async fn list_filters() {
        let (q, _d) = setup().await;
        let a = q.create("ws1", "master", "a").await.unwrap();
        let b = q.create("ws1", "master", "b").await.unwrap();
        q.create("ws2", "master", "c").await.unwrap();
        q.transition(a.task_id, TaskStatus::Running, None)
            .await
            .unwrap();
        q.transition(a.task_id, TaskStatus::Completed, None)
            .await
            .unwrap();
        q.transition(b.task_id, TaskStatus::Running, None)
            .await
            .unwrap();
        q.transition(b.task_id, TaskStatus::Failed, Some("boom"))
            .await
            .unwrap();

        let ws1_all = q.list(Some("ws1"), None, 100).await.unwrap();
        assert_eq!(ws1_all.len(), 2);
        let ws1_failed = q
            .list(Some("ws1"), Some(TaskStatus::Failed), 100)
            .await
            .unwrap();
        assert_eq!(ws1_failed.len(), 1);
    }

    // ---- S0 lane F1: append-only lifecycle event store ----

    #[tokio::test]
    async fn lifecycle_events_appended_in_order_created_started_completed() {
        let (q, _d) = setup().await;
        let t = q.create("ws1", "master", "hi").await.unwrap();
        q.transition(t.task_id, TaskStatus::Running, None)
            .await
            .unwrap();
        q.transition(t.task_id, TaskStatus::Completed, None)
            .await
            .unwrap();

        let evs = q.events_for(t.task_id).await.unwrap();
        let kinds: Vec<_> = evs.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TaskEventKind::Created,
                TaskEventKind::Started,
                TaskEventKind::Completed
            ]
        );
        // Append order (seq) is strictly increasing.
        assert!(evs.windows(2).all(|w| w[0].seq < w[1].seq));
        // Timestamps are monotonic non-decreasing.
        assert!(evs.windows(2).all(|w| w[0].timestamp_ms <= w[1].timestamp_ms));
        // All events belong to the same task.
        assert!(evs.iter().all(|e| e.task_id == t.task_id));
    }

    #[tokio::test]
    async fn lifecycle_events_cancelled_path_and_failure_detail() {
        let (q, _d) = setup().await;

        // Cancelled straight from Pending.
        let c = q.create("ws1", "master", "cancel me").await.unwrap();
        q.transition(c.task_id, TaskStatus::Cancelled, None)
            .await
            .unwrap();
        let cancel_kinds: Vec<_> = q
            .events_for(c.task_id)
            .await
            .unwrap()
            .iter()
            .map(|e| e.kind)
            .collect();
        assert_eq!(
            cancel_kinds,
            vec![TaskEventKind::Created, TaskEventKind::Cancelled]
        );

        // Failed path carries the error string as the event detail.
        let f = q.create("ws1", "master", "fail me").await.unwrap();
        q.transition(f.task_id, TaskStatus::Running, None)
            .await
            .unwrap();
        q.transition(f.task_id, TaskStatus::Failed, Some("boom"))
            .await
            .unwrap();
        let fail_evs = q.events_for(f.task_id).await.unwrap();
        assert_eq!(
            fail_evs.iter().map(|e| e.kind).collect::<Vec<_>>(),
            vec![
                TaskEventKind::Created,
                TaskEventKind::Started,
                TaskEventKind::Failed
            ]
        );
        assert_eq!(fail_evs[2].detail.as_deref(), Some("boom"));

        // Events are isolated per task — cancel-task history is untouched.
        assert_eq!(q.events_for(c.task_id).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn mark_interrupted_appends_interrupted_event() {
        let (q, _d) = setup().await;
        let a = q.create("ws1", "master", "running").await.unwrap();
        q.transition(a.task_id, TaskStatus::Running, None)
            .await
            .unwrap();

        let n = q.mark_interrupted().await.unwrap();
        assert_eq!(n, 1);

        let evs = q.events_for(a.task_id).await.unwrap();
        let kinds: Vec<_> = evs.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TaskEventKind::Created,
                TaskEventKind::Started,
                TaskEventKind::Interrupted
            ]
        );
        assert_eq!(
            evs.last().unwrap().detail.as_deref(),
            Some("interrupted: daemon restart")
        );
    }
}
