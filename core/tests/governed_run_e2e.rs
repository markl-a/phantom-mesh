//! Hermetic governed-loop tests: feed a synthetic CliEvent stream through `drive`
//! with a MemRecorder + a MockEscalator (no real CLI, no real phone).
use phantom_mesh::cli_session::CliKind;
use phantom_mesh::cli_session::event::{CliEvent, EventKind, Fidelity, Source};
use phantom_mesh::execution_contract::ApprovalDecision;
use phantom_mesh::governed_run::escalation::MockEscalator;
use phantom_mesh::governed_run::recorder::{MemRecorder, RunRecord};
use phantom_mesh::governed_run::{GovernPolicy, RunOutcome, drive, drive_fold};
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
fn claude_high_risk_tool_is_delegated_parent_observes_only() {
    // claude gates CHILD-SIDE via the PreToolUse hook (the sole awaiter). The parent
    // drive loop only OBSERVES: a high-risk ToolCall in claude's stream produces NO
    // parent await and NO parent governance record; the run completes. The real
    // approve/deny is exercised by governed_run::permission's hook unit tests and the
    // gated live test (govern_claude_pretooluse_live). This locks in the agy-#3 fix:
    // exactly one awaiter (the hook), never a second parent-side await.
    let events = stream(vec![
        ev(EventKind::SessionStarted { id: "s1".into() }),
        ev(EventKind::ToolCall {
            name: "Bash".into(),
            args: json!({"cmd":"ls"}),
        }),
        ev(EventKind::TurnDone { stop_reason: "end".into() }),
    ]);
    let mut rec = MemRecorder::default();
    let mut esc = MockEscalator::default();
    // Would force a deny IF the parent (wrongly) awaited — proving it does not.
    esc.force_decision = Some(ApprovalDecision::Deny);
    let outcome = drive(
        CliKind::Claude,
        events,
        &mut rec,
        &mut esc,
        &GovernPolicy::default(),
    );
    assert!(
        esc.sent.is_empty(),
        "parent must NOT await for claude (hook gates child-side), got {:?}",
        esc.sent
    );
    assert!(
        !rec.records.iter().any(|r| matches!(r, RunRecord::Governance { .. })),
        "no parent governance record for a delegated claude tool"
    );
    // The raw tool_use is still captured for the signed flight transcript.
    assert!(rec.records.iter().any(|r| matches!(
        r,
        RunRecord::Event(e) if matches!(&e.event, EventKind::ToolCall { .. })
    )));
    assert_eq!(outcome, RunOutcome::Completed);
}

#[test]
fn codex_high_risk_tool_alerts_and_stop_aborts_the_run() {
    let events = stream(vec![
        ev(EventKind::ToolCall {
            name: "Bash".into(),
            args: json!({}),
        }),
        ev(EventKind::TurnDone { stop_reason: "end".into() }),
    ]);
    let mut rec = MemRecorder::default();
    let mut esc = MockEscalator::default();
    esc.force_stop = true;
    let outcome = drive(
        CliKind::Codex,
        events,
        &mut rec,
        &mut esc,
        &GovernPolicy::default(),
    );
    // codex is observed post-action (alert), and STOP aborts the rest of the run.
    assert!(
        esc.sent.iter().any(|s| s.starts_with("alert:")),
        "codex should ALERT (post-action), got {:?}",
        esc.sent
    );
    assert_eq!(outcome, RunOutcome::Aborted);
}

#[test]
fn codex_high_risk_tool_not_stopped_completes() {
    let events = stream(vec![
        ev(EventKind::ToolCall {
            name: "Bash".into(),
            args: json!({}),
        }),
        ev(EventKind::TurnDone { stop_reason: "end".into() }),
    ]);
    let mut rec = MemRecorder::default();
    let mut esc = MockEscalator::default();
    let outcome = drive(
        CliKind::Codex,
        events,
        &mut rec,
        &mut esc,
        &GovernPolicy::default(),
    );
    assert!(esc.sent.iter().any(|s| s.starts_with("alert:")));
    assert_eq!(outcome, RunOutcome::Completed); // not stopped
}

