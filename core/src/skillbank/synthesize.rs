//! Top-down skill synthesis: a natural-language goal -> a verified
//! `SkillDocument`. This is the counterpart to `extract.rs` (bottom-up:
//! trajectory -> skill). Where the extractor *distills* a skill from an
//! already-run evolve session, the synthesizer *proposes* one from a stated
//! goal, then closes the loop by running the proposal in the sandboxed
//! executor and iterating on the diagnostics until it verifies (or the round
//! budget is exhausted).
//!
//! Pipeline:
//!   1. `propose_skill_from_goal` — ask an `LlmProvider` for a SKILL.md,
//!      extract the fenced markdown block, `parse_str` it.
//!   2. `verify_skill` — run `SkillExecutor::execute` in `Sandboxed` mode
//!      (allowlist = `goal.allowed_commands`); pass = no executor errors and
//!      (when `require_clean_run`) no non-zero bash exits.
//!   3. `synthesize_skill` — the propose -> dry-run -> live-run -> iterate
//!      loop. On success, `store_skill` writes the serialized SKILL.md into
//!      the FTS5-backed `SkillMemory` (kind = "skill").
//!
//! CLI wiring (`spectyn skill new "<goal>"`) and provider/api_key resolution
//! are intentionally deferred to a later slice; this module takes the
//! provider + key as parameters so the caller owns that resolution.
//!
//! Gated behind BOTH `experimental-curator` (skill parser + executor)
//! and `experimental-memory` (the store), mirroring how the
//! `integration` façade composes feature tracks.

#![cfg(all(
    feature = "experimental-curator",
    feature = "experimental-memory"
))]

use std::sync::Arc;

use crate::skillbank::memory::{SkillMemory, NewMemory};
use crate::skillbank::skill::{parse_str, serialize, SkillDocument};
use crate::skillbank::skill_executor::{
    ExecutionMode, ExecutionOpts, SkillExecutionResult, SkillExecutor, StepOutcome,
};
use crate::providers::llm_provider::LlmProvider;
use crate::providers::traits::ChatMessage;

/// The user's natural-language goal plus the constraints the synthesizer must
/// honour. `allowed_commands` is the sandbox allowlist handed to the executor
/// (a bash step whose program isn't on this list is rejected, never spawned).
#[derive(Debug, Clone)]
pub struct SkillGoal {
    /// Plain-language description of what the skill should accomplish.
    pub goal: String,
    /// Sandbox allowlist — only these program names may appear as the first
    /// token of a bash step (see `skill_executor::parse_sandboxed_argv`).
    pub allowed_commands: Vec<String>,
    /// Optional preferences (preferred tools, output formats, etc.) folded
    /// into the proposal prompt.
    pub hints: Vec<String>,
}

/// Loop + model configuration for a synthesis run.
#[derive(Debug, Clone)]
pub struct SynthConfig {
    /// Model id passed to `LlmProvider::complete`.
    pub model: String,
    /// Maximum propose -> verify rounds before giving up.
    pub max_rounds: u8,
    /// When true, a proposal only passes if every bash step exits 0 (and the
    /// executor reports no errors). When false, a proposal passes as long as
    /// it ran at least one step without an executor-level error.
    pub require_clean_run: bool,
    /// When true, each proposal is dry-run first; a dry-run that already shows
    /// errors (e.g. a sandbox rejection, which is evaluated even in dry-run
    /// for the allowlist parse) short-circuits to the next round.
    pub dry_run_first: bool,
}

impl Default for SynthConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-6".to_string(),
            max_rounds: 3,
            require_clean_run: true,
            dry_run_first: true,
        }
    }
}

/// The result of a successful synthesis: the verified document, how many
/// rounds it took, and the store rowid (if the caller's store accepted it).
#[derive(Debug, Clone)]
pub struct SynthOutcome {
    pub doc: SkillDocument,
    pub rounds_used: u8,
    pub stored_id: Option<i64>,
}

/// Failure modes of the synthesis loop.
#[derive(Debug, thiserror::Error)]
pub enum SynthError {
    /// The model never returned a parseable SKILL.md.
    #[error("model proposed no parseable SKILL.md")]
    NoValidProposal,
    /// A proposal was produced but never verified within the round budget.
    /// `.0` = rounds used, `.1` = last diagnostics.
    #[error("verification never passed after {0} round(s); last diagnostics: {1}")]
    NeverVerified(u8, String),
    /// The provider call itself failed.
    #[error("provider error: {0}")]
    Provider(String),
    /// Persisting the verified skill failed.
    #[error("store error: {0}")]
    Store(String),
}

