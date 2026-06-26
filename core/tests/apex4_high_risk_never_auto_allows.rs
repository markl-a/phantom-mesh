//! Locks the SYS-C guardrail at the LIVE drive-loop level: high-risk actions can
//! never be configured to auto-allow. This complements the policy-predicate lib
//! test `high_risk_can_never_be_configured_to_auto_allow`.

use phantom_mesh::cli_session::CliKind;
use phantom_mesh::cli_session::event::{CliEvent, EventKind, Fidelity, Source};
use phantom_mesh::governed_run::escalation::MockEscalator;
use phantom_mesh::governed_run::recorder::{MemRecorder, RunRecord};
use phantom_mesh::governed_run::{GovernPolicy, RunOutcome, drive};
use serde_json::json;

fn stream(events: Vec<CliEvent>) -> std::sync::mpsc::Receiver<CliEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    for e in events {
        tx.send(e).unwrap();
    }
    drop(tx);
    rx
}

fn ev(k: EventKind) -> CliEvent {
    CliEvent::new(k, Fidelity::StructuredVerified, Source::LiveStream)
}

#[test]
fn high_risk_still_governed_under_auto_continue_opt_in() {
    let policy = GovernPolicy {
        auto_continue_low_risk: true,
        ..Default::default()
    };
    let events = stream(vec![
        ev(EventKind::ToolCall {
            name: "Bash".into(),
            args: json!({"command":"deploy prod"}),
        }),
        ev(EventKind::TurnDone {
            stop_reason: "end".into(),
        }),
    ]);
    let mut rec = MemRecorder::default();
    let mut esc = MockEscalator::default();

    let outcome = drive(CliKind::Codex, events, &mut rec, &mut esc, &policy);

    assert_eq!(outcome, RunOutcome::Completed);
    assert!(
        esc.sent.iter().any(|s| s.starts_with("alert:")),
        "high-risk MUST still escalate even with auto_continue_low_risk=true"
    );
    assert!(
        rec.records.iter().any(|r| matches!(
            r,
            RunRecord::Governance {
                enforcement: "post_action_observed",
                ..
            }
        )),
        "the high-risk action is STILL recorded as a governance moment under the opt-in"
    );
}

#[test]
fn low_risk_under_same_opt_in_is_not_escalated() {
    let policy = GovernPolicy {
        auto_continue_low_risk: true,
        ..Default::default()
    };
    let events = stream(vec![
        ev(EventKind::ToolCall {
            name: "Read".into(),
            args: json!({"path":"src/main.rs"}),
        }),
        ev(EventKind::TurnDone {
            stop_reason: "end".into(),
        }),
    ]);
    let mut rec = MemRecorder::default();
    let mut esc = MockEscalator::default();

    let outcome = drive(CliKind::Codex, events, &mut rec, &mut esc, &policy);

    assert_eq!(outcome, RunOutcome::Completed);
    assert!(
        esc.sent.is_empty(),
        "a low-risk tool must not escalate, got {:?}",
        esc.sent
    );
    assert!(
        !rec.records
            .iter()
            .any(|r| matches!(r, RunRecord::Governance { .. })),
        "no governance moment for an auto-allowed low-risk tool"
    );
}
