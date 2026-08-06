use spectyn_mesh::tools::file;
use serde_json::json;
use std::sync::{Mutex, OnceLock};
use tempfile::tempdir;

// T7 fix (codex audit 2026-05-15): safe_path now confines results to a
// workspace-roots set (CWD + ~/.spectyn-mesh + SPECTYN_EXTRA_ALLOWED_ROOTS).
// All file::{read,write,edit} tests below operate on tempdirs that are
// nowhere near CWD or $HOME, so we have to whitelist each tempdir before
// touching it. Env vars are process-global; the mutex below serialises
// access so parallel tests don't clobber each other's whitelist.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// Append `td.path()` to `SPECTYN_EXTRA_ALLOWED_ROOTS`. Caller must hold an
/// `env_lock()` guard for the duration of the test.
fn allow_tempdir(td: &tempfile::TempDir) {
    let p = td.path().to_string_lossy().to_string();
    let prev = std::env::var("SPECTYN_EXTRA_ALLOWED_ROOTS").unwrap_or_default();
    let sep = if cfg!(windows) { ";" } else { ":" };
    let merged = if prev.is_empty() {
        p
    } else {
        format!("{prev}{sep}{p}")
    };
    std::env::set_var("SPECTYN_EXTRA_ALLOWED_ROOTS", merged);
}

// ---------------------------------------------------------------------------
// safe_path
// ---------------------------------------------------------------------------

#[test]
fn test_safe_path_existing() {
    let _g = env_lock();
    let dir = tempdir().unwrap();
    allow_tempdir(&dir);
    let file_path = dir.path().join("exists.txt");
    std::fs::write(&file_path, "hi").unwrap();

    let result = file::safe_path(file_path.to_str().unwrap()).unwrap();

    // canonicalize resolves symlinks; the result should end with the same
    // file name and actually point to an existing path.
    assert_eq!(result.file_name().unwrap(), "exists.txt");
    assert!(result.is_absolute());
    std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
}

#[test]
fn test_safe_path_new_file() {
    let _g = env_lock();
    let dir = tempdir().unwrap();
    allow_tempdir(&dir);
    let file_path = dir.path().join("new_file.txt");

    // File does not exist yet — safe_path should still return a valid path.
    let result = file::safe_path(file_path.to_str().unwrap()).unwrap();

    assert_eq!(result.file_name().unwrap(), "new_file.txt");
    // The parent directory (tempdir) must exist.
    assert!(result.parent().unwrap().exists());
    std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
}

// ---------------------------------------------------------------------------
// file_write + file_read roundtrip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_write_then_read() {
    let _g = env_lock();
    let dir = tempdir().unwrap();
    allow_tempdir(&dir);
    let file_path = dir.path().join("hello.txt");
    let path_str = file_path.to_str().unwrap();

    let write_result = file::write(&json!({
        "path": path_str,
        "content": "hello world"
    }))
    .await;
    assert!(
        write_result.starts_with("Written"),
        "unexpected write result: {write_result}"
    );

    let read_result = file::read(&json!({ "path": path_str })).await;
    assert_eq!(read_result, "hello world");
    std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
}

// ---------------------------------------------------------------------------
// file_write creates parent directories
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_write_creates_parents() {
    let _g = env_lock();
    let dir = tempdir().unwrap();
    allow_tempdir(&dir);
    let nested = dir.path().join("nested").join("path").join("file.txt");
    let path_str = nested.to_str().unwrap();

    let result = file::write(&json!({
        "path": path_str,
        "content": "deep content"
    }))
    .await;

    assert!(
        result.starts_with("Written"),
        "expected write success but got: {result}"
    );
    assert!(nested.exists(), "file should have been created");
    assert_eq!(std::fs::read_to_string(&nested).unwrap(), "deep content");
    std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
}