// ── ① Propose ──────────────────────────────────────────────────────────────

/// Ask the model for a SKILL.md that achieves `goal`, optionally feeding back
/// the previous round's diagnostics (`prior_failure`) so it can self-correct.
/// Extracts the fenced markdown block from the reply and parses it.
pub async fn propose_skill_from_goal(
    provider: &Arc<dyn LlmProvider>,
    api_key: &str,
    cfg: &SynthConfig,
    goal: &SkillGoal,
    prior_failure: Option<&str>,
) -> Result<SkillDocument, SynthError> {
    let messages = build_propose_messages(goal, prior_failure);
    let (reply, _raw) = provider
        .complete(api_key, &cfg.model, &messages, &[])
        .await
        .map_err(|e| SynthError::Provider(e.to_string()))?;
    let md = extract_skill_md_block(&reply.content);
    parse_str(&md).map_err(|_| SynthError::NoValidProposal)
}

// ── ③ Execute + Verify ──────────────────────────────────────────────────────

/// Run a proposal in `Sandboxed` mode and decide whether it meets the goal.
///
/// Returns `(passed, diagnostics)`. `passed` is true when the executor
/// reports no errors AND (when `cfg.require_clean_run`) no `BashRan` step
/// exited non-zero. `diagnostics` summarizes the failing outcomes so the next
/// proposal round can be steered.
pub fn verify_skill(
    doc: &SkillDocument,
    goal: &SkillGoal,
    cfg: &SynthConfig,
    dry_run: bool,
) -> (bool, String) {
    let opts = ExecutionOpts {
        dry_run,
        mode: ExecutionMode::Sandboxed {
            allowed_commands: goal.allowed_commands.clone(),
        },
        ..Default::default()
    };

    let result = match SkillExecutor::execute(doc, opts) {
        Ok(r) => r,
        Err(e) => return (false, format!("executor refused to run: {e}")),
    };

    let diagnostics = summarize_outcomes(&result);

    let no_bad_exit = result.outcomes.iter().all(|o| {
        !matches!(
            o,
            StepOutcome::BashRan { exit_code, .. } if *exit_code != 0
        )
    });
    let clean = result.errors.is_empty() && no_bad_exit;

    let passed = if cfg.require_clean_run {
        clean
    } else {
        // Looser bar: at least one step ran and the executor itself didn't
        // error out. Sandbox rejections / spawn failures populate `errors`.
        result.errors.is_empty() && result.steps_run > 0
    };

    (passed, diagnostics)
}

// ── Loop: propose -> verify -> iterate ──────────────────────────────────────

/// Drive the full synthesis loop. On a verified proposal, persist it via
/// `store_skill` and return the outcome (with the assigned store id). On
/// exhaustion, return `SynthError::NeverVerified` carrying the last
/// diagnostics.
pub async fn synthesize_skill(
    provider: &Arc<dyn LlmProvider>,
    api_key: &str,
    mem: &SkillMemory,
    cfg: &SynthConfig,
    goal: &SkillGoal,
) -> Result<SynthOutcome, SynthError> {
    let mut prior_failure: Option<String> = None;

    for round in 1..=cfg.max_rounds {
        // ① + ② Propose & parse. A non-parseable proposal is itself a
        // recoverable failure: feed that back and try again unless we're out
        // of rounds.
        let doc = match propose_skill_from_goal(
            provider,
            api_key,
            cfg,
            goal,
            prior_failure.as_deref(),
        )
        .await
        {
            Ok(d) => d,
            Err(SynthError::Provider(e)) => return Err(SynthError::Provider(e)),
            Err(_) if round < cfg.max_rounds => {
                prior_failure =
                    Some("previous reply was not a valid SKILL.md; re-emit a single ```markdown fenced block with `---` YAML frontmatter then a body".to_string());
                continue;
            }
            Err(e) => return Err(e),
        };

        // ③ Optional dry-run gate (catches sandbox-allowlist rejections
        // before we spend a live run).
        if cfg.dry_run_first {
            let (passed, diag) = verify_skill(doc_ref(&doc), goal, cfg, true);
            if !passed {
                prior_failure = Some(diag);
                continue;
            }
        }

        // Live run.
        let (passed, diag) = verify_skill(&doc, goal, cfg, false);
        if passed {
            let id = store_skill(mem, &doc, goal)
                .await
                .map_err(SynthError::Store)?;
            return Ok(SynthOutcome {
                doc,
                rounds_used: round,
                stored_id: Some(id),
            });
        }
        prior_failure = Some(diag);
    }

    Err(SynthError::NeverVerified(
        cfg.max_rounds,
        prior_failure.unwrap_or_else(|| "no diagnostics captured".to_string()),
    ))
}

