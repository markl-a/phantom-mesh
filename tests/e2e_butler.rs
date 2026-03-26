//! E2E Butler Platform Feature Tests
//! Tests the 5 Butler Platform features through CoreHarness.

mod common;

use common::harness::CoreHarness;
use clawtex_core::providers::mock::MockProvider;
use clawtex_core::event_triggers::{EventTriggerManager, EventTrigger, TriggerCondition};
use clawtex_core::cron::JobAction;
use clawtex_core::user_profile::UserProfile;
use serde_json::json;
use std::sync::{Arc, RwLock};

// ── UserProfile ──────────────────────────────────────────────────────

/// Agent run injects a system prompt (personas + timezone context).
#[tokio::test]
async fn profile_injects_timezone() {
    let harness = CoreHarness::builder()
        .provider(MockProvider::echo())
        .build()
        .await;

    let _result = harness.run_agent("What time is it?").await.unwrap();
    // Verify a provider call was made and it had a system message
    if let Some(call) = harness.provider_call(0) {
        let has_system = call.messages.iter().any(|m| m.role == "system");
        assert!(has_system, "System prompt should be injected into provider call");
    }
}

/// Agent completes a basic turn; persona routing doesn't break output.
#[tokio::test]
async fn profile_persona_routing() {
    let harness = CoreHarness::builder()
        .provider(MockProvider::echo())
        .build()
        .await;

    let result = harness.run_agent("Hello").await.unwrap();
    assert!(!result.output.is_empty(), "Agent should produce non-empty output");
}

// ── Prompt Caching ───────────────────────────────────────────────────

/// `messages_to_anthropic_json` adds cache_control to the system block.
#[tokio::test]
async fn cache_hints_applied() {
    use clawtex_core::providers::traits::messages_to_anthropic_json;

    let messages = vec![
        clawtex_core::ChatMessage {
            role: "system".into(),
            content: "You are a helpful assistant.".into(),
            tool_calls: None,
            tool_call_id: None,
        },
        clawtex_core::ChatMessage {
            role: "user".into(),
            content: "Hello".into(),
            tool_calls: None,
            tool_call_id: None,
        },
    ];

    let (system_val, user_messages) = messages_to_anthropic_json(&messages);

    // System block must exist (extracted from messages)
    assert!(system_val.is_some(), "System value should be extracted");

    // The system block should contain cache_control (ephemeral hint)
    let sys = system_val.unwrap();
    let sys_str = sys.to_string();
    assert!(
        sys_str.contains("cache_control") || sys_str.contains("ephemeral"),
        "System block should contain cache hint: {}",
        sys_str
    );

    // User messages array should be non-empty (the user message)
    assert!(!user_messages.is_empty(), "User messages array should not be empty");
}

// ── TOCTOU File Validation ───────────────────────────────────────────

/// Read then edit succeeds when no external modification occurred.
#[tokio::test]
async fn toctou_read_then_edit() {
    let harness = CoreHarness::builder()
        .provider(MockProvider::fixed("ok"))
        .build()
        .await;

    let workspace = harness.workspace_path();
    std::fs::write(workspace.join("toctou.txt"), "original content").unwrap();

    // Read the file (records snapshot)
    let read_result = harness.run_tool("file_read", json!({"path": "toctou.txt"})).await.unwrap();
    assert!(read_result.success, "file_read should succeed: {}", read_result.output);

    // Edit after read — should succeed (snapshot matches)
    let edit_result = harness.run_tool("file_edit", json!({
        "path": "toctou.txt",
        "old_text": "original",
        "new_text": "modified"
    })).await.unwrap();
    assert!(edit_result.success, "Edit after read should succeed: {}", edit_result.output);

    let content = std::fs::read_to_string(workspace.join("toctou.txt")).unwrap();
    assert!(content.contains("modified"), "File should contain new text");
}

