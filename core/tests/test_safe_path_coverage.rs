//! [T7f] safe_path coverage sweep — regression tests for HIGH findings
//! H-6, H-7, H-8 from PR #75 audit (2026-05-15).
//!
//! Audit findings (verbatim):
//!   H-6: `diff_view::diff_files` reads path_a/path_b without `safe_path`,
//!        letting a model exfiltrate `/etc/passwd` as a "diff" against
//!        any in-workspace file.
//!   H-7: `ls::list` and `ls::stat` accept any absolute path, exposing
//!        `/root`, `~/.ssh`, etc. directory listings.
//!   H-8: `fs::rename_file` guards `src` via safe_path but the `dst`
//!        path bypasses it entirely — write-anywhere primitive.
//!
//! Each test pair: pre-fix proves the attack vector works (or at least
//! reaches the dangerous path), post-fix proves the call is rejected
//! with a workspace-boundary error and matching no-side-effect on the
//! sensitive target.

use serde_json::json;
use std::sync::{Mutex, OnceLock};

/// Shared lock — these tests mutate `PHANTOM_EXTRA_ALLOWED_ROOTS` and
/// must serialise with anything else that reads or writes that env var
/// (see `test_security_t7.rs::env_guard`). Env vars are process-global.
fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// A path that is guaranteed outside CWD and ~/.phantom-mesh on every
/// supported dev OS. We never write to it; we only check that tools
/// refuse to *read* / *touch* it.
fn outside_path() -> &'static str {
    if cfg!(windows) {
        "C:\\Windows\\System32\\drivers\\etc\\hosts"
    } else {
        "/etc/hostname"
    }
}

// ── H-6 diff_view::diff_files ───────────────────────────────────────────────

use phantom_mesh::tools::diff_view as ph_diff;

#[tokio::test]
async fn diff_files_rejects_absolute_path_a_outside_workspace() {
    let _g = env_guard();
    std::env::remove_var("PHANTOM_EXTRA_ALLOWED_ROOTS");

    // path_a points at a sensitive system file. With the fix in place
    // safe_path() must refuse before any I/O happens.
    let r = ph_diff::diff_files(&json!({
        "path_a": outside_path(),
        "path_b": "Cargo.toml",
    }))
    .await;

    let rl = r.to_lowercase();
    assert!(
        rl.contains("outside") || rl.contains("workspace") || rl.contains("invalid path"),
        "diff_files must reject absolute outside-workspace path_a, got: {r}"
    );
}

#[tokio::test]
async fn diff_files_rejects_absolute_path_b_outside_workspace() {
    let _g = env_guard();
    std::env::remove_var("PHANTOM_EXTRA_ALLOWED_ROOTS");

    let r = ph_diff::diff_files(&json!({
        "path_a": "Cargo.toml",
        "path_b": outside_path(),
    }))
    .await;

    let rl = r.to_lowercase();
    assert!(
        rl.contains("outside") || rl.contains("workspace") || rl.contains("invalid path"),
        "diff_files must reject absolute outside-workspace path_b, got: {r}"
    );
}

#[tokio::test]
async fn diff_files_accepts_paths_inside_workspace() {
    let _g = env_guard();
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    std::fs::write(&a, "hello\n").unwrap();
    std::fs::write(&b, "world\n").unwrap();

    let prev = std::env::var("PHANTOM_EXTRA_ALLOWED_ROOTS").ok();
    std::env::set_var(
        "PHANTOM_EXTRA_ALLOWED_ROOTS",
        dir.path().to_string_lossy().to_string(),
    );

    let r = ph_diff::diff_files(&json!({
        "path_a": a.to_str().unwrap(),
        "path_b": b.to_str().unwrap(),
    }))
    .await;

    if let Some(v) = prev {
        std::env::set_var("PHANTOM_EXTRA_ALLOWED_ROOTS", v);
    } else {
        std::env::remove_var("PHANTOM_EXTRA_ALLOWED_ROOTS");
    }

    // A real diff contains hunk headers; a workspace-rejection contains "outside".
    assert!(
        r.contains("@@") || r.contains("---"),
        "in-workspace diff must produce a real diff, got: {r}"
    );
}

// ── H-7 ls::list / ls::stat ─────────────────────────────────────────────────

use phantom_mesh::tools::ls as ph_ls;

#[tokio::test]
async fn ls_list_rejects_absolute_path_outside_workspace() {
    let _g = env_guard();
    std::env::remove_var("PHANTOM_EXTRA_ALLOWED_ROOTS");

    // /etc and C:\Windows\System32 always exist; pre-fix this would dump
    // the directory contents, post-fix it must be a workspace rejection.
    let target = if cfg!(windows) {
        "C:\\Windows\\System32"
    } else {
        "/etc"
    };
    let r = ph_ls::list(&json!({ "path": target })).await;

    let rl = r.to_lowercase();
    assert!(
        rl.contains("outside") || rl.contains("workspace") || rl.contains("invalid path"),
        "ls must reject absolute outside-workspace path, got: {r}"
    );
}

