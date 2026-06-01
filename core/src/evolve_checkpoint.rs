//! Evolve Checkpoint — stateful, mesh-aware self-improvement journal.
//!
//! Inspired by jcode's `ReloadContext` (which preserved a one-line
//! `task_context` string across binary swaps) — this version extends
//! the idea along several axes that matter for phantom-mesh:
//!
//! 1. **Rich structure.** Stores not just "what the agent was working on"
//!    but the full causal chain: plan → hypotheses → dead-ends →
//!    completed steps → artifacts → binary swaps. Future-you can audit
//!    autonomous work step-by-step.
//!
//! 2. **Mesh-aware.** Each checkpoint records `origin_node`, `current_node`,
//!    and a `journey` of cross-machine hops. An evolve task that gets stuck
//!    on one peer (e.g., quota exhausted) can hand off to another peer with
//!    the checkpoint in hand — that peer reads it, continues from where the
//!    first peer left off.
//!    jcode is single-machine; this is what phantom-mesh's mesh enables.
//!
//! 3. **Git-integrated.** When autoevolve commits a green fix, the commit
//!    message embeds the plan + dead-ends + binary swap history. `git log`
//!    on the repo becomes a readable record of every autonomous decision.
//!
//! 4. **Atomic persistence.** Each update writes to a `.tmp` file and
//!    renames atomically — no half-written checkpoint state on crash.
//!
//! Storage: `~/.phantom-mesh/evolve-checkpoints/<session-id>.json`.
//! Schema is `serde_json` for cross-version readability and easy debug.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Where evolve checkpoints live, keyed by session_id.
pub fn checkpoints_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("home dir not resolved")?;
    let dir = home.join(".phantom-mesh/evolve-checkpoints");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// One discrete moment in the evolve agent's reasoning loop.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum EvolvePhase {
    /// Just started — agent is reading files, building mental model.
    Discovering,
    /// Agent has formed a hypothesis about the root cause.
    Hypothesizing,
    /// Agent is applying edits.
    Editing,
    /// Agent is running cargo check / tests to verify the fix.
    Verifying,
    /// Build green — agent is creating the commit.
    Committing,
    /// Terminal state.
    Done { outcome: EvolveOutcome },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum EvolveOutcome {
    /// Cargo target restored to green and changes committed.
    Success { commit_sha: String, rounds: u32 },
    /// Agent ran out of rounds / providers / ideas.
    Stuck {
        reason: String,
        last_error: Option<String>,
    },
    /// Agent handed off to another mesh node.
    Migrated { to_node: String, reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletedStep {
    pub at_ms: i64,
    pub description: String,
    pub tool: Option<String>,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeadEnd {
    pub at_ms: i64,
    /// What the agent thought might fix it.
    pub hypothesis: String,
    /// Concrete reason the hypothesis didn't pan out.
    pub why_failed: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtifactKind {
    Patch { path: String },
    NewFile { path: String },
    ModifiedFile { path: String },
    Commit { sha: String, subject: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Artifact {
    pub at_ms: i64,
    #[serde(flatten)]
    pub kind: ArtifactKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BinarySwap {
    pub at_ms: i64,
    pub version_before: String,
    pub version_after: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeHop {
    pub at_ms: i64,
    pub from: String,
    pub to: String,
    pub reason: String,
}

// ─── H1: judge verdict (append-only, serde-defaulted for back-compat) ────
//
// Added by track [H1]. Per spec §6.2, this is an append-only edit to
// EvolveCheckpoint — the field is `Option<JudgeVerdict>` with
// `#[serde(default)]` so checkpoint files written by a pre-H1 binary
// continue to deserialize without migration.

/// V2 (T28): how much the ensemble's judges agree on their score.
/// Drives whether the ensemble verdict is trustworthy or whether a human
/// should review the agent's session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgreementClass {
    /// All succeeded judges returned the same score (population stddev == 0).
    Unanimous,
    /// Population stddev is in (0.0, 2.0] on the 0..10 score scale.
    Consensus,
    /// Population stddev > 2.0, OR fewer than 2 judges succeeded.
    NeedsHumanReview,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JudgeVerdict {
    /// Numeric score on the rubric (current rubric h1-v1 = 0..10 inclusive).
    pub score: u8,
    /// Frozen rubric identifier; lets future judges score against the same
    /// rubric version their predecessors used.
    pub rubric_version: String,
    /// The judging LLM (canonical model id, not alias).
    pub model: String,
    /// Plain-text justification the LLM produced; bounded length when
    /// stored — see Curator::judge() for truncation policy.
    pub rationale: String,
    /// Wall-clock millis when the verdict was written.
    pub judged_at_ms: i64,
}

/// V2 (T28): aggregated multi-judge verdict. Persisted onto an EvolveCheckpoint
/// alongside (NOT replacing) the V1 `judge_score` field. Append-only:
/// `#[serde(default)]` keeps old checkpoint files readable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EnsembleVerdict {
    /// The verdict used by downstream consumers (commit trailer, replay UI).
    /// score = median across succeeded judges. model = "ensemble:<N>".
    /// rubric_version = whatever the underlying judges use (currently "h1-v1").
    pub aggregated: JudgeVerdict,
    /// One JudgeVerdict per succeeded judge, in dispatch order.
    pub individual: Vec<JudgeVerdict>,
    pub agreement: AgreementClass,
    pub score_median: f32,
    pub score_stddev: f32,
    pub judges_attempted: u8,
    pub judges_succeeded: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvolveCheckpoint {
    /// Stable session id, suitable as a filename. Auto-generated by `new()`.
    pub session_id: String,

    pub started_at_ms: i64,
    pub last_updated_ms: i64,

    /// Initial goal text passed to autoevolve (e.g., "Restore cargo check green").
    pub goal: String,

    pub phase: EvolvePhase,

    /// Cargo target the evolve loop is gating on (`check` / `test`).
    pub target: String,

    /// LLM-stated multi-step plan, populated as the agent reasons.
    /// Newer steps appended to the end.
    pub plan: Vec<String>,

    /// Each meaningful action the agent has taken so far.
    pub completed_steps: Vec<CompletedStep>,

    /// What the agent currently thinks the root cause is.
    pub current_hypothesis: Option<String>,

    /// Hypotheses that were tried and rejected. Useful for
    /// (a) preventing the agent from re-trying them after restart,
    /// (b) human auditors reviewing decisions.
    pub dead_ends: Vec<DeadEnd>,

    /// Files modified, commits created, etc.
    pub artifacts: Vec<Artifact>,

    /// History of binary swaps during the evolve loop. If the agent
    /// rebuilt phantom mid-task and the new binary picked up the
    /// checkpoint, each swap is recorded here.
    pub binary_swaps: Vec<BinarySwap>,

    /// Mesh-awareness: the node this evolve was first started on.
    pub origin_node: String,

    /// Mesh-awareness: the node currently working on it (may differ
    /// after a handoff).
    pub current_node: String,

    /// Trail of node-to-node migrations.
    pub journey: Vec<NodeHop>,

    /// H1: optional verdict from `phantom evolve --judge`. `None` until a
    /// judge run produces a score. `#[serde(default)]` so old checkpoint
    /// files (written before H1 shipped) continue to deserialize.
    #[serde(default)]
    pub judge_score: Option<JudgeVerdict>,

    /// V2 (T28): optional ensemble verdict from `phantom evolve --judge --ensemble N`.
    /// `None` for single-judge runs or pre-V2 checkpoints. Independent of
    /// `judge_score` — both may coexist.
    #[serde(default)]
    pub judge_ensemble: Option<EnsembleVerdict>,
}

impl EvolveCheckpoint {
    /// Start a new checkpoint with a generated session id.
    pub fn new(
        goal: impl Into<String>,
        target: impl Into<String>,
        node: impl Into<String>,
    ) -> Self {
        // Append a process-monotonic counter so back-to-back `new()` calls
        // (which can land in the same millisecond) get distinct session_ids.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let now = now_ms();
        let node_str = node.into();
        let session_id = format!("evolve-{}-{}-{}", now, n, &node_str.replace('/', "_"));
        Self {
            session_id,
            started_at_ms: now,
            last_updated_ms: now,
            goal: goal.into(),
            phase: EvolvePhase::Discovering,
            target: target.into(),
            plan: vec![],
            completed_steps: vec![],
            current_hypothesis: None,
            dead_ends: vec![],
            artifacts: vec![],
            binary_swaps: vec![],
            origin_node: node_str.clone(),
            current_node: node_str,
            journey: vec![],
            judge_score: None,
            judge_ensemble: None,
        }
    }

    pub fn path_for(session_id: &str) -> Result<PathBuf> {
        Ok(checkpoints_dir()?.join(format!("{}.json", session_id)))
    }

    /// Atomic write. Serializes to a `.tmp` sibling and renames into place,
    /// so a crash mid-write leaves the previous checkpoint intact.
    pub fn save(&self) -> Result<()> {
        let path = Self::path_for(&self.session_id)?;
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn load(session_id: &str) -> Result<Option<Self>> {
        let path = Self::path_for(session_id)?;
        if !path.exists() {
            return Ok(None);
        }
        let body = std::fs::read_to_string(&path)?;
        Ok(Some(serde_json::from_str(&body)?))
    }

    /// List checkpoints sorted newest-first, optionally filtering to
    /// non-terminal ones (i.e., excluding `Done`).
    pub fn list_all(active_only: bool) -> Result<Vec<Self>> {
        let dir = checkpoints_dir()?;
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir)?.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let body = match std::fs::read_to_string(&p) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let ck: Self = match serde_json::from_str(&body) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if active_only && matches!(ck.phase, EvolvePhase::Done { .. }) {
                continue;
            }
            out.push(ck);
        }
        out.sort_by(|a, b| b.last_updated_ms.cmp(&a.last_updated_ms));
        Ok(out)
    }

    pub fn delete(session_id: &str) -> Result<()> {
        let path = Self::path_for(session_id)?;
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    // ─── mutation helpers ────────────────────────────────────────────────

    pub fn touch(&mut self) {
        self.last_updated_ms = now_ms();
    }

    pub fn set_phase(&mut self, phase: EvolvePhase) {
        self.phase = phase;
        self.touch();
    }

    pub fn append_step(
        &mut self,
        description: impl Into<String>,
        tool: Option<String>,
        success: bool,
    ) {
        self.completed_steps.push(CompletedStep {
            at_ms: now_ms(),
            description: description.into(),
            tool,
            success,
        });
        self.touch();
    }

    pub fn record_dead_end(
        &mut self,
        hypothesis: impl Into<String>,
        why_failed: impl Into<String>,
    ) {
        self.dead_ends.push(DeadEnd {
            at_ms: now_ms(),
            hypothesis: hypothesis.into(),
            why_failed: why_failed.into(),
        });
        self.touch();
    }

    pub fn record_artifact(&mut self, kind: ArtifactKind) {
        self.artifacts.push(Artifact {
            at_ms: now_ms(),
            kind,
        });
        self.touch();
    }

    pub fn record_binary_swap(
        &mut self,
        version_before: impl Into<String>,
        version_after: impl Into<String>,
        reason: impl Into<String>,
    ) {
        self.binary_swaps.push(BinarySwap {
            at_ms: now_ms(),
            version_before: version_before.into(),
            version_after: version_after.into(),
            reason: reason.into(),
        });
        self.touch();
    }

    pub fn record_node_hop(&mut self, to: impl Into<String>, reason: impl Into<String>) {
        let to_str = to.into();
        let from = std::mem::replace(&mut self.current_node, to_str.clone());
        self.journey.push(NodeHop {
            at_ms: now_ms(),
            from,
            to: to_str,
            reason: reason.into(),
        });
        self.touch();
    }

    /// H1: persist a judge verdict on this checkpoint and touch the
    /// last-updated timestamp. Caller (Curator::judge()) owns the LLM call;
    /// this method is pure-data write.
    pub fn record_judge_verdict(&mut self, verdict: JudgeVerdict) {
        self.judge_score = Some(verdict);
        self.touch();
    }

    /// V2 (T28): persist an ensemble verdict. Idempotent w.r.t. the V1
    /// single-judge verdict — both fields are independent and may coexist
    /// on one checkpoint.
    pub fn record_ensemble_verdict(&mut self, verdict: EnsembleVerdict) {
        self.judge_ensemble = Some(verdict);
        self.touch();
    }

    // ─── rendering ───────────────────────────────────────────────────────

    /// Human-readable markdown timeline. Used by `phantom evolve replay`
    /// and embedded in autoevolve commit messages so `git log` becomes a
    /// readable record of autonomous decisions.
    pub fn render_markdown(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("# evolve session: {}\n\n", self.session_id));
        s.push_str(&format!("- **goal**: {}\n", self.goal));
        s.push_str(&format!("- **target**: cargo {}\n", self.target));
        s.push_str(&format!("- **phase**: {}\n", phase_short(&self.phase)));
        s.push_str(&format!("- **started**: {}\n", fmt_ts(self.started_at_ms)));
        s.push_str(&format!(
            "- **updated**: {}\n",
            fmt_ts(self.last_updated_ms)
        ));
        s.push_str(&format!("- **origin node**: {}\n", self.origin_node));
        s.push_str(&format!("- **current node**: {}\n", self.current_node));
        s.push('\n');

        if !self.plan.is_empty() {
            s.push_str("## plan\n");
            for (i, step) in self.plan.iter().enumerate() {
                s.push_str(&format!("{}. {}\n", i + 1, step));
            }
            s.push('\n');
        }

        if let Some(h) = &self.current_hypothesis {
            s.push_str(&format!("## current hypothesis\n{}\n\n", h));
        }

        if !self.completed_steps.is_empty() {
            s.push_str("## timeline\n");
            for st in &self.completed_steps {
                let mark = if st.success { "✓" } else { "✗" };
                let tool = st
                    .tool
                    .as_deref()
                    .map(|t| format!(" [{}]", t))
                    .unwrap_or_default();
                s.push_str(&format!(
                    "- `{}` {} {}{}\n",
                    fmt_ts(st.at_ms),
                    mark,
                    st.description,
                    tool
                ));
            }
            s.push('\n');
        }

        if !self.dead_ends.is_empty() {
            s.push_str("## dead ends (won't retry on resume)\n");
            for d in &self.dead_ends {
                s.push_str(&format!(
                    "- **{}** — {} ({})\n",
                    d.hypothesis,
                    d.why_failed,
                    fmt_ts(d.at_ms)
                ));
            }
            s.push('\n');
        }

        if !self.artifacts.is_empty() {
            s.push_str("## artifacts\n");
            for a in &self.artifacts {
                let line = match &a.kind {
                    ArtifactKind::Patch { path } => format!("patch @ {}", path),
                    ArtifactKind::NewFile { path } => format!("new file @ {}", path),
                    ArtifactKind::ModifiedFile { path } => format!("modified {}", path),
                    ArtifactKind::Commit { sha, subject } => {
                        format!("commit `{}` — {}", sha, subject)
                    }
                };
                s.push_str(&format!("- `{}` {}\n", fmt_ts(a.at_ms), line));
            }
            s.push('\n');
        }

        if !self.binary_swaps.is_empty() {
            s.push_str("## binary swaps (agent rebuilt itself)\n");
            for sw in &self.binary_swaps {
                s.push_str(&format!(
                    "- `{}` {} → {}: {}\n",
                    fmt_ts(sw.at_ms),
                    sw.version_before,
                    sw.version_after,
                    sw.reason
                ));
            }
            s.push('\n');
        }

        if !self.journey.is_empty() {
            s.push_str("## node journey (mesh handoffs)\n");
            for hop in &self.journey {
                s.push_str(&format!(
                    "- `{}` {} → {}: {}\n",
                    fmt_ts(hop.at_ms),
                    hop.from,
                    hop.to,
                    hop.reason
                ));
            }
            s.push('\n');
        }

        if let EvolvePhase::Done { outcome } = &self.phase {
            s.push_str("## outcome\n");
            match outcome {
                EvolveOutcome::Success { commit_sha, rounds } => {
                    s.push_str(&format!(
                        "✓ green after {} round(s); commit `{}`\n",
                        rounds, commit_sha
                    ));
                }
                EvolveOutcome::Stuck { reason, last_error } => {
                    s.push_str(&format!("✗ stuck — {}\n", reason));
                    if let Some(e) = last_error {
                        s.push_str(&format!("  last error: {}\n", e));
                    }
                }
                EvolveOutcome::Migrated { to_node, reason } => {
                    s.push_str(&format!("→ migrated to {} ({})\n", to_node, reason));
                }
            }
        }

        if let Some(v) = &self.judge_score {
            s.push_str("## judge verdict\n");
            s.push_str(&format!(
                "- **score**: {}/10 (rubric {})\n",
                v.score, v.rubric_version
            ));
            s.push_str(&format!("- **model**: {}\n", v.model));
            s.push_str(&format!("- **judged at**: {}\n", fmt_ts(v.judged_at_ms)));
            s.push_str(&format!("- **rationale**: {}\n", v.rationale));
            s.push('\n');
        }

        if let Some(e) = &self.judge_ensemble {
            s.push_str("## ensemble verdict\n");
            s.push_str(&format!(
                "- **{}/{} judges** succeeded; agreement = {}\n",
                e.judges_succeeded,
                e.judges_attempted,
                agreement_short(&e.agreement),
            ));
            s.push_str(&format!(
                "- **median {}** (stddev {:.2}); rubric {}\n",
                e.aggregated.score, e.score_stddev, e.aggregated.rubric_version,
            ));
            for v in &e.individual {
                s.push_str(&format!(
                    "  - {} → {}/10: {}\n",
                    v.model, v.score, v.rationale
                ));
            }
            s.push('\n');
        }
        s
    }

    /// Compact one-line summary for `phantom evolve list`.
    pub fn render_one_line(&self) -> String {
        format!(
            "{}  {}  steps={}  dead-ends={}  swaps={}  hops={}  goal=\"{}\"",
            self.session_id,
            phase_short(&self.phase),
            self.completed_steps.len(),
            self.dead_ends.len(),
            self.binary_swaps.len(),
            self.journey.len(),
            truncate(&self.goal, 60),
        )
    }

    /// What the autoevolve loop should embed in its git commit message
    /// so `git log` carries the agent's full reasoning. Returns lines that
    /// belong AFTER the commit subject + blank line.
    pub fn render_commit_trailer(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("evolve-session: {}\n", self.session_id));
        s.push_str(&format!("evolve-target:  cargo {}\n", self.target));
        s.push_str(&format!("evolve-rounds:  {}\n", self.completed_steps.len()));
        if !self.dead_ends.is_empty() {
            s.push_str(&format!("evolve-dead-ends: {}\n", self.dead_ends.len()));
        }
        if !self.binary_swaps.is_empty() {
            s.push_str(&format!(
                "evolve-binary-swaps: {}\n",
                self.binary_swaps.len()
            ));
        }
        if !self.journey.is_empty() {
            s.push_str(&format!("evolve-mesh-hops: {}\n", self.journey.len()));
        }
        if let Some(v) = &self.judge_score {
            s.push_str(&format!("evolve-judge-score: {}/10\n", v.score));
            s.push_str(&format!("evolve-judge-rubric: {}\n", v.rubric_version));
            s.push_str(&format!("evolve-judge-model: {}\n", v.model));
        }
        if let Some(e) = &self.judge_ensemble {
            s.push_str(&format!(
                "evolve-judge-ensemble-score: {}/10\n",
                e.aggregated.score
            ));
            s.push_str(&format!(
                "evolve-judge-ensemble-agreement: {}\n",
                agreement_short(&e.agreement)
            ));
            s.push_str(&format!(
                "evolve-judge-ensemble-stddev: {:.1}\n",
                e.score_stddev
            ));
            s.push_str(&format!(
                "evolve-judge-ensemble-judges: {}/{}\n",
                e.judges_succeeded, e.judges_attempted
            ));
        }
        s
    }
}

fn fmt_ts(ms: i64) -> String {
    // No chrono dep — keep it simple. For replay rendering, offsets within
    // a single session matter more than absolute calendar dates, so we show
    // HH:MM:SS UTC plus the raw epoch ms. A human reading `git log` cares
    // mostly about ordering and intra-session deltas.
    let total_secs = (ms / 1000).max(0) as u64;
    let hh = (total_secs / 3600) % 24;
    let mm = (total_secs % 3600) / 60;
    let ss = total_secs % 60;
    format!("{:02}:{:02}:{:02}Z (+{}ms)", hh, mm, ss, ms)
}

fn agreement_short(a: &AgreementClass) -> &'static str {
    match a {
        AgreementClass::Unanimous => "unanimous",
        AgreementClass::Consensus => "consensus",
        AgreementClass::NeedsHumanReview => "needs_human_review",
    }
}

fn phase_short(p: &EvolvePhase) -> &'static str {
    match p {
        EvolvePhase::Discovering => "discovering",
        EvolvePhase::Hypothesizing => "hypothesizing",
        EvolvePhase::Editing => "editing",
        EvolvePhase::Verifying => "verifying",
        EvolvePhase::Committing => "committing",
        EvolvePhase::Done { .. } => "done",
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

// ─── tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Tests touch the filesystem at ~/.phantom-mesh/evolve-checkpoints,
    // so serialize them to avoid concurrent dir manipulation.
    static FS_LOCK: Mutex<()> = Mutex::new(());

    fn fresh_ckpt() -> EvolveCheckpoint {
        EvolveCheckpoint::new("test goal", "check", "mac-test")
    }

    #[test]
    fn new_starts_in_discovering_phase_with_origin_node() {
        let c = fresh_ckpt();
        assert!(matches!(c.phase, EvolvePhase::Discovering));
        assert_eq!(c.origin_node, "mac-test");
        assert_eq!(c.current_node, "mac-test");
        assert!(c.session_id.starts_with("evolve-"));
        assert!(c.session_id.contains("mac-test"));
    }

    #[test]
    fn save_load_round_trip() {
        let _env = crate::env_lock::acquire();
        let _guard = FS_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut c = fresh_ckpt();
        c.append_step("read Cargo.toml", Some("file_read".into()), true);
        c.record_dead_end("typo on line 42", "no typo found, line was correct");
        c.save().expect("save failed");

        let loaded = EvolveCheckpoint::load(&c.session_id)
            .expect("load")
            .expect("present");
        assert_eq!(loaded.completed_steps.len(), 1);
        assert_eq!(loaded.dead_ends.len(), 1);
        assert_eq!(loaded.completed_steps[0].description, "read Cargo.toml");

        EvolveCheckpoint::delete(&c.session_id).expect("delete");
    }

    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn list_active_excludes_done() {
        let _env = crate::env_lock::acquire();
        let _guard = FS_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let a = EvolveCheckpoint::new("a", "check", "n1");
        let mut b = EvolveCheckpoint::new("b", "check", "n1");
        b.set_phase(EvolvePhase::Done {
            outcome: EvolveOutcome::Success {
                commit_sha: "abc".into(),
                rounds: 3,
            },
        });
        a.save().unwrap();
        b.save().unwrap();

        let actives = EvolveCheckpoint::list_all(true).unwrap();
        assert!(actives.iter().any(|c| c.session_id == a.session_id));
        assert!(!actives.iter().any(|c| c.session_id == b.session_id));

        let all = EvolveCheckpoint::list_all(false).unwrap();
        assert!(all.iter().any(|c| c.session_id == b.session_id));

        EvolveCheckpoint::delete(&a.session_id).unwrap();
        EvolveCheckpoint::delete(&b.session_id).unwrap();
    }

    #[test]
    fn node_hop_updates_current_and_appends_journey() {
        let mut c = fresh_ckpt();
        c.record_node_hop("peer-b", "origin peer groq quota exhausted");
        assert_eq!(c.current_node, "peer-b");
        assert_eq!(c.journey.len(), 1);
        assert_eq!(c.journey[0].from, "mac-test");
        assert_eq!(c.journey[0].to, "peer-b");
    }

    #[test]
    fn render_markdown_contains_all_sections_when_populated() {
        let mut c = fresh_ckpt();
        c.plan.push("step 1".into());
        c.append_step("did step 1", Some("shell".into()), true);
        c.record_dead_end("bad theory", "wrong file");
        c.record_artifact(ArtifactKind::Commit {
            sha: "deadbeef".into(),
            subject: "fix: thing".into(),
        });
        c.record_binary_swap("0.1.0+abc", "0.1.0+def", "rebuilt mid-evolve");
        c.record_node_hop("peer-b", "handoff");
        c.set_phase(EvolvePhase::Done {
            outcome: EvolveOutcome::Success {
                commit_sha: "deadbeef".into(),
                rounds: 2,
            },
        });

        let md = c.render_markdown();
        for needle in [
            "## plan",
            "## timeline",
            "## dead ends",
            "## artifacts",
            "## binary swaps",
            "## node journey",
            "## outcome",
            "✓ green",
        ] {
            assert!(md.contains(needle), "missing '{}' in:\n{}", needle, md);
        }
    }

    #[test]
    fn commit_trailer_is_compact_and_includes_session_id() {
        let mut c = fresh_ckpt();
        c.append_step("did a thing", None, true);
        c.record_dead_end("h", "f");

        let trailer = c.render_commit_trailer();
        assert!(trailer.contains(&c.session_id));
        assert!(trailer.contains("evolve-target:"));
        assert!(trailer.contains("evolve-rounds:"));
        assert!(trailer.contains("evolve-dead-ends:"));
    }

    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn atomic_save_doesnt_leave_tmp_behind() {
        let _env = crate::env_lock::acquire();
        let _guard = FS_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let c = fresh_ckpt();
        c.save().unwrap();
        let path = EvolveCheckpoint::path_for(&c.session_id).unwrap();
        let tmp = path.with_extension("json.tmp");
        assert!(path.exists());
        assert!(!tmp.exists(), ".tmp file should have been renamed away");
        EvolveCheckpoint::delete(&c.session_id).unwrap();
    }

    // ─── H1: judge verdict persistence ────────────────────────────────────

    #[test]
    fn judge_verdict_defaults_to_none_on_new_checkpoint() {
        let c = fresh_ckpt();
        assert!(
            c.judge_score.is_none(),
            "fresh checkpoint must have no judge_score yet"
        );
    }

    #[test]
    fn record_judge_verdict_sets_score_and_touches_timestamp() {
        let mut c = fresh_ckpt();
        let before = c.last_updated_ms;
        // Sleep one ms so the touch is observable on fast clocks.
        std::thread::sleep(std::time::Duration::from_millis(2));
        c.record_judge_verdict(JudgeVerdict {
            score: 7,
            rubric_version: "h1-v1".into(),
            model: "claude-haiku-4-5-20251001".into(),
            rationale: "made progress on stated goal".into(),
            judged_at_ms: now_ms(),
        });
        let after = c.judge_score.as_ref().expect("score must be set");
        assert_eq!(after.score, 7);
        assert_eq!(after.rubric_version, "h1-v1");
        assert!(
            c.last_updated_ms > before,
            "touch() must advance last_updated_ms"
        );
    }

    #[test]
    fn judge_verdict_round_trips_through_save_load() {
        let _env = crate::env_lock::acquire();
        let _guard = FS_LOCK.lock().unwrap_or_else(|p| p.into_inner());

        let mut c = fresh_ckpt();
        c.record_judge_verdict(JudgeVerdict {
            score: 9,
            rubric_version: "h1-v1".into(),
            model: "claude-haiku-4-5-20251001".into(),
            rationale: "shipped fix + green tests".into(),
            judged_at_ms: 1_700_000_000_000,
        });
        c.save().unwrap();

        let loaded = EvolveCheckpoint::load(&c.session_id).unwrap().unwrap();
        let v = loaded.judge_score.as_ref().expect("loaded judge_score");
        assert_eq!(v.score, 9);
        assert_eq!(v.rubric_version, "h1-v1");
        assert_eq!(v.judged_at_ms, 1_700_000_000_000);

        EvolveCheckpoint::delete(&c.session_id).unwrap();
    }

    #[test]
    fn render_markdown_includes_judge_section_when_score_set() {
        let mut c = fresh_ckpt();
        c.record_judge_verdict(JudgeVerdict {
            score: 8,
            rubric_version: "h1-v1".into(),
            model: "claude-haiku-4-5-20251001".into(),
            rationale: "good progress".into(),
            judged_at_ms: now_ms(),
        });
        let md = c.render_markdown();
        assert!(
            md.contains("## judge verdict"),
            "expected judge section in:\n{}",
            md
        );
        assert!(md.contains("8/10"), "expected score in:\n{}", md);
        assert!(md.contains("h1-v1"), "expected rubric version");
        assert!(md.contains("good progress"), "expected rationale");
    }

    #[test]
    fn render_markdown_omits_judge_section_when_no_score() {
        let c = fresh_ckpt();
        let md = c.render_markdown();
        assert!(
            !md.contains("## judge verdict"),
            "no judge section without a score"
        );
    }

    #[test]
    fn commit_trailer_includes_judge_when_score_set() {
        let mut c = fresh_ckpt();
        c.record_judge_verdict(JudgeVerdict {
            score: 6,
            rubric_version: "h1-v1".into(),
            model: "claude-haiku-4-5-20251001".into(),
            rationale: "partial".into(),
            judged_at_ms: now_ms(),
        });
        let trailer = c.render_commit_trailer();
        assert!(trailer.contains("evolve-judge-score: 6/10"));
        assert!(trailer.contains("evolve-judge-rubric: h1-v1"));
    }

    #[test]
    fn judge_verdict_serde_default_preserves_old_checkpoint_files() {
        // A checkpoint JSON written by a phantom binary that predates H1
        // will not contain the judge_score field. Serde must accept that
        // and default it to None, otherwise we break old user state.
        let json_without_field = r#"{
            "session_id": "evolve-old-1-mac",
            "started_at_ms": 1700000000000,
            "last_updated_ms": 1700000000000,
            "goal": "old goal",
            "phase": { "phase": "discovering" },
            "target": "check",
            "plan": [],
            "completed_steps": [],
            "current_hypothesis": null,
            "dead_ends": [],
            "artifacts": [],
            "binary_swaps": [],
            "origin_node": "mac",
            "current_node": "mac",
            "journey": []
        }"#;
        let parsed: EvolveCheckpoint =
            serde_json::from_str(json_without_field).expect("must parse old shape");
        assert!(parsed.judge_score.is_none());
    }

    // ─── V2 (T28): ensemble verdict ──────────────────────────────────────

    #[test]
    fn agreement_class_serializes_snake_case() {
        let v = AgreementClass::NeedsHumanReview;
        let s = serde_json::to_string(&v).unwrap();
        assert_eq!(s, "\"needs_human_review\"");
        let back: AgreementClass = serde_json::from_str("\"unanimous\"").unwrap();
        assert_eq!(back, AgreementClass::Unanimous);
    }

    #[test]
    fn ensemble_verdict_serde_default_preserves_old_checkpoint_files() {
        // Same shape as the existing back-compat test for judge_score —
        // a checkpoint written before V2 must still deserialize.
        let json_without_field = r#"{
            "session_id": "evolve-old-1-mac",
            "started_at_ms": 1700000000000,
            "last_updated_ms": 1700000000000,
            "goal": "old goal",
            "phase": { "phase": "discovering" },
            "target": "check",
            "plan": [],
            "completed_steps": [],
            "current_hypothesis": null,
            "dead_ends": [],
            "artifacts": [],
            "binary_swaps": [],
            "origin_node": "mac",
            "current_node": "mac",
            "journey": []
        }"#;
        let parsed: EvolveCheckpoint =
            serde_json::from_str(json_without_field).expect("must parse pre-V2 shape");
        assert!(parsed.judge_score.is_none());
        assert!(
            parsed.judge_ensemble.is_none(),
            "V2 field must serde-default to None for back-compat"
        );
    }

    #[test]
    fn record_ensemble_verdict_sets_field_and_touches_timestamp() {
        let mut c = fresh_ckpt();
        let before = c.last_updated_ms;
        std::thread::sleep(std::time::Duration::from_millis(2));
        let v = EnsembleVerdict {
            aggregated: JudgeVerdict {
                score: 7,
                rubric_version: "h1-v1".into(),
                model: "ensemble:3".into(),
                rationale: "ensemble of 3 judges; see individual[*]".into(),
                judged_at_ms: now_ms(),
            },
            individual: vec![],
            agreement: AgreementClass::Consensus,
            score_median: 7.0,
            score_stddev: 0.8,
            judges_attempted: 3,
            judges_succeeded: 3,
        };
        c.record_ensemble_verdict(v);
        assert!(c.judge_ensemble.is_some());
        assert_eq!(c.judge_ensemble.as_ref().unwrap().aggregated.score, 7);
        assert!(c.last_updated_ms > before);
    }

    #[test]
    fn render_markdown_includes_ensemble_section_when_set() {
        let mut c = fresh_ckpt();
        c.record_ensemble_verdict(EnsembleVerdict {
            aggregated: JudgeVerdict {
                score: 8,
                rubric_version: "h1-v1".into(),
                model: "ensemble:3".into(),
                rationale: "ensemble of 3 judges".into(),
                judged_at_ms: now_ms(),
            },
            individual: vec![
                JudgeVerdict {
                    score: 8,
                    rubric_version: "h1-v1".into(),
                    model: "claude-haiku-4-5-20251001".into(),
                    rationale: "good".into(),
                    judged_at_ms: now_ms(),
                },
                JudgeVerdict {
                    score: 7,
                    rubric_version: "h1-v1".into(),
                    model: "mistral-small-latest@mistral".into(),
                    rationale: "ok".into(),
                    judged_at_ms: now_ms(),
                },
                JudgeVerdict {
                    score: 9,
                    rubric_version: "h1-v1".into(),
                    model: "grok-4@xai".into(),
                    rationale: "great".into(),
                    judged_at_ms: now_ms(),
                },
            ],
            agreement: AgreementClass::Consensus,
            score_median: 8.0,
            score_stddev: 0.82,
            judges_attempted: 3,
            judges_succeeded: 3,
        });
        let md = c.render_markdown();
        assert!(md.contains("## ensemble verdict"));
        assert!(md.contains("3/3 judges"));
        assert!(md.contains("consensus"));
        assert!(md.contains("median 8"));
    }

    #[test]
    fn commit_trailer_includes_ensemble_when_set() {
        let mut c = fresh_ckpt();
        c.record_ensemble_verdict(EnsembleVerdict {
            aggregated: JudgeVerdict {
                score: 6,
                rubric_version: "h1-v1".into(),
                model: "ensemble:3".into(),
                rationale: "x".into(),
                judged_at_ms: now_ms(),
            },
            individual: vec![],
            agreement: AgreementClass::NeedsHumanReview,
            score_median: 6.0,
            score_stddev: 3.2,
            judges_attempted: 3,
            judges_succeeded: 3,
        });
        let trailer = c.render_commit_trailer();
        assert!(trailer.contains("evolve-judge-ensemble-score: 6/10"));
        assert!(trailer.contains("evolve-judge-ensemble-agreement: needs_human_review"));
        assert!(trailer.contains("evolve-judge-ensemble-stddev: 3.2"));
    }
}
