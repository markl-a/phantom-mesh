use serde_json::Value;
use std::path::PathBuf;

/// T7 fix (codex audit 2026-05-15): canonicalise the requested path
/// and confine it to a known-good roots set. Before this fix `safe_path`
/// would happily return `/etc/passwd` because canonicalisation alone
/// doesn't bound the result.
///
/// Allowed roots (recomputed every call so tests that `set_current_dir`
/// or flip env vars are observed):
///   - process CWD at call time (canonicalised)
///   - `$HOME/.phantom-mesh/` — phantom's own state dir
///   - any path listed in `PHANTOM_EXTRA_ALLOWED_ROOTS` (split on `:`
///     on Unix, `;` on Windows) — for test scaffolds + advanced users
///     who really need broader scope
///
/// Behaviour:
///   - existing path → canonicalised, then must live inside an allowed root
///   - non-existent path → its closest existing ancestor must canonicalise
///     inside an allowed root (so `file_write` can still create new files)
///   - relative path → resolved against CWD before the boundary check
///   - `..` segments are resolved by canonicalisation; if the result
///     escapes every allowed root, `Err` is returned with a clear hint
pub fn safe_path(raw: &str) -> anyhow::Result<PathBuf> {
    let p = PathBuf::from(raw);
    let candidate = if p.exists() {
        p.canonicalize()?
    } else {
        // Walk up to the closest existing ancestor, canonicalise it,
        // then rebuild the path by appending the missing tail. This
        // handles both `dir_that_exists/new_file.txt` (parent exists)
        // and `new/nested/path/leaf.txt` (only an ancestor exists).
        let mut tail: Vec<std::ffi::OsString> = Vec::new();
        let mut cursor: PathBuf = p.clone();
        loop {
            if cursor.as_os_str().is_empty() {
                // Pure relative path — anchor on CWD.
                let cwd = std::env::current_dir()?;
                let mut acc = cwd;
                while let Some(seg) = tail.pop() {
                    acc.push(seg);
                }
                break acc;
            }
            if cursor.exists() {
                let mut acc = cursor.canonicalize()?;
                while let Some(seg) = tail.pop() {
                    acc.push(seg);
                }
                break acc;
            }
            match cursor.file_name() {
                Some(name) => tail.push(name.to_os_string()),
                None => {
                    // No file_name (e.g. ends in `..` or `/`). Bail.
                    return Err(anyhow::anyhow!(
                        "cannot resolve path: {} (no existing ancestor)",
                        p.display()
                    ));
                }
            }
            if !cursor.pop() {
                // Already at root and still no existing ancestor → bail.
                return Err(anyhow::anyhow!(
                    "cannot resolve path: {} (no existing ancestor)",
                    p.display()
                ));
            }
        }
    };

    let roots = allowed_roots();
    if roots.iter().any(|r| candidate.starts_with(r)) {
        return Ok(candidate);
    }
    Err(anyhow::anyhow!(
        "path outside workspace: {} (allowed roots: {})",
        candidate.display(),
        roots
            .iter()
            .map(|r| r.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// Compute the allowed-roots set fresh on every call. Cheap — at most
/// 3 paths + a few canonicalises. Doing it per-call keeps tests that
/// flip env vars / cwd correct without `OnceCell` invalidation pain.
fn allowed_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(c) = cwd.canonicalize() {
            roots.push(c);
        } else {
            roots.push(cwd);
        }
    }
    if let Ok(phantom_dir) = crate::cli_config::phantom_data_dir() {
        if let Ok(c) = phantom_dir.canonicalize() {
            roots.push(c);
        } else {
            roots.push(phantom_dir);
        }
    }
    if let Ok(extra) = std::env::var("PHANTOM_EXTRA_ALLOWED_ROOTS") {
        let sep = if cfg!(windows) { ';' } else { ':' };
        for piece in extra.split(sep) {
            if piece.is_empty() {
                continue;
            }
            let pb = PathBuf::from(piece);
            if let Ok(c) = pb.canonicalize() {
                roots.push(c);
            } else {
                roots.push(pb);
            }
        }
    }
    roots
}

/// Detect binary content by checking for null bytes in the first 512 bytes.
fn is_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(512)].contains(&0u8)
}

