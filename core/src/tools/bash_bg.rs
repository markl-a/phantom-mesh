//! Background bash-job tools: `bash_run_background`, `bash_output`, `bash_kill`.
//!
//! Spawns shell commands and tracks them in a process-wide registry so callers
//! can poll output and kill them later via opaque UUID handles.
//!
//! ## [T58] Lifecycle hygiene (audit M-3 + M-4)
//!
//! Pre-fix the registry was an append-only `HashMap<String, Arc<Mutex<BgJob>>>`
//! and handles were derived from `SystemTime::now()` × constant + PID, so a
//! peer agent could enumerate live handles and the registry grew unbounded.
//! Now:
//!
//! - **Handles** are `uuid::Uuid::new_v4()` — 122 bits of cryptographic random;
//!   unguessable without crashing the registry first.
//! - **Per-job output buffer** is capped at `MAX_OUTPUT_BYTES` (1 MiB). When
//!   the limit is reached, further bytes are dropped silently except for a
//!   single `[T58 truncated …]` marker emitted at the cap boundary. Status
//!   surfaces a `truncated: true` flag.
//! - **Registry size** is capped at `MAX_REGISTRY_SIZE` (100 jobs). When the
//!   cap is hit, the oldest *finished* job is evicted; if every entry is
//!   still running we evict the oldest by `started` timestamp regardless.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use uuid::Uuid;

/// Maximum bytes retained per background job. Beyond this the collector
/// drops further output and surfaces `truncated: true` in `bash_output`.
/// 1 MiB is enough for thousands of lines of typical CI output yet keeps
/// 100 long-running jobs under 100 MiB total RSS in the worst case.
pub const MAX_OUTPUT_BYTES: usize = 1 << 20; // 1 MiB

/// Maximum number of jobs (running + finished) the global registry retains.
/// When the cap is reached the oldest *finished* job is evicted; if every
/// entry is still running we evict the oldest by `started` timestamp.
pub const MAX_REGISTRY_SIZE: usize = 100;

/// Marker appended once when an output buffer first hits `MAX_OUTPUT_BYTES`.
/// Keep this an ASCII byte-string so `String::from_utf8_lossy` round-trips.
const TRUNC_MARKER: &[u8] = b"\n[T58 truncated: output exceeded 1 MiB cap]\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BgStatus {
    Running,
    Exited(i32),
    Killed,
}

pub struct BgJob {
    pub command: String,
    pub started: SystemTime,
    pub output: Vec<u8>,
    /// [T58 M-4] true once the output buffer hit `MAX_OUTPUT_BYTES` and the
    /// collector started dropping further bytes. Sticky; never cleared.
    pub truncated: bool,
    pub status: BgStatus,
    /// Held in an Arc<Mutex<Option<Child>>> so the kill path can take it.
    pub child: Arc<Mutex<Option<Child>>>,
    pub pid: Option<u32>,
}

/// Append bytes to a bounded job buffer. [T58 M-4] — pre-fix the buffer was
/// an unbounded `Vec<u8>` and a chatty subprocess could OOM the host.
fn append_bounded(job: &mut BgJob, bytes: &[u8]) {
    if job.truncated {
        return;
    }
    let remaining = MAX_OUTPUT_BYTES.saturating_sub(job.output.len());
    if bytes.len() <= remaining {
        job.output.extend_from_slice(bytes);
        return;
    }
    // Take whatever fits, then emit the marker once and flip the flag.
    if remaining > 0 {
        job.output.extend_from_slice(&bytes[..remaining]);
    }
    job.output.extend_from_slice(TRUNC_MARKER);
    job.truncated = true;
}

type Registry = Mutex<HashMap<String, Arc<Mutex<BgJob>>>>;

fn registry() -> &'static Registry {
    static REG: OnceLock<Registry> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// [T58 M-3] Unguessable background-job handle.
///
/// Pre-fix the handle was `bg-<nanos>-<mix>` where `<mix> = nanos * K + pid`
/// — invertible and enumerable by anyone who could see PIDs (everyone on
/// Linux). A peer agent calling `bash_output` could iterate plausible
/// timestamps and probe other agents' jobs.
///
/// `Uuid::new_v4()` draws from the OS CSPRNG (122 bits of entropy). Even
/// if the entire `Vec<u8>` of live handles leaked, a guess has ~`2^-122`
/// chance of colliding with a real one — security-equivalent to the
/// session tokens spectyn already uses.
fn new_handle() -> String {
    format!("bg-{}", Uuid::new_v4())
}

