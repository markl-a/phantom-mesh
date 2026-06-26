//! L0 normalized event vocabulary + per-event fidelity. Pure data — no I/O.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Honesty tag on every event so flight-recorder replay is explicit, not assumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fidelity {
    /// Structured stream AND cross-checked against the on-disk transcript.
    StructuredVerified,
    /// Structured-ish but can race / lose lines (opencode CLI json, agy klog).
    StructuredBestEffort,
    /// Recovered from a PTY capture (agy stdout); least trusted.
    PtyScraped,
}

/// Where an event was derived from (for audit + reconcile).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    LiveStream,
    Transcript,
    Klog,
}

/// The single event vocabulary every adapter normalizes UP to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    SessionStarted { id: String },
    AssistantText { delta: String },
    ToolCall { name: String, args: Value },
    ToolResult { name: String, output: String, ok: bool },
    Usage { input_tokens: u64, output_tokens: u64, cost_usd: f64 },
    TurnDone { stop_reason: String },
    Error { error_kind: String, detail: String },
}

/// A normalized event with its provenance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CliEvent {
    pub event: EventKind,
    pub fidelity: Fidelity,
    pub source: Source,
}

impl CliEvent {
    pub fn new(event: EventKind, fidelity: Fidelity, source: Source) -> Self {
        Self { event, fidelity, source }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_serializes_with_kind_tag() {
        let e = CliEvent::new(
            EventKind::SessionStarted { id: "abc".into() },
            Fidelity::StructuredVerified,
            Source::LiveStream,
        );
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("\"kind\":\"session_started\""), "got {j}");
        assert!(j.contains("\"fidelity\":\"structured_verified\""), "got {j}");
        let back: CliEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(back, e);
    }
}
