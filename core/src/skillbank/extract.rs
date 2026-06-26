//! A1 / T91 — Skill extractor: turn a `JudgeVerdict` (over an
//! `EvolveCheckpoint`) into a candidate `SkillDocument`.
//!
//! ## A8 update (PR-aligned with A2)
//!
//! The original A1 design (PR #144) only extracted skills from **failed**
//! evolve sessions (score < θ): the lesson was "here is a recurring trap,
//! avoid it next time". That is asymmetric with A2 (PR #146), which only
//! invokes the extractor for **successful** runs (score ≥ θ): the assumption
//! there is "a high-scoring run is worth distilling for reuse".
//!
//! The two never overlapped, so the end-to-end loop in DEMO-2 (PR #157)
//! had to install a `VerdictBackedExtractor` adapter that disabled A1's
//! score gate (`extract_skill_with_threshold(_, _, u8::MAX)`).
//!
//! A8 reconciles this by giving A1 a dual API:
//!
//! * [`extract_skill_from_failure`] — original behaviour, score < θ;
//!   classifies the failure (retry loop, wrong hypothesis, wrong direction,
//!   stuck) and emits a "lesson learned" skill.
//! * [`extract_skill_from_success`] — NEW, score ≥ θ; classifies the
//!   successful pattern (solution_pattern, workflow_pattern,
//!   confirmed_hypothesis, stuck_pattern_resolved) and emits a "reusable
//!   recipe" skill.
//! * [`extract_skill`] — backward-compat shim: tries the success-side
//!   extractor first, then the failure-side, returning the first `Some`.
//!   A2 calls this directly through `VerdictBackedExtractor`; no more
//!   threshold workaround.
//! * [`extract_skill_with_threshold`] — retained for callers (and tests)
//!   that want a different cutoff. Threshold semantics still control the
//!   *failure* path; the success path uses the symmetric `>= threshold`
//!   check internally.
//!
//! Confidence floor remains: if neither side has any signal we can latch
//! onto (no dead ends, no completed_steps, empty rationale, no hypothesis),
//! both functions return `None` — we don't manufacture skills from nothing.
//!
//! Gated behind `experimental-curator` so the default build pulls
//! neither this nor its `serde_yaml` round-trip dep.

#![cfg(feature = "experimental-curator")]

use std::collections::BTreeMap;

use crate::evolve_checkpoint::{EvolveCheckpoint, JudgeVerdict};
use crate::skillbank::skill::{serialize as serialize_skill, SkillDocument, SkillFrontmatter};

/// Verdicts at or above this score are treated as "successful" — the
/// success-side extractor runs; the failure-side abstains. 0.5 in the
/// 0..1 sense from the task spec maps to **5/10** on the curator rubric
/// (`h1-v1` scores integer 0..=10). Same number as the A2 gate.
pub const DEFAULT_SCORE_THRESHOLD: u8 = 5;

/// Minimum number of distinct signals we want to see before we believe the
/// extraction is meaningful enough to emit. One signal = one of:
/// (a) at least one dead end, (b) at least one failed completed_step,
/// (c) a non-empty `current_hypothesis`, (d) a non-empty rationale, or
/// (e) at least one *successful* completed_step (for the success path).
const MIN_CONFIDENCE_SIGNALS: usize = 1;

// ─── public API ──────────────────────────────────────────────────────────

/// A8 backward-compat: try the success-side extractor, then the
/// failure-side; return the first `Some` (or `None` if neither fires).
///
/// Net effect: callers (notably A2's [`SkillExtractor`] adapters) get a
/// skill back regardless of polarity — the function inspects the verdict
/// score and routes to the right classifier. This is what
/// `VerdictBackedExtractor` in DEMO-2 was emulating with a `u8::MAX`
/// threshold; it can now disappear.
pub fn extract_skill(verdict: &JudgeVerdict, context: &EvolveCheckpoint) -> Option<SkillDocument> {
    extract_skill_with_threshold(verdict, context, DEFAULT_SCORE_THRESHOLD)
}

/// Same as [`extract_skill`] but lets the caller choose the score
/// threshold. The success / failure routing is anchored at `threshold`:
/// `score >= threshold` runs the success classifier, `score < threshold`
/// runs the failure classifier.
///
/// Retained for tests (we exercise both sides by sliding the threshold)
/// and for any caller that wants a stricter / looser bar.
pub fn extract_skill_with_threshold(
    verdict: &JudgeVerdict,
    context: &EvolveCheckpoint,
    threshold: u8,
) -> Option<SkillDocument> {
    if verdict.score >= threshold {
        extract_skill_from_success(verdict, context)
    } else {
        extract_skill_from_failure(verdict, context)
    }
}