/// [T58 M-4] Evict the oldest finished job (or, if every entry is still
/// running, the oldest by `started` timestamp) when the registry hits its
/// cap. Called once per `run_background` before insertion.
///
/// Eviction is best-effort: a poisoned mutex on a job entry just skips that
/// entry rather than aborting the whole sweep. We hold the registry lock for
/// the entire scan to avoid TOCTOU between "I observed N entries" and "I
/// inserted one more."
fn evict_one(reg: &mut HashMap<String, Arc<Mutex<BgJob>>>) {
    if reg.len() < MAX_REGISTRY_SIZE {
        return;
    }

    // Snapshot (handle, status, started) for every entry. Skip poisoned jobs.
    let mut snapshot: Vec<(String, BgStatus, SystemTime)> = Vec::with_capacity(reg.len());
    for (k, v) in reg.iter() {
        if let Ok(j) = v.lock() {
            snapshot.push((k.clone(), j.status, j.started));
        }
    }

    // Prefer finished jobs (Exited / Killed). Within each preference tier,
    // oldest `started` wins.
    snapshot.sort_by(|a, b| {
        let a_done = !matches!(a.1, BgStatus::Running);
        let b_done = !matches!(b.1, BgStatus::Running);
        match (a_done, b_done) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.2.cmp(&b.2),
        }
    });

    if let Some((handle, _, _)) = snapshot.first() {
        reg.remove(handle);
    }
}

fn build_shell_command(cmd: &str) -> Command {
    // Delegate to `platform::shell_command` so the same sandbox-exec
    // wrapping (macOS) and shell selection (sh / cmd.exe) used by the
    // foreground `shell` tool also covers background jobs. Earlier
    // duplication here meant the macOS sandbox was bypassed for
    // `bash_run_background` calls — fixed by consolidating on the
    // central platform helper.
    crate::platform::shell_command(cmd)
}

// ── bash_run_background ───────────────────────────────────────────────────

