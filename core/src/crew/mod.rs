//! Crew choreography — governed cross-vendor collaborative dev orchestration,
//! ported from the standalone `ensemble` project (Apache-2.0) into this AGPL
//! crate per ECOSYSTEM master-plan §6 (the "port, not crate-dep" decision: a
//! `rusqlite` `links="sqlite3"` version clash makes ensemble un-depend-able, and
//! the orchestration files are pure-logic anyway).
//!
//! Slice-1 ported the dependency-light, self-tested **gate subsystem** — the
//! `Verdict` parser, the `GatePolicy` policy types, and the distinct-vendor
//! approval `gate::decide` — which directly encodes this repo's own double-/
//! triple-gate philosophy. Slice-2a adds the conductor's pure foundation: the
//! `Adapter` integration seam (+ rate-limit detection + `MockAdapter`) and the
//! mediated `Blackboard` (inter-agent comms). The conductor (round-runner
//! brain) + its runtime deps (supervise / test_gate / worktree / journal /
//! full `CrewConfig`) and spectyn's own `GovernedCliAdapter` (driving
//! codex/claude/agy/opencode through `cli_session` + `governed_run`) land in
//! later slices.

pub mod adapter;
pub mod blackboard;
pub mod conductor;
pub mod config;
pub mod gate;
pub mod governed_adapter;
pub mod matrix_plan;
pub mod spectyn_adapter;
pub mod supervise;
pub mod test_gate;
pub mod verdict;

pub use adapter::{
    detect_rate_limit, Adapter, AdapterError, AgentOutput, MockAdapter, RateLimitInfo,
};
pub use blackboard::{Blackboard, Message};
pub use conductor::{Conductor, Decision, RunOutcome};
pub use config::{
    AgentConfig, CrewConfig, CrewError, CrewInspection, CrewRoleInspection, GatePolicy, OnFlake,
    RoleConfig, TestConfig,
};
pub use gate::{decide, GateDecision, RoleVerdict};
pub use governed_adapter::{
    cli_kind_for, fold_session_events, map_governed_fold, GovernanceCtx, GovernedCliAdapter,
};
pub use matrix_plan::{
    capability_for, parse_matrix_targets, render_spec_toml, target_slug, MatrixStatus, MatrixTarget,
};
pub use spectyn_adapter::{fold_spectyn_output, SpectynAgentAdapter};
pub use supervise::{ControlState, RunObserver};
pub use test_gate::{run_tests, TestOutcome};
pub use verdict::{parse_verdict, Verdict};
