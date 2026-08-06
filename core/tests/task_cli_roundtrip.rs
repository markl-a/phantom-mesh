//! Real-path integration test for the `spectyn task` CLI (S0 lane F2).
//!
//! Spawns the actually-built binary (`env!("CARGO_BIN_EXE_spectyn")`) against a
//! temp `$HOME` so it exercises the same durable TaskStore/TaskQueue
//! (`<home>/.spectyn-mesh/spectyn.db`) a real user hits — no mocks, no in-proc
//! shortcuts. Asserts the submit → show → cancel round-trip:
//!
//!   * `submit` mints a UUID task_id on stdout (Pending),
//!   * `show --json` reflects that id with status `pending`,
//!   * `cancel` transitions it (Pending → Cancelled),
//!   * a second `cancel` hits the terminal-state guard and reports cleanly
//!     (exit 0, "already cancelled"), never panics,
//!   * a bad / missing id exits nonzero with a clear error (no panic).
//!
//! ## Isolation / platform gate
//!
//! `HOME` + `USERPROFILE` + `SPECTYN_HOME` all point at a unique temp dir so the
//! child never touches the developer's real `~/.spectyn-mesh`. Gated to `unix`:
//! the `task` store path is resolved via `resolve_home_dir()` (HOME →
//! USERPROFILE → dirs::home_dir()), which on Windows still lets a bare
//! `dirs::home_dir()` fallback win in some child contexts; the platform-agnostic
//! store/CLI logic is fully covered on the Linux CI runner. (Mirrors the gate on
//! `cli_exec_jsonl_schema_hermetic.rs`.)

#![cfg(unix)]

use std::process::Command;

fn spectyn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_spectyn")
}

fn unique_home() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "spectyn-task-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// Run `spectyn <args...>` under the temp `$HOME`, returning the captured output.
fn run(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    let home_s = home.to_string_lossy().to_string();
    Command::new(spectyn_bin())
        .args(args)
        .env("HOME", &home_s)
        .env("USERPROFILE", &home_s)
        .env("SPECTYN_HOME", &home_s)
        // Don't let dev-machine routing/config env perturb the surface.
        .env_remove("SPECTYN_LOCAL_FIRST")
        .env_remove("SPECTYN_RUNTIME_OVERRIDE")
        .output()
        .expect("spectyn must spawn")
}

