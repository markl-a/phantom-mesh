use serde_json::Value;
use std::path::{Path, PathBuf};

/// Parse the target file path from a `+++ b/path` line.
/// Handles `+++ b/path`, `+++ path`, and `+++ /dev/null`.
fn parse_target_path(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("+++")?;
    let rest = rest.trim();
    // `+++ /dev/null` means file is deleted — skip
    if rest == "/dev/null" {
        return None;
    }
    // Strip leading `b/` prefix produced by git diff
    let path = rest.strip_prefix("b/").unwrap_or(rest);
    Some(path)
}

/// One `@@ -old_start,old_count +new_start,new_count @@` hunk plus its lines.
#[derive(Debug)]
struct Hunk {
    /// 1-based line number in the *new* file where this hunk starts.
    new_start: usize,
    /// 1-based line number in the *old* file where this hunk starts.
    old_start: usize,
    /// Lines: `' '` context, `'-'` removed, `'+'` added.
    lines: Vec<(char, String)>,
}

/// Parse `@@ -L,N +L,N @@` header; returns `(old_start, new_start)`.
fn parse_hunk_header(header: &str) -> Option<(usize, usize)> {
    // Format: @@ -<old_start>[,<old_count>] +<new_start>[,<new_count>] @@
    let inner = header.strip_prefix("@@")?.trim();
    let inner = inner.split("@@").next()?.trim();
    let mut parts = inner.split_whitespace();
    let old_part = parts.next()?; // e.g. "-10,7"
    let new_part = parts.next()?; // e.g. "+10,8"

    let old_start_str = old_part.strip_prefix('-')?.split(',').next()?;
    let new_start_str = new_part.strip_prefix('+')?.split(',').next()?;

    let old_start: usize = old_start_str.parse().ok()?;
    let new_start: usize = new_start_str.parse().ok()?;
    Some((old_start, new_start))
}

/// All hunks for a single target file.
#[derive(Debug)]
struct FilePatch {
    path: String,
    hunks: Vec<Hunk>,
}

/// Split the full patch text into per-file patches.
fn parse_patch(patch_text: &str) -> Vec<FilePatch> {
    let mut result: Vec<FilePatch> = Vec::new();

    // Each file section starts with `--- ` (but we key on `+++ ` to get the target).
    // We scan line by line and collect sections.
    let mut lines = patch_text.lines().peekable();
    let mut current_path: Option<String> = None;
    let mut current_hunks: Vec<Hunk> = Vec::new();
    let mut current_hunk: Option<Hunk> = None;

    while let Some(line) = lines.next() {
        if line.starts_with("--- ") {
            // Next line should be `+++ `.
            if let Some(plus_line) = lines.next() {
                // Flush previous file
                if let Some(path) = current_path.take() {
                    if let Some(h) = current_hunk.take() {
                        current_hunks.push(h);
                    }
                    if !current_hunks.is_empty() {
                        result.push(FilePatch {
                            path,
                            hunks: current_hunks.drain(..).collect(),
                        });
                    }
                }

                if plus_line.starts_with("+++ ") {
                    if let Some(p) = parse_target_path(&plus_line) {
                        current_path = Some(p.to_string());
                    }
                    // If path is None (e.g. /dev/null), current_path stays None and we skip.
                }
            }
            continue;
        }

        if line.starts_with("@@ ") {
            // Flush previous hunk
            if let Some(h) = current_hunk.take() {
                current_hunks.push(h);
            }
            if let Some((old_start, new_start)) = parse_hunk_header(line) {
                current_hunk = Some(Hunk {
                    old_start,
                    new_start,
                    lines: Vec::new(),
                });
            }
            continue;
        }

        if let Some(ref mut hunk) = current_hunk {
            if line.starts_with('+') {
                hunk.lines.push(('+', line[1..].to_string()));
            } else if line.starts_with('-') {
                hunk.lines.push(('-', line[1..].to_string()));
            } else if line.starts_with(' ') {
                hunk.lines.push((' ', line[1..].to_string()));
            } else if line.is_empty() {
                // Some diffs have bare empty lines as context
                hunk.lines.push((' ', String::new()));
            }
            // Lines like `\ No newline at end of file` are ignored
        }
    }

    // Flush last file
    if let Some(path) = current_path.take() {
        if let Some(h) = current_hunk.take() {
            current_hunks.push(h);
        }
        if !current_hunks.is_empty() {
            result.push(FilePatch {
                path,
                hunks: current_hunks,
            });
        }
    }

    result
}

