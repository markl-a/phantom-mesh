//! macOS Spotlight (`mdfind`) wrapper as a tool.
//!
//! Spotlight maintains a system-wide content index, so query latency is
//! typically <100ms even across hundreds of thousands of files — orders of
//! magnitude faster than `glob_search` (ripgrep across cwd) for whole-system
//! questions like "find files modified today" or "find every Swift source
//! containing AppDelegate".
//!
//! The tool is gated `#[cfg(target_os = "macos")]` and not exposed at all on
//! other platforms.

use serde_json::Value;
use tokio::process::Command;

/// Run a Spotlight query.
///
/// Args (JSON):
/// - `query` (required): the live mdfind query, OR a plain substring. If it
///   contains an `=` we assume it's already a Spotlight query expression
///   (e.g. `kMDItemContentType == "public.swift-source"`); otherwise we wrap
///   it in `kMDItemDisplayName == "*<query>*"c` (case-insensitive name).
/// - `scope` (optional): directory to limit to (`-onlyin <scope>`).
/// - `changed_within_hours` (optional, integer): adds an InRange clause
///   restricting results to items whose content-change date is within the
///   given window.
/// - `max_results` (optional, default 50): hard cap on lines returned.
///
/// Returns the matching paths as a newline-separated list, or an error
/// string if `mdfind` cannot be invoked.
pub async fn search(args: &Value) -> String {
    let query = match args.get("query").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return "[spotlight_search] missing required 'query' argument".to_string(),
    };

    let scope = args.get("scope").and_then(|v| v.as_str()).map(String::from);
    let changed_within_hours = args
        .get("changed_within_hours")
        .and_then(|v| v.as_u64());
    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .min(500) as usize;

    // Build the Spotlight expression.
    let base_expr = if query.contains('=') || query.contains("kMDItem") {
        // user supplied a real Spotlight expression
        query.clone()
    } else {
        // wrap as case-insensitive display-name match
        format!("kMDItemDisplayName == \"*{}*\"c", escape_spotlight(&query))
    };

    let expr = if let Some(hours) = changed_within_hours {
        // $time.now(-N) — Spotlight understands time tokens with negative
        // offset in seconds. Use 3600 * hours.
        let secs = (hours as i64) * 3600;
        format!(
            "({}) && kMDItemFSContentChangeDate >= $time.now(-{})",
            base_expr, secs
        )
    } else {
        base_expr
    };

    let mut cmd = Command::new("mdfind");
    cmd.arg(&expr);
    if let Some(s) = &scope {
        cmd.arg("-onlyin").arg(s);
    }

    let out = match cmd.output().await {
        Ok(o) => o,
        Err(e) => return format!("[spotlight_search] could not run mdfind: {}", e),
    };

    if !out.status.success() {
        return format!(
            "[spotlight_search] mdfind failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut lines: Vec<&str> = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .collect();

    let total = lines.len();
    let truncated = total > max_results;
    if truncated {
        lines.truncate(max_results);
    }

    if lines.is_empty() {
        return format!(
            "[spotlight_search] no results for: {}\n(hint: ensure the directory is indexed by Spotlight; try `mdutil -s <path>`)",
            expr
        );
    }

    let mut body = lines.join("\n");
    body.push('\n');
    body.push_str(&format!(
        "\n[spotlight_search] {} match{} returned{}",
        lines.len(),
        if lines.len() == 1 { "" } else { "es" },
        if truncated {
            format!(" (truncated from {})", total)
        } else {
            String::new()
        }
    ));
    body
}

fn escape_spotlight(s: &str) -> String {
    s.replace('"', "\\\"")
}
