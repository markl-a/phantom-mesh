use anyhow::{anyhow, Result};
use pm_types::{TaskRecord, TaskStatus};
use uuid::Uuid;

use super::store::TaskStore;

/// Facade over the task store that enforces legal state transitions and
/// (future) emits DomainEvents for each transition.
///
/// For now, event emission is a stub — integrate with EventSpine when P6 lands.
#[derive(Clone)]
pub struct TaskQueue {
    store: TaskStore,
}

impl TaskQueue {
    pub fn new(store: TaskStore) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &TaskStore {
        &self.store
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
        // TODO (P6): emit DomainEvent::TaskCreated
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
        // TODO (P6): emit matching DomainEvent (TaskStarted / TaskCompleted / TaskFailed / TaskCancelled)
        Ok(self
            .store
            .get(task_id)
            .await?
            .ok_or_else(|| anyhow!("task vanished after transition"))?)
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
        self.store.record_progress(task_id, turns_delta, cost_delta).await
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
        for status in [TaskStatus::Pending, TaskStatus::AwaitingApproval, TaskStatus::Running] {
            let stale = self.store.list(None, Some(status), 10_000).await?;
            for t in stale {
                let _ = self
                    .store
                    .update_status(t.task_id, TaskStatus::Failed, Some("interrupted: daemon restart"))
                    .await;
                count += 1;
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
        // AwaitingApproval -> Running (approved) or Failed (rejected)
        (AwaitingApproval, Running) => true,
        (AwaitingApproval, Failed) => true,
        // Running -> Completed / Failed
        (Running, Completed) => true,
        (Running, Failed) => true,
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

        let running = q.transition(t.task_id, TaskStatus::Running, None).await.unwrap();
        assert_eq!(running.status, TaskStatus::Running);
        assert!(running.started_at.is_some());

        let done = q.transition(t.task_id, TaskStatus::Completed, None).await.unwrap();
        assert_eq!(done.status, TaskStatus::Completed);
        assert!(done.finished_at.is_some());
    }

    #[tokio::test]
    async fn cannot_resurrect_terminal_task() {
        let (q, _d) = setup().await;
        let t = q.create("ws1", "master", "hi").await.unwrap();
        q.transition(t.task_id, TaskStatus::Running, None).await.unwrap();
        q.transition(t.task_id, TaskStatus::Completed, None).await.unwrap();

        let bad = q.transition(t.task_id, TaskStatus::Running, None).await;
        assert!(bad.is_err());
    }

    #[tokio::test]
    async fn awaiting_approval_path() {
        let (q, _d) = setup().await;
        let t = q.create("ws1", "master", "risky").await.unwrap();
        q.transition(t.task_id, TaskStatus::AwaitingApproval, None).await.unwrap();

        // approved → running
        q.transition(t.task_id, TaskStatus::Running, None).await.unwrap();
        q.transition(t.task_id, TaskStatus::Completed, None).await.unwrap();
    }

    #[tokio::test]
    async fn rejected_approval_goes_to_failed() {
        let (q, _d) = setup().await;
        let t = q.create("ws1", "master", "risky").await.unwrap();
        q.transition(t.task_id, TaskStatus::AwaitingApproval, None).await.unwrap();
        q.transition(t.task_id, TaskStatus::Failed, Some("rejected by user")).await.unwrap();
    }

    #[tokio::test]
    async fn cancel_from_any_nonterminal_state() {
        let (q, _d) = setup().await;
        let t = q.create("ws1", "master", "hi").await.unwrap();
        q.transition(t.task_id, TaskStatus::Cancelled, None).await.unwrap();

        let t2 = q.create("ws1", "master", "hi2").await.unwrap();
        q.transition(t2.task_id, TaskStatus::Running, None).await.unwrap();
        q.transition(t2.task_id, TaskStatus::Cancelled, None).await.unwrap();
    }

    #[tokio::test]
    async fn mark_interrupted_sweeps_running_and_pending() {
        let (q, _d) = setup().await;
        let a = q.create("ws1", "master", "running").await.unwrap();
        let _b = q.create("ws1", "master", "still pending").await.unwrap();
        let c = q.create("ws1", "master", "completed before").await.unwrap();
        q.transition(a.task_id, TaskStatus::Running, None).await.unwrap();
        q.transition(c.task_id, TaskStatus::Running, None).await.unwrap();
        q.transition(c.task_id, TaskStatus::Completed, None).await.unwrap();

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
    async fn list_filters() {
        let (q, _d) = setup().await;
        let a = q.create("ws1", "master", "a").await.unwrap();
        let b = q.create("ws1", "master", "b").await.unwrap();
        q.create("ws2", "master", "c").await.unwrap();
        q.transition(a.task_id, TaskStatus::Running, None).await.unwrap();
        q.transition(a.task_id, TaskStatus::Completed, None).await.unwrap();
        q.transition(b.task_id, TaskStatus::Running, None).await.unwrap();
        q.transition(b.task_id, TaskStatus::Failed, Some("boom")).await.unwrap();

        let ws1_all = q.list(Some("ws1"), None, 100).await.unwrap();
        assert_eq!(ws1_all.len(), 2);
        let ws1_failed = q.list(Some("ws1"), Some(TaskStatus::Failed), 100).await.unwrap();
        assert_eq!(ws1_failed.len(), 1);
    }
}
