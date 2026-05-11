use phantom_mesh::tools::file;
use serde_json::json;
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// safe_path
// ---------------------------------------------------------------------------

#[test]
fn test_safe_path_existing() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("exists.txt");
    std::fs::write(&file_path, "hi").unwrap();

    let result = file::safe_path(file_path.to_str().unwrap()).unwrap();

    // canonicalize resolves symlinks; the result should end with the same
    // file name and actually point to an existing path.
    assert_eq!(result.file_name().unwrap(), "exists.txt");
    assert!(result.is_absolute());
}

#[test]
fn test_safe_path_new_file() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("new_file.txt");

    // File does not exist yet — safe_path should still return a valid path.
    let result = file::safe_path(file_path.to_str().unwrap()).unwrap();

    assert_eq!(result.file_name().unwrap(), "new_file.txt");
    // The parent directory (tempdir) must exist.
    assert!(result.parent().unwrap().exists());
}

// ---------------------------------------------------------------------------
// file_write + file_read roundtrip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_write_then_read() {
    let dir = tempdir().unwrap();
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
}

// ---------------------------------------------------------------------------
// file_write creates parent directories
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_write_creates_parents() {
    let dir = tempdir().unwrap();
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
}

// ---------------------------------------------------------------------------
// file_edit
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_edit_replaces_once() {
    let dir = tempdir().unwrap();
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
}

#[tokio::test]
async fn test_edit_not_found() {
    let dir = tempdir().unwrap();
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
}

#[tokio::test]
async fn test_edit_ambiguous() {
    let dir = tempdir().unwrap();
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
}

// ---------------------------------------------------------------------------
// file_read edge cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_read_nonexistent() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("does_not_exist.txt");
    let path_str = missing.to_str().unwrap();

    let result = file::read(&json!({ "path": path_str })).await;

    assert!(
        result.starts_with("Error"),
        "expected an error string but got: {result}"
    );
}

#[tokio::test]
async fn test_read_large_file() {
    let dir = tempdir().unwrap();
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
}

// ---------------------------------------------------------------------------
// path traversal guard
// ---------------------------------------------------------------------------

#[test]
fn test_no_path_traversal() {
    // A naively resolved "../../etc/passwd" from some CWD could reach /etc/passwd.
    // safe_path must not return /etc/passwd.
    let result = file::safe_path("../../etc/passwd");

    match result {
        Ok(p) => {
            // If it returns Ok, the resolved path must NOT be /etc/passwd.
            assert_ne!(
                p,
                std::path::PathBuf::from("/etc/passwd"),
                "safe_path resolved to /etc/passwd — path traversal possible!"
            );
        }
        // An Err is also an acceptable response (e.g., parent doesn't exist).
        Err(_) => {}
    }
}