/// Failure-side extractor (original A1 behaviour). Routes the verdict +
/// checkpoint through the four `FailurePattern` classifiers (retry_pattern,
/// hypothesis_check, direction_check, stuck_pattern) and emits a "lesson
/// learned" skill.
///
/// Returns `None` if there are insufficient signals or the round-trip
/// sanity check fails.
pub fn extract_skill_from_failure(
    verdict: &JudgeVerdict,
    context: &EvolveCheckpoint,
) -> Option<SkillDocument> {
    let pattern = classify_failure(verdict, context);
    let signals = count_failure_signals(verdict, context);
    if signals < MIN_CONFIDENCE_SIGNALS {
        return None;
    }
    let frontmatter = build_frontmatter(&pattern, verdict, context, /*success=*/ false);
    let body = build_body_failure(&pattern, verdict, context);
    let doc = SkillDocument { frontmatter, body };
    if !roundtrips_cleanly(&doc) {
        return None;
    }
    Some(doc)
}

/// Success-side extractor (A8, new). Routes the verdict + checkpoint
/// through the four `SuccessPattern` classifiers (solution_pattern,
/// workflow_pattern, confirmed_hypothesis, stuck_pattern_resolved) and
/// emits a "reusable recipe" skill.
///
/// The classifier inspects the *positive* signals on the checkpoint —
/// completed_steps with success=true, a hypothesis that survived (no
/// dead ends), and rationale tokens like "fixed" / "shipped" / "done".
/// If the checkpoint is empty (no completed work, no hypothesis), we
/// abstain — a verdict alone is too weak to distil.
pub fn extract_skill_from_success(
    verdict: &JudgeVerdict,
    context: &EvolveCheckpoint,
) -> Option<SkillDocument> {
    let pattern = classify_success(verdict, context);
    let signals = count_success_signals(verdict, context);
    if signals < MIN_CONFIDENCE_SIGNALS {
        return None;
    }
    let frontmatter = build_frontmatter(&pattern, verdict, context, /*success=*/ true);
    let body = build_body_success(&pattern, verdict, context);
    let doc = SkillDocument { frontmatter, body };
    if !roundtrips_cleanly(&doc) {
        return None;
    }
    Some(doc)
}

// ─── failure patterns (original A1) ──────────────────────────────────────

/// Coarse-grained failure category derived from rationale + checkpoint shape.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FailurePattern {
    /// Agent re-tried the same thing many times and got nowhere.
    RetryLoop,
    /// Agent latched onto a hypothesis that turned out wrong (>= 1 dead end).
    WrongHypothesis,
    /// Agent took destructive action / wrong direction (very low score).
    WrongDirection,
    /// Generic catch-all — we saw some signal but no clear pattern.
    Stuck,
}

impl FailurePattern {
    fn slug(&self) -> &'static str {
        match self {
            FailurePattern::RetryLoop => "retry_pattern",
            FailurePattern::WrongHypothesis => "hypothesis_check",
            FailurePattern::WrongDirection => "direction_check",
            FailurePattern::Stuck => "stuck_pattern",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            FailurePattern::RetryLoop => {
                "Avoid retry loops: when the same step fails twice, change approach instead of repeating."
            }
            FailurePattern::WrongHypothesis => {
                "Verify a hypothesis with a cheap test before committing to multi-step work."
            }
            FailurePattern::WrongDirection => {
                "Before acting destructively, confirm the diagnosis matches observed evidence."
            }
            FailurePattern::Stuck => {
                "When stuck, summarize current state and ask for help instead of guessing further."
            }
        }
    }

    fn primary_trigger(&self) -> &'static str {
        match self {
            FailurePattern::RetryLoop => "repeated tool failure",
            FailurePattern::WrongHypothesis => "hypothesis rejected by evidence",
            FailurePattern::WrongDirection => "destructive or off-goal action",
            FailurePattern::Stuck => "no progress after multiple steps",
        }
    }
}

