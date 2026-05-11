use serde_json::Value;

use super::file::safe_path;

#[derive(Clone)]
struct EditSpec {
    path: String,
    old_string: String,
    new_string: String,
}

pub async fn execute(args: &Value) -> String {
    let edits_val = match args["edits"].as_array() {
        Some(a) => a,
        None => return "Error: missing or invalid 'edits' array".into(),
    };

    let dry_run = args["dry_run"].as_bool().unwrap_or(false);

    // Parse edit specs, skipping entries with empty path.
    let mut specs: Vec<EditSpec> = Vec::new();
    for item in edits_val {
        let path = match item["path"].as_str() {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => continue, // skip empty path
        };
        let old_string = item["old_string"].as_str().unwrap_or("").to_string();
        let new_string = item["new_string"].as_str().unwrap_or("").to_string();
        specs.push(EditSpec { path, old_string, new_string });
    }

    if specs.is_empty() {
        return "No edits to apply (all paths were empty or edits array was empty).".into();
    }

    // Phase 1: Validate ALL edits, collect file contents.
    struct Validated {
        spec: EditSpec,
        resolved_path: std::path::PathBuf,
        content: String,
    }

    let mut validated: Vec<Validated> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for spec in specs {
        let resolved = match safe_path(&spec.path) {
            Ok(p) => p,
            Err(e) => {
                errors.push(format!("ERROR: {}: invalid path: {}", spec.path, e));
                continue;
            }
        };

        if !resolved.exists() {
            errors.push(format!("ERROR: {} not found", spec.path));
            continue;
        }

        let content = match tokio::fs::read_to_string(&resolved).await {
            Ok(c) => c,
            Err(e) => {
                errors.push(format!("ERROR: {}: could not read file: {}", spec.path, e));
                continue;
            }
        };

        let count = content.matches(spec.old_string.as_str()).count();
        if count == 0 {
            errors.push(format!("ERROR: {}: old_string not found", spec.path));
        } else if count > 1 {
            errors.push(format!(
                "ERROR: {}: old_string matches {} times (must match exactly once)",
                spec.path, count
            ));
        } else {
            validated.push(Validated { spec, resolved_path: resolved, content });
        }
    }

    // Phase 2: If any validation failed, return all errors without applying anything.
    if !errors.is_empty() {
        return format!(
            "Validation failed — no changes were made:\n{}",
            errors.join("\n")
        );
    }

    // Phase 3: dry_run or apply.
    let mut lines: Vec<String> = Vec::new();

    if dry_run {
        lines.push(format!("Dry run — {} edit(s) would be applied:", validated.len()));
        for v in &validated {
            let old_preview: String = v.spec.old_string.chars().take(40).collect();
            let new_preview: String = v.spec.new_string.chars().take(40).collect();
            lines.push(format!(
                "  {}: '{}' → '{}'",
                v.spec.path,
                truncate_str(&old_preview, 40),
                truncate_str(&new_preview, 40),
            ));
        }
        return lines.join("\n");
    }

    // Apply all edits. When multiple edits target the same file, the
    // previous loop wrote each edit against the ORIGINAL content captured
    // during validation, so the last write overwrote earlier edits — only
    // the final edit per file actually persisted. Group edits by path,
    // apply them sequentially in memory against the in-progress buffer,
    // then write each file once at the end.
    use std::collections::BTreeMap;
    let total = validated.len();
    let mut by_path: BTreeMap<std::path::PathBuf, (String, Vec<EditSpec>)> = BTreeMap::new();
    for v in validated {
        by_path
            .entry(v.resolved_path.clone())
            .and_modify(|(_, specs)| specs.push(v.spec.clone()))
            .or_insert_with(|| (v.content.clone(), vec![v.spec.clone()]));
    }

    for (path, (mut buffer, specs)) in by_path {
        let mut path_lines: Vec<String> = Vec::with_capacity(specs.len());
        let mut all_ok = true;
        for spec in &specs {
            // Each successive edit operates on the running buffer, not the
            // pristine original. replacen with n=1 still applies on each
            // pass because validation guaranteed a unique match in the
            // ORIGINAL content; if a later edit happens to also be unique
            // in the modified buffer, replacen still does the right thing.
            // If a prior edit removed the only match for a later edit's
            // old_string, we surface that as an apply-time error.
            if !buffer.contains(&spec.old_string) {
                path_lines.push(format!(
                    "  {} ERROR: '{}' no longer present after prior edits",
                    spec.path,
                    truncate_str(&spec.old_string, 40),
                ));
                all_ok = false;
                continue;
            }
            buffer = buffer.replacen(&spec.old_string, &spec.new_string, 1);
            path_lines.push(format!(
                "  {}: replaced '{}' → '{}'",
                spec.path,
                truncate_str(&spec.old_string, 40),
                truncate_str(&spec.new_string, 40),
            ));
        }

        if all_ok {
            if let Err(e) = tokio::fs::write(&path, &buffer).await {
                path_lines.push(format!("  {} ERROR writing: {}", path.display(), e));
            }
        }
        // If any edit on this file errored, leave the file untouched (skip
        // the write) — keeps the per-file edit set atomic.
        for line in path_lines { lines.push(line); }
    }

    format!("Applied {} edit(s):\n{}", total, lines.join("\n"))
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        format!("{}...", chars[..max_chars].iter().collect::<String>())
    }
}
