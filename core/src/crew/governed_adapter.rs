//! `GovernedCliAdapter` — phantom's OWN implementation of the ported crew
//! `Adapter` seam (this is new glue, not ported from ensemble). It connects
//! ensemble's conductor brain to phantom's hands: each crew turn drives a real
//! vendor CLI (codex / claude / agy / opencode, or a registered external
//! gateway) through phantom's `cli_session` L0 substrate and folds the
//! normalized event stream back into an `AgentOutput` / `AdapterError`.
//!
//! Slice-3a wires the cli_session drive + the (unit-tested) event→output fold.
//! The governed pass — routing the same event `Receiver` through
//! `governed_run::drive_fold` (governor + flight-recorder + phone escalation,
//! apex-④) — is wired by the `phantom crew` entry point (slice-3b), which owns
//! the recorder / EventStore / policy context that `drive_fold` requires.

use crate::cli_session::error::SessionError;
use crate::cli_session::event::{CliEvent, EventKind};
use crate::cli_session::{self, external_gateway, CliKind, SessionSpec, TurnInput};
use crate::crew::{detect_rate_limit, AdapterError, AgentOutput};
use crate::governed_run::run::{run_govern_folded, GovernConfig};
use crate::governed_run::{GovernPolicy, GovernedFold};
use std::path::Path;

/// Map a crew agent NAME to a phantom `CliKind`. The four first-class vendors map directly; any
/// other name is looked up in the external-gateway registry. `None` ⇒ unknown agent.
pub fn cli_kind_for(agent: &str) -> Option<CliKind> {
    match agent {
        "claude" => Some(CliKind::Claude),
        "codex" => Some(CliKind::Codex),
        "opencode" => Some(CliKind::Opencode),
        "agy" => Some(CliKind::Agy),
        other => external_gateway::lookup_by_name(other).map(CliKind::External),
    }
}

/// Translate an L0 `SessionError` (start/turn failure) into the crew degrade vocabulary. A missing
/// binary is `NotInstalled`; everything else (auth, spawn, timeout, transport) is a `Flaked` —
/// degrade signals the gate must treat as "this agent is unavailable", never as approval.
pub fn map_session_error(e: SessionError) -> AdapterError {
    match e {
        SessionError::CliNotFound(d) => AdapterError::NotInstalled(d),
        other => AdapterError::Flaked(other.to_string()),
    }
}

/// Fold a finished turn's normalized event stream into the crew result. Pure over the events so
/// the mapping is hermetically testable without a live CLI:
///   - concatenate every `AssistantText` delta into the reply text;
///   - a quota / rate-limit signal anywhere (in the text OR an `Error` line) takes PRECEDENCE →
///     `RateLimited` (the gate retries later, never counts it as a flake or an answer);
///   - otherwise an empty reply with an `Error` is a `Flaked`, and a bare empty reply is `Empty`.
pub fn fold_session_events(
    agent: &str,
    events: impl IntoIterator<Item = CliEvent>,
) -> Result<AgentOutput, AdapterError> {
    let mut text = String::new();
    // Accumulate EVERY error line (not just the last) — a rate-limit signal can arrive in an
    // earlier Error event and be followed by an unrelated one; keeping only the last would lose it
    // and misclassify a quota as a flake.
    let mut errors: Vec<String> = Vec::new();
    for ev in events {
        match ev.event {
            EventKind::AssistantText { delta } => text.push_str(&delta),
            EventKind::Error { error_kind, detail } => {
                errors.push(if detail.is_empty() {
                    error_kind
                } else {
                    format!("{error_kind}: {detail}")
                });
            }
            _ => {}
        }
    }
    classify_reply(agent, text, &errors.join("\n"))
}

/// Shared classification of a finished turn into the crew result, used by BOTH the plain
/// cli_session drive ([`fold_session_events`]) and the governed drive ([`map_governed_fold`]).
/// A quota/rate-limit anywhere (text OR the error blob) takes precedence → `RateLimited`; an empty
/// reply with an error is `Flaked`; a bare empty reply is `Empty`.
fn classify_reply(agent: &str, text: String, error_blob: &str) -> Result<AgentOutput, AdapterError> {
    let haystack = if error_blob.is_empty() {
        text.clone()
    } else {
        format!("{text}\n{error_blob}")
    };
    if let Some(info) = detect_rate_limit(&haystack) {
        return Err(AdapterError::RateLimited(info));
    }
    if text.trim().is_empty() {
        return if error_blob.is_empty() {
            Err(AdapterError::Empty)
        } else {
            Err(AdapterError::Flaked(error_blob.to_string()))
        };
    }
    Ok(AgentOutput {
        agent: agent.to_string(),
        text,
    })
}

