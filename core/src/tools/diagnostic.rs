use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

const TIMEOUT_SECS: u64 = 120;

// ── Shared helper ──────────────────────────────────────────────────────────

/// Run a command, capturing combined stdout+stderr, with a 120s timeout.
async fn run_cmd(
    program: &str,
    args: &[&str],
    cwd: Option<&str>,
) -> Result<(String, bool), String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn '{}': {}", program, e))?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let collect: tokio::task::JoinHandle<(Vec<u8>, Vec<u8>)> = tokio::spawn(async move {
        let mut out = Vec::new();
        let mut err = Vec::new();
        if let Some(mut p) = stdout_pipe {
            let _ = p.read_to_end(&mut out).await;
        }
        if let Some(mut p) = stderr_pipe {
            let _ = p.read_to_end(&mut err).await;
        }
        (out, err)
    });

    match tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), child.wait()).await {
        Ok(Ok(status)) => {
            let (out, err) = collect.await.unwrap_or_default();
            let mut combined = String::from_utf8_lossy(&out).into_owned();
            let stderr_str = String::from_utf8_lossy(&err).into_owned();
            if !stderr_str.is_empty() {
                if !combined.is_empty() {
                    combined.push('\n');
                }
                combined.push_str(&stderr_str);
            }
            Ok((combined, status.success()))
        }
        Ok(Err(e)) => Err(format!("Process error: {}", e)),
        Err(_) => {
            collect.abort();
            Err(format!("Command timed out after {}s", TIMEOUT_SECS))
        }
    }
}

// ── 1. cargo_check ─────────────────────────────────────────────────────────

/// Tool name: "cargo_check"
/// Params: path (manifest dir, default "."), package (optional)
pub async fn cargo_check(args: &Value) -> String {
    let path = args["path"].as_str().unwrap_or(".");
    let package = args["package"].as_str();

    let mut cmd_args: Vec<&str> = vec!["check", "--message-format=short"];
    let pkg_flag;
    if let Some(pkg) = package {
        pkg_flag = pkg.to_string();
        cmd_args.push("--package");
        cmd_args.push(&pkg_flag);
    }

    let (output, success) = match run_cmd("cargo", &cmd_args, Some(path)).await {
        Ok(r) => r,
        Err(e) => {
            // Retry without --message-format=short in case cargo version doesn't support it
            let mut fallback_args: Vec<&str> = vec!["check"];
            if let Some(pkg) = package {
                fallback_args.push("--package");
                fallback_args.push(pkg);
            }
            match run_cmd("cargo", &fallback_args, Some(path)).await {
                Ok(r) => r,
                Err(_) => return format!("Error running cargo check: {}", e),
            }
        }
    };

    if success {
        // Count warnings
        let warning_count = output
            .lines()
            .filter(|l| l.contains("warning:") || l.contains("warning["))
            .count();
        format!("✓ cargo check passed ({} warnings)", warning_count)
    } else {
        // Extract error lines
        let errors: Vec<&str> = output
            .lines()
            .filter(|l| l.contains("error") || l.contains("warning"))
            .collect();

        if errors.is_empty() {
            crate::tools::truncate(format!("cargo check failed:\n{}", output), 5000)
        } else {
            let summary = errors.join("\n");
            crate::tools::truncate(format!("cargo check failed:\n{}", summary), 5000)
        }
    }
}

// ── 2. cargo_test ──────────────────────────────────────────────────────────

/// Tool name: "cargo_test"
/// Params: path, filter (test name filter), package
pub async fn cargo_test(args: &Value) -> String {
    let path = args["path"].as_str().unwrap_or(".");
    let filter = args["filter"].as_str();
    let package = args["package"].as_str();

    let mut cmd_args: Vec<String> = vec!["test".to_string()];

    if let Some(pkg) = package {
        cmd_args.push("--package".to_string());
        cmd_args.push(pkg.to_string());
    }

    if let Some(f) = filter {
        cmd_args.push(f.to_string());
    }

    cmd_args.push("--".to_string());
    cmd_args.push("--nocapture".to_string());

    let args_refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();

    let (output, success) = match run_cmd("cargo", &args_refs, Some(path)).await {
        Ok(r) => r,
        Err(e) => return format!("Error running cargo test: {}", e),
    };

    // Parse test results
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for line in output.lines() {
        if line.starts_with("test ") && line.ends_with(" ... ok") {
            passed += 1;
        } else if line.starts_with("test ") && line.ends_with(" ... FAILED") {
            failed += 1;
            failures.push(line.to_string());
        } else if line.contains("test result:") {
            // e.g. "test result: ok. 5 passed; 0 failed; ..."
            // already summarized below
        }
    }

    let summary = if success {
        format!("{} passed, {} failed", passed, failed)
    } else {
        let failure_detail = if failures.is_empty() {
            output.clone()
        } else {
            failures.join("\n")
        };
        format!(
            "{} passed, {} failed\nFailed tests:\n{}",
            passed, failed, failure_detail
        )
    };

    crate::tools::truncate(summary, 3000)
}