/// Heuristic classifier for the failure path. Order matters — most-specific
/// patterns first.
fn classify_failure(verdict: &JudgeVerdict, context: &EvolveCheckpoint) -> FailurePattern {
    let rationale_lc = verdict.rationale.to_lowercase();

    // Very low score → wrong direction takes priority (matches rubric "0/2").
    if verdict.score <= 1 {
        return FailurePattern::WrongDirection;
    }

    // Retry-loop signal: failed step appears more than once with same tool, or
    // rationale mentions retry/loop/again/repeat.
    if rationale_lc.contains("retry")
        || rationale_lc.contains("loop")
        || rationale_lc.contains("again")
        || rationale_lc.contains("repeat")
        || has_repeated_failed_step(context)
    {
        return FailurePattern::RetryLoop;
    }

    // Wrong-hypothesis signal: any dead ends present, or rationale references hypothesis/wrong.
    if !context.dead_ends.is_empty()
        || rationale_lc.contains("hypothesis")
        || rationale_lc.contains("wrong assumption")
        || rationale_lc.contains("got stuck")
    {
        return FailurePattern::WrongHypothesis;
    }

    FailurePattern::Stuck
}

/// True if the same `tool` appears in two-or-more failed completed_steps.
fn has_repeated_failed_step(ck: &EvolveCheckpoint) -> bool {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for s in &ck.completed_steps {
        if !s.success {
            let key = s.tool.clone().unwrap_or_else(|| "<no-tool>".to_string());
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    counts.values().any(|n| *n >= 2)
}

/// Count distinct independent "signals" supporting a failure-side extraction.
fn count_failure_signals(verdict: &JudgeVerdict, ck: &EvolveCheckpoint) -> usize {
    let mut n = 0;
    if !verdict.rationale.trim().is_empty() {
        n += 1;
    }
    if !ck.dead_ends.is_empty() {
        n += 1;
    }
    if ck.completed_steps.iter().any(|s| !s.success) {
        n += 1;
    }
    if ck
        .current_hypothesis
        .as_deref()
        .map(|h| !h.is_empty())
        .unwrap_or(false)
    {
        n += 1;
    }
    n
}

// ─── success patterns (A8, new) ──────────────────────────────────────────

/// Coarse-grained success category derived from rationale + checkpoint shape.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SuccessPattern {
    /// Agent shipped a clean fix in a small number of well-chosen steps.
    SolutionPattern,
    /// Agent followed a multi-step workflow that lands well (good template
    /// for similar future tasks).
    WorkflowPattern,
    /// A hypothesis the agent stated up front survived through to success.
    ConfirmedHypothesis,
    /// Agent was stuck (had dead ends) but recovered and shipped — useful
    /// "what unblocked me" recipe.
    StuckPatternResolved,
}

impl SuccessPattern {
    fn slug(&self) -> &'static str {
        match self {
            SuccessPattern::SolutionPattern => "solution_pattern",
            SuccessPattern::WorkflowPattern => "workflow_pattern",
            SuccessPattern::ConfirmedHypothesis => "confirmed_hypothesis",
            SuccessPattern::StuckPatternResolved => "stuck_pattern_resolved",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            SuccessPattern::SolutionPattern => {
                "Clean direct fix — replay the same step sequence when a similar symptom shows up."
            }
            SuccessPattern::WorkflowPattern => {
                "Multi-step workflow that worked end-to-end. Reuse the template; adjust step details to the new task."
            }
            SuccessPattern::ConfirmedHypothesis => {
                "Hypothesis held up under verification. Trust this diagnosis the next time the same symptoms appear."
            }
            SuccessPattern::StuckPatternResolved => {
                "Got stuck then unstuck — record what unblocked the agent so a future run can skip the dead ends."
            }
        }
    }

    fn primary_trigger(&self) -> &'static str {
        match self {
            SuccessPattern::SolutionPattern => "task fits a known clean-fix shape",
            SuccessPattern::WorkflowPattern => "multi-step task with sequenced tool calls",
            SuccessPattern::ConfirmedHypothesis => "hypothesis matches observed evidence",
            SuccessPattern::StuckPatternResolved => "previous attempt was stuck; this one shipped",
        }
    }
}

