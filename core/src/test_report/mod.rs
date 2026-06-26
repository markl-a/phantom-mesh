//! P2-2 release-evidence ship-gate collector (SPEC-60 §7 / SPEC-61).
//!
//! This module is the machine-checkable "are we actually done?" engine: it loads
//! a *data* gate-map (`appendix/ship-gate-map.toml`) mapping each SPEC-60 ship-gate
//! `V1..V12` (+ SPEC-61 `S1..S40` scenarios) to a concrete check, resolves and
//! (optionally) runs the checks it can, and emits a structured [`ShipGateReport`].
//!
//! THE HONESTY CONTRACT IS LOAD-BEARING (do not violate):
//! - A gate with **no resolvable check** is [`GateStatus::Unknown`] — NEVER silently
//!   colored green. "No check wired" ≠ "passed".
//! - A check that fails/errors, or whose cited target does not resolve to real code,
//!   makes its gate [`GateStatus::Red`] (`unresolved-check`), mirroring the rule
//!   `scripts/check-test-citations.sh` already enforces.
//! - `manual` checks are never auto-run and never auto-green.
//! - `overall_status` is [`OverallStatus::Green`] only if every gate is `Green` (or
//!   `Skipped`, or non-green-but-operator-overridden). Any unacknowledged
//!   `Red`/`Manual`/`Unknown` ⇒ `Red` — you cannot ship un-evidenced.
//!
//! Network is OUT OF SCOPE for P2-2: parsing/classification/report-building is pure;
//! the report is written to a file + stdout only (SPEC-60 §9.5/§9.6 Sentry/Telegram
//! wiring is a deliberately-deferred follow-up — operator decision 1).

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub mod runner;

// ───────────────────────────────────────────────────────────────────────────
// SPEC-60 §7 data model (rename_all = "camelCase" to match the TS interface).
// ───────────────────────────────────────────────────────────────────────────

/// The 12 SPEC-60 ship-gates.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "UPPERCASE")]
pub enum GateId {
    V1,
    V2,
    V3,
    V4,
    V5,
    V6,
    V7,
    V8,
    V9,
    V10,
    V11,
    V12,
}

impl GateId {
    pub fn as_str(&self) -> &'static str {
        match self {
            GateId::V1 => "V1",
            GateId::V2 => "V2",
            GateId::V3 => "V3",
            GateId::V4 => "V4",
            GateId::V5 => "V5",
            GateId::V6 => "V6",
            GateId::V7 => "V7",
            GateId::V8 => "V8",
            GateId::V9 => "V9",
            GateId::V10 => "V10",
            GateId::V11 => "V11",
            GateId::V12 => "V12",
        }
    }

    pub fn parse(s: &str) -> Option<GateId> {
        match s.to_ascii_uppercase().as_str() {
            "V1" => Some(GateId::V1),
            "V2" => Some(GateId::V2),
            "V3" => Some(GateId::V3),
            "V4" => Some(GateId::V4),
            "V5" => Some(GateId::V5),
            "V6" => Some(GateId::V6),
            "V7" => Some(GateId::V7),
            "V8" => Some(GateId::V8),
            "V9" => Some(GateId::V9),
            "V10" => Some(GateId::V10),
            "V11" => Some(GateId::V11),
            "V12" => Some(GateId::V12),
            _ => None,
        }
    }

    /// All 12 gates in V1..V12 order — used for completeness validation.
    pub fn all() -> [GateId; 12] {
        [
            GateId::V1,
            GateId::V2,
            GateId::V3,
            GateId::V4,
            GateId::V5,
            GateId::V6,
            GateId::V7,
            GateId::V8,
            GateId::V9,
            GateId::V10,
            GateId::V11,
            GateId::V12,
        ]
    }
}