// ── 3. tsc_check ──────────────────────────────────────────────────────────

/// Tool name: "tsc_check"
/// Params: path (project dir), config (tsconfig path)
pub async fn tsc_check(args: &Value) -> String {
    let path = args["path"].as_str().unwrap_or(".");
    let config = args["config"].as_str();

    // Try `tsc` first, fall back to `npx tsc`
    let tsc_available = run_cmd("tsc", &["--version"], None)
        .await
        .map(|(_, ok)| ok)
        .unwrap_or(false);

    let program = if tsc_available { "tsc" } else { "npx" };

    let mut cmd_args: Vec<String> = if tsc_available {
        vec!["--noEmit".to_string()]
    } else {
        vec!["tsc".to_string(), "--noEmit".to_string()]
    };

    if let Some(cfg) = config {
        cmd_args.push("--project".to_string());
        cmd_args.push(cfg.to_string());
    }

    let args_refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();

    let (output, success) = match run_cmd(program, &args_refs, Some(path)).await {
        Ok(r) => r,
        Err(e) => return format!("Error running tsc: {}", e),
    };

    if success && output.trim().is_empty() {
        "✓ TypeScript check passed".to_string()
    } else if success {
        format!("✓ TypeScript check passed\n{}", output.trim())
    } else {
        let errors: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();

        if errors.is_empty() {
            "TypeScript check failed (no output)".to_string()
        } else {
            crate::tools::truncate(format!("TypeScript errors:\n{}", errors.join("\n")), 5000)
        }
    }
}

// ── 4. run_tests ──────────────────────────────────────────────────────────

/// Tool name: "run_tests"
/// Params: command (required), path (optional working dir)
pub async fn run_tests(args: &Value) -> String {
    let command = match args["command"].as_str() {
        Some(c) if !c.is_empty() => c,
        _ => return "Error: missing 'command' argument".to_string(),
    };
    let path = args["path"].as_str().unwrap_or(".");

    // Split into program + args using shell-like splitting
    let parts: Vec<&str> = command.splitn(2, ' ').collect();
    let program = parts[0];
    let rest: Vec<&str> = if parts.len() > 1 {
        parts[1].split_whitespace().collect()
    } else {
        vec![]
    };

    let (output, success) = match run_cmd(program, &rest, Some(path)).await {
        Ok(r) => r,
        Err(e) => return format!("Error running '{}': {}", command, e),
    };

    let status_line = if success {
        "Tests completed successfully."
    } else {
        "Tests finished with failures."
    };

    let body = if output.trim().is_empty() {
        "(no output)".to_string()
    } else {
        output
    };

    crate::tools::truncate(format!("{}\n{}", status_line, body), 5000)
}

// ── 5. dev_verify (anti-fake-pass gate) ─────────────────────────────────────

/// Heuristic: does this command rely on shell features (pipes, `&&`, redirects,
/// quoting, globs, env-expansion)? If so the naive whitespace split mangles it,
/// so we run it through `sh -c` / `cmd /C` instead.
fn needs_shell(cmd: &str) -> bool {
    const META: [char; 11] = ['|', '&', ';', '>', '<', '"', '\'', '$', '*', '`', '('];
    cmd.chars().any(|c| META.contains(&c))
}

