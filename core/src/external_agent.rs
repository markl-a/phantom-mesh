//! External coding-agent delegation: adapter trait + unified result schema +
//! probe (sprint MVP T10).
//!
//! phantom is a *control tower* — it can route a unit of work to an external
//! coding agent (Claude Code, Codex, Gemini CLI) and get a comparable result
//! back, rather than being just another model wrapper. This module is the
//! contract layer:
//!
//!   * [`AgentKind`] — the supported external agents.
//!   * [`AgentProbe`] / [`probe_all`] — detect which agents are signed in on
//!     this machine (reuses the existing `providers::*_cli` credential finders;
//!     NO network / API calls).
//!   * [`AgentRequest`] / [`AgentRunResult`] — the unified request/result schema
//!     so `delegate --to claude` and `--to codex` are comparable.
//!   * [`ExternalAgentAdapter`] — the trait every adapter implements. `probe()`
//!     is real here; the live `run()` (actually invoking the agent) is a
//!     separate lane (T11/T12) and is intentionally left to concrete adapters —
//!     this module ships only the contract + detection.

use serde::{Deserialize, Serialize};

/// A supported external coding agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentKind {
    /// Anthropic Claude Code subscription CLI (`claude`).
    ClaudeCode,
    /// OpenAI Codex / "Sign in with ChatGPT" CLI (`codex`).
    Codex,
    /// Google Gemini CLI (`gemini`).
    Gemini,
}

impl AgentKind {
    pub const ALL: [AgentKind; 3] = [AgentKind::ClaudeCode, AgentKind::Codex, AgentKind::Gemini];

    pub fn as_str(&self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Gemini => "gemini",
        }
    }

    // inherent from_str: returns Option (not Result), so it can't be FromStr; callers depend on the Option form.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude_cli" | "claude-code" => Some(AgentKind::ClaudeCode),
            "codex" | "codex_oauth" | "chatgpt" => Some(AgentKind::Codex),
            "gemini" | "gemini_oauth" => Some(AgentKind::Gemini),
            _ => None,
        }
    }

    /// The CLI command a user runs to sign this agent in.
    pub fn signin_command(&self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Gemini => "gemini",
        }
    }

    /// Detect whether this agent is signed in on this machine, reusing the
    /// existing credential finders (filesystem/keychain reads — no API calls).
    pub fn detect_signed_in(&self) -> bool {
        match self {
            AgentKind::ClaudeCode => crate::providers::claude_cli::find_claude_token().is_some(),
            AgentKind::Codex => crate::providers::codex_cli::find_codex_auth().is_some(),
            AgentKind::Gemini => crate::providers::gemini_cli::find_gemini_auth().is_some(),
        }
    }
}

/// Result of probing one external agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentProbe {
    pub kind: AgentKind,
    pub name: String,
    /// True when a usable credential was found (the agent can be delegated to).
    pub signed_in: bool,
    /// What to run to sign in if not detected.
    pub signin_command: String,
}

/// Probe every supported agent. Pure detection — no network, no subprocess.
pub fn probe_all() -> Vec<AgentProbe> {
    AgentKind::ALL
        .iter()
        .map(|k| AgentProbe {
            kind: *k,
            name: k.as_str().to_string(),
            signed_in: k.detect_signed_in(),
            signin_command: k.signin_command().to_string(),
        })
        .collect()
}

/// What kind of work to delegate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentMode {
    /// Produce a plan, no edits.
    Plan,
    /// Produce a patch / code change proposal.
    Patch,
    /// Review a diff / produce a critique.
    Review,
}

impl AgentMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentMode::Plan => "plan",
            AgentMode::Patch => "patch",
            AgentMode::Review => "review",
        }
    }

    // inherent from_str: returns Option (not Result), so it can't be FromStr; callers depend on the Option form.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "plan" => Some(AgentMode::Plan),
            "patch" => Some(AgentMode::Patch),
            "review" => Some(AgentMode::Review),
            _ => None,
        }
    }
}

/// Unified delegation request — the same shape regardless of target agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRequest {
    pub mode: AgentMode,
    pub prompt: String,
    /// Files to include as context (paths, repo-relative).
    #[serde(default)]
    pub files: Vec<String>,
    /// Working directory for the agent.
    pub cwd: String,
}

/// Unified delegation result — so results from different agents are comparable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRunResult {
    pub agent: AgentKind,
    pub mode: AgentMode,
    pub success: bool,
    /// Human-facing output (plan text / review summary).
    pub output: String,
    /// A unified diff, when `mode == Patch` and the agent produced one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
    /// One-line summary for tables / logs.
    pub summary: String,
}