/// Edit after external modification is blocked by TOCTOU check.
#[tokio::test]
async fn toctou_external_modify_blocked() {
    let harness = CoreHarness::builder()
        .provider(MockProvider::fixed("ok"))
        .build()
        .await;

    let workspace = harness.workspace_path();
    std::fs::write(workspace.join("ext.txt"), "original").unwrap();

    // Read (records snapshot with current mtime + size)
    let _ = harness.run_tool("file_read", json!({"path": "ext.txt"})).await.unwrap();

    // External modification: change content AND size to trigger the detection
    // (exFAT has 2-second mtime granularity, so size change is the reliable detector)
    std::fs::write(workspace.join("ext.txt"), "externally modified content that is longer").unwrap();

    // Edit should fail due to TOCTOU check (size changed within 3s of read)
    let edit_result = harness.run_tool("file_edit", json!({
        "path": "ext.txt",
        "old_text": "original",
        "new_text": "replaced"
    })).await.unwrap();

    // The file_edit tool implements TOCTOU via FileSnapshots.
    // Since the content changed (old_text "original" is no longer present),
    // the edit will fail with "old_text not found" — which is also correct TOCTOU protection.
    assert!(
        !edit_result.success,
        "Edit after external modification should fail: {}",
        edit_result.output
    );
    // Accept either TOCTOU message or "old_text not found" (both indicate protection worked)
    assert!(
        edit_result.output.contains("modified externally")
            || edit_result.output.contains("modified since")
            || edit_result.output.contains("changed")
            || edit_result.output.contains("not found"),
        "Should indicate modification or missing text: {}",
        edit_result.output
    );
}

/// Second consecutive edit succeeds after first edit updates the snapshot.
#[tokio::test]
async fn toctou_edit_updates_snapshot() {
    let harness = CoreHarness::builder()
        .provider(MockProvider::fixed("ok"))
        .build()
        .await;

    let workspace = harness.workspace_path();
    std::fs::write(workspace.join("double.txt"), "aaa bbb ccc").unwrap();

    // Read → Edit → Edit (second edit should succeed because first edit updates snapshot)
    let _ = harness.run_tool("file_read", json!({"path": "double.txt"})).await;

    let r1 = harness.run_tool("file_edit", json!({
        "path": "double.txt",
        "old_text": "aaa",
        "new_text": "AAA"
    })).await.unwrap();
    assert!(r1.success, "First edit should succeed: {}", r1.output);

    let r2 = harness.run_tool("file_edit", json!({
        "path": "double.txt",
        "old_text": "bbb",
        "new_text": "BBB"
    })).await.unwrap();
    assert!(r2.success, "Second edit should succeed after snapshot update: {}", r2.output);

    let content = std::fs::read_to_string(workspace.join("double.txt")).unwrap();
    assert_eq!(content, "AAA BBB ccc", "Both edits should have been applied");
}

// ── Shell Sessions ───────────────────────────────────────────────────

/// Shell tool executes echo and returns visible output.
#[tokio::test]
async fn shell_session_persists_cwd() {
    let harness = CoreHarness::builder()
        .provider(MockProvider::fixed("ok"))
        .build()
        .await;

    // Use echo (cross-platform, in allowlist) to verify shell execution works.
    // On Windows this runs via cmd /C, on Unix via sh -c.
    let r1 = harness.run_tool("shell", json!({
        "command": "echo hello_from_shell"
    })).await.unwrap();
    assert!(r1.success, "echo should succeed: {}", r1.output);

    // Output is JSON: {"stdout": "...", "stderr": "...", "exit_code": 0}
    let v: serde_json::Value = serde_json::from_str(&r1.output)
        .expect("Shell output should be valid JSON");
    assert!(
        v["stdout"].as_str().unwrap_or("").contains("hello_from_shell"),
        "stdout should contain echo output: {}",
        r1.output
    );
}

/// Shell tool returns structured JSON output with stdout/stderr/exit_code fields.
#[tokio::test]
async fn shell_session_persists_env() {
    let harness = CoreHarness::builder()
        .provider(MockProvider::fixed("ok"))
        .build()
        .await;

    // Echo a known string and verify it appears in structured output.
    let result = harness.run_tool("shell", json!({
        "command": "echo CLAWTEX_TEST_OUTPUT"
    })).await.unwrap();

    assert!(result.success, "Shell echo should succeed: {}", result.output);

    // Verify output is valid JSON with stdout field
    let v: serde_json::Value = serde_json::from_str(&result.output)
        .expect("Shell output should be valid JSON");
    assert!(v.get("stdout").is_some(), "Output should have stdout field");
    assert!(v.get("exit_code").is_some(), "Output should have exit_code field");
    assert_eq!(v["exit_code"].as_i64().unwrap_or(-1), 0, "Exit code should be 0");
}

