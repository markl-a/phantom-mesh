//! User-facing CLI subcommands for managing API keys and provider priority.
//!
//! Two surfaces:
//!
//!   `phantom keys ...`        — manage `~/.phantom-mesh/env` (the file phantom
//!                               auto-loads at startup) so the user never has
//!                               to touch PowerShell `[Environment]::SetEnvironmentVariable`
//!                               or shell rcfile exports by hand.
//!
//!   `phantom providers ...`   — view configured providers and edit
//!                               `[agent.<name>].providers = [...]` priority
//!                               in `~/.phantom-mesh/agents.toml`.
//!
//! Both subcommands are read-write to user-owned files only — no service
//! restart, no network. Edits use `toml_edit` to preserve comments and
//! formatting in the user's hand-tuned config.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config::AgentsConfig;

// ── Paths ────────────────────────────────────────────────────────────────

/// `~/.phantom-mesh/env` — line-oriented `KEY=value` file phantom auto-loads
/// at process start. Sourced by shell idiom `set -a; source <file>; set +a`
/// for parity with operator runbooks.
pub fn env_file_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".phantom-mesh").join("env"))
}

/// `~/.phantom-mesh/agents.toml` — primary phantom config.
pub fn agents_toml_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".phantom-mesh").join("agents.toml"))
}

/// `~/.phantom-mesh/runtime-override` — single-line file containing the
/// effective provider:model override for ALL phantom processes (TUI,
/// serve daemon, repl, mcp). Set by `/model X:Y` in the TUI; read by
/// every dispatch path in agent.rs and streaming.rs at call time.
///
/// Without this file the override is per-process via PHANTOM_RUNTIME_OVERRIDE
/// env var, which doesn't reach a separately-spawned `phantom serve`.
pub fn runtime_override_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".phantom-mesh").join("runtime-override"))
}

/// Read the cluster-wide override file. None when missing or empty.
/// Trimmed of whitespace. Cheap to call on every dispatch — small file
/// (typically <50 bytes), no JSON parse.
pub fn read_runtime_override() -> Option<String> {
    let path = runtime_override_path()?;
    let s = fs::read_to_string(path).ok()?;
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// Write or clear the cluster-wide override file. Atomic via .tmp + rename.
pub fn write_runtime_override(value: Option<&str>) -> std::io::Result<()> {
    let path = runtime_override_path()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no $HOME"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match value {
        Some(v) if !v.trim().is_empty() => {
            let tmp = path.with_extension("override.tmp");
            fs::write(&tmp, v.trim().as_bytes())?;
            fs::rename(&tmp, &path)?;
        }
        _ => {
            // Clear by removing the file.
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

// ── Env file (KEY=value lines) ───────────────────────────────────────────

/// Parse `~/.phantom-mesh/env` into a name→value map, ignoring blanks and
/// `#`-comment lines. Quoted values keep their inner content; unquoted are
/// taken verbatim (no shell-style expansion). Resilient to malformed lines:
/// lines without `=` are skipped silently.
pub fn read_env_file(path: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(content) = fs::read_to_string(path) else { return out; };
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        let Some((k, v)) = line.split_once('=') else { continue; };
        let k = k.trim().to_string();
        let v = v.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
        if !k.is_empty() {
            out.insert(k, v);
        }
    }
    out
}

/// Write a name→value map back to `path`, sorted by key for deterministic
/// diffs. The file's parent directory is created if missing.
pub fn write_env_file(path: &Path, vars: &HashMap<String, String>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut keys: Vec<&String> = vars.keys().collect();
    keys.sort();
    let mut f = fs::File::create(path)?;
    writeln!(f, "# phantom-mesh env file — auto-loaded at process start.")?;
    writeln!(f, "# Edit via `phantom keys set/remove`, or hand-edit (KEY=value lines).")?;
    writeln!(f)?;
    for k in keys {
        let v = &vars[k];
        // Quote values containing whitespace or special chars; keep simple ones bare.
        if v.chars().any(|c| c.is_whitespace() || c == '"' || c == '\'') {
            writeln!(f, "{}={}", k, format_args!("\"{}\"", v.replace('"', "\\\"")))?;
        } else {
            writeln!(f, "{}={}", k, v)?;
        }
    }
    Ok(())
}

/// Source `~/.phantom-mesh/env` into the current process's environment.
/// Idempotent: callable multiple times. Existing env vars are NOT overwritten
/// — explicit shell-set vars always win. Returns the count of new vars set.
/// Called from `phantom`'s `main()` before any subcommand dispatch.
pub fn auto_load_env() -> usize {
    let Some(path) = env_file_path() else { return 0; };
    let vars = read_env_file(&path);
    let mut set = 0;
    for (k, v) in vars {
        if std::env::var(&k).is_err() {
            std::env::set_var(&k, v);
            set += 1;
        }
    }
    set
}

// ── Mask helpers ─────────────────────────────────────────────────────────

/// "sk-wrdrOfA..." → "sk-wrd…" so the user sees enough to identify a key
/// without leaking the secret part. The trailing ellipsis is intentionally
/// preserved for short values too (e.g. `"abc" → "abc…"`) so the masked
/// form has a consistent visual marker — the test suite pins this.
pub fn mask_key(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::from("(empty)");
    }
    let prefix_chars = trimmed.chars().take(6).collect::<String>();
    format!("{}…", prefix_chars)
}

/// Lowercase provider name → conventional env var name.
/// `groq` → `GROQ_API_KEY`, `nvidia` → `NVIDIA_NIM_API_KEY` (special),
/// `gemini` → `GEMINI_API_KEY`, etc.
pub fn provider_env_var_name(provider: &str) -> String {
    match provider.to_lowercase().as_str() {
        "nvidia" | "nvidia_nim" | "nim" => "NVIDIA_NIM_API_KEY".into(),
        other => format!("{}_API_KEY", other.to_uppercase().replace('-', "_")),
    }
}

// ── `phantom debug` subcommand ───────────────────────────────────────────
//
// One-command diagnostic bundle. The user runs `phantom debug` after any
// failure on any machine, pastes the output into chat or a bug report,
// and the responder has everything needed to root-cause without a 20-
// message back-and-forth asking for `phantom doctor` then `phantom keys
// list` then `cat agents.toml` etc.
//
// Output sections (in order):
//   1. Header   — phantom version, OS, build hash, current time
//   2. doctor   — env + service + provider sanity check (existing)
//   3. keys     — masked LLM API key inventory (existing, masked already)
//   4. providers — agents.toml [providers.*] + per-agent priority lists
//   5. agents.toml — full file with api_key / cluster_secret redacted
//   6. events  — last 30 lines of events.jsonl, ISO-formatted
//   7. crashes — most recent crash file (if any) head 100 lines
//
// Pipe to clipboard:    phantom debug | Set-Clipboard
// Save to file:         phantom debug > debug.txt
// Email me directly:    phantom debug | iwr ... (future, --upload flag)

pub async fn run_debug(args: &[String]) -> anyhow::Result<()> {
    let mut tail_events: usize = 30;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--tail" | "-n" => {
                i += 1;
                tail_events = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(30);
            }
            "--help" | "-h" => {
                eprintln!("phantom debug — collect everything needed to root-cause an error");
                eprintln!();
                eprintln!("  phantom debug                  default bundle (last 30 events)");
                eprintln!("  phantom debug --tail 200       include more event history");
                eprintln!();
                eprintln!("Pipe to clipboard:  phantom debug | Set-Clipboard");
                eprintln!("Save to file:       phantom debug > debug.txt");
                eprintln!();
                eprintln!("Output is auto-redacted: api_key= and cluster_secret= values");
                eprintln!("are replaced with [REDACTED]. Safe to paste into chat / issues.");
                return Ok(());
            }
            other => anyhow::bail!("unknown flag {} for `phantom debug`", other),
        }
        i += 1;
    }

    println!("=== phantom debug bundle ===");
    println!("generated: {}", iso_ms(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64).unwrap_or(0)));
    println!();

    // 1. version + OS
    println!("## version + os");
    println!("  phantom: {}", env!("CARGO_PKG_VERSION"));
    println!("  os:      {} {}", std::env::consts::OS, std::env::consts::ARCH);
    println!();

    // 2. keys (masked already by mask_key)
    println!("## keys (~/.phantom-mesh/env, masked)");
    match keys_list_lines() {
        Ok(lines) => for l in lines { println!("  {}", l); }
        Err(e) => println!("  (error: {})", e),
    }
    println!();

    // 3. providers
    println!("## providers (~/.phantom-mesh/agents.toml [providers.*])");
    match providers_list_lines() {
        Ok(lines) => for l in lines { println!("  {}", l); }
        Err(e) => println!("  (error: {})", e),
    }
    println!();

    // 4. agents.toml content with secrets redacted
    println!("## agents.toml (redacted)");
    if let Some(path) = agents_toml_path() {
        match fs::read_to_string(&path) {
            Ok(content) => {
                println!("  path: {}", path.display());
                println!("  ---");
                for line in redact_secrets(&content).lines() {
                    println!("  {}", line);
                }
                println!("  ---");
            }
            Err(e) => println!("  (read error: {})", e),
        }
    } else {
        println!("  (no $HOME)");
    }
    println!();

    // 5. broker config (also redacted)
    println!("## broker config (~/.phantom-mesh/broker.json)");
    if let Some(cfg) = read_broker_config() {
        println!("  url:   {}", cfg.url);
        println!("  token: {} ({} chars)", mask_key(&cfg.token), cfg.token.len());
    } else {
        println!("  (none — never ran `phantom config pull` / `phantom login`)");
    }
    println!();

    // 6. events tail
    println!("## last {} events (~/.phantom-mesh/events.jsonl)", tail_events);
    if let Some(path) = events_log_path() {
        match fs::read_to_string(&path) {
            Ok(text) => {
                let lines: Vec<&str> = text.lines().collect();
                let total = lines.len();
                let start = total.saturating_sub(tail_events);
                println!("  total: {} events, showing last {}", total, total - start);
                for line in &lines[start..] {
                    let v: serde_json::Value = match serde_json::from_str(line) {
                        Ok(v) => v,
                        Err(_) => { println!("  (malformed) {}", line); continue; }
                    };
                    let ts = v.get("ts_ms").and_then(|t| t.as_i64()).unwrap_or(0);
                    let kind = v.get("kind").and_then(|x| x.as_str()).unwrap_or("?");
                    let summary = v.get("summary").and_then(|x| x.as_str()).unwrap_or("");
                    println!("  [{}] {:>16}  {}", iso_ms(ts), kind, summary);
                }
            }
            Err(e) => println!("  (read error: {})", e),
        }
    }
    println!();

    // 7. most recent crash
    println!("## most recent crash (~/.phantom-mesh/crashes/, head 100 lines)");
    if let Some(home) = dirs::home_dir() {
        let crashes_dir = home.join(".phantom-mesh").join("crashes");
        let recent = fs::read_dir(&crashes_dir).ok().and_then(|rd| {
            let mut entries: Vec<(std::time::SystemTime, std::path::PathBuf)> = rd
                .filter_map(|e| e.ok())
                .filter_map(|e| e.metadata().ok().and_then(|m| m.modified().ok()).map(|t| (t, e.path())))
                .collect();
            entries.sort_by(|a, b| b.0.cmp(&a.0));
            entries.into_iter().next().map(|(_, p)| p)
        });
        match recent {
            Some(path) => {
                println!("  path: {}", path.display());
                println!("  ---");
                if let Ok(text) = fs::read_to_string(&path) {
                    for (i, line) in text.lines().take(100).enumerate() {
                        println!("  {:>4}: {}", i + 1, line);
                    }
                }
                println!("  ---");
            }
            None => println!("  (no crash files — clean history)"),
        }
    }
    println!();
    println!("=== end debug bundle ===");
    Ok(())
}

/// Replace `api_key = "..."`, `cluster_secret = "..."`, etc. with the
/// keyword + `= [REDACTED]` so an agents.toml dump is safe to paste into
/// chat. Conservative: only matches lines that have one of the known
/// secret keys; any other content passes through untouched.
fn redact_secrets(s: &str) -> String {
    let secret_keys = [
        "api_key", "cluster_secret", "password", "secret",
        "broker_jwt_secret", "google_client_secret", "apple_p8_private_key",
    ];
    s.lines().map(|line| {
        let trimmed = line.trim_start();
        let lc = trimmed.to_lowercase();
        for key in &secret_keys {
            if lc.starts_with(key) {
                let leading = &line[..line.len() - trimmed.len()];
                if let Some(eq_pos) = trimmed.find('=') {
                    let key_part = &trimmed[..eq_pos];
                    return format!("{}{}= \"[REDACTED]\"", leading, key_part);
                }
            }
        }
        line.to_string()
    }).collect::<Vec<_>>().join("\n")
}

// ── `phantom logs` subcommand ────────────────────────────────────────────
//
// Surface:
//   phantom logs                       — last 50 events
//   phantom logs --tail N              — last N events
//   phantom logs --since 5m            — events from the last 5 minutes
//                                        (units: s|m|h|d, e.g. 30s 2m 1h 1d)
//   phantom logs --kind error          — filter by event kind substring
//   phantom logs --raw                 — emit raw JSON lines (no formatting)
//
// Source: ~/.phantom-mesh/events.jsonl. One JSON object per line:
//   { "ts_ms": 1777..., "kind": "...", "summary": "..." }
// crashes/ panic dumps and the cluster.db / costs.db SQLite stores are
// not in this view (they have their own tooling — `phantom doctor`, etc.).

pub fn events_log_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".phantom-mesh").join("events.jsonl"))
}

