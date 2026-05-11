use serde_json::Value;
use crate::tools::file::safe_path;

fn validate_search_path(path: &str) -> Result<(), String> {
    if path.contains("..") {
        return Err("Error: path traversal not allowed".into());
    }
    for ch in [';', '|', '&', '$', '`', '>', '<'] {
        if path.contains(ch) {
            return Err(format!("Error: invalid character '{}' in path argument", ch));
        }
    }
    Ok(())
}

/// content_search — search file contents using ripgrep.
///
/// Args (JSON):
///   pattern        : String  — regex/literal to search (required)
///   path           : String  — directory or file to search (default ".")
///   context_lines  : usize   — lines of context before/after each match (default 2)
///   file_type      : String  — filter by extension without dot, e.g. "rs", "ts" (optional)
///   case_sensitive : bool    — true = case-sensitive, false = case-insensitive (default false)
///   max_results    : usize   — max matches to return (default 50)
pub async fn content(args: &Value) -> String {
    let pattern = match args["pattern"].as_str() {
        Some(p) => p,
        None => return "Error: missing 'pattern' argument".into(),
    };
    if pattern.len() > 500 {
        return "Error: search pattern too long (max 500 characters)".into();
    }

    let raw_path = args["path"].as_str().unwrap_or(".");
    if let Err(e) = validate_search_path(raw_path) {
        return e;
    }
    let search_path = match safe_path(raw_path) {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(e) => return format!("Error: invalid path: {}", e),
    };

    let context_lines = args["context_lines"].as_u64().unwrap_or(2) as usize;
    let max_results = args["max_results"].as_u64().unwrap_or(50) as usize;
    let case_sensitive = args["case_sensitive"].as_bool().unwrap_or(false);
    let file_type = args["file_type"].as_str();

    let context_str = context_lines.to_string();
    let max_count_str = max_results.to_string();

    // Build rg argument list
    let mut rg_args: Vec<&str> = vec![
        "--color=never",
        "-n",
        "--with-filename",
        "-C", &context_str,
        "--max-count", &max_count_str,
    ];

    if !case_sensitive {
        rg_args.push("-i");
    }

    // file_type flag: rg uses -t <type> where type is a named type (rs, ts, py, etc.)
    // We store it so the lifetime is valid.
    let ft_flag: Option<String> = file_type.map(|ft| ft.to_string());
    if let Some(ref ft) = ft_flag {
        rg_args.push("-t");
        rg_args.push(ft.as_str());
    }

    rg_args.push(pattern);
    rg_args.push(&search_path);

    match tokio::process::Command::new("rg")
        .args(&rg_args)
        .output()
        .await
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            if stdout.is_empty() {
                "No matches found".into()
            } else {
                truncate_results(stdout, max_results)
            }
        }
        Err(_) => {
            // rg not found — fall back to grep (no context/type filtering)
            let mut grep_args = vec!["-rn"];
            if !case_sensitive {
                grep_args.push("-i");
            }
            grep_args.push("--include=*");
            grep_args.push(pattern);
            grep_args.push(&search_path);

            match tokio::process::Command::new("grep")
                .args(&grep_args)
                .output()
                .await
            {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                    if stdout.is_empty() {
                        "No matches found".into()
                    } else {
                        truncate_results(stdout, max_results)
                    }
                }
                Err(e) => format!("Search error: {}", e),
            }
        }
    }
}

/// Truncate output to at most `max` non-separator lines (best-effort).
fn truncate_results(output: String, max: usize) -> String {
    // rg separates match blocks with "--" lines; count actual match lines
    let lines: Vec<&str> = output.lines().collect();
    let match_lines: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.trim_start().starts_with("--"))
        .map(|(i, _)| i)
        .collect();

    if match_lines.len() <= max {
        return output;
    }

    // Find the last line index we should include
    let cutoff_line = match_lines[max - 1];
    let truncated: Vec<&str> = lines[..=cutoff_line].to_vec();
    format!(
        "{}\n... (truncated at {} matches)",
        truncated.join("\n"),
        max
    )
}