/// Tiny borrow helper so the dry-run call site reads symmetrically with the
/// live call. (Keeps the borrow explicit; no behavioural difference.)
#[inline]
fn doc_ref(doc: &SkillDocument) -> &SkillDocument {
    doc
}

// ── ④ Store ──────────────────────────────────────────────────────────────────

/// Serialize the verified document and persist it as a `kind="skill"` memory
/// row. Tags come from the frontmatter so the FTS5 store stays searchable by
/// the same tags the skill declares.
async fn store_skill(
    mem: &SkillMemory,
    doc: &SkillDocument,
    _goal: &SkillGoal,
) -> Result<i64, String> {
    let text = serialize(doc).map_err(|e| e.to_string())?;
    let tags = doc.frontmatter.tags.join(",");
    mem.insert(NewMemory {
        kind: "skill",
        source: "synthesize",
        text: &text,
        tags: &tags,
    })
    .await
    .map_err(|e| e.to_string())
}

// ── Pure helpers ─────────────────────────────────────────────────────────────

const PROPOSE_SYSTEM_PROMPT: &str = "\
You produce a Skill Document (Anthropic SKILL.md compatible). Output \
EXACTLY ONE ```markdown fenced code block and nothing else. Inside it put a \
`---`-fenced YAML frontmatter (fields: name, version, description, triggers, \
tools, inputs, outputs, tags) followed by a Markdown body. The body is an \
ordered set of steps: a ```bash code block is an executable step, a line \
starting with `> note:` records context, and a `## Prompt:` section defers \
back to the agent.

The skill will be executed in SANDBOXED mode. Every bash step MUST be a \
single line with NO pipes, NO redirects, NO command substitution, NO env-var \
expansion, NO quoting, and NO statement chaining (`;`, `&&`, `||`, `&`). The \
first token of each bash step MUST be one of the allowed commands listed by \
the user. Keep the skill minimal and verifiable.";

/// Build the `[system, user]` message pair for a proposal request. When
/// `prior_failure` is `Some`, a third user message carrying the diagnostics
/// is appended so the model can repair its previous attempt.
fn build_propose_messages(goal: &SkillGoal, prior_failure: Option<&str>) -> Vec<ChatMessage> {
    let mut messages = Vec::with_capacity(3);

    messages.push(ChatMessage {
        role: "system".to_string(),
        content: PROPOSE_SYSTEM_PROMPT.to_string(),
        tool_calls: None,
    });

    let allowed = if goal.allowed_commands.is_empty() {
        "(none — use only `## Prompt:` and `> note:` steps)".to_string()
    } else {
        goal.allowed_commands.join(", ")
    };
    let hints = if goal.hints.is_empty() {
        "(none)".to_string()
    } else {
        goal.hints.join("; ")
    };
    let user = format!(
        "Goal:\n{goal}\n\nAllowed commands (sandbox allowlist): {allowed}\n\nPreferences: {hints}\n\nProduce the minimal verifiable skill that achieves this goal.",
        goal = goal.goal,
    );
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: user,
        tool_calls: None,
    });

    if let Some(diag) = prior_failure {
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: format!(
                "The previous version failed verification. Diagnostics:\n{diag}\n\nFix the issue and re-emit the ENTIRE SKILL.md as a single ```markdown fenced block."
            ),
            tool_calls: None,
        });
    }

    messages
}

/// Extract the SKILL.md text from an assistant reply. Looks for a fenced code
/// block — preferring a ```markdown ... ``` fence, falling back to the first
/// bare ``` ... ``` fence — and returns its inner content. If no fence is
/// present, returns the whole reply trimmed (the model may have emitted the
/// raw `---`-fenced document directly).
fn extract_skill_md_block(reply: &str) -> String {
    // Prefer an explicit ```markdown (or ```md) fence.
    for tag in ["```markdown", "```md"] {
        if let Some(inner) = fenced_block_after(reply, tag) {
            return inner;
        }
    }
    // Fall back to the first bare ``` fence whose info-string isn't another
    // language we'd misattribute. We accept the first ``` opening regardless
    // of info string and take until the next ```.
    if let Some(inner) = first_bare_fenced_block(reply) {
        return inner;
    }
    reply.trim().to_string()
}

