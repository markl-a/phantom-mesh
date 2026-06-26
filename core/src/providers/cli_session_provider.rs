//! Provider that executes a completion by driving a LOCAL AI CLI (codex / opencode
//! / agy / claude) through the L0 `cli_session` substrate. The generalisation of
//! `claude_agent` (which only drives `claude -p`) to all four CLIs, reusing L0's
//! per-CLI parsing + timeout watchdog. Non-streaming: the agent routes it via
//! `complete()`. This is the SANCTIONED-CLI path (shells out to the official CLI),
//! distinct from the OAuth-token-reuse `codex_cli`/`claude_cli` discovery modules.

use crate::cli_session::event::{CliEvent, EventKind};
use crate::cli_session::{self, CliKind, SessionSpec, TurnInput};
use crate::providers::llm_provider::{BuildRequestOpts, BuildRequestParts, LlmProvider};
use crate::providers::traits::{ChatMessage, ProviderError};
use async_trait::async_trait;
use serde_json::Value;

/// Fold an L0 event stream into `(assistant text, usage json)`; an `Error` event
/// aborts. `usage` is `Null` when the CLI emitted no `Usage` event.
fn fold_events<I: IntoIterator<Item = CliEvent>>(
    events: I,
) -> Result<(String, Value), ProviderError> {
    let mut text = String::new();
    let mut usage = Value::Null;
    for ev in events {
        match ev.event {
            EventKind::AssistantText { delta } => text.push_str(&delta),
            EventKind::Usage { input_tokens, output_tokens, cost_usd } => {
                usage = serde_json::json!({
                    "input_tokens": input_tokens,
                    "output_tokens": output_tokens,
                    "cost_usd": cost_usd,
                });
            }
            EventKind::Error { error_kind, detail } => {
                return Err(ProviderError::Unknown(format!(
                    "cli error [{error_kind}]: {detail}"
                )));
            }
            _ => {}
        }
    }
    Ok((text, usage))
}

/// Extract the LAST user message's text from agent-runtime `Value` messages
/// (content may be a plain string or an array of text parts). Used by the agent
/// runtime short-circuit — we pass the raw user task to the agentic CLI, NOT a
/// system-prompt-prepended render (the CLI has its own agent behaviour; prepending
/// phantom's system prompt makes codex/agy answer the framing, not the task).
pub(crate) fn last_user_text(messages: &[Value]) -> String {
    let text_of = |m: &Value| -> String {
        if let Some(s) = m.get("content").and_then(|c| c.as_str()) {
            return s.to_string();
        }
        m.get("content")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default()
    };
    messages
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .map(text_of)
        .unwrap_or_default()
}

/// Map an agent-runtime provider key to the CLI it drives (`None` for keys that
/// aren't cli_session providers). Used by the agent runtime's non-streaming
/// short-circuit (mirrors how `claude_agent` is special-cased).
pub(crate) fn cli_for_provider_key(key: &str) -> Option<CliKind> {
    match key {
        "codex_agent" => Some(CliKind::Codex),
        "opencode_agent" => Some(CliKind::Opencode),
        "agy_agent" => Some(CliKind::Agy),
        // governed claude via cli_session (apex-④ pre-action gate); distinct from
        // the ungoverned `claude_agent` (= `claude -p` complete()).
        "claude_session" => Some(CliKind::Claude),
        key if crate::cli_session::external_gateway::lookup(key).is_some() => {
            Some(CliKind::External(crate::cli_session::external_gateway::lookup(key).unwrap()))
        }
        _ => None,
    }
}

fn cli_short(cli: CliKind) -> &'static str {
    match cli {
        CliKind::Claude => "claude",
        CliKind::Codex => "codex",
        CliKind::Opencode => "opencode",
        CliKind::Agy => "agy",
        CliKind::External(spec) => spec.program,
    }
}

/// Who commits/pushes the worker's changes after the CLI edits the repo.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GitMode {
    /// phantom runs git deterministically (default — the CLI is flaky at git).
    Phantom,
    /// the CLI handles its own git (the prompt told it to); phantom skips git.
    Cli,
}

struct GitDirective {
    branch: Option<String>,
    mode: GitMode,
}