/// Build a mini unified diff with `context` lines around the replaced region.
fn mini_diff(old_content: &str, old_str: &str, new_str: &str, context: usize) -> String {
    // Find the byte position of the first occurrence.
    let pos = match old_content.find(old_str) {
        Some(p) => p,
        None => return String::new(),
    };

    // Count lines before the match.
    let before = &old_content[..pos];
    let first_changed_line = before.lines().count(); // 0-based index of first changed line

    let old_lines: Vec<&str> = old_content.lines().collect();
    let old_match_lines: Vec<&str> = old_str.lines().collect();
    let new_match_lines: Vec<&str> = new_str.lines().collect();

    let change_end_line = first_changed_line + old_match_lines.len(); // exclusive

    let ctx_start = first_changed_line.saturating_sub(context);
    let ctx_end = (change_end_line + context).min(old_lines.len());

    let mut diff = String::new();
    // Context before
    for i in ctx_start..first_changed_line {
        diff.push_str(&format!("  {}\n", old_lines[i]));
    }
    // Removed lines
    for l in &old_match_lines {
        diff.push_str(&format!("- {}\n", l));
    }
    // Added lines
    for l in &new_match_lines {
        diff.push_str(&format!("+ {}\n", l));
    }
    // Context after
    for i in change_end_line..ctx_end {
        diff.push_str(&format!("  {}\n", old_lines[i]));
    }
    diff
}

// ---------------------------------------------------------------------------
// file_read
// ---------------------------------------------------------------------------

pub async fn read(args: &Value) -> String {
    let raw = match args["path"].as_str() {
        Some(p) => p,
        None => return "Error: missing 'path' argument".into(),
    };
    let path = match safe_path(raw) {
        Ok(p) => p,
        Err(e) => return format!("Error: invalid path: {}", e),
    };

    // Read raw bytes first so we can do binary detection.
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(e) => return format!("Error reading {}: {}", path.display(), e),
    };

    if is_binary(&bytes) {
        return format!("[binary file, {} bytes]", bytes.len());
    }

    let content = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return format!("[binary file, cannot decode as UTF-8]"),
    };

    // New params: offset (1-based start line) and limit (max lines).
    let offset = args["offset"].as_u64().map(|n| n as usize);
    let limit = args["limit"].as_u64().map(|n| n as usize);

    // Legacy params still supported.
    let start_line = args["start_line"].as_u64().map(|n| n as usize);
    let end_line = args["end_line"].as_u64().map(|n| n as usize);
    let show_line_numbers = args["show_line_numbers"].as_bool().unwrap_or(false);

    // Prefer new offset/limit params; fall back to legacy start_line/end_line.
    if offset.is_some() || limit.is_some() {
        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        // offset is 1-based; convert to 0-based index.
        let start = offset.unwrap_or(1).saturating_sub(1).min(total);
        let end = if let Some(lim) = limit {
            (start + lim).min(total)
        } else {
            total
        };
        let slice = &lines[start..end];
        let numbered: String = slice
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{}: {}", start + i + 1, l))
            .collect::<Vec<_>>()
            .join("\n");
        return format!("[Lines {}-{} of {}]\n{}", start + 1, end, total, numbered);
    }

    if start_line.is_some() || end_line.is_some() {
        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        let start = start_line.unwrap_or(1).saturating_sub(1);
        let end = end_line.unwrap_or(total).min(total);
        let slice = &lines[start..end];
        let numbered: String = slice
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{}: {}", start + i + 1, l))
            .collect::<Vec<_>>()
            .join("\n");
        return format!(
            "Lines {}-{} of {}:\n{}",
            start + 1,
            end,
            path.display(),
            numbered
        );
    }

    if show_line_numbers {
        let numbered: String = content
            .lines()
            .enumerate()
            .map(|(i, l)| format!("    {}: {}", i + 1, l))
            .collect::<Vec<_>>()
            .join("\n");
        return numbered;
    }

    crate::tools::truncate(content, 100_000)
}

