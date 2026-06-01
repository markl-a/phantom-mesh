use crate::tools::file::safe_path;
use serde_json::Value;
use std::path::PathBuf;

pub async fn list_files(args: &Value) -> String {
    let base = args["path"].as_str().unwrap_or(".");
    let pattern = args["pattern"].as_str().unwrap_or("");

    let base_path = PathBuf::from(base);
    if !base_path.exists() {
        return format!("Error: path '{}' does not exist", base);
    }

    let mut results = Vec::new();
    collect_files(&base_path, pattern, &mut results, 0);

    if results.is_empty() {
        return "No files found".into();
    }
    results.truncate(500); // max 500 files
    results.join("\n")
}

fn collect_files(dir: &PathBuf, pattern: &str, results: &mut Vec<String>, depth: usize) {
    if depth > 15 {
        return;
    }
    let skip_dirs = [
        "node_modules",
        ".git",
        "target",
        ".next",
        "dist",
        "__pycache__",
        ".cache",
    ];

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if path.is_dir() {
            if !skip_dirs.contains(&name_str.as_ref()) {
                collect_files(&path, pattern, results, depth + 1);
            }
        } else if pattern.is_empty() || matches_pattern(&name_str, pattern) {
            results.push(path.display().to_string());
        }
    }
}

fn matches_pattern(name: &str, pattern: &str) -> bool {
    // Simple glob: only support prefix*, *suffix, *middle*, exact
    if pattern.starts_with('*') && pattern.ends_with('*') {
        let mid = &pattern[1..pattern.len() - 1];
        return name.contains(mid);
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return name.ends_with(suffix);
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    name == pattern
}

pub async fn list_dir(args: &Value) -> String {
    let raw = args["path"].as_str().unwrap_or(".");

    // Path traversal guard
    if raw.contains("..") {
        return "Error: path traversal not allowed".into();
    }

    let path = match safe_path(raw) {
        Ok(p) => p,
        Err(e) => return format!("Error: invalid path: {}", e),
    };

    let mut entries = match tokio::fs::read_dir(&path).await {
        Ok(rd) => rd,
        Err(e) => return format!("Error reading directory {}: {}", path.display(), e),
    };

    let mut lines: Vec<String> = Vec::new();
    loop {
        match entries.next_entry().await {
            Ok(Some(entry)) => {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                let meta = entry.metadata().await;
                let size_str = match meta {
                    Ok(m) if m.is_file() => format!("{} bytes", m.len()),
                    Ok(m) if m.is_dir() => "dir".to_string(),
                    Ok(m) if m.is_symlink() => "symlink".to_string(),
                    _ => "?".to_string(),
                };
                lines.push(format!("{} ({})", name_str, size_str));
            }
            Ok(None) => break,
            Err(e) => {
                lines.push(format!("<error reading entry: {}>", e));
                break;
            }
        }
    }

    lines.sort();
    crate::tools::truncate(lines.join("\n"), 10_000)
}

pub async fn create_dir(args: &Value) -> String {
    let raw = match args["path"].as_str() {
        Some(p) => p,
        None => return "Error: missing 'path' argument".into(),
    };

    // Path traversal guard
    if raw.contains("..") {
        return "Error: path traversal not allowed".into();
    }

    let path = match safe_path(raw) {
        Ok(p) => p,
        Err(e) => return format!("Error: invalid path: {}", e),
    };

    // [C5/T74 V9 H-1] CO-EVO Phase 1 sandbox guard (SPEC-FREEZE-V1.1 §4.1-d).
    // Without this check, create_dir could materialise sandboxed paths (e.g.
    // `core/src/newdir/`) even when `file_write`/`file_edit` would refuse.
    if let crate::sandbox::Verdict::Denied(msg) = crate::sandbox::check(&path) {
        return format!("Error: {}", msg);
    }

    match tokio::fs::create_dir_all(&path).await {
        Ok(_) => format!("Created directory: {}", path.display()),
        Err(e) => format!("Error creating directory {}: {}", path.display(), e),
    }
}

/// Rename (or move) a file from `src` to `dst`.
///
/// Requires PHANTOM_AUTO_APPROVE=1 or returns APPROVAL_REQUIRED.
pub async fn rename_file(args: &Value) -> String {
    let src_raw = match args["src"].as_str() {
        Some(p) => p,
        None => return "Error: missing 'src' argument".into(),
    };
    let dst_raw = match args["dst"].as_str() {
        Some(p) => p,
        None => return "Error: missing 'dst' argument".into(),
    };

    if src_raw.contains("..") || dst_raw.contains("..") {
        return "Error: path traversal not allowed".into();
    }

    if std::env::var("PHANTOM_AUTO_APPROVE").as_deref() != Ok("1") {
        return format!(
            "APPROVAL_REQUIRED: renaming '{}' to '{}' requires explicit approval. \
             Set PHANTOM_AUTO_APPROVE=1 to allow.",
            src_raw, dst_raw
        );
    }

    let src = match safe_path(src_raw) {
        Ok(p) => p,
        Err(e) => return format!("Error: invalid src path: {}", e),
    };

    // [T7f] Workspace-boundary check on dst (PR #75 audit H-8). Pre-fix
    // `dst` was taken raw from the args, giving the agent a write-anywhere
    // primitive: `rename_file { src: "in/workspace.txt", dst: "/etc/cron.d/x" }`
    // would happily move a file into a privileged location (write-where) or
    // overwrite ~/.ssh/authorized_keys (persistence). The `..`-string guard
    // above does not stop absolute paths.
    let dst = match safe_path(dst_raw) {
        Ok(p) => p,
        Err(e) => return format!("Error: invalid dst path: {}", e),
    };

    // [C5/T74 V9 H-1] CO-EVO Phase 1 sandbox guard (SPEC-FREEZE-V1.1 §4.1-d).
    // Both source and destination must clear the sandbox: a rename out of a
    // protected prefix would silently delete the original; a rename into one
    // would create a new write-where primitive.
    if let crate::sandbox::Verdict::Denied(msg) = crate::sandbox::check(&src) {
        return format!("Error: {}", msg);
    }
    if let crate::sandbox::Verdict::Denied(msg) = crate::sandbox::check(&dst) {
        return format!("Error: {}", msg);
    }

    match tokio::fs::rename(&src, &dst).await {
        Ok(_) => format!("Renamed: {} -> {}", src.display(), dst.display()),
        Err(e) => format!(
            "Error renaming {} to {}: {}",
            src.display(),
            dst.display(),
            e
        ),
    }
}

pub async fn delete_file(args: &Value) -> String {
    let raw = match args["path"].as_str() {
        Some(p) => p,
        None => return "Error: missing 'path' argument".into(),
    };

    // Path traversal guard
    if raw.contains("..") {
        return "Error: path traversal not allowed".into();
    }

    let path = match safe_path(raw) {
        Ok(p) => p,
        Err(e) => return format!("Error: invalid path: {}", e),
    };

    // [C5/T74 V9 H-1] CO-EVO Phase 1 sandbox guard (SPEC-FREEZE-V1.1 §4.1-d).
    // Without this check, delete_file could remove sandboxed paths (e.g.
    // `core/src/serve.rs`) even when `file_write`/`file_edit` would refuse.
    if let crate::sandbox::Verdict::Denied(msg) = crate::sandbox::check(&path) {
        return format!("Error: {}", msg);
    }

    match tokio::fs::metadata(&path).await {
        Ok(m) if m.is_dir() => {
            return format!(
                "Error: {} is a directory. delete_file only removes files.",
                path.display()
            );
        }
        Ok(m) if m.len() > 10 * 1024 * 1024 => {
            return format!(
                "Error: {} is larger than 10MB ({} bytes). Refusing to delete.",
                path.display(),
                m.len()
            );
        }
        Err(e) => return format!("Error: cannot stat {}: {}", path.display(), e),
        _ => {}
    }

    if std::env::var("PHANTOM_AUTO_APPROVE").as_deref() != Ok("1") {
        return format!(
            "APPROVAL_REQUIRED: deleting '{}' is irreversible. \
             Set PHANTOM_AUTO_APPROVE=1 to allow.",
            path.display()
        );
    }

    match tokio::fs::remove_file(&path).await {
        Ok(_) => format!("Deleted: {}", path.display()),
        Err(e) => format!("Error deleting {}: {}", path.display(), e),
    }
}

#[cfg(test)]
mod sandbox_guard_tests {
    //! [C5/T74 V9 H-1] Regression tests for the sandbox guard on
    //! `create_dir`, `rename_file`, and `delete_file`. Each of these
    //! was a mutation surface that bypassed sandbox::check before this
    //! patch, even though `file_write`/`file_edit` were already guarded.
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn next_id() -> usize {
        COUNTER.fetch_add(1, Ordering::SeqCst)
    }

    async fn phantom_dir() -> PathBuf {
        let home = dirs::home_dir().expect("HOME");
        let dir = home.join(".phantom-mesh").join("test-c5-sandbox-fs");
        tokio::fs::create_dir_all(&dir).await.expect("mkdir");
        dir
    }

    async fn seed_file(initial: &str) -> PathBuf {
        let dir = phantom_dir().await;
        let path = dir.join(format!("seed-{}-{}.txt", std::process::id(), next_id()));
        tokio::fs::write(&path, initial).await.expect("seed");
        path
    }

    // ----- create_dir -----

    #[tokio::test]
    async fn sandbox_denies_create_dir_into_protected_prefix() {
        // Target an existing directory inside the protected `core/` prefix
        // (`src/` is a sibling that lives under the same canonical `\core\`
        // path). `create_dir_all` would have been a no-op on the existing
        // dir, but the sandbox guard fires first and refuses the request.
        let _g = crate::sandbox::test_lock();
        crate::sandbox::enable(true);

        let result = create_dir(&json!({ "path": "src" })).await;

        crate::sandbox::enable(false);

        assert!(
            result.contains("sandbox guard"),
            "create_dir must refuse sandboxed prefix, got: {}",
            result
        );
    }

    #[tokio::test]
    async fn sandbox_allows_create_dir_in_phantom_mesh() {
        let dir = phantom_dir().await;
        let target = dir.join(format!("create-{}-{}", std::process::id(), next_id()));

        let _g = crate::sandbox::test_lock();
        crate::sandbox::enable(true);

        let result = create_dir(&json!({ "path": target.to_string_lossy() })).await;

        crate::sandbox::enable(false);

        assert!(
            result.starts_with("Created directory:"),
            "create_dir in ~/.phantom-mesh/ should succeed under sandbox, got: {}",
            result
        );
        let _ = tokio::fs::remove_dir(&target).await;
    }

    #[tokio::test]
    async fn sandbox_disabled_back_compat_create_dir() {
        let dir = phantom_dir().await;
        let target = dir.join(format!("compat-{}-{}", std::process::id(), next_id()));

        let _g = crate::sandbox::test_lock();
        crate::sandbox::enable(false);

        let result = create_dir(&json!({ "path": target.to_string_lossy() })).await;

        assert!(result.starts_with("Created directory:"), "got: {}", result);
        let _ = tokio::fs::remove_dir(&target).await;
    }

    // ----- rename_file -----

    #[tokio::test]
    async fn sandbox_denies_rename_into_protected_prefix() {
        let src = seed_file("payload\n").await;
        let _g = crate::sandbox::test_lock();
        std::env::set_var("PHANTOM_AUTO_APPROVE", "1");
        crate::sandbox::enable(true);

        // dst targets an existing file inside the protected `core/` prefix
        // — `src/sandbox.rs` itself, relative to cargo's CWD. safe_path
        // succeeds (real file, inside cwd root); the sandbox guard then
        // refuses the rename because the dst canonicalises under `\core\`.
        let result = rename_file(&json!({
            "src": src.to_string_lossy(),
            "dst": "src/sandbox.rs",
        }))
        .await;

        crate::sandbox::enable(false);
        std::env::remove_var("PHANTOM_AUTO_APPROVE");

        assert!(
            result.contains("sandbox guard"),
            "rename_file must refuse dst inside sandbox, got: {}",
            result
        );
        // Source must still exist — refusal must be non-destructive.
        assert!(src.exists(), "source file must be untouched after refusal");
        let _ = tokio::fs::remove_file(&src).await;
    }

    #[tokio::test]
    async fn sandbox_allows_rename_within_phantom_mesh() {
        let src = seed_file("payload\n").await;
        let dst =
            phantom_dir()
                .await
                .join(format!("renamed-{}-{}.txt", std::process::id(), next_id()));
        let _g = crate::sandbox::test_lock();
        std::env::set_var("PHANTOM_AUTO_APPROVE", "1");
        crate::sandbox::enable(true);

        let result = rename_file(&json!({
            "src": src.to_string_lossy(),
            "dst": dst.to_string_lossy(),
        }))
        .await;

        crate::sandbox::enable(false);
        std::env::remove_var("PHANTOM_AUTO_APPROVE");

        assert!(
            result.starts_with("Renamed:"),
            "rename within ~/.phantom-mesh/ should succeed under sandbox, got: {}",
            result
        );
        let _ = tokio::fs::remove_file(&dst).await;
    }

    #[tokio::test]
    async fn sandbox_disabled_back_compat_rename() {
        let src = seed_file("payload\n").await;
        let dst = phantom_dir().await.join(format!(
            "compat-rename-{}-{}.txt",
            std::process::id(),
            next_id()
        ));
        let _g = crate::sandbox::test_lock();
        std::env::set_var("PHANTOM_AUTO_APPROVE", "1");
        crate::sandbox::enable(false);

        let result = rename_file(&json!({
            "src": src.to_string_lossy(),
            "dst": dst.to_string_lossy(),
        }))
        .await;

        std::env::remove_var("PHANTOM_AUTO_APPROVE");

        assert!(result.starts_with("Renamed:"), "got: {}", result);
        let _ = tokio::fs::remove_file(&dst).await;
    }

    // ----- delete_file -----

    #[tokio::test]
    async fn sandbox_denies_delete_in_protected_prefix() {
        // Aim at a real file inside the protected `core/` prefix
        // (`src/sandbox.rs` — relative to cargo's CWD `core/`). The
        // sandbox check fires BEFORE metadata(), so the on-disk file is
        // never touched.
        let _g = crate::sandbox::test_lock();
        std::env::set_var("PHANTOM_AUTO_APPROVE", "1");
        crate::sandbox::enable(true);

        let result = delete_file(&json!({ "path": "src/sandbox.rs" })).await;

        crate::sandbox::enable(false);
        std::env::remove_var("PHANTOM_AUTO_APPROVE");

        assert!(
            result.contains("sandbox guard"),
            "delete_file must refuse sandboxed path, got: {}",
            result
        );
        // Sanity: the file must still exist after the refusal.
        assert!(
            std::path::Path::new("src/sandbox.rs").exists(),
            "delete_file refusal must be non-destructive"
        );
    }

    #[tokio::test]
    async fn sandbox_allows_delete_in_phantom_mesh() {
        let target = seed_file("to be deleted\n").await;
        let _g = crate::sandbox::test_lock();
        std::env::set_var("PHANTOM_AUTO_APPROVE", "1");
        crate::sandbox::enable(true);

        let result = delete_file(&json!({ "path": target.to_string_lossy() })).await;

        crate::sandbox::enable(false);
        std::env::remove_var("PHANTOM_AUTO_APPROVE");

        assert!(
            result.starts_with("Deleted:"),
            "delete in ~/.phantom-mesh/ should succeed under sandbox, got: {}",
            result
        );
        assert!(!target.exists(), "file should be gone");
    }

    #[tokio::test]
    async fn sandbox_disabled_back_compat_delete() {
        let target = seed_file("legacy\n").await;
        let _g = crate::sandbox::test_lock();
        std::env::set_var("PHANTOM_AUTO_APPROVE", "1");
        crate::sandbox::enable(false);

        let result = delete_file(&json!({ "path": target.to_string_lossy() })).await;

        std::env::remove_var("PHANTOM_AUTO_APPROVE");

        assert!(result.starts_with("Deleted:"), "got: {}", result);
    }
}