#[tokio::test]
async fn ls_stat_rejects_absolute_path_outside_workspace() {
    let _g = env_guard();
    std::env::remove_var("PHANTOM_EXTRA_ALLOWED_ROOTS");

    let r = ph_ls::stat(&json!({ "path": outside_path() })).await;

    let rl = r.to_lowercase();
    assert!(
        rl.contains("outside") || rl.contains("workspace") || rl.contains("invalid path"),
        "stat must reject absolute outside-workspace path, got: {r}"
    );
}

#[tokio::test]
async fn ls_list_accepts_directory_inside_workspace() {
    let _g = env_guard();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "x").unwrap();

    let prev = std::env::var("PHANTOM_EXTRA_ALLOWED_ROOTS").ok();
    std::env::set_var(
        "PHANTOM_EXTRA_ALLOWED_ROOTS",
        dir.path().to_string_lossy().to_string(),
    );

    let r = ph_ls::list(&json!({ "path": dir.path().to_str().unwrap() })).await;

    if let Some(v) = prev {
        std::env::set_var("PHANTOM_EXTRA_ALLOWED_ROOTS", v);
    } else {
        std::env::remove_var("PHANTOM_EXTRA_ALLOWED_ROOTS");
    }

    assert!(
        r.contains("hello.txt"),
        "in-workspace ls must list the file, got: {r}"
    );
}

// ── H-8 fs::rename_file dst path ────────────────────────────────────────────

use phantom_mesh::tools::fs as ph_fs;

#[tokio::test]
async fn rename_rejects_dst_outside_workspace() {
    let _g = env_guard();
    std::env::remove_var("PHANTOM_EXTRA_ALLOWED_ROOTS");
    std::env::set_var("PHANTOM_AUTO_APPROVE", "1");

    // Create an in-workspace src so safe_path(src) succeeds, isolating
    // the test to the dst-side gap.
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var(
        "PHANTOM_EXTRA_ALLOWED_ROOTS",
        dir.path().to_string_lossy().to_string(),
    );
    let src = dir.path().join("src.txt");
    std::fs::write(&src, "content").unwrap();

    // dst points outside any allowed root. Use a unique filename so a
    // pre-existing file in C:\Windows\Temp / /tmp can't false-positive
    // the post-rename existence check.
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let bad_dst = if cfg!(windows) {
        format!("C:\\Windows\\Temp\\phantom_pwn_dst_{unique}.txt")
    } else {
        format!("/tmp/phantom_pwn_dst_{unique}.txt")
    };
    // Sanity-check it doesn't exist before the call.
    assert!(
        !std::path::Path::new(&bad_dst).exists(),
        "test setup error: {bad_dst} unexpectedly already exists"
    );

    let r = ph_fs::rename_file(&json!({
        "src": src.to_str().unwrap(),
        "dst": &bad_dst,
    }))
    .await;

    std::env::remove_var("PHANTOM_EXTRA_ALLOWED_ROOTS");
    std::env::remove_var("PHANTOM_AUTO_APPROVE");

    let rl = r.to_lowercase();
    assert!(
        rl.contains("outside") || rl.contains("workspace") || rl.contains("invalid"),
        "rename must reject dst outside workspace, got: {r}"
    );
    // And the rename must NOT have happened.
    assert!(src.exists(), "src should still exist after rejected rename");
    assert!(
        !std::path::Path::new(&bad_dst).exists(),
        "dst at {bad_dst} must not have been created by the rejected rename"
    );
}

#[tokio::test]
async fn rename_accepts_dst_inside_workspace() {
    let _g = env_guard();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var(
        "PHANTOM_EXTRA_ALLOWED_ROOTS",
        dir.path().to_string_lossy().to_string(),
    );
    std::env::set_var("PHANTOM_AUTO_APPROVE", "1");

    let src = dir.path().join("src.txt");
    let dst = dir.path().join("dst.txt");
    std::fs::write(&src, "content").unwrap();

    let r = ph_fs::rename_file(&json!({
        "src": src.to_str().unwrap(),
        "dst": dst.to_str().unwrap(),
    }))
    .await;

    std::env::remove_var("PHANTOM_EXTRA_ALLOWED_ROOTS");
    std::env::remove_var("PHANTOM_AUTO_APPROVE");

    assert!(
        r.contains("Renamed"),
        "in-workspace rename must succeed, got: {r}"
    );
    assert!(!src.exists(), "src should be gone after rename");
    assert!(dst.exists(), "dst should exist after rename");
}