#[test]
fn task_submit_show_cancel_round_trip() {
    let home = unique_home();
    std::fs::create_dir_all(home.join(".spectyn-mesh")).expect("seed temp home");

    // ── submit: mints a Pending task, prints the id on stdout ────────────────
    let submit = run(
        &home,
        &["task", "submit", "write a haiku", "--agent", "coder"],
    );
    let submit_stdout = String::from_utf8_lossy(&submit.stdout);
    let submit_stderr = String::from_utf8_lossy(&submit.stderr);
    assert!(
        submit.status.success(),
        "submit exited {:?}\nstdout:{}\nstderr:{}",
        submit.status,
        submit_stdout,
        submit_stderr
    );
    let task_id = submit_stdout.trim().to_string();
    assert!(
        uuid::Uuid::parse_str(&task_id).is_ok(),
        "submit stdout must be a bare task UUID, got: {:?}",
        submit_stdout
    );

    // ── show --json: the durable row reflects the minted id, status pending ──
    let show = run(&home, &["task", "show", &task_id, "--json"]);
    assert!(
        show.status.success(),
        "show exited {:?}\nstderr:{}",
        show.status,
        String::from_utf8_lossy(&show.stderr)
    );
    let show_json: serde_json::Value =
        serde_json::from_slice(&show.stdout).expect("show --json must emit one JSON object");
    assert_eq!(show_json["task_id"], serde_json::json!(task_id));
    assert_eq!(show_json["status"], serde_json::json!("pending"));
    assert_eq!(show_json["agent_name"], serde_json::json!("coder"));
    assert_eq!(show_json["prompt"], serde_json::json!("write a haiku"));

    // ── logs: no output yet → clean note on stderr, empty stdout, exit 0 ─────
    let logs = run(&home, &["task", "logs", &task_id]);
    assert!(logs.status.success(), "logs exited {:?}", logs.status);
    assert!(
        logs.stdout.is_empty(),
        "logs stdout should be empty for a task with no output yet, got: {:?}",
        String::from_utf8_lossy(&logs.stdout)
    );

    // ── cancel: Pending → Cancelled (exit 0) ─────────────────────────────────
    let cancel = run(&home, &["task", "cancel", &task_id]);
    assert!(
        cancel.status.success(),
        "cancel exited {:?}\nstderr:{}",
        cancel.status,
        String::from_utf8_lossy(&cancel.stderr)
    );

    // show --json again: status is now cancelled, finished_at recorded.
    let show2 = run(&home, &["task", "show", &task_id, "--json"]);
    assert!(show2.status.success());
    let show2_json: serde_json::Value =
        serde_json::from_slice(&show2.stdout).expect("show --json after cancel");
    assert_eq!(show2_json["status"], serde_json::json!("cancelled"));
    assert!(
        show2_json["finished_at"].is_number(),
        "cancelled task should have a finished_at, got: {}",
        show2_json["finished_at"]
    );

    // ── terminal-state guard: a second cancel reports cleanly, never panics ──
    let cancel2 = run(&home, &["task", "cancel", &task_id]);
    assert!(
        cancel2.status.success(),
        "re-cancelling a terminal task must exit 0 (clean report), got {:?}\nstderr:{}",
        cancel2.status,
        String::from_utf8_lossy(&cancel2.stderr)
    );
    let cancel2_stdout = String::from_utf8_lossy(&cancel2.stdout);
    assert!(
        cancel2_stdout.contains("already") && cancel2_stdout.contains("cancelled"),
        "re-cancel should report 'already cancelled', got: {:?}",
        cancel2_stdout
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn task_show_invalid_and_missing_id_fail_cleanly() {
    let home = unique_home();
    std::fs::create_dir_all(home.join(".spectyn-mesh")).expect("seed temp home");

    // Malformed id → nonzero exit, clear error, no panic.
    let bad = run(&home, &["task", "show", "not-a-uuid"]);
    assert!(
        !bad.status.success(),
        "show with a malformed id must exit nonzero"
    );
    assert!(
        String::from_utf8_lossy(&bad.stderr).contains("invalid task id"),
        "expected a clear 'invalid task id' error, got: {:?}",
        String::from_utf8_lossy(&bad.stderr)
    );

    // Well-formed but unknown id → nonzero exit, "not found".
    let missing_id = uuid::Uuid::new_v4().to_string();
    let unknown = run(&home, &["task", "show", &missing_id]);
    assert!(
        !unknown.status.success(),
        "show with an unknown id must exit nonzero"
    );
    assert!(
        String::from_utf8_lossy(&unknown.stderr).contains("not found"),
        "expected a 'not found' error, got: {:?}",
        String::from_utf8_lossy(&unknown.stderr)
    );

    // No id at all → nonzero exit, usage hint.
    let no_id = run(&home, &["task", "show"]);
    assert!(
        !no_id.status.success(),
        "show with no id must exit nonzero"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn task_replay_and_export_render_the_event_timeline() {
    let home = unique_home();
    std::fs::create_dir_all(home.join(".spectyn-mesh")).expect("seed temp home");

    // submit → mints a Pending task (records a `created` lifecycle event).
    let submit = run(
        &home,
        &["task", "submit", "summarize the report", "--agent", "coder"],
    );
    assert!(
        submit.status.success(),
        "submit exited {:?}\nstderr:{}",
        submit.status,
        String::from_utf8_lossy(&submit.stderr)
    );
    let task_id = String::from_utf8_lossy(&submit.stdout).trim().to_string();
    assert!(uuid::Uuid::parse_str(&task_id).is_ok());

    // cancel → transitions Pending → Cancelled (records a `cancelled` event).
    let cancel = run(&home, &["task", "cancel", &task_id]);
    assert!(
        cancel.status.success(),
        "cancel exited {:?}\nstderr:{}",
        cancel.status,
        String::from_utf8_lossy(&cancel.stderr)
    );

    // ── replay: human timeline shows both events in append order ─────────────
    let replay = run(&home, &["task", "replay", &task_id]);
    assert!(
        replay.status.success(),
        "replay exited {:?}\nstderr:{}",
        replay.status,
        String::from_utf8_lossy(&replay.stderr)
    );
    let replay_out = String::from_utf8_lossy(&replay.stdout);
    let created_at = replay_out.find("created");
    let cancelled_at = replay_out.find("cancelled");
    assert!(
        created_at.is_some() && cancelled_at.is_some(),
        "replay must show both `created` and `cancelled`, got:\n{}",
        replay_out
    );
    assert!(
        created_at < cancelled_at,
        "replay must show `created` before `cancelled`, got:\n{}",
        replay_out
    );

    // ── replay --json: raw event list, ordered, with the kinds ───────────────
    let replay_json = run(&home, &["task", "replay", &task_id, "--json"]);
    assert!(replay_json.status.success());
    let evs: serde_json::Value =
        serde_json::from_slice(&replay_json.stdout).expect("replay --json must emit a JSON array");
    let arr = evs.as_array().expect("replay --json is an array");
    assert_eq!(arr.len(), 2, "expected two lifecycle events, got: {}", evs);
    assert_eq!(arr[0]["kind"], serde_json::json!("created"));
    assert_eq!(arr[1]["kind"], serde_json::json!("cancelled"));
    assert_eq!(arr[0]["task_id"], serde_json::json!(task_id));
    assert!(
        arr[0]["seq"].as_i64().unwrap() < arr[1]["seq"].as_i64().unwrap(),
        "events must be ordered by seq"
    );

    // ── export: Markdown report carries the id + both event kinds ────────────
    let export = run(&home, &["task", "export", &task_id]);
    assert!(
        export.status.success(),
        "export exited {:?}\nstderr:{}",
        export.status,
        String::from_utf8_lossy(&export.stderr)
    );
    let md = String::from_utf8_lossy(&export.stdout);
    assert!(
        md.contains(&task_id),
        "export Markdown must contain the task id, got:\n{}",
        md
    );
    assert!(
        md.contains("## Timeline"),
        "export must have a Timeline section, got:\n{}",
        md
    );
    assert!(
        md.contains("created") && md.contains("cancelled"),
        "export Timeline must list both event kinds, got:\n{}",
        md
    );
    assert!(
        md.contains("summarize the report"),
        "export header must echo the prompt, got:\n{}",
        md
    );

    // unknown id → clean "not found", nonzero exit (both subcommands).
    let missing = uuid::Uuid::new_v4().to_string();
    let bad_replay = run(&home, &["task", "replay", &missing]);
    assert!(!bad_replay.status.success(), "replay of unknown id must exit nonzero");
    assert!(String::from_utf8_lossy(&bad_replay.stderr).contains("not found"));
    let bad_export = run(&home, &["task", "export", &missing]);
    assert!(!bad_export.status.success(), "export of unknown id must exit nonzero");

    let _ = std::fs::remove_dir_all(&home);
}

/// Regression (multi-AI review 2026-06-16): the task id must resolve whether a
/// flag comes BEFORE or AFTER it — `task replay --json <id>` previously failed
/// with a confusing "missing <id>" because only arg slot 3 was checked.
#[test]
fn task_id_resolves_with_flag_before_id() {
    let home = unique_home();
    std::fs::create_dir_all(home.join(".spectyn-mesh")).expect("seed temp home");

    let submit = run(&home, &["task", "submit", "regression check", "--agent", "coder"]);
    assert!(submit.status.success());
    let task_id = String::from_utf8_lossy(&submit.stdout).trim().to_string();
    assert!(uuid::Uuid::parse_str(&task_id).is_ok());

    // flag BEFORE id must still resolve the id (not "missing <id>").
    let replay = run(&home, &["task", "replay", "--json", &task_id]);
    assert!(
        replay.status.success(),
        "`task replay --json <id>` (flag first) must work; stderr:\n{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    let evs: serde_json::Value =
        serde_json::from_slice(&replay.stdout).expect("flag-first replay --json must emit JSON");
    assert!(evs.as_array().map(|a| !a.is_empty()).unwrap_or(false));

    // show with flag-first too.
    let show = run(&home, &["task", "show", "--json", &task_id]);
    assert!(show.status.success(), "`task show --json <id>` must resolve the id");

    let _ = std::fs::remove_dir_all(&home);
}

/// Lane G: the `task approvals/approve/deny` CLI arms — arg handling + the
/// no-pending path. (The record/approve/deny LOGIC is unit-tested in
/// tasks::approvals; here we cover the CLI dispatch the review flagged untested.)
#[test]
fn task_approval_cli_arg_handling() {
    let home = unique_home();
    std::fs::create_dir_all(home.join(".spectyn-mesh")).expect("seed temp home");

    let submit = run(&home, &["task", "submit", "needs approval", "--agent", "coder"]);
    assert!(submit.status.success());
    let id = String::from_utf8_lossy(&submit.stdout).trim().to_string();

    // No contracts raised yet → clean "no pending approvals", exit 0.
    let appr = run(&home, &["task", "approvals", &id]);
    assert!(appr.status.success(), "approvals of a task with none must exit 0");
    assert!(String::from_utf8_lossy(&appr.stderr).contains("no pending approvals"));
    // --json with none → empty array on stdout.
    let appr_json = run(&home, &["task", "approvals", &id, "--json"]);
    assert!(appr_json.status.success());
    let v: serde_json::Value = serde_json::from_slice(&appr_json.stdout).expect("approvals --json");
    assert_eq!(v.as_array().map(|a| a.len()), Some(0));

    // approve missing the contract-id → usage error, nonzero.
    let no_cid = run(&home, &["task", "approve", &id]);
    assert!(!no_cid.status.success(), "approve without contract-id must fail");

    // approve an unknown contract → "no contract awaiting approval", nonzero.
    let bad = run(&home, &["task", "approve", &id, "no-such-contract"]);
    assert!(!bad.status.success(), "approve of unknown contract must fail");
    assert!(String::from_utf8_lossy(&bad.stderr).contains("no contract"));

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn task_submit_json_emits_pending_record() {
    let home = unique_home();
    std::fs::create_dir_all(home.join(".spectyn-mesh")).expect("seed temp home");

    let submit = run(&home, &["task", "submit", "--json", "do the thing"]);
    assert!(
        submit.status.success(),
        "submit --json exited {:?}\nstderr:{}",
        submit.status,
        String::from_utf8_lossy(&submit.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&submit.stdout).expect("submit --json must emit one JSON object");
    assert!(
        uuid::Uuid::parse_str(json["task_id"].as_str().unwrap_or("")).is_ok(),
        "submit --json must carry a UUID task_id, got: {}",
        json["task_id"]
    );
    assert_eq!(json["status"], serde_json::json!("pending"));
    assert_eq!(json["agent_name"], serde_json::json!("master")); // default agent
    assert_eq!(json["prompt"], serde_json::json!("do the thing"));

    let _ = std::fs::remove_dir_all(&home);
}