/// Map a [`GovernedFold`] (the result of a governed `drive_fold` — assistant text already folded,
/// plus an optional `(kind, detail)` error) into the crew result, via the same [`classify_reply`]
/// rules as the plain drive.
pub fn map_governed_fold(agent: &str, fold: GovernedFold) -> Result<AgentOutput, AdapterError> {
    let error_blob = fold
        .error
        .map(|(kind, detail)| {
            if detail.is_empty() {
                kind
            } else {
                format!("{kind}: {detail}")
            }
        })
        .unwrap_or_default();
    classify_reply(agent, fold.text, &error_blob)
}

/// Per-turn governance context: when present, each crew turn is driven under the governor +
/// flight-recorder + phone escalation (apex-④) via `run_govern_folded`, instead of a plain
/// cli_session drive. Holds a tokio runtime `Handle` because `run_govern_folded` is async and the
/// `Adapter::run` seam is sync — the handle is used to `block_on` it (the conductor must therefore
/// run on a BLOCKING thread, e.g. under `tokio::task::spawn_blocking`, not an async worker).
#[derive(Clone)]
pub struct GovernanceCtx {
    handle: tokio::runtime::Handle,
    policy: GovernPolicy,
}

impl GovernanceCtx {
    pub fn new(handle: tokio::runtime::Handle, policy: GovernPolicy) -> Self {
        Self { handle, policy }
    }
}

/// A crew `Adapter` that drives a real vendor CLI through phantom's `cli_session` substrate.
/// Ungoverned by default (a plain drive); wire [`GovernedCliAdapter::with_governance`] to route
/// every turn through the governor + flight-recorder (apex-④).
pub struct GovernedCliAdapter {
    name: String,
    cli: CliKind,
    timeout_secs: u64,
    model: Option<String>,
    governance: Option<GovernanceCtx>,
}

impl GovernedCliAdapter {
    pub fn new(name: impl Into<String>, cli: CliKind, timeout_secs: u64, model: Option<String>) -> Self {
        Self {
            name: name.into(),
            cli,
            timeout_secs,
            model,
            governance: None,
        }
    }

    /// Build an adapter for a crew agent name (codex / claude / agy / opencode or a registered
    /// external gateway). `None` for an unknown agent.
    pub fn for_agent(agent: &str, timeout_secs: u64, model: Option<String>) -> Option<Self> {
        cli_kind_for(agent).map(|cli| Self::new(agent, cli, timeout_secs, model))
    }

    /// Route every turn through the governor + flight-recorder + phone escalation (apex-④).
    pub fn with_governance(mut self, ctx: GovernanceCtx) -> Self {
        self.governance = Some(ctx);
        self
    }

    /// The plain (ungoverned) cli_session drive — start a session, run one turn, fold the stream.
    fn run_plain(&self, prompt: &str, cwd: &Path) -> Result<AgentOutput, AdapterError> {
        let spec = SessionSpec::new(self.cli, cwd.to_path_buf(), self.timeout_secs, self.model.clone());
        let mut session = cli_session::start(spec).map_err(map_session_error)?;
        let rx = session
            .turn(TurnInput { prompt: prompt.to_string() })
            .map_err(map_session_error)?;
        // The event channel closes when the turn ends, so draining it collects the full turn.
        let events: Vec<CliEvent> = rx.into_iter().collect();
        fold_session_events(&self.name, events)
    }

    /// The governed drive — build a `GovernConfig` and run it under `run_govern_folded` (governor +
    /// flight-recorder + escalator), blocking on the supplied runtime handle.
    fn run_governed(&self, g: &GovernanceCtx, prompt: &str, cwd: &Path) -> Result<AgentOutput, AdapterError> {
        let mut cfg = GovernConfig::new(self.cli, prompt.to_string());
        cfg.cwd = cwd.to_path_buf();
        cfg.timeout_secs = self.timeout_secs;
        cfg.model = self.model.clone();
        cfg.policy = g.policy.clone();
        // `run_govern_folded` is async; `Adapter::run` is sync. block_on is legal here because the
        // conductor is driven on a blocking thread (the `phantom crew` CLI uses spawn_blocking).
        match g.handle.block_on(run_govern_folded(cfg)) {
            Ok((fold, _task_id)) => map_governed_fold(&self.name, fold),
            Err(e) => Err(AdapterError::Flaked(format!("governed run failed: {e}"))),
        }
    }
}

