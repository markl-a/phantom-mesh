use phantom_mesh::cost::CostTracker;
use phantom_mesh::scaffold;
/// Phase-3 integration tests.
///
/// Tests that depend on features not yet implemented are marked `#[ignore]`
/// with a comment describing which feature is needed.
use phantom_mesh::tools::{fetch, fs as phantom_fs, git};
use serde_json::json;
use tempfile::tempdir;
use tokio::net::TcpListener;

fn env_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Spin up a minimal axum HTTP server that serves the given `body` with the
/// given `content_type` on GET /. Returns the base URL.
async fn start_http_mock(body: &'static str, content_type: &'static str) -> String {
    use axum::routing::get;
    use axum::Router;

    let app = Router::new().route(
        "/",
        get(move || async move {
            axum::response::Response::builder()
                .status(200)
                .header("content-type", content_type)
                .body(axum::body::Body::from(body))
                .expect("build mock response")
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock server");
    let addr = listener.local_addr().expect("local addr");

    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("mock server error");
    });

    format!("http://127.0.0.1:{}", addr.port())
}

/// Build a fresh `CostTracker` isolated to a temp dir so tests don't touch
/// `~/.phantom-mesh/costs.json` and don't interfere with each other.
fn fresh_tracker(dir: &tempfile::TempDir) -> CostTracker {
    std::env::set_var("HOME", dir.path());
    CostTracker::new()
}

mod common;
use common::workspace_tempdir;

// ═══════════════════════════════════════════════════════════════════════════
// 1. File tools — list_dir
// ═══════════════════════════════════════════════════════════════════════════

/// list_dir on a temp dir with 3 files — all three names appear in the output.
#[tokio::test]
async fn test_file_list_directory() {
    let dir = workspace_tempdir();
    for name in &["alpha.txt", "beta.txt", "gamma.txt"] {
        std::fs::write(dir.path().join(name), "content").unwrap();
    }

    let result = phantom_fs::list_dir(&json!({ "path": dir.path().to_str().unwrap() })).await;

    assert!(
        result.contains("alpha.txt"),
        "expected alpha.txt in listing, got: {result}"
    );
    assert!(
        result.contains("beta.txt"),
        "expected beta.txt in listing, got: {result}"
    );
    assert!(
        result.contains("gamma.txt"),
        "expected gamma.txt in listing, got: {result}"
    );
}

/// list_dir sorts alphabetically; a subdirectory entry (annotated "dir")
/// should appear before plain files when the name sorts first.
#[tokio::test]
async fn test_file_list_sorts_dirs_first() {
    let dir = workspace_tempdir();
    // Subdirectory name starts with 'a' so it sorts before files starting with 'b'/'c'.
    std::fs::create_dir(dir.path().join("aaa_subdir")).unwrap();
    std::fs::write(dir.path().join("bbb_file.txt"), "x").unwrap();
    std::fs::write(dir.path().join("ccc_file.txt"), "y").unwrap();

    let result = phantom_fs::list_dir(&json!({ "path": dir.path().to_str().unwrap() })).await;

    // The dir entry and file entries should both be present.
    assert!(
        result.contains("aaa_subdir"),
        "expected subdir in listing, got: {result}"
    );
    assert!(
        result.contains("bbb_file.txt"),
        "expected bbb_file.txt in listing, got: {result}"
    );

    // Because list_dir sorts all entries alphabetically, "aaa_subdir" (dir)
    // comes before "bbb_file.txt" (file) in the output string.
    let pos_dir = result
        .find("aaa_subdir")
        .expect("aaa_subdir should be in output");
    let pos_file = result
        .find("bbb_file.txt")
        .expect("bbb_file.txt should be in output");
    assert!(
        pos_dir < pos_file,
        "directory 'aaa_subdir' should appear before 'bbb_file.txt' in sorted output"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 3-4. File tools — delete_file
// ═══════════════════════════════════════════════════════════════════════════

/// delete_file without PHANTOM_AUTO_APPROVE should return APPROVAL_REQUIRED.
///
/// NOTE: The current `fs::delete_file` implementation does not have an
/// APPROVAL_REQUIRED gate — it deletes unconditionally. This test is ignored
/// until a future agent adds the gate.
#[tokio::test]
async fn test_file_delete_requires_approval() {
    let _g = env_lock().lock().await;
    std::env::remove_var("PHANTOM_AUTO_APPROVE");

    let dir = workspace_tempdir();
    let file = dir.path().join("to_delete.txt");
    std::fs::write(&file, "data").unwrap();

    let result = phantom_fs::delete_file(&json!({ "path": file.to_str().unwrap() })).await;

    assert!(
        result.contains("APPROVAL_REQUIRED"),
        "expected APPROVAL_REQUIRED without env var, got: {result}"
    );
    assert!(
        file.exists(),
        "file should not have been deleted without approval"
    );
}

#[tokio::test]
async fn test_file_delete_with_approval() {
    let _g = env_lock().lock().await;

    let dir = workspace_tempdir();
    let file = dir.path().join("should_delete.txt");
    std::fs::write(&file, "data").unwrap();
    assert!(file.exists(), "pre-condition: file must exist");

    std::env::set_var("PHANTOM_AUTO_APPROVE", "1");
    let result = phantom_fs::delete_file(&json!({ "path": file.to_str().unwrap() })).await;
    std::env::remove_var("PHANTOM_AUTO_APPROVE");

    assert!(
        result.contains("Deleted"),
        "expected 'Deleted' in result, got: {result}"
    );
    assert!(!file.exists(), "file should be gone after delete_file");
}

// ═══════════════════════════════════════════════════════════════════════════
// 5-6. File tools — rename_file (not yet implemented)
// ═══════════════════════════════════════════════════════════════════════════

/// rename_file without PHANTOM_AUTO_APPROVE should return APPROVAL_REQUIRED.
#[tokio::test]
async fn test_file_rename_requires_approval() {
    let _g = env_lock().lock().await;
    std::env::remove_var("PHANTOM_AUTO_APPROVE");

    let dir = workspace_tempdir();
    let src = dir.path().join("old_name.txt");
    std::fs::write(&src, "data").unwrap();

    let result = phantom_fs::rename_file(&json!({
        "src": src.to_str().unwrap(),
        "dst": dir.path().join("new_name.txt").to_str().unwrap()
    }))
    .await;

    assert!(
        result.contains("APPROVAL_REQUIRED"),
        "expected APPROVAL_REQUIRED without env var, got: {result}"
    );
}

/// rename_file with PHANTOM_AUTO_APPROVE=1 renames the file.
#[tokio::test]
async fn test_file_rename_with_approval() {
    let _g = env_lock().lock().await;
    std::env::set_var("PHANTOM_AUTO_APPROVE", "1");

    let dir = workspace_tempdir();
    let src = dir.path().join("before.txt");
    let dst = dir.path().join("after.txt");
    std::fs::write(&src, "data").unwrap();

    let result = phantom_fs::rename_file(&json!({
        "src": src.to_str().unwrap(),
        "dst": dst.to_str().unwrap()
    }))
    .await;

    std::env::remove_var("PHANTOM_AUTO_APPROVE");

    assert!(
        !result.starts_with("Error"),
        "expected success, got: {result}"
    );
    assert!(!src.exists(), "source file should be gone after rename");
    assert!(dst.exists(), "destination file should exist after rename");
}

// ═══════════════════════════════════════════════════════════════════════════
// 7-8. Shell background jobs (not yet implemented)
// ═══════════════════════════════════════════════════════════════════════════

/// shell::run_bg spawns a process in the background and returns a line
/// containing "PID=" and a positive integer.
#[tokio::test]
async fn test_shell_bg_returns_pid() {
    use phantom_mesh::tools::shell;

    let result = shell::run_bg(&json!({ "command": "sleep 5" })).await;

    assert!(
        result.contains("PID="),
        "expected 'PID=' in background job output, got: {result}"
    );

    // Extract PID and verify the process is running.
    // The output format is "Job started: PID=<pid> label='...'\n..."
    if let Some(pid_str) = result.split("PID=").nth(1) {
        let pid_str = pid_str.split_whitespace().next().unwrap_or("").trim();
        let pid: u32 = pid_str.parse().expect("PID should be a number");
        assert!(pid > 0, "PID should be positive, got {pid}");

        // On Unix, kill -0 checks if the process exists without sending a signal.
        #[cfg(unix)]
        {
            let check = std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .status()
                .expect("kill -0 command failed");
            assert!(check.success(), "process PID={pid} should be running");

            // Clean up: kill the background sleep.
            let _ = std::process::Command::new("kill")
                .arg(pid.to_string())
                .status();
        }
    }
}

/// shell::check_bg on a running background process reports "running".
#[tokio::test]
async fn test_shell_check_bg_running() {
    use phantom_mesh::tools::shell;

    let bg_result = shell::run_bg(&json!({ "command": "sleep 10" })).await;
    assert!(
        bg_result.contains("PID="),
        "expected PID= from run_bg, got: {bg_result}"
    );

    let pid_str = bg_result.split("PID=").nth(1).unwrap().trim().to_string();

    let check_result = shell::check_bg(&json!({ "pid": pid_str })).await;
    assert!(
        check_result.contains("running"),
        "expected 'running' status for active background process, got: {check_result}"
    );

    // Clean up.
    #[cfg(unix)]
    let _ = std::process::Command::new("kill").arg(&pid_str).status();
}

// ═══════════════════════════════════════════════════════════════════════════
// 9-11. fetch_url
// ═══════════════════════════════════════════════════════════════════════════

/// fetch_url against a local axum server serving plain HTML extracts visible
/// text — specifically the paragraph "Hello World".
///
/// NOTE: The fetch_url implementation blocks 127.0.0.1, so we use the mock
/// server's dynamic port via the URL returned from start_http_mock.
/// Because the blocking check rejects 127.0.0.1, this test uses a workaround:
/// we spin up the server on 127.0.0.1 but we know fetch_url will reject it.
/// The correct approach is to test HTML extraction via the public extract
/// logic; we call the internal mock indirectly through the fetch module's
/// public function, but we accept that the IP block will fire.
///
/// Therefore this test is marked #[ignore] until fetch_url allows configurable
/// allow-lists or we expose an internal extraction helper.
#[tokio::test]
async fn test_fetch_url_extracts_text() {
    let base_url =
        start_http_mock("<html><body><p>Hello World</p></body></html>", "text/html").await;
    let url = format!("{}/", base_url);

    std::env::set_var("PHANTOM_FETCH_ALLOW_LOCAL", "1");
    let result = fetch::fetch_url(&json!({ "url": url })).await;
    std::env::remove_var("PHANTOM_FETCH_ALLOW_LOCAL");

    assert!(
        result.contains("Hello World"),
        "expected 'Hello World' extracted from HTML, got: {result}"
    );
}

/// fetch_url with a private IPv4 address returns an error.
#[tokio::test]
async fn test_fetch_url_blocks_private_ip() {
    let result = fetch::fetch_url(&json!({ "url": "http://127.0.0.1:9999/" })).await;
    assert!(
        result.starts_with("Error:"),
        "expected Error: for private IP, got: {result}"
    );
}

/// fetch_url rejects non-HTTP(S) schemes.
#[tokio::test]
async fn test_fetch_url_blocks_non_http() {
    let result = fetch::fetch_url(&json!({ "url": "ftp://example.com" })).await;
    assert!(
        result.starts_with("Error:"),
        "expected Error: for ftp:// scheme, got: {result}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 12-14. Git tools
// ═══════════════════════════════════════════════════════════════════════════

/// git_add with a path containing a semicolon should return an Error.
#[tokio::test]
async fn test_git_add_validates_path() {
    let result = git::add(&json!({ "path": "valid.rs; rm -rf /" })).await;
    assert!(
        result.starts_with("Error:"),
        "expected Error: for path with semicolon injection, got: {result}"
    );
}

/// git_push without PHANTOM_AUTO_APPROVE returns APPROVAL_REQUIRED.
///
/// Depends on a future agent adding `git::push` with approval gate.
#[tokio::test]
async fn test_git_push_requires_approval() {
    let _g = env_lock().lock().await;
    std::env::remove_var("PHANTOM_AUTO_APPROVE");

    let result = git::push(&json!({})).await;
    assert!(
        result.contains("APPROVAL_REQUIRED"),
        "expected APPROVAL_REQUIRED for git push without approval, got: {result}"
    );
}

#[tokio::test]
async fn test_git_reset_hard_requires_approval() {
    let _g = env_lock().lock().await;
    std::env::remove_var("PHANTOM_AUTO_APPROVE");

    let result = git::reset(&json!({ "mode": "hard" })).await;
    assert!(
        result.contains("APPROVAL_REQUIRED"),
        "expected APPROVAL_REQUIRED for hard reset without approval, got: {result}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 15-17. scaffold::generate_phantom_md
// ═══════════════════════════════════════════════════════════════════════════

/// generate_phantom_md detects a Rust project from Cargo.toml and includes
/// the package name and "Rust" in the output.
#[test]
fn test_scaffold_rust_project() {
    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test-proj\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let md = scaffold::generate_phantom_md(dir.path());

    assert!(
        md.contains("test-proj"),
        "expected project name 'test-proj' in scaffold output, got: {md}"
    );
    assert!(
        md.contains("Rust"),
        "expected 'Rust' project type in scaffold output, got: {md}"
    );
}

/// generate_phantom_md detects a Node.js project from package.json and
/// includes the package name in the output.
#[test]
fn test_scaffold_node_project() {
    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("package.json"),
        "{\n  \"name\": \"my-app\",\n  \"description\": \"test\"\n}\n",
    )
    .unwrap();

    let md = scaffold::generate_phantom_md(dir.path());

    assert!(
        md.contains("my-app"),
        "expected project name 'my-app' in scaffold output, got: {md}"
    );
}

/// generate_phantom_md on an empty directory reports "Unknown" project type.
#[test]
fn test_scaffold_unknown_project() {
    let dir = tempdir().unwrap();

    let md = scaffold::generate_phantom_md(dir.path());

    assert!(
        md.contains("Unknown"),
        "expected 'Unknown' project type for empty directory, got: {md}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 18-20. Cost tracking
// ═══════════════════════════════════════════════════════════════════════════

/// reset_session zeroes the session cost while preserving the lifetime total.
#[tokio::test]
async fn test_cost_session_reset() {
    let dir = tempdir().unwrap();
    let tracker = fresh_tracker(&dir);

    // Record some tokens so session_cost() > 0.
    // claude-sonnet-4-6 @ $3.0/MTok prompt — 1M tokens = $3.00
    tracker.record("claude-sonnet-4-6", 1_000_000, 0).await;

    let total_before = tracker.summary().await["total_usd"].as_f64().unwrap();
    assert!(total_before > 0.0, "total_usd should be > 0 before reset");

    let session_before = tracker.session_cost().await;
    assert!(
        session_before > 0.0,
        "session_cost should be > 0 before reset"
    );

    tracker.reset_session().await;

    assert_eq!(
        tracker.session_cost().await,
        0.0,
        "session_cost() should be 0.0 after reset_session()"
    );

    // Lifetime total must be unchanged.
    let total_after = tracker.summary().await["total_usd"].as_f64().unwrap();
    assert!(
        (total_after - total_before).abs() < 1e-6,
        "lifetime total should be unchanged after reset_session(), was {total_before}, now {total_after}"
    );
}

/// last_request_cost() > 0 after recording a real request.
#[tokio::test]
async fn test_cost_last_request() {
    let dir = tempdir().unwrap();
    let tracker = fresh_tracker(&dir);

    // o3 @ $10.0/MTok prompt — 1M tokens = $10.00
    tracker.record("o3", 1_000_000, 0).await;

    let last = tracker.last_request_cost().await;
    assert!(
        last > 0.0,
        "last_request_cost() should be > 0 after recording a request, got {last}"
    );
}

/// summary().by_model contains entries for each model used.
#[tokio::test]
async fn test_cost_by_model_breakdown() {
    let dir = tempdir().unwrap();
    let tracker = fresh_tracker(&dir);

    tracker.record("claude-sonnet-4-6", 500_000, 0).await;
    tracker.record("gpt-4.1", 500_000, 0).await;

    let summary = tracker.summary().await;
    let by_model = &summary["by_model"];

    assert!(
        by_model.get("claude-sonnet-4-6").is_some(),
        "expected 'claude-sonnet-4-6' in by_model breakdown, got: {by_model}"
    );
    assert!(
        by_model.get("gpt-4.1").is_some(),
        "expected 'gpt-4.1' in by_model breakdown, got: {by_model}"
    );

    // Sanity check: each model's cost should be positive.
    let sonnet_cost = by_model["claude-sonnet-4-6"]["cost_usd"]
        .as_f64()
        .unwrap_or(0.0);
    let gpt_cost = by_model["gpt-4.1"]["cost_usd"].as_f64().unwrap_or(0.0);
    assert!(
        sonnet_cost > 0.0,
        "claude-sonnet-4-6 cost should be > 0, got {sonnet_cost}"
    );
    assert!(gpt_cost > 0.0, "gpt-4.1 cost should be > 0, got {gpt_cost}");
}