// ---------------------------------------------------------------------------
// file_edit
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_edit_replaces_once() {
    let _g = env_lock();
    let dir = tempdir().unwrap();
    allow_tempdir(&dir);
    let file_path = dir.path().join("edit_me.txt");
    let path_str = file_path.to_str().unwrap();

    std::fs::write(&file_path, "foo bar baz").unwrap();

    let result = file::edit(&json!({
        "path": path_str,
        "old_string": "bar",
        "new_string": "qux"
    }))
    .await;

    assert!(
        result.starts_with("Edited") || result.contains("successfully"),
        "unexpected edit result: {result}"
    );

    let content = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "foo qux baz");
    std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
}

#[tokio::test]
async fn test_edit_not_found() {
    let _g = env_lock();
    let dir = tempdir().unwrap();
    allow_tempdir(&dir);
    let file_path = dir.path().join("no_match.txt");
    let path_str = file_path.to_str().unwrap();

    std::fs::write(&file_path, "something entirely different").unwrap();

    let result = file::edit(&json!({
        "path": path_str,
        "old_string": "THIS_DOES_NOT_EXIST",
        "new_string": "irrelevant"
    }))
    .await;

    assert!(
        result.contains("not found"),
        "expected 'not found' error but got: {result}"
    );
    std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
}

#[tokio::test]
async fn test_edit_ambiguous() {
    let _g = env_lock();
    let dir = tempdir().unwrap();
    allow_tempdir(&dir);
    let file_path = dir.path().join("ambiguous.txt");
    let path_str = file_path.to_str().unwrap();

    // "repeat" appears 3 times.
    std::fs::write(&file_path, "repeat repeat repeat").unwrap();

    let result = file::edit(&json!({
        "path": path_str,
        "old_string": "repeat",
        "new_string": "once"
    }))
    .await;

    assert!(
        result.contains("3 times"),
        "expected ambiguity error mentioning '3 times' but got: {result}"
    );
    std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
}

// ---------------------------------------------------------------------------
// file_read edge cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_read_nonexistent() {
    let _g = env_lock();
    let dir = tempdir().unwrap();
    allow_tempdir(&dir);
    let missing = dir.path().join("does_not_exist.txt");
    let path_str = missing.to_str().unwrap();

    let result = file::read(&json!({ "path": path_str })).await;

    assert!(
        result.starts_with("Error"),
        "expected an error string but got: {result}"
    );
    std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
}

#[tokio::test]
async fn test_read_large_file() {
    let _g = env_lock();
    let dir = tempdir().unwrap();
    allow_tempdir(&dir);
    let big_file = dir.path().join("large.txt");
    let path_str = big_file.to_str().unwrap();

    // 150_000 'x' characters — well above the 100_000-char truncation limit.
    let big_content: String = "x".repeat(150_000);
    std::fs::write(&big_file, &big_content).unwrap();

    let result = file::read(&json!({ "path": path_str })).await;

    assert!(
        result.contains("truncated") || result.contains("omitted"),
        "expected truncation marker in output but got a string of length {}",
        result.len()
    );
    // The result must be shorter than the original content.
    assert!(
        result.len() < big_content.len(),
        "truncated output should be shorter than the original"
    );
    std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
}

// ---------------------------------------------------------------------------
// path traversal guard
// ---------------------------------------------------------------------------

#[test]
fn test_no_path_traversal() {
    let _g = env_lock();
    std::env::remove_var("SPECTYN_EXTRA_ALLOWED_ROOTS");
    // T7 fix (codex audit 2026-05-15): safe_path MUST reject ../../etc/passwd.
    let result = file::safe_path("../../etc/passwd");

    if let Ok(p) = &result {
        assert_ne!(
            p,
            &std::path::PathBuf::from("/etc/passwd"),
            "safe_path resolved to /etc/passwd — path traversal possible!"
        );
        // Stricter: result must be inside CWD (or another allowed root).
        let cwd = std::env::current_dir().unwrap();
        let cwd = cwd.canonicalize().unwrap_or(cwd);
        assert!(
            p.starts_with(&cwd) || p.to_string_lossy().contains(".spectyn-mesh"),
            "safe_path returned {p:?} which is outside CWD {cwd:?} and not in .spectyn-mesh"
        );
    }
}
