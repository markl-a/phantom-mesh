//! Crew configuration — the pipeline of roles, the distinct-vendor gate policy,
//! per-agent overrides (backup / remote node / args / timeout), and the optional
//! automated test gate. Loaded from a `crew.toml`. Ported from the `ensemble`
//! project (Apache-2.0) §6 crew port (slice-2b: the full `CrewConfig` loader;
//! slice-1 carried only the plain `GatePolicy`/`OnFlake` the gate decision needs).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// What to do when a reviewer agent flakes. `Exclude` drops it from the quorum with a logged
/// reason (never fake a pass). `Retry` re-runs the same agent once; `Substitute` falls back to the
/// agent's configured backup. In every case a verdict only enters the quorum from a real
/// successful run — a flake is never counted as approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnFlake {
    Exclude,
    Retry,
    Substitute,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatePolicy {
    /// Distinct-vendor approvals required to LAND (the double-/triple-gate quorum).
    pub min_approvals: u32,
    /// Hard cap on review rounds before escalation.
    pub max_rounds: u32,
    #[serde(deserialize_with = "de_on_flake")]
    pub on_flake: OnFlake,
    /// Firewall B: break early if the implementer makes no progress (byte-identical output + test
    /// result) for this many consecutive rounds. 0 (default) = disabled; only `max_rounds` applies.
    #[serde(default)]
    pub stall_limit: u32,
    /// Firewall B: a wall-clock budget per task in seconds — a practical stand-in for a token
    /// budget. 0 (default) = disabled.
    #[serde(default)]
    pub max_task_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RoleConfig {
    pub agent: String,
    #[serde(default)]
    pub blind: bool,
}

/// Per-agent overrides. `backup` names the agent to substitute when this agent flakes and the
/// gate's `on_flake = "substitute"`. `node` is the base URL of a remote `ensemble serve` host that
/// runs this agent (e.g. "http://acer.tail.ts.net:7878") — when set, the orchestrator drives the
/// agent on that node over HTTP instead of locally.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    #[serde(default)]
    pub backup: Option<String>,
    #[serde(default)]
    pub node: Option<String>,
    /// Extra CLI args appended to this agent's local invocation (`[agents.<n>] args = [...]`).
    /// Vendor-agnostic: e.g. `args = ["--model", "gpt-5.5"]` selects a model. Ignored for a remote node.
    #[serde(default)]
    pub args: Option<Vec<String>>,
    /// Per-command timeout in SECONDS for this agent's local turns (`[agents.<n>] timeout = N`),
    /// overriding the adapter default. Ignored for a remote node.
    #[serde(default)]
    pub timeout: Option<u64>,
}