#[test]
fn low_risk_read_tool_is_recorded_but_not_escalated() {
    let events = stream(vec![
        ev(EventKind::ToolCall {
            name: "Read".into(),
            args: json!({"path":"x"}),
        }),
        ev(EventKind::TurnDone { stop_reason: "end".into() }),
    ]);
    let mut rec = MemRecorder::default();
    let mut esc = MockEscalator::default();
    let outcome = drive(
        CliKind::Claude,
        events,
        &mut rec,
        &mut esc,
        &GovernPolicy::default(),
    );
    // Read is ReadOnly → no escalation, no Governance record, run completes.
    assert!(esc.sent.is_empty(), "low-risk should not escalate, got {:?}", esc.sent);
    assert!(!rec.records.iter().any(|r| matches!(r, RunRecord::Governance { .. })));
    assert_eq!(outcome, RunOutcome::Completed);
}

#[test]
fn drive_fold_folds_text_and_usage_while_governing() {
    // The worker path: codex makes a high-risk edit, narrates, and reports usage.
    let events = stream(vec![
        ev(EventKind::SessionStarted { id: "s".into() }),
        ev(EventKind::AssistantText { delta: "Made the ".into() }),
        ev(EventKind::ToolCall {
            name: "Bash".into(),
            args: json!({"cmd":"touch proof.txt"}),
        }),
        ev(EventKind::AssistantText { delta: "change.".into() }),
        ev(EventKind::Usage { input_tokens: 11, output_tokens: 3, cost_usd: 0.0 }),
        ev(EventKind::TurnDone { stop_reason: "end".into() }),
    ]);
    let mut rec = MemRecorder::default();
    let mut esc = MockEscalator::default(); // codex: observe, no STOP
    let fold = drive_fold(
        CliKind::Codex,
        events,
        &mut rec,
        &mut esc,
        &GovernPolicy::default(),
    );
    // The worker still gets the CLI's full answer + usage...
    assert_eq!(fold.text, "Made the change.");
    assert_eq!(fold.usage["input_tokens"], 11);
    assert_eq!(fold.usage["output_tokens"], 3);
    assert!(fold.error.is_none());
    assert_eq!(fold.outcome, RunOutcome::Completed);
    // ...AND the high-risk ToolCall was governed (post-action alert + recorded).
    assert!(
        esc.sent.iter().any(|s| s.starts_with("alert:")),
        "high-risk Bash should alert, got {:?}",
        esc.sent
    );
    assert!(rec.records.iter().any(|r| matches!(
        r,
        RunRecord::Governance { enforcement: "post_action_observed", .. }
    )));
    // The full raw stream (session/text/tool/text/usage/turn) is flight-recorded.
    let raw = rec
        .records
        .iter()
        .filter(|r| matches!(r, RunRecord::Event(_)))
        .count();
    assert!(raw >= 5, "raw events captured: {raw}");
}

#[test]
fn drive_fold_surfaces_cli_error() {
    let events = stream(vec![
        ev(EventKind::AssistantText { delta: "partial".into() }),
        ev(EventKind::Error {
            error_kind: "spawn".into(),
            detail: "codex not found".into(),
        }),
    ]);
    let mut rec = MemRecorder::default();
    let mut esc = MockEscalator::default();
    let fold = drive_fold(
        CliKind::Codex,
        events,
        &mut rec,
        &mut esc,
        &GovernPolicy::default(),
    );
    assert_eq!(
        fold.error,
        Some(("spawn".to_string(), "codex not found".to_string()))
    );
    assert_eq!(fold.text, "partial"); // text captured up to the error
}