pub async fn run_background(args: &Value) -> String {
    let command = match args.get("command").and_then(|v| v.as_str()) {
        Some(c) if !c.is_empty() => c.to_string(),
        _ => return "ERROR: missing required parameter 'command'".to_string(),
    };

    // Audit C-1 + M-5: enforce the same control-char and blocklist gate as
    // `shell::run`. Pre-fix, `bash_run_background` had **no** blocklist —
    // a model could sidestep `shell` entirely by routing destructive
    // commands through this background path.
    if let Err(e) = crate::tools::validate::reject_dangerous_chars(&command) {
        return format!("{}", json!({"error": e}));
    }
    if let Some(pat) =
        crate::tools::validate::match_blocklist(&command, crate::tools::shell::blocked_patterns())
    {
        return format!(
            "{}",
            json!({"error": format!("blocked command pattern '{}'", pat)})
        );
    }
    if command.contains("$(") || command.contains('`') {
        return format!(
            "{}",
            json!({"error": "subshell / backtick substitution is not allowed"})
        );
    }

    // [C7/T76] V9 H-3: enforce the same `requires_confirmation` approval gate
    // as the foreground `shell::run` path. Pre-fix, `bash_run_background`
    // skipped this check entirely — `rm -rf foo` was blocked in foreground
    // but allowed in background (a model could simply route destructive
    // commands through `bash_run_background` to bypass approval).
    //
    // Honors the same `SPECTYN_AUTO_APPROVE=1` escape hatch as `shell::run`
    // for non-interactive CI/agent workflows. Returns the {"error": "..."}
    // JSON shape to match the rest of this entry point's contract.
    if let Some(reason) = crate::tools::shell::requires_confirmation(&command) {
        if std::env::var("SPECTYN_AUTO_APPROVE").as_deref() != Ok("1") {
            return format!(
                "{}",
                json!({
                    "error": format!(
                        "APPROVAL_REQUIRED: command '{}' matches pattern '{}'. \
                         Set SPECTYN_AUTO_APPROVE=1 or explicitly confirm to proceed.",
                        command, reason
                    )
                })
            );
        }
        tracing::warn!(
            "SPECTYN_AUTO_APPROVE active — bash_run_background executing potentially dangerous command: '{}'",
            command
        );
    }

    let cwd = args.get("cwd").and_then(|v| v.as_str()).map(String::from);
    let timeout_secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(600);

    let mut shell_cmd = build_shell_command(&command);
    shell_cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    if let Some(ref dir) = cwd {
        let p = std::path::Path::new(dir);
        if !p.exists() || !p.is_dir() {
            return format!(
                "{}",
                json!({"error": format!("cwd '{}' does not exist or is not a directory", dir)})
            );
        }
        shell_cmd.current_dir(dir);
    }

    let mut child = match shell_cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return format!("{}", json!({"error": format!("failed to spawn: {}", e)}));
        }
    };

    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let handle = new_handle();
    let job = Arc::new(Mutex::new(BgJob {
        command: command.clone(),
        started: SystemTime::now(),
        output: Vec::new(),
        truncated: false,
        status: BgStatus::Running,
        child: Arc::new(Mutex::new(Some(child))),
        pid,
    }));

    if let Ok(mut reg) = registry().lock() {
        // [T58 M-4] Evict the oldest finished job (or oldest running, if all
        // entries are still live) BEFORE insertion so the registry never
        // exceeds MAX_REGISTRY_SIZE.
        evict_one(&mut reg);
        reg.insert(handle.clone(), Arc::clone(&job));
    }

    // Spawn collector tasks for stdout/stderr. [T58 M-4] each `append_bounded`
    // call enforces the per-job MAX_OUTPUT_BYTES ring-cap.
    if let Some(mut s) = stdout {
        let job_clone = Arc::clone(&job);
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                match s.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut j) = job_clone.lock() {
                            append_bounded(&mut j, &buf[..n]);
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }
    if let Some(mut s) = stderr {
        let job_clone = Arc::clone(&job);
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                match s.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut j) = job_clone.lock() {
                            append_bounded(&mut j, &buf[..n]);
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // Spawn waiter that records the exit status (and enforces timeout).
    let job_for_wait = Arc::clone(&job);
    tokio::spawn(async move {
        // Take the child out for the wait.
        let child_opt = {
            let child_arc = {
                let g = match job_for_wait.lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };
                Arc::clone(&g.child)
            };
            let mut c = match child_arc.lock() {
                Ok(c) => c,
                Err(_) => return,
            };
            c.take()
        };
        let mut child = match child_opt {
            Some(c) => c,
            None => return, // already killed/taken
        };

        let wait_fut = child.wait();
        let result = tokio::time::timeout(Duration::from_secs(timeout_secs), wait_fut).await;

        match result {
            Ok(Ok(status)) => {
                if let Ok(mut j) = job_for_wait.lock() {
                    if j.status == BgStatus::Running {
                        j.status = BgStatus::Exited(status.code().unwrap_or(-1));
                    }
                }
            }
            Ok(Err(_)) => {
                if let Ok(mut j) = job_for_wait.lock() {
                    if j.status == BgStatus::Running {
                        j.status = BgStatus::Exited(-1);
                    }
                }
            }
            Err(_) => {
                // Timeout — kill the child.
                let _ = child.start_kill();
                let _ = child.wait().await;
                if let Ok(mut j) = job_for_wait.lock() {
                    if j.status == BgStatus::Running {
                        j.status = BgStatus::Killed;
                    }
                }
            }
        }
    });

    json!({
        "handle": handle,
        "pid": pid.unwrap_or(0),
    })
    .to_string()
}

// ── bash_output ───────────────────────────────────────────────────────────

pub async fn output(args: &Value) -> String {
    let handle = match args.get("handle").and_then(|v| v.as_str()) {
        Some(h) => h.to_string(),
        None => return "ERROR: missing required parameter 'handle'".to_string(),
    };
    let since_byte = args.get("since_byte").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    let job_arc = {
        let reg = match registry().lock() {
            Ok(r) => r,
            Err(_) => return json!({"error": "registry poisoned"}).to_string(),
        };
        match reg.get(&handle) {
            Some(j) => Arc::clone(j),
            None => return json!({"error": format!("unknown handle '{}'", handle)}).to_string(),
        }
    };

    let job = match job_arc.lock() {
        Ok(j) => j,
        Err(_) => return json!({"error": "job lock poisoned"}).to_string(),
    };

    let total_bytes = job.output.len();
    let start = since_byte.min(total_bytes);
    let slice = &job.output[start..];
    let text = String::from_utf8_lossy(slice).to_string();

    let (status_str, exit_code): (&str, Value) = match job.status {
        BgStatus::Running => ("running", Value::Null),
        BgStatus::Exited(code) => ("exited", json!(code)),
        BgStatus::Killed => ("killed", Value::Null),
    };

    json!({
        "output": text,
        "status": status_str,
        "exit_code": exit_code,
        "total_bytes": total_bytes,
        // [T58 M-4] surface whether the buffer was capped so the model knows
        // it cannot rely on bytes past MAX_OUTPUT_BYTES being present.
        "truncated": job.truncated,
    })
    .to_string()
}

