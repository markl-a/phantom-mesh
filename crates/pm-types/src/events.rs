use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::identity::NodeId;

/// A domain event representing a state change in the system.
/// This is the core unit of the Universal Event Spine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainEvent {
    pub event_id: Uuid,
    pub node_id: NodeId,
    /// Per-node monotonic sequence number. DB UNIQUE constraint per node.
    pub seq: u64,
    /// Display timestamp (not used for sync — use seq instead).
    pub timestamp: i64,
    pub trace_id: Uuid,
    pub source: EventSource,
    pub event_type: DomainEventType,
    pub local_payload: PayloadRef,
    pub push_payload: EventSummary,
    /// Post-beta: delegation chain / DAG. MVP = None.
    pub parent_event_id: Option<Uuid>,
    /// Post-beta: privacy model. MVP = Cluster.
    pub visibility: EventVisibility,
}

/// Where the event originated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum EventSource {
    Local,
    Peer,
    External(String),
}

/// Types of domain events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DomainEventType {
    TaskCreated,
    TaskStarted,
    TaskCompleted,
    TaskFailed,
    TaskCancelled,
    ConversationMessage,
    CostAlert,
    ToolExecuted,
    ProviderSelected,
    InferenceCompleted,
    SettingsChanged,
    CronTriggered,
    LogError,
    TaskRetried,
}

/// Reference to a local payload stored in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayloadRef {
    pub table: String,
    pub row_id: i64,
}

/// Inline summary for remote push.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSummary {
    pub summary: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Cluster-level events (separate from domain events).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ClusterEvent {
    Heartbeat { node_id: NodeId },
    NodeOnline { node_id: NodeId },
    NodeOffline { node_id: NodeId },
    ElectionComplete { coordinator_id: NodeId, term: u64 },
}

/// Visibility level for events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum EventVisibility {
    Local,
    Cluster,
    Public,
}

impl Default for EventVisibility {
    fn default() -> Self {
        Self::Cluster
    }
}

/// Push policy controlling remote event distribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PushPolicy {
    Broadcast,
    NoPush,
    Conditional,
}

/// System-level events for monitoring and alerting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SystemEvent {
    TaskCompleted { task_id: String },
    NodeOnline { node_id: NodeId },
    NodeOffline { node_id: NodeId },
    ElectionComplete { coordinator_id: NodeId },
    CostAlert { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_domain_event_creation() {
        let event = DomainEvent {
            event_id: Uuid::new_v4(),
            node_id: "node-1".into(),
            seq: 1,
            timestamp: 1234567890,
            trace_id: Uuid::new_v4(),
            source: EventSource::Local,
            event_type: DomainEventType::TaskCreated,
            local_payload: PayloadRef { table: "tasks".into(), row_id: 1 },
            push_payload: EventSummary { summary: "Task created".into(), metadata: serde_json::Value::Null },
            parent_event_id: None,
            visibility: EventVisibility::default(),
        };
        assert_eq!(event.seq, 1);
        assert_eq!(event.visibility, EventVisibility::Cluster);
    }

    #[test]
    fn test_domain_event_serde_roundtrip() {
        let event = DomainEvent {
            event_id: Uuid::new_v4(),
            node_id: "n1".into(),
            seq: 42,
            timestamp: 0,
            trace_id: Uuid::new_v4(),
            source: EventSource::External("api-key-123".into()),
            event_type: DomainEventType::ToolExecuted,
            local_payload: PayloadRef { table: "tools".into(), row_id: 5 },
            push_payload: EventSummary { summary: "shell ls".into(), metadata: serde_json::json!({"tool": "shell"}) },
            parent_event_id: Some(Uuid::new_v4()),
            visibility: EventVisibility::Local,
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: DomainEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.seq, 42);
        assert_eq!(back.source, EventSource::External("api-key-123".into()));
    }

    #[test]
    fn test_cluster_event_variants() {
        let ev = ClusterEvent::ElectionComplete {
            coordinator_id: "node-1".into(),
            term: 5,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("ElectionComplete"));
    }

    #[test]
    fn test_event_visibility_default() {
        assert_eq!(EventVisibility::default(), EventVisibility::Cluster);
    }
}