// ---------------------------------------------------------------------------
// file_write
// ---------------------------------------------------------------------------

pub async fn write(args: &Value) -> String {
    let raw = match args["path"].as_str() {
        Some(p) => p,
        None => return "Error: missing 'path' argument".into(),
    };
    let path = match safe_path(raw) {
        Ok(p) => p,
        Err(e) => return format!("Error: invalid path: {}", e),
    };
    // CO-EVO Phase 1 sandbox guard (SPEC-FREEZE-V1.1 §4.1-d).
    if let crate::sandbox::Verdict::Denied(msg) = crate::sandbox::check(&path) {
        return format!("Error: {}", msg);
    }
    let content = args["content"].as_str().unwrap_or("");
    // create_dirs defaults to true.
    let create_dirs = args["create_dirs"].as_bool().unwrap_or(true);

    if create_dirs {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    return format!(
                        "Error: could not create directories for {}: {}",
                        parent.display(),
                        e
                    );
                }
            }
        }
    } else {
        // Ensure parent exists when create_dirs is false.
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                return format!(
                    "Error: parent directory {} does not exist (set create_dirs: true to create it)",
                    parent.display()
                );
            }
        }
    }

    let bytes = content.len();
    match tokio::fs::write(&path, content).await {
        Ok(_) => format!("Written {} bytes to {}", bytes, path.display()),
        Err(e) => format!("Error writing {}: {}", path.display(), e),
    }
}

// ---------------------------------------------------------------------------
// file_edit
// ---------------------------------------------------------------------------

