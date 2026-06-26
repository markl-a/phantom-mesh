use phantom_mesh::cli_session::CliKind;
use phantom_mesh::cli_session::event::{CliEvent, EventKind, Fidelity, Source};
use phantom_mesh::governed_run::escalation::MockEscalator;
use phantom_mesh::governed_run::recorder::{
    EventStoreRecorder, verify_transcript_with_identity,
};
use phantom_mesh::governed_run::{GovernPolicy, RunOutcome, drive_fold};
use phantom_mesh::tasks::events::EventStore;
use serde_json::json;
use std::sync::mpsc::{Receiver, channel};

fn stream(events: Vec<CliEvent>) -> Receiver<CliEvent> {
    let (tx, rx) = channel();
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
fn replay_verifies_round_trips_and_detects_tamper() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let raw = rusqlite::Connection::open_in_memory().unwrap();
    let store = EventStore::from_conn(std::sync::Arc::new(tokio::sync::Mutex::new(raw)));
    let task_id = uuid::Uuid::new_v4();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("governed_runs");
    let identity_path = tmp.path().join("identity.key");
    let mut rec = EventStoreRecorder::new_with_identity_path(
        store,
        task_id,
        rt.handle().clone(),
        dir.clone(),
        identity_path.clone(),
    )
    .unwrap();

    let events = vec![
        ev(EventKind::SessionStarted { id: "s1".into() }),
        ev(EventKind::AssistantText {
            delta: "hello ".into(),
        }),
        ev(EventKind::ToolCall {
            name: "Read".into(),
            args: json!({"path":"src/main.rs"}),
        }),
        ev(EventKind::AssistantText {
            delta: "world".into(),
        }),
        ev(EventKind::Usage {
            input_tokens: 5,
            output_tokens: 7,
            cost_usd: 0.0,
        }),
        ev(EventKind::TurnDone {
            stop_reason: "end".into(),
        }),
    ];

    let fold = drive_fold(
        CliKind::Codex,
        stream(events.clone()),
        &mut rec,
        &mut MockEscalator::default(),
        &GovernPolicy::default(),
    );
    assert_eq!(fold.outcome, RunOutcome::Completed);
    assert_eq!(fold.text, "hello world");

    let transcript = dir.join(format!("{task_id}.jsonl"));
    assert!(transcript.exists());

    let verified = verify_transcript_with_identity(&transcript, &identity_path).unwrap();
    assert_eq!(verified.len(), 6);
    assert_eq!(verified, events);
    assert!(matches!(
        verified[0].event,
        EventKind::SessionStarted { .. }
    ));
    assert!(verified.iter().any(|e| matches!(
        &e.event,
        EventKind::AssistantText { delta } if delta == "hello "
    )));
    assert!(verified.iter().any(|e| matches!(
        &e.event,
        EventKind::AssistantText { delta } if delta == "world"
    )));
    assert!(verified.iter().any(|e| matches!(
        &e.event,
        EventKind::ToolCall { name, args }
            if name == "Read" && args == &json!({"path":"src/main.rs"})
    )));

    let mut bytes = std::fs::read(&transcript).unwrap();
    let pos = bytes
        .windows(b"world".len())
        .position(|window| window == b"world")
        .unwrap();
    bytes[pos] = b'W';
    std::fs::write(&transcript, bytes).unwrap();
    assert!(
        verify_transcript_with_identity(&transcript, &identity_path).is_err(),
        "flipping one recorded byte must fail verification"
    );
}
