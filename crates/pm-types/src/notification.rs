use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::task::TaskStatus;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NotificationPriority {
    /// Immediate delivery to every channel. Failed / cancelled / cost alerts.
    P0,
    /// Batched per-channel summaries (30-minute windows). Completed tasks.
    P1,
    /// Debug log only. Transient status flips.
    P2,
}

impl NotificationPriority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::P0 => "p0",
            Self::P1 => "p1",
            Self::P2 => "p2",
        }
    }
}

/// Derive a priority from a task status. Called by the dispatcher when a
/// TaskRecord transitions.
pub fn classify_priority(status: TaskStatus) -> NotificationPriority {
    match status {
        TaskStatus::Failed | TaskStatus::Cancelled => NotificationPriority::P0,
        TaskStatus::Completed => NotificationPriority::P1,
        _ => NotificationPriority::P2,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotificationAction {
    Retry { task_id: Uuid },
    Details { task_id: Uuid },
    OpenUrl { label: String, url: String },
    Custom { label: String, callback: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: Uuid,
    /// Dedupe key — e.g. `task:<uuid>:completed`. Suppresses duplicate sends
    /// inside the dispatcher's recent-send window.
    pub dedup_key: String,
    pub task_id: Option<Uuid>,
    pub workspace_id: String,
    pub priority: NotificationPriority,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub actions: Vec<NotificationAction>,
    pub timestamp: i64,
}

impl Notification {
    pub fn task_update(
        task_id: Uuid,
        workspace_id: String,
        status: TaskStatus,
        title: String,
        body: String,
    ) -> Self {
        let priority = classify_priority(status);
        Self {
            id: Uuid::new_v4(),
            dedup_key: format!("task:{}:{}", task_id, status.as_str()),
            task_id: Some(task_id),
            workspace_id,
            priority,
            title,
            body,
            actions: vec![],
            timestamp: now_millis(),
        }
    }
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

    #[test]
    fn classify_maps_status_to_priority() {
        assert_eq!(classify_priority(TaskStatus::Failed), NotificationPriority::P0);
        assert_eq!(classify_priority(TaskStatus::Cancelled), NotificationPriority::P0);
        assert_eq!(classify_priority(TaskStatus::Completed), NotificationPriority::P1);
        assert_eq!(classify_priority(TaskStatus::Running), NotificationPriority::P2);
        assert_eq!(classify_priority(TaskStatus::Pending), NotificationPriority::P2);
        assert_eq!(classify_priority(TaskStatus::AwaitingApproval), NotificationPriority::P2);
    }

    #[test]
    fn task_update_dedup_key_format() {
        let tid = Uuid::new_v4();
        let n = Notification::task_update(
            tid,
            "ws1".into(),
            TaskStatus::Failed,
            "Refactor failed".into(),
            "provider timeout".into(),
        );
        assert_eq!(n.dedup_key, format!("task:{}:failed", tid));
        assert_eq!(n.priority, NotificationPriority::P0);
        assert_eq!(n.task_id, Some(tid));
    }

    #[test]
    fn task_update_completed_is_p1() {
        let tid = Uuid::new_v4();
        let n = Notification::task_update(
            tid,
            "ws1".into(),
            TaskStatus::Completed,
            "Done".into(),
            "refactor applied".into(),
        );
        assert_eq!(n.priority, NotificationPriority::P1);
    }
}