pub async fn edit(args: &Value) -> String {
    let raw = match args["path"].as_str() {
        Some(p) => p,
        None => return "Error: missing 'path' argument".into(),
    };
    let path = match safe_path(raw) {
        Ok(p) => p,
        Err(e) => return format!("Error: invalid path: {}", e),
    };
    // CO-EVO Phase 1 sandbox guard (SPEC-FREEZE-V1.1 §4.1-d).
    if let crate::sandbox::Verdict::Denied(msg) = crate::sandbox::check(&path) {
        return format!("Error: {}", msg);
    }
    let old = match args["old_string"].as_str() {
        Some(s) => s,
        None => return "Error: missing 'old_string' argument".into(),
    };
    let new = args["new_string"].as_str().unwrap_or("");
    let replace_all = args["replace_all"].as_bool().unwrap_or(false);

    // Optional line_range to scope the search.
    let line_range_start = args["line_range"]["start"].as_u64().map(|n| n as usize);
    let line_range_end = args["line_range"]["end"].as_u64().map(|n| n as usize);

    let full_content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(e) => return format!("Error reading {}: {}", path.display(), e),
    };

    // If line_range is specified, restrict search to those lines (1-based, inclusive).
    let (search_content, range_offset_bytes): (&str, usize) =
        if line_range_start.is_some() || line_range_end.is_some() {
            let lines: Vec<&str> = full_content.lines().collect();
            let total = lines.len();
            let start = line_range_start.unwrap_or(1).saturating_sub(1).min(total);
            let end = line_range_end.unwrap_or(total).min(total);

            // Compute byte offset of the start line in the original string.
            let byte_start: usize = full_content
                .lines()
                .take(start)
                .map(|l| l.len() + 1) // +1 for '\n'
                .sum();

            let scoped: String = lines[start..end].join("\n");
            // We need to work on a slice of the original bytes, but `scoped` is a new
            // allocation. We return it as a &str via a leak trick — safer to just work
            // on the owned string. We'll use a different approach: build indices.
            // Store offset for reassembly.
            drop(scoped); // not used directly
                          // Return the slice of the original &str between those byte positions.
            let byte_end: usize = full_content
                .lines()
                .take(end)
                .map(|l| l.len() + 1)
                .sum::<usize>()
                .saturating_sub(if end < total { 0 } else { 1 }); // strip trailing newline for last line
            let byte_end = byte_end.min(full_content.len());
            (&full_content[byte_start..byte_end], byte_start)
        } else {
            (&full_content[..], 0)
        };

    let count = search_content.matches(old).count();

    if count == 0 {
        // Show first 200 chars of the search scope so the agent can see what's there.
        let preview: String = search_content.chars().take(200).collect();
        return format!(
            "Error: old_string not found in {} (searched {} chars{}). File begins with:\n{}",
            path.display(),
            search_content.len(),
            if line_range_start.is_some() || line_range_end.is_some() {
                format!(
                    " within line_range {}–{}",
                    line_range_start.unwrap_or(1),
                    line_range_end.unwrap_or_else(|| full_content.lines().count())
                )
            } else {
                String::new()
            },
            preview
        );
    }

    if replace_all {
        let updated =
            if range_offset_bytes == 0 && line_range_start.is_none() && line_range_end.is_none() {
                full_content.replace(old, new)
            } else {
                // Replace within the scoped region, reassemble.
                let replaced_scope = search_content.replace(old, new);
                format!(
                    "{}{}{}",
                    &full_content[..range_offset_bytes],
                    replaced_scope,
                    &full_content[range_offset_bytes + search_content.len()..]
                )
            };
        match tokio::fs::write(&path, &updated).await {
            Ok(_) => format!(
                "Edited {} ({} occurrence(s) replaced).",
                path.display(),
                count
            ),
            Err(e) => format!("Error writing {}: {}", path.display(), e),
        }
    } else {
        if count > 1 {
            // List line numbers of each occurrence (count '\n' chars before each match).
            let mut occurrence_lines: Vec<usize> = Vec::new();
            for m in search_content.match_indices(old) {
                let newlines_before = search_content[..m.0].chars().filter(|&c| c == '\n').count();
                occurrence_lines.push(line_range_start.unwrap_or(1) + newlines_before);
            }
            return format!(
                "Error: old_string appears {} times in {} at lines: {}. \
                 Provide more context or use replace_all:true to replace all.",
                count,
                path.display(),
                occurrence_lines
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }

        // Exactly one match — do the replacement.
        let updated =
            if range_offset_bytes == 0 && line_range_start.is_none() && line_range_end.is_none() {
                full_content.replacen(old, new, 1)
            } else {
                let replaced_scope = search_content.replacen(old, new, 1);
                format!(
                    "{}{}{}",
                    &full_content[..range_offset_bytes],
                    replaced_scope,
                    &full_content[range_offset_bytes + search_content.len()..]
                )
            };

        match tokio::fs::write(&path, &updated).await {
            Ok(_) => {
                let diff = mini_diff(&full_content, old, new, 2);
                format!("Edited {} successfully.\n\nDiff:\n{}", path.display(), diff)
            }
            Err(e) => format!("Error writing {}: {}", path.display(), e),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests — safe_path allowed-roots boundary
// ---------------------------------------------------------------------------
//
// `safe_path` is the allowed-roots confinement that backs file_read /
// file_write / file_edit (T7 fix, codex audit 2026-05-15). It must:
//   (a) accept writes INSIDE an allowed root (incl. not-yet-existing files),
//   (b) reject writes OUTSIDE every allowed root,
//   (c) reject `..` traversal that escapes the root (canonicalisation
//       collapses `..`, so the resolved path must still be bounded).
//
// `allowed_roots()` reads `PHANTOM_EXTRA_ALLOWED_ROOTS` fresh on every call,
// so the tests register a tempdir as an extra root and exercise paths
// relative to it. The env var is process-global, so we serialise via the
// shared `crate::sandbox::test_lock()` (already used by other tool tests).
#[cfg(test)]
mod safe_path_tests {
    use super::safe_path;

    /// RAII guard that sets PHANTOM_EXTRA_ALLOWED_ROOTS to `dir` for the
    /// duration of a test and restores the previous value (or removes it)
    /// on drop. Holds the process-global sandbox test lock so concurrent
    /// tests don't clobber each other's env.
    struct RootGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        prev: Option<String>,
    }
    impl RootGuard {
        fn new(dir: &std::path::Path) -> Self {
            let lock = crate::sandbox::test_lock();
            let prev = std::env::var("PHANTOM_EXTRA_ALLOWED_ROOTS").ok();
            std::env::set_var("PHANTOM_EXTRA_ALLOWED_ROOTS", dir);
            RootGuard { _lock: lock, prev }
        }
    }
    impl Drop for RootGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("PHANTOM_EXTRA_ALLOWED_ROOTS", v),
                None => std::env::remove_var("PHANTOM_EXTRA_ALLOWED_ROOTS"),
            }
        }
    }

    #[test]
    fn write_inside_allowed_root_is_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        // Canonicalise so the comparison matches what safe_path computes
        // (macOS /var → /private/var symlink etc.).
        let root = tmp.path().canonicalize().unwrap();
        let _g = RootGuard::new(&root);

        // New (not-yet-existing) file whose parent IS the allowed root.
        let target = root.join("new_file.txt");
        let resolved = safe_path(target.to_str().unwrap())
            .expect("path inside the allowed root must be accepted");
        assert!(
            resolved.starts_with(&root),
            "resolved {} should be under root {}",
            resolved.display(),
            root.display()
        );

        // Nested-not-yet-existing path (only an ancestor exists) is also fine.
        let nested = root.join("a/b/c/leaf.txt");
        let resolved = safe_path(nested.to_str().unwrap())
            .expect("nested new path inside the root must be accepted");
        assert!(resolved.starts_with(&root), "got: {}", resolved.display());
    }

    #[test]
    fn write_outside_allowed_roots_is_rejected() {
        // Two sibling tempdirs: one allowed, one NOT registered as a root.
        let allowed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let allowed_root = allowed.path().canonicalize().unwrap();
        let outside_root = outside.path().canonicalize().unwrap();
        let _g = RootGuard::new(&allowed_root);

        let target = outside_root.join("escape.txt");
        let err = safe_path(target.to_str().unwrap())
            .expect_err("path outside every allowed root must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("outside workspace"),
            "expected an outside-workspace error, got: {}",
            msg
        );
    }

    #[test]
    fn dotdot_traversal_escaping_root_is_rejected() {
        // Allowed root is a SUBDIR of the tempdir; `..` from inside it
        // climbs back out of the allowed root and must be rejected even
        // though the literal string starts with the root prefix.
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let allowed_root = base.join("allowed");
        std::fs::create_dir(&allowed_root).unwrap();
        // A sibling target that exists outside the allowed root.
        let secret_dir = base.join("secret");
        std::fs::create_dir(&secret_dir).unwrap();
        std::fs::write(secret_dir.join("loot.txt"), b"x").unwrap();

        let _g = RootGuard::new(&allowed_root);

        // `allowed/../secret/loot.txt` canonicalises to base/secret/loot.txt,
        // which is OUTSIDE allowed_root → must be rejected.
        let traversal = allowed_root.join("../secret/loot.txt");
        let err = safe_path(traversal.to_str().unwrap())
            .expect_err("`..` escaping the allowed root must be rejected");
        assert!(
            err.to_string().contains("outside workspace"),
            "got: {}",
            err
        );

        // Sanity: a `..` that stays inside the root is still fine.
        std::fs::create_dir(allowed_root.join("sub")).unwrap();
        let inside = allowed_root.join("sub/../kept.txt");
        let resolved = safe_path(inside.to_str().unwrap())
            .expect("`..` that stays inside the root must be accepted");
        assert!(
            resolved.starts_with(&allowed_root),
            "got: {}",
            resolved.display()
        );
    }
}