impl crate::crew::Adapter for GovernedCliAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn run(&self, prompt: &str, cwd: &Path) -> Result<AgentOutput, AdapterError> {
        match &self.governance {
            Some(g) => self.run_governed(g, prompt, cwd),
            None => self.run_plain(prompt, cwd),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_session::event::{Fidelity, Source};

    fn ev(kind: EventKind) -> CliEvent {
        CliEvent::new(kind, Fidelity::StructuredVerified, Source::LiveStream)
    }

    #[test]
    fn cli_kind_for_maps_the_four_vendors_and_rejects_unknown() {
        assert!(matches!(cli_kind_for("claude"), Some(CliKind::Claude)));
        assert!(matches!(cli_kind_for("codex"), Some(CliKind::Codex)));
        assert!(matches!(cli_kind_for("opencode"), Some(CliKind::Opencode)));
        assert!(matches!(cli_kind_for("agy"), Some(CliKind::Agy)));
        assert!(cli_kind_for("totally-unknown-agent").is_none());
    }

    #[test]
    fn map_session_error_distinguishes_missing_binary_from_a_flake() {
        assert!(matches!(
            map_session_error(SessionError::CliNotFound("agy".into())),
            AdapterError::NotInstalled(_)
        ));
        assert!(matches!(
            map_session_error(SessionError::NotAuthenticated("login first".into())),
            AdapterError::Flaked(_)
        ));
        assert!(matches!(
            map_session_error(SessionError::Timeout("slow".into())),
            AdapterError::Flaked(_)
        ));
    }

    #[test]
    fn fold_concatenates_assistant_text_into_the_reply() {
        let out = fold_session_events(
            "codex",
            vec![
                ev(EventKind::SessionStarted { id: "s".into() }),
                ev(EventKind::AssistantText { delta: "VERDICT: ".into() }),
                ev(EventKind::AssistantText { delta: "LGTM".into() }),
                ev(EventKind::TurnDone { stop_reason: "end".into() }),
            ],
        )
        .expect("a turn with text must yield output");
        assert_eq!(out.agent, "codex");
        assert_eq!(out.text, "VERDICT: LGTM");
    }

    #[test]
    fn fold_empty_turn_is_empty_error() {
        let out = fold_session_events(
            "claude",
            vec![ev(EventKind::TurnDone { stop_reason: "end".into() })],
        );
        assert!(matches!(out, Err(AdapterError::Empty)));
    }

    #[test]
    fn fold_error_with_no_text_is_flaked() {
        let out = fold_session_events(
            "agy",
            vec![ev(EventKind::Error {
                error_kind: "spawn".into(),
                detail: "pty closed".into(),
            })],
        );
        match out {
            Err(AdapterError::Flaked(d)) => assert!(d.contains("pty closed")),
            other => panic!("expected Flaked, got {other:?}"),
        }
    }

    #[test]
    fn fold_rate_limit_takes_precedence_over_text_and_flake() {
        // A vendor that prints a quota line (even alongside some text or an error) must surface as
        // RateLimited so the gate retries later — never an answer, never a plain flake.
        let out = fold_session_events(
            "codex",
            vec![
                ev(EventKind::AssistantText { delta: "working on it...".into() }),
                ev(EventKind::Error {
                    error_kind: "quota".into(),
                    detail: "You've hit your usage limit. try again at Jun 25th, 2026 5:33 AM"
                        .into(),
                }),
            ],
        );
        match out {
            Err(AdapterError::RateLimited(info)) => {
                assert_eq!(info.retry_at.as_deref(), Some("Jun 25th, 2026 5:33 AM"));
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[test]
    fn map_governed_fold_classifies_text_error_quota_and_empty() {
        use crate::governed_run::RunOutcome as GovOutcome;
        let gf = |text: &str, error: Option<(&str, &str)>| GovernedFold {
            outcome: GovOutcome::Completed,
            text: text.to_string(),
            usage: serde_json::json!({}),
            error: error.map(|(k, d)| (k.to_string(), d.to_string())),
        };
        // Text + no error → an answer.
        let out = map_governed_fold("claude", gf("DONE", None)).expect("text yields output");
        assert_eq!(out.agent, "claude");
        assert_eq!(out.text, "DONE");
        // Empty text + an error → Flaked (carries the error).
        match map_governed_fold("agy", gf("", Some(("spawn", "pty closed")))) {
            Err(AdapterError::Flaked(d)) => assert!(d.contains("pty closed")),
            other => panic!("expected Flaked, got {other:?}"),
        }
        // Empty text + no error → Empty.
        assert!(matches!(
            map_governed_fold("codex", gf("", None)),
            Err(AdapterError::Empty)
        ));
        // A quota line anywhere takes precedence over an otherwise-Ok partial answer.
        assert!(matches!(
            map_governed_fold("codex", gf("partial", Some(("quota", "rate limit exceeded")))),
            Err(AdapterError::RateLimited(_))
        ));
    }

    #[test]
    fn fold_finds_a_rate_limit_in_an_earlier_error_not_just_the_last() {
        // Two error events: the FIRST carries the quota signal, a later unrelated one follows.
        // Keeping only the last error would lose the quota and misclassify it — all errors are
        // scanned, so it still surfaces as RateLimited.
        let out = fold_session_events(
            "codex",
            vec![
                ev(EventKind::Error {
                    error_kind: "quota".into(),
                    detail: "rate limit exceeded".into(),
                }),
                ev(EventKind::Error {
                    error_kind: "transport".into(),
                    detail: "connection reset".into(),
                }),
            ],
        );
        assert!(
            matches!(out, Err(AdapterError::RateLimited(_))),
            "a rate-limit in an earlier error must still surface, got {out:?}"
        );
    }
}