/// The contract every external-agent adapter implements.
///
/// `probe()` is provided/real; `run()` (the live invocation) is the separate
/// T11/T12 lane and is left to concrete adapters so this module carries no I/O.
pub trait ExternalAgentAdapter {
    fn kind(&self) -> AgentKind;

    /// Detect sign-in state for this adapter's agent.
    fn probe(&self) -> AgentProbe {
        let k = self.kind();
        AgentProbe {
            kind: k,
            name: k.as_str().to_string(),
            signed_in: k.detect_signed_in(),
            signin_command: k.signin_command().to_string(),
        }
    }

    /// Run a delegation request. Concrete adapters implement the live call
    /// (subprocess / API) in the T11/T12 lane; the trait only fixes the shape.
    fn run(&self, request: &AgentRequest) -> anyhow::Result<AgentRunResult>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_str_round_trips() {
        for k in AgentKind::ALL {
            assert_eq!(AgentKind::from_str(k.as_str()), Some(k));
        }
        // provider-block aliases also resolve
        assert_eq!(AgentKind::from_str("claude_cli"), Some(AgentKind::ClaudeCode));
        assert_eq!(AgentKind::from_str("codex_oauth"), Some(AgentKind::Codex));
        assert_eq!(AgentKind::from_str("gemini_oauth"), Some(AgentKind::Gemini));
        assert_eq!(AgentKind::from_str("nope"), None);
    }

    #[test]
    fn mode_str_round_trips() {
        for m in [AgentMode::Plan, AgentMode::Patch, AgentMode::Review] {
            assert_eq!(AgentMode::from_str(m.as_str()), Some(m));
        }
        assert_eq!(AgentMode::from_str("bogus"), None);
    }

    #[test]
    fn probe_all_covers_every_agent_with_signin_commands() {
        let probes = probe_all();
        assert_eq!(probes.len(), AgentKind::ALL.len());
        for k in AgentKind::ALL {
            let p = probes.iter().find(|p| p.kind == k).expect("probe present");
            // detection is environment-dependent, but the command is stable.
            assert_eq!(p.signin_command, k.signin_command());
            assert_eq!(p.name, k.as_str());
        }
    }

    #[test]
    fn request_result_schema_round_trips_json() {
        let req = AgentRequest {
            mode: AgentMode::Review,
            prompt: "review this diff".into(),
            files: vec!["core/src/lib.rs".into()],
            cwd: "/repo".into(),
        };
        let j = serde_json::to_string(&req).unwrap();
        let back: AgentRequest = serde_json::from_str(&j).unwrap();
        assert_eq!(req, back);

        let res = AgentRunResult {
            agent: AgentKind::Codex,
            mode: AgentMode::Patch,
            success: true,
            output: "applied".into(),
            patch: Some("--- a\n+++ b\n".into()),
            summary: "1 file changed".into(),
        };
        let j = serde_json::to_string(&res).unwrap();
        let back: AgentRunResult = serde_json::from_str(&j).unwrap();
        assert_eq!(res, back);
        assert!(j.contains("\"patch\""));

        // patch omitted when None (skip_serializing_if)
        let res_no_patch = AgentRunResult { patch: None, ..res };
        let j2 = serde_json::to_string(&res_no_patch).unwrap();
        assert!(!j2.contains("\"patch\""));
    }

    // A trivial adapter proving the trait + default probe() compose.
    struct DummyAdapter(AgentKind);
    impl ExternalAgentAdapter for DummyAdapter {
        fn kind(&self) -> AgentKind {
            self.0
        }
        fn run(&self, request: &AgentRequest) -> anyhow::Result<AgentRunResult> {
            Ok(AgentRunResult {
                agent: self.0,
                mode: request.mode,
                success: true,
                output: format!("dummy {} for: {}", request.mode.as_str(), request.prompt),
                patch: None,
                summary: "dummy".into(),
            })
        }
    }

    #[test]
    fn adapter_trait_default_probe_and_run_compose() {
        let a = DummyAdapter(AgentKind::ClaudeCode);
        assert_eq!(a.kind(), AgentKind::ClaudeCode);
        let p = a.probe();
        assert_eq!(p.kind, AgentKind::ClaudeCode);
        assert_eq!(p.signin_command, "claude");
        let req = AgentRequest {
            mode: AgentMode::Plan,
            prompt: "x".into(),
            files: vec![],
            cwd: ".".into(),
        };
        let r = a.run(&req).unwrap();
        assert!(r.success);
        assert_eq!(r.mode, AgentMode::Plan);
        assert!(r.output.contains("plan"));
    }
}
