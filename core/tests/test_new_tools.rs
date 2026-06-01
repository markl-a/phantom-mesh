use phantom_mesh::project_context;
use phantom_mesh::providers::traits::ChatMessage;
use phantom_mesh::session::ConversationStore;
/// Integration tests for improvements added by parallel agents A3, A4, A5, A7, A8, A10.
///
/// Tests that exercise APIs not yet in main are marked `#[ignore]` with a comment
/// explaining which agent adds the feature. Remove `#[ignore]` once the relevant
/// agent's branch is merged and all assertions compile and pass.
use phantom_mesh::tools::{file, shell};
use serde_json::json;
use tempfile::tempdir;

// ── Helpers ───────────────────────────────────────────────────────────────

fn user_msg(content: &str) -> ChatMessage {
    ChatMessage {
        role: "user".into(),
        content: content.into(),
        tool_calls: None,
    }
}

fn asst_msg(content: &str) -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: content.into(),
        tool_calls: None,
    }
}

mod common;
use common::workspace_tempdir;

// ═══════════════════════════════════════════════════════════════════════════
// A3 — file_edit: replace_all, better errors, line range read, show_line_numbers
// ═══════════════════════════════════════════════════════════════════════════

/// A3: file_edit with replace_all=true replaces every occurrence.
#[tokio::test]
async fn test_file_edit_replace_all() {
    let dir = workspace_tempdir();
    let path = dir.path().join("replace_all.txt");
    let path_str = path.to_str().unwrap();

    std::fs::write(&path, "FOO one FOO two FOO three").unwrap();

    let result = file::edit(&json!({
        "path": path_str,
        "old_string": "FOO",
        "new_string": "BAR",
        "replace_all": true
    }))
    .await;

    // The implementation returns "Edited … (3 occurrences replaced).\n…" on success.
    assert!(
        result.contains("Edited") || result.contains("replaced"),
        "expected success message, got: {result}"
    );

    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        content.matches("FOO").count(),
        0,
        "expected all FOO occurrences replaced, content: {content}"
    );
    assert_eq!(
        content.matches("BAR").count(),
        3,
        "expected 3 BAR occurrences, content: {content}"
    );
}

/// A3: editing a file where old_string is not present returns an error containing "not found".
#[tokio::test]
async fn test_file_edit_not_found_error() {
    let dir = workspace_tempdir();
    let path = dir.path().join("no_match.txt");
    let path_str = path.to_str().unwrap();

    std::fs::write(&path, "hello world").unwrap();

    let result = file::edit(&json!({
        "path": path_str,
        "old_string": "DOES_NOT_EXIST",
        "new_string": "replacement"
    }))
    .await;

    assert!(
        result.contains("not found"),
        "expected 'not found' in error message, got: {result}"
    );
}

/// A3: editing a file where old_string appears more than once returns an error
/// that mentions the count.
#[tokio::test]
async fn test_file_edit_ambiguous_error() {
    let dir = workspace_tempdir();
    let path = dir.path().join("ambiguous.txt");
    let path_str = path.to_str().unwrap();

    std::fs::write(&path, "FOO and FOO and FOO").unwrap();

    let result = file::edit(&json!({
        "path": path_str,
        "old_string": "FOO",
        "new_string": "BAR"
    }))
    .await;

    // "Error: old_string found 3 times — must match exactly once."
    assert!(
        result.contains("3") || result.contains("times"),
        "expected ambiguity error mentioning the count, got: {result}"
    );
}