/// Heuristic classifier for the success path. Order matters — most-specific
/// patterns first.
fn classify_success(verdict: &JudgeVerdict, context: &EvolveCheckpoint) -> SuccessPattern {
    let rationale_lc = verdict.rationale.to_lowercase();
    let successful_steps = context.completed_steps.iter().filter(|s| s.success).count();
    let had_dead_ends = !context.dead_ends.is_empty();
    let has_hypothesis = context
        .current_hypothesis
        .as_deref()
        .map(|h| !h.is_empty())
        .unwrap_or(false);

    // Stuck-and-recovered takes priority: dead ends are a strong "this was
    // hard" signal, and the success score says we got through anyway.
    if had_dead_ends && successful_steps >= 1 {
        return SuccessPattern::StuckPatternResolved;
    }

    // Confirmed-hypothesis: an explicit hypothesis on the checkpoint plus
    // rationale that endorses it ("confirmed", "as predicted", "matched
    // evidence", or just "hypothesis").
    if has_hypothesis
        && (rationale_lc.contains("confirmed")
            || rationale_lc.contains("as predicted")
            || rationale_lc.contains("matched evidence")
            || rationale_lc.contains("hypothesis"))
    {
        return SuccessPattern::ConfirmedHypothesis;
    }

    // Workflow: multiple sequenced successful steps look like a recipe worth
    // canonicalising. >= 3 steps (matches "shipped after a non-trivial loop").
    if successful_steps >= 3 {
        return SuccessPattern::WorkflowPattern;
    }

    // Default: a clean one- or two-step fix.
    SuccessPattern::SolutionPattern
}

/// Count distinct independent "signals" supporting a success-side extraction.
/// Unlike the failure path, a successful completed_step counts as a signal —
/// it is the primary evidence that the agent actually did productive work.
fn count_success_signals(verdict: &JudgeVerdict, ck: &EvolveCheckpoint) -> usize {
    let mut n = 0;
    if !verdict.rationale.trim().is_empty() {
        n += 1;
    }
    if ck.completed_steps.iter().any(|s| s.success) {
        n += 1;
    }
    if ck
        .current_hypothesis
        .as_deref()
        .map(|h| !h.is_empty())
        .unwrap_or(false)
    {
        n += 1;
    }
    // A success run that also had dead ends is interesting (stuck-and-recovered).
    if !ck.dead_ends.is_empty() {
        n += 1;
    }
    n
}

// ─── shared frontmatter / body builders ──────────────────────────────────

/// Common frontmatter builder. `pattern_slug` / `pattern_description` /
/// `pattern_trigger` come from whichever side called us; everything else
/// (tags, target provenance, rubric provenance, author) is identical so we
/// share the code path.
fn build_frontmatter(
    pattern: &dyn PatternMeta,
    verdict: &JudgeVerdict,
    ck: &EvolveCheckpoint,
    success: bool,
) -> SkillFrontmatter {
    let provenance_tag = if success {
        "provenance:success"
    } else {
        "provenance:failure"
    };
    let mut tags = vec![
        "auto-extracted".to_string(),
        provenance_tag.to_string(),
        format!("rubric:{}", verdict.rubric_version),
        format!("score:{}", verdict.score),
        format!("pattern:{}", pattern.slug()),
    ];
    if !ck.target.is_empty() {
        tags.push(format!("target:{}", ck.target));
    }

    SkillFrontmatter {
        name: pattern.slug().to_string(),
        version: "0.1.0".to_string(),
        description: pattern.description().to_string(),
        triggers: vec![
            pattern.primary_trigger().to_string(),
            if success {
                "high judge score".to_string()
            } else {
                "low judge score".to_string()
            },
        ],
        tools: vec![],
        inputs: BTreeMap::new(),
        outputs: vec![],
        tags,
        created_at: Some(fmt_iso8601(verdict.judged_at_ms)),
        author: Some(format!("skill-extractor/{}", verdict.rubric_version)),
    }
}

/// Trait for the small bit of pattern metadata both classifiers share.
/// Implemented for `FailurePattern` + `SuccessPattern`.
trait PatternMeta {
    fn slug(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn primary_trigger(&self) -> &'static str;
}

impl PatternMeta for FailurePattern {
    fn slug(&self) -> &'static str {
        FailurePattern::slug(self)
    }
    fn description(&self) -> &'static str {
        FailurePattern::description(self)
    }
    fn primary_trigger(&self) -> &'static str {
        FailurePattern::primary_trigger(self)
    }
}

impl PatternMeta for SuccessPattern {
    fn slug(&self) -> &'static str {
        SuccessPattern::slug(self)
    }
    fn description(&self) -> &'static str {
        SuccessPattern::description(self)
    }
    fn primary_trigger(&self) -> &'static str {
        SuccessPattern::primary_trigger(self)
    }
}