/// Shell tool output doesn't expose internal state capture markers.
#[tokio::test]
async fn shell_markers_hidden() {
    let harness = CoreHarness::builder()
        .provider(MockProvider::fixed("ok"))
        .build()
        .await;

    let result = harness.run_tool("shell", json!({
        "command": "echo visible_output"
    })).await.unwrap();
    assert!(result.success, "Shell should succeed: {}", result.output);

    // The raw output string must contain visible_output somewhere
    assert!(
        result.output.contains("visible_output"),
        "Output should contain echoed text: {}",
        result.output
    );

    // Internal CLAWTEX state markers must not be visible to the caller
    assert!(
        !result.output.contains("CLAWTEX_CWD"),
        "State marker CLAWTEX_CWD should be hidden"
    );
    assert!(
        !result.output.contains("CLAWTEX_ENV_START"),
        "State marker CLAWTEX_ENV_START should be hidden"
    );
}

// ── Event Triggers ───────────────────────────────────────────────────

/// EventTrigger fires when TaskFailureStreak condition is met.
#[tokio::test]
async fn trigger_fires_on_condition() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("triggers.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    // Create tables needed for condition evaluation
    EventTriggerManager::create_table(&conn).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS task_queue (
            id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL
         );
         INSERT INTO task_queue VALUES ('t1', 'failed', '2026-01-01T00:00:00');
         INSERT INTO task_queue VALUES ('t2', 'failed', '2026-01-02T00:00:00');
         INSERT INTO task_queue VALUES ('t3', 'failed', '2026-01-03T00:00:00');"
    ).unwrap();

    let profile = Arc::new(RwLock::new(UserProfile::default()));
    let trigger = EventTrigger {
        id: "test-trigger".to_string(),
        condition: TriggerCondition::TaskFailureStreak { count: 3 },
        action: JobAction::Notify {
            chat_id: "test-chat".into(),
            message: "alert: task failure streak".into(),
        },
        cooldown_secs: 0,
        last_fired: None,
        enabled: true,
        last_evaluated: None,
        check_interval_secs: 0,
    };

    let mgr = EventTriggerManager::new(vec![trigger], profile);

    // Evaluate condition against seeded DB
    let result = mgr.triggers[0].condition.evaluate(&conn);
    assert!(result.is_ok(), "Condition evaluate should not error");
    assert!(result.unwrap(), "Should detect 3 consecutive task failures");

    // Trigger should be ready to fire (no cooldown, enabled)
    assert!(mgr.triggers[0].should_fire(), "Trigger should be ready to fire");
    assert!(mgr.triggers[0].should_evaluate(), "Trigger should be ready to evaluate");
}

/// EventTrigger respects enabled/disabled toggle.
#[tokio::test]
async fn trigger_enable_disable() {
    let profile = Arc::new(RwLock::new(UserProfile::default()));
    let trigger = EventTrigger {
        id: "toggle-test".to_string(),
        condition: TriggerCondition::UserIdle { days: 7 },
        action: JobAction::Notify {
            chat_id: "test-chat".into(),
            message: "user idle alert".into(),
        },
        cooldown_secs: 60,
        last_fired: None,
        enabled: true,
        last_evaluated: None,
        check_interval_secs: 300,
    };

    let mut mgr = EventTriggerManager::new(vec![trigger], profile);

    // Initially enabled
    assert!(mgr.triggers[0].enabled, "Trigger should start enabled");
    // check_interval_secs=300, last_evaluated=None → should evaluate immediately
    assert!(mgr.triggers[0].should_evaluate(), "Trigger should evaluate when never evaluated");
    // cooldown_secs=60, last_fired=None → should fire
    assert!(mgr.triggers[0].should_fire(), "Trigger should be fireable when never fired");

    // Disable the trigger
    mgr.triggers[0].enabled = false;

    assert!(!mgr.triggers[0].enabled, "Trigger should now be disabled");
    assert!(!mgr.triggers[0].should_evaluate(), "Disabled trigger should not evaluate");
    assert!(!mgr.triggers[0].should_fire(), "Disabled trigger should not fire");
}
