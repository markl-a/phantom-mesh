//! Shared value types for the fleet orchestrator.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Pending,
    Claimed,
    Executing,
    Gating,
    Landing,
    Landed,
    Staged,
    Parked,
}

impl TaskState {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskState::Pending => "pending",
            TaskState::Claimed => "claimed",
            TaskState::Executing => "executing",
            TaskState::Gating => "gating",
            TaskState::Landing => "landing",
            TaskState::Landed => "landed",
            TaskState::Staged => "staged",
            TaskState::Parked => "parked",
        }
    }
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => TaskState::Pending,
            "claimed" => TaskState::Claimed,
            "executing" => TaskState::Executing,
            "gating" => TaskState::Gating,
            "landing" => TaskState::Landing,
            "landed" => TaskState::Landed,
            "staged" => TaskState::Staged,
            "parked" => TaskState::Parked,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Satellite,
    Main,
}

impl Tier {
    /// The main AGPL repo is the only `Main`; everything else is a satellite.
    pub fn for_repo(repo: &str) -> Tier {
        if repo == "phantom-mesh" || repo == "phantom-mesh" {
            Tier::Main
        } else {
            Tier::Satellite
        }
    }
}

/// What an `Executor` produces from one task run.
#[derive(Debug, Clone)]
pub struct ExecOutcome {
    pub diff: String,
    pub build_ok: bool,
    pub logs: String,
}

/// The double-gate verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Lgtm,
    Changes(Vec<String>),
    Inconclusive,
}

/// A task as parsed from a backlog file plus its identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacklogTask {
    pub task_id: String,
    pub repo: String,
    pub slug: String,
    pub component: String,
    pub acceptance: String,
    pub caps: Vec<String>,
    pub max_files: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_state_roundtrips_through_str() {
        for s in [
            TaskState::Pending,
            TaskState::Claimed,
            TaskState::Executing,
            TaskState::Gating,
            TaskState::Landed,
            TaskState::Staged,
            TaskState::Parked,
        ] {
            assert_eq!(TaskState::from_str(s.as_str()), Some(s));
        }
        assert_eq!(TaskState::from_str("bogus"), None);
    }

    #[test]
    fn tier_of_known_repos() {
        assert_eq!(Tier::for_repo("phantom-quant"), Tier::Satellite);
        assert_eq!(Tier::for_repo("phantom-mesh"), Tier::Main);
    }
}