fn build_body_failure(
    pattern: &FailurePattern,
    verdict: &JudgeVerdict,
    ck: &EvolveCheckpoint,
) -> String {
    let mut s = String::new();
    s.push_str(&format!("# {}\n\n", pattern.slug()));
    s.push_str("> Auto-extracted from a failed evolve session. Review before promoting to a first-party skill.\n\n");

    s.push_str("## Context\n");
    s.push_str(&format!("- Goal: {}\n", ck.goal.trim()));
    s.push_str(&format!("- Cargo target: {}\n", ck.target));
    s.push_str(&format!("- Origin node: {}\n", ck.origin_node));
    s.push_str(&format!(
        "- Judge score: {}/10 (rubric {})\n",
        verdict.score, verdict.rubric_version,
    ));
    if !verdict.rationale.trim().is_empty() {
        s.push_str(&format!(
            "- Judge rationale: {}\n",
            verdict.rationale.trim()
        ));
    }
    s.push('\n');

    s.push_str("## Lesson\n");
    s.push_str(pattern.description());
    s.push_str("\n\n");

    if !ck.dead_ends.is_empty() {
        s.push_str("## Dead ends to skip\n");
        for d in &ck.dead_ends {
            s.push_str(&format!(
                "- **{}** — {}\n",
                d.hypothesis.trim(),
                d.why_failed.trim(),
            ));
        }
        s.push('\n');
    }

    let failed: Vec<_> = ck.completed_steps.iter().filter(|s| !s.success).collect();
    if !failed.is_empty() {
        s.push_str("## Failed steps observed\n");
        for st in failed {
            let tool = st
                .tool
                .as_deref()
                .map(|t| format!(" [{}]", t))
                .unwrap_or_default();
            s.push_str(&format!("- {}{}\n", st.description.trim(), tool));
        }
        s.push('\n');
    }

    s.push_str("## Suggested check before retry\n");
    s.push_str(
        "1. Re-read the goal and verify the current hypothesis still matches the evidence.\n",
    );
    s.push_str(
        "2. If a previous step already failed for the same reason, change tools/approach.\n",
    );
    s.push_str("3. Bound the next attempt — fail fast, do not silently loop.\n");
    s
}

fn build_body_success(
    pattern: &SuccessPattern,
    verdict: &JudgeVerdict,
    ck: &EvolveCheckpoint,
) -> String {
    let mut s = String::new();
    s.push_str(&format!("# {}\n\n", pattern.slug()));
    s.push_str("> Auto-extracted from a SUCCESSFUL evolve session. Reuse before promoting to a first-party skill.\n\n");

    s.push_str("## Context\n");
    s.push_str(&format!("- Goal: {}\n", ck.goal.trim()));
    s.push_str(&format!("- Cargo target: {}\n", ck.target));
    s.push_str(&format!("- Origin node: {}\n", ck.origin_node));
    s.push_str(&format!(
        "- Judge score: {}/10 (rubric {})\n",
        verdict.score, verdict.rubric_version,
    ));
    if !verdict.rationale.trim().is_empty() {
        s.push_str(&format!(
            "- Judge rationale: {}\n",
            verdict.rationale.trim()
        ));
    }
    s.push('\n');

    s.push_str("## Recipe\n");
    s.push_str(pattern.description());
    s.push_str("\n\n");

    let successful: Vec<_> = ck.completed_steps.iter().filter(|s| s.success).collect();
    if !successful.is_empty() {
        s.push_str("## Successful steps to replay\n");
        for st in successful {
            let tool = st
                .tool
                .as_deref()
                .map(|t| format!(" [{}]", t))
                .unwrap_or_default();
            s.push_str(&format!("- {}{}\n", st.description.trim(), tool));
        }
        s.push('\n');
    }

    if let Some(h) = ck.current_hypothesis.as_deref() {
        if !h.is_empty() {
            s.push_str("## Confirmed hypothesis\n");
            s.push_str(h.trim());
            s.push_str("\n\n");
        }
    }

    if !ck.dead_ends.is_empty() {
        s.push_str("## Dead ends already ruled out\n");
        for d in &ck.dead_ends {
            s.push_str(&format!(
                "- **{}** — {}\n",
                d.hypothesis.trim(),
                d.why_failed.trim(),
            ));
        }
        s.push('\n');
    }

    s.push_str("## Suggested reuse\n");
    s.push_str("1. Confirm the new task's symptoms match the rationale above.\n");
    s.push_str(
        "2. Replay the successful steps in order; substitute file paths / identifiers as needed.\n",
    );
    s.push_str("3. If a dead-end shape reappears, skip it — it was already ruled out.\n");
    s
}

