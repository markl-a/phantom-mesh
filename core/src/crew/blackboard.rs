//! The mediated blackboard — the crew's inter-agent comms. Agents run as
//! separate CLI subprocesses and can't talk directly, so each posts here and the
//! conductor injects `summary()` into the next agent's prompt. Ported from the
//! `ensemble` project (Apache-2.0) §6 crew port.

use serde::{Deserialize, Serialize};

/// One message agents leave for each other on a task-run's shared channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub from: String,
    pub kind: String, // "result" | "verdict" | "finding" | "question"
    pub body: String,
}

/// Append-only per-task-run shared channel. Agents can't talk directly (they are subprocesses),
/// so each posts here and the conductor injects `summary()` into the next agent's prompt — the
/// mediated-blackboard inter-agent-comms pattern (design §4a).
#[derive(Debug, Default)]
pub struct Blackboard {
    msgs: Vec<Message>,
}

impl Blackboard {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn post(&mut self, from: &str, kind: &str, body: &str) {
        self.msgs.push(Message {
            from: from.to_string(),
            kind: kind.to_string(),
            body: body.to_string(),
        });
    }
    pub fn len(&self) -> usize {
        self.msgs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.msgs.is_empty()
    }
    /// Messages at index >= `n`.
    pub fn read_since(&self, n: usize) -> &[Message] {
        let n = n.min(self.msgs.len());
        &self.msgs[n..]
    }
    /// A compact rolling summary injected into the next agent's prompt. Bodies are excerpted to
    /// keep the prompt budget bounded.
    pub fn summary(&self) -> String {
        if self.msgs.is_empty() {
            return String::new();
        }
        let mut s = String::from("Other agents are working on this task. Recent activity:\n");
        for m in &self.msgs {
            let body = excerpt(&m.body, 400);
            s.push_str(&format!("- {} [{}]: {}\n", m.from, m.kind, body));
        }
        s
    }
}

fn excerpt(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= max {
        s
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posts_reads_and_summarizes() {
        let mut bb = Blackboard::new();
        assert!(bb.summary().is_empty());
        bb.post("codex", "result", "implemented the parser");
        bb.post("claude", "verdict", "VERDICT: CHANGES: handle empty input");
        assert_eq!(bb.len(), 2);
        let s = bb.summary();
        assert!(s.contains("codex"));
        assert!(s.contains("implemented the parser"));
        assert!(s.contains("claude"));
        // read_since returns only newer messages
        assert_eq!(bb.read_since(1).len(), 1);
        assert_eq!(bb.read_since(1)[0].from, "claude");
    }
}
