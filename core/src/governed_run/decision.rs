//! Pure governance decision logic — no I/O. Classifies an L0 event, decides the
//! enforcement mode per CLI, and maps a (risk, decision) to a runnable outcome.

use crate::cli_session::CliKind;
use crate::cli_session::event::{CliEvent, EventKind};
use crate::execution_contract::{ApprovalDecision, ContractState, RiskLevel, apply};
use crate::tasks::approvals::{GateOutcome, classify_tool, gate};

/// How an action is governed for a given CLI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enforcement {
    /// The parent loop awaits the operator BEFORE the tool runs. (Reserved for a CLI
    /// whose pre-action pause is driven from the parent; not used by claude now that
    /// claude gates child-side — see `PreActionDelegated`.)
    PreActionBlocking,
    /// claude's pre-action gate is handled CHILD-SIDE by the PreToolUse hook (the
    /// sole awaiter + authoritative recorder, bound to the run's task_id). The parent
    /// only OBSERVES the stream — no second await (the agy-#3 fix).
    PreActionDelegated,
    /// The tool already ran; we record + alert + can abort the rest (codex/opencode/agy).
    PostActionObserved,
}

/// claude pauses pre-action via a child-side PreToolUse hook (delegated); the others
/// are observed post-action.
pub fn enforcement_for(cli: CliKind) -> Enforcement {
    match cli {
        CliKind::Claude => Enforcement::PreActionDelegated,
        CliKind::Codex | CliKind::Opencode | CliKind::Agy | CliKind::External(_) => {
            Enforcement::PostActionObserved
        }
    }
}

/// If the event is a ToolCall, return its (name, args, risk); else None.
pub fn classify_event(event: &CliEvent) -> Option<(String, serde_json::Value, RiskLevel)> {
    if let EventKind::ToolCall { name, args } = &event.event {
        Some((name.clone(), args.clone(), classify_tool(name, args)))
    } else {
        None
    }
}

/// Given the operator's decision, the runnable gate outcome (reuses the governor's apply+gate).
pub fn outcome_for(decision: ApprovalDecision) -> GateOutcome {
    gate(apply(ContractState::Pending, decision))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_session::event::{CliEvent, EventKind, Fidelity, Source};
    use serde_json::json;

    fn tool_call(name: &str) -> CliEvent {
        CliEvent::new(
            EventKind::ToolCall { name: name.into(), args: json!({}) },
            Fidelity::StructuredVerified,
            Source::LiveStream,
        )
    }

    #[test]
    fn high_risk_tool_is_classified_and_claude_blocks() {
        // "shell"/"bash" should classify high (needs approval); claude blocks pre-action.
        let ev = tool_call("Bash");
        let (_n, _a, risk) = classify_event(&ev).expect("a ToolCall");
        assert!(risk.requires_approval(), "Bash should be high-risk, got {risk:?}");
        assert_eq!(enforcement_for(CliKind::Claude), Enforcement::PreActionDelegated);
        assert_eq!(enforcement_for(CliKind::Codex), Enforcement::PostActionObserved);
    }

    #[test]
    fn non_toolcall_event_is_not_classified() {
        let ev = CliEvent::new(
            EventKind::AssistantText { delta: "hi".into() },
            Fidelity::StructuredVerified,
            Source::LiveStream,
        );
        assert!(classify_event(&ev).is_none());
    }

    #[test]
    fn approve_yields_allow_deny_yields_deny() {
        assert_eq!(outcome_for(ApprovalDecision::ApproveOnce), GateOutcome::Allow);
        assert_eq!(outcome_for(ApprovalDecision::Deny), GateOutcome::Deny);
        assert_eq!(outcome_for(ApprovalDecision::Cancel), GateOutcome::Deny);
    }

    #[test]
    fn outcome_for_covers_remaining_decision_variants() {
        // The mapping above omits three variants; lock them here so the full
        // ApprovalDecision -> GateOutcome contract is pinned.
        //
        // ApproveTask (cache the approval for the task) and DryRun (allowed
        // side-effect-free downgrade) both Allow the pending action to run.
        assert_eq!(outcome_for(ApprovalDecision::ApproveTask), GateOutcome::Allow);
        assert_eq!(outcome_for(ApprovalDecision::DryRun), GateOutcome::Allow);

        // ④-SAFETY: a phone Redirect does NOT approve the pending action — the
        // operator is steering with a new instruction, so for THIS contract it
        // must behave exactly like a deny (the pending tool is not run). A
        // regression mapping Redirect -> Allow would run the dangerous pending
        // tool despite the operator saying "no, do this instead".
        assert_eq!(
            outcome_for(ApprovalDecision::Redirect("do this instead".into())),
            GateOutcome::Deny,
            "a redirect must block the pending action, not run it"
        );
    }
}