/// Validate the doc by serializing → parsing → comparing. If anything goes
/// wrong the extractor refuses to return it.
fn roundtrips_cleanly(doc: &SkillDocument) -> bool {
    let serialized = match serialize_skill(doc) {
        Ok(s) => s,
        Err(_) => return false,
    };
    match crate::skillbank::skill::parse_str(&serialized) {
        Ok(reparsed) => &reparsed == doc,
        Err(_) => false,
    }
}

/// Minimal ISO-8601 UTC formatter so we don't pull `chrono` into this feature.
/// Input is millis since the Unix epoch.
fn fmt_iso8601(ms: i64) -> String {
    // Howard Hinnant's civil-from-days algorithm.
    let secs = ms / 1_000;
    let (days_raw, time_in_day) = if secs >= 0 {
        (secs / 86_400, secs % 86_400)
    } else {
        // Floor-divide for negative times.
        let q = (secs - 86_399) / 86_400;
        let r = secs - q * 86_400;
        (q, r)
    };
    let z = days_raw + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0,399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = y + if m <= 2 { 1 } else { 0 };

    let h = (time_in_day / 3600) as u32;
    let mi = ((time_in_day % 3600) / 60) as u32;
    let se = (time_in_day % 60) as u32;
    format!("{year:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{se:02}Z")
}