/// Honest per-gate status.
///
/// DRIFT (DOCUMENTATION-CHARTER): SPEC-60 §7.1 specifies
/// `pending|running|green|red|flaky|requires_investigation`. P2-2 EXTENDS this with
/// the three honest evidence states `Manual|Unknown|Skipped` that the spec's enum
/// could not express — the whole point of the collector is that "no check wired"
/// (`Unknown`) and "human-only check" (`Manual`) are first-class, distinct from a
/// proven `Green`. Spec backfill tracked via this DRIFT marker.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    Pending,
    Running,
    Green,
    Red,
    Flaky,
    RequiresInvestigation,
    /// Human-only check; carries an explicit marker, never auto-run, never auto-green.
    Manual,
    /// Mapped but no resolvable/runnable check and no manual marker — un-evidenced.
    Unknown,
    /// Entry-criteria not met for this trigger (e.g. release-only gate on a PR).
    Skipped,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Override {
    pub operator: String,
    pub reason: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ShipGate {
    pub gate_id: GateId,
    pub name: String,
    pub status: GateStatus,
    pub blocking_tests: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage_pct: Option<f64>,
    pub entry_criteria: Vec<String>,
    pub exit_criteria: Vec<String>,
    pub duration_seconds: u64,
    pub retry_count: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub override_by: Option<Override>,
    pub last_run: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Ios,
    Android,
    Macos,
    Windows,
    Linux,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TestStatus {
    Pass,
    Fail,
    Skip,
    Flaky,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TestRunResult {
    pub test_id: String,
    pub spec_id: String,
    pub gate_id: GateId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<Os>,
    pub status: TestStatus,
    pub duration_ms: u64,
    pub attempts: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reproducer_cmd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_log: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriggeredBy {
    PullRequest,
    TagPush,
    Schedule,
    WorkflowDispatch,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverallStatus {
    Green,
    RedWithOverride,
    Red,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ShipGateReportSummary {
    pub total_tests: u32,
    pub passed: u32,
    pub failed: u32,
    pub flaky: u32,
    pub skipped: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ShipGateReport {
    pub release_tag: String,
    pub commit_sha: String,
    pub triggered_by: TriggeredBy,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub overall_status: OverallStatus,
    pub gates: Vec<ShipGate>,
    pub test_results: Vec<TestRunResult>,
    pub summary: ShipGateReportSummary,
}

// ───────────────────────────────────────────────────────────────────────────
// Gate-map: DATA, not code. Deserialized from appendix/ship-gate-map.toml.
// ───────────────────────────────────────────────────────────────────────────

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct GateMap {
    #[serde(rename = "gate", default)]
    pub gates: Vec<GateSpec>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct GateSpec {
    pub id: GateId,
    pub name: String,
    #[serde(default)]
    pub entry_criteria: Vec<String>,
    #[serde(default)]
    pub exit_criteria: Vec<String>,
    /// Whether this gate is REQUIRED on a `pull_request` trigger (SPEC-60 §9.4:
    /// V1–V5 are PR-required; V6–V12 are release-only and `Skipped` on a PR).
    #[serde(default)]
    pub pr_required: bool,
    #[serde(default)]
    pub checks: Vec<Check>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Check {
    pub kind: CheckKind,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub spec: String,
    #[serde(default)]
    pub blocking_test_ids: Vec<String>,
    /// Required (non-empty) for `kind = "manual"`; explains why no automation exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_reason: Option<String>,
    /// Extra cargo features to enable for a `cargo_test` check whose target is
    /// gated behind a non-default feature (e.g. `experimental-memory`).
    /// Empty ⇒ default features. Honestly runs the REAL feature-gated test — not
    /// a fake-green: a wired feature test must still genuinely pass.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    CargoTest,
    Script,
    DocLint,
    Scenario,
    Manual,
}

// ───────────────────────────────────────────────────────────────────────────
// Resolver / classifier intermediate types.
// ───────────────────────────────────────────────────────────────────────────

/// Result of statically resolving a [`Check`] against the repo (no execution).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckResolution {
    /// The cited target exists as real code/script (runnable).
    Resolved,
    /// The cited target does not resolve — anti-fake-green: this is a hard Red.
    Unresolved(String),
    /// A `manual` marker — intentionally not runnable.
    Manual,
}

/// The honest outcome of one check after resolve (+ optional run).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// Ran and passed.
    Passed,
    /// Ran-and-failed, errored, or `unresolved-check`. Carries blocking test ids.
    Failed(Vec<String>),
    /// `manual` marker — never auto-green.
    Manual,
    /// Resolvable but not run, or no runnable check — un-evidenced.
    Unknown,
}

/// One check's full evidence record (outcome + any per-test results it produced).
#[derive(Debug, Clone)]
pub struct CheckRun {
    pub outcome: CheckOutcome,
    pub results: Vec<TestRunResult>,
}

/// Context for a collector run (which trigger we are evaluating for).
#[derive(Debug, Clone, Copy)]
pub struct RunContext {
    pub trigger: TriggeredBy,
}

#[derive(thiserror::Error, Debug)]
pub enum GateError {
    #[error("gate-map not found or unreadable: {0}")]
    Io(String),
    #[error("gate-map TOML parse error: {0}")]
    Parse(String),
    #[error("gate-map validation failed: {0}")]
    Validation(String),
}

// ───────────────────────────────────────────────────────────────────────────
// Public API (frozen in Task 1; bodies land in Tasks 2–6). Downstream tracks
// compile against these signatures immediately.
// ───────────────────────────────────────────────────────────────────────────

/// Load + validate the gate-map TOML. (Task 2)
pub fn load_gate_map(path: &Path) -> Result<GateMap, GateError> {
    let text = std::fs::read_to_string(path).map_err(|e| GateError::Io(format!("{path:?}: {e}")))?;
    parse_gate_map(&text)
}

/// Parse + validate a gate-map from a TOML string (split out for hermetic tests).
///
/// Validation: every `V1..V12` present exactly once; every non-`Manual` check has
/// a non-empty `target`; every `Manual` check has a non-empty `manual_reason`.
pub fn parse_gate_map(text: &str) -> Result<GateMap, GateError> {
    let map: GateMap = toml::from_str(text).map_err(|e| GateError::Parse(e.to_string()))?;

    // Each V1..V12 present exactly once.
    let mut seen: Vec<GateId> = Vec::new();
    for g in &map.gates {
        if seen.contains(&g.id) {
            return Err(GateError::Validation(format!(
                "gate {} declared more than once",
                g.id.as_str()
            )));
        }
        seen.push(g.id);
    }
    for want in GateId::all() {
        if !seen.contains(&want) {
            return Err(GateError::Validation(format!(
                "gate {} missing from gate-map",
                want.as_str()
            )));
        }
    }

    // Per-check structural rules (the contract that keeps `manual` honest).
    for g in &map.gates {
        for (i, c) in g.checks.iter().enumerate() {
            match c.kind {
                CheckKind::Manual => {
                    if c.manual_reason.as_deref().unwrap_or("").trim().is_empty() {
                        return Err(GateError::Validation(format!(
                            "{} check #{i} kind=manual has empty manual_reason",
                            g.id.as_str()
                        )));
                    }
                }
                _ => {
                    if c.target.trim().is_empty() {
                        return Err(GateError::Validation(format!(
                            "{} check #{i} kind={:?} has empty target",
                            g.id.as_str(),
                            c.kind
                        )));
                    }
                }
            }
        }
    }

    Ok(map)
}

/// Statically resolve a [`Check`] against the repo (no execution).
///
/// Mirrors the resolution rules of `scripts/check-test-citations.sh` so the two
/// can never silently disagree (pinned by the `T-testing-citations-agreement`
/// meta-test). The anti-fake-green rule: a non-manual check whose target does not
/// resolve to real code/scripts is [`CheckResolution::Unresolved`] (→ Red).
pub fn resolve_check(check: &Check, repo_root: &Path) -> CheckResolution {
    match check.kind {
        CheckKind::Manual => CheckResolution::Manual,
        CheckKind::CargoTest => {
            // gate-map cargo_test targets are bare `core/tests/<basename>` test files
            // (`--test` semantics). Delegate to the shared citation resolver.
            resolve_citation(&format!("--test {}", check.target.trim()), repo_root)
        }
        CheckKind::Script | CheckKind::DocLint => {
            let rel = check.target.trim();
            if rel.is_empty() {
                return CheckResolution::Unresolved("empty script target".into());
            }
            if repo_root.join(rel).is_file() {
                CheckResolution::Resolved
            } else {
                CheckResolution::Unresolved(format!("script not found: {rel}"))
            }
        }
        CheckKind::Scenario => {
            // target = "S<N>", N in 1..=40 (catalog contiguity proven separately).
            match scenario_number(check.target.trim()) {
                Some(n) if (1..=40).contains(&n) => CheckResolution::Resolved,
                _ => CheckResolution::Unresolved(format!(
                    "scenario id out of S1..S40 range: {}",
                    check.target
                )),
            }
        }
    }
}

fn scenario_number(s: &str) -> Option<u32> {
    let rest = s.strip_prefix('S').or_else(|| s.strip_prefix('s'))?;
    rest.parse::<u32>().ok()
}

/// Resolve a raw `cargo test` citation exactly like `check-test-citations.sh`:
/// `--test <name>` → `core/tests/<name>.rs`; `--lib a::b::<fn>` → a real `fn`;
/// `--lib <tok>` / bare `<tok>` → module basename OR exact fn OR fn-substring.
pub fn resolve_citation(citation: &str, repo_root: &Path) -> CheckResolution {
    let c = citation.trim();

    // `--test <name>` → core/tests/<name>.rs
    if let Some(rest) = c.strip_prefix("--test ") {
        let name = rest.split_whitespace().next().unwrap_or("");
        if name.is_empty() {
            return CheckResolution::Unresolved("empty --test target".into());
        }
        if repo_root.join("core/tests").join(format!("{name}.rs")).is_file() {
            return CheckResolution::Resolved;
        }
        return CheckResolution::Unresolved(format!("--test {name} → core/tests/{name}.rs missing"));
    }

    // `--lib <arg>` (or bare) token resolution.
    let arg = c.strip_prefix("--lib ").unwrap_or(c);
    let arg = arg.split_whitespace().next().unwrap_or("");
    if arg.is_empty() {
        return CheckResolution::Unresolved("empty citation".into());
    }

    let (fns, mods) = repo_index(repo_root);

    if let Some((_, last)) = arg.rsplit_once("::") {
        // `a::b::<fn>` — last segment must be a real fn (bash treats `::tests` as OK).
        if last == "tests" || fns.contains(last) {
            return CheckResolution::Resolved;
        }
        return CheckResolution::Unresolved(format!("cites ::{last} but no 'fn {last}' in core/"));
    }

    if mods.contains(arg) || fns.contains(arg) || fns.iter().any(|f| f.contains(arg)) {
        CheckResolution::Resolved
    } else {
        CheckResolution::Unresolved(format!("--lib {arg} → no module/fn/fn-substring in core/"))
    }
}

/// Walk core/src (+ core/tests for fns) → (`fn` name set, core/src module-basename set),
/// mirroring `check-test-citations.sh`'s FNFILE/MODFILE construction.
fn repo_index(repo_root: &Path) -> (std::collections::HashSet<String>, std::collections::HashSet<String>) {
    use std::collections::HashSet;
    let fn_re = regex::Regex::new(r"fn ([a-zA-Z0-9_]+)").unwrap();
    let mut fns: HashSet<String> = HashSet::new();
    let mut mods: HashSet<String> = HashSet::new();

    // fns: core/src + core/tests
    for sub in ["core/src", "core/tests"] {
        collect_rs(&repo_root.join(sub), &mut |path| {
            if let Ok(text) = std::fs::read_to_string(path) {
                for cap in fn_re.captures_iter(&text) {
                    fns.insert(cap[1].to_string());
                }
            }
        });
    }
    // mods: core/src basenames only
    collect_rs(&repo_root.join("core/src"), &mut |path| {
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            mods.insert(stem.to_string());
        }
    });

    (fns, mods)
}

fn collect_rs(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, f);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            f(&path);
        }
    }
}

/// `phantom test gate map --check` backing: collect every non-manual check whose
/// target is `Unresolved`. Empty result ⇒ the gate-map does not rot. (Used by CLI
/// in Task 9 + the `T-testing-gate-map-resolves` meta-test.)
pub fn lint_gate_map(map: &GateMap, repo_root: &Path) -> Vec<String> {
    let mut unresolved = Vec::new();
    for g in &map.gates {
        for (i, c) in g.checks.iter().enumerate() {
            if let CheckResolution::Unresolved(reason) = resolve_check(c, repo_root) {
                unresolved.push(format!(
                    "{} check #{i} (kind={:?}, target='{}'): {reason}",
                    g.id.as_str(),
                    c.kind,
                    c.target
                ));
            }
        }
    }
    unresolved
}

/// Combine one gate's check outcomes into a single [`ShipGate`] per the honesty
/// contract:
/// - release-only gate on a PR (`!pr_required` + `trigger == PullRequest`) ⇒ `Skipped`;
/// - `checks = []` (no runs) ⇒ `Unknown` (NEVER `Green`);
/// - any `Failed`/unresolved ⇒ `Red` (blocking_tests = the failing checks' ids);
/// - every check `Passed` ⇒ `Green`;
/// - else any `Manual` ⇒ `Manual`; otherwise ⇒ `Unknown`.
pub fn classify_gate(
    spec: &GateSpec,
    runs: &[CheckRun],
    ctx: &RunContext,
    now: DateTime<Utc>,
) -> ShipGate {
    let base = |status: GateStatus, blocking: Vec<String>| ShipGate {
        gate_id: spec.id,
        name: spec.name.clone(),
        status,
        blocking_tests: blocking,
        coverage_pct: None,
        entry_criteria: spec.entry_criteria.clone(),
        exit_criteria: spec.exit_criteria.clone(),
        duration_seconds: 0,
        retry_count: 0,
        override_by: None,
        last_run: now,
    };

    // Entry-criteria: a release-only gate evaluated on a PR is Skipped (SPEC-60 §9.4).
    if ctx.trigger == TriggeredBy::PullRequest && !spec.pr_required {
        return base(GateStatus::Skipped, vec![]);
    }
    // Mapped but no runnable check ⇒ Unknown (honest — never Green).
    if runs.is_empty() {
        return base(GateStatus::Unknown, vec![]);
    }

    let mut blocking: Vec<String> = Vec::new();
    let mut any_failed = false;
    let mut any_manual = false;
    let mut all_passed = true;
    for r in runs {
        match &r.outcome {
            CheckOutcome::Passed => {}
            CheckOutcome::Failed(b) => {
                any_failed = true;
                all_passed = false;
                for id in b {
                    if !blocking.contains(id) {
                        blocking.push(id.clone());
                    }
                }
            }
            CheckOutcome::Manual => {
                any_manual = true;
                all_passed = false;
            }
            CheckOutcome::Unknown => {
                all_passed = false;
            }
        }
    }

    let status = if any_failed {
        GateStatus::Red
    } else if all_passed {
        GateStatus::Green
    } else if any_manual {
        GateStatus::Manual
    } else {
        GateStatus::Unknown
    };
    base(status, if any_failed { blocking } else { vec![] })
}

/// True for the un-evidenced/blocking statuses (the ones that must not ship without
/// an operator override). `Skipped` is non-blocking (release-only gate on a PR).
fn is_blocking_status(s: GateStatus) -> bool {
    matches!(
        s,
        GateStatus::Red
            | GateStatus::Manual
            | GateStatus::Unknown
            | GateStatus::RequiresInvestigation
            | GateStatus::Pending
            | GateStatus::Running
            | GateStatus::Flaky
    )
}

/// Aggregate gates into a [`ShipGateReport`] + decide `overall_status` per the
/// honesty contract: `Green` iff every gate is `Green` (or `Skipped`, or a
/// non-green gate carries an `override_by`). Any unacknowledged blocking gate
/// (`Red`/`Manual`/`Unknown`) ⇒ `Red`. If the only blocking gates are
/// override-acknowledged ⇒ `RedWithOverride`. You cannot ship un-evidenced.
pub fn build_report(
    gates: Vec<ShipGate>,
    results: Vec<TestRunResult>,
    trigger: TriggeredBy,
    release_tag: &str,
    commit_sha: &str,
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
) -> ShipGateReport {
    let mut summary = ShipGateReportSummary {
        total_tests: results.len() as u32,
        passed: 0,
        failed: 0,
        flaky: 0,
        skipped: 0,
    };
    for r in &results {
        match r.status {
            TestStatus::Pass => summary.passed += 1,
            TestStatus::Fail => summary.failed += 1,
            TestStatus::Flaky => summary.flaky += 1,
            TestStatus::Skip => summary.skipped += 1,
        }
    }

    let mut unacked_blocking = false;
    let mut acked_blocking = false;
    let mut any_green = false;
    for g in &gates {
        if g.status == GateStatus::Green {
            any_green = true;
        }
        if is_blocking_status(g.status) {
            if g.override_by.is_some() {
                acked_blocking = true;
            } else {
                unacked_blocking = true;
            }
        }
    }
    let overall_status = if unacked_blocking {
        OverallStatus::Red
    } else if acked_blocking {
        OverallStatus::RedWithOverride
    } else if any_green {
        // Green requires real positive evidence — at least one gate actually Green.
        OverallStatus::Green
    } else {
        // No blocking gate AND no Green gate ⇒ every gate is Skipped (or the list is
        // empty): nothing was evidenced, so we cannot claim green (codex review,
        // 2026-06-17). Honest default: Red.
        OverallStatus::Red
    };

    ShipGateReport {
        release_tag: release_tag.to_string(),
        commit_sha: commit_sha.to_string(),
        triggered_by: trigger,
        started_at,
        finished_at,
        overall_status,
        gates,
        test_results: results,
        summary,
    }
}

fn status_label(s: GateStatus) -> &'static str {
    match s {
        GateStatus::Pending => "PENDING",
        GateStatus::Running => "RUNNING",
        GateStatus::Green => "GREEN",
        GateStatus::Red => "RED",
        GateStatus::Flaky => "FLAKY",
        GateStatus::RequiresInvestigation => "INVESTIGATE",
        GateStatus::Manual => "MANUAL",
        GateStatus::Unknown => "UNKNOWN",
        GateStatus::Skipped => "SKIPPED",
    }
}

/// Render the human `phantom test report` table (ASCII, OSS-safe — gate names +
/// repo-relative ids only, no host/IP/path leak).
pub fn render_table(report: &ShipGateReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "SPEC-60 ship-gate evidence — {} @{}  (trigger: {:?})\n",
        report.release_tag, report.commit_sha, report.triggered_by
    ));
    out.push_str("──────────────────────────────────────────────────────────────────────\n");
    out.push_str(&format!("{:<5} {:<12} {}\n", "gate", "status", "name / blocking"));
    out.push_str("──────────────────────────────────────────────────────────────────────\n");
    for g in &report.gates {
        let mut line = format!("{:<5} {:<12} {}", g.gate_id.as_str(), status_label(g.status), g.name);
        if !g.blocking_tests.is_empty() {
            line.push_str(&format!("  [blocking: {}]", g.blocking_tests.join(", ")));
        }
        if let Some(ov) = &g.override_by {
            line.push_str(&format!("  [override: {} — {}]", ov.operator, ov.reason));
        }
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("──────────────────────────────────────────────────────────────────────\n");
    out.push_str(&format!(
        "overall_status: {}\n",
        match report.overall_status {
            OverallStatus::Green => "GREEN",
            OverallStatus::RedWithOverride => "RED_WITH_OVERRIDE",
            OverallStatus::Red => "RED",
        }
    ));
    out.push_str(&format!(
        "tests: total={} passed={} failed={} flaky={} skipped={}\n",
        report.summary.total_tests,
        report.summary.passed,
        report.summary.failed,
        report.summary.flaky,
        report.summary.skipped
    ));
    out
}

/// End-to-end collector: for each gate in the map, run (or statically resolve) its
/// checks, classify, and aggregate into a [`ShipGateReport`]. `run_checks=false`
/// is the fast `--no-run` static path (Green is never produced — every resolvable
/// check is `Unknown` until actually run).
pub fn collect_report(
    map: &GateMap,
    repo_root: &Path,
    ctx: &RunContext,
    release_tag: &str,
    commit_sha: &str,
    now: DateTime<Utc>,
    run_checks: bool,
) -> ShipGateReport {
    let mut gates: Vec<ShipGate> = Vec::new();
    let mut all_results: Vec<TestRunResult> = Vec::new();

    // Evaluate in canonical V1..V12 order regardless of TOML ordering.
    for want in GateId::all() {
        let Some(spec) = map.gates.iter().find(|g| g.id == want) else { continue };
        let mut runs: Vec<CheckRun> = Vec::new();
        // Skip running entirely for a release-only gate on a PR (it will be Skipped).
        let skip_run = ctx.trigger == TriggeredBy::PullRequest && !spec.pr_required;
        if !skip_run {
            for check in &spec.checks {
                let run = if run_checks {
                    runner::run_check(check, spec.id, repo_root)
                } else {
                    runner::resolve_only(check, repo_root)
                };
                all_results.extend(run.results.clone());
                runs.push(run);
            }
        }
        gates.push(classify_gate(spec, &runs, ctx, now));
    }

    build_report(gates, all_results, ctx.trigger, release_tag, commit_sha, now, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_ts() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-06-17T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    /// RED-first honesty round-trip: a ShipGateReport serializes to camelCase JSON,
    /// deserializes back, and is structurally equal. Locks the wire contract.
    #[test]
    fn ship_gate_report_serde_round_trips_camelcase() {
        let report = ShipGateReport {
            release_tag: "v0.6.0-rc1".into(),
            commit_sha: "50ca57b2".into(),
            triggered_by: TriggeredBy::WorkflowDispatch,
            started_at: fixed_ts(),
            finished_at: fixed_ts(),
            overall_status: OverallStatus::Red,
            gates: vec![ShipGate {
                gate_id: GateId::V8,
                name: "security".into(),
                status: GateStatus::Green,
                blocking_tests: vec!["T-oauth-es256".into()],
                coverage_pct: None,
                entry_criteria: vec!["V1 green".into()],
                exit_criteria: vec!["no PII leak".into()],
                duration_seconds: 42,
                retry_count: 0,
                override_by: None,
                last_run: fixed_ts(),
            }],
            test_results: vec![TestRunResult {
                test_id: "T-oauth-es256".into(),
                spec_id: "SPEC-08".into(),
                gate_id: GateId::V8,
                os: None,
                status: TestStatus::Pass,
                duration_ms: 1200,
                attempts: 1,
                reproducer_cmd: Some("cargo test --test oauth_es256_regression".into()),
                failure_log: None,
                timestamp: fixed_ts(),
            }],
            summary: ShipGateReportSummary {
                total_tests: 1,
                passed: 1,
                failed: 0,
                flaky: 0,
                skipped: 0,
            },
        };

        let json = serde_json::to_string(&report).expect("serialize");
        // camelCase keys + lowercase/snake_case enum tags on the wire.
        assert!(json.contains("\"gateId\":\"V8\""), "gateId camelCase: {json}");
        assert!(json.contains("\"blockingTests\":"), "blockingTests camelCase");
        assert!(json.contains("\"overallStatus\":\"red\""), "overallStatus snake_case");
        assert!(json.contains("\"totalTests\":1"), "summary camelCase");
        assert!(json.contains("\"specId\":\"SPEC-08\""), "specId camelCase");
        // `coveragePct`/`overrideBy` are None ⇒ omitted.
        assert!(!json.contains("coveragePct"), "None coveragePct omitted");

        let back: ShipGateReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.gates.len(), 1);
        assert_eq!(back.gates[0].gate_id, GateId::V8);
        assert_eq!(back.gates[0].status, GateStatus::Green);
        assert_eq!(back.overall_status, OverallStatus::Red);
        assert_eq!(back.summary.total_tests, 1);
        assert_eq!(back.test_results[0].status, TestStatus::Pass);
        // round-trip JSON is byte-identical (no field reordering surprises).
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    fn repo_root() -> &'static Path {
        // cargo runs lib tests with CWD/manifest at core/ → repo root is its parent.
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
    }

    fn full_map_toml(extra_checks_v8: &str) -> String {
        let mut s = String::new();
        for g in GateId::all() {
            s.push_str(&format!(
                "[[gate]]\nid = \"{}\"\nname = \"gate {}\"\nentry_criteria = []\nexit_criteria = []\n",
                g.as_str(),
                g.as_str()
            ));
            if g == GateId::V8 {
                s.push_str(extra_checks_v8);
            } else {
                s.push_str("checks = []\n");
            }
            s.push('\n');
        }
        s
    }

    /// RED-first (Task 2): the resolver returns Resolved / Unresolved / Manual for a
    /// real test, a bogus target, and a manual row — the anti-fake-green core.
    #[test]
    fn resolve_check_resolved_unresolved_manual() {
        let root = repo_root();

        let good = Check {
            kind: CheckKind::CargoTest,
            target: "coach_shame_free_fixture".into(), // real core/tests/*.rs
            spec: "SPEC-23".into(),
            blocking_test_ids: vec![],
            manual_reason: None,
            features: vec![],
        };
        assert_eq!(resolve_check(&good, root), CheckResolution::Resolved);

        let bogus = Check {
            kind: CheckKind::CargoTest,
            target: "no_such_test".into(),
            spec: "SPEC-99".into(),
            blocking_test_ids: vec![],
            manual_reason: None,
            features: vec![],
        };
        assert!(matches!(resolve_check(&bogus, root), CheckResolution::Unresolved(_)));

        let manual = Check {
            kind: CheckKind::Manual,
            target: String::new(),
            spec: "SPEC-29".into(),
            blocking_test_ids: vec![],
            manual_reason: Some("iOS sign is CI-only".into()),
            features: vec![],
        };
        assert_eq!(resolve_check(&manual, root), CheckResolution::Manual);
    }

    #[test]
    fn resolve_check_script_doclint_scenario() {
        let root = repo_root();
        let script_ok = Check { kind: CheckKind::Script, target: "scripts/check-test-citations.sh".into(), spec: "".into(), blocking_test_ids: vec![], manual_reason: None, features: vec![] };
        assert_eq!(resolve_check(&script_ok, root), CheckResolution::Resolved);

        let doc_ok = Check { kind: CheckKind::DocLint, target: "scripts/check-doc-tree.ps1".into(), spec: "".into(), blocking_test_ids: vec![], manual_reason: None, features: vec![] };
        assert_eq!(resolve_check(&doc_ok, root), CheckResolution::Resolved);

        let script_bad = Check { kind: CheckKind::Script, target: "scripts/does-not-exist.sh".into(), spec: "".into(), blocking_test_ids: vec![], manual_reason: None, features: vec![] };
        assert!(matches!(resolve_check(&script_bad, root), CheckResolution::Unresolved(_)));

        let scen_ok = Check { kind: CheckKind::Scenario, target: "S1".into(), spec: "".into(), blocking_test_ids: vec![], manual_reason: None, features: vec![] };
        assert_eq!(resolve_check(&scen_ok, root), CheckResolution::Resolved);
        let scen_bad = Check { kind: CheckKind::Scenario, target: "S99".into(), spec: "".into(), blocking_test_ids: vec![], manual_reason: None, features: vec![] };
        assert!(matches!(resolve_check(&scen_bad, root), CheckResolution::Unresolved(_)));
    }

    #[test]
    fn parse_gate_map_accepts_valid_and_rejects_invalid() {
        // valid: 12 gates, no checks.
        let ok = full_map_toml("checks = []\n");
        let map = parse_gate_map(&ok).expect("valid map parses");
        assert_eq!(map.gates.len(), 12);

        // missing a gate → Validation.
        let missing = ok.replace("id = \"V12\"", "id = \"V11\"");
        assert!(matches!(parse_gate_map(&missing), Err(GateError::Validation(_))));

        // manual check with empty reason → Validation.
        let manual_no_reason = full_map_toml(
            "[[gate.checks]]\nkind = \"manual\"\nspec = \"SPEC-29\"\n",
        );
        assert!(matches!(parse_gate_map(&manual_no_reason), Err(GateError::Validation(_))));

        // non-manual check with empty target → Validation.
        let cargo_no_target = full_map_toml(
            "[[gate.checks]]\nkind = \"cargo_test\"\nspec = \"SPEC-08\"\n",
        );
        assert!(matches!(parse_gate_map(&cargo_no_target), Err(GateError::Validation(_))));
    }

    #[test]
    fn lint_gate_map_flags_unresolved_nonmanual() {
        let toml = full_map_toml(
            "[[gate.checks]]\nkind = \"cargo_test\"\ntarget = \"no_such_test\"\nspec = \"SPEC-08\"\n",
        );
        let map = parse_gate_map(&toml).unwrap();
        let unresolved = lint_gate_map(&map, repo_root());
        assert_eq!(unresolved.len(), 1, "exactly the bogus V8 check is flagged");
        assert!(unresolved[0].contains("V8"));
    }

    // ── Task 5: classifier ───────────────────────────────────────────────────

    fn gspec(id: GateId, pr_required: bool) -> GateSpec {
        GateSpec {
            id,
            name: format!("gate {}", id.as_str()),
            entry_criteria: vec![],
            exit_criteria: vec![],
            pr_required,
            checks: vec![],
        }
    }
    fn passed() -> CheckRun { CheckRun { outcome: CheckOutcome::Passed, results: vec![] } }
    fn failed(ids: &[&str]) -> CheckRun {
        CheckRun { outcome: CheckOutcome::Failed(ids.iter().map(|s| s.to_string()).collect()), results: vec![] }
    }
    fn manual() -> CheckRun { CheckRun { outcome: CheckOutcome::Manual, results: vec![] } }
    fn unknown() -> CheckRun { CheckRun { outcome: CheckOutcome::Unknown, results: vec![] } }
    fn full_ctx() -> RunContext { RunContext { trigger: TriggeredBy::WorkflowDispatch } }

    /// THE load-bearing test: a gate with checks=[] is Unknown, NEVER Green.
    #[test]
    fn classify_empty_checks_is_unknown_not_green() {
        let g = classify_gate(&gspec(GateId::V6, false), &[], &full_ctx(), fixed_ts());
        assert_eq!(g.status, GateStatus::Unknown);
        assert_ne!(g.status, GateStatus::Green);
    }

    #[test]
    fn classify_all_passed_is_green() {
        let g = classify_gate(&gspec(GateId::V2, true), &[passed(), passed()], &full_ctx(), fixed_ts());
        assert_eq!(g.status, GateStatus::Green);
        assert!(g.blocking_tests.is_empty());
    }

    #[test]
    fn classify_any_failed_is_red_with_blocking() {
        let g = classify_gate(
            &gspec(GateId::V8, false),
            &[passed(), failed(&["T-oauth-es256"]), manual()],
            &full_ctx(),
            fixed_ts(),
        );
        assert_eq!(g.status, GateStatus::Red);
        assert_eq!(g.blocking_tests, vec!["T-oauth-es256".to_string()]);
    }

    #[test]
    fn classify_passed_plus_manual_is_manual() {
        let g = classify_gate(&gspec(GateId::V5, false), &[passed(), manual()], &full_ctx(), fixed_ts());
        assert_eq!(g.status, GateStatus::Manual);
    }

    #[test]
    fn classify_passed_plus_unknown_is_unknown() {
        let g = classify_gate(&gspec(GateId::V3, true), &[passed(), unknown()], &full_ctx(), fixed_ts());
        assert_eq!(g.status, GateStatus::Unknown);
    }

    #[test]
    fn classify_release_only_gate_on_pr_is_skipped() {
        let ctx = RunContext { trigger: TriggeredBy::PullRequest };
        // V6 is release-only (pr_required=false): even with a failing check it is Skipped on a PR.
        let g = classify_gate(&gspec(GateId::V6, false), &[failed(&["x"])], &ctx, fixed_ts());
        assert_eq!(g.status, GateStatus::Skipped);
        // a PR-required gate (V2) on a PR is NOT skipped — empty ⇒ Unknown.
        let g2 = classify_gate(&gspec(GateId::V2, true), &[], &ctx, fixed_ts());
        assert_eq!(g2.status, GateStatus::Unknown);
    }

    // ── Task 6: report aggregator + overall_status ──────────────────────────

    fn sg(id: GateId, status: GateStatus, ov: Option<Override>) -> ShipGate {
        ShipGate {
            gate_id: id,
            name: format!("gate {}", id.as_str()),
            status,
            blocking_tests: vec![],
            coverage_pct: None,
            entry_criteria: vec![],
            exit_criteria: vec![],
            duration_seconds: 0,
            retry_count: 0,
            override_by: ov,
            last_run: fixed_ts(),
        }
    }
    fn an_override() -> Override {
        Override { operator: "operator".into(), reason: "accepted risk".into(), timestamp: fixed_ts() }
    }
    fn report_of(gates: Vec<ShipGate>) -> ShipGateReport {
        build_report(gates, vec![], TriggeredBy::WorkflowDispatch, "v0.6.0", "50ca57b2", fixed_ts(), fixed_ts())
    }

    #[test]
    fn overall_red_when_unknown_gate_unacknowledged() {
        let r = report_of(vec![
            sg(GateId::V2, GateStatus::Green, None),
            sg(GateId::V6, GateStatus::Unknown, None),
        ]);
        assert_eq!(r.overall_status, OverallStatus::Red);
    }

    #[test]
    fn overall_green_only_when_all_green_or_skipped_or_overridden() {
        // all green + a skipped (release-only on PR) ⇒ Green.
        let r = report_of(vec![
            sg(GateId::V1, GateStatus::Green, None),
            sg(GateId::V6, GateStatus::Skipped, None),
        ]);
        assert_eq!(r.overall_status, OverallStatus::Green);
        // a single unacknowledged manual ⇒ NOT green.
        let r2 = report_of(vec![
            sg(GateId::V1, GateStatus::Green, None),
            sg(GateId::V7, GateStatus::Manual, None),
        ]);
        assert_ne!(r2.overall_status, OverallStatus::Green);
    }

    #[test]
    fn override_flips_red_to_red_with_override() {
        let red_acked = report_of(vec![
            sg(GateId::V2, GateStatus::Green, None),
            sg(GateId::V6, GateStatus::Red, Some(an_override())),
        ]);
        assert_eq!(red_acked.overall_status, OverallStatus::RedWithOverride);

        let red_unacked = report_of(vec![
            sg(GateId::V2, GateStatus::Green, None),
            sg(GateId::V6, GateStatus::Red, None),
        ]);
        assert_eq!(red_unacked.overall_status, OverallStatus::Red);
    }

    /// Hardening (codex review 2026-06-17): all-Skipped (or empty) ⇒ NOT Green —
    /// Green needs ≥1 genuinely-Green gate (real positive evidence).
    #[test]
    fn overall_all_skipped_or_empty_is_not_green() {
        let all_skipped = report_of(vec![
            sg(GateId::V6, GateStatus::Skipped, None),
            sg(GateId::V7, GateStatus::Skipped, None),
        ]);
        assert_ne!(all_skipped.overall_status, OverallStatus::Green);
        assert_eq!(all_skipped.overall_status, OverallStatus::Red);

        let empty = report_of(vec![]);
        assert_ne!(empty.overall_status, OverallStatus::Green);
    }

    /// An override on a DIFFERENT gate must NOT flip a still-red gate (red stays red).
    #[test]
    fn red_stays_red_when_override_is_on_a_different_gate() {
        let r = report_of(vec![
            sg(GateId::V8, GateStatus::Red, None),            // un-overridden red
            sg(GateId::V6, GateStatus::Unknown, Some(an_override())), // override on another gate
        ]);
        assert_eq!(r.overall_status, OverallStatus::Red);
    }

    #[test]
    fn render_table_is_readable_and_ossafe() {
        let r = report_of(vec![
            sg(GateId::V2, GateStatus::Green, None),
            sg(GateId::V6, GateStatus::Unknown, None),
        ]);
        let table = render_table(&r);
        assert!(table.contains("V2"));
        assert!(table.contains("UNKNOWN"));
        assert!(table.contains("overall_status: RED"));
        // OSS-safe: no absolute Windows/Unix path leaks into the table.
        assert!(!table.contains(":\\Users\\"), "abs path leaked: {table}");
    }

    // ── Task 8: the SHIPPED gate-map resolves ────────────────────────────────

    /// The real appendix/ship-gate-map.toml parses (12 gates) and every non-manual
    /// check resolves to real code/scripts (the `gate map --check` contract).
    #[test]
    fn shipped_gate_map_parses_and_resolves() {
        let root = repo_root();
        let path = root.join("docs/superpowers/specs/v060-deep-spec/appendix/ship-gate-map.toml");
        let map = load_gate_map(&path).expect("shipped gate-map parses + validates");
        assert_eq!(map.gates.len(), 12, "all 12 gates present");
        let unresolved = lint_gate_map(&map, root);
        assert!(unresolved.is_empty(), "unresolved checks in shipped map: {unresolved:#?}");
    }

    /// The honest 5-state extension serializes to the expected wire tags.
    #[test]
    fn gate_status_honest_states_serialize() {
        assert_eq!(serde_json::to_string(&GateStatus::Unknown).unwrap(), "\"unknown\"");
        assert_eq!(serde_json::to_string(&GateStatus::Manual).unwrap(), "\"manual\"");
        assert_eq!(serde_json::to_string(&GateStatus::Skipped).unwrap(), "\"skipped\"");
        assert_eq!(
            serde_json::to_string(&GateStatus::RequiresInvestigation).unwrap(),
            "\"requires_investigation\""
        );
    }
}