/// Apply a single hunk to the file lines (0-based vector).
/// Returns an error string if the hunk context doesn't match.
fn apply_hunk(file_lines: &mut Vec<String>, hunk: &Hunk) -> Result<(), String> {
    // We'll rebuild by walking the hunk lines against the file.
    // First, verify that context / removed lines match what's in the file.
    {
        let mut check_pos = hunk.old_start.saturating_sub(1);
        for (kind, expected) in &hunk.lines {
            match kind {
                ' ' | '-' => {
                    if check_pos >= file_lines.len() {
                        return Err(format!(
                            "Hunk at line {} extends beyond end of file (file has {} lines)",
                            hunk.old_start,
                            file_lines.len()
                        ));
                    }
                    if file_lines[check_pos] != *expected {
                        return Err(format!(
                            "Hunk context mismatch at file line {}:\n  expected: {:?}\n  found:    {:?}",
                            check_pos + 1,
                            expected,
                            file_lines[check_pos]
                        ));
                    }
                    check_pos += 1;
                }
                '+' => {}
                _ => {}
            }
        }
    }

    // Apply: walk old_start position, remove `-` lines, insert `+` lines.
    let mut pos = hunk.old_start.saturating_sub(1);
    let mut i = 0;
    while i < hunk.lines.len() {
        let (kind, text) = &hunk.lines[i];
        match kind {
            ' ' => {
                pos += 1;
                i += 1;
            }
            '-' => {
                file_lines.remove(pos);
                // don't advance pos; next element slides into place
                i += 1;
            }
            '+' => {
                file_lines.insert(pos, text.clone());
                pos += 1;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    Ok(())
}

/// Describe what a hunk would change (for dry_run output).
fn describe_hunk(hunk: &Hunk) -> String {
    let removed: Vec<&str> = hunk
        .lines
        .iter()
        .filter(|(k, _)| *k == '-')
        .map(|(_, l)| l.as_str())
        .collect();
    let added: Vec<&str> = hunk
        .lines
        .iter()
        .filter(|(k, _)| *k == '+')
        .map(|(_, l)| l.as_str())
        .collect();
    format!(
        "  @@ line {} — remove {} line(s), add {} line(s)",
        hunk.new_start,
        removed.len(),
        added.len()
    )
}

pub async fn apply(args: &Value) -> String {
    let patch_text = match args["patch"].as_str() {
        Some(p) => p,
        None => return "Error: missing required 'patch' argument".to_string(),
    };

    let base_dir: PathBuf = args["base_dir"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let dry_run = args["dry_run"].as_bool().unwrap_or(false);

    let file_patches = parse_patch(patch_text);

    if file_patches.is_empty() {
        return "No valid file patches found in the provided diff".to_string();
    }

    let mut results: Vec<String> = Vec::new();
    let mut total_hunks = 0usize;
    let mut modified_files: Vec<String> = Vec::new();

    for fp in &file_patches {
        // T7 fix (codex audit 2026-05-15): every patch target must pass the
        // workspace boundary check in tools::file::safe_path. Absolute paths
        // and `..`-traversal targets that escape the allowed roots are
        // rejected here.
        let raw_path: PathBuf = if Path::new(&fp.path).is_absolute() {
            PathBuf::from(&fp.path)
        } else {
            base_dir.join(&fp.path)
        };
        let raw_str = match raw_path.to_str() {
            Some(s) => s,
            None => {
                results.push(format!(
                    "Error: non-UTF-8 patch target path: {}",
                    raw_path.display()
                ));
                continue;
            }
        };
        let file_path: PathBuf = match crate::tools::file::safe_path(raw_str) {
            Ok(p) => p,
            Err(e) => {
                results.push(format!(
                    "Error: patch target rejected by workspace boundary check: {} ({})",
                    raw_path.display(),
                    e
                ));
                continue;
            }
        };

        // [C5/T74 V9 H-1] CO-EVO Phase 1 sandbox guard (SPEC-FREEZE-V1.1 §4.1-d).
        // Without this check, apply_patch could mutate sandboxed paths (e.g.
        // `core/src/*`) even when `file_write`/`file_edit` would refuse.
        if let crate::sandbox::Verdict::Denied(msg) = crate::sandbox::check(&file_path) {
            results.push(format!("Error: {}", msg));
            continue;
        }

        if dry_run {
            let hunk_descs: Vec<String> = fp.hunks.iter().map(describe_hunk).collect();
            results.push(format!(
                "[dry_run] Would patch: {}\n{}",
                file_path.display(),
                hunk_descs.join("\n")
            ));
            total_hunks += fp.hunks.len();
            modified_files.push(fp.path.clone());
            continue;
        }

        // Read file
        let content = match tokio::fs::read_to_string(&file_path).await {
            Ok(c) => c,
            Err(e) => {
                results.push(format!("Error reading {}: {}", file_path.display(), e));
                continue;
            }
        };

        let mut file_lines: Vec<String> = content.lines().map(String::from).collect();
        // Track whether the original content ended with a newline
        let trailing_newline = content.ends_with('\n');

        let mut file_ok = true;
        for hunk in &fp.hunks {
            if let Err(e) = apply_hunk(&mut file_lines, hunk) {
                results.push(format!(
                    "Error applying hunk to {}: {}",
                    file_path.display(),
                    e
                ));
                file_ok = false;
                break;
            }
        }

        if file_ok {
            let mut new_content = file_lines.join("\n");
            if trailing_newline {
                new_content.push('\n');
            }
            match tokio::fs::write(&file_path, &new_content).await {
                Ok(_) => {
                    let n = fp.hunks.len();
                    results.push(format!(
                        "Patched {} ({} hunk{})",
                        file_path.display(),
                        n,
                        if n == 1 { "" } else { "s" }
                    ));
                    total_hunks += n;
                    modified_files.push(fp.path.clone());
                }
                Err(e) => {
                    results.push(format!("Error writing {}: {}", file_path.display(), e));
                }
            }
        }
    }

    let prefix = if dry_run {
        format!(
            "[dry_run] Would apply {} hunk{} to {} file{}.",
            total_hunks,
            if total_hunks == 1 { "" } else { "s" },
            modified_files.len(),
            if modified_files.len() == 1 { "" } else { "s" }
        )
    } else {
        format!(
            "Applied {} hunk{} to {} file{}.",
            total_hunks,
            if total_hunks == 1 { "" } else { "s" },
            modified_files.len(),
            if modified_files.len() == 1 { "" } else { "s" }
        )
    };

    let detail = if !modified_files.is_empty() {
        format!(" Modified: {}", modified_files.join(", "))
    } else {
        String::new()
    };

    let per_file = results.join("\n");

    format!("{}{}\n\n{}", prefix, detail, per_file)
}

#[cfg(test)]
mod sandbox_guard_tests {
    //! [C5/T74 V9 H-1] Regression tests for the sandbox guard wired into
    //! `apply_patch` (this module's `apply` function). Without this guard,
    //! a unified diff targeting `core/src/*.rs` would be applied even when
    //! `file_write`/`file_edit` would refuse the same path.
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    async fn fresh_temp_in_phantom(initial: &str) -> PathBuf {
        let home = dirs::home_dir().expect("HOME");
        let dir = home.join(".phantom-mesh").join("test-c5-sandbox-patch");
        tokio::fs::create_dir_all(&dir).await.expect("mkdir");
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = dir.join(format!("c5-{}-{}.txt", std::process::id(), n));
        tokio::fs::write(&path, initial).await.expect("seed");
        path
    }

    fn one_line_patch(target: &str, old: &str, new: &str) -> String {
        format!(
            "--- a/{t}\n+++ b/{t}\n@@ -1,1 +1,1 @@\n-{o}\n+{n}\n",
            t = target,
            o = old,
            n = new,
        )
    }

    #[tokio::test]
    async fn sandbox_denies_apply_patch_into_protected_prefix() {
        let _g = crate::sandbox::test_lock();
        crate::sandbox::enable(true);

        let patch_text = one_line_patch("core/src/x.rs", "old line", "new line");
        let result = apply(&json!({
            "patch": patch_text,
            "base_dir": ".",
        }))
        .await;

        crate::sandbox::enable(false);

        assert!(
            result.contains("sandbox guard"),
            "apply_patch must surface sandbox refusal, got: {}",
            result
        );
    }

    #[tokio::test]
    async fn sandbox_allows_apply_patch_in_phantom_mesh_dir() {
        // Build a real file we can validly patch.
        let path = fresh_temp_in_phantom("alpha\n").await;
        let rel = path.file_name().unwrap().to_string_lossy().to_string();
        let base = path.parent().unwrap().to_path_buf();

        let _g = crate::sandbox::test_lock();
        crate::sandbox::enable(true);

        let patch_text = one_line_patch(&rel, "alpha", "beta");
        let result = apply(&json!({
            "patch": patch_text,
            "base_dir": base.to_string_lossy(),
        }))
        .await;

        crate::sandbox::enable(false);

        assert!(
            result.starts_with("Applied"),
            "apply_patch on ~/.phantom-mesh/ should succeed under sandbox, got: {}",
            result
        );
        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn sandbox_disabled_back_compat_apply_patch() {
        let path = fresh_temp_in_phantom("seed\n").await;
        let rel = path.file_name().unwrap().to_string_lossy().to_string();
        let base = path.parent().unwrap().to_path_buf();

        let _g = crate::sandbox::test_lock();
        crate::sandbox::enable(false);

        let patch_text = one_line_patch(&rel, "seed", "grown");
        let result = apply(&json!({
            "patch": patch_text,
            "base_dir": base.to_string_lossy(),
        }))
        .await;

        assert!(result.starts_with("Applied"), "got: {}", result);
        let _ = tokio::fs::remove_file(&path).await;
    }
}
