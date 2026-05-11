use std::path::PathBuf;
use serde_json::Value;
use crate::tools::file::safe_path;

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

    // dst parent must exist
    let dst = PathBuf::from(dst_raw);

    match tokio::fs::rename(&src, &dst).await {
        Ok(_) => format!("Renamed: {} -> {}", src.display(), dst.display()),
        Err(e) => format!("Error renaming {} to {}: {}", src.display(), dst.display(), e),
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