pub fn run_logs(args: &[String]) -> anyhow::Result<()> {
    let mut tail: usize = 50;
    let mut since_ms: Option<i64> = None;
    let mut kind_filter: Option<String> = None;
    let mut raw = false;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--tail" | "-n" => {
                i += 1;
                tail = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(50);
            }
            "--since" | "-s" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    since_ms = parse_duration_to_ms(v);
                    if since_ms.is_none() {
                        anyhow::bail!("bad --since value '{}' — expected like 30s 5m 1h 1d", v);
                    }
                }
            }
            "--kind" | "-k" => {
                i += 1;
                kind_filter = args.get(i).cloned();
            }
            "--raw" => { raw = true; }
            "--help" | "-h" => { logs_help(); return Ok(()); }
            other => anyhow::bail!("unknown flag {} for `phantom logs` (try `phantom logs help`)", other),
        }
        i += 1;
    }

    let path = events_log_path().ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let raw_text = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("(no events log yet at {} — {})", path.display(), e);
            return Ok(());
        }
    };

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let cutoff_ms = since_ms.map(|d| now_ms - d);

    // Walk lines newest-first by collecting then reversing — events.jsonl
    // is append-only so the file order IS chronological. Most-recent at
    // the end. Matches `tail -n` shape.
    let mut all_lines: Vec<&str> = raw_text.lines().collect();
    let total = all_lines.len();
    if cutoff_ms.is_none() && kind_filter.is_none() {
        // Fast path: no filtering, just slice the last N.
        if all_lines.len() > tail { all_lines = all_lines[all_lines.len() - tail..].to_vec(); }
    } else {
        // Filter then take last N.
        let filtered: Vec<&str> = all_lines.into_iter().filter(|line| {
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v, Err(_) => return false,
            };
            if let Some(cut) = cutoff_ms {
                let ts = v.get("ts_ms").and_then(|t| t.as_i64()).unwrap_or(0);
                if ts < cut { return false; }
            }
            if let Some(ref k) = kind_filter {
                let kind = v.get("kind").and_then(|x| x.as_str()).unwrap_or("");
                if !kind.contains(k.as_str()) { return false; }
            }
            true
        }).collect();
        all_lines = if filtered.len() > tail {
            filtered[filtered.len() - tail..].to_vec()
        } else { filtered };
    }

    if all_lines.is_empty() {
        eprintln!("(no events match — file has {} total)", total);
        return Ok(());
    }

    eprintln!("# {} events ({} total in {}; first→last):", all_lines.len(), total, path.display());
    for line in &all_lines {
        if raw {
            println!("{}", line);
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => { println!("(malformed) {}", line); continue; }
        };
        let ts = v.get("ts_ms").and_then(|t| t.as_i64()).unwrap_or(0);
        let kind = v.get("kind").and_then(|x| x.as_str()).unwrap_or("?");
        let summary = v.get("summary").and_then(|x| x.as_str()).unwrap_or("");
        println!("[{}] {:>14}  {}", iso_ms(ts), kind, summary);
    }
    Ok(())
}

fn logs_help() {
    eprintln!("phantom logs — recent events from ~/.phantom-mesh/events.jsonl");
    eprintln!();
    eprintln!("  phantom logs                  last 50 events");
    eprintln!("  phantom logs --tail 200       last 200");
    eprintln!("  phantom logs --since 5m       events from last 5 min (s|m|h|d)");
    eprintln!("  phantom logs --kind error     only kinds containing 'error'");
    eprintln!("  phantom logs --raw            emit JSON lines (machine-readable)");
    eprintln!();
    eprintln!("Useful when something fails on a remote box — paste the output");
    eprintln!("into chat / a bug report. Crash dumps live separately in");
    eprintln!("~/.phantom-mesh/crashes/ — see `phantom doctor` for that.");
}

/// "5m" → 300_000 ms, "30s" → 30_000, "2h" → 7_200_000, "1d" → 86_400_000.
/// Returns None on unparseable input so callers can bail with a help line.
fn parse_duration_to_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() { return None; }
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: i64 = num.parse().ok()?;
    let mult = match unit {
        "s" => 1_000,
        "m" => 60 * 1_000,
        "h" => 60 * 60 * 1_000,
        "d" => 24 * 60 * 60 * 1_000,
        _ => return None,
    };
    Some(n * mult)
}

/// "2026-05-03T07:30:42Z"-style timestamp from epoch millis. No external
/// `chrono` dep — manual UTC math is enough for log line prefixes.
fn iso_ms(ts_ms: i64) -> String {
    if ts_ms <= 0 { return "----------------".into(); }
    let secs = ts_ms / 1000;
    let ms = (ts_ms % 1000) as u32;
    // days since 1970-01-01
    let days = secs / 86_400;
    let sec_of_day = secs % 86_400;
    let hh = (sec_of_day / 3600) as u32;
    let mm = ((sec_of_day % 3600) / 60) as u32;
    let ss = (sec_of_day % 60) as u32;
    let (y, mo, d) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z", y, mo, d, hh, mm, ss, ms)
}

/// Convert "days since 1970-01-01" into (y, m, d). Algorithm from
/// Howard Hinnant's date library; correct for any date in [1, 65535].
fn days_to_ymd(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z / 146097 } else { (z - 146096) / 146097 };
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe/1460 + doe/36524 - doe/146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365*yoe + yoe/4 - yoe/100);
    let mp = (5*doy + 2) / 153;
    let d = (doy - (153*mp + 2)/5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = (y + (if m <= 2 { 1 } else { 0 })) as i32;
    (y, m, d)
}

// ── `phantom dispatch` subcommand ────────────────────────────────────────
//
// Capability-based RPC dispatcher. Beats hand-typing
//   phantom rpc assign --target http://100.107.205.98:7878 --agent master "..."
// by looking up peers from peers.json + filtering on capability tags.
//
// Surface:
//   phantom dispatch "task"                  → any peer (round-robin pick)
//   phantom dispatch --tag rust "build"      → peers with "rust" capability
//   phantom dispatch --tag rust --tag gpu .. → peers with rust AND gpu
//   phantom dispatch --to ayaneo "..."       → specific peer by name
//   phantom dispatch --agent coder "..."     → run as that agent on remote
//   phantom dispatch --async ...             → don't wait, print job_id
//
// Auth: HMAC-SHA256 over body with cluster_secret from local agents.toml
// (same as `phantom rpc assign` already does internally — we reuse the
// signing logic).

pub async fn run_dispatch(args: &[String]) -> anyhow::Result<()> {
    let mut tags:   Vec<String> = Vec::new();
    let mut target_name: Option<String> = None;
    let mut agent:  String = "master".to_string();
    let mut async_mode = false;
    let mut all = false;
    let mut prompt_parts: Vec<String> = Vec::new();
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--tag" | "-t"   => { i += 1; if let Some(v) = args.get(i) { tags.push(v.to_lowercase()); } }
            "--to"           => { i += 1; if let Some(v) = args.get(i) { target_name = Some(v.clone()); } }
            "--agent" | "-a" => { i += 1; if let Some(v) = args.get(i) { agent = v.clone(); } }
            "--async"        => { async_mode = true; }
            "--all"          => { all = true; }
            "--help" | "-h"  => {
                eprintln!("phantom dispatch — capability-routed cross-node RPC");
                eprintln!();
                eprintln!("  phantom dispatch \"task description\"        any peer");
                eprintln!("  phantom dispatch --tag rust \"cargo build\"  peers with 'rust' cap");
                eprintln!("  phantom dispatch --tag rust --tag gpu ..    intersect of all tags");
                eprintln!("  phantom dispatch --to ayaneo \"...\"          specific peer by name");
                eprintln!("  phantom dispatch --all \"cargo test\"         broadcast: every peer in parallel");
                eprintln!("  phantom dispatch --agent coder \"...\"        agent on remote (default master)");
                eprintln!("  phantom dispatch --async \"...\"              return job_id, don't wait");
                eprintln!();
                eprintln!("Peers + capabilities come from ~/.phantom-mesh/peers.json");
                eprintln!("(populated by `phantom config pull`). Edit caps via");
                eprintln!("https://phantommesh.io/account → Cluster peers.");
                return Ok(());
            }
            other if !other.starts_with("--") => { prompt_parts.push(other.to_string()); }
            other => anyhow::bail!("unknown flag {} for `phantom dispatch`", other),
        }
        i += 1;
    }
    let prompt = prompt_parts.join(" ");
    if prompt.trim().is_empty() {
        anyhow::bail!("no prompt — usage: phantom dispatch [--tag X] [--to Y] [--all] [--agent Z] \"task description\"");
    }

    // Broadcast path: fan-out to every peer in peers.json (skipping self),
    // run them concurrently, group results under "─ <peer> ─" headers.
    if all {
        if target_name.is_some() || !tags.is_empty() {
            anyhow::bail!("--all is mutually exclusive with --to / --tag (broadcast hits everyone)");
        }
        let me = resolve_self_node_name();
        let peers: Vec<ClusterPeer> = read_peers_json().unwrap_or_default()
            .into_iter()
            .filter(|p| Some(p.name.as_str()) != me.as_deref())
            .collect();
        if peers.is_empty() {
            anyhow::bail!("no peers found in peers.json — run `phantom config pull` first");
        }
        eprintln!("◆ fanout → {} peer(s): {}",
            peers.len(),
            peers.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "));
        let probes = peers.iter().map(|p| {
            let name = p.name.clone();
            let agent = agent.clone();
            let prompt = prompt.clone();
            async move {
                let lines = dispatch_lines(&[], Some(&name), &agent, &prompt, async_mode).await;
                (name, lines)
            }
        });
        let results = futures::future::join_all(probes).await;
        for (name, res) in results {
            eprintln!();
            eprintln!("─ {} ─", name);
            match res {
                Ok(lines) => for l in lines { eprintln!("{}", l); }
                Err(e)    => eprintln!("✗ dispatch failed: {}", e),
            }
        }
        return Ok(());
    }

    for line in dispatch_lines(&tags, target_name.as_deref(), &agent, &prompt, async_mode).await? {
        eprintln!("{}", line);
    }
    Ok(())
}

pub async fn dispatch_lines(
    tags: &[String],
    target_name: Option<&str>,
    agent: &str,
    prompt: &str,
    async_mode: bool,
) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();

    // 1. Pick the target peer.
    let peers = read_peers_json().unwrap_or_else(|| {
        // Fallback to hardcoded topology if vault peers haven't been
        // synced yet. Each entry has empty capabilities, so --tag
        // filters won't match — but --to and "any peer" still work.
        CLUSTER_TOPOLOGY.iter().map(|(n, u)| ClusterPeer {
            name: n.to_string(), url: u.to_string(),
            label: None, capabilities: Vec::new(),
        }).collect()
    });

    let target = if let Some(name) = target_name {
        peers.into_iter().find(|p| p.name == name)
            .ok_or_else(|| anyhow::anyhow!(
                "no peer named '{}' in peers.json — try `phantom cluster sync` first",
                name
            ))?
    } else {
        // Filter by all required capabilities (intersect).
        let candidates: Vec<ClusterPeer> = peers.into_iter()
            .filter(|p| tags.iter().all(|t| p.capabilities.iter().any(|c| c.eq_ignore_ascii_case(t))))
            .collect();
        if candidates.is_empty() {
            if tags.is_empty() {
                anyhow::bail!("no peers in peers.json — run `phantom config pull` or `phantom cluster join <name>`");
            } else {
                anyhow::bail!(
                    "no peers match tags {:?} — check capabilities at https://phantommesh.io/account",
                    tags
                );
            }
        }
        // Random pick — simple "round-robin" via SystemTime nanos as
        // entropy. Avoids needing rand crate or persistent counter.
        let idx = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as usize).unwrap_or(0) % candidates.len();
        candidates[idx].clone()
    };

    out.push(format!("◆ dispatching to '{}' ({})", target.name, target.url));
    if !target.capabilities.is_empty() {
        out.push(format!("  capabilities: {:?}", target.capabilities));
    }

    // 2. Read cluster_secret from local agents.toml for HMAC signing.
    let cfg_path = agents_toml_path()
        .ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let raw = fs::read_to_string(&cfg_path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", cfg_path.display(), e))?;
    let cfg: AgentsConfig = toml::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("parse {}: {}", cfg_path.display(), e))?;
    let secret = cfg.cluster.cluster_secret.clone()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!(
            "cluster_secret missing in [cluster] block of {} — run `phantom cluster join <name>` first",
            cfg_path.display()
        ))?;

    // 3. POST /rpc/task/assign with HMAC-SHA256 header.
    let body = serde_json::json!({"agent": agent, "prompt": prompt});
    let body_str = serde_json::to_string(&body)?;
    let auth = hmac_sha256_hex(&secret, body_str.as_bytes());
    let url = format!("{}/rpc/task/assign", target.url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let resp = client.post(&url)
        .header("X-Cluster-Auth", &auth)
        .header("Content-Type", "application/json")
        .body(body_str)
        .send().await
        .map_err(|e| anyhow::anyhow!("POST {}: {}", url, e))?;
    let status = resp.status();
    let resp_body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("RPC assign failed: HTTP {} — {}",
            status.as_u16(),
            resp_body.chars().take(200).collect::<String>());
    }
    let parsed: serde_json::Value = serde_json::from_str(&resp_body)
        .map_err(|e| anyhow::anyhow!("non-JSON response: {} — {}", e, resp_body))?;
    let job_id = parsed.get("job_id").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("response missing job_id: {}", resp_body))?
        .to_string();
    out.push(format!("◆ job assigned: {}", job_id));

    if async_mode {
        out.push(format!("  (async mode — poll later: phantom dispatch-status {} --to {})",
            job_id, target.name));
        return Ok(out);
    }

    // 4. Poll status until done. 60s budget, 1s interval.
    out.push("◆ waiting for result …".into());
    let status_url = format!("{}/rpc/task/status/{}", target.url.trim_end_matches('/'), job_id);
    let empty_auth = hmac_sha256_hex(&secret, b"");
    let started = std::time::Instant::now();
    let mut last_status = String::new();
    loop {
        if started.elapsed() > std::time::Duration::from_secs(60) {
            out.push(format!("⚠ timeout after 60s. last status: {}. Continue polling: phantom dispatch-status {}",
                last_status, job_id));
            break;
        }
        let r = client.get(&status_url)
            .header("X-Cluster-Auth", &empty_auth)
            .send().await
            .map_err(|e| anyhow::anyhow!("status poll: {}", e))?;
        let body = r.text().await.unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
        last_status = v.get("status").and_then(|s| s.as_str()).unwrap_or("?").to_string();
        match last_status.as_str() {
            "done" => {
                let output = v.get("output").and_then(|o| o.as_str()).unwrap_or("(empty)");
                out.push(format!("✓ result ({}ms): {}", started.elapsed().as_millis(), output));
                break;
            }
            "error" => {
                let err = v.get("error").and_then(|e| e.as_str()).unwrap_or("(no detail)");
                out.push(format!("✗ remote error: {}", err));
                break;
            }
            "running" | "pending" | "queued" | "?" => {
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            }
            other => {
                out.push(format!("⚠ unknown status '{}': {}", other, body.chars().take(200).collect::<String>()));
                break;
            }
        }
    }

    Ok(out)
}

/// `phantom git ...` — cluster git operations.
///
/// Today: only `phantom git sync [--all|--to <name>] [--cwd <path>]
/// [--branch <name>]` — fan-out git pull across peers, report each
/// peer's resulting HEAD commit. Skips self by default.
///
/// Per-peer cwd resolution (server-side admin_shell endpoint):
///   1. --cwd flag if provided
///   2. else this peer's own [workspace].default_dir from agents.toml
///   3. else serve's own working dir at startup
pub async fn run_git(args: &[String]) -> anyhow::Result<()> {
    let sub = args.get(2).map(|s| s.as_str()).unwrap_or("help");
    match sub {
        "sync" => run_git_sync(args).await,
        "help" | "--help" | "-h" => {
            eprintln!("phantom git — cluster git operations");
            eprintln!();
            eprintln!("  phantom git sync --all              git pull on every peer's pinned project");
            eprintln!("  phantom git sync --to ayaneo        git pull on one peer");
            eprintln!("  phantom git sync --all --branch X   checkout branch X then pull");
            eprintln!("  phantom git sync --all --cwd D:/path   override the cwd on each peer");
            eprintln!();
            eprintln!("Each peer runs `git pull` (and optional `git checkout`) in its own");
            eprintln!("[workspace].default_dir from agents.toml (set via `phantom workspace set`).");
            eprintln!("Use --cwd to override that for this run.");
            Ok(())
        }
        other => anyhow::bail!("unknown `phantom git` subcommand: {} — try `phantom git help`", other),
    }
}

