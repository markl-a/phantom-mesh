use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// One line in a session JSONL log. The agent loop appends a `SessionEntry`
/// every time it produces a meaningful side-effect (a user prompt arrives, the
/// LLM emits content / tool calls, a tool returns). Replaying these in order
/// reconstructs enough state to resume a crashed task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionEntry {
    /// The user-facing prompt that started a task or arrived mid-conversation.
    User {
        content: String,
        timestamp: i64,
    },
    /// Free-text content emitted by the assistant (no tool call this round).
    Assistant {
        content: String,
        timestamp: i64,
    },
    /// A tool call requested by the assistant. `id` matches the tool_call_id
    /// the LLM uses to correlate with the eventual ToolResult; `args` is the
    /// JSON payload the LLM passed.
    ToolCall {
        call_id: String,
        name: String,
        args: Value,
        timestamp: i64,
    },
    /// Output of a tool execution. `call_id` matches the parent ToolCall.
    /// `synthetic = true` indicates this entry was injected by Session Repair
    /// (e.g. to patch up orphan ToolCalls left behind by a crashed daemon).
    ToolResult {
        call_id: String,
        output: String,
        #[serde(default)]
        synthetic: bool,
        timestamp: i64,
    },
    /// Out-of-band system note — used for resume markers, repair audit, etc.
    System {
        content: String,
        timestamp: i64,
    },
}

impl SessionEntry {
    pub fn timestamp(&self) -> i64 {
        match self {
            Self::User { timestamp, .. }
            | Self::Assistant { timestamp, .. }
            | Self::ToolCall { timestamp, .. }
            | Self::ToolResult { timestamp, .. }
            | Self::System { timestamp, .. } => *timestamp,
        }
    }
}

/// Identity referencing a JSONL session. Kept simple — the canonical path
/// derivation lives in `core::tasks::session`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRef {
    pub workspace_id: String,
    pub task_id: Uuid,
}

impl SessionRef {
    pub fn relative_path(&self) -> String {
        format!("{}/{}.jsonl", self.workspace_id, self.task_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_roundtrip_jsonl() {
        let entries = vec![
            SessionEntry::User {
                content: "hi".into(),
                timestamp: 1,
            },
            SessionEntry::ToolCall {
                call_id: "c1".into(),
                name: "shell".into(),
                args: serde_json::json!({"command": "ls"}),
                timestamp: 2,
            },
            SessionEntry::ToolResult {
                call_id: "c1".into(),
                output: "Cargo.toml".into(),
                synthetic: false,
                timestamp: 3,
            },
            SessionEntry::Assistant {
                content: "found Cargo.toml".into(),
                timestamp: 4,
            },
        ];

        let lines: Vec<String> = entries
            .iter()
            .map(|e| serde_json::to_string(e).unwrap())
            .collect();
        let parsed: Vec<SessionEntry> = lines
            .iter()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert_eq!(parsed.len(), 4);
        assert!(matches!(parsed[2], SessionEntry::ToolResult { synthetic: false, .. }));
    }

    #[test]
    fn synthetic_default_is_false() {
        // Older logs without the `synthetic` field should still parse.
        let line = r#"{"kind":"tool_result","call_id":"c1","output":"x","timestamp":1}"#;
        let entry: SessionEntry = serde_json::from_str(line).unwrap();
        match entry {
            SessionEntry::ToolResult { synthetic, .. } => assert!(!synthetic),
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn session_ref_path_format() {
        let r = SessionRef {
            workspace_id: "abc".into(),
            task_id: Uuid::nil(),
        };
        assert_eq!(r.relative_path(), "abc/00000000-0000-0000-0000-000000000000.jsonl");
    }
}
