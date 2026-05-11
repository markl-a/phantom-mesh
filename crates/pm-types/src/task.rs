use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::identity::NodeId;

/// Lifecycle state of a long-running task.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    AwaitingApproval,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "awaiting_approval" => Some(Self::AwaitingApproval),
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// Persisted long-running task record. One row in the `tasks` SQLite table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub task_id: Uuid,
    pub workspace_id: String,
    pub session_id: String,
    pub agent_name: String,
    pub prompt: String,
    pub status: TaskStatus,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub parent_task_id: Option<Uuid>,
    pub assigned_node: Option<NodeId>,
    pub cost_usd: f64,
    pub turns: u32,
    pub error: Option<String>,
    pub trace_id: Uuid,
}

impl TaskRecord {
    pub fn new(workspace_id: String, agent_name: String, prompt: String) -> Self {
        let task_id = Uuid::new_v4();
        let trace_id = Uuid::new_v4();
        let session_id = format!("{}-{}", workspace_id, task_id);
        Self {
            task_id,
            workspace_id,
            session_id,
            agent_name,
            prompt,
            status: TaskStatus::Pending,
            created_at: now_millis(),
            started_at: None,
            finished_at: None,
            parent_task_id: None,
            assigned_node: None,
            cost_usd: 0.0,
            turns: 0,
            error: None,
            trace_id,
        }
    }
}

pub(crate) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_roundtrip() {
        for s in [
            TaskStatus::Pending,
            TaskStatus::AwaitingApproval,
            TaskStatus::Running,
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ] {
            assert_eq!(TaskStatus::from_str(s.as_str()), Some(s));
        }
    }

    #[test]
    fn terminal_statuses() {
        assert!(TaskStatus::Completed.is_terminal());
        assert!(TaskStatus::Failed.is_terminal());
        assert!(TaskStatus::Cancelled.is_terminal());
        assert!(!TaskStatus::Running.is_terminal());
        assert!(!TaskStatus::Pending.is_terminal());
    }

    #[test]
    fn new_task_has_ids() {
        let t = TaskRecord::new("ws1".into(), "master".into(), "hi".into());
        assert_eq!(t.status, TaskStatus::Pending);
        assert!(t.session_id.starts_with("ws1-"));
        assert_eq!(t.workspace_id, "ws1");
    }
}
