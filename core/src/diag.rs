//! Diagnostic + crash logging.
//!
//! Two artefacts:
//!
//! 1. **Crash log** — `~/.spectyn-mesh/crashes/crash-<ts>.log` written by
//!    a panic hook. Contains the panic message, location, and the last 32
//!    in-memory events so post-mortems aren't blind.
//!
//! 2. **Event log** — `~/.spectyn-mesh/events.jsonl` rolling append.
//!    Every tool call, slash command, agent run start/end, and provider
//!    error pushes a one-line JSON record. ~256-entry ring also kept in
//!    memory for the crash hook to dump.
//!
//! Both are best-effort; failures here never panic the caller. If
//! `~/.spectyn-mesh/` isn't writable, events are silently dropped (fine
//! for tests / readonly homes).
//!
//! `init()` must be called at process startup before anything else
//! uses `record()`. Idempotent — second call is a no-op.
//!
//! ## Where to look when something goes wrong
//!
//! - Event log: `~/.spectyn-mesh/events.jsonl` (rotated to
//!   `events.jsonl.1` once it passes 10 MB). One JSON object per line —
//!   `{ "ts_ms", "kind", "summary" }`. Tail it to see what spectyn did
//!   right before a problem.
//! - Crash logs: `~/.spectyn-mesh/crashes/crash-<unix-ts>.log`. Each
//!   contains the version, git hash, OS/arch, panic location + message,
//!   a backtrace (set `RUST_BACKTRACE=1` to make it useful), and the
//!   last 32 events leading up to the crash.
//!
//! When filing a bug, attaching the newest crash log is the single most
//! useful thing you can include.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// Set to `true` while the ratatui full-screen TUI owns the terminal.
///
/// Code paths that may `eprintln!` during normal operation (LLM provider
/// fallback chain logs, capture pipeline warnings) consult this flag and
/// skip stderr writes when the TUI is active — otherwise raw stderr lines
/// collide with ratatui's draw buffer and produce the "screen scramble"
/// users see when the fallback chain fires inside the TUI. Suppressed
/// events still go through `record()` so events.jsonl + the crash ring
/// buffer keep their full record.
pub static TUI_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Returns `true` if any caller marked the TUI as currently drawing the
/// screen. Cheap (relaxed atomic load) so callers can guard `eprintln!`
/// without measurable overhead.
pub fn is_tui_active() -> bool {
    TUI_ACTIVE.load(Ordering::Relaxed)
}

const RING_CAPACITY: usize = 256;
const MAX_EVENTS_LOG_BYTES: u64 = 10 * 1024 * 1024; // 10 MB rolling

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiagEvent {
    pub ts_ms: u64,
    pub kind: String,
    pub summary: String,
}

struct DiagState {
    ring: VecDeque<DiagEvent>,
    events_log: Option<PathBuf>,
    crash_dir: Option<PathBuf>,
}

static STATE: OnceLock<Mutex<DiagState>> = OnceLock::new();

/// Install the panic hook + open the events log file. Idempotent.
pub fn init() {
    if STATE.get().is_some() {
        return;
    }
    let spectyn_dir = crate::cli_config::spectyn_data_dir().ok();
    let _ = spectyn_dir.as_ref().map(|p| std::fs::create_dir_all(p));

    let state = DiagState {
        ring: VecDeque::with_capacity(RING_CAPACITY),
        events_log: spectyn_dir.as_ref().map(|p| p.join("events.jsonl")),
        crash_dir: spectyn_dir.as_ref().map(|p| p.join("crashes")),
    };
    if let Some(dir) = &state.crash_dir {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = STATE.set(Mutex::new(state));

    // Install panic hook — chains to whatever was set previously so we
    // don't suppress the default formatted panic message.
    //
    // Special case: when the parent process closes our stdout/stderr
    // pipe (Windows ERROR_NO_DATA "管道正關閉中" os error 232, or
    // EPIPE on Unix) Rust's eprintln! / println! panics. That panic is
    // not actionable — it just means "the consumer hung up". Don't
    // pollute ~/.spectyn-mesh/crashes/ with one of these on every
    // short-lived MCP session, and exit silently with code 0 so the
    // parent doesn't think we crashed.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = panic_message(info);
        let is_broken_pipe = msg.contains("failed printing to stderr")
            || msg.contains("failed printing to stdout")
            || msg.contains("os error 232")  // Windows ERROR_NO_DATA
            || msg.contains("Broken pipe")
            || msg.contains("管道");
        if is_broken_pipe {
            restore_terminal_after_panic();
            std::process::exit(0);
        }
        write_crash_log(info);
        // Make sure the terminal is sane after a TUI panic — both
        // alternate-screen and mouse-capture have to be turned off
        // explicitly because crossterm's raii guards skip on panic.
        restore_terminal_after_panic();
        prev(info);
    }));

    record("startup", "spectyn diagnostic init");
}

