use serde_json::Value;

pub fn safe_path(raw: &str) -> anyhow::Result<std::path::PathBuf> {
    let p = std::path::PathBuf::from(raw);
    if p.exists() {
        Ok(p.canonicalize()?)
    } else {
        let parent = p.parent().unwrap_or(&p);
        let canon_parent = if parent.as_os_str().is_empty() {
            std::path::PathBuf::from(".")
        } else if parent.exists() {
            parent.canonicalize()?
        } else {
            std::path::PathBuf::from(parent)
        };
        Ok(canon_parent.join(p.file_name().unwrap_or_default()))
    }
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
        return format!(
            "[Lines {}-{} of {}]\n{}",
            start + 1,
            end,
            total,
            numbered
        );
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
        let updated = if range_offset_bytes == 0 && line_range_start.is_none() && line_range_end.is_none() {
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
        let updated = if range_offset_bytes == 0 && line_range_start.is_none() && line_range_end.is_none() {
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