/// The automated test gate (firewall A). When set, the project's real test `command` must pass
/// (exit 0 = GREEN) before a task can land; a RED suite bounces the traceback back to the
/// implementer. Absent ⇒ no test gate (AI review is the only gate, as before).
#[derive(Debug, Clone, Deserialize)]
pub struct TestConfig {
    /// shell command run in the worktree; exit 0 = GREEN.
    pub command: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CrewConfig {
    pub gate: GatePolicy,
    pub pipeline: Vec<String>,
    pub roles: HashMap<String, RoleConfig>,
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,
    /// Optional automated test gate (firewall A).
    #[serde(default)]
    pub test: Option<TestConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrewRoleInspection {
    pub role: String,
    pub agent: String,
    pub blind: bool,
    pub node: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrewInspection {
    pub pipeline: Vec<String>,
    pub min_approvals: u32,
    pub max_rounds: u32,
    pub on_flake: String,
    pub test_command: Option<String>,
    pub implementer: Option<CrewRoleInspection>,
    pub reviewers: Vec<CrewRoleInspection>,
    pub reviewer_agents: Vec<String>,
    pub distinct_reviewer_agents: usize,
    pub explicit_remote_agents: Vec<String>,
}

fn de_on_flake<'de, D: serde::Deserializer<'de>>(d: D) -> Result<OnFlake, D::Error> {
    let s = String::deserialize(d)?;
    match s.as_str() {
        "exclude" => Ok(OnFlake::Exclude),
        "retry" => Ok(OnFlake::Retry),
        "substitute" => Ok(OnFlake::Substitute),
        other => Err(serde::de::Error::custom(format!(
            "on_flake = \"{other}\" is not supported (use \"exclude\", \"retry\", or \"substitute\")"
        ))),
    }
}

/// Error from loading a crew config: a TOML parse failure, or a semantic-validation failure
/// (e.g. an empty `pipeline`, which would otherwise panic the conductor at `pipeline[0]`).
#[derive(Debug, thiserror::Error)]
pub enum CrewError {
    #[error("crew config parse: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("crew config invalid: {0}")]
    Invalid(String),
}

impl CrewConfig {
    pub fn from_toml(s: &str) -> Result<Self, CrewError> {
        let c: CrewConfig = toml::from_str(s)?;
        // A valid-but-empty pipeline parses fine but would panic the conductor at `pipeline[0]`
        // (the implementer). Reject it here so every real entry point (from_path → the CLI) fails
        // cleanly instead of panicking on malformed input.
        if c.pipeline.is_empty() {
            return Err(CrewError::Invalid(
                "pipeline must have at least one role (the implementer)".to_string(),
            ));
        }
        Ok(c)
    }
    pub fn from_path(p: &std::path::Path) -> std::io::Result<Self> {
        let s = std::fs::read_to_string(p)?;
        Self::from_toml(&s).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
    /// First pipeline role = the implementer.
    pub fn implementer_role(&self) -> &str {
        &self.pipeline[0]
    }
    /// All pipeline roles after the implementer = reviewers (their verdicts feed the gate).
    pub fn reviewer_roles(&self) -> Vec<&str> {
        self.pipeline.iter().skip(1).map(|s| s.as_str()).collect()
    }
    /// The backup agent configured for `agent` (used when `on_flake = "substitute"`), if any.
    pub fn backup_for(&self, agent: &str) -> Option<&str> {
        self.agents.get(agent).and_then(|a| a.backup.as_deref())
    }
    /// The remote node base URL configured for `agent` (a `[agents.<n>] node = "http://..."`),
    /// if any. When set, the orchestrator drives `agent` on that node via a remote adapter.
    pub fn node_for(&self, agent: &str) -> Option<&str> {
        self.agents.get(agent).and_then(|a| a.node.as_deref())
    }
    /// Extra CLI args configured for `agent`, appended to its LOCAL invocation, if any.
    pub fn args_for(&self, agent: &str) -> Option<&[String]> {
        self.agents.get(agent).and_then(|a| a.args.as_deref())
    }
    /// The per-command timeout (seconds) configured for `agent` for its LOCAL turns, if any.
    pub fn timeout_for(&self, agent: &str) -> Option<u64> {
        self.agents.get(agent).and_then(|a| a.timeout)
    }

    pub fn inspect(&self) -> CrewInspection {
        let role_inspection = |role: &str| -> Option<CrewRoleInspection> {
            let cfg = self.roles.get(role)?;
            Some(CrewRoleInspection {
                role: role.to_string(),
                agent: cfg.agent.clone(),
                blind: cfg.blind,
                node: self.node_for(&cfg.agent).map(|n| n.to_string()),
            })
        };
        let implementer = role_inspection(self.implementer_role());
        let reviewers: Vec<CrewRoleInspection> = self
            .reviewer_roles()
            .into_iter()
            .filter_map(role_inspection)
            .collect();
        let reviewer_agents: Vec<String> = reviewers.iter().map(|r| r.agent.clone()).collect();
        let distinct_reviewer_agents = reviewer_agents
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len();
        let mut role_agents = std::collections::HashSet::new();
        if let Some(imp) = &implementer {
            role_agents.insert(imp.agent.clone());
        }
        for reviewer in &reviewers {
            role_agents.insert(reviewer.agent.clone());
        }
        let mut explicit_remote_agents: Vec<String> = self
            .agents
            .iter()
            .filter_map(|(agent, cfg)| {
                if cfg.node.is_some() && role_agents.contains(agent) {
                    Some(agent.clone())
                } else {
                    None
                }
            })
            .collect();
        explicit_remote_agents.sort();
        CrewInspection {
            pipeline: self.pipeline.clone(),
            min_approvals: self.gate.min_approvals,
            max_rounds: self.gate.max_rounds,
            on_flake: match self.gate.on_flake {
                OnFlake::Exclude => "exclude",
                OnFlake::Retry => "retry",
                OnFlake::Substitute => "substitute",
            }
            .to_string(),
            test_command: self.test.as_ref().map(|t| t.command.clone()),
            implementer,
            reviewers,
            reviewer_agents,
            distinct_reviewer_agents,
            explicit_remote_agents,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pipeline_roles_and_gate() {
        let toml = r#"
            pipeline = ["implement", "review", "debug"]
            [gate]
            min_approvals = 2
            max_rounds = 2
            on_flake = "exclude"
            [roles.implement]
            agent = "codex"
            [roles.review]
            agent = "claude"
            blind = true
            [roles.debug]
            agent = "agy"
        "#;
        let c = CrewConfig::from_toml(toml).unwrap();
        assert_eq!(c.pipeline, vec!["implement", "review", "debug"]);
        assert_eq!(c.gate.min_approvals, 2);
        assert_eq!(c.gate.max_rounds, 2);
        assert!(matches!(c.gate.on_flake, OnFlake::Exclude));
        assert_eq!(c.roles["implement"].agent, "codex");
        assert!(c.roles["review"].blind);
        assert!(!c.roles["debug"].blind);
        // implementer = first pipeline role; reviewers = the rest
        assert_eq!(c.implementer_role(), "implement");
        assert_eq!(c.reviewer_roles(), vec!["review", "debug"]);
    }

    #[test]
    fn rejects_empty_pipeline_so_conductor_cannot_panic() {
        let toml = r#"
            pipeline = []
            [gate]
            min_approvals = 1
            max_rounds = 1
            on_flake = "exclude"
        "#;
        assert!(
            CrewConfig::from_toml(toml).is_err(),
            "empty pipeline must be rejected"
        );
    }

    #[test]
    fn parses_all_on_flake_and_agent_backups() {
        let toml = r#"
            pipeline = ["implement", "review"]
            [gate]
            min_approvals = 1
            max_rounds = 1
            on_flake = "substitute"
            [roles.implement]
            agent = "codex"
            [roles.review]
            agent = "claude"
            [agents.claude]
            backup = "opencode"
        "#;
        let c = CrewConfig::from_toml(toml).unwrap();
        assert!(matches!(c.gate.on_flake, OnFlake::Substitute));
        assert_eq!(c.backup_for("claude"), Some("opencode"));
        assert_eq!(c.backup_for("codex"), None);
    }

    #[test]
    fn rejects_unknown_on_flake() {
        let toml = r#"
            pipeline = ["implement", "review"]
            [gate]
            min_approvals = 1
            max_rounds = 1
            on_flake = "teleport"
            [roles.implement]
            agent = "codex"
            [roles.review]
            agent = "claude"
        "#;
        assert!(CrewConfig::from_toml(toml).is_err());
    }

    #[test]
    fn parses_firewall_b_gate_fields() {
        let toml = r#"
            pipeline = ["i"]
            [gate]
            min_approvals = 1
            max_rounds = 5
            on_flake = "exclude"
            stall_limit = 2
            max_task_secs = 30
            [roles.i]
            agent = "codex"
        "#;
        let c = CrewConfig::from_toml(toml).unwrap();
        assert_eq!(c.gate.stall_limit, 2);
        assert_eq!(c.gate.max_task_secs, 30);
        // absent → 0 (disabled), backward compatible
        let c2 = CrewConfig::from_toml(
            "pipeline=[\"i\"]\n[gate]\nmin_approvals=1\nmax_rounds=1\non_flake=\"exclude\"\n[roles.i]\nagent=\"codex\"",
        )
        .unwrap();
        assert_eq!(c2.gate.stall_limit, 0);
        assert_eq!(c2.gate.max_task_secs, 0);
    }

    #[test]
    fn parses_optional_test_gate() {
        let toml = r#"
            pipeline = ["implement","review"]
            [gate]
            min_approvals = 1
            max_rounds = 2
            on_flake = "exclude"
            [roles.implement]
            agent = "codex"
            [roles.review]
            agent = "claude"
            [test]
            command = "cargo test --quiet"
        "#;
        let c = CrewConfig::from_toml(toml).unwrap();
        assert_eq!(c.test.as_ref().unwrap().command, "cargo test --quiet");
        // absent [test] → None (backward compatible)
        let c2 = CrewConfig::from_toml(
            "pipeline=[\"i\"]\n[gate]\nmin_approvals=1\nmax_rounds=1\non_flake=\"exclude\"\n[roles.i]\nagent=\"codex\"",
        )
        .unwrap();
        assert!(c2.test.is_none());
    }

    #[test]
    fn parses_remote_agent_node_url() {
        let toml = r#"
            pipeline = ["implement","review"]
            [gate]
            min_approvals = 1
            max_rounds = 1
            on_flake = "exclude"
            [roles.implement]
            agent = "codex"
            [roles.review]
            agent = "claude"
            [agents.claude]
            node = "http://acer.tail.ts.net:7878"
        "#;
        let c = CrewConfig::from_toml(toml).unwrap();
        assert_eq!(c.node_for("claude"), Some("http://acer.tail.ts.net:7878"));
        assert_eq!(c.node_for("codex"), None);
    }

    #[test]
    fn parses_per_agent_args_and_timeout() {
        let toml = r#"
            pipeline = ["implement"]
            [gate]
            min_approvals = 1
            max_rounds = 1
            on_flake = "exclude"
            [roles.implement]
            agent = "codex"
            [agents.codex]
            args = ["--model", "gpt-5.5"]
            timeout = 900
        "#;
        let c = CrewConfig::from_toml(toml).unwrap();
        assert_eq!(
            c.args_for("codex"),
            Some(&["--model".to_string(), "gpt-5.5".to_string()][..])
        );
        assert_eq!(c.timeout_for("codex"), Some(900));
        assert_eq!(c.args_for("claude"), None);
        assert_eq!(c.timeout_for("claude"), None);
    }

    #[test]
    fn from_path_reads_a_file_then_parses() {
        // Replaces ensemble's two `examples/crew*.toml` file tests (those example files are
        // ensemble-specific and not carried into phantom): prove from_path reads a real file and
        // routes through from_toml, against a tempfile we write ourselves.
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(
            f,
            "pipeline = [\"implement\", \"review\", \"debug\"]\n\
             [gate]\nmin_approvals = 2\nmax_rounds = 2\non_flake = \"exclude\"\n\
             [roles.implement]\nagent = \"codex\"\n\
             [roles.review]\nagent = \"claude\"\n\
             [roles.debug]\nagent = \"agy\"\n"
        )
        .unwrap();
        let c = CrewConfig::from_path(f.path()).expect("from_path must parse a written file");
        assert_eq!(c.implementer_role(), "implement");
        assert_eq!(c.pipeline, vec!["implement", "review", "debug"]);
        assert!(matches!(c.gate.on_flake, OnFlake::Exclude));
    }

    #[test]
    fn inspect_reports_phase2_governance_inputs() {
        let toml = r#"
            pipeline = ["implement","review","audit"]
            [gate]
            min_approvals = 2
            max_rounds = 2
            on_flake = "exclude"
            [test]
            command = "cargo test --quiet"
            [roles.implement]
            agent = "codex"
            [roles.review]
            agent = "claude"
            blind = true
            [roles.audit]
            agent = "agy"
            blind = true
            [agents.claude]
            node = "http://m2:7878"
            [agents.unused]
            node = "http://unused:7878"
        "#;
        let c = CrewConfig::from_toml(toml).unwrap();
        let got = c.inspect();

        assert_eq!(got.min_approvals, 2);
        assert_eq!(got.test_command.as_deref(), Some("cargo test --quiet"));
        assert_eq!(got.implementer.as_ref().unwrap().agent, "codex");
        assert_eq!(
            got.reviewer_agents,
            vec!["claude".to_string(), "agy".to_string()]
        );
        assert_eq!(got.distinct_reviewer_agents, 2);
        assert_eq!(got.reviewers[0].node.as_deref(), Some("http://m2:7878"));
        assert_eq!(
            got.explicit_remote_agents,
            vec!["claude".to_string()],
            "only explicit remote routes for active pipeline roles should count"
        );
    }
}
