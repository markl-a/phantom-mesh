//! Event variants recorded by `Tracer`.
//!
//! Schema is intentionally narrow — adding fields is OK; removing or
//! renaming requires a versioned migration. JSONL files written today
//! should still parse with the tracer one year from now.

use serde::{Deserialize, Serialize};

/// A high-level event in an agent task execution.
///
/// Tagged with a `kind` discriminator so JSONL consumers can fan out
/// by event type without trying every variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// LLM planning output — what the agent intends to do next.
    Plan { plan: String },

    /// Provider / model routing decision and the reason it was picked.
    Route {
        provider: String,
        model: String,
        reason: String,
    },

    /// A tool was invoked with arguments.
    ToolCall {
        name: String,
        args: serde_json::Value,
    },

    /// Result of a tool invocation.
    ToolResult {
        name: String,
        ok: bool,
        summary: String,
        duration_ms: u64,
    },

    /// Final result of the whole task.
    Result { ok: bool, summary: String },
}

/// Wrapper adding task_id, sequence number, and timestamp to an `Event`.
/// One of these is written per line in the JSONL trace file.
///
/// `timestamp_secs` + `timestamp_nanos` are Unix epoch components — chosen
/// over chrono to keep the tracer free of optional-feature dependencies.
/// Consumers can reconstruct an ISO 8601 / RFC 3339 timestamp if needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimestampedEvent {
    pub task_id: String,
    pub seq: u64,
    pub timestamp_secs: u64,
    pub timestamp_nanos: u32,
    #[serde(flatten)]
    pub event: Event,
}
