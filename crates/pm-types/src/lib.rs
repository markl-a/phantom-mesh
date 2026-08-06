pub mod capability;
pub mod errors;
pub mod events;
pub mod identity;
pub mod notification;
pub mod rpc;
pub mod session;
pub mod sync;
pub mod task;
pub mod workspace;

// Re-export all public types for convenience.
pub use capability::{Capability, CapabilityLevel, CapabilityQualifier, CapabilityRequirement};
pub use errors::{ErrorKind, SpectynError, TraceContext, TraceId};
pub use events::{
    ClusterEvent, DomainEvent, DomainEventType, EventSource, EventSummary, EventVisibility,
    PayloadRef, PushPolicy, SystemEvent,
};
pub use identity::{ControlPlane, NodeId, NodeIdentity, NodeRole, PeerInfo, PeerStatus};
pub use rpc::{Epoch, RpcRequest, RpcResponse, Term};
pub use sync::SyncStrategy;
pub use notification::{
    classify_priority, Notification, NotificationAction, NotificationPriority,
};
pub use session::{SessionEntry, SessionRef};
pub use task::{TaskRecord, TaskStatus};
pub use workspace::{Workspace, WorkspaceId};