async fn run_git_sync(args: &[String]) -> anyhow::Result<()> {
    let mut all = false;
    let mut to: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--all"            => { all = true; }
            "--to"             => { i += 1; to     = args.get(i).cloned(); }
            "--branch" | "-b"  => { i += 1; branch = args.get(i).cloned(); }
            "--cwd"            => { i += 1; cwd    = args.get(i).cloned(); }
            other => anyhow::bail!("unknown flag {} for `phantom git sync`", other),
        }
        i += 1;
    }
    if !all && to.is_none() {
        anyhow::bail!("usage: phantom git sync (--all | --to <name>) [--branch X] [--cwd P]");
    }

    // Build the shell command — single line so it's atomic at the remote.
    // git rev-parse HEAD at the end gives the user a clear "this is what
    // you're on now" line per peer.
    let cmd = match &branch {
        Some(b) => format!("git fetch && git checkout {} && git pull && git rev-parse --short HEAD", b),
        None    => "git pull && git rev-parse --short HEAD".to_string(),
    };

    // Pick targets.
    let me = resolve_self_node_name();
    let mut peers: Vec<ClusterPeer> = read_peers_json().unwrap_or_default();
    if let Some(name) = &to {
        peers.retain(|p| &p.name == name);
        if peers.is_empty() {
            anyhow::bail!("no peer named '{}' in peers.json", name);
        }
    } else {
        peers.retain(|p| Some(p.name.as_str()) != me.as_deref());
    }
    if peers.is_empty() {
        anyhow::bail!("no peers — run `phantom config pull` first");
    }

    // Read cluster_secret for HMAC.
    let cfg_path = agents_toml_path().ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let raw = fs::read_to_string(&cfg_path)?;
    let cfg: AgentsConfig = toml::from_str(&raw)?;
    let secret = cfg.cluster.cluster_secret.clone()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("cluster_secret missing"))?;

    eprintln!("◆ git sync → {} peer(s): {}",
        peers.len(),
        peers.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "));
    if let Some(b) = &branch { eprintln!("  branch: {}", b); }
    if let Some(c) = &cwd    { eprintln!("  cwd:    {}", c); }
    eprintln!();

    let probes = peers.iter().map(|peer| {
        let peer = peer.clone();
        let secret = secret.clone();
        let cmd = cmd.clone();
        let cwd = cwd.clone();
        async move {
            let result = admin_shell(&peer.url, &secret, &cmd, cwd.as_deref()).await;
            (peer.name, result)
        }
    });
    let results = futures::future::join_all(probes).await;

    let mut ok_count = 0;
    for (name, res) in results {
        match res {
            Ok(out) => {
                let status = if out.exit_code == 0 { ok_count += 1; "✓" } else { "✗" };
                eprintln!("─ {} ─ exit={}", name, out.exit_code);
                if !out.stdout.trim().is_empty() {
                    eprintln!("  {} {}", status, out.stdout.lines().last().unwrap_or(""));
                    let preview = out.stdout.trim();
                    if preview.lines().count() > 1 {
                        for l in preview.lines().take(5) {
                            eprintln!("    {}", l);
                        }
                    }
                }
                if !out.stderr.trim().is_empty() {
                    for l in out.stderr.trim().lines().take(3) {
                        eprintln!("    ⚠ {}", l);
                    }
                }
            }
            Err(e) => eprintln!("─ {} ─ ✗ {}", name, e),
        }
    }
    eprintln!();
    eprintln!("◆ summary: {}/{} peers synced", ok_count, peers.len());
    Ok(())
}

/// Result of an /rpc/admin/shell call.
#[derive(Debug)]
pub struct AdminShellResult {
    pub exit_code: i64,
    pub stdout:    String,
    pub stderr:    String,
}

/// POST /rpc/admin/shell to a peer with HMAC auth, return parsed result.
/// Used by `phantom git sync` and any future fan-out admin task.
pub async fn admin_shell(
    peer_url: &str,
    secret:   &str,
    cmd:      &str,
    cwd:      Option<&str>,
) -> anyhow::Result<AdminShellResult> {
    let body = serde_json::json!({
        "cmd": cmd,
        "cwd": cwd.unwrap_or(""),
        "timeout_secs": 120,
    }).to_string();
    let auth = hmac_sha256_hex(secret, body.as_bytes());
    let url = format!("{}/rpc/admin/shell", peer_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(150))
        .build()?;
    let resp = client.post(&url)
        .header("X-Cluster-Auth", &auth)
        .header("Content-Type", "application/json")
        .body(body)
        .send().await
        .map_err(|e| anyhow::anyhow!("POST {}: {}", url, e))?;
    let status = resp.status();
    let txt = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("HTTP {}: {}", status.as_u16(),
            txt.chars().take(200).collect::<String>());
    }
    let v: serde_json::Value = serde_json::from_str(&txt)
        .map_err(|e| anyhow::anyhow!("non-JSON: {} — {}", e, txt))?;
    Ok(AdminShellResult {
        exit_code: v.get("exit_code").and_then(|x| x.as_i64()).unwrap_or(-1),
        stdout:    v.get("stdout").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        stderr:    v.get("stderr").and_then(|x| x.as_str()).unwrap_or("").to_string(),
    })
}

/// HMAC-SHA256 over `body` with key `secret`, hex-encoded. Same shape
/// as `openssl dgst -sha256 -hmac` produces; matches what the broker's
/// /rpc/* handlers verify on the receiving side.
fn hmac_sha256_hex(secret: &str, body: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(body);
    let bytes = mac.finalize().into_bytes();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ── `phantom sessions` + TUI heartbeat ───────────────────────────────────
//
// Live presence across the user's machines. The TUI POSTs to
// /api/me/sessions/heartbeat on launch and every 30s; on graceful exit
// it DELETEs its row. Other machines list peers via `phantom sessions`
// (which calls GET /api/me/sessions and renders the result).
//
// Session id is a uuid generated once per TUI process — survives the
// 60s stale-window even if heartbeats briefly fail (network blip).
//
// All calls are best-effort: if there's no broker_token in auth.json,
// the heartbeat task no-ops silently. We don't want to break the TUI
// for a presence feature.

#[derive(Clone)]
pub struct SessionHandle {
    pub id: String,
    /// Cancel-signal: drop or send () to ask the heartbeat task to stop
    /// and (best-effort) DELETE its session row before returning.
    pub stop: tokio::sync::mpsc::Sender<()>,
}

/// Spawn a background heartbeat task. Returns a handle the TUI can drop
/// on shutdown to trigger graceful DELETE. If no broker config is
/// available, returns None (caller treats as "no presence; no-op").
///
/// agent + cwd are captured at spawn time. The TUI doesn't currently
/// notify us when the user switches agent mid-session, so the displayed
/// agent will be the one that was active at TUI launch — refining this
/// later means making this a Arc<Mutex<...>> and updating from the TUI
/// state machine on agent switch.
pub fn start_session_heartbeat(agent: String, cwd: String) -> Option<SessionHandle> {
    let auth = crate::auth::load()?;
    if auth.broker_token.is_empty() || auth.broker_url.is_empty() {
        return None;
    }
    let machine = resolve_self_node_name().unwrap_or_else(|| "unknown".into());
    let id = format!("{:x}-{:x}", rand::random::<u64>(), rand::random::<u64>());
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);

    let beat_id   = id.clone();
    let beat_url  = auth.broker_url.trim_end_matches('/').to_string();
    let beat_tok  = auth.broker_token.clone();
    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build() { Ok(c) => c, Err(_) => return };
        let beat_url_post   = format!("{}/api/me/sessions/heartbeat", beat_url);
        let beat_url_delete = format!("{}/api/me/sessions/{}", beat_url, beat_id);
        let body = serde_json::json!({
            "id": beat_id, "machine": machine, "agent": agent, "cwd": cwd,
        });
        // Initial beat fires immediately; subsequent every 30s until
        // the stop signal lands. Errors are swallowed — if the broker
        // is unreachable the row will just go stale (60s window).
        loop {
            let _ = client.post(&beat_url_post)
                .header("Authorization", format!("Bearer {}", beat_tok))
                .header("Content-Type", "application/json")
                .body(body.to_string())
                .send().await;
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {},
                _ = rx.recv() => {
                    // Best-effort delete on graceful shutdown.
                    let _ = client.delete(&beat_url_delete)
                        .header("Authorization", format!("Bearer {}", beat_tok))
                        .send().await;
                    return;
                }
            }
        }
    });

    Some(SessionHandle { id, stop: tx })
}

/// Fetch + format active TUI sessions from the broker. Returns lines
/// suitable for either CLI stdout (run_sessions) or in-TUI transcript
/// (`/cluster who`). Caller decides where to write them.
pub async fn sessions_lines() -> anyhow::Result<Vec<String>> {
    let auth = crate::auth::load()
        .ok_or_else(|| anyhow::anyhow!("not logged in — run `phantom login` first"))?;
    if auth.broker_token.is_empty() || auth.broker_url.is_empty() {
        anyhow::bail!("no broker token — run `phantom login` to refresh");
    }
    let url = format!("{}/api/me/sessions", auth.broker_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let resp = client.get(&url)
        .header("Authorization", format!("Bearer {}", auth.broker_token))
        .send().await
        .map_err(|e| anyhow::anyhow!("GET {}: {}", url, e))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("HTTP {}: {}", status.as_u16(),
            body.chars().take(200).collect::<String>());
    }
    let parsed: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("non-JSON response: {} — {}", e, body))?;
    let sessions = parsed.get("sessions").and_then(|v| v.as_array())
        .cloned().unwrap_or_default();
    let mut out = Vec::new();
    if sessions.is_empty() {
        out.push("no active sessions in the last 60s".into());
        out.push("(open a phantom TUI on any logged-in machine to register one)".into());
        return Ok(out);
    }
    let now_s = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)) as i64;
    let me = resolve_self_node_name().unwrap_or_default();
    out.push(format!("◆ {} active session{}:",
        sessions.len(), if sessions.len() == 1 { "" } else { "s" }));
    for s in sessions {
        let machine    = s.get("machine").and_then(|v| v.as_str()).unwrap_or("?");
        let agent      = s.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
        let cwd        = s.get("cwd").and_then(|v| v.as_str()).unwrap_or("");
        let started_at = s.get("started_at").and_then(|v| v.as_i64()).unwrap_or(0) / 1000;
        let last_seen  = s.get("last_seen_at").and_then(|v| v.as_i64()).unwrap_or(0) / 1000;
        let alive_for  = (now_s - started_at).max(0);
        let last_ago   = (now_s - last_seen).max(0);
        let here = if machine.eq_ignore_ascii_case(&me) { "  ← this machine" } else { "" };
        let cwd_disp = if cwd.is_empty() { "—".to_string() } else {
            cwd.replace('\\', "/")
        };
        out.push(format!("  {:14} {:8} · {} · alive {} · seen {}s ago{}",
            machine, agent, cwd_disp,
            human_duration(alive_for),
            last_ago, here));
    }
    Ok(out)
}

/// `phantom sessions` — print live TUI sessions across the user's mesh.
pub async fn run_sessions(_args: &[String]) -> anyhow::Result<()> {
    for line in sessions_lines().await? {
        println!("{}", line);
    }
    Ok(())
}

fn human_duration(secs: i64) -> String {
    if secs < 60       { format!("{}s",   secs) }
    else if secs < 3600 { format!("{}m",  secs / 60) }
    else if secs < 86400 { format!("{}h{}m", secs / 3600, (secs % 3600) / 60) }
    else                 { format!("{}d{}h", secs / 86400, (secs % 86400) / 3600) }
}

// ── `phantom workspace` subcommand ───────────────────────────────────────
//
// Per-machine pin: which directory + which agent the bare `phantom`
// command lands you in. Lets you keep one Windows box dedicated to one
// project so opening a fresh PowerShell + typing `phantom` drops you
// straight into the right context.
//
// Surface:
//   phantom workspace show               — current pin (or "no pin")
//   phantom workspace set <dir> [agent]  — pin this dir + optional agent
//   phantom workspace clear              — remove the [workspace] block
//   phantom workspace help

pub fn run_workspace(args: &[String]) -> anyhow::Result<()> {
    let sub = args.get(2).map(|s| s.as_str()).unwrap_or("show");
    match sub {
        "show" | "status" => {
            for line in workspace_show_lines()? { eprintln!("{}", line); }
            Ok(())
        }
        "set" => {
            let dir = args.get(3).ok_or_else(|| anyhow::anyhow!(
                "usage: phantom workspace set <dir> [pinned-agent]\n\
                 example: phantom workspace set C:\\Users\\m4932\\Projects\\foo coder"
            ))?;
            let pinned_agent = args.get(4).map(|s| s.as_str());
            for line in workspace_set_lines(dir, pinned_agent)? { eprintln!("{}", line); }
            Ok(())
        }
        "clear" | "unpin" => {
            for line in workspace_clear_lines()? { eprintln!("{}", line); }
            Ok(())
        }
        "help" | "--help" | "-h" => { workspace_help(); Ok(()) }
        other => anyhow::bail!("unknown `phantom workspace` subcommand: {}", other),
    }
}

fn workspace_help() {
    eprintln!("phantom workspace — pin this machine to a project dir + agent");
    eprintln!();
    eprintln!("  phantom workspace show                    show current pin");
    eprintln!("  phantom workspace set <dir> [agent]       set pin (agent default = master)");
    eprintln!("  phantom workspace clear                   remove [workspace] block");
    eprintln!();
    eprintln!("Once pinned, bare `phantom` (no args) auto-cd to <dir>, pre-selects");
    eprintln!("[agent.<agent>], and the conversation history lives under that path's");
    eprintln!("cwd-hash. Per-machine isolation: ayaneo can pin /projects/foo while");
    eprintln!("acer pins /projects/bar without either machine's phantom getting confused.");
}

pub fn workspace_show_lines() -> anyhow::Result<Vec<String>> {
    let path = agents_toml_path().ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let raw = fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;
    let cfg: AgentsConfig = toml::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("parse {}: {}", path.display(), e))?;
    let mut out = Vec::new();
    out.push(format!("workspace pin in {}:", path.display()));
    match cfg.workspace.default_dir.as_deref() {
        Some(d) if !d.is_empty() => {
            out.push(format!("  default_dir:  {}", d));
            let exists = std::path::Path::new(d).exists();
            out.push(format!("  dir status:   {}", if exists { "✓ exists" } else { "⚠ missing" }));
        }
        _ => out.push("  default_dir:  (unset — bare `phantom` uses caller's cwd)".into()),
    }
    out.push(format!("  pinned_agent: {}",
        cfg.workspace.pinned_agent.as_deref().unwrap_or("(unset — uses 'master')")));
    Ok(out)
}

