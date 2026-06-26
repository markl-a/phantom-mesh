//! P4 behavior gate: a live API key / token captured by the governed-run
//! flight-recorder OR the tracing JSONL writer must NEVER reach disk in CLEAR.
//!
//! These tests drive the REAL writers (no mocks of the redactor) against a temp
//! dir, feeding a fixture event whose args contain `sk-LIVEKEY123…`, then read
//! the on-disk `.jsonl` back and assert the secret bytes are absent. The
//! flight-recorder case ALSO asserts the redacted transcript still verifies (the
//! HMAC chain signs the redacted bytes), proving redaction did not break replay.

use phantom_mesh::cli_session::event::{CliEvent, EventKind, Fidelity, Source};
use phantom_mesh::governed_run::recorder::{
    EventStoreRecorder, RunRecord, RunRecorder, verify_transcript_with_identity,
};
use phantom_mesh::tasks::events::EventStore;
use phantom_mesh::tracing::{Event, Tracer};
use serde_json::json;

/// The literal secret used across both writers. If ANY byte run of this appears
/// on disk, redaction failed.
const SECRET: &str = "sk-LIVEKEY123abcDEF456ghiJKL789mnopQRS";

#[test]
fn flight_recorder_jsonl_contains_no_secret() {
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

    // A realistic governed-run event: the CLI tried to run a command whose argv
    // embeds the live key (exactly the P4 leak path).
    rec.record(RunRecord::Event(CliEvent::new(
        EventKind::ToolCall {
            name: "Bash".into(),
            args: json!(["env", format!("OPENAI_API_KEY={SECRET}"), "&&", "deploy"]),
        },
        Fidelity::StructuredVerified,
        Source::LiveStream,
    )));
    // Also exercise a tool RESULT carrying the secret in free text.
    rec.record(RunRecord::Event(CliEvent::new(
        EventKind::ToolResult {
            name: "Bash".into(),
            output: format!("printed token: {SECRET}"),
            ok: true,
        },
        Fidelity::StructuredVerified,
        Source::LiveStream,
    )));

    let transcript = dir.join(format!("{task_id}.jsonl"));
    let body = std::fs::read_to_string(&transcript).unwrap();
    assert!(
        !body.contains(SECRET),
        "flight-recorder transcript leaked the secret IN CLEAR:\n{body}"
    );
    assert!(
        body.contains("[REDACTED]"),
        "expected a redaction marker in the transcript:\n{body}"
    );

    // Redaction happened BEFORE signing, so the (redacted) transcript must still
    // verify end-to-end — replay/audit is preserved.
    let verified = verify_transcript_with_identity(&transcript, &identity_path)
        .expect("redacted transcript must still verify");
    assert_eq!(verified.len(), 2, "both events present after redaction");
}

#[test]
fn tracing_jsonl_contains_no_secret() {
    let tmp = tempfile::tempdir().unwrap();
    let mut tracer = Tracer::new_in_dir("redact-trace-task", tmp.path().to_path_buf()).unwrap();

    // A tool call whose args embed the live key.
    tracer
        .record(Event::ToolCall {
            name: "shell".into(),
            args: json!({ "cmd": format!("curl -H 'Authorization: Bearer {SECRET}' https://api") }),
        })
        .unwrap();
    // And a plan line that should pass through UNCHANGED (conservative check).
    tracer
        .record(Event::Plan {
            plan: "Refactor the recorder, add a redaction unit test, then verify.".into(),
        })
        .unwrap();
    tracer.flush().unwrap();

    let path = tmp.path().join("redact-trace-task.jsonl");
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(
        !body.contains(SECRET),
        "trace jsonl leaked the secret IN CLEAR:\n{body}"
    );
    assert!(
        body.contains("[REDACTED]"),
        "expected a redaction marker in the trace:\n{body}"
    );
    // Conservative guarantee: ordinary plan prose survives intact.
    assert!(
        body.contains("Refactor the recorder, add a redaction unit test, then verify."),
        "normal plan text must be preserved verbatim:\n{body}"
    );
    // Each line is still valid JSON after redaction.
    for line in body.lines().filter(|l| !l.trim().is_empty()) {
        let _: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("non-JSON trace line {line:?}: {e}"));
    }
}