/// A3: reading a file with start_line / end_line returns only the requested range.
#[tokio::test]
async fn test_file_read_line_range() {
    let dir = workspace_tempdir();
    let path = dir.path().join("twenty_lines.txt");
    let path_str = path.to_str().unwrap();

    // 20-line file: line N contains only "N"
    let content: String = (1u32..=20).map(|n| format!("{}\n", n)).collect();
    std::fs::write(&path, &content).unwrap();

    let result = file::read(&json!({
        "path": path_str,
        "start_line": 5,
        "end_line": 10
    }))
    .await;

    // Output is prefixed with "Lines 5-10 of …:\n"
    assert!(
        result.contains("Lines 5-10"),
        "expected line-range header 'Lines 5-10', got: {result}"
    );
    // Lines 5-10 must be present
    for n in 5u32..=10 {
        assert!(
            result.contains(&n.to_string()),
            "expected line {n} in output, got: {result}"
        );
    }
    // Lines outside the range should NOT appear as standalone tokens
    // (line 1, 2, 3, 4 don't appear in the range prefix "Lines 5-10" context,
    // but we only check that lines 11-20 are not present since "1" would
    // appear in "Lines 5-10 of ...").
    for n in 11u32..=20 {
        // A line value like "15" should not appear in output.
        assert!(
            !result.contains(&format!("\n{}\n", n)),
            "line {n} should not appear in line-range output, got: {result}"
        );
    }
}