pub fn workspace_set_lines(dir: &str, pinned_agent: Option<&str>) -> anyhow::Result<Vec<String>> {
    let path = agents_toml_path().ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let raw = fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;
    let mut doc: toml_edit::DocumentMut = raw.parse()
        .map_err(|e| anyhow::anyhow!("parse {}: {}", path.display(), e))?;

    let ws_tbl = doc
        .entry("workspace")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[workspace] is not a table"))?;
    ws_tbl.insert("default_dir", toml_edit::value(dir));
    if let Some(a) = pinned_agent {
        ws_tbl.insert("pinned_agent", toml_edit::value(a));
    }

    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, doc.to_string())?;
    fs::rename(&tmp, &path)?;

    let exists = std::path::Path::new(dir).exists();
    let mut out = vec![
        format!("✓ pinned workspace → {}", dir),
        format!("  agent:    {}", pinned_agent.unwrap_or("master (default)")),
        format!("  status:   {}", if exists { "✓ dir exists" } else { "⚠ dir missing — create it before launching phantom" }),
        format!("  effective on next `phantom` (no args)."),
    ];
    out.push(format!("  config:   {}", path.display()));
    Ok(out)
}

pub fn workspace_clear_lines() -> anyhow::Result<Vec<String>> {
    let path = agents_toml_path().ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let raw = fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;
    let mut doc: toml_edit::DocumentMut = raw.parse()
        .map_err(|e| anyhow::anyhow!("parse {}: {}", path.display(), e))?;
    let removed = doc.remove("workspace").is_some();
    if !removed {
        return Ok(vec!["(no [workspace] block to remove)".into()]);
    }
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, doc.to_string())?;
    fs::rename(&tmp, &path)?;
    Ok(vec![format!("✓ removed [workspace] block from {}", path.display())])
}

// ── `phantom cluster` subcommand ─────────────────────────────────────────
//
// Auto-wire the local node into the user's phantom-mesh cluster. The
// cluster_secret is HMAC-SHA256 key shared across nodes; lives in the
// vault as CLUSTER_SECRET so a fresh install can pull it via `phantom
// config pull` and then `phantom cluster join <name>` writes the right
// [cluster] block to agents.toml.
//
// Surface:
//   phantom cluster join <name>      — wire this box as <name> in the mesh
//   phantom cluster status           — ping each peer + show RPC reachability
//   phantom cluster leave            — remove the [cluster] block
//
// Known node names: yoyogood, ayaneo, laptop-gur943mk. Add new ones to
// CLUSTER_TOPOLOGY below + redeploy. Each node writes the OTHERS as its
// peers (skips itself), so config is self-correcting if you re-run.

/// FALLBACK cluster topology — used only when ~/.phantom-mesh/peers.json
/// is missing/empty (i.e. the user hasn't yet run `phantom config pull`
/// against a broker that has cluster peers configured). The vault
/// version (via /api/me/cluster-peers) is the source of truth once it
/// exists. Add a new machine via the dashboard — these constants are
/// just bootstrap defaults so a fresh box can `phantom cluster join
/// <name>` against the historic 4-node mesh without first having to
/// configure the dashboard.
const CLUSTER_TOPOLOGY: &[(&str, &str)] = &[
    ("mac-coordinator",  "http://100.87.93.58:7878"),
    ("yoyogood",         "http://100.87.70.65:7879"),
    ("ayaneo",           "http://100.107.205.98:7878"),
    ("laptop-gur943mk",  "http://100.106.176.125:7878"),
];

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ClusterPeer {
    pub name: String,
    pub url:  String,
    #[serde(default)]
    pub label: Option<String>,
    /// Tag list for capability-based dispatch. Lower-cased / deduped on
    /// the server side. Empty = "no auto-routing match", but caller can
    /// still --to that peer explicitly.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

pub fn peers_json_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".phantom-mesh").join("peers.json"))
}

/// Read the broker-pulled peer list. Returns Some only when the file
/// exists AND parses to a non-empty array — empty file or list reverts
/// callers to the hardcoded fallback. Failure modes silent: this is a
/// "if it works, use it" path, not "if it fails, error".
pub fn read_peers_json() -> Option<Vec<ClusterPeer>> {
    let path = peers_json_path()?;
    let raw = fs::read_to_string(&path).ok()?;
    let v: Vec<ClusterPeer> = serde_json::from_str(&raw).ok()?;
    if v.is_empty() { None } else { Some(v) }
}

pub fn write_peers_json(peers: &[ClusterPeer]) -> std::io::Result<()> {
    let path = peers_json_path()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no $HOME"))?;
    if let Some(p) = path.parent() { fs::create_dir_all(p)?; }
    let json = serde_json::to_string_pretty(peers).unwrap_or_else(|_| "[]".into());
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Resolve the effective topology: vault peers if present, else hardcoded.
/// Used by every cluster command so the choice is centralized.
fn effective_topology() -> Vec<(String, String)> {
    if let Some(peers) = read_peers_json() {
        return peers.into_iter().map(|p| (p.name, p.url)).collect();
    }
    CLUSTER_TOPOLOGY.iter()
        .map(|(n, u)| (n.to_string(), u.to_string()))
        .collect()
}

/// Detect this machine's Tailscale IPv4 by shelling out to `tailscale ip
/// -4`. Returns None when the binary isn't on PATH or the call fails
/// (CGNAT range guard skipped because tailscale only emits 100.x.y.z
/// when authenticated; non-Tailscale outputs are filtered by the trim).
pub fn detect_tailscale_ipv4() -> Option<String> {
    let out = std::process::Command::new("tailscale")
        .args(["ip", "-4"])
        .output()
        .ok()?;
    if !out.status.success() { return None; }
    let s = String::from_utf8_lossy(&out.stdout);
    let first = s.lines().next()?.trim().to_string();
    if first.is_empty() || !first.starts_with("100.") { return None; }
    Some(first)
}

/// Resolve a node_name for self-registration. Order:
///   1. PHANTOM_NODE_NAME env var (explicit override)
///   2. existing [cluster].node_name in agents.toml (don't overwrite user choice)
///   3. system hostname (lowercased; '_' replaced with '-' for cleaner urls)
pub fn resolve_self_node_name() -> Option<String> {
    if let Ok(v) = std::env::var("PHANTOM_NODE_NAME") {
        let v = v.trim();
        if !v.is_empty() { return Some(v.to_string()); }
    }
    if let Some(path) = agents_toml_path() {
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(cfg) = toml::from_str::<AgentsConfig>(&raw) {
                if let Some(n) = cfg.cluster.node_name {
                    if !n.trim().is_empty() { return Some(n); }
                }
            }
        }
    }
    // hostname() — std doesn't have it; shell out.
    let out = std::process::Command::new("hostname").output().ok()?;
    if !out.status.success() { return None; }
    let h = String::from_utf8_lossy(&out.stdout).trim().to_lowercase()
        .replace('_', "-");
    if h.is_empty() { None } else { Some(h) }
}

/// Read [core].port from agents.toml. Default 7878.
pub fn detect_listen_port() -> u16 {
    let Some(path) = agents_toml_path() else { return 7878; };
    let Ok(raw) = fs::read_to_string(&path) else { return 7878; };
    let Ok(cfg) = toml::from_str::<AgentsConfig>(&raw) else { return 7878; };
    let p = cfg.core.port;
    if p == 0 { 7878 } else { p }
}

/// POST {name, url, label, capabilities?} to broker
/// /api/me/cluster-peers/upsert. Returns the canonical {name, url} that
/// got registered. Best-effort: any failure becomes None + the caller
/// surfaces a notice but doesn't abort the login flow.
///
/// Note: capabilities is only sent if the upsert is the FIRST one for
/// this peer (i.e. the row didn't exist before, so we can default to
/// auto-detected os/arch tags). On subsequent re-registers, we omit
/// capabilities so the user's hand-edited dashboard tags don't get
/// silently overwritten on every login. The server-side semantics
/// (db.ts) treat missing capabilities = "keep existing".
pub async fn register_self_with_broker(
    broker_url: &str,
    token: &str,
    name: &str,
    url: &str,
    label: Option<&str>,
) -> Option<(String, String)> {
    let post_url = format!("{}/api/me/cluster-peers/upsert", broker_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build().ok()?;

    // Auto-detected capabilities for FIRST-register: os + arch.
    // We send them only if peers.json doesn't already contain this name
    // — that's the "fresh box, no prior caps" signal.
    let already_known = read_peers_json()
        .map(|peers| peers.iter().any(|p| p.name == name))
        .unwrap_or(false);
    let body = if already_known {
        serde_json::json!({
            "name":  name,
            "url":   url,
            "label": label.unwrap_or(""),
            // capabilities omitted → server keeps existing
        })
    } else {
        let auto_caps = vec![
            std::env::consts::OS.to_string(),
            std::env::consts::ARCH.to_string(),
        ];
        serde_json::json!({
            "name":  name,
            "url":   url,
            "label": label.unwrap_or(""),
            "capabilities": auto_caps,
        })
    };
    let resp = client.post(&post_url)
        .header("Authorization", format!("Bearer {}", token))
        .json(&body)
        .send().await.ok()?;
    if !resp.status().is_success() { return None; }
    Some((name.to_string(), url.to_string()))
}

/// Compose the full self-register flow: detect identity → POST upsert →
/// re-pull peers (so the local list includes our row) → cluster_join
/// (writes [cluster] block with the right peers list).
///
/// Returns the printed lines so callers can write them to stderr in
/// their own banner. Each step is best-effort: a failure surfaces in
/// the lines but doesn't abort.
pub async fn login_post_register_lines(
    broker_url: &str,
    token: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    let ts_ip = detect_tailscale_ipv4();
    let node_name = resolve_self_node_name();
    match (ts_ip, node_name) {
        (Some(ip), Some(name)) => {
            let port = detect_listen_port();
            let url = format!("http://{}:{}", ip, port);
            let registered = register_self_with_broker(
                broker_url, token, &name, &url,
                Some("auto-registered via phantom login"),
            ).await;
            match registered {
                Some((n, u)) => {
                    out.push(format!("  ✓ registered as '{}' → {}", n, u));
                    // Re-pull peers so the local peers.json includes the
                    // updated full list (our row + any others on the broker).
                    let _ = config_pull_lines(broker_url, token).await;
                    match cluster_join_lines(&n) {
                        Ok(lines) => for l in lines { out.push(l); }
                        Err(e) => {
                            out.push(format!("  ⚠ cluster join skipped: {}", e));
                            out.push(format!("    (you can retry: phantom cluster join {})", n));
                        }
                    }
                }
                None => {
                    out.push("  ⚠ broker upsert failed — peer registry unchanged".into());
                    out.push(format!("    (you can manually add via {}/account)", broker_url));
                }
            }
        }
        (None, _) => {
            out.push("  ◇ tailscale not detected — skipping self-register".into());
            out.push("    (install Tailscale + auth, then run: phantom cluster join <name>)".into());
        }
        (_, None) => {
            out.push("  ◇ couldn't determine node_name — skipping self-register".into());
            out.push("    (set: $env:PHANTOM_NODE_NAME='<name>'; phantom login)".into());
        }
    }
    out
}

pub async fn run_cluster(args: &[String]) -> anyhow::Result<()> {
    let sub = args.get(2).map(|s| s.as_str()).unwrap_or("status");
    match sub {
        "join" => {
            let name = args.get(3).ok_or_else(|| anyhow::anyhow!(
                "usage: phantom cluster join <node-name>\n\
                 known names: {}\n\
                 Add new names to CLUSTER_TOPOLOGY in core/src/cli_config.rs.",
                CLUSTER_TOPOLOGY.iter()
                    .filter(|(n, _)| *n != "mac-coordinator")
                    .map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
            ))?;
            for line in cluster_join_lines(name)? {
                eprintln!("{}", line);
            }
            Ok(())
        }
        // sync = config pull (gets latest peers list from broker) then
        // re-run cluster_join with this machine's stored node_name. The
        // 1-step shortcut for "a new machine joined the mesh; refresh me".
        // Same effect as `phantom config pull && phantom cluster join <name>`
        // but doesn't require remembering the node name on every machine.
        "sync" | "refresh" => {
            for line in cluster_sync_lines().await? {
                eprintln!("{}", line);
            }
            Ok(())
        }
        "status" | "ls" | "list" => {
            for line in cluster_status_lines().await? {
                eprintln!("{}", line);
            }
            Ok(())
        }
        "leave" => {
            for line in cluster_leave_lines()? {
                eprintln!("{}", line);
            }
            Ok(())
        }
        // Fan-out self-update: each peer downloads the latest binary for
        // its platform from the broker's R2 mirror, swaps via trampoline,
        // restarts `phantom serve`. --to <name> targets one node only.
        "upgrade" | "update" | "self-update" => {
            // Optional `--to <name>` to scope to a single peer.
            let mut target: Option<String> = None;
            let mut url_override: Option<String> = None;
            let mut i = 3;
            while i < args.len() {
                match args[i].as_str() {
                    "--to" => { target = args.get(i+1).cloned(); i += 2; }
                    "--url" => { url_override = args.get(i+1).cloned(); i += 2; }
                    "--help" | "-h" => {
                        eprintln!("phantom cluster upgrade — fan-out self-update across the mesh");
                        eprintln!();
                        eprintln!("  phantom cluster upgrade               every peer in peers.json");
                        eprintln!("  phantom cluster upgrade --to ayaneo   one peer only");
                        eprintln!("  phantom cluster upgrade --url <u>     override download url for ALL targets");
                        eprintln!();
                        eprintln!("Each peer downloads the platform-specific binary from");
                        eprintln!("https://phantommesh.io/dist/<asset> by default, swaps via");
                        eprintln!("trampoline (3s delay), then restarts `phantom serve`.");
                        eprintln!();
                        eprintln!("Auth: HMAC-SHA256 with cluster_secret in agents.toml.");
                        return Ok(());
                    }
                    _ => i += 1,
                }
            }
            for line in cluster_upgrade_lines(target.as_deref(), url_override.as_deref()).await? {
                eprintln!("{}", line);
            }
            Ok(())
        }
        "help" | "--help" | "-h" => {
            cluster_help();
            Ok(())
        }
        other => anyhow::bail!("unknown `phantom cluster` subcommand: {}", other),
    }
}

fn cluster_help() {
    eprintln!("phantom cluster — wire this machine into the phantom-mesh cluster");
    eprintln!();
    eprintln!("  phantom cluster join <name>   add [cluster] block to agents.toml");
    eprintln!("                                <name> picks the node's identity from the");
    eprintln!("                                vault peers (or hardcoded topology) + lists");
    eprintln!("                                the OTHERS as peers.");
    eprintln!("  phantom cluster sync          shortcut: pull latest peers + rejoin self");
    eprintln!("                                (use after a new machine joined the mesh)");
    eprintln!("  phantom cluster status        parallel ping each peer + show alive/dead + RTT");
    eprintln!("  phantom cluster upgrade       fan-out self-update: every peer pulls the latest");
    eprintln!("                                binary, swaps it via trampoline, restarts serve");
    eprintln!("  phantom cluster leave         remove the [cluster] block");
    eprintln!();
    eprintln!("Known names:");
    for (name, url) in CLUSTER_TOPOLOGY {
        eprintln!("  {:<18} {}", name, url);
    }
    eprintln!();
    eprintln!("CLUSTER_SECRET is read from process env (set via `phantom config pull`");
    eprintln!("which fetches it from the vault, OR from a user-scope env var). Bail with");
    eprintln!("a useful error if missing.");
}

pub fn cluster_join_lines(node_name: &str) -> anyhow::Result<Vec<String>> {
    // Validate name against EFFECTIVE topology (vault peers or hardcoded).
    let topology = effective_topology();
    let known_in_topology = topology.iter().any(|(n, _)| n == node_name);
    if !known_in_topology {
        let known: Vec<String> = topology.iter().map(|(n, _)| n.clone()).collect();
        anyhow::bail!(
            "unknown node-name '{}'. Known: {}\n\
             Add this name via the dashboard at https://phantommesh.io/account\n\
             (Cluster peers section), then run `phantom config pull` and retry.",
            node_name, known.join(", ")
        );
    }

    // CLUSTER_SECRET MUST be in the process env at call time. Fail loud
    // with the recovery hint inline so the user doesn't need to grep docs.
    let secret = std::env::var("CLUSTER_SECRET").ok().filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!(
            "CLUSTER_SECRET not in env — set via:\n\
             \n\
             1. (recommended) phantommesh.io/account → CLUSTER_SECRET field → Save\n\
                then on this box: phantom config pull\n\
             \n\
             2. (one-time, this shell only) $env:CLUSTER_SECRET = '<paste>'\n\
             \n\
             3. (one-time, persistent on this user account)\n\
                [Environment]::SetEnvironmentVariable('CLUSTER_SECRET','<paste>','User')\n\
                then OPEN A NEW SHELL before re-running."
        ))?;

    // Build peers list: every node from topology EXCEPT this one.
    let peers_lines: Vec<String> = topology.iter()
        .filter(|(n, _)| n != node_name)
        .map(|(n, url)| format!("  \"{}\",  # {}", url, n))
        .collect();

    // Use toml_edit so we PRESERVE existing [providers.*], [agent.*],
    // comments, formatting. Pure regex would mangle on first edge case.
    let path = agents_toml_path().ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let raw = fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;
    let mut doc: toml_edit::DocumentMut = raw.parse()
        .map_err(|e| anyhow::anyhow!("agents.toml is not valid TOML — fix syntax first: {}", e))?;

    // Build [cluster] table fresh — overwrites any existing values for
    // these keys (idempotent re-join). Other fields the user added under
    // [cluster] manually (capabilities, custom keys) are kept.
    let cluster_tbl = doc
        .entry("cluster")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[cluster] is not a table — refusing to overwrite"))?;
    cluster_tbl.insert("node_name", toml_edit::value(node_name));
    cluster_tbl.insert("cluster_secret", toml_edit::value(secret));
    // Build the peers list as a multi-line TOML snippet then parse it.
    // The previous approach (Array::push_formatted with set_suffix) produced
    // a single-line array where each value's "# name" suffix swallowed the
    // following comma as part of the comment — emitting unparseable
    //   peers = ["a"  # n1, "b"  # n2]
    // Building from a string forces the array onto multiple lines so each
    // comment naturally ends at EOL. (Two parallel fixes converged here on
    // 2026-05-04 — kept Z13's snippet-parse approach over the toml_edit
    // decor approach because it matches the original agents.toml peer
    // formatting that users hand-write.)
    let arr_lines: Vec<String> = topology.iter()
        .filter(|(n, _)| n != node_name)
        .map(|(n, u)| format!("  \"{}\",  # {}", u, n))
        .collect();
    let snippet = format!("peers = [\n{}\n]\n", arr_lines.join("\n"));
    let parsed: toml_edit::DocumentMut = snippet.parse()
        .expect("constructed peers snippet must parse");
    let item = parsed.as_table().get("peers").expect("peers we just inserted").clone();
    cluster_tbl.insert("peers", item);

    // Atomic write (.tmp + rename)
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, doc.to_string())?;
    fs::rename(&tmp, &path)?;

    let mut out = Vec::new();
    out.push(format!("✓ joined cluster as '{}'", node_name));
    out.push(format!("  agents.toml: {}", path.display()));
    out.push(format!("  peers ({}):", peers_lines.len()));
    for line in peers_lines { out.push(format!("    {}", line.trim_start_matches("  "))); }
    out.push("".into());
    out.push("  Next: restart `phantom serve` so the new [cluster] block takes effect:".into());
    // Per-OS restart hint. Was hardcoded to PowerShell, which made
    // macOS/Linux users see Stop-Process commands they couldn't run.
    if cfg!(target_os = "windows") {
        out.push("    Stop-Process -Name phantom -Force; Start-Process \"$env:USERPROFILE\\.local\\bin\\phantom.exe\" serve -WindowStyle Hidden".into());
    } else if cfg!(target_os = "macos") {
        out.push("    launchctl kickstart -k gui/$(id -u)/ai.phantommesh.serve".into());
    } else {
        // Linux + everything else — assume systemd user unit when
        // present, else fall back to pkill + nohup.
        out.push("    systemctl --user restart phantom-serve  # if installed as a user unit".into());
        out.push("    # or:  pkill -f 'phantom serve' && nohup phantom serve >/dev/null 2>&1 &".into());
    }
    out.push("  Then verify: phantom cluster status".into());
    Ok(out)
}

