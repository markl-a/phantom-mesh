//! Public-drive apex-4 escalation STOP tests: a stopped post-action alert is
//! auditable and halts the run before later stream work is consumed.
use spectyn_mesh::cli_session::CliKind;
use spectyn_mesh::cli_session::event::{CliEvent, EventKind, Fidelity, Source};
use spectyn_mesh::governed_run::escalation::MockEscalator;
use spectyn_mesh::governed_run::recorder::{MemRecorder, RunRecord};
use spectyn_mesh::governed_run::{GovernPolicy, RunOutcome, drive};
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
fn operator_stop_halts_run_and_is_recorded() {
    let mut esc = MockEscalator::default();
    esc.force_stop = true;
    let events = stream(vec![
        ev(EventKind::ToolCall {
            name: "Bash".into(),
            args: json!({"command":"deploy prod"}),
        }),
        ev(EventKind::ToolCall {
            name: "Bash".into(),
            args: json!({"command":"rm -rf /important"}),
        }),
        ev(EventKind::TurnDone {
            stop_reason: "end".into(),
        }),
    ]);
    let mut rec = MemRecorder::default();

    let outcome = drive(
        CliKind::Codex,
        events,
        &mut rec,
        &mut esc,
        &GovernPolicy::default(),
    );

    assert_eq!(outcome, RunOutcome::Aborted, "an operator STOP halts the run");
    assert!(
        rec.records.iter().any(|r| matches!(
            r,
            RunRecord::Governance {
                enforcement: "post_action_observed",
                ..
            }
        )),
        "the stopped high-risk action is recorded as a governance moment"
    );
    assert_eq!(
        esc.sent.iter().filter(|s| s.starts_with("alert:")).count(),
        1,
        "stop halts after the first high-risk action; the second is never escalated, got {:?}",
        esc.sent
    );
}
