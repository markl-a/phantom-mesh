use spectyn_mesh::tools::{git, search};
use serde_json::json;

mod common;
use common::workspace_tempdir;

// ── content_search ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_content_search_finds_match() {
    let dir = workspace_tempdir();
    let file_path = dir.path().join("haystack.txt");
    std::fs::write(&file_path, "line one\nneedle_xyz is here\nline three\n").unwrap();

    let args = json!({
        "pattern": "needle_xyz",
        "path": dir.path().to_str().unwrap()
    });
    let result = search::content(&args).await;

    assert!(
        result.contains("needle_xyz"),
        "expected 'needle_xyz' in result, got: {result}"
    );
    assert!(
        result.contains("haystack.txt"),
        "expected filename 'haystack.txt' in result, got: {result}"
    );
}

#[tokio::test]
async fn test_content_search_no_match() {
    let dir = workspace_tempdir();
    std::fs::write(dir.path().join("data.txt"), "foo bar baz\n").unwrap();

    let args = json!({
        "pattern": "zzz_not_found_zzz",
        "path": dir.path().to_str().unwrap()
    });
    let result = search::content(&args).await;

    assert_eq!(result, "No matches found");
}

#[tokio::test]
async fn test_content_search_missing_pattern() {
    // Pass an empty JSON object — no "pattern" key at all.
    let args = json!({});
    let result = search::content(&args).await;

    assert!(
        result.starts_with("Error:") && result.to_lowercase().contains("missing"),
        "expected a missing-pattern error, got: {result}"
    );
}

// ── glob_search ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_glob_finds_rs_files() {
    let dir = workspace_tempdir();
    std::fs::write(dir.path().join("a.rs"), "fn main() {}").unwrap();
    std::fs::write(dir.path().join("b.ts"), "export {}").unwrap();

    let args = json!({
        "pattern": "**/*.rs",
        "path": dir.path().to_str().unwrap()
    });
    let result = search::glob(&args).await;

    assert!(
        result.contains("a.rs"),
        "expected 'a.rs' in glob result, got: {result}"
    );
    assert!(
        !result.contains("b.ts"),
        "did not expect 'b.ts' in glob result, got: {result}"
    );
}

#[tokio::test]
async fn test_glob_no_match() {
    let dir = workspace_tempdir();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();

    let args = json!({
        "pattern": "**/*.xyz",
        "path": dir.path().to_str().unwrap()
    });
    let result = search::glob(&args).await;

    assert_eq!(result, "No files found");
}

// ── git_status ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_git_status_runs() {
    // Use the core crate directory itself, which is within CWD when tests run.
    let cwd = std::env::current_dir().unwrap();
    let args = json!({
        "path": cwd.to_str().unwrap()
    });
    let result = git::status(&args).await;

    // Must be either "Working tree clean" or a status listing — never empty,
    // never a panic. We just assert it is a non-empty string.
    assert!(!result.is_empty(), "expected a non-empty git status result");
}

#[tokio::test]
async fn test_git_status_bad_path() {
    let args = json!({
        "path": "/tmp/not_a_git_repo_xyzabc"
    });
    let result = git::status(&args).await;

    // Should return some error message rather than panicking.
    // git writes "fatal: ..." to stderr for non-repos; our wrapper may also
    // surface it as a "git error: ..." string. Either way it must be non-empty.
    assert!(
        !result.is_empty(),
        "expected an error message for a non-git path, got empty string"
    );
}

// ── git_log ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_git_log_returns_commits() {
    // Use the core crate directory itself, which is within CWD when tests run.
    let cwd = std::env::current_dir().unwrap();
    let args = json!({
        "path": cwd.to_str().unwrap(),
        "n": 5
    });
    let result = git::log(&args).await;

    // `git log --oneline` lines start with a short hash (7+ hex chars).
    // We verify at least one such token is present.
    let has_short_hash = result.lines().any(|line| {
        line.split_whitespace()
            .next()
            .map(|tok| tok.len() >= 7 && tok.chars().all(|c| c.is_ascii_hexdigit()))
            .unwrap_or(false)
    });

    assert!(
        has_short_hash,
        "expected commit hashes in git log output, got: {result}"
    );
}

// ── git_commit (SKIPPED) ──────────────────────────────────────────────────
// git_commit is intentionally not tested here to avoid creating real commits.