pub async fn cluster_status_lines() -> anyhow::Result<Vec<String>> {
    let path = agents_toml_path().ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let raw = fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;
    let cfg: AgentsConfig = toml::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("parse {}: {}", path.display(), e))?;

    let mut out = Vec::new();
    out.push(format!("Cluster status (via agents.toml [cluster] in {})", path.display()));
    out.push(format!("  this node: {}", cfg.cluster.node_name.as_deref().unwrap_or("(unset)")));
    out.push(format!("  peers:     {} configured", cfg.cluster.peers.len()));
    out.push(String::new());

    // Parallel pings via futures::join_all — sequential was 3s × N which
    // hit 24s+ for an 8-node mesh. Now bounded by the slowest single peer
    // (3s timeout). RTT also measured per-peer for ranking.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?;
    let probes = cfg.cluster.peers.iter().map(|peer| {
        let client = client.clone();
        let peer = peer.clone();
        async move {
            let url = format!("{}/healthz", peer.trim_end_matches('/'));
            let started = std::time::Instant::now();
            let result = client.get(&url).send().await;
            let rtt_ms = started.elapsed().as_millis();
            (peer, result, rtt_ms)
        }
    });
    let results = futures::future::join_all(probes).await;
    let mut up = 0usize;
    for (peer, result, rtt_ms) in results {
        match result {
            Ok(resp) if resp.status().is_success() => {
                up += 1;
                out.push(format!("  ✓ {:<40}  200 ok    ({}ms)", peer, rtt_ms));
            }
            Ok(resp) => {
                out.push(format!("  ⚠ {:<40}  HTTP {}    ({}ms)", peer, resp.status().as_u16(), rtt_ms));
            }
            Err(e) => {
                let short = e.to_string().chars().take(60).collect::<String>();
                out.push(format!("  ✗ {:<40}  unreachable: {}", peer, short));
            }
        }
    }
    out.push(String::new());
    out.push(format!("  summary: {}/{} peers reachable", up, cfg.cluster.peers.len()));
    Ok(out)
}

/// Fan-out self-update via the cluster RPC `/rpc/admin/self-update` endpoint.
/// Each peer downloads its platform's binary, swaps via trampoline, restarts.
///
/// `target_name` = Some("ayaneo") restricts to one peer; None hits every
/// peer in peers.json. `url_override` lets callers point all targets at a
/// non-default URL (testing, staging mirror).
///
/// Each call is best-effort: a single peer's failure doesn't abort the
/// fan-out; we just record the error in the output line and move on.
pub async fn cluster_upgrade_lines(
    target_name: Option<&str>,
    url_override: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    // 1. Load the peer registry (vault-synced) — fall back to hardcoded
    //    topology if the user hasn't run `phantom config pull` yet.
    let mut peers = read_peers_json().unwrap_or_else(|| {
        CLUSTER_TOPOLOGY.iter().map(|(n, u)| ClusterPeer {
            name: n.to_string(), url: u.to_string(),
            label: None, capabilities: Vec::new(),
        }).collect()
    });
    if let Some(name) = target_name {
        peers.retain(|p| p.name == name);
        if peers.is_empty() {
            anyhow::bail!(
                "no peer named '{}' in peers.json — run `phantom config pull` to refresh",
                name
            );
        }
    }
    // Don't try to upgrade ourselves over the network — would just fail
    // when our own serve goes down mid-request.
    let self_name = resolve_self_node_name();
    let original_count = peers.len();
    if let Some(me) = &self_name {
        peers.retain(|p| &p.name != me);
    }
    let skipped_self = original_count - peers.len();

    // 2. cluster_secret needed for HMAC.
    let cfg_path = agents_toml_path().ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let raw = fs::read_to_string(&cfg_path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", cfg_path.display(), e))?;
    let cfg: AgentsConfig = toml::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("parse {}: {}", cfg_path.display(), e))?;
    let secret = cfg.cluster.cluster_secret.clone()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!(
            "cluster_secret missing in [cluster] block of {} — run `phantom cluster join <name>` first",
            cfg_path.display()
        ))?;

    let mut out = Vec::new();
    out.push(format!("◆ cluster upgrade — {} peer(s){}",
        peers.len(),
        if skipped_self > 0 { format!(" (skipped self: {})", self_name.as_deref().unwrap_or("?")) } else { String::new() }));
    if let Some(u) = url_override {
        out.push(format!("  url override: {}", u));
    }

    // 3. Parallel POST. Each peer gets its own platform-specific URL —
    //    we DON'T know the peer's OS from peers.json, so let the server
    //    side pick (default_dist_asset_name in serve.rs) unless the
    //    caller forced a specific URL.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(330))  // > server's 300s download budget
        .build()?;
    let probes = peers.iter().map(|peer| {
        let client = client.clone();
        let peer = peer.clone();
        let secret = secret.clone();
        let url_override = url_override.map(|s| s.to_string());
        async move {
            let body = if let Some(u) = url_override {
                serde_json::json!({ "url": u }).to_string()
            } else {
                "{}".to_string()
            };
            let auth = hmac_sha256_hex(&secret, body.as_bytes());
            let url = format!("{}/rpc/admin/self-update", peer.url.trim_end_matches('/'));
            let started = std::time::Instant::now();
            let result = client.post(&url)
                .header("X-Cluster-Auth", &auth)
                .header("Content-Type", "application/json")
                .body(body)
                .send().await;
            (peer, result, started.elapsed())
        }
    });
    let results = futures::future::join_all(probes).await;

    let mut ok_count = 0usize;
    for (peer, result, elapsed) in results {
        match result {
            Ok(resp) if resp.status().is_success() => {
                ok_count += 1;
                let body = resp.text().await.unwrap_or_default();
                let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
                let bytes = parsed.get("downloaded").and_then(|v| v.as_u64()).unwrap_or(0);
                out.push(format!("  ✓ {:<14} scheduled (downloaded {} bytes, ~{:.1}s)",
                    peer.name, bytes, elapsed.as_secs_f32()));
            }
            Ok(resp) => {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                let snippet = body.chars().take(120).collect::<String>();
                out.push(format!("  ✗ {:<14} HTTP {} — {}", peer.name, status, snippet));
            }
            Err(e) => {
                let short = e.to_string().chars().take(80).collect::<String>();
                out.push(format!("  ✗ {:<14} unreachable: {}", peer.name, short));
            }
        }
    }
    out.push(String::new());
    out.push(format!("  summary: {}/{} peers scheduled", ok_count, peers.len()));
    out.push(String::new());
    out.push("  Each peer's serve will exit ~500ms after responding, then a".into());
    out.push("  trampoline waits ~3s, swaps phantom.exe.new → phantom.exe,".into());
    out.push("  and starts `phantom serve` again. Verify with:".into());
    out.push("    sleep 10 && phantom cluster status".into());
    Ok(out)
}

/// Pull the latest peer registry from the broker, then rewrite this
/// machine's [cluster] block so it points at the (now-larger) set of
/// peers minus self. Use when another machine just joined the mesh and
/// you want to rope it in without remembering the local node name +
/// stitching together two commands.
pub async fn cluster_sync_lines() -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();

    // Need a broker URL + token to pull. Fail loud if neither is stashed.
    let stored = read_broker_config();
    let from_auth = crate::auth::load();
    let url = stored.as_ref().map(|s| s.url.clone())
        .or_else(|| from_auth.as_ref().map(|a| a.broker_url.clone()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://phantommesh.io".to_string());
    let token = stored.as_ref().map(|s| s.token.clone())
        .or_else(|| from_auth.as_ref().map(|a| a.broker_token.clone()))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!(
            "no broker token — run `phantom login` first"
        ))?;

    out.push(format!("◆ pulling latest cluster peers from {}…", url));
    match config_pull_lines(&url, &token).await {
        Ok(lines) => for l in lines { out.push(format!("  {}", l)); }
        Err(e) => {
            out.push(format!("⚠ config pull failed: {} — using cached peers.json if any", e));
        }
    }
    out.push(String::new());

    // Resolve this machine's name from agents.toml (set by previous
    // `phantom cluster join`) — fall back to PHANTOM_NODE_NAME or hostname
    // detection so a fresh box can also `phantom cluster sync` without
    // a prior explicit join.
    let node_name = resolve_self_node_name()
        .ok_or_else(|| anyhow::anyhow!(
            "couldn't determine this machine's node name. \
             Set $env:PHANTOM_NODE_NAME='<name>' or run `phantom cluster join <name>` once first."
        ))?;
    out.push(format!("◆ rewriting [cluster] block as '{}'…", node_name));
    match cluster_join_lines(&node_name) {
        Ok(lines) => for l in lines { out.push(format!("  {}", l)); }
        Err(e) => {
            out.push(format!("⚠ cluster join failed: {}", e));
            return Ok(out);
        }
    }
    Ok(out)
}

pub fn cluster_leave_lines() -> anyhow::Result<Vec<String>> {
    let path = agents_toml_path().ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let raw = fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;
    let mut doc: toml_edit::DocumentMut = raw.parse()
        .map_err(|e| anyhow::anyhow!("agents.toml parse: {}", e))?;
    let removed = doc.remove("cluster").is_some();
    if !removed {
        return Ok(vec!["(no [cluster] block to remove)".into()]);
    }
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, doc.to_string())?;
    fs::rename(&tmp, &path)?;
    Ok(vec![format!("✓ removed [cluster] block from {}", path.display())])
}