/// glob_search — find files by name pattern.
///
/// Args (JSON):
///   pattern     : String       — glob pattern, e.g. "*.rs" or "src/**/*.ts" (required)
///   path        : String       — base directory (default ".")
///   exclude     : [String]     — patterns to exclude, e.g. ["target/**", "*.lock"] (optional)
///   max_results : usize        — max files to return (default 200)
pub async fn glob(args: &Value) -> String {
    let pattern = match args["pattern"].as_str() {
        Some(p) => p,
        None => return "Error: missing 'pattern' argument".into(),
    };
    if pattern.len() > 200 {
        return "Error: glob pattern too long (max 200 characters)".into();
    }

    let raw_base = args["path"].as_str().unwrap_or(".");
    if let Err(e) = validate_search_path(raw_base) {
        return e;
    }
    let base = match safe_path(raw_base) {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(e) => return format!("Error: invalid path: {}", e),
    };

    let max_results = args["max_results"].as_u64().unwrap_or(200) as usize;

    // Collect exclude patterns
    let excludes: Vec<String> = args["exclude"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Try rg --files first (respects .gitignore, fast)
    let rg_result = try_glob_with_rg(&base, pattern, &excludes, max_results).await;
    if let Some(output) = rg_result {
        return output;
    }

    // Fall back to find
    glob_with_find(&base, pattern, &excludes, max_results).await
}

async fn try_glob_with_rg(
    base: &str,
    pattern: &str,
    excludes: &[String],
    max_results: usize,
) -> Option<String> {
    let mut rg_args: Vec<String> = vec![
        "--color=never".into(),
        "--files".into(),
        "-g".into(),
        pattern.to_string(),
        // Always exclude common noise
        "--glob=!.git/**".into(),
        "--glob=!node_modules/**".into(),
        "--glob=!target/**".into(),
    ];

    for ex in excludes {
        rg_args.push(format!("--glob=!{}", ex));
    }

    rg_args.push(base.to_string());

    let out = tokio::process::Command::new("rg")
        .args(&rg_args)
        .output()
        .await
        .ok()?;

    if !out.status.success() && out.stdout.is_empty() {
        // rg may exit non-zero when no files match; that is still a valid result
        if out.stderr.is_empty()
            || String::from_utf8_lossy(&out.stderr).contains("error")
        {
            // genuine rg error (e.g. unknown flag) — fall through to find
            return None;
        }
    }

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    Some(format_glob_results(stdout, max_results))
}

async fn glob_with_find(
    base: &str,
    pattern: &str,
    excludes: &[String],
    max_results: usize,
) -> String {
    // Decompose pattern into directory prefix + filename glob
    let (find_base, name_pat) = if let Some(slash) = pattern.rfind('/') {
        let dir_part = &pattern[..slash];
        let name_part = &pattern[slash + 1..];
        let clean_dir = dir_part.trim_start_matches("**/").trim_end_matches("/**");
        if clean_dir.is_empty() || clean_dir == "**" {
            (base.to_string(), name_part.to_string())
        } else {
            (format!("{}/{}", base, clean_dir), name_part.to_string())
        }
    } else {
        (base.to_string(), pattern.to_string())
    };

    let mut find_args: Vec<String> = vec![
        find_base.clone(),
        "-name".into(),
        name_pat,
        "-not".into(), "-path".into(), "*/node_modules/*".into(),
        "-not".into(), "-path".into(), "*/.git/*".into(),
        "-not".into(), "-path".into(), "*/target/*".into(),
    ];

    // Add user-supplied excludes as -not -path patterns
    for ex in excludes {
        find_args.push("-not".into());
        find_args.push("-path".into());
        // Wrap in wildcard if not already absolute
        let pat = if ex.starts_with('*') || ex.starts_with('/') {
            ex.clone()
        } else {
            format!("*/{}", ex)
        };
        find_args.push(pat);
    }

    match tokio::process::Command::new("find")
        .args(&find_args)
        .output()
        .await
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            format_glob_results(stdout, max_results)
        }
        Err(e) => format!("Glob search error: {}", e),
    }
}

fn format_glob_results(raw: String, max_results: usize) -> String {
    if raw.trim().is_empty() {
        return "No files found".into();
    }

    let mut files: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();

    // Sort alphabetically
    files.sort_unstable();

    let total = files.len();
    let truncated = total > max_results;
    if truncated {
        files.truncate(max_results);
    }

    let mut result = files.join("\n");
    if truncated {
        result.push_str(&format!("\n... ({} more files not shown)", total - max_results));
    }
    result
}