/// Find the content of a fenced block opened by exactly `open_tag` (e.g.
/// "```markdown"). The opener must be at the start of a line; content runs
/// until the next line that is exactly "```".
fn fenced_block_after(text: &str, open_tag: &str) -> Option<String> {
    let mut lines = text.lines();
    // Advance to the opener line.
    let mut found_open = false;
    let mut collected: Vec<&str> = Vec::new();
    while let Some(line) = lines.next() {
        if !found_open {
            if line.trim_end() == open_tag || line.trim_end().starts_with(open_tag) {
                // Guard against "```markdownish" — require the char after the
                // tag (if any) to be whitespace.
                let rest = line.trim_end().strip_prefix(open_tag).unwrap_or("");
                if rest.is_empty() || rest.chars().all(char::is_whitespace) {
                    found_open = true;
                }
            }
            continue;
        }
        if line.trim_end() == "```" {
            return Some(collected.join("\n"));
        }
        collected.push(line);
    }
    // Opener found but no closer: return what we have (better than nothing).
    if found_open {
        Some(collected.join("\n"))
    } else {
        None
    }
}

/// Find the first ``` ... ``` block (any info string) and return its inner
/// content.
fn first_bare_fenced_block(text: &str) -> Option<String> {
    let mut lines = text.lines();
    let mut found_open = false;
    let mut collected: Vec<&str> = Vec::new();
    while let Some(line) = lines.next() {
        let t = line.trim_end();
        if !found_open {
            if t.starts_with("```") {
                found_open = true;
            }
            continue;
        }
        if t == "```" {
            return Some(collected.join("\n"));
        }
        collected.push(line);
    }
    if found_open {
        Some(collected.join("\n"))
    } else {
        None
    }
}

/// Summarize the failing outcomes of an execution result into a compact,
/// model-readable diagnostics string. Empty string when nothing is wrong.
fn summarize_outcomes(result: &SkillExecutionResult) -> String {
    let mut lines: Vec<String> = Vec::new();

    for (idx, outcome) in result.outcomes.iter().enumerate() {
        match outcome {
            StepOutcome::BashRan {
                exit_code,
                stderr,
                ..
            } if *exit_code != 0 => {
                let tail = stderr.trim();
                if tail.is_empty() {
                    lines.push(format!("step {idx}: bash exited {exit_code}"));
                } else {
                    lines.push(format!("step {idx}: bash exited {exit_code}: {tail}"));
                }
            }
            StepOutcome::BashError { message } => {
                lines.push(format!("step {idx}: bash error: {message}"));
            }
            StepOutcome::BashRejected { reason } => {
                lines.push(format!("step {idx}: sandbox rejected: {reason}"));
            }
            _ => {}
        }
    }

    // Surface any executor-level errors not already captured per-step.
    for err in &result.errors {
        let already = lines.iter().any(|l| l.contains(err.as_str()));
        if !already {
            lines.push(format!("executor: {err}"));
        }
    }

    lines.join("\n")
}