// ── bash_kill ─────────────────────────────────────────────────────────────

pub async fn kill(args: &Value) -> String {
    let handle = match args.get("handle").and_then(|v| v.as_str()) {
        Some(h) => h.to_string(),
        None => return json!({"killed": false, "reason": "missing handle"}).to_string(),
    };

    let job_arc = {
        let reg = match registry().lock() {
            Ok(r) => r,
            Err(_) => return json!({"killed": false, "reason": "registry poisoned"}).to_string(),
        };
        match reg.get(&handle) {
            Some(j) => Arc::clone(j),
            None => {
                return json!({"killed": false, "reason": "handle not found"}).to_string();
            }
        }
    };

    // Snapshot status & take child out (if still present).
    let (already_done, child_arc) = {
        let j = match job_arc.lock() {
            Ok(j) => j,
            Err(_) => {
                return json!({"killed": false, "reason": "job lock poisoned"}).to_string();
            }
        };
        (j.status != BgStatus::Running, Arc::clone(&j.child))
    };

    if already_done {
        return json!({"killed": false, "reason": "already exited"}).to_string();
    }

    // Try to take the Child handle and kill it. If it's None, the waiter
    // already has it — fall back to OS-level kill via the recorded PID.
    let child_taken = {
        let mut c = match child_arc.lock() {
            Ok(c) => c,
            Err(_) => {
                return json!({"killed": false, "reason": "child lock poisoned"}).to_string();
            }
        };
        c.take()
    };

    if let Some(mut child) = child_taken {
        let _ = child.start_kill();
        let _ = child.wait().await;
    } else {
        // Fall back to OS kill.
        let pid = match job_arc.lock() {
            Ok(j) => j.pid,
            Err(_) => None,
        };
        if let Some(p) = pid {
            #[cfg(windows)]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(["/F", "/PID", &p.to_string()])
                    .output();
            }
            #[cfg(not(windows))]
            {
                let _ = std::process::Command::new("kill")
                    .args(["-TERM", &p.to_string()])
                    .output();
            }
        }
    }

    if let Ok(mut j) = job_arc.lock() {
        j.status = BgStatus::Killed;
    }

    json!({"killed": true}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).expect("valid json")
    }

    #[tokio::test]
    async fn bg_lifecycle_short_command() {
        let res = run_background(&json!({"command": "echo hello-bg"})).await;
        let v = parse(&res);
        let handle = v["handle"].as_str().unwrap().to_string();
        assert!(handle.starts_with("bg-"));

        // Give it a moment to finish.
        tokio::time::sleep(Duration::from_millis(1000)).await;

        let out_res = output(&json!({"handle": handle})).await;
        let out_v = parse(&out_res);
        let combined = out_v["output"].as_str().unwrap_or("");
        assert!(combined.contains("hello-bg"), "got: {}", combined);
        // Status may already be exited.
        let status = out_v["status"].as_str().unwrap_or("");
        assert!(status == "exited" || status == "running", "got: {}", status);
    }

    #[cfg(unix)] // uses POSIX sleep 30; Windows cmd lacks it (process exits before kill)
    #[tokio::test]
    async fn bg_kill_running_job() {
        let res = run_background(&json!({"command": "sleep 30"})).await;
        let handle = parse(&res)["handle"].as_str().unwrap().to_string();
        // Brief pause to let it actually start.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let kr = kill(&json!({"handle": handle.clone()})).await;
        let kv = parse(&kr);
        assert_eq!(kv["killed"].as_bool(), Some(true));

        // Killing twice → "already exited" / killed.
        let kr2 = kill(&json!({"handle": handle})).await;
        let kv2 = parse(&kr2);
        assert_eq!(kv2["killed"].as_bool(), Some(false));
    }

    #[tokio::test]
    async fn bg_unknown_handle() {
        let r = output(&json!({"handle": "bg-does-not-exist"})).await;
        assert!(r.contains("error") || r.contains("unknown"));
        let k = kill(&json!({"handle": "bg-does-not-exist"})).await;
        let kv = parse(&k);
        assert_eq!(kv["killed"].as_bool(), Some(false));
    }

    #[tokio::test]
    async fn bg_since_byte_works() {
        let res = run_background(&json!({"command": "echo 0123456789"})).await;
        let handle = parse(&res)["handle"].as_str().unwrap().to_string();
        tokio::time::sleep(Duration::from_millis(1000)).await;
        let r = output(&json!({"handle": handle, "since_byte": 5})).await;
        let v = parse(&r);
        let out = v["output"].as_str().unwrap_or("");
        // "0123456789\n" — bytes 5+ are "56789\n"
        assert!(out.contains("56789"), "got: {}", out);
        assert!(!out.contains("01234"), "got: {}", out);
    }

    #[tokio::test]
    async fn bg_missing_command() {
        let r = run_background(&json!({})).await;
        assert!(r.starts_with("ERROR:"), "got: {}", r);
    }

    // ── [T58] M-3 + M-4 lifecycle hygiene regression tests ───────────────

    /// [T58 M-3] Handles must come from a CSPRNG, not the nanos-mix scheme.
    /// We assert the post-`bg-` portion parses as a standard UUID v4
    /// (hyphenated 8-4-4-4-12) and that two consecutive handles have wildly
    /// different bit patterns (the old scheme produced near-identical
    /// timestamps for back-to-back calls).
    #[tokio::test]
    async fn handle_uses_uuid_v4_format() {
        let r1 = run_background(&json!({"command": "echo a"})).await;
        let r2 = run_background(&json!({"command": "echo b"})).await;
        let h1 = parse(&r1)["handle"].as_str().unwrap().to_string();
        let h2 = parse(&r2)["handle"].as_str().unwrap().to_string();

        // Format check: "bg-XXXXXXXX-XXXX-4XXX-YXXX-XXXXXXXXXXXX" (39 chars total)
        // where the 14th-after-prefix nibble is '4' (UUID v4 marker) and the
        // 19th-after-prefix nibble is one of 8/9/a/b (RFC 4122 variant).
        assert!(h1.starts_with("bg-"), "missing prefix: {}", h1);
        let uuid_part = &h1[3..];
        let parsed = uuid::Uuid::parse_str(uuid_part)
            .unwrap_or_else(|e| panic!("handle is not a valid UUID: {} ({})", h1, e));
        assert_eq!(
            parsed.get_version(),
            Some(uuid::Version::Random),
            "handle is not UUID v4: {}",
            h1
        );

        // Distinctness: two back-to-back handles must NOT share their first
        // 8 hex chars (probability of accidental collision: 2^-32).
        assert_ne!(
            &h1[3..11],
            &h2[3..11],
            "handles too similar — predictable? {} vs {}",
            h1,
            h2
        );
    }

    /// [T58 M-4] The per-job output buffer must stop growing past
    /// MAX_OUTPUT_BYTES + TRUNC_MARKER.len(). Without the cap, a chatty
    /// subprocess could grow the buffer to GB scale and OOM the host.
    #[tokio::test]
    async fn output_buffer_capped_at_max() {
        // Forge a job entry directly — much faster than spawning a real
        // subprocess that prints a megabyte.
        let job = Arc::new(Mutex::new(BgJob {
            command: "synthetic".into(),
            started: SystemTime::now(),
            output: Vec::new(),
            truncated: false,
            status: BgStatus::Running,
            child: Arc::new(Mutex::new(None)),
            pid: None,
        }));

        // Append 3 MiB worth of data in 64 KiB chunks.
        let chunk = vec![b'x'; 64 * 1024];
        for _ in 0..48 {
            let mut j = job.lock().unwrap();
            append_bounded(&mut j, &chunk);
        }

        let j = job.lock().unwrap();
        assert!(j.truncated, "truncated flag never set");
        // Buffer must not exceed the cap plus a single marker.
        assert!(
            j.output.len() <= MAX_OUTPUT_BYTES + TRUNC_MARKER.len(),
            "buffer leaked past cap: {} > {} + {}",
            j.output.len(),
            MAX_OUTPUT_BYTES,
            TRUNC_MARKER.len()
        );
        // Marker must be present somewhere in the tail.
        let tail = &j.output[j.output.len().saturating_sub(TRUNC_MARKER.len())..];
        assert_eq!(tail, TRUNC_MARKER, "trunc marker missing");
    }

    // ── [C7/T76] V9 H-3 approval-gate parity tests ───────────────────────


    /// V9 H-3 regression: `bash_run_background` must refuse commands
    /// matching `requires_confirmation` (e.g. `rm`) when
    /// `SPECTYN_AUTO_APPROVE` is not set. Pre-fix this would silently
    /// spawn — letting a model route `rm -rf foo` (not blocklisted) past
    /// the approval gate by going through the background path.
    #[tokio::test]
    async fn bg_requires_confirmation_blocks_rm() {
        let _g = crate::sandbox::test_lock();
        std::env::remove_var("SPECTYN_AUTO_APPROVE");
        let r = run_background(&json!({"command": "rm -rf foo"})).await;
        let v = parse(&r);
        let err = v["error"].as_str().unwrap_or("");
        assert!(
            err.contains("APPROVAL_REQUIRED"),
            "expected APPROVAL_REQUIRED error, got: {}",
            r
        );
    }

    /// V9 H-3 regression: harmless commands must still spawn successfully
    /// — the gate is an approval check, not a deny-all.
    #[tokio::test]
    async fn bg_safe_command_still_runs() {
        let _g = crate::sandbox::test_lock();
        std::env::remove_var("SPECTYN_AUTO_APPROVE");
        let r = run_background(&json!({"command": "echo hi"})).await;
        let v = parse(&r);
        assert!(
            v.get("handle").and_then(|h| h.as_str()).is_some(),
            "echo should produce a handle, got: {}",
            r
        );
        assert!(
            v.get("error").is_none(),
            "echo should not be gated, got: {}",
            r
        );
    }

    /// `SPECTYN_AUTO_APPROVE=1` must let confirmation-required commands
    /// through (CI / non-interactive workflows rely on this escape hatch).
    #[tokio::test]
    async fn bg_auto_approve_bypasses_gate() {
        let _g = crate::sandbox::test_lock();
        std::env::set_var("SPECTYN_AUTO_APPROVE", "1");
        // Use a path that almost certainly doesn't exist; we only need the
        // gate to NOT fire — we don't care whether the spawn ultimately
        // succeeds or fails, just that we got a handle (gate passed).
        let r = run_background(&json!({"command": "rm /tmp/__c7_t76_nonexistent__"})).await;
        std::env::remove_var("SPECTYN_AUTO_APPROVE");
        let v = parse(&r);
        let err = v.get("error").and_then(|e| e.as_str()).unwrap_or("");
        assert!(
            !err.contains("APPROVAL_REQUIRED"),
            "auto-approve should bypass the gate, got: {}",
            r
        );
    }

    /// [T58 M-4] The global registry must cap at MAX_REGISTRY_SIZE. We can
    /// test the eviction policy directly without spawning 100 subprocesses
    /// — it's a pure function over the HashMap.
    #[tokio::test]
    async fn registry_evicts_when_full() {
        let mut reg: HashMap<String, Arc<Mutex<BgJob>>> = HashMap::new();
        // Fill registry to capacity. Mix of finished + running so the
        // policy has both tiers to choose from. First N/2 are finished
        // (oldest started), next N/2 are running.
        for i in 0..MAX_REGISTRY_SIZE {
            let status = if i < MAX_REGISTRY_SIZE / 2 {
                BgStatus::Exited(0)
            } else {
                BgStatus::Running
            };
            let job = Arc::new(Mutex::new(BgJob {
                command: format!("job-{}", i),
                started: SystemTime::UNIX_EPOCH + Duration::from_secs(i as u64),
                output: Vec::new(),
                truncated: false,
                status,
                child: Arc::new(Mutex::new(None)),
                pid: None,
            }));
            reg.insert(format!("bg-test-{:032x}", i), job);
        }
        assert_eq!(reg.len(), MAX_REGISTRY_SIZE);

        // Eviction MUST reduce size by exactly one — and the victim must
        // be a finished job (the oldest one — index 0).
        evict_one(&mut reg);
        assert_eq!(reg.len(), MAX_REGISTRY_SIZE - 1);
        // The "job-0" entry — oldest finished — should be gone.
        let surviving_cmds: Vec<String> = reg
            .values()
            .filter_map(|v| v.lock().ok().map(|j| j.command.clone()))
            .collect();
        assert!(
            !surviving_cmds.contains(&"job-0".to_string()),
            "oldest finished job was not evicted; survivors: {:?}",
            surviving_cmds
        );

        // Under-capacity → no-op.
        let before = reg.len();
        evict_one(&mut reg);
        assert_eq!(reg.len(), before, "evict_one ran when under capacity");
    }
}