/// Like `run_cmd`, but returns the REAL exit code (not just success bool) and
/// takes a caller-chosen timeout (a full `cargo build` blows past run_cmd's 120s).
/// When `shell` is true the command is handed verbatim to `sh -c` (unix) /
/// `cmd /C` (windows) so pipes/`&&`/quoting work; otherwise it is whitespace-split.
async fn run_cmd_rc(
    command: &str,
    shell: bool,
    cwd: Option<&str>,
    timeout_secs: u64,
    env: &[(String, String)],
) -> Result<(String, i32), String> {
    let mut cmd = if shell {
        if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(command);
            c
        } else {
            let mut c = Command::new("sh");
            c.arg("-c").arg(command);
            c
        }
    } else {
        let parts: Vec<&str> = command.splitn(2, ' ').collect();
        let mut c = Command::new(parts[0]);
        if parts.len() > 1 {
            c.args(parts[1].split_whitespace());
        }
        c
    };
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    if !env.is_empty() {
        cmd.envs(env.iter().cloned());
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn '{}': {}", command, e))?;
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let collect: tokio::task::JoinHandle<(Vec<u8>, Vec<u8>)> = tokio::spawn(async move {
        let mut out = Vec::new();
        let mut err = Vec::new();
        if let Some(mut p) = stdout_pipe {
            let _ = p.read_to_end(&mut out).await;
        }
        if let Some(mut p) = stderr_pipe {
            let _ = p.read_to_end(&mut err).await;
        }
        (out, err)
    });
    match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait()).await {
        Ok(Ok(status)) => {
            let (out, err) = collect.await.unwrap_or_default();
            let mut combined = String::from_utf8_lossy(&out).into_owned();
            let stderr_str = String::from_utf8_lossy(&err).into_owned();
            if !stderr_str.is_empty() {
                if !combined.is_empty() {
                    combined.push('\n');
                }
                combined.push_str(&stderr_str);
            }
            Ok((combined, status.code().unwrap_or(-1)))
        }
        Ok(Err(e)) => Err(format!("Process error: {}", e)),
        Err(_) => {
            collect.abort();
            Err(format!("Command timed out after {}s", timeout_secs))
        }
    }
}

/// Write the full captured output to a temp log so a "pass" claim is auditable.
fn write_verify_log(label: &str, content: &str) -> String {
    let safe: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("spectyn-verify-{}-{}.log", safe, ts));
    let _ = std::fs::write(&path, content);
    path.to_string_lossy().into_owned()
}

// ── artifact integrity (optional, post-exit) ────────────────────────────────

/// What artifact (if any) a verify run should additionally check AFTER the build
/// command exits 0. Default (`None`) keeps `dev_verify` behaviour byte-identical.
#[derive(Clone)]
struct ArtifactCheck {
    path: String,
    /// Minimum byte size the artifact must reach (0 = no size floor).
    min_bytes: u64,
    /// Also run `<path> --help` and assert it does not crash (exit, any code).
    run_help: bool,
}

/// Verify a built artifact exists, meets a size floor, and (optionally) runs
/// `--help` without crashing. Returns a structured `{checked, exists, size,
/// size_ok, ok, (help_ok, help_exit)}` value. `ok` is the AND of the checks that
/// were requested. Pure-ish: only filesystem metadata + an optional subprocess.
async fn verify_artifact(check: &ArtifactCheck) -> serde_json::Value {
    let meta = std::fs::metadata(&check.path);
    let exists = meta.is_ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let size_ok = exists && size >= check.min_bytes;

    let mut v = serde_json::json!({
        "checked": true,
        "path": check.path,
        "exists": exists,
        "size": size,
        "min_bytes": check.min_bytes,
        "size_ok": size_ok,
    });

    let mut ok = size_ok;
    if check.run_help {
        // "不崩" = ran to completion (we got an exit code), regardless of the
        // code itself — many tools exit non-zero on `--help`. A spawn failure or
        // timeout is the only "crash" we fail on. Only attempt if the file exists.
        let help_ok = if exists {
            match run_cmd_rc(
                &format!("{} --help", check.path),
                false,
                None,
                30,
                &[],
            )
            .await
            {
                Ok((_, code)) => {
                    v["help_exit"] = serde_json::json!(code);
                    true
                }
                Err(e) => {
                    v["help_error"] = serde_json::json!(e);
                    false
                }
            }
        } else {
            false
        };
        v["help_ok"] = serde_json::json!(help_ok);
        ok = ok && help_ok;
    }

    v["ok"] = serde_json::json!(ok);
    v
}

