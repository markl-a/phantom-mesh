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
    /// Agent output text for a finished async dispatch job (DISPATCH-MESH
    /// gap-a durable store). `None` until a terminal `Completed` result is
    /// recorded. `#[serde(default)]` so payloads from older binaries (which
    /// never carried this field) still deserialize.
    #[serde(default)]
    pub output: Option<String>,
    /// The governance `ExecutionContract.id` of the pending approval that parked
    /// this task in [`TaskStatus::AwaitingApproval`]. Lets a desktop/phone client
    /// correlate an awaiting-approval task row with its pending approval card
    /// (whose key is the same per-action contract id), instead of falling back to
    /// `task_id`. `None` for any task with no outstanding governance approval.
    /// `#[serde(default)]` so payloads from older binaries still deserialize, and
    /// `skip_serializing_if` so the wire/JSON stays byte-compatible when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
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
            output: None,
            approval_id: None,
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

    #[test]
    fn from_str_rejects_unknown() {
        // Unknown / malformed status strings must return None rather than
        // silently defaulting — the caller decides how to handle bad rows.
        assert_eq!(TaskStatus::from_str("done"), None);
        assert_eq!(TaskStatus::from_str("Completed"), None); // case-sensitive
        assert_eq!(TaskStatus::from_str(""), None);
        assert_eq!(TaskStatus::from_str("awaitingapproval"), None);
    }

    #[test]
    fn as_str_matches_serde_wire_format() {
        // `as_str()` is hand-written; the serde wire format comes from
        // `#[serde(rename_all = "snake_case")]`. These two must stay in
        // lockstep or the SQLite text column and the JSON wire value diverge.
        for s in [
            TaskStatus::Pending,
            TaskStatus::AwaitingApproval,
            TaskStatus::Running,
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ] {
            let json = serde_json::to_string(&s).unwrap();
            // JSON string is the as_str() value wrapped in quotes.
            assert_eq!(json, format!("\"{}\"", s.as_str()), "mismatch for {s:?}");
        }
    }

    #[test]
    fn awaiting_approval_uses_snake_case_wire() {
        // Guard the one multi-word variant explicitly — snake_case, not camelCase.
        let json = serde_json::to_string(&TaskStatus::AwaitingApproval).unwrap();
        assert_eq!(json, "\"awaiting_approval\"");
        let back: TaskStatus = serde_json::from_str("\"awaiting_approval\"").unwrap();
        assert_eq!(back, TaskStatus::AwaitingApproval);
    }

    #[test]
    fn task_record_output_defaults_when_absent() {
        // Records serialized by older binaries omit `output`; #[serde(default)]
        // must let them deserialize with `output == None`.
        let t = TaskRecord::new("ws".into(), "agent".into(), "p".into());
        let mut value = serde_json::to_value(&t).unwrap();
        value.as_object_mut().unwrap().remove("output");
        let back: TaskRecord = serde_json::from_value(value).unwrap();
        assert_eq!(back.output, None);
        assert_eq!(back.task_id, t.task_id);
    }

    #[test]
    fn approval_id_absent_by_default_and_round_trips_when_set() {
        // A normal task carries no governance approval: `approval_id` is None and,
        // because of skip_serializing_if, the key is ABSENT from the JSON the
        // /tasks endpoint emits (so a client sees no approval correlation to make).
        let mut t = TaskRecord::new("ws".into(), "agent".into(), "p".into());
        assert_eq!(t.approval_id, None);
        let json = serde_json::to_value(&t).unwrap();
        assert!(
            json.as_object().unwrap().get("approval_id").is_none(),
            "approval_id key must be ABSENT when None (skip_serializing_if)"
        );

        // Once a contract id is correlated onto the row, it serializes out so a
        // desktop client can match the awaiting-approval task to its pending card.
        let contract_id = "contract-abc-123";
        t.approval_id = Some(contract_id.to_string());
        let json = serde_json::to_value(&t).unwrap();
        assert_eq!(
            json.get("approval_id").and_then(|v| v.as_str()),
            Some(contract_id)
        );

        // Older payloads (no approval_id key) still deserialize (serde default).
        let mut older = serde_json::to_value(&t).unwrap();
        older.as_object_mut().unwrap().remove("approval_id");
        let back: TaskRecord = serde_json::from_value(older).unwrap();
        assert_eq!(back.approval_id, None);
    }
}