// ── Tests (pure — no live LLM / provider / network) ──────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skillbank::skill::{SkillDocument, SkillFrontmatter};
    use std::collections::BTreeMap;

    fn goal_with(allowed: &[&str]) -> SkillGoal {
        SkillGoal {
            goal: "say hello".to_string(),
            allowed_commands: allowed.iter().map(|s| s.to_string()).collect(),
            hints: vec![],
        }
    }

    fn doc_with_bash(body_bash: &str) -> SkillDocument {
        SkillDocument {
            frontmatter: SkillFrontmatter {
                name: "trivial".to_string(),
                version: "0.1.0".to_string(),
                description: "a trivial skill".to_string(),
                triggers: vec!["test".to_string()],
                tools: vec![],
                inputs: BTreeMap::new(),
                outputs: vec![],
                tags: vec!["t1".to_string(), "t2".to_string()],
                created_at: None,
                author: None,
            },
            body: format!("```bash\n{body_bash}\n```\n"),
        }
    }

    // ── extract_skill_md_block ──────────────────────────────────────────────

    #[test]
    fn extract_skill_md_block_prefers_markdown_fence() {
        let reply = "Here is your skill:\n\n```markdown\n---\nname: x\n---\nbody\n```\n\nDone.";
        let got = extract_skill_md_block(reply);
        assert_eq!(got, "---\nname: x\n---\nbody");
    }

    #[test]
    fn extract_skill_md_block_falls_back_to_bare_fence() {
        let reply = "intro\n```\n---\nname: y\n---\nb\n```\ntrailer";
        let got = extract_skill_md_block(reply);
        assert_eq!(got, "---\nname: y\n---\nb");
    }

    #[test]
    fn extract_skill_md_block_md_alias() {
        let reply = "```md\nhello\nworld\n```";
        assert_eq!(extract_skill_md_block(reply), "hello\nworld");
    }

    #[test]
    fn extract_skill_md_block_no_fence_returns_whole_trimmed() {
        let reply = "   ---\nname: z\n---\nbody\n   ";
        let got = extract_skill_md_block(reply);
        assert_eq!(got, "---\nname: z\n---\nbody");
    }

    #[test]
    fn extract_skill_md_block_unclosed_fence_returns_remainder() {
        let reply = "```markdown\nline1\nline2";
        let got = extract_skill_md_block(reply);
        assert_eq!(got, "line1\nline2");
    }

    // ── build_propose_messages ──────────────────────────────────────────────

    #[test]
    fn build_propose_messages_system_then_user_no_prior() {
        let goal = goal_with(&["echo"]);
        let msgs = build_propose_messages(&goal, None);
        assert_eq!(msgs.len(), 2, "system + user when no prior failure");
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[1].role, "user");
        assert!(msgs[0].content.contains("Skill Document"));
        assert!(
            msgs[1].content.contains("echo"),
            "user message should list the allowlist: {}",
            msgs[1].content
        );
        assert!(msgs[1].content.contains("say hello"));
    }

    #[test]
    fn build_propose_messages_appends_prior_failure() {
        let goal = goal_with(&["echo"]);
        let msgs = build_propose_messages(&goal, Some("step 0: bash exited 7"));
        assert_eq!(msgs.len(), 3, "system + user + repair user");
        assert_eq!(msgs[2].role, "user");
        assert!(
            msgs[2].content.contains("step 0: bash exited 7"),
            "diagnostics must be fed back: {}",
            msgs[2].content
        );
    }

    #[test]
    fn build_propose_messages_empty_allowlist_hint() {
        let goal = goal_with(&[]);
        let msgs = build_propose_messages(&goal, None);
        assert!(
            msgs[1].content.contains("none"),
            "empty allowlist should be signalled to the model: {}",
            msgs[1].content
        );
    }

    // ── verify_skill (uses the REAL SkillExecutor, harmless `echo`) ─────────

    #[test]
    fn verify_skill_dry_run_passes_with_allowed_command() {
        // Dry-run never spawns, so it passes as long as the executor produced
        // outcomes without errors.
        let doc = doc_with_bash("echo hello");
        let goal = goal_with(&["echo"]);
        let cfg = SynthConfig::default();
        let (passed, diag) = verify_skill(&doc, &goal, &cfg, true);
        assert!(passed, "dry-run of a single echo step should pass: {diag}");
        assert!(diag.is_empty(), "no diagnostics expected, got: {diag}");
    }

    #[test]
    fn verify_skill_live_run_passes_for_zero_exit() {
        let doc = doc_with_bash("echo spectyn-synth-ok");
        let goal = goal_with(&["echo"]);
        let cfg = SynthConfig::default();
        let (passed, diag) = verify_skill(&doc, &goal, &cfg, false);
        assert!(passed, "live echo (exit 0) should pass: {diag}");
    }

    #[test]
    fn verify_skill_fails_when_command_not_on_allowlist() {
        // `echo` is NOT on the allowlist -> sandbox rejection -> not clean.
        let doc = doc_with_bash("echo nope");
        let goal = goal_with(&["ls"]); // echo absent on purpose
        let cfg = SynthConfig::default();
        let (passed, diag) = verify_skill(&doc, &goal, &cfg, false);
        assert!(!passed, "rejected command must fail verification");
        assert!(
            diag.contains("rejected") || diag.contains("allowlist"),
            "diagnostics should explain the rejection: {diag}"
        );
    }

    #[test]
    fn verify_skill_fails_on_nonzero_exit_when_require_clean() {
        // `false` is a real binary that exits 1.
        let doc = doc_with_bash("false");
        let goal = goal_with(&["false"]);
        let cfg = SynthConfig {
            require_clean_run: true,
            ..SynthConfig::default()
        };
        let (passed, diag) = verify_skill(&doc, &goal, &cfg, false);
        assert!(!passed, "non-zero exit must fail under require_clean_run");
        assert!(
            diag.contains("exited") || !diag.is_empty(),
            "should report the failing exit: {diag}"
        );
    }

    #[test]
    fn synth_config_defaults() {
        let c = SynthConfig::default();
        assert_eq!(c.max_rounds, 3);
        assert!(c.require_clean_run);
        assert!(c.dry_run_first);
        assert!(!c.model.is_empty());
    }
}
