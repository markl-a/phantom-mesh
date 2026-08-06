//! Fleet orchestrator: single-machine, multi-project meta-scheduler.
//! See docs/superpowers/specs/2026-06-21-fleet-orchestrator-design.md.
pub mod backlog; // Task 2
pub mod build_pool; // Task 5
pub mod conductor;
pub mod crew_executor;
pub mod executor; // Task 6
pub mod gate; // Task 7
pub mod landing; // Task 8
pub mod queue; // Task 3
pub mod scheduler; // Task 4
pub mod types; // Task 9

pub use crew_executor::CrewExecutor;
pub use types::{BacklogTask, ExecOutcome, TaskState, Tier, Verdict};

/// Per-repo scheduling limits.
#[derive(Debug, Clone)]
pub struct Limits {
    pub per_repo_cap: usize,
    pub main_cap: usize,
    pub build_permits: usize,
    pub max_changes_rounds: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            per_repo_cap: 2,
            main_cap: 1,
            build_permits: 4,
            max_changes_rounds: 3,
        }
    }
}

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct RepoEntry {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct FleetConfig {
    #[serde(default)]
    pub repo: Vec<RepoEntry>,
}

impl FleetConfig {
    /// Load from `~/.spectyn-mesh/fleet.toml`, or an empty config if absent.
    pub fn load() -> anyhow::Result<Self> {
        let dir = crate::cli_config::spectyn_data_dir()?;
        let p = dir.join("fleet.toml");
        if !p.exists() {
            return Ok(FleetConfig { repo: vec![] });
        }
        Ok(toml::from_str(&std::fs::read_to_string(p)?)?)
    }
}

/// Ingest every configured repo's backlog into the queue.
/// Returns the count of NEWLY-inserted tasks (re-ingesting an unchanged backlog adds 0).
pub async fn ingest_all(cfg: &FleetConfig, queue: &queue::FleetQueue) -> anyhow::Result<usize> {
    let mut n = 0;
    for r in &cfg.repo {
        for t in backlog::scan_repo(&r.name, std::path::Path::new(&r.path))? {
            if queue.upsert(&t).await? {
                n += 1;
            }
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fleet_config_parses_repo_roots() {
        let toml = "[[repo]]\nname = \"spectyn-quant\"\npath = \"D:/Projects/spectyn-quant\"\n";
        let cfg: FleetConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.repo[0].name, "spectyn-quant");
    }
}