// ── verdict building, structured counts, ledger ─────────────────────────────

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// `~/.spectyn-mesh` (parent of agents.toml) — home for the verify ledger.
fn spectyn_home() -> Option<std::path::PathBuf> {
    crate::cli_config::agents_toml_path().and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

/// Best-effort short git commit of `cwd`, so the ledger ties a verdict to code.
fn git_short_commit(cwd: &str) -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(cwd)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Grab the integer token immediately preceding `kw` on a line, e.g.
/// `"12 passed"` with kw "passed" -> 12; `"due to 5 previous errors"` with
/// kw "previous error" -> 5. Returns 0 when not found.
fn num_before(line: &str, kw: &str) -> u64 {
    match line.find(kw) {
        Some(idx) => line[..idx]
            .split_whitespace()
            .last()
            .and_then(|t| t.parse().ok())
            .unwrap_or(0),
        None => 0,
    }
}

/// Sum `cargo test` result lines into (passed, failed, ignored). None if no
/// `test result:` line is present (e.g. a `cargo check`/build command).
fn parse_test_counts(output: &str) -> Option<(u64, u64, u64)> {
    let mut found = false;
    let (mut p, mut f, mut i) = (0u64, 0u64, 0u64);
    for line in output.lines().filter(|l| l.contains("test result:")) {
        found = true;
        p += num_before(line, "passed");
        f += num_before(line, "failed");
        i += num_before(line, "ignored");
    }
    found.then_some((p, f, i))
}

/// Warning count: prefer cargo's "generated N warning(s)" totals; else count
/// raw `warning:`-prefixed lines.
fn parse_warning_count(output: &str) -> u64 {
    let generated: u64 = output
        .lines()
        .filter(|l| l.contains("generated") && l.contains("warning"))
        .map(|l| num_before(l, "warning"))
        .sum();
    if generated > 0 {
        return generated;
    }
    output
        .lines()
        .filter(|l| l.trim_start().starts_with("warning:"))
        .count() as u64
}

/// Compiler error count from "could not compile ... due to N previous error(s)".
fn parse_compile_errors(output: &str) -> Option<u64> {
    output
        .lines()
        .find(|l| l.contains("could not compile") && l.contains("due to"))
        .map(|l| num_before(l, "previous error"))
}

/// Append one audit line to `~/.spectyn-mesh/verify-log.jsonl` so the history
/// of "what was green at which commit" survives across runs.
fn append_ledger(command: &str, label: &str, cwd: &str, passed: bool, exit_code: i32, summary: &str) {
    let dir = match spectyn_home() {
        Some(d) => d,
        None => return,
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("verify-log.jsonl");
    let line = serde_json::json!({
        "ts": now_ms(),
        "command": command,
        "label": label,
        "cwd": cwd,
        "passed": passed,
        "exit_code": exit_code,
        "summary": summary,
        "commit": git_short_commit(cwd),
    })
    .to_string();
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{}", line);
    }
}

/// Turn captured output + exit code into the structured anti-fake-pass verdict.
/// Writes the full log, structured counts, and appends the audit ledger.
/// `output_tail_bytes > 0` embeds the last N bytes of combined output in the
/// verdict (`output_tail`) so a remote caller can actually READ what happened
/// without fetching the peer's log file — essential for autonomous remote
/// discovery/debugging.
async fn build_verdict(
    command: &str,
    label: &str,
    cwd: &str,
    output: &str,
    exit_code: i32,
    output_tail_bytes: usize,
    artifact: Option<&ArtifactCheck>,
) -> serde_json::Value {
    let log_path = write_verify_log(label, output);
    let exit_passed = exit_code == 0;
    // Backward-compatible: with no artifact requested, `passed` is exactly the
    // process's `exit_code == 0` (unchanged). When an artifact IS requested, the
    // build is only "passed" if it both exited 0 AND the artifact checks pass —
    // a green exit that produced no/under-size binary is not a real pass.
    let mut passed = exit_passed;

    // Best-effort one-line summary: last "test result:" line, else first error.
    let summary = output
        .lines()
        .rev()
        .find(|l| l.contains("test result:"))
        .or_else(|| {
            output
                .lines()
                .find(|l| l.contains("error[") || l.starts_with("error:"))
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| {
            if exit_passed {
                "ok".into()
            } else {
                "non-zero exit, no recognizable summary".into()
            }
        });

    let failed: Vec<String> = output
        .lines()
        .filter(|l| l.starts_with("test ") && l.ends_with(" ... FAILED"))
        .map(|l| {
            l.trim_start_matches("test ")
                .trim_end_matches(" ... FAILED")
                .trim()
                .to_string()
        })
        .collect();

    // Optional artifact integrity check (only meaningful once the build exited 0;
    // a failed build's artifact is irrelevant). When requested, fold `artifact.ok`
    // into the final `passed` so a green-but-no-binary run is caught.
    let artifact_verdict = match artifact {
        Some(a) if exit_passed => {
            let av = verify_artifact(a).await;
            passed = passed && av["ok"].as_bool().unwrap_or(false);
            Some(av)
        }
        _ => None,
    };

    append_ledger(command, label, cwd, passed, exit_code, &summary);

    let mut verdict = serde_json::json!({
        "passed": passed,
        "exit_code": exit_code,
        "summary": summary,
        "failed": failed,
        "warnings": parse_warning_count(output),
        "log_path": log_path,
        "command": command,
    });
    if let Some(av) = artifact_verdict {
        verdict["artifact"] = av;
    }
    if let Some((p, f, i)) = parse_test_counts(output) {
        verdict["passed_count"] = serde_json::json!(p);
        verdict["failed_count"] = serde_json::json!(f);
        verdict["ignored_count"] = serde_json::json!(i);
    }
    if let Some(ec) = parse_compile_errors(output) {
        verdict["error_count"] = serde_json::json!(ec);
    }
    if output_tail_bytes > 0 && !output.is_empty() {
        let (tail, truncated) = if output.len() <= output_tail_bytes {
            (output, false)
        } else {
            // Start at a char boundary at or after (len - output_tail_bytes).
            let mut start = output.len() - output_tail_bytes;
            while start < output.len() && !output.is_char_boundary(start) {
                start += 1;
            }
            (&output[start..], true)
        };
        verdict["output_tail"] = serde_json::json!(tail);
        verdict["output_truncated"] = serde_json::json!(truncated);
    }
    verdict
}

// ── background jobs ─────────────────────────────────────────────────────────

struct JobEntry {
    status: &'static str, // "running" | "done"
    started_ms: u128,
    command: String,
    verdict: Option<String>, // full verdict JSON once done
}

fn jobs() -> &'static Mutex<HashMap<String, JobEntry>> {
    static JOBS: OnceLock<Mutex<HashMap<String, JobEntry>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Poll a background verify job. Running -> {status, elapsed_secs}; done ->
/// the stored verdict verbatim.
fn poll_job(job_id: &str) -> String {
    let map = jobs().lock().unwrap();
    match map.get(job_id) {
        None => serde_json::json!({
            "passed": false,
            "status": "unknown",
            "error": format!("no such job: {}", job_id),
        })
        .to_string(),
        Some(e) if e.status == "running" => serde_json::json!({
            "job_id": job_id,
            "status": "running",
            "elapsed_secs": now_ms().saturating_sub(e.started_ms) / 1000,
            "command": e.command,
        })
        .to_string(),
        Some(e) => e.verdict.clone().unwrap_or_else(|| {
            serde_json::json!({"job_id": job_id, "status": "done", "error": "no verdict stored"})
                .to_string()
        }),
    }
}

fn sanitize_tag(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// Tool name: "dev_verify"
///
/// Anti-fake-pass gate. Runs a build/test command, captures full output to a
/// log file, and returns STRUCTURED JSON: `{passed, exit_code, summary, failed,
/// warnings, log_path, command, (passed_count/failed_count/ignored_count for
/// tests, error_count on compile failure)}`. `passed` is the process's real
/// `exit_code == 0` — a tool-returned fact, not an agent's claim. Any AI tool
/// (via MCP) or the CLI can use this as the single shared "is it green?" gate.
///
/// Modes:
/// - `shell` (bool): run via `sh -c`/`cmd /C` (auto-on when the command contains
///   pipes/`&&`/quoting/globs). Lets you do `cd core && cargo test ...`.
/// - `background` (bool): spawn and return `{job_id}` immediately; poll later
///   with `{job: "<id>"}`. Unblocks 9-minute builds.
/// - `job` (string): poll a previously-started background job.
/// - `remote` (string): run on a cluster peer (HMAC-signed).
///
/// Params: command (required unless `job`), path (cwd, default "."), label (log
/// tag, default "verify"), timeout_secs (default 600), env (object).
pub async fn dev_verify(args: &Value) -> String {
    // Poll an existing background job.
    if let Some(job_id) = args["job"].as_str().filter(|s| !s.is_empty()) {
        return poll_job(job_id);
    }

    let command = match args["command"].as_str() {
        Some(c) if !c.is_empty() => c,
        _ => {
            return serde_json::json!({
                "passed": false,
                "error": "missing 'command' argument (or 'job' to poll)",
            })
            .to_string()
        }
    };
    // Remote mode: dispatch this verify to a cluster peer's /rpc/dev-verify
    // (HMAC-signed). The command runs in the PEER's context (synchronous).
    if let Some(remote) = args["remote"].as_str().filter(|s| !s.is_empty()) {
        return run_remote_verify(remote, args).await;
    }

    let path = args["path"].as_str().unwrap_or(".").to_string();
    let label = args["label"].as_str().unwrap_or("verify").to_string();
    let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(600);
    let shell = args["shell"].as_bool().unwrap_or_else(|| needs_shell(command));
    let background = args["background"].as_bool().unwrap_or(false);
    // Embed captured output in the verdict so a (possibly remote) caller can READ
    // what happened, not just pass/fail. `output_bytes` sets an explicit tail
    // size; `include_output` is a 4 KB shorthand. Default off.
    let output_tail_bytes = args["output_bytes"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or_else(|| {
            if args["include_output"].as_bool().unwrap_or(false) {
                4000
            } else {
                0
            }
        });
    // Optional env vars for the command (e.g. {"SPECTYN_MESH_GOOGLE_CLIENT_ID": "..."}),
    // so callers can verify env-gated flows without a shell `export ... &&` prefix.
    let env_pairs: Vec<(String, String)> = args["env"]
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    // Optional artifact integrity check, run AFTER a 0-exit build: assert the
    // built file exists, meets a byte-size floor, and (optionally) `--help` runs
    // without crashing. Absent `artifact_path`, behaviour is unchanged.
    let artifact: Option<ArtifactCheck> = args["artifact_path"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|p| ArtifactCheck {
            path: p.to_string(),
            min_bytes: args["artifact_min_bytes"].as_u64().unwrap_or(0),
            run_help: args["artifact_run_help"].as_bool().unwrap_or(false),
        });

    // Background mode: spawn, register, return a poll handle immediately.
    if background {
        let job_id = format!("{}-{}", sanitize_tag(&label), now_ms());
        jobs().lock().unwrap().insert(
            job_id.clone(),
            JobEntry {
                status: "running",
                started_ms: now_ms(),
                command: command.to_string(),
                verdict: None,
            },
        );
        let (cmd_s, path_s, label_s, jid) =
            (command.to_string(), path.clone(), label.clone(), job_id.clone());
        let artifact_s = artifact.clone();
        tokio::spawn(async move {
            let verdict = match run_cmd_rc(&cmd_s, shell, Some(&path_s), timeout_secs, &env_pairs).await {
                Ok((output, ec)) => build_verdict(&cmd_s, &label_s, &path_s, &output, ec, output_tail_bytes, artifact_s.as_ref()).await,
                Err(e) => serde_json::json!({
                    "passed": false,
                    "exit_code": -1,
                    "error": e,
                    "summary": format!("failed to run: {}", cmd_s),
                    "command": cmd_s,
                }),
            };
            if let Some(entry) = jobs().lock().unwrap().get_mut(&jid) {
                entry.status = "done";
                entry.verdict = Some(verdict.to_string());
            }
        });
        return serde_json::json!({
            "job_id": job_id,
            "started": true,
            "status": "running",
            "command": command,
            "note": "poll with dev_verify {\"job\": \"<job_id>\"}",
        })
        .to_string();
    }

    // Synchronous mode.
    match run_cmd_rc(command, shell, Some(&path), timeout_secs, &env_pairs).await {
        Ok((output, exit_code)) => build_verdict(command, &label, &path, &output, exit_code, output_tail_bytes, artifact.as_ref()).await.to_string(),
        Err(e) => serde_json::json!({
            "passed": false,
            "exit_code": -1,
            "error": e,
            "summary": format!("failed to run: {}", command),
            "command": command,
        })
        .to_string(),
    }
}

// ── dev_verify remote mode (run on a cluster peer) ──────────────────────────

/// HMAC-SHA256 hex over the body with the cluster_secret — matches the legacy
/// body-HMAC scheme that the server's `require_cluster_auth_dual` accepts
/// (same as `spectyn dispatch`).
fn dv_hmac_sha256_hex(secret: &str, body: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Dispatch a dev_verify to a cluster peer's `/rpc/dev-verify` endpoint.
/// `remote` = peer base URL (e.g. "http://100.64.0.5:7878"). The command runs
/// in the PEER's filesystem/cwd, so `path` must be valid there. Returns the
/// peer's structured verdict annotated with `"remote"`.
async fn run_remote_verify(remote: &str, args: &Value) -> String {
    // cluster_secret from local agents.toml [cluster] for HMAC signing.
    let secret = match crate::cli_config::agents_toml_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| toml::from_str::<crate::config::AgentsConfig>(&raw).ok())
        .and_then(|cfg| cfg.cluster.cluster_secret.clone())
        .filter(|s| !s.trim().is_empty())
    {
        Some(s) => s,
        None => {
            return serde_json::json!({
                "passed": false,
                "error": "no cluster_secret in agents.toml [cluster] — cannot sign remote verify",
                "remote": remote,
            })
            .to_string()
        }
    };

    // Strip `remote` from the forwarded body so the peer runs locally.
    let body = serde_json::json!({
        "command": args["command"],
        "path": args["path"],
        "label": args["label"],
        "timeout_secs": args["timeout_secs"],
        "env": args["env"],
        "shell": args["shell"],
        "include_output": args["include_output"],
        "output_bytes": args["output_bytes"],
    });
    let body_str = body.to_string();
    let sig = dv_hmac_sha256_hex(&secret, body_str.as_bytes());
    let url = format!("{}/rpc/dev-verify", remote.trim_end_matches('/'));
    let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(600);

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs + 30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return serde_json::json!({"passed": false, "error": format!("client build: {e}"), "remote": remote}).to_string()
        }
    };

    let resp = match client
        .post(&url)
        .header("X-Cluster-Auth", sig)
        .header("Content-Type", "application/json")
        .body(body_str)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return serde_json::json!({"passed": false, "error": format!("remote unreachable: {e}"), "remote": remote}).to_string()
        }
    };

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return serde_json::json!({
            "passed": false,
            "error": format!("peer HTTP {}: {}", status.as_u16(), text),
            "remote": remote,
        })
        .to_string();
    }
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(mut v) => {
            if let Some(obj) = v.as_object_mut() {
                obj.insert("remote".to_string(), serde_json::json!(remote));
            }
            v.to_string()
        }
        Err(_) => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// An existing file at/above the byte floor → `exists`, `size_ok`, `ok`.
    #[tokio::test]
    async fn artifact_present_above_threshold_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("artifact.bin");
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(&[0u8; 64]).unwrap();
        f.flush().unwrap();

        let v = verify_artifact(&ArtifactCheck {
            path: p.to_string_lossy().into_owned(),
            min_bytes: 32,
            run_help: false,
        })
        .await;

        assert_eq!(v["exists"], true, "{v}");
        assert_eq!(v["size"], 64, "{v}");
        assert_eq!(v["size_ok"], true, "{v}");
        assert_eq!(v["ok"], true, "{v}");
        // help_ok must be absent when run_help is false.
        assert!(v.get("help_ok").is_none(), "{v}");
    }

    /// A missing artifact → `exists=false`, `size_ok=false`, `ok=false`.
    #[tokio::test]
    async fn artifact_missing_is_not_ok() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("does-not-exist.bin");

        let v = verify_artifact(&ArtifactCheck {
            path: p.to_string_lossy().into_owned(),
            min_bytes: 0,
            run_help: false,
        })
        .await;

        assert_eq!(v["exists"], false, "{v}");
        assert_eq!(v["size"], 0, "{v}");
        assert_eq!(v["size_ok"], false, "{v}");
        assert_eq!(v["ok"], false, "{v}");
    }

    /// A file below the byte floor → exists but `size_ok=false` → `ok=false`.
    #[tokio::test]
    async fn artifact_below_threshold_fails_size() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("tiny.bin");
        std::fs::write(&p, b"hi").unwrap(); // 2 bytes

        let v = verify_artifact(&ArtifactCheck {
            path: p.to_string_lossy().into_owned(),
            min_bytes: 1024,
            run_help: false,
        })
        .await;

        assert_eq!(v["exists"], true, "{v}");
        assert_eq!(v["size"], 2, "{v}");
        assert_eq!(v["size_ok"], false, "{v}");
        assert_eq!(v["ok"], false, "{v}");
    }

    /// `--help` on a real tiny executable script must run without crashing →
    /// `help_ok=true`. Unix-only (relies on a `#!/bin/sh` + chmod +x).
    #[cfg(unix)]
    #[tokio::test]
    async fn artifact_run_help_does_not_crash() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("toolish.sh");
        std::fs::write(&p, "#!/bin/sh\necho usage: toolish\nexit 0\n").unwrap();
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&p, perms).unwrap();

        let v = verify_artifact(&ArtifactCheck {
            path: p.to_string_lossy().into_owned(),
            min_bytes: 1,
            run_help: true,
        })
        .await;

        assert_eq!(v["exists"], true, "{v}");
        assert_eq!(v["size_ok"], true, "{v}");
        assert_eq!(v["help_ok"], true, "{v}");
        assert_eq!(v["help_exit"], 0, "{v}");
        assert_eq!(v["ok"], true, "{v}");
    }

    /// Backward-compatibility: with NO artifact requested, `build_verdict`'s
    /// `passed` is exactly `exit_code == 0` and carries no `artifact` field.
    #[tokio::test]
    async fn build_verdict_no_artifact_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_string_lossy().into_owned();

        let pass = build_verdict("true", "verify", &cwd, "ok\n", 0, 0, None).await;
        assert_eq!(pass["passed"], true, "{pass}");
        assert_eq!(pass["exit_code"], 0, "{pass}");
        assert!(pass.get("artifact").is_none(), "no artifact field: {pass}");

        let fail = build_verdict("false", "verify", &cwd, "boom\n", 1, 0, None).await;
        assert_eq!(fail["passed"], false, "{fail}");
        assert_eq!(fail["exit_code"], 1, "{fail}");
        assert!(fail.get("artifact").is_none(), "{fail}");
    }

    /// A green exit (0) that produced an under-size / missing artifact must flip
    /// `passed` to false and attach the `artifact` sub-verdict.
    #[tokio::test]
    async fn build_verdict_green_exit_but_bad_artifact_fails() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_string_lossy().into_owned();
        let missing = dir.path().join("never-built").to_string_lossy().into_owned();

        let v = build_verdict(
            "build",
            "verify",
            &cwd,
            "ok\n",
            0,
            0,
            Some(&ArtifactCheck { path: missing, min_bytes: 1, run_help: false }),
        )
        .await;

        assert_eq!(v["exit_code"], 0, "exit was green: {v}");
        assert_eq!(v["passed"], false, "missing artifact must fail the verdict: {v}");
        assert_eq!(v["artifact"]["exists"], false, "{v}");
    }

    /// A green exit WITH a satisfied artifact stays passed and reports the check.
    #[tokio::test]
    async fn build_verdict_green_exit_good_artifact_passes() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_string_lossy().into_owned();
        let art = dir.path().join("built.bin");
        std::fs::write(&art, &[7u8; 128]).unwrap();
        let art_s = art.to_string_lossy().into_owned();

        let v = build_verdict(
            "build",
            "verify",
            &cwd,
            "ok\n",
            0,
            0,
            Some(&ArtifactCheck { path: art_s, min_bytes: 64, run_help: false }),
        )
        .await;

        assert_eq!(v["passed"], true, "{v}");
        assert_eq!(v["artifact"]["size_ok"], true, "{v}");
        assert_eq!(v["artifact"]["ok"], true, "{v}");
    }

    /// A FAILED build (non-zero exit) must NOT run the artifact check (the
    /// artifact is irrelevant) — `passed` stays false, no `artifact` field.
    #[tokio::test]
    async fn build_verdict_failed_exit_skips_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_string_lossy().into_owned();
        let art = dir.path().join("built.bin");
        std::fs::write(&art, &[7u8; 128]).unwrap();
        let art_s = art.to_string_lossy().into_owned();

        let v = build_verdict(
            "build",
            "verify",
            &cwd,
            "error: boom\n",
            1,
            0,
            Some(&ArtifactCheck { path: art_s, min_bytes: 64, run_help: false }),
        )
        .await;

        assert_eq!(v["passed"], false, "{v}");
        assert!(v.get("artifact").is_none(), "no artifact check on failed build: {v}");
    }
}