/// Gated live smoke (run on a machine with codex installed):
///   cargo test --test governed_run_e2e -- --ignored --nocapture
/// Drives a real codex turn under governance against an ISOLATED home, asserting
/// the governed loop completes and mints a real run id. codex is
/// PostActionObserved, so it records + alerts (an OS desktop notification) but
/// does not block; no inbox reply is needed.
#[test]
#[ignore = "live: runs real codex + writes an OS notification"]
fn govern_codex_live_completes_and_records() {
    use phantom_mesh::governed_run::RunOutcome;
    use phantom_mesh::governed_run::run::{GovernConfig, run_govern_blocking};

    let tmp = std::env::temp_dir().join(format!("govern-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let mut cfg = GovernConfig::new(
        CliKind::Codex,
        "Reply with the single word DONE and nothing else.",
    );
    cfg.home = Some(tmp.clone());
    cfg.deadline = std::time::Duration::from_secs(45);

    let (outcome, run_id) = run_govern_blocking(cfg).expect("govern run should not error");
    eprintln!("govern codex -> outcome={outcome:?} run_id={run_id}");
    assert!(matches!(outcome, RunOutcome::Completed | RunOutcome::Aborted));
    assert!(!run_id.is_nil(), "a real run id is minted");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Gated live: the TRUE apex-④ claude pre-action gate end-to-end. Drives a real
/// claude under governance; claude is spawned with the PreToolUse hook
/// (`phantom pretooluse-gate`) that pauses BEFORE each high-risk tool. With NO
/// operator reply and a short deadline, the hook FAIL-SAFE DENIES the Bash, so
/// `echo LIVEGATE` never runs and claude narrates the block.
///
/// REQUIRES a real claude on PATH + a freshly-built phantom; point the hook at it:
///   PHANTOM_GOVERN_HOOK_CMD='"<abs>/phantom.exe" pretooluse-gate' \
///   PHANTOM_GOVERN_DEADLINE_SECS=8 PHANTOM_GOVERN_POLL_SECS=1 \
///   cargo test --test governed_run_e2e govern_claude_pretooluse_live -- --ignored --nocapture
#[test]
#[ignore = "live: runs real claude + the phantom PreToolUse hook gate"]
fn govern_claude_pretooluse_live_blocks_high_risk_tool() {
    use phantom_mesh::governed_run::run::{GovernConfig, run_govern_folded_blocking};

    if std::env::var("PHANTOM_GOVERN_HOOK_CMD").is_err() {
        eprintln!("skip: set PHANTOM_GOVERN_HOOK_CMD='\"<phantom-exe>\" pretooluse-gate' first");
        return;
    }
    let tmp = std::env::temp_dir().join(format!("govern-claude-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let mut cfg = GovernConfig::new(
        CliKind::Claude,
        "Use the Bash tool to run exactly: echo LIVEGATE. You must use the Bash tool.",
    );
    cfg.home = Some(tmp.clone());
    cfg.deadline = std::time::Duration::from_secs(90);

    let (fold, run_id) =
        run_govern_folded_blocking(cfg).expect("govern run should not error");
    eprintln!(
        "govern claude -> outcome={:?} run_id={run_id}\n--- claude text ---\n{}",
        fold.outcome, fold.text
    );

    // The signed flight transcript captured claude's attempted tool_use.
    let transcript = phantom_mesh::cli_config::phantom_dir_under(&tmp)
        .join("governed_runs")
        .join(format!("{run_id}.jsonl"));
    assert!(transcript.exists(), "flight transcript should exist at {transcript:?}");

    // The hook fail-safe DENIED the Bash, so claude reports it was blocked/denied and
    // `echo LIVEGATE` never produced its output through the tool.
    let txt = fold.text.to_lowercase();
    assert!(
        txt.contains("block") || txt.contains("den") || txt.contains("permission"),
        "claude should report the tool was hook-blocked, got: {}",
        fold.text
    );
    let _ = std::fs::remove_dir_all(&tmp);
}