/// An optional FIRST line `[phantom-git branch=<name> mode=phantom|cli]` lets the
/// dispatched task pick the branch + who runs git. Returns the directive + the
/// prompt with that header line stripped. Absent → phantom mode, auto branch.
fn parse_git_directive(prompt: &str) -> (GitDirective, String) {
    let first = prompt.lines().next().unwrap_or("").trim();
    if first.starts_with("[phantom-git") && first.ends_with(']') {
        let inner = &first[1..first.len() - 1];
        let mut branch = None;
        let mut mode = GitMode::Phantom;
        for tok in inner.split_whitespace() {
            if let Some(b) = tok.strip_prefix("branch=") {
                if !b.is_empty() {
                    branch = Some(b.to_string());
                }
            }
            if let Some(m) = tok.strip_prefix("mode=") {
                mode = if m == "cli" { GitMode::Cli } else { GitMode::Phantom };
            }
        }
        let rest = prompt.splitn(2, '\n').nth(1).unwrap_or("").to_string();
        return (GitDirective { branch, mode }, rest);
    }
    (GitDirective { branch: None, mode: GitMode::Phantom }, prompt.to_string())
}

/// Run one git command in `repo`; error carries stderr.
fn git(repo: &std::path::Path, args: &[&str]) -> Result<String, ProviderError> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| ProviderError::Unknown(format!("git {args:?}: {e}")))?;
    if !out.status.success() {
        return Err(ProviderError::Unknown(format!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// After the CLI edits `repo`, commit any changes onto `branch` (auto-named if
/// None) and push to `origin`. Returns a summary, or None if nothing changed.
fn git_commit_push(
    repo: &std::path::Path,
    branch: Option<&str>,
    cli: CliKind,
) -> Result<Option<String>, ProviderError> {
    if git(repo, &["status", "--porcelain"])?.trim().is_empty() {
        return Ok(None); // no edits — a Q&A task, nothing to commit
    }
    let branch = branch.map(str::to_string).unwrap_or_else(|| {
        let id = uuid::Uuid::new_v4().simple().to_string();
        format!("cli-session/{}/{}", cli_short(cli), &id[..8])
    });
    git(repo, &["checkout", "-B", &branch])?;
    git(repo, &["add", "-A"])?;
    git(
        repo,
        &[
            "-c",
            "core.hooksPath=", // skip repo hooks for an unattended worker commit
            "commit",
            "-m",
            &format!("cli-session ({}) automated change", cli_short(cli)),
        ],
    )?;
    git(repo, &["push", "-u", "origin", &branch, "--no-verify"])?;
    let sha = git(repo, &["rev-parse", "--short", "HEAD"])?.trim().to_string();
    Ok(Some(format!("pushed branch '{branch}' @ {sha}")))
}

/// Drive a local AI CLI for a single prompt, returning `(assistant text, usage)`.
/// Runs L0 on a blocking thread (its transports are blocking + yield a sync
/// Receiver). Shared by `CliSessionProvider::complete` and the agent runtime's
/// non-streaming short-circuit.
///
/// REPO MODE: if `PHANTOM_CLI_SESSION_REPO` is set, the CLI runs IN that repo
/// (cwd = repo) so it can read/edit the code, and — unless the task's
/// `[phantom-git mode=cli]` directive says otherwise — phantom commits + pushes
/// the changes to a branch (the git/GitHub collaboration path). Unset → the CLI
/// runs in a neutral home dir (Q&A mode, no repo, no git).
pub(crate) async fn run_cli_session(
    cli: CliKind,
    prompt: String,
    model: Option<String>,
    timeout_secs: u64,
    // apex-④ dispatch↔govern correlation: when this run is driving a DISPATCHED
    // task, the dispatch row's `job_uuid` (see `AgentRuntime::with_dispatch_task_id`
    // / `serve.rs` `rpc_task_assign`). Threaded into `GovernConfig.dispatch_task_id`
    // so a governed run uses it AS the govern `task_id` (one correlation key) and an
    // approval raised mid-run stamps its `approval_id` onto the dispatch row live.
    // `None` (ungoverned, standalone `phantom govern`, or the bare provider path) =
    // a fresh govern id is minted as before — byte-identical behavior.
    dispatch_task_id: Option<uuid::Uuid>,
) -> Result<(String, Value), ProviderError> {
    let repo = std::env::var("PHANTOM_CLI_SESSION_REPO")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(std::path::PathBuf::from);
    let (directive, clean_prompt) = parse_git_directive(&prompt);
    let cwd = repo.clone().unwrap_or_else(|| {
        crate::providers::credential_scanner::home_dir_lenient()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    });

    // GOVERNED PATH (apex ④ — safe unattended runs): when `PHANTOM_GOVERN_CLI` is
    // truthy, the run is driven through L1 — every ToolCall is risk-classified,
    // the whole session is captured by the flight-recorder, and high-risk actions
    // escalate (codex/opencode/agy are PostActionObserved: record + alert +
    // abort-on-STOP). Unset → the original ungoverned fast path, unchanged (the
    // proven M1/M2 loopback behaviour).
    let (mut text, usage) = if govern_enabled() {
        run_governed(cli, clean_prompt, cwd, model, timeout_secs, dispatch_task_id).await?
    } else {
        run_ungoverned(cli, clean_prompt, cwd, model, timeout_secs).await?
    };

    // REPO MODE: phantom commits + pushes the worker's edits (unless the task asked
    // the CLI to do its own git). The push summary is appended so the master sees
    // the branch it must fetch/integrate. Shared by both governance paths.
    if let Some(repo_path) = repo {
        if directive.mode == GitMode::Phantom {
            let branch = directive.branch.clone();
            let note = tokio::task::spawn_blocking(move || {
                match git_commit_push(&repo_path, branch.as_deref(), cli) {
                    Ok(Some(info)) => format!("[phantom-git] {info}"),
                    Ok(None) => "[phantom-git] no file changes to commit".to_string(),
                    Err(e) => format!("[phantom-git] FAILED: {e}"),
                }
            })
            .await
            .map_err(|e| ProviderError::Unknown(format!("git join: {e}")))?;
            text = format!("{text}\n\n{note}");
        }
    }
    Ok((text, usage))
}

/// `PHANTOM_GOVERN_CLI` truthy → route worker CLI runs through L1 governance.
fn govern_enabled() -> bool {
    std::env::var("PHANTOM_GOVERN_CLI")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Original ungoverned fast path: drive L0 directly on a blocking thread + fold
/// the event stream into `(text, usage)`.
async fn run_ungoverned(
    cli: CliKind,
    prompt: String,
    cwd: std::path::PathBuf,
    model: Option<String>,
    timeout_secs: u64,
) -> Result<(String, Value), ProviderError> {
    tokio::task::spawn_blocking(move || -> Result<(String, Value), ProviderError> {
        let spec = SessionSpec::new(cli, cwd, timeout_secs, model);
        let mut session = cli_session::start(spec)
            .map_err(|e| ProviderError::Unknown(format!("cli_session start: {e}")))?;
        let rx = session
            .turn(TurnInput { prompt })
            .map_err(|e| ProviderError::Unknown(format!("cli_session turn: {e}")))?;
        fold_events(rx.into_iter())
    })
    .await
    .map_err(|e| ProviderError::Unknown(format!("cli_session join: {e}")))?
}

/// Governed path: drive L0 under L1 (flight-recorder + governor + phone
/// escalation), folding the assistant text out of the SAME stream the governor
/// consumes. A CLI `Error` event surfaces as a `ProviderError`; a non-`Completed`
/// governance outcome (operator STOP / a denied high-risk action) is annotated
/// onto the returned text so the master sees it.
///
/// Apex ④ REDIRECT RE-DISPATCH: when the operator STEERS the run (`Redirected`),
/// the carried instruction is not merely stringified + dropped — a fresh governed
/// pass is RE-DISPATCHED with that instruction as the new prompt (preserving
/// cwd / cli / model / timeout / policy). Bounded by [`DEFAULT_REDIRECT_CAP`]
/// re-dispatches so a pathological "steer → steer → steer …" cycle cannot loop
/// forever; on exhausting the cap the run ends with a CLEAR outcome carrying the
/// last steer (never a silent drop). The mirror of this loop over pure
/// `GovernedFold`s is `governed_run::drive_redirect_chain` (unit-tested).
async fn run_governed(
    cli: CliKind,
    prompt: String,
    cwd: std::path::PathBuf,
    model: Option<String>,
    timeout_secs: u64,
    dispatch_task_id: Option<uuid::Uuid>,
) -> Result<(String, Value), ProviderError> {
    use crate::governed_run::run::{GovernConfig, run_govern_folded};
    use crate::governed_run::{DEFAULT_REDIRECT_CAP, RunOutcome};

    // Run one governed L0 pass with `prompt`, preserving everything else.
    async fn one_pass(
        cli: CliKind,
        prompt: String,
        cwd: std::path::PathBuf,
        model: Option<String>,
        timeout_secs: u64,
        dispatch_task_id: Option<uuid::Uuid>,
    ) -> Result<(crate::governed_run::GovernedFold, uuid::Uuid), ProviderError> {
        let mut cfg = GovernConfig::new(cli, prompt);
        cfg.cwd = cwd;
        cfg.timeout_secs = timeout_secs;
        cfg.model = model;
        // apex-④: make the dispatch id the govern task_id (one correlation key).
        cfg.dispatch_task_id = dispatch_task_id;
        run_govern_folded(cfg)
            .await
            .map_err(|e| ProviderError::Unknown(format!("governed run: {e}")))
    }

    // REDIRECT RE-DISPATCH loop: the initial pass runs `prompt`; each operator
    // `Redirected(instruction)` RE-ENTERS the governed run with `instruction` as
    // the new prompt, up to `DEFAULT_REDIRECT_CAP` re-dispatches. The initial pass
    // does not count against the cap; only re-dispatches do.
    let mut current_prompt = prompt;
    let mut redirects: u32 = 0;
    loop {
        let (fold, task_id) = one_pass(
            cli,
            current_prompt.clone(),
            cwd.clone(),
            model.clone(),
            timeout_secs,
            dispatch_task_id,
        )
        .await?;
        if let Some((kind, detail)) = fold.error {
            return Err(ProviderError::Unknown(format!("cli error [{kind}]: {detail}")));
        }
        let mut text = fold.text;
        match fold.outcome {
            RunOutcome::Completed => return Ok((text, fold.usage)),
            RunOutcome::Aborted => {
                text = format!(
                    "{text}\n\n[governed] run ABORTED by operator STOP (flight-recorder task {task_id})"
                );
                return Ok((text, fold.usage));
            }
            RunOutcome::Denied => {
                text = format!(
                    "{text}\n\n[governed] a high-risk action was DENIED (flight-recorder task {task_id})"
                );
                return Ok((text, fold.usage));
            }
            RunOutcome::Redirected(instruction) => {
                // Apex ④ PHONE REDIRECT: the operator steered the run with a new
                // instruction instead of approving the pending high-risk action.
                // The pending tool did NOT run; RE-DISPATCH a fresh governed pass
                // with the carried instruction (bounded by the redirect cap).
                if redirects >= DEFAULT_REDIRECT_CAP {
                    text = format!(
                        "{text}\n\n[governed] run REDIRECTED by operator (flight-recorder task \
                         {task_id}) but the redirect-depth cap ({DEFAULT_REDIRECT_CAP}) was \
                         reached; STOPPING. last instruction (not re-dispatched): {instruction}"
                    );
                    return Ok((text, fold.usage));
                }
                redirects += 1;
                current_prompt = instruction;
                // loop: re-enter run_govern_folded with the new prompt.
            }
            RunOutcome::RedirectCapExhausted(instruction) => {
                // Defensive: a fold may already carry the terminal cap-exhausted
                // outcome (e.g. from the pure chain driver). Surface it clearly.
                text = format!(
                    "{text}\n\n[governed] redirect-depth cap reached (flight-recorder task \
                     {task_id}); STOPPING. last instruction (not re-dispatched): {instruction}"
                );
                return Ok((text, fold.usage));
            }
        }
    }
}

/// Drives `self.cli` (the LOCAL AI CLI) once per `complete()` via L0.
pub(crate) struct CliSessionProvider {
    cli: CliKind,
    timeout_secs: u64,
}

impl CliSessionProvider {
    pub fn new(cli: CliKind) -> Self {
        Self { cli, timeout_secs: 600 }
    }
    fn cli_name(&self) -> &'static str {
        match self.cli {
            CliKind::Claude => "claude",
            CliKind::Codex => "codex",
            CliKind::Opencode => "opencode",
            CliKind::Agy => "agy",
            CliKind::External(spec) => spec.program,
        }
    }
}

#[async_trait]
impl LlmProvider for CliSessionProvider {
    async fn complete(
        &self,
        _api_key: &str,
        model: &str,
        messages: &[ChatMessage],
        _tools: &[Value],
    ) -> Result<(ChatMessage, Value), ProviderError> {
        let prompt = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.clone())
            .ok_or_else(|| ProviderError::Unknown("cli_session: no user message".into()))?;
        let model_owned = (!model.is_empty()).then(|| model.to_string());
        // The bare provider path has no dispatch context (it's used by direct
        // LlmProvider callers, not the dispatch runtime), so no dispatch id.
        let (text, usage) =
            run_cli_session(self.cli, prompt, model_owned, self.timeout_secs, None).await?;

        let msg = ChatMessage { role: "assistant".into(), content: text, tool_calls: None };
        let raw = serde_json::json!({ "cli_session": self.cli_name(), "usage": usage });
        Ok((msg, raw))
    }

    async fn stream(
        &self,
        _api_key: &str,
        _model: &str,
        _messages: &[ChatMessage],
        _tools: &[Value],
    ) -> Result<reqwest::Response, ProviderError> {
        Err(ProviderError::Unknown(
            "cli_session is non-streaming; the agent routes it through complete()".into(),
        ))
    }

    fn provider_type(&self) -> &'static str {
        "cli_session"
    }

    fn build_stream_request(
        &self,
        _opts: &BuildRequestOpts<'_>,
    ) -> Result<BuildRequestParts, ProviderError> {
        Err(ProviderError::Unknown(
            "cli_session is non-streaming; route via complete()".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_session::event::{CliEvent, EventKind, Fidelity, Source};

    fn ev(k: EventKind) -> CliEvent {
        CliEvent::new(k, Fidelity::StructuredVerified, Source::LiveStream)
    }

    #[test]
    fn concats_assistant_text_and_ignores_other_events() {
        let evs = vec![
            ev(EventKind::SessionStarted { id: "s".into() }),
            ev(EventKind::AssistantText { delta: "hello ".into() }),
            ev(EventKind::AssistantText { delta: "world".into() }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ];
        let (text, usage) = fold_events(evs).unwrap();
        assert_eq!(text, "hello world");
        assert!(usage.is_null(), "no Usage event -> null");
    }

    #[test]
    fn captures_usage_when_present() {
        let evs = vec![
            ev(EventKind::AssistantText { delta: "hi".into() }),
            ev(EventKind::Usage { input_tokens: 5, output_tokens: 7, cost_usd: 0.01 }),
        ];
        let (text, usage) = fold_events(evs).unwrap();
        assert_eq!(text, "hi");
        assert_eq!(usage["input_tokens"], 5);
        assert_eq!(usage["output_tokens"], 7);
    }

    #[test]
    fn error_event_aborts_with_provider_error() {
        let evs = vec![ev(EventKind::Error {
            error_kind: "spawn".into(),
            detail: "not found".into(),
        })];
        assert!(fold_events(evs).is_err());
    }

    #[test]
    fn parse_git_directive_extracts_branch_mode_and_strips_header() {
        let (d, rest) = parse_git_directive("[phantom-git branch=feat/x mode=cli]\nDo the thing");
        assert_eq!(d.branch.as_deref(), Some("feat/x"));
        assert_eq!(d.mode, GitMode::Cli);
        assert_eq!(rest, "Do the thing");

        let (d2, rest2) = parse_git_directive("just a task, no header");
        assert!(d2.branch.is_none());
        assert_eq!(d2.mode, GitMode::Phantom);
        assert_eq!(rest2, "just a task, no header");
    }

    #[test]
    fn git_commit_push_returns_none_when_no_changes() {
        let dir = std::env::temp_dir().join(format!("clisess-git-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let init = std::process::Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["init", "-q"])
            .status();
        if init.map(|s| s.success()).unwrap_or(false) {
            // a fresh repo with no files has nothing to commit -> None (no push attempted).
            let r = git_commit_push(&dir, Some("test"), CliKind::Codex);
            assert!(matches!(r, Ok(None)), "no changes -> None, got {r:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Live: drives the REAL local codex via the provider. Ignored by default
    /// (needs codex installed). Windows-native only (codex is a Windows binary):
    ///   cargo test --lib providers::cli_session_provider -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "live: runs real codex"]
    async fn live_codex_completes() {
        let p = CliSessionProvider::new(CliKind::Codex);
        let msgs = vec![ChatMessage {
            role: "user".into(),
            content: "Reply with exactly one word: PONG".into(),
            tool_calls: None,
        }];
        let (msg, _raw) = p.complete("", "", &msgs, &[]).await.expect("codex completion");
        assert!(
            msg.content.to_uppercase().contains("PONG"),
            "got: {}",
            msg.content
        );
    }
}