// ── `phantom config` subcommand ──────────────────────────────────────────
//
// Surface:
//   phantom config pull  --token <jwt> [--url <broker>]   — fetch + write env
//   phantom config show                                   — show stored broker config
//   phantom config clear                                  — drop stored broker config
//
// The pull flow is the other half of the phantommesh.io key vault: user
// stores keys once via /account UI, then runs `phantom config pull` on
// each box to sync. First call needs --token (copy from the dashboard);
// subsequent calls remember the token in ~/.phantom-mesh/broker.json so
// you can re-pull without arguments.

pub fn broker_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".phantom-mesh").join("broker.json"))
}

#[derive(serde::Serialize, serde::Deserialize, Default, Debug, Clone)]
pub struct BrokerConfig {
    /// Base URL of the broker, e.g. "https://phantommesh.io". Trailing slash
    /// is stripped on save so we always join with "/api/...".
    pub url: String,
    /// JWT broker_token issued by the broker's OAuth callback. Stored
    /// verbatim. Treat as a secret — file is mode 0600 best-effort.
    pub token: String,
}

pub fn read_broker_config() -> Option<BrokerConfig> {
    let path = broker_config_path()?;
    let raw = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn write_broker_config(cfg: &BrokerConfig) -> anyhow::Result<()> {
    let path = broker_config_path()
        .ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    if let Some(p) = path.parent() { fs::create_dir_all(p)?; }
    let json = serde_json::to_string_pretty(cfg)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, &path)?;
    // Best-effort restrict perms on Unix; on Windows, ACLs default to
    // user-only inside ~/AppData so this is mostly a no-op platform-wise.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub async fn run_config(args: &[String]) -> anyhow::Result<()> {
    let sub = args.get(2).map(|s| s.as_str()).unwrap_or("show");
    match sub {
        "pull" => {
            // Parse --token / --url from the remaining argv.
            let mut token: Option<String> = None;
            let mut url:   Option<String> = None;
            let mut i = 3;
            while i < args.len() {
                match args[i].as_str() {
                    "--token" | "-t" => { i += 1; if let Some(v) = args.get(i) { token = Some(v.clone()); } }
                    "--url"   | "-u" => { i += 1; if let Some(v) = args.get(i) { url   = Some(v.clone()); } }
                    other => anyhow::bail!("unknown flag {} for `phantom config pull` (expected --token / --url)", other),
                }
                i += 1;
            }
            // Resolution order for url + token:
            //   1. explicit --url / --token flags (highest priority)
            //   2. ~/.phantom-mesh/broker.json from a previous `config pull`
            //   3. ~/.phantom-mesh/auth.json from a previous `phantom login`
            //      (broker_token field — set by login_broker)
            //   4. URL default = https://phantommesh.io; token default = error
            // Means a fresh box can do `phantom login` once and then
            // `phantom config pull` is zero-arg from there on.
            let stored = read_broker_config();
            let from_auth = crate::auth::load();
            let url   = url
                .or_else(|| stored.as_ref().map(|s| s.url.clone()))
                .or_else(|| from_auth.as_ref().map(|a| a.broker_url.clone()))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "https://phantommesh.io".to_string());
            let token = token
                .or_else(|| stored.as_ref().map(|s| s.token.clone()))
                .or_else(|| from_auth.as_ref().map(|a| a.broker_token.clone()))
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!(
                    "no token — run `phantom login` first to get one (it'll auto-pull keys), \
                     or pass --token <jwt> manually (copy from {}/account)",
                    url,
                ))?;
            for line in config_pull_lines(&url, &token).await? {
                eprintln!("{}", line);
            }
            // Persist for next time (only when pull was successful — the
            // helper either returns Ok or bails before reaching here).
            write_broker_config(&BrokerConfig {
                url:   url.trim_end_matches('/').to_string(),
                token,
            })?;
            Ok(())
        }
        "show" | "status" => {
            match read_broker_config() {
                Some(cfg) => {
                    eprintln!("broker config: {}", broker_config_path().map(|p| p.display().to_string()).unwrap_or_default());
                    eprintln!("  url:   {}", if cfg.url.is_empty() { "(unset)" } else { cfg.url.as_str() });
                    eprintln!("  token: {}", mask_key(&cfg.token));
                    Ok(())
                }
                None => {
                    eprintln!("(no broker config saved yet — run `phantom config pull --token <jwt>` once)");
                    Ok(())
                }
            }
        }
        "clear" | "logout" => {
            if let Some(p) = broker_config_path() {
                let _ = fs::remove_file(&p);
                eprintln!("✓ removed {}", p.display());
            }
            Ok(())
        }
        "help" | "--help" | "-h" => {
            config_help();
            Ok(())
        }
        other => anyhow::bail!("unknown `phantom config` subcommand: {} — try `phantom config help`", other),
    }
}

fn config_help() {
    eprintln!("phantom config — pull LLM API keys from your phantommesh.io vault");
    eprintln!();
    eprintln!("  phantom config pull --token <jwt> [--url <broker>]");
    eprintln!("                                first-time pull. Token comes from the");
    eprintln!("                                broker's /account page; URL defaults to");
    eprintln!("                                https://phantommesh.io.");
    eprintln!("  phantom config pull           subsequent pulls reuse the saved token.");
    eprintln!("  phantom config show           show saved url + masked token");
    eprintln!("  phantom config clear          delete saved broker config");
    eprintln!();
    eprintln!("Pulled keys are written to ~/.phantom-mesh/env (auto-loaded by phantom");
    eprintln!("on every command). Existing entries are merged: keys returned by the");
    eprintln!("broker overwrite the local file's values; locals not in the response");
    eprintln!("are kept untouched (so a key you've only set locally isn't deleted).");
}

/// HTTP fetch + write logic, factored out so tests can hit a mock URL.
/// Returns the formatted output lines; caller writes them to stderr.
pub async fn config_pull_lines(broker_url: &str, token: &str) -> anyhow::Result<Vec<String>> {
    let url = format!("{}/api/me/settings/raw", broker_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let resp = client.get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("GET {}: {}", url, e))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let head: String = body.chars().take(200).collect();
        anyhow::bail!("broker returned HTTP {} — {}", status.as_u16(), head);
    }
    let parsed: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("non-JSON response from broker: {} (body head: {})",
            e, body.chars().take(120).collect::<String>()))?;
    let env_obj = parsed.get("env").and_then(|v| v.as_object())
        .ok_or_else(|| anyhow::anyhow!("broker response missing `env` object"))?;

    // Merge into existing env file: locals win for keys NOT in the
    // response, broker wins for keys IN the response. Empty broker
    // values are skipped (treated as "not stored").
    let env_path = env_file_path().ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let mut existing = read_env_file(&env_path);
    let mut applied: Vec<String> = Vec::new();
    for (k, v) in env_obj {
        if let Some(val) = v.as_str() {
            if val.is_empty() { continue; }
            existing.insert(k.clone(), val.to_string());
            // Also push into current process env so a follow-up phantom
            // command in the same shell sees the new keys without needing
            // a re-source ritual.
            std::env::set_var(k, val);
            applied.push(k.clone());
        }
    }
    write_env_file(&env_path, &existing)?;

    let mut out = Vec::new();
    out.push(format!("✓ pulled {} keys from {}", applied.len(), broker_url));
    let mut keys = applied.clone();
    keys.sort();
    for k in &keys {
        out.push(format!("    {}", k));
    }
    out.push(format!("  written to: {}", env_path.display()));

    // Also pull the cluster peer registry. Best-effort: any failure here
    // doesn't break the env-key sync above. If the broker doesn't have
    // the endpoint (older deploy) or the user just hasn't configured
    // peers yet, the resulting peers.json stays empty and the cluster
    // commands fall back to the hardcoded CLUSTER_TOPOLOGY.
    let peers_url = format!("{}/api/me/cluster-peers", broker_url.trim_end_matches('/'));
    match client.get(&peers_url)
        .header("Authorization", format!("Bearer {}", token))
        .send().await
    {
        Ok(resp) if resp.status().is_success() => {
            let body = resp.text().await.unwrap_or_default();
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(arr) = json.get("peers").and_then(|v| v.as_array()) {
                    let peers: Vec<ClusterPeer> = arr.iter().filter_map(|p| {
                        let name = p.get("name")?.as_str()?.to_string();
                        let url = p.get("url")?.as_str()?.to_string();
                        let label = p.get("label").and_then(|v| v.as_str()).map(String::from);
                        let capabilities: Vec<String> = p.get("capabilities")
                            .and_then(|v| v.as_array())
                            .map(|arr| arr.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                            .unwrap_or_default();
                        if name.is_empty() || url.is_empty() { None } else {
                            Some(ClusterPeer { name, url, label, capabilities })
                        }
                    }).collect();
                    if let Some(path) = peers_json_path() {
                        if write_peers_json(&peers).is_ok() {
                            out.push(format!("  cluster peers: {} synced → {}",
                                peers.len(), path.display()));
                        }
                    }
                }
            }
        }
        Ok(resp) => {
            // 404 / 403 / etc — broker doesn't have the endpoint or rejected.
            // Surface the status code so the user knows but keep going.
            out.push(format!("  cluster peers: skipped (broker returned HTTP {})",
                resp.status().as_u16()));
        }
        Err(_) => {
            // Network failure to the same broker we just pulled keys from
            // is unexpected, but treat as best-effort skip.
            out.push("  cluster peers: skipped (network error)".into());
        }
    }

    out.push("  active in this shell session immediately.".into());
    out.push("  for `phantom serve` to pick up: restart the service.".into());
    Ok(out)
}

// ── `phantom keys` subcommand ────────────────────────────────────────────

/// Entry point for `phantom keys ...`.
/// Writes always go through `~/.phantom-mesh/env`; the running phantom
/// process is told to set the var in its own env too so `phantom keys set`
/// followed by another phantom command in the same shell sees the new value.
pub fn run_keys(args: &[String]) -> anyhow::Result<()> {
    let sub = args.get(2).map(|s| s.as_str()).unwrap_or("list");
    match sub {
        "list" | "ls" => keys_list(),
        "set" => {
            let provider = args.get(3).ok_or_else(|| anyhow::anyhow!(
                "usage: phantom keys set <provider> <key>\n\
                 example: phantom keys set groq sk-...\n\
                 list known: groq, cerebras, opencode, openai, anthropic, gemini, openrouter, nvidia, deepseek, mistral, together"
            ))?;
            let key = args.get(4).ok_or_else(|| anyhow::anyhow!(
                "usage: phantom keys set <provider> <key> — got provider but no key value"
            ))?;
            keys_set(provider, key)
        }
        "remove" | "rm" | "unset" => {
            let provider = args.get(3).ok_or_else(|| anyhow::anyhow!(
                "usage: phantom keys remove <provider>"
            ))?;
            keys_remove(provider)
        }
        "help" | "--help" | "-h" => {
            keys_help();
            Ok(())
        }
        other => anyhow::bail!(
            "unknown `phantom keys` subcommand: {} — try `phantom keys help`",
            other
        ),
    }
}

fn keys_help() {
    eprintln!("phantom keys — manage ~/.phantom-mesh/env (auto-loaded at startup)");
    eprintln!();
    eprintln!("  phantom keys list                       show all configured keys (masked)");
    eprintln!("  phantom keys set <provider> <key>       save a key (overwrites if present)");
    eprintln!("  phantom keys remove <provider>          delete a key");
    eprintln!();
    eprintln!("Provider names map to env vars by convention:");
    eprintln!("  groq        → GROQ_API_KEY");
    eprintln!("  cerebras    → CEREBRAS_API_KEY");
    eprintln!("  opencode    → OPENCODE_API_KEY");
    eprintln!("  openai      → OPENAI_API_KEY");
    eprintln!("  anthropic   → ANTHROPIC_API_KEY");
    eprintln!("  gemini      → GEMINI_API_KEY");
    eprintln!("  openrouter  → OPENROUTER_API_KEY");
    eprintln!("  nvidia      → NVIDIA_NIM_API_KEY");
    eprintln!("  deepseek    → DEEPSEEK_API_KEY");
    eprintln!();
    eprintln!("After `phantom keys set`, the new var takes effect for any phantom");
    eprintln!("command in this shell session. To pick it up in an already-running");
    eprintln!("`phantom serve`, restart the service.");
}

fn keys_list() -> anyhow::Result<()> {
    for line in keys_list_lines()? { eprintln!("{}", line); }
    Ok(())
}

/// Same as `keys_list` but returns the formatted lines instead of writing
/// to stderr. Used by the TUI `/keys` slash command (renders as transcript
/// items, not stderr) and by future programmatic callers.
pub fn keys_list_lines() -> anyhow::Result<Vec<String>> {
    let path = env_file_path().ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let vars = read_env_file(&path);
    let mut out = Vec::new();
    if vars.is_empty() {
        out.push(format!("(no keys saved in {})", path.display()));
        out.push("set one via: phantom keys set <provider> <key>".into());
        return Ok(out);
    }
    out.push(format!("Keys saved in {}", path.display()));
    out.push(String::new());
    let mut keys: Vec<&String> = vars.keys().collect();
    keys.sort();
    for k in keys {
        let masked = mask_key(&vars[k]);
        let live = std::env::var(k).ok();
        let status = match &live {
            Some(v) if v == &vars[k] => "✓ active",
            Some(_) => "⚠ overridden by shell env",
            None => "  saved (not yet sourced)",
        };
        out.push(format!("  {:<28} {}  [{}]", k, masked, status));
    }
    Ok(out)
}

fn keys_set(provider: &str, key: &str) -> anyhow::Result<()> {
    for line in keys_set_lines(provider, key)? { eprintln!("{}", line); }
    Ok(())
}

pub fn keys_set_lines(provider: &str, key: &str) -> anyhow::Result<Vec<String>> {
    let path = env_file_path().ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let mut vars = read_env_file(&path);
    let var_name = provider_env_var_name(provider);
    let prev = vars.get(&var_name).cloned();
    vars.insert(var_name.clone(), key.to_string());
    write_env_file(&path, &vars)?;
    std::env::set_var(&var_name, key);
    let mut out = Vec::new();
    out.push(match prev {
        Some(_) => format!("✓ updated {} ({})", var_name, mask_key(key)),
        None => format!("✓ saved {} ({})", var_name, mask_key(key)),
    });
    out.push(format!("  file: {}", path.display()));
    out.push("  active in this shell session immediately.".into());
    out.push("  for `phantom serve` to pick it up: restart the service.".into());
    Ok(out)
}

fn keys_remove(provider: &str) -> anyhow::Result<()> {
    for line in keys_remove_lines(provider)? { eprintln!("{}", line); }
    Ok(())
}

pub fn keys_remove_lines(provider: &str) -> anyhow::Result<Vec<String>> {
    let path = env_file_path().ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let mut vars = read_env_file(&path);
    let var_name = provider_env_var_name(provider);
    if vars.remove(&var_name).is_none() {
        return Ok(vec![format!("(no key saved for {})", var_name)]);
    }
    write_env_file(&path, &vars)?;
    std::env::remove_var(&var_name);
    Ok(vec![format!("✓ removed {} from {}", var_name, path.display())])
}