// ─── tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evolve_checkpoint::EvolveCheckpoint;
    use crate::skillbank::curator::RUBRIC_VERSION;

    fn mk_verdict(score: u8, rationale: &str) -> JudgeVerdict {
        JudgeVerdict {
            score,
            rubric_version: RUBRIC_VERSION.to_string(),
            model: "claude-haiku-4-5-20251001".to_string(),
            rationale: rationale.to_string(),
            judged_at_ms: 1_715_000_000_000,
        }
    }

    fn mk_checkpoint(goal: &str) -> EvolveCheckpoint {
        EvolveCheckpoint::new(goal, "check", "test-node")
    }

    // ── failure path (original A1 behaviour preserved) ──

    #[test]
    fn failure_with_retry_pattern_emits_retry_pattern_skill() {
        let v = mk_verdict(2, "agent gave up at retry attempt 4");
        let mut ck = mk_checkpoint("fix lint");
        ck.append_step("ran cargo check", Some("cargo".into()), false);
        ck.append_step("ran cargo check again", Some("cargo".into()), false);

        let doc = extract_skill_from_failure(&v, &ck).expect("should extract a skill");
        assert_eq!(doc.frontmatter.name, "retry_pattern");
        assert!(doc.frontmatter.tags.contains(&"auto-extracted".to_string()));
        assert!(doc
            .frontmatter
            .tags
            .iter()
            .any(|t| t.starts_with("pattern:retry_pattern")));
        assert!(doc
            .frontmatter
            .tags
            .iter()
            .any(|t| t == "provenance:failure"));
        assert!(doc.body.contains("retry_pattern"));
        assert!(doc.body.contains("Failed steps observed"));
    }

    #[test]
    fn extract_skill_routes_success_score_to_success_path() {
        // Score >= threshold (5) + a successful step ⇒ extract_skill emits
        // a success-side skill rather than returning None.
        let v = mk_verdict(8, "clean fix, tests green");
        let mut ck = mk_checkpoint("fix lint");
        ck.append_step("ran cargo check", Some("cargo".into()), true);
        let doc = extract_skill(&v, &ck).expect("success-side should emit a skill");
        assert_eq!(doc.frontmatter.name, "solution_pattern");
        assert!(doc
            .frontmatter
            .tags
            .iter()
            .any(|t| t == "provenance:success"));
    }

    #[test]
    fn extract_skill_routes_failure_score_to_failure_path() {
        let v = mk_verdict(2, "agent gave up at retry attempt 4");
        let mut ck = mk_checkpoint("fix lint");
        ck.append_step("ran cargo check", Some("cargo".into()), false);
        ck.append_step("ran cargo check again", Some("cargo".into()), false);
        let doc = extract_skill(&v, &ck).expect("failure-side should emit a skill");
        assert_eq!(doc.frontmatter.name, "retry_pattern");
        assert!(doc
            .frontmatter
            .tags
            .iter()
            .any(|t| t == "provenance:failure"));
    }

    #[test]
    fn boundary_score_at_threshold_routes_to_success_path() {
        // A8: at-threshold goes to the SUCCESS side (matches A2's `>=` gate).
        // Pre-A8 this returned None; that test is replaced by the routing
        // assertion below.
        let v = mk_verdict(DEFAULT_SCORE_THRESHOLD, "borderline");
        let mut ck = mk_checkpoint("fix lint");
        ck.append_step("did the thing", None, true);
        let doc = extract_skill(&v, &ck).expect("at-threshold should run success extractor");
        assert!(doc
            .frontmatter
            .tags
            .iter()
            .any(|t| t == "provenance:success"));
    }

    #[test]
    fn low_confidence_failure_extraction_returns_none() {
        // Score is low BUT we have zero signals.
        let v = mk_verdict(2, "");
        let ck = mk_checkpoint("fix lint");
        assert!(extract_skill(&v, &ck).is_none());
    }

    #[test]
    fn low_confidence_success_extraction_returns_none() {
        // Score is high BUT we have zero signals (no successful steps,
        // no hypothesis, no dead ends, empty rationale).
        let v = mk_verdict(9, "");
        let ck = mk_checkpoint("fix lint");
        assert!(extract_skill(&v, &ck).is_none());
    }

    #[test]
    fn malformed_verdict_blank_everything_returns_none() {
        let mut v = mk_verdict(0, "");
        v.rubric_version = "".to_string();
        v.model = "".to_string();
        let ck = mk_checkpoint("");
        assert!(extract_skill(&v, &ck).is_none());
    }

    #[test]
    fn wrong_direction_classifier_fires_on_very_low_score() {
        let v = mk_verdict(1, "agent did destructive rm -rf");
        let mut ck = mk_checkpoint("clean build dir");
        ck.append_step("rm -rf .", Some("bash".into()), false);
        let doc = extract_skill_from_failure(&v, &ck).expect("should extract");
        assert_eq!(doc.frontmatter.name, "direction_check");
    }

    #[test]
    fn wrong_hypothesis_classifier_fires_on_dead_end_with_no_retry_keywords() {
        let v = mk_verdict(4, "score reflects partial progress");
        let mut ck = mk_checkpoint("fix flaky test");
        ck.record_dead_end(
            "test is flaky due to TZ",
            "actually a fixture race condition",
        );
        let doc = extract_skill_from_failure(&v, &ck).expect("should extract");
        assert_eq!(doc.frontmatter.name, "hypothesis_check");
        assert!(doc.body.contains("Dead ends to skip"));
    }

    #[test]
    fn stuck_pattern_is_default_failure_with_only_hypothesis_signal() {
        // No retry keyword, no dead ends — but hypothesis is a signal.
        let v = mk_verdict(3, "made some progress then plateaued");
        let mut ck = mk_checkpoint("g");
        ck.current_hypothesis = Some("the parser drops the newline".to_string());
        let doc = extract_skill_from_failure(&v, &ck).expect("should extract");
        assert_eq!(doc.frontmatter.name, "stuck_pattern");
    }

    // ── success path (A8, new) ──

    #[test]
    fn success_solution_pattern_default_one_step_clean_fix() {
        let v = mk_verdict(8, "clean fix, tests green");
        let mut ck = mk_checkpoint("fix lint");
        ck.append_step("applied the patch", Some("apply_patch".into()), true);
        let doc = extract_skill_from_success(&v, &ck).expect("should extract");
        assert_eq!(doc.frontmatter.name, "solution_pattern");
        assert!(doc.body.contains("Successful steps to replay"));
        assert!(doc
            .frontmatter
            .triggers
            .iter()
            .any(|t| t == "high judge score"));
    }

    #[test]
    fn success_workflow_pattern_fires_on_3_plus_successful_steps() {
        let v = mk_verdict(9, "shipped after a 4-step workflow");
        let mut ck = mk_checkpoint("rebase and ship");
        ck.append_step("git fetch", Some("git".into()), true);
        ck.append_step("git rebase main", Some("git".into()), true);
        ck.append_step("cargo check", Some("cargo".into()), true);
        ck.append_step("git push", Some("git".into()), true);
        let doc = extract_skill_from_success(&v, &ck).expect("should extract");
        assert_eq!(doc.frontmatter.name, "workflow_pattern");
    }

    #[test]
    fn success_confirmed_hypothesis_when_rationale_endorses_it() {
        let v = mk_verdict(9, "hypothesis confirmed by green tests");
        let mut ck = mk_checkpoint("debug flake");
        ck.current_hypothesis = Some("race in setup".to_string());
        ck.append_step("added sync", None, true);
        let doc = extract_skill_from_success(&v, &ck).expect("should extract");
        assert_eq!(doc.frontmatter.name, "confirmed_hypothesis");
        assert!(doc.body.contains("Confirmed hypothesis"));
    }

    #[test]
    fn success_stuck_pattern_resolved_when_dead_ends_present_but_success() {
        let v = mk_verdict(7, "shipped after backtracking past two false leads");
        let mut ck = mk_checkpoint("debug flake");
        ck.record_dead_end("TZ", "not it");
        ck.append_step("identified real fixture race", None, true);
        let doc = extract_skill_from_success(&v, &ck).expect("should extract");
        assert_eq!(doc.frontmatter.name, "stuck_pattern_resolved");
        assert!(doc.body.contains("Dead ends already ruled out"));
    }

    #[test]
    fn round_trip_extracted_failure_skill_serialize_parse_equal() {
        let v = mk_verdict(2, "agent kept retrying the failing build");
        let mut ck = mk_checkpoint("restore green build");
        ck.append_step("cargo check", Some("cargo".into()), false);
        ck.append_step("cargo check", Some("cargo".into()), false);
        ck.record_dead_end("missing import", "import exists but feature gated");

        let doc = extract_skill(&v, &ck).expect("should extract");
        let serialized = serialize_skill(&doc).expect("serialize ok");
        let reparsed = crate::skillbank::skill::parse_str(&serialized).expect("re-parse ok");
        assert_eq!(reparsed, doc, "extract → serialize → parse must round-trip");
    }

    #[test]
    fn round_trip_extracted_success_skill_serialize_parse_equal() {
        let v = mk_verdict(9, "shipped clean fix");
        let mut ck = mk_checkpoint("ship a fix");
        ck.append_step("applied patch", Some("apply_patch".into()), true);

        let doc = extract_skill(&v, &ck).expect("should extract");
        let serialized = serialize_skill(&doc).expect("serialize ok");
        let reparsed = crate::skillbank::skill::parse_str(&serialized).expect("re-parse ok");
        assert_eq!(reparsed, doc, "success-side must also round-trip");
    }

    #[test]
    fn custom_threshold_routes_score_relative_to_threshold() {
        // Score = 6. With threshold=8 we route to failure-side; with
        // threshold=5 (default) we route to success-side.
        let v = mk_verdict(6, "some retry happened");
        let mut ck = mk_checkpoint("g");
        ck.append_step("did a thing", None, true);
        ck.record_dead_end("a", "b");

        let success_side = extract_skill(&v, &ck).expect("success route");
        assert!(success_side
            .frontmatter
            .tags
            .iter()
            .any(|t| t == "provenance:success"));

        let failure_side =
            extract_skill_with_threshold(&v, &ck, 8).expect("failure route with threshold=8");
        assert!(failure_side
            .frontmatter
            .tags
            .iter()
            .any(|t| t == "provenance:failure"));
    }

    #[test]
    fn frontmatter_tags_carry_rubric_target_and_provenance_for_failure() {
        let v = mk_verdict(3, "agent got stuck");
        let mut ck = mk_checkpoint("g");
        ck.target = "test".to_string();
        ck.record_dead_end("h", "w");
        let doc = extract_skill(&v, &ck).expect("ok");
        let tags = &doc.frontmatter.tags;
        assert!(tags.iter().any(|t| t == "auto-extracted"));
        assert!(tags.iter().any(|t| t == "rubric:h1-v1"));
        assert!(tags.iter().any(|t| t == "score:3"));
        assert!(tags.iter().any(|t| t == "target:test"));
        assert!(tags.iter().any(|t| t == "provenance:failure"));
    }

    #[test]
    fn frontmatter_tags_carry_provenance_success_for_success_path() {
        let v = mk_verdict(8, "clean");
        let mut ck = mk_checkpoint("g");
        ck.target = "check".to_string();
        ck.append_step("did the work", None, true);
        let doc = extract_skill(&v, &ck).expect("ok");
        let tags = &doc.frontmatter.tags;
        assert!(tags.iter().any(|t| t == "provenance:success"));
        assert!(tags.iter().any(|t| t == "score:8"));
        assert!(tags.iter().any(|t| t == "target:check"));
    }

    #[test]
    fn fmt_iso8601_known_epoch_anchors() {
        assert_eq!(fmt_iso8601(0), "1970-01-01T00:00:00Z");
        let s = fmt_iso8601(1_715_000_000_000);
        assert!(s.starts_with("2024-05-06T"), "got: {}", s);
        assert!(s.ends_with("Z"));
    }
}