/// A3: reading a file with show_line_numbers=true prefixes each line with its number.
#[tokio::test]
async fn test_file_read_show_line_numbers() {
    let dir = workspace_tempdir();
    let path = dir.path().join("numbered.txt");
    let path_str = path.to_str().unwrap();

    std::fs::write(&path, "alpha\nbeta\ngamma\n").unwrap();

    let result = file::read(&json!({
        "path": path_str,
        "show_line_numbers": true
    }))
    .await;

    // Implementation formats as "    1: alpha", "    2: beta", etc.
    assert!(
        result.contains("1:") && result.contains("alpha"),
        "expected '1:' and 'alpha' in output, got: {result}"
    );
    assert!(
        result.contains("2:") && result.contains("beta"),
        "expected '2:' and 'beta' in output, got: {result}"
    );
    assert!(
        result.contains("3:") && result.contains("gamma"),
        "expected '3:' and 'gamma' in output, got: {result}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// A5 — shell: PHANTOM_AUTO_APPROVE gate and || operator handling
// ═══════════════════════════════════════════════════════════════════════════

/// A5: when PHANTOM_AUTO_APPROVE is not set, shell::run should return a response
/// containing "APPROVAL_REQUIRED" instead of executing the command.
///
/// Depends on A5 adding the approval gate to `shell::run`.
#[tokio::test]
async fn test_shell_approval_required() {
    // Ensure the env var is absent for this test.
    std::env::remove_var("PHANTOM_AUTO_APPROVE");

    let result = shell::run(&json!({"command": "rm test.txt"})).await;

    assert!(
        result.contains("APPROVAL_REQUIRED"),
        "expected APPROVAL_REQUIRED when PHANTOM_AUTO_APPROVE is unset, got: {result}"
    );
}

/// A5: when PHANTOM_AUTO_APPROVE=1 is set, the command is executed rather than
/// being held for approval.
///
/// Depends on A5 adding the approval gate to `shell::run`.
#[tokio::test]
async fn test_shell_auto_approve() {
    std::env::set_var("PHANTOM_AUTO_APPROVE", "1");

    let result = shell::run(&json!({"command": "echo auto_approved"})).await;

    std::env::remove_var("PHANTOM_AUTO_APPROVE");

    assert!(
        !result.contains("APPROVAL_REQUIRED"),
        "command should execute when PHANTOM_AUTO_APPROVE=1, but got APPROVAL_REQUIRED: {result}"
    );
    assert!(
        result.contains("auto_approved") || result.contains("[exit"),
        "expected command output when auto-approved, got: {result}"
    );
}

/// `false || echo fallback` — the fallback branch should execute and "fallback"
/// should appear in output.
///
/// The current `run_compound` splits only on `&&` and `;`, not `||`, so `false`
/// runs and its exit code is swallowed — "fallback" never appears. This test is
/// ignored until A5 fixes the `||` operator handling.
#[tokio::test]
async fn test_shell_compound_or_operator() {
    let result = shell::run(&json!({"command": "false || echo fallback"})).await;

    assert!(
        result.contains("fallback"),
        "expected 'fallback' in output of 'false || echo fallback', got: {result}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// A7 — session: auto_title, session_info, list_with_info, delete
// ═══════════════════════════════════════════════════════════════════════════

/// A7: auto_title sets a title of at most 60 characters derived from a long message.
///
/// `auto_title` stores the title and returns `()`. Read it back with `get_title`.
#[tokio::test]
async fn test_session_auto_title() {
    let dir = tempdir().unwrap();
    let store = ConversationStore::new_with_dir(dir.path().to_path_buf());

    let long_message = "This is a very long message that contains a lot of words \
                        and should be summarised into a short title by auto_title";

    store.auto_title("title-chat", long_message).await;

    let title = store
        .get_title("title-chat")
        .await
        .expect("expected a title to be stored by auto_title");

    assert!(
        title.len() <= 60,
        "auto_title should store at most 60 chars, got {} chars: {title:?}",
        title.len()
    );
    assert!(
        !title.is_empty(),
        "auto_title should store a non-empty title"
    );
}

/// A7: session_info returns a SessionInfo struct whose message_count equals the number
/// of messages appended (each `append` adds 2 messages: user + assistant).
#[tokio::test]
async fn test_session_info() {
    let dir = tempdir().unwrap();
    let store = ConversationStore::new_with_dir(dir.path().to_path_buf());

    store
        .append("info-chat", user_msg("first"), asst_msg("reply1"))
        .await;
    store
        .append("info-chat", user_msg("second"), asst_msg("reply2"))
        .await;

    let info = store.session_info("info-chat").await;

    assert_eq!(
        info.message_count, 4,
        "expected 4 messages (2 user + 2 asst), got {}",
        info.message_count
    );
    assert_eq!(info.id, "info-chat");
}

/// A7: delete removes the session so that list() no longer contains it.
#[tokio::test]
async fn test_session_delete() {
    let dir = tempdir().unwrap();
    let store = ConversationStore::new_with_dir(dir.path().to_path_buf());

    store
        .append("delete-chat", user_msg("hi"), asst_msg("hello"))
        .await;

    // Confirm it exists before deletion.
    let before = store.list().await;
    assert!(
        before.contains(&"delete-chat".to_string()),
        "session should exist before deletion"
    );

    let deleted = store.delete("delete-chat").await;
    assert!(
        deleted,
        "delete() should return true when the session existed"
    );

    let after = store.list().await;
    assert!(
        !after.contains(&"delete-chat".to_string()),
        "session should be gone after deletion, but list still contains it: {after:?}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// A4 — context: load_project_config() / walk-up from CWD
// ═══════════════════════════════════════════════════════════════════════════

/// `load_project_context` finds a PHANTOM.md placed directly in the given directory.
///
/// This exercises the direct-hit case of the walk-up logic already present in
/// `project_context::load_project_context`.
#[tokio::test]
async fn test_load_project_config_finds_phantom_md() {
    let dir = tempdir().unwrap();
    let phantom_md = dir.path().join("PHANTOM.md");
    std::fs::write(&phantom_md, "test content").unwrap();

    let result: Option<String> = project_context::load_project_context(dir.path()).await;

    assert!(
        result.is_some(),
        "expected Some(content) when PHANTOM.md is present"
    );
    let content = result.unwrap();
    assert!(
        content.contains("test content"),
        "expected 'test content' in loaded context, got: {content}"
    );
}

/// `load_project_context` walks up the directory tree to find PHANTOM.md placed
/// only in a parent directory.
#[tokio::test]
async fn test_load_project_config_walk_up() {
    let parent_dir = tempdir().unwrap();
    let child_dir = parent_dir.path().join("child");
    std::fs::create_dir_all(&child_dir).unwrap();

    // Place PHANTOM.md only in the parent.
    std::fs::write(
        parent_dir.path().join("PHANTOM.md"),
        "parent project context",
    )
    .unwrap();

    // Call from the child dir (no PHANTOM.md there).
    let result: Option<String> = project_context::load_project_context(&child_dir).await;

    assert!(
        result.is_some(),
        "expected walk-up to find PHANTOM.md in parent directory"
    );
    let content = result.unwrap();
    assert!(
        content.contains("parent project context"),
        "expected parent's PHANTOM.md content, got: {content}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// A8 — git: blame, show, branch, stash
// ═══════════════════════════════════════════════════════════════════════════

/// A8: git::blame returns blame annotation for a file.
#[tokio::test]
async fn test_git_blame() {
    use phantom_mesh::tools::git;

    // Use the project's own Cargo.toml so we know the file exists.
    let result = git::blame(&json!({
        "path": env!("CARGO_MANIFEST_DIR"),
        "file": "Cargo.toml"
    }))
    .await;

    assert!(
        !result.starts_with("Error"),
        "expected blame output, got error: {result}"
    );
    // git blame output should be non-empty for a real file.
    assert!(!result.trim().is_empty(), "expected non-empty blame output");
}

/// A8: git::show returns information about a specific ref.
#[tokio::test]
async fn test_git_show() {
    use phantom_mesh::tools::git;

    let result = git::show(&json!({
        "ref_": "HEAD"
    }))
    .await;

    // Should not start with "Error" and should contain something commit-like.
    assert!(
        !result.starts_with("Error"),
        "expected show output for HEAD, got: {result}"
    );
    assert!(
        !result.trim().is_empty(),
        "expected non-empty show output, got: {result}"
    );
}

/// A8: git::branch lists local branches without error.
#[tokio::test]
async fn test_git_branch() {
    use phantom_mesh::tools::git;

    let result = git::branch(&json!({})).await;

    assert!(
        !result.starts_with("Error"),
        "expected branch list, got: {result}"
    );
    // There should be at least one branch name.
    assert!(
        !result.trim().is_empty(),
        "expected non-empty branch listing"
    );
}

/// A8: git::stash list doesn't error (may return empty if no stashes).
#[tokio::test]
async fn test_git_stash() {
    use phantom_mesh::tools::git;

    let result = git::stash(&json!({"action": "list"})).await;

    assert!(
        !result.starts_with("Error"),
        "git stash list should not error, got: {result}"
    );
    // Output is either "(no stash entries)" stub or actual stash list — either is fine.
}

// ═══════════════════════════════════════════════════════════════════════════
// A10 — memory: list, delete, search, disk persistence
// ═══════════════════════════════════════════════════════════════════════════

/// A10: memory::list returns all stored keys.
///
/// Depends on A10 adding `memory::list`.
/// Tests memory::list, memory::delete, and memory::search together
/// using an isolated temp file. Running sequentially avoids env-var races.
#[tokio::test]
async fn test_memory_list_delete_search() {
    use phantom_mesh::tools::memory;

    let f = tempfile::NamedTempFile::new().unwrap();
    std::env::set_var("PHANTOM_MEMORY_FILE", f.path());

    // ── list ──────────────────────────────────────────────────────────────
    memory::store(&json!({"key": "list_key_a", "value": "value_a"})).await;
    memory::store(&json!({"key": "list_key_b", "value": "value_b"})).await;

    let list_result = memory::list(&json!({})).await;
    assert!(
        list_result.contains("list_key_a"),
        "expected 'list_key_a' in list, got: {list_result}"
    );
    assert!(
        list_result.contains("list_key_b"),
        "expected 'list_key_b' in list, got: {list_result}"
    );

    // ── delete ────────────────────────────────────────────────────────────
    memory::store(&json!({"key": "delete_me", "value": "temporary"})).await;
    let before = memory::recall(&json!({"key": "delete_me"})).await;
    assert!(
        before.contains("temporary"),
        "expected value before deletion, got: {before}"
    );

    let del_result = memory::delete(&json!({"key": "delete_me"})).await;
    assert!(
        !del_result.starts_with("Error"),
        "expected successful deletion, got: {del_result}"
    );

    let after = memory::recall(&json!({"key": "delete_me"})).await;
    assert!(
        !after.contains("temporary"),
        "expected key absent after deletion, got: {after}"
    );

    // ── search ────────────────────────────────────────────────────────────
    memory::store(&json!({"key": "search_needle_key", "value": "some_value"})).await;
    memory::store(&json!({"key": "unrelated_key", "value": "search_needle_in_value"})).await;
    memory::store(&json!({"key": "nothing_here", "value": "nothing_relevant"})).await;

    let search_result = memory::search(&json!({"query": "search_needle"})).await;

    std::env::remove_var("PHANTOM_MEMORY_FILE");

    assert!(
        search_result.contains("search_needle_key")
            || search_result.contains("search_needle_in_value"),
        "expected search_needle entries in search results, got: {search_result}"
    );
}