// ── `phantom providers` subcommand ───────────────────────────────────────

pub fn run_providers(args: &[String]) -> anyhow::Result<()> {
    let sub = args.get(2).map(|s| s.as_str()).unwrap_or("list");
    match sub {
        "list" | "ls" => providers_list(),
        "priority" => providers_priority(args),
        "help" | "--help" | "-h" => {
            providers_help();
            Ok(())
        }
        other => anyhow::bail!(
            "unknown `phantom providers` subcommand: {} — try `phantom providers help`",
            other
        ),
    }
}

fn providers_help() {
    eprintln!("phantom providers — view configured providers and edit failover priority");
    eprintln!();
    eprintln!("  phantom providers list                          show configured providers + key status");
    eprintln!("  phantom providers priority <agent>              show current priority for that agent");
    eprintln!("  phantom providers priority <agent> <p1> <p2>... set priority list for that agent");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  phantom providers priority master groq cerebras opencode");
    eprintln!("  phantom providers priority coder groq");
    eprintln!();
    eprintln!("Edits ~/.phantom-mesh/agents.toml in place, preserving comments and formatting.");
    eprintln!("Effective on next `phantom repl` / restart of `phantom serve`.");
}

fn providers_list() -> anyhow::Result<()> {
    for line in providers_list_lines()? { eprintln!("{}", line); }
    Ok(())
}

pub fn providers_list_lines() -> anyhow::Result<Vec<String>> {
    let path = agents_toml_path().ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let raw = fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;
    let cfg: AgentsConfig = toml::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("parse {}: {}", path.display(), e))?;

    let mut out = Vec::new();
    if cfg.providers.is_empty() {
        out.push(format!("(no [providers.*] blocks in {})", path.display()));
        return Ok(out);
    }
    out.push(format!("Providers configured in {}", path.display()));
    out.push(String::new());
    let mut names: Vec<&String> = cfg.providers.keys().collect();
    names.sort();
    for name in names {
        let p = &cfg.providers[name];
        let key_status = if let Some(env_var) = p.api_key_env.as_ref() {
            match std::env::var(env_var) {
                Ok(v) if !v.is_empty() => format!("✓ {} = {}", env_var, mask_key(&v)),
                _ => format!("⚠ {} (env var unset — /keys set {} <key>)", env_var, name),
            }
        } else if p.api_key.as_ref().is_some_and(|k| !k.is_empty()) {
            "✓ inline key set".to_string()
        } else {
            "⚠ no key configured".to_string()
        };
        let url = p.url.clone().unwrap_or_else(|| "(default for provider type)".into());
        let model = p.default_model.clone().unwrap_or_else(|| "(none)".into());
        out.push(format!("  {} ({})", name, p.provider_type.is_empty().then(|| "<no type>").unwrap_or(&p.provider_type)));
        out.push(format!("    url:    {}", url));
        out.push(format!("    model:  {}", model));
        out.push(format!("    key:    {}", key_status));
    }

    if !cfg.agent.is_empty() {
        out.push(String::new());
        out.push("Agents and their failover order:".into());
        let mut anames: Vec<&String> = cfg.agent.keys().collect();
        anames.sort();
        for n in anames {
            let a = &cfg.agent[n];
            let order = if let Some(p) = a.providers.as_ref() {
                format!("priority list: {}", p.join(" → "))
            } else if !a.provider.is_empty() {
                format!("primary: {} (then alphabetical of others)", a.provider)
            } else {
                "(no provider configured)".to_string()
            };
            out.push(format!("  {}: {}", n, order));
        }
    }
    Ok(out)
}

fn providers_priority(args: &[String]) -> anyhow::Result<()> {
    // Convert `phantom providers priority <agent> ...` argv into the
    // generic _lines API: `_lines(agent, [p1, p2, ...])`.
    let agent = args.get(3).cloned();
    let names: Vec<String> = args.get(4..).map(|s| s.to_vec()).unwrap_or_default();
    for line in providers_priority_lines(agent.as_deref(), &names)? {
        eprintln!("{}", line);
    }
    Ok(())
}

/// Returns lines describing the action taken (or current state if no
/// `new_order` provided). Reads + edits agents.toml in place via toml_edit,
/// preserving comments and formatting.
/// Read the current `[agent.<name>].providers` priority list from
/// agents.toml. Returns Vec<provider:model> strings in declared order.
/// Empty when the agent block doesn't exist or has no providers field.
/// Used by the TUI's /priority modal to populate its initial state.
pub fn read_agent_priority(agent: &str) -> Vec<String> {
    let path = match agents_toml_path() { Some(p) => p, None => return Vec::new() };
    let raw = match fs::read_to_string(&path) { Ok(r) => r, Err(_) => return Vec::new() };
    let doc: toml_edit::DocumentMut = match raw.parse() { Ok(d) => d, Err(_) => return Vec::new() };
    let arr = doc.get("agent")
        .and_then(|v| v.get(agent))
        .and_then(|t| t.get("providers"))
        .and_then(|v| v.as_array());
    match arr {
        Some(a) => a.iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        None => Vec::new(),
    }
}

pub fn providers_priority_lines(
    agent: Option<&str>,
    new_order: &[String],
) -> anyhow::Result<Vec<String>> {
    let agent = agent.ok_or_else(|| anyhow::anyhow!(
        "usage: providers priority <agent> [<p1> <p2> ...]"
    ))?;
    let path = agents_toml_path().ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let raw = fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;

    // Show current if no list given
    if new_order.is_empty() {
        let doc: toml_edit::DocumentMut = raw.parse()?;
        let order = doc
            .get(&"agent")
            .and_then(|v| v.get(agent))
            .and_then(|t| t.get("providers"))
            .map(|v| v.to_string())
            .unwrap_or_else(|| "(no priority list set — uses 'provider' + alphabetical fallback)".to_string());
        let primary = doc
            .get(&"agent")
            .and_then(|v| v.get(agent))
            .and_then(|t| t.get("provider"))
            .map(|v| v.as_str().unwrap_or("(unset)").to_string())
            .unwrap_or_else(|| format!("(no [agent.{}] block)", agent));
        return Ok(vec![
            format!("agent.{}", agent),
            format!("  provider:  {}", primary),
            format!("  providers: {}", order),
        ]);
    }

    // Validate names exist as [providers.X].
    // Each entry can be `<provider>` or `<provider>:<model>` — strip the
    // model suffix before checking, otherwise "opencode:claude-sonnet-4-6"
    // would warn even though the underlying provider IS configured. The
    // model id itself is dynamic (provider catalog can change) so we
    // intentionally don't try to validate it here.
    let cfg: AgentsConfig = toml::from_str(&raw)?;
    let known: std::collections::HashSet<String> = cfg.providers.keys().cloned().collect();
    let mut out = Vec::new();
    let unknown: Vec<&String> = new_order.iter()
        .filter(|n| {
            let provider_part = n.split_once(':').map(|(p, _)| p).unwrap_or(n);
            !known.contains(provider_part)
        })
        .collect();
    if !unknown.is_empty() {
        out.push(format!("⚠ these provider names are NOT in [providers.*]: {:?}", unknown));
        out.push(format!("  configured providers: {:?}", known));
        out.push("  proceeding anyway — runtime will skip unknown names at dispatch time.".into());
    }

    // Edit with toml_edit to preserve formatting
    let mut doc: toml_edit::DocumentMut = raw.parse()?;
    if !doc.contains_key("agent") {
        anyhow::bail!("agents.toml has no [agent.*] tables to edit");
    }
    let agent_tbl = doc["agent"].as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[agent] is not a table"))?;
    if !agent_tbl.contains_key(agent) {
        agent_tbl.insert(agent, toml_edit::Item::Table(toml_edit::Table::new()));
    }
    let sub = agent_tbl[agent].as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[agent.{}] is not a table", agent))?;
    let mut arr = toml_edit::Array::new();
    for n in new_order {
        arr.push(n.as_str());
    }
    sub.insert("providers", toml_edit::value(arr));

    // Atomic write
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, doc.to_string())?;
    fs::rename(&tmp, &path)?;
    out.push(format!("✓ agent.{}.providers = [{}]", agent, new_order.join(", ")));
    out.push(format!("  file: {}", path.display()));
    out.push("  effective on next /agent switch / repl restart / serve restart.".into());
    Ok(out)
}

// ── `phantom models` subcommand ──────────────────────────────────────────
//
// Surface:
//   phantom models status                  — show cache age + free/paid counts
//   phantom models refresh                 — refresh ALL configured providers
//   phantom models refresh <provider>      — refresh just that one
//
// Why this exists separate from the TUI `/models` command: the TUI command
// fetches into the same on-disk cache (~/.phantom-mesh/models-cache.json)
// but only when the user is sitting in the TUI. Operators warming a fresh
// box, CI smoke tests, and `phantom serve` startup all want a no-TUI way
// to populate or audit the cache. Symmetric with `phantom keys` /
// `phantom providers` for hands-off automation.

pub async fn run_models(args: &[String]) -> anyhow::Result<()> {
    let sub = args.get(2).map(|s| s.as_str()).unwrap_or("status");
    match sub {
        "status" | "ls" | "list" => {
            for line in models_status_lines()? { eprintln!("{}", line); }
            Ok(())
        }
        "refresh" => {
            let filter = args.get(3).map(|s| s.as_str());
            for line in models_refresh_lines(filter).await? {
                eprintln!("{}", line);
            }
            Ok(())
        }
        // `phantom models test [provider:model | provider]` — probe whether
        // a model actually executes tool calls when the API request asks for
        // them. Background: free models (especially preview tier on opencode
        // like hy3-preview-free) frequently respond with plausible-sounding
        // "✓ done!" text WITHOUT ever emitting a tool_calls block, even when
        // the request includes tools=[…] and tool_choice="auto". That breaks
        // every workflow that depends on file_write / shell / etc.
        //
        // This subcommand isolates the question "does THIS model honor tool
        // calls" from the broader "does my agent work" question, so the user
        // can pick a model based on evidence instead of guesswork.
        "test" | "probe" => {
            let target = args.get(3).map(|s| s.as_str());
            for line in models_test_lines(target).await? {
                eprintln!("{}", line);
            }
            Ok(())
        }
        "help" | "--help" | "-h" => { models_help(); Ok(()) }
        other => anyhow::bail!(
            "unknown `phantom models` subcommand: {} — try `phantom models help`",
            other
        ),
    }
}

fn models_help() {
    eprintln!("phantom models — manage the model cache + probe model behavior");
    eprintln!();
    eprintln!("  phantom models status                show cached providers, model counts, age");
    eprintln!("  phantom models refresh               refresh ALL configured providers");
    eprintln!("  phantom models refresh <provider>    refresh just that one");
    eprintln!("  phantom models test                  probe ALL provider default_models for");
    eprintln!("                                       real tool-call support (vs. text-only fake)");
    eprintln!("  phantom models test <provider>           probe that provider's default_model");
    eprintln!("  phantom models test <provider>:<model>   probe a specific model");
    eprintln!();
    eprintln!("test is the truth check for hallucinating models — sends a request that");
    eprintln!("explicitly demands a `shell` tool call, then checks whether the response");
    eprintln!("includes a real tool_calls block or just text pretending to call it.");
}

/// Resolve `(provider_type, base_url, api_key)` for one [providers.X] block,
/// or describe what's missing. Mirrors the resolution the TUI's /model fetch
/// path uses so cache refreshes hit the same endpoint as user-driven fetches.
fn resolve_provider_call_params(
    name: &str,
    ent: &crate::config::ProviderEntry,
) -> Result<(String, String, String), String> {
    let key = ent.api_key.clone().filter(|s| !s.is_empty())
        .or_else(|| ent.api_key_env.as_ref()
            .and_then(|v| std::env::var(v).ok())
            .filter(|s| !s.is_empty()));
    let url = ent.url.clone()
        .or_else(|| crate::keys::default_provider_meta(name).map(|(_, u)| u.to_string()))
        .filter(|s| !s.is_empty());
    let ptype = if ent.provider_type.is_empty() {
        crate::keys::default_provider_meta(name).map(|(t, _)| t.to_string())
            .unwrap_or_else(|| name.to_string())
    } else {
        ent.provider_type.clone()
    };
    match (key, url) {
        (None, _) => Err(format!("no key for {} (set via `phantom keys set {} <key>`)", name, name)),
        (_, None) => Err(format!("no base url for {} and no default known", name)),
        (Some(k), Some(u)) => Ok((ptype, u, k)),
    }
}

