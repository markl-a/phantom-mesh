//! Self-introspection tool — `diag_read`.
//!
//! Lets the agent read phantom's own diagnostic state without having to
//! know the underlying paths. Powers self-debugging: when `phantom evolve`
//! or `phantom autoevolve` runs, it can call this tool to ground the
//! prompt in real recent events instead of guessing.
//!
//! Three modes:
//!   {kind: "events", limit: 30}    last N events from the in-memory ring
//!   {kind: "crashes", limit: 5}    list of recent crash logs (newest first)
//!   {kind: "summary"}              one-paragraph overview: counts + last crash

use serde_json::Value;

pub async fn read(args: &Value) -> String {
    let kind = args
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("summary");
    match kind {
        "events" => events(args.get("limit").and_then(|v| v.as_u64()).unwrap_or(30) as usize),
        "crashes" => crashes(args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize),
        "summary" => summary(),
        "last_crash" => last_crash(),
        _ => format!(
            "[diag_read error] unknown kind '{}' — try events|crashes|summary|last_crash",
            kind
        ),
    }
}

fn events(limit: usize) -> String {
    let snap = crate::diag::snapshot();
    if snap.is_empty() {
        return "[diag] no events recorded yet (process just started or diag not initialised)"
            .to_string();
    }
    let take = snap.len().saturating_sub(limit);
    let mut out = format!(
        "=== last {} events (of {} in ring) ===\n",
        snap.len() - take,
        snap.len()
    );
    for ev in &snap[take..] {
        out.push_str(&format!(
            "[{:>13} ms] {:<14} {}\n",
            ev.ts_ms, ev.kind, ev.summary
        ));
    }
    out
}

fn crashes(limit: usize) -> String {
    let dir = match crate::cli_config::phantom_data_dir() {
        Ok(d) => d.join("crashes"),
        Err(_) => return "[diag] no home dir".to_string(),
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return format!("[diag] no crashes recorded ({})", dir.display()),
    };
    let mut paths: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            e.metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| (t, e.path()))
        })
        .collect();
    if paths.is_empty() {
        return format!("[diag] no crashes recorded ({})", dir.display());
    }
    paths.sort_by_key(|p| std::cmp::Reverse(p.0));
    paths.truncate(limit);
    let mut out = format!("=== {} most recent crash log(s) ===\n", paths.len());
    for (modified, path) in &paths {
        let secs = modified
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        out.push_str(&format!(
            "  {} ({} bytes, ts={})\n",
            path.display(),
            bytes,
            secs
        ));
    }
    out.push_str("\nUse {kind:'last_crash'} to read the newest one's full content.\n");
    out
}

fn last_crash() -> String {
    let path = match crate::diag::last_crash_path() {
        Some(p) => p,
        None => return "[diag] no crashes recorded.".to_string(),
    };
    let body = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => return format!("[diag] could not read {}: {}", path.display(), e),
    };
    // Cap so a giant backtrace doesn't blow the agent's context window.
    let capped: String = body.chars().take(8_000).collect();
    if body.len() > 8_000 {
        format!(
            "=== {} (truncated to 8000 chars of {}) ===\n{}\n…[truncated]",
            path.display(),
            body.len(),
            capped
        )
    } else {
        format!("=== {} ===\n{}", path.display(), capped)
    }
}

fn summary() -> String {
    let events = crate::diag::snapshot();
    let dir = crate::cli_config::phantom_data_dir().ok().map(|d| d.join("crashes"));
    let crash_count: usize = dir
        .as_ref()
        .and_then(|d| std::fs::read_dir(d).ok())
        .map(|it| it.flatten().count())
        .unwrap_or(0);
    let last_crash = crate::diag::last_crash_path().map(|p| p.display().to_string());
    // Top kinds in ring
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for ev in &events {
        *counts.entry(ev.kind.clone()).or_insert(0) += 1;
    }
    let mut sorted: Vec<(String, usize)> = counts.into_iter().collect();
    sorted.sort_by_key(|kc| std::cmp::Reverse(kc.1));

    let mut out = String::from("=== phantom diag summary ===\n");
    out.push_str(&format!("events in ring : {}\n", events.len()));
    out.push_str(&format!("crash logs     : {}\n", crash_count));
    if let Some(p) = last_crash {
        out.push_str(&format!("last crash     : {}\n", p));
        out.push_str("                 use {kind:'last_crash'} to read it\n");
    } else {
        out.push_str("last crash     : (none)\n");
    }
    if let Some(p) = crate::diag::events_path() {
        out.push_str(&format!("events log     : {}\n", p.display()));
    }
    out.push_str("\nevent kinds in ring (top 5):\n");
    for (kind, n) in sorted.iter().take(5) {
        out.push_str(&format!("  {:>4}× {}\n", n, kind));
    }
    out
}