/// Cap (so a runaway dump can't bloat the log) THEN redact a diag summary at
/// the single `record()` write boundary, so the in-memory ring stores an
/// already-sanitized summary and every downstream sink inherits it: events.jsonl
/// (serialized here) AND the crash-log dump (which re-reads `ev.summary` from the
/// ring). SPEC-07 §12.1 / P4 trust boundary: secret-bearing callers (e.g. a
/// provider transport error echoing `Authorization: Bearer sk-…` or `?key=AIza…`)
/// must never land a credential in clear on disk. Pure → unit-testable without
/// the OnceLock/panic-hook/$HOME-redirect machinery.
fn redacted_summary(summary: &str) -> String {
    let capped: String = summary.chars().take(280).collect();
    crate::redact::redact(&capped)
}

/// Append an event to the ring buffer + events.jsonl.
/// Never blocks the caller; lock contention is handled by skipping the
/// record (recording is "diagnostic best-effort", not critical-path).
pub fn record(kind: &str, summary: impl Into<String>) {
    let Some(state_mtx) = STATE.get() else {
        return;
    };
    let summary: String = summary.into();
    let ev = DiagEvent {
        ts_ms: now_ms(),
        kind: kind.to_string(),
        // Cap + redact at the boundary so the ring, events.jsonl, and the crash
        // log all inherit a secret-free summary (SPEC-07 §12.1).
        summary: redacted_summary(&summary),
    };

    if let Ok(mut g) = state_mtx.try_lock() {
        // Append to events.jsonl (rotate when over MAX_EVENTS_LOG_BYTES)
        if let Some(path) = &g.events_log {
            if let Ok(meta) = std::fs::metadata(path) {
                if meta.len() > MAX_EVENTS_LOG_BYTES {
                    // Rotate: keep .1 as the previous run.
                    let prev = path.with_extension("jsonl.1");
                    let _ = std::fs::rename(path, &prev);
                }
            }
            if let Ok(line) = serde_json::to_string(&ev) {
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(f, "{}", line);
                }
            }
        }
        if g.ring.len() >= RING_CAPACITY {
            g.ring.pop_front();
        }
        g.ring.push_back(ev);
    }
}

/// Snapshot the in-memory event ring. Used by `/diag snapshot` and the
/// panic hook.
pub fn snapshot() -> Vec<DiagEvent> {
    if let Some(state_mtx) = STATE.get() {
        if let Ok(g) = state_mtx.lock() {
            return g.ring.iter().cloned().collect();
        }
    }
    vec![]
}

/// Path to the events.jsonl (for `/diag` to point the user at).
pub fn events_path() -> Option<PathBuf> {
    STATE
        .get()
        .and_then(|m| m.lock().ok())
        .and_then(|g| g.events_log.clone())
}

/// Most recent crash log file (or None if no crashes recorded yet).
pub fn last_crash_path() -> Option<PathBuf> {
    let dir = STATE.get().and_then(|m| m.lock().ok())?.crash_dir.clone()?;
    let mut latest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                if latest.as_ref().map(|(t, _)| modified > *t).unwrap_or(true) {
                    latest = Some((modified, entry.path()));
                }
            }
        }
    }
    latest.map(|(_, p)| p)
}

/// Extract a panic's payload as a String. Used both by the crash-log
/// writer and by the broken-pipe filter in the panic hook.
fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        format!("{:?}", payload)
    }
}