/// Refresh one or all providers in the cache. Returns one line per provider
/// describing the outcome (✓ count or ✗ reason). When `only` is None,
/// iterates every [providers.X] block in agents.toml in alphabetical order.
pub async fn models_refresh_lines(only: Option<&str>) -> anyhow::Result<Vec<String>> {
    let path = agents_toml_path().ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let raw = fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;
    let cfg: AgentsConfig = toml::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("parse {}: {}", path.display(), e))?;

    let mut targets: Vec<&String> = match only {
        Some(name) => {
            if !cfg.providers.contains_key(name) {
                anyhow::bail!(
                    "no [providers.{}] block in {}\n  configured: {:?}",
                    name, path.display(),
                    cfg.providers.keys().collect::<Vec<_>>()
                );
            }
            vec![cfg.providers.keys().find(|k| k.as_str() == name).unwrap()]
        }
        None => {
            let mut v: Vec<&String> = cfg.providers.keys().collect();
            v.sort();
            v
        }
    };
    targets.sort();

    let mut out = Vec::new();
    out.push(format!("Refreshing model cache → {}", path.display()));
    if let Some(cp) = crate::models_cache::cache_path() {
        out.push(format!("  cache: {}", cp.display()));
    }
    out.push(String::new());

    for name in targets {
        let ent = &cfg.providers[name];
        match resolve_provider_call_params(name, ent) {
            Err(why) => {
                out.push(format!("  ✗ {:<14} skipped — {}", name, why));
            }
            Ok((ptype, url, key)) => {
                match crate::models_cache::refresh_provider(name, &ptype, &url, &key).await {
                    Ok(models) => {
                        let n_free = models.iter().filter(|m| m.is_free).count();
                        out.push(format!(
                            "  ✓ {:<14} {} models  ({} free · {} paid)",
                            name, models.len(), n_free, models.len() - n_free
                        ));
                    }
                    Err(e) => {
                        // Don't abort the loop — one bad key shouldn't block
                        // refreshing the other providers. Operators usually
                        // run `refresh` on a multi-provider config and want
                        // every other entry to still get updated.
                        out.push(format!("  ✗ {:<14} fetch failed — {}", name, e));
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Report current cache state: per provider, model count, freshness vs TTL,
/// and human-readable age. Sorted alphabetically. Empty cache → one-line hint.
pub fn models_status_lines() -> anyhow::Result<Vec<String>> {
    let cache = crate::models_cache::read_cache();
    let cache_path = crate::models_cache::cache_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<no $HOME>".into());

    let mut out = Vec::new();
    if cache.providers.is_empty() {
        out.push(format!("(empty cache at {})", cache_path));
        out.push("  populate via: phantom models refresh".into());
        return Ok(out);
    }

    out.push(format!("Models cache → {}", cache_path));
    out.push(format!("  TTL: {}m (entries older than this are refetched on next /models call)",
        crate::models_cache::DEFAULT_TTL_MS / 60_000));
    out.push(String::new());

    let now = crate::models_cache::now_ms();
    let mut names: Vec<&String> = cache.providers.keys().collect();
    names.sort();
    for name in names {
        let entry = &cache.providers[name];
        let age_ms = now.saturating_sub(entry.fetched_at_ms);
        let n_free = entry.models.iter().filter(|m| m.is_free).count();
        let stale = age_ms > crate::models_cache::DEFAULT_TTL_MS;
        let status = if stale { "stale" } else { "fresh" };
        out.push(format!(
            "  {} {:<14} {} models  ({} free · {} paid)  [{}, {}]",
            if stale { "⚠" } else { "✓" },
            name,
            entry.models.len(),
            n_free,
            entry.models.len() - n_free,
            status,
            human_age(age_ms),
        ));
    }
    Ok(out)
}

/// Result of probing one (provider, model) for real tool-calling behavior.
/// Distinguishes the three cases the user actually cares about:
///   - tool_calls block came back → model honors function calling ✓
///   - response was text-only → model ignored tools=[…] (lies) ✗
///   - HTTP/auth/transport failure → can't tell ⚠
#[derive(Debug)]
enum ToolProbe {
    Called { tool_names: Vec<String>, snippet: String },
    TextOnly { snippet: String },
    Error(String),
}

/// Send a deterministic prompt that explicitly demands a `shell` tool call,
/// then inspect the response for a real `tool_calls` block. Returns a tagged
/// result so the caller can render a row consistent with the rest of the
/// `phantom models` output.
///
/// Important: this only works for OpenAI-compatible endpoints (opencode,
/// openai, openrouter, groq, deepseek, mistral, together, cerebras, nvidia
/// nim). Anthropic uses a different request shape (`content` blocks +
/// `tool_use`); skipped for v1 to keep this short. Anthropic provider type
/// returns Error("anthropic — not implemented in models test yet, use openai-compat models").
async fn probe_tool_capability(
    ptype: &str,
    base_url: &str,
    model: &str,
    api_key: &str,
) -> ToolProbe {
    if ptype.eq_ignore_ascii_case("anthropic") {
        return ToolProbe::Error(
            "anthropic provider uses a different request shape; not yet probed".into()
        );
    }

    let body = serde_json::json!({
        "model": model,
        "messages": [
            {
                "role": "system",
                "content": "You are a tool-use tester. When the user asks you to call a tool, you MUST emit a real function/tool call. Do not roleplay calling it — actually call it.",
            },
            {
                "role": "user",
                "content": "Call the shell tool with command 'echo TOOL-USE-CONFIRMED-9971'. Reply ONLY by invoking the function — no narration.",
            }
        ],
        "tools": [{
            "type": "function",
            "function": {
                "name": "shell",
                "description": "Execute a shell command and return its stdout.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": {"type": "string", "description": "command to run"}
                    },
                    "required": ["command"]
                }
            }
        }],
        "tool_choice": "auto",
        "max_tokens": 200,
        "stream": false,
    });

    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => return ToolProbe::Error(format!("client build: {}", e)),
    };
    let resp = match client.post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return ToolProbe::Error(format!("post {}: {}", url, e)),
    };
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        // Truncate body so a long HTML 502 page doesn't blow up the row.
        let trimmed: String = text.chars().take(160).collect();
        return ToolProbe::Error(format!("HTTP {} — {}", status.as_u16(), trimmed));
    }
    let json: serde_json::Value = match serde_json::from_str(&text) {
        Ok(j) => j,
        Err(e) => return ToolProbe::Error(format!("non-JSON response: {} (body head: {})",
            e, text.chars().take(120).collect::<String>())),
    };

    let msg = &json["choices"][0]["message"];
    if let Some(arr) = msg.get("tool_calls").and_then(|v| v.as_array()) {
        if !arr.is_empty() {
            let names: Vec<String> = arr.iter()
                .filter_map(|tc| tc.get("function").and_then(|f| f.get("name")).and_then(|n| n.as_str()).map(String::from))
                .collect();
            // Capture the args for the first call as a sanity check that
            // the model actually formed a structured call (not a stub).
            let first_args = arr.first()
                .and_then(|tc| tc.get("function").and_then(|f| f.get("arguments")))
                .map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))
                .unwrap_or_default();
            let snippet: String = first_args.chars().take(120).collect();
            return ToolProbe::Called { tool_names: names, snippet };
        }
    }

    // No tool_calls block — surface the first 120 chars of text content as
    // diagnostic so the user can see WHAT the model did instead. (This is
    // usually the smoking gun: "✅ I ran shell and got TOOL-USE-CONFIRMED-9971"
    // — pure fabrication with no underlying call.)
    let content_text = msg.get("content")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| msg.get("content").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>().join(" ")
        }))
        .unwrap_or_default();
    let snippet: String = content_text.chars().take(120).collect();
    ToolProbe::TextOnly { snippet }
}

/// Walk providers from agents.toml, probe each (provider, model) for real
/// tool-call execution, return one row per probe. `target` filters:
///   None              → every provider's default_model
///   Some("opencode")  → just that provider's default_model
///   Some("opencode:claude-haiku-4-5") → just that exact provider+model pair
pub async fn models_test_lines(target: Option<&str>) -> anyhow::Result<Vec<String>> {
    let path = agents_toml_path().ok_or_else(|| anyhow::anyhow!("no $HOME"))?;
    let raw = fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;
    let cfg: AgentsConfig = toml::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("parse {}: {}", path.display(), e))?;

    // Build the (provider_name, model_to_probe) work list.
    let mut work: Vec<(String, String)> = Vec::new();
    match target {
        None => {
            let mut names: Vec<&String> = cfg.providers.keys().collect();
            names.sort();
            for name in names {
                let ent = &cfg.providers[name];
                let model = ent.default_model.clone().unwrap_or_default();
                if model.is_empty() {
                    work.push((name.clone(), String::new())); // marker for "no default model"
                } else {
                    work.push((name.clone(), model));
                }
            }
        }
        Some(t) => {
            // "<provider>" or "<provider>:<model>" form
            let (pname, model_opt) = match t.split_once(':') {
                Some((p, m)) => (p.to_string(), Some(m.to_string())),
                None => (t.to_string(), None),
            };
            if !cfg.providers.contains_key(&pname) {
                anyhow::bail!(
                    "no [providers.{}] block in {}\n  configured: {:?}",
                    pname, path.display(),
                    cfg.providers.keys().collect::<Vec<_>>()
                );
            }
            let model = model_opt
                .unwrap_or_else(|| cfg.providers[&pname].default_model.clone().unwrap_or_default());
            if model.is_empty() {
                anyhow::bail!(
                    "no model to probe for {} (provider has no default_model — pass <provider>:<model>)",
                    pname
                );
            }
            work.push((pname, model));
        }
    }

    let mut out = Vec::new();
    out.push("Probing tool-call behavior — sends one request per model that demands a shell call.".into());
    out.push("Looks for a real tool_calls block in the response (not text claiming success).".into());
    out.push(String::new());

    // Sequential, not parallel — clearer diagnostics, and free-tier rate
    // limits are often per-second so concurrent probes can spuriously 429.
    for (pname, model) in work {
        if model.is_empty() {
            out.push(format!("  ⚠ {:<14} skipped — no default_model and no <provider>:<model> arg", pname));
            continue;
        }
        let ent = &cfg.providers[&pname];
        let (ptype, url, key) = match resolve_provider_call_params(&pname, ent) {
            Ok(t) => t,
            Err(why) => { out.push(format!("  ⚠ {:<14} {} — {}", pname, model, why)); continue; }
        };
        let probe = probe_tool_capability(&ptype, &url, &model, &key).await;
        match probe {
            ToolProbe::Called { tool_names, snippet } => {
                let names = tool_names.join(", ");
                out.push(format!(
                    "  ✓ {:<14} {} → CALLED [{}]",
                    pname, model, names
                ));
                out.push(format!("      args: {}", snippet));
            }
            ToolProbe::TextOnly { snippet } => {
                let collapsed: String = snippet.chars().filter(|c| *c != '\n' && *c != '\r').collect();
                out.push(format!(
                    "  ✗ {:<14} {} → TEXT-ONLY (no tool_calls — model is hallucinating completion)",
                    pname, model
                ));
                out.push(format!("      said: {}", collapsed));
            }
            ToolProbe::Error(e) => {
                out.push(format!("  ⚠ {:<14} {} → {}", pname, model, e));
            }
        }
    }
    Ok(out)
}

/// "37s ago", "12m ago", "3h ago", "2d ago". Coarse on purpose — stamping
/// to the second on a cache that updates hourly is noise.
fn human_age(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60        { format!("{}s ago", s) }
    else if s < 3600 { format!("{}m ago", s / 60) }
    else if s < 86_400 { format!("{}h ago", s / 3600) }
    else             { format!("{}d ago", s / 86_400) }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_env_var_name_common_aliases() {
        assert_eq!(provider_env_var_name("groq"),       "GROQ_API_KEY");
        assert_eq!(provider_env_var_name("Groq"),       "GROQ_API_KEY");
        assert_eq!(provider_env_var_name("CEREBRAS"),   "CEREBRAS_API_KEY");
        assert_eq!(provider_env_var_name("opencode"),   "OPENCODE_API_KEY");
        assert_eq!(provider_env_var_name("nvidia"),     "NVIDIA_NIM_API_KEY");
        assert_eq!(provider_env_var_name("nvidia_nim"), "NVIDIA_NIM_API_KEY");
        assert_eq!(provider_env_var_name("nim"),        "NVIDIA_NIM_API_KEY");
        // Hyphen → underscore
        assert_eq!(provider_env_var_name("local-ollama"), "LOCAL_OLLAMA_API_KEY");
    }

    #[test]
    fn mask_key_short_and_long() {
        assert_eq!(mask_key(""), "(empty)");
        assert_eq!(mask_key("abc"), "abc…");
        assert_eq!(mask_key("sk-wrdrOfAFtwq"), "sk-wrd…");
        assert_eq!(mask_key("gsk_yQFf12345"), "gsk_yQ…");
    }

    #[test]
    fn env_file_roundtrip() {
        let dir = std::env::temp_dir().join(format!("phantom-test-env-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("env");
        let mut vars = HashMap::new();
        vars.insert("GROQ_API_KEY".to_string(), "gsk_test123".to_string());
        vars.insert("CEREBRAS_API_KEY".to_string(), "csk-quoted value with space".to_string());
        write_env_file(&path, &vars).unwrap();
        let parsed = read_env_file(&path);
        assert_eq!(parsed.get("GROQ_API_KEY"), Some(&"gsk_test123".to_string()));
        assert_eq!(
            parsed.get("CEREBRAS_API_KEY"),
            Some(&"csk-quoted value with space".to_string())
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_file_skips_blanks_and_comments() {
        let dir = std::env::temp_dir().join(format!("phantom-test-env-comments-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("env");
        fs::write(&path,
            "# comment line\n\
             \n\
             GROQ_API_KEY=ok\n\
             # another comment\n\
             malformed line without equals\n\
             CEREBRAS_API_KEY=also ok\n"
        ).unwrap();
        let parsed = read_env_file(&path);
        assert_eq!(parsed.len(), 2);
        assert!(parsed.contains_key("GROQ_API_KEY"));
        assert!(parsed.contains_key("CEREBRAS_API_KEY"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn auto_load_does_not_overwrite_existing_env() {
        let dir = std::env::temp_dir().join(format!("phantom-test-env-noclobber-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("env");
        fs::write(&path, "PHANTOM_TEST_NOCLOBBER=from_file\n").unwrap();
        std::env::set_var("PHANTOM_TEST_NOCLOBBER", "from_shell");

        // Use read_env_file directly since auto_load_env hardcodes the home path.
        let vars = read_env_file(&path);
        for (k, v) in vars {
            if std::env::var(&k).is_err() {
                std::env::set_var(&k, v);
            }
        }
        // Shell value must still win.
        assert_eq!(std::env::var("PHANTOM_TEST_NOCLOBBER").ok(), Some("from_shell".into()));
        std::env::remove_var("PHANTOM_TEST_NOCLOBBER");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn human_age_buckets() {
        assert_eq!(human_age(0), "0s ago");
        assert_eq!(human_age(45_000), "45s ago");
        assert_eq!(human_age(90_000), "1m ago");
        assert_eq!(human_age(60 * 60 * 1000), "1h ago");
        assert_eq!(human_age(3 * 60 * 60 * 1000), "3h ago");
        assert_eq!(human_age(2 * 24 * 60 * 60 * 1000), "2d ago");
    }

    #[test]
    fn resolve_provider_call_params_uses_env_var_then_default_url() {
        // ent missing url → falls back to default_provider_meta("groq").1
        let mut ent = crate::config::ProviderEntry::default();
        ent.provider_type = "groq".into();
        ent.api_key_env = Some("PHANTOM_TEST_RESOLVE_KEY".into());
        std::env::set_var("PHANTOM_TEST_RESOLVE_KEY", "gsk_resolved_from_env");
        let (ptype, url, key) = resolve_provider_call_params("groq", &ent).unwrap();
        assert_eq!(ptype, "groq");
        assert!(url.starts_with("https://api.groq.com"),
            "expected default base url for groq, got: {}", url);
        assert_eq!(key, "gsk_resolved_from_env");
        std::env::remove_var("PHANTOM_TEST_RESOLVE_KEY");
    }

    #[test]
    fn resolve_provider_call_params_missing_key_returns_err() {
        let mut ent = crate::config::ProviderEntry::default();
        ent.provider_type = "groq".into();
        ent.api_key_env = Some("PHANTOM_TEST_DEFINITELY_NOT_SET".into());
        std::env::remove_var("PHANTOM_TEST_DEFINITELY_NOT_SET");
        let err = resolve_provider_call_params("groq", &ent).unwrap_err();
        assert!(err.contains("no key"), "expected no-key err, got: {}", err);
    }

    #[test]
    fn resolve_provider_call_params_inline_key_wins_over_env() {
        let mut ent = crate::config::ProviderEntry::default();
        ent.provider_type = "groq".into();
        ent.api_key = Some("inline_key".into());
        ent.api_key_env = Some("PHANTOM_TEST_SHOULD_BE_IGNORED".into());
        std::env::set_var("PHANTOM_TEST_SHOULD_BE_IGNORED", "env_key_loses");
        let (_, _, key) = resolve_provider_call_params("groq", &ent).unwrap();
        assert_eq!(key, "inline_key");
        std::env::remove_var("PHANTOM_TEST_SHOULD_BE_IGNORED");
    }
}
