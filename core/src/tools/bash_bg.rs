//! Background bash-job tools: `bash_run_background`, `bash_output`, `bash_kill`.
//!
//! Spawns shell commands and tracks them in a process-wide registry so callers
//! can poll output and kill them later via opaque UUID handles.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

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
    pub status: BgStatus,
    /// Held in an Arc<Mutex<Option<Child>>> so the kill path can take it.
    pub child: Arc<Mutex<Option<Child>>>,
    pub pid: Option<u32>,
}

type Registry = Mutex<HashMap<String, Arc<Mutex<BgJob>>>>;

fn registry() -> &'static Registry {
    static REG: OnceLock<Registry> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

fn new_handle() -> String {
    // Lightweight UUID-ish handle: 32 hex chars from rand-ish source.
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let mix = nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(pid);
    format!("bg-{:016x}-{:016x}", nanos & 0xFFFF_FFFF_FFFF_FFFF, mix & 0xFFFF_FFFF_FFFF_FFFF)
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
        status: BgStatus::Running,
        child: Arc::new(Mutex::new(Some(child))),
        pid,
    }));

    if let Ok(mut reg) = registry().lock() {
        reg.insert(handle.clone(), Arc::clone(&job));
    }

    // Spawn collector tasks for stdout/stderr.
    if let Some(mut s) = stdout {
        let job_clone = Arc::clone(&job);
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                match s.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut j) = job_clone.lock() {
                            j.output.extend_from_slice(&buf[..n]);
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
                            j.output.extend_from_slice(&buf[..n]);
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
    let since_byte = args
        .get("since_byte")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

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
}