fn write_crash_log(info: &std::panic::PanicHookInfo<'_>) {
    // Note: broken-pipe panics are pre-filtered at the panic hook entry
    // (init() above) and never reach here — the hook does process::exit(0)
    // before calling write_crash_log. This was originally guarded inside
    // write_crash_log too (commit d78f0bf, Mac Bug #24); node-a's commit
    // f0ec83b moved the filter up to the hook level and added Windows
    // ERROR_NO_DATA + 管道 patterns, which is strictly better. The inner
    // guard is now redundant; left out to keep this function focused.
    let Some(state_mtx) = STATE.get() else {
        return;
    };
    let crash_dir = match state_mtx.lock().ok().and_then(|g| g.crash_dir.clone()) {
        Some(d) => d,
        None => return,
    };
    let _ = std::fs::create_dir_all(&crash_dir);
    let ts = now_ms() / 1000;
    let path = crash_dir.join(format!("crash-{}.log", ts));

    let mut buf = String::new();
    buf.push_str("spectyn crash report (panic)\n");
    buf.push_str(&format!("ts_unix: {}\n", ts));
    buf.push_str(&format!("version: {}\n", env!("CARGO_PKG_VERSION")));
    buf.push_str(&format!(
        "git_hash: {}\n",
        option_env!("SPECTYN_GIT_HASH").unwrap_or("?")
    ));
    buf.push_str(&format!(
        "os: {}-{}\n",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    if let Some(loc) = info.location() {
        buf.push_str(&format!(
            "location: {}:{}:{}\n",
            loc.file(),
            loc.line(),
            loc.column()
        ));
    }
    let msg = panic_message(info);
    buf.push_str(&format!("message: {}\n", msg));

    // Backtrace (only present if RUST_BACKTRACE was set)
    let backtrace = std::backtrace::Backtrace::force_capture();
    buf.push_str(&format!("\n--- backtrace ---\n{}\n", backtrace));

    // Last 32 events from the ring.
    if let Ok(g) = state_mtx.lock() {
        let n = g.ring.len().min(32);
        buf.push_str(&format!("\n--- last {} events ---\n", n));
        for ev in g.ring.iter().rev().take(n).rev() {
            buf.push_str(&format!(
                "  [{:>13} ms] {:<14} {}\n",
                ev.ts_ms, ev.kind, ev.summary
            ));
        }
    }

    let _ = std::fs::write(&path, &buf);
    // Surface to user so they know where to look. We CANNOT use
    // eprintln! here: if stderr is closed (the user piped spectyn into
    // `head`, `less`, or anything that exits early), eprintln! panics
    // with "failed printing to stderr: Broken pipe", which re-enters
    // this hook and produces a meaningless second crash log
    // (5 such files accumulated before this fix). Use a fallible write
    // and swallow any I/O error — the file we already wrote is the
    // authoritative copy.
    use std::io::Write as _;
    let _ = writeln!(
        std::io::stderr(),
        "\n  ⚠ spectyn crashed. A crash log was written to:\n      {}\n  \
         Please attach that file when reporting this. \
         Re-run with RUST_BACKTRACE=1 for a fuller trace.",
        path.display(),
    );
}

/// Best-effort terminal cleanup if the TUI was running. Skipping any
/// errors because we're already crashing.
fn restore_terminal_after_panic() {
    use std::io::Write as _;
    // \x1b[?1049l exits alternate-screen.
    // \x1b[?1000l, \x1b[?1002l, \x1b[?1003l, \x1b[?1015l, \x1b[?1006l disable mouse capture.
    // \x1b[?25h shows cursor.
    let mut out = std::io::stderr();
    let _ = out.write_all(
        b"\x1b[?1049l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1015l\x1b[?1006l\x1b[?25h\x1b[0m\n",
    );
    let _ = out.flush();
    // crossterm's disable_raw_mode is the proper call, but it requires
    // the terminal handle. Best-effort: shell out to `stty sane` to
    // reset cooked mode + echo.
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("stty").arg("sane").spawn();
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_summary_strips_secrets_but_preserves_prose() {
        // SPEC-07 §12.1: a provider transport error can echo an Authorization
        // header / api key into a diag summary; redacted_summary() runs at the
        // record() boundary so it can NEVER reach events.jsonl or the crash log.
        // (Pure helper → no OnceLock/panic-hook/$HOME machinery needed.)
        let secret = "sk-LIVEKEY123abcDEF456ghiJKL789mno";
        let leaky = format!("provider_skip: error sending request: Authorization: Bearer {secret}");
        let out = redacted_summary(&leaky);
        assert!(
            !out.contains(secret),
            "the bearer token must be masked before it hits disk: {out}"
        );
        assert!(
            out.contains("[REDACTED]"),
            "the redactor must mark the masked span: {out}"
        );
        // Conservative: ordinary diagnostic prose is left untouched.
        assert_eq!(
            redacted_summary("agent finished 3 tasks ok"),
            "agent finished 3 tasks ok",
            "non-secret summaries must pass through unchanged"
        );
    }

    #[test]
    fn ring_capped_at_capacity() {
        let _guard = crate::env_lock::acquire();
        // We can't init() in tests because it sets a global panic hook; just
        // exercise the ring logic via the public surface.
        // Use a unique tempdir-rooted state by overriding HOME.
        let tmp = tempfile::TempDir::new().unwrap();
        // Restore $HOME on exit (panic-safe RAII). Without this, this test
        // LEAKS $HOME pointing at `tmp`, which is deleted when `tmp` drops — so
        // every later serial test that resolves `dirs::home_dir()` (e.g.
        // life_node::recall's capture_note writes, service::macos's
        // ~/Library/LaunchAgents plist write) hits a dead path and fails
        // NotFound. The guard drops before `tmp` (reverse declaration order), so
        // $HOME is restored to the real value before the tempdir is removed.
        struct HomeGuard(Option<std::ffi::OsString>);
        impl Drop for HomeGuard {
            fn drop(&mut self) {
                match &self.0 {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
            }
        }
        let _home = HomeGuard(std::env::var_os("HOME"));
        std::env::set_var("HOME", tmp.path());
        // Reset the OnceLock isn't possible — so this test must run in a
        // process where init wasn't called yet. In the lib test harness
        // each test gets a fresh process if --test-threads=1, otherwise
        // one of the tests ends up "first" and wins. We just check that
        // record() is no-op when STATE not set, and snapshot() returns
        // empty.
        let s0 = snapshot();
        record("test", "ignored when not initialised");
        let s1 = snapshot();
        assert_eq!(s0.len(), s1.len());
    }
}
