use crate::platform;
use crate::tools::validate;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

// ── Job registry for background tasks ─────────────────────────────────────

fn job_registry() -> &'static Mutex<HashMap<u32, String>> {
    static REGISTRY: OnceLock<Mutex<HashMap<u32, String>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

// ── Security lists ─────────────────────────────────────────────────────────

const BLOCKED: &[&str] = &[
    "rm -rf /",
    "rm -rf ~",
    "rm -rf $HOME",
    "sudo rm",
    "sudo dd",
    "sudo mkfs",
    ":(){:|:&};:",
    "chmod -r 777 /",
    "chmod 777 /",
    "> /etc/",
    ">> /etc/",
    "tee /etc/",
    "curl | sh",
    "curl|sh",
    "wget -o- | sh",
    "wget -o- |sh",
    "mkfs",
    "dd if=/dev/zero of=/dev/",
];

/// Public accessor for the shell-blocklist used by `bash_bg::run_background`
/// so background jobs share the exact same deny-list as foreground `shell`.
/// Patterns must already be in the lowercase-collapsed form that
/// `validate::match_blocklist` produces from raw input.
pub fn blocked_patterns() -> &'static [&'static str] {
    BLOCKED
}

const REQUIRES_CONFIRM: &[&str] = &[
    "rm ",
    "sudo ",
    "kill ",
    "pkill ",
    "killall ",
    "git reset --hard",
    "git clean ",
    "chmod ",
    "chown ",
    "DROP TABLE",
    "DROP DATABASE",
    "TRUNCATE ",
    "curl ",
    "wget ",
    "nc ",
    "netcat ",
    "cp ",
];

pub fn requires_confirmation(cmd: &str) -> Option<&'static str> {
    // Special case for cp/mv: only when destination is absolute or home path
    for prefix in &["cp ", "mv "] {
        if cmd.starts_with(prefix) {
            let parts: Vec<&str> = cmd.splitn(3, ' ').collect();
            if parts.len() == 3 && (parts[2].starts_with('/') || parts[2].starts_with("~/")) {
                return Some(prefix);
            }
        }
    }
    for pat in REQUIRES_CONFIRM {
        if cmd.contains(pat) {
            return Some(pat);
        }
    }
    // python -c / python3 -c only need confirmation when piping suspicious content
    // (i.e. when combined with shell pipe operators)
    for pat in &["python -c", "python3 -c"] {
        if cmd.contains(pat) && cmd.contains('|') {
            return Some(pat);
        }
    }
    None
}

/// Returns true if SPECTYN_AUTO_APPROVE is set to "1".
fn auto_approve_enabled() -> bool {
    std::env::var("SPECTYN_AUTO_APPROVE").as_deref() == Ok("1")
}

/// Delegate to `platform::make_command` — all platform logic lives there.
fn make_command(program: &str, args: &[String], raw_cmd: &str) -> tokio::process::Command {
    platform::make_command(program, args, raw_cmd)
}

// ── Background job execution ───────────────────────────────────────────────

/// Spawn a command in the background without waiting for it to complete.
/// Returns immediately with the PID and job label.
pub async fn run_bg(args: &Value) -> String {
    let cmd = match args["command"].as_str() {
        Some(c) if !c.is_empty() => c,
        _ => return "Error: missing 'command' argument".into(),
    };
    let label = args["label"].as_str().unwrap_or(cmd).to_string();

    // Audit C-1: enforce the same control-char + blocklist gate as `run`.
    // Pre-fix, `run_bg` accepted arbitrary commands with zero validation —
    // a model could sidestep the entire approval pipeline by saying
    // `shell` with `mode=bg` instead of the synchronous form.
    if let Err(e) = validate::reject_dangerous_chars(cmd) {
        return e;
    }
    if let Some(pat) = validate::match_blocklist(cmd, BLOCKED) {
        return format!("Error: blocked command pattern '{}'", pat);
    }
    if cmd.contains("$(") || cmd.contains('`') {
        return "Error: subshell / backtick substitution is not allowed".into();
    }

    // [C7/T76] V9 H-3: enforce the same `requires_confirmation` gate as
    // `shell::run`. Pre-fix, the background path skipped this check, so a
    // model could route destructive commands through `mode=bg` to bypass
    // approval. Honors `SPECTYN_AUTO_APPROVE=1` like the foreground path.
    // Returns a formatted string (not JSON) to match this entry point's
    // existing return shape — by design distinct from `bash_run_background`
    // which returns JSON.
    if let Some(reason) = requires_confirmation(cmd) {
        if !auto_approve_enabled() {
            return format!(
                "APPROVAL_REQUIRED: command '{}' matches pattern '{}'.\n\
                 To allow this command, the caller must set SPECTYN_AUTO_APPROVE=1 or explicitly confirm.\n\
                 Re-run with confirmation to proceed.",
                cmd, reason
            );
        }
        tracing::warn!(
            "SPECTYN_AUTO_APPROVE is active — shell::run_bg executing potentially dangerous command: '{}'",
            cmd
        );
    }

    let argv = match shell_words::split(cmd) {
        Ok(v) if !v.is_empty() => v,
        Ok(_) => return "Error: empty command".into(),
        Err(e) => return format!("Error: invalid command: {}", e),
    };
    let (program, rest) = argv.split_first().unwrap();
    let rest: Vec<String> = rest.to_vec();

    let spawn_result = make_command(program, &rest, cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    let child = match spawn_result {
        Ok(c) => c,
        Err(e) => return format!("Error: failed to spawn '{}': {}", program, e),
    };

    let pid = match child.id() {
        Some(p) => p,
        None => return "Error: could not determine PID of spawned process".into(),
    };

    // Register in job registry before detaching the child handle
    if let Ok(mut reg) = job_registry().lock() {
        reg.insert(pid, label.clone());
    }

    // Intentionally drop the child handle so it detaches (process keeps running)
    drop(child);

    format!(
        "Job started: PID={pid} label='{label}'\n\
         Use shell with command 'kill {pid}' to stop, or check with 'ps aux | grep {pid}'"
    )
}

/// Check status of background jobs.
/// If `pid` arg provided, checks that specific process; otherwise lists all tracked jobs.
pub async fn check_bg(args: &Value) -> String {
    if let Some(pid_val) = args["pid"].as_u64() {
        let pid = pid_val as u32;
        let status = process_status(pid);
        let label = job_registry()
            .lock()
            .ok()
            .and_then(|reg| reg.get(&pid).cloned())
            .unwrap_or_else(|| "(unknown)".to_string());
        format!("PID {} ({}): {}", pid, label, status)
    } else {
        let jobs: Vec<(u32, String)> = job_registry()
            .lock()
            .map(|reg| {
                reg.iter()
                    .map(|(&pid, label)| (pid, label.clone()))
                    .collect()
            })
            .unwrap_or_default();

        if jobs.is_empty() {
            return "No background jobs tracked.".into();
        }

        let mut lines: Vec<String> = jobs
            .into_iter()
            .map(|(pid, label)| format!("PID {} ({}): {}", pid, label, process_status(pid)))
            .collect();
        lines.sort();
        lines.join("\n")
    }
}

/// Check if a process is still running.
fn process_status(pid: u32) -> &'static str {
    #[cfg(windows)]
    {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid), "/NH"])
            .output();
        match out {
            Ok(o) => {
                let s = String::from_utf8_lossy(&o.stdout);
                if s.contains(&pid.to_string()) {
                    "running"
                } else {
                    "finished"
                }
            }
            _ => "unknown",
        }
    }
    #[cfg(not(windows))]
    {
        match std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
        {
            Ok(out) if out.status.success() => "running",
            _ => "finished",
        }
    }
}

// ── Schema ─────────────────────────────────────────────────────────────────

/// JSON schema for the `shell` tool, documenting all parameters.
pub fn schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "shell",
            "description": "Execute a shell command and return stdout/stderr separately with exit code. \
                            Supports custom working directory, extra environment variables, and stdin input.",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute. Supports compound commands with &&, ||, and ;"
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Timeout in seconds (default 120, max 600). Use 300+ for cargo build/test."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Working directory for the command. Defaults to current directory. \
                                        Must exist; an error is returned if the path does not exist."
                    },
                    "env": {
                        "type": "object",
                        "additionalProperties": {"type": "string"},
                        "description": "Additional environment variables to set for this command. \
                                        These are merged with the current environment (not replacing it)."
                    },
                    "stdin": {
                        "type": "string",
                        "description": "Text to pipe into the command's stdin. \
                                        Useful for commands that read from stdin."
                    }
                },
                "required": ["command"]
            }
        }
    })
}

// ── Main synchronous shell runner ──────────────────────────────────────────

pub async fn run(args: &Value) -> String {
    let cmd = match args["command"].as_str() {
        Some(c) => c,
        None => return "Error: missing 'command' argument".into(),
    };

    // Reject control / zero-width / bidi characters outright. Per audit C-1,
    // newline-line-continuations (`rm \<newline>-rf /`) and Unicode
    // zero-width joiners are common bypasses for substring blocklists.
    if let Err(e) = validate::reject_dangerous_chars(cmd) {
        return e;
    }

    // Pre-parse blocklist check (Unicode-normalised + multi-space collapsed,
    // see validate::match_blocklist). Catches `rm  -rf  /`, `rm\u{2010}rf /`,
    // `rm\u{00A0}-rf /` etc.
    if let Some(pat) = validate::match_blocklist(cmd, BLOCKED) {
        return format!("Error: blocked command pattern '{}'", pat);
    }
    if cmd.contains("$(") {
        return "Error: subshell substitution $(...) is not allowed".into();
    }
    if cmd.contains('`') {
        return "Error: backtick substitution is not allowed".into();
    }

    // Validate and resolve cwd. LLMs (especially small ones) routinely
    // emit `cwd: "~"` or `cwd: "~/some/sub"` because shell-style tilde
    // is the dominant convention they've seen — but Windows `cmd.exe`
    // and POSIX `PathBuf::from("~")` don't expand it, so the tool used
    // to fail with `cwd '~' does not exist` and the agent had to burn
    // a retry round just to switch to "/" or "./". Expand here.
    let cwd: Option<std::path::PathBuf> = if let Some(dir) = args["cwd"].as_str() {
        let expanded: std::path::PathBuf = if dir == "~" {
            dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."))
        } else if let Some(rest) = dir.strip_prefix("~/").or_else(|| dir.strip_prefix("~\\")) {
            dirs::home_dir()
                .map(|h| h.join(rest))
                .unwrap_or_else(|| std::path::PathBuf::from(dir))
        } else {
            std::path::PathBuf::from(dir)
        };
        if !expanded.exists() {
            return format!("Error: cwd '{}' does not exist", dir);
        }
        if !expanded.is_dir() {
            return format!("Error: cwd '{}' is not a directory", dir);
        }
        Some(expanded)
    } else {
        None
    };

    // Parse extra env vars
    let extra_env: HashMap<String, String> = args["env"]
        .as_object()
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    // Audit C-2 (2026-05-15 tools-audit): block known env-injection
    // vectors (LD_PRELOAD, DYLD_*, PATH, GIT_SSH_COMMAND, …) BEFORE
    // they ever reach `Command::env()`. Applies to all dispatch paths
    // below — direct argv, native shell, and compound runner all share
    // this `extra_env` map, so guarding once here covers them all.
    if let Err(reason) = crate::tools::env_filter::validate_extra_env(&extra_env) {
        return format!("Error: {}", reason);
    }

    // Optional stdin text
    let stdin_text: Option<String> = args["stdin"].as_str().map(|s| s.to_string());

    let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(120).min(600);

    // Pipe `|` and redirects `>` / `<` / `2>&1` require a real shell interpreter.
    // Route through run_shell_native which uses `sh -c` / `cmd.exe /C`.
    let needs_shell = {
        let bytes = cmd.as_bytes();
        let mut i = 0;
        let mut found = false;
        while i < bytes.len() {
            match bytes[i] {
                b'|' if bytes.get(i + 1) != Some(&b'|') => {
                    found = true;
                    break;
                }
                b'>' | b'<' => {
                    found = true;
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        found
    };

    // Commands with pipe/redirect need a real shell; bypass the custom compound
    // parser and hand them directly to sh/cmd.
    if needs_shell {
        return run_via_native_shell(
            cmd,
            timeout_secs,
            cwd.as_deref(),
            &extra_env,
            stdin_text.as_deref(),
        )
        .await;
    }

    let has_operators = cmd.contains(" && ") || cmd.contains(" || ") || cmd.contains(';');
    if has_operators {
        let part_count = cmd
            .split(';')
            .flat_map(|p| p.split(" && ").flat_map(|q| q.split(" || ")))
            .filter(|p| !p.trim().is_empty())
            .count();
        if part_count > 10 {
            return format!(
                "Error: too many command parts ({}), maximum is 10",
                part_count
            );
        }
        return run_compound(
            cmd,
            timeout_secs,
            cwd.as_deref(),
            &extra_env,
            stdin_text.as_deref(),
        )
        .await;
    }

    let argv = match shell_words::split(cmd) {
        Ok(v) => v,
        Err(e) => return format!("Error: invalid command: {}", e),
    };

    // Post-parse blocklist check: catches quoted bypasses like rm '-rf' '/'
    let rejoined = argv.join(" ");
    if let Some(pat) = validate::match_blocklist(&rejoined, BLOCKED) {
        return format!("Error: blocked command pattern '{}'", pat);
    }

    // Approval gate runs AFTER blocklist — hard-blocked commands are always
    // rejected outright above, never just "APPROVAL_REQUIRED".
    if let Some(reason) = requires_confirmation(cmd) {
        if !auto_approve_enabled() {
            return format!(
                "APPROVAL_REQUIRED: command '{}' matches pattern '{}'.\n\
                 To allow this command, the caller must set SPECTYN_AUTO_APPROVE=1 or explicitly confirm.\n\
                 Re-run with confirmation to proceed.",
                cmd, reason
            );
        }
        tracing::warn!(
            "SPECTYN_AUTO_APPROVE is active — executing potentially dangerous command: '{}'",
            cmd
        );
    }

    let (program, rest) = match argv.split_first() {
        Some(pair) => pair,
        None => return "Error: empty command".into(),
    };
    let rest: Vec<String> = rest.to_vec();

    let mut command = make_command(program, &rest, cmd);
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    if let Some(ref dir) = cwd {
        command.current_dir(dir);
    }
    for (k, v) in &extra_env {
        command.env(k, v);
    }

    // Set up stdin
    if stdin_text.is_some() {
        command.stdin(std::process::Stdio::piped());
    } else {
        command.stdin(std::process::Stdio::null());
    }

    // Spawn with piped stdout and stderr so we can capture partial output on timeout
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => return format!("Error: failed to spawn '{}': {}", program, e),
    };
    let child_pid = child.id();

    // Write stdin if provided
    if let Some(ref text) = stdin_text {
        use tokio::io::AsyncWriteExt;
        if let Some(mut stdin_handle) = child.stdin.take() {
            // Best-effort: ignore errors (process may not read all of stdin)
            let _ = stdin_handle.write_all(text.as_bytes()).await;
            // Drop closes the pipe, signaling EOF to the child
        }
    }

    // Take stdio handles before waiting so the collector task can read them
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    // Spawn a task to collect all output while the process runs
    let collect_handle: tokio::task::JoinHandle<(Vec<u8>, Vec<u8>)> = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
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

    // Race between process completion and timeout
    match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), child.wait()).await {
        Ok(Ok(status)) => {
            let (stdout_bytes, stderr_bytes) = collect_handle.await.unwrap_or_default();
            let out = std::process::Output {
                status,
                stdout: stdout_bytes,
                stderr: stderr_bytes,
            };
            format_output(&out, timeout_secs)
        }
        Ok(Err(e)) => {
            collect_handle.abort();
            format!("Error: {}\n[exit code: 1]", e)
        }
        Err(_) => {
            // Timeout: kill the process then harvest any buffered output
            kill_pid(child_pid);
            let partial_result =
                tokio::time::timeout(std::time::Duration::from_millis(200), collect_handle).await;
            let (partial_out, partial_err) = match partial_result {
                Ok(Ok((stdout_bytes, stderr_bytes))) => (stdout_bytes, stderr_bytes),
                _ => (Vec::new(), Vec::new()),
            };

            let partial_display = format_separated_output(&partial_out, &partial_err);
            let partial_display = if partial_display.is_empty() {
                "(no output captured before timeout)".to_string()
            } else if partial_display.len() > 2000 {
                format!("...{}", &partial_display[partial_display.len() - 2000..])
            } else {
                partial_display
            };

            format!(
                "Command timed out after {}s.\nPartial output (last 2000 chars):\n{}\n[exit code: -1]",
                timeout_secs, partial_display
            )
        }
    }
}

// ── Native shell runner (for pipe / redirect commands) ─────────────────────

/// Run `cmd` inside the system shell (`sh -c` on Unix, `cmd.exe /C` on Windows).
/// Used when the command contains `|`, `>`, `<`, or `2>&1` which require the
/// shell to interpret — passing them as arguments to the binary doesn't work.
async fn run_via_native_shell(
    cmd: &str,
    timeout_secs: u64,
    cwd: Option<&std::path::Path>,
    extra_env: &HashMap<String, String>,
    stdin_text: Option<&str>,
) -> String {
    let mut command = platform::shell_command(cmd);
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    for (k, v) in extra_env {
        command.env(k, v);
    }
    if stdin_text.is_some() {
        command.stdin(std::process::Stdio::piped());
    } else {
        command.stdin(std::process::Stdio::null());
    }

    match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async {
        command.output().await
    })
    .await
    {
        Ok(Ok(out)) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let exit = out.status.code().unwrap_or(-1);
            let mut result = String::new();
            if !stdout.is_empty() {
                result.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !result.is_empty() && !result.ends_with('\n') {
                    result.push('\n');
                }
                result.push_str("STDERR:\n");
                result.push_str(&stderr);
            }
            if exit != 0 {
                if !result.is_empty() && !result.ends_with('\n') {
                    result.push('\n');
                }
                result.push_str(&format!("[exit code: {}]", exit));
            }
            if result.is_empty() {
                result.push_str(&format!("[exit code: {}]", exit));
            }
            result
        }
        Ok(Err(e)) => format!("Error: failed to run shell: {}", e),
        Err(_) => format!("Command timed out after {}s.", timeout_secs),
    }
}

// ── Compound command runner ────────────────────────────────────────────────

/// Operator that separates parts of a compound command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    /// Always run the next part (`;`)
    Semi,
    /// Run next only if last exit was 0 (`&&`)
    And,
    /// Run next only if last exit was non-zero (`||`)
    Or,
}

/// Parse a compound command string into `(operator_before_this_part, command_text)` pairs.
/// The very first entry always carries `Op::Semi` ("unconditionally run").
fn parse_compound(cmd: &str) -> Vec<(Op, &str)> {
    let mut result: Vec<(Op, &str)> = Vec::new();
    let bytes = cmd.as_bytes();
    let len = bytes.len();
    let mut start = 0usize;
    let mut current_op = Op::Semi;
    let mut i = 0usize;

    while i < len {
        // `&&` — must be checked before lone `&`
        if i + 1 < len && bytes[i] == b'&' && bytes[i + 1] == b'&' {
            let part = cmd[start..i].trim();
            if !part.is_empty() {
                result.push((current_op, part));
            }
            current_op = Op::And;
            i += 2;
            start = i;
            continue;
        }
        // `||`
        if i + 1 < len && bytes[i] == b'|' && bytes[i + 1] == b'|' {
            let part = cmd[start..i].trim();
            if !part.is_empty() {
                result.push((current_op, part));
            }
            current_op = Op::Or;
            i += 2;
            start = i;
            continue;
        }
        // `;`
        if bytes[i] == b';' {
            let part = cmd[start..i].trim();
            if !part.is_empty() {
                result.push((current_op, part));
            }
            current_op = Op::Semi;
            i += 1;
            start = i;
            continue;
        }
        i += 1;
    }
    let part = cmd[start..].trim();
    if !part.is_empty() {
        result.push((current_op, part));
    }
    result
}

async fn run_compound(
    cmd: &str,
    timeout_secs: u64,
    cwd: Option<&std::path::Path>,
    extra_env: &HashMap<String, String>,
    stdin_text: Option<&str>,
) -> String {
    let parts = parse_compound(cmd);
    let mut combined = String::new();
    let mut last_exit = 0i32;

    for (op, part) in &parts {
        match op {
            Op::And if last_exit != 0 => continue,
            Op::Or if last_exit == 0 => continue,
            _ => {}
        }

        if let Some(pat) = validate::match_blocklist(part, BLOCKED) {
            return format!("Error: blocked command pattern '{}' in '{}'", pat, part);
        }
        let argv = match shell_words::split(part) {
            Ok(v) if !v.is_empty() => v,
            _ => continue,
        };
        let (prog, rest) = argv.split_first().unwrap();
        let rest: Vec<String> = rest.to_vec();

        let mut command = make_command(prog, &rest, part);

        if let Some(dir) = cwd {
            command.current_dir(dir);
        }
        for (k, v) in extra_env {
            command.env(k, v);
        }

        // Only pipe stdin for the first part of a compound command
        if stdin_text.is_some() && combined.is_empty() {
            command.stdin(std::process::Stdio::piped());
        } else {
            command.stdin(std::process::Stdio::null());
        }

        match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async {
            command.output().await
        })
        .await
        {
            Ok(Ok(out)) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let have_out = !stdout.is_empty();
                let have_err = !stderr.is_empty();
                if have_out {
                    combined.push_str(&stdout);
                }
                if have_err {
                    if have_out {
                        if !combined.ends_with('\n') {
                            combined.push('\n');
                        }
                        combined.push_str("STDERR:\n");
                    } else {
                        combined.push_str("STDERR:\n");
                    }
                    combined.push_str(&stderr);
                }
                last_exit = out.status.code().unwrap_or(0);
            }
            Ok(Err(e)) => {
                combined.push_str(&format!("Error running '{}': {}\n", part, e));
                last_exit = 1;
            }
            Err(_) => {
                let snippet: String = if combined.is_empty() {
                    "(no output captured before timeout)".to_string()
                } else {
                    combined[..combined.len().min(500)].to_string()
                };
                combined.push_str(&format!(
                    "Command timed out after {}s. Last output:\n{}\n",
                    timeout_secs, snippet
                ));
                last_exit = -1;
                break;
            }
        }
    }

    if combined.is_empty() {
        format!("[exit code: {}]", last_exit)
    } else {
        if !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(&format!("[exit code: {}]", last_exit));
        crate::tools::truncate(combined, 20_000)
    }
}

// ── Output formatting ──────────────────────────────────────────────────────

/// Format stdout and stderr bytes into a separated string (no exit code appended).
fn format_separated_output(stdout_bytes: &[u8], stderr_bytes: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout_bytes);
    let stderr = String::from_utf8_lossy(stderr_bytes);
    let have_out = !stdout.is_empty();
    let have_err = !stderr.is_empty();

    match (have_out, have_err) {
        (true, true) => {
            let mut s = String::with_capacity(stdout.len() + stderr.len() + 20);
            s.push_str("STDOUT:\n");
            s.push_str(&stdout);
            if !s.ends_with('\n') {
                s.push('\n');
            }
            s.push_str("STDERR:\n");
            s.push_str(&stderr);
            s
        }
        (true, false) => stdout.into_owned(),
        (false, true) => {
            let mut s = String::with_capacity(stderr.len() + 8);
            s.push_str("STDERR:\n");
            s.push_str(&stderr);
            s
        }
        (false, false) => String::new(),
    }
}

fn format_output(out: &std::process::Output, _timeout_secs: u64) -> String {
    let exit_code = out.status.code().unwrap_or(0);
    let body = format_separated_output(&out.stdout, &out.stderr);

    if body.is_empty() {
        format!("[exit code: {}]", exit_code)
    } else {
        let mut result = body;
        if !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str(&format!("[exit code: {}]", exit_code));
        crate::tools::truncate(result, 20_000)
    }
}

fn kill_pid(pid: Option<u32>) {
    if let Some(p) = pid {
        #[cfg(windows)]
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/PID", &p.to_string()])
            .output();
        #[cfg(not(windows))]
        let _ = std::process::Command::new("kill")
            .args(["-9", &p.to_string()])
            .output();
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ---- parse_compound tests ----

    #[test]
    fn test_parse_compound_semicolon() {
        let parts = parse_compound("echo a; echo b");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], (Op::Semi, "echo a"));
        assert_eq!(parts[1], (Op::Semi, "echo b"));
    }

    #[test]
    fn test_parse_compound_and() {
        let parts = parse_compound("true && echo yes");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], (Op::Semi, "true"));
        assert_eq!(parts[1], (Op::And, "echo yes"));
    }

    #[test]
    fn test_parse_compound_or() {
        let parts = parse_compound("false || echo fallback");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], (Op::Semi, "false"));
        assert_eq!(parts[1], (Op::Or, "echo fallback"));
    }

    #[test]
    fn test_parse_compound_mixed() {
        let parts = parse_compound("a && b || c; d");
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0].0, Op::Semi);
        assert_eq!(parts[1].0, Op::And);
        assert_eq!(parts[2].0, Op::Or);
        assert_eq!(parts[3].0, Op::Semi);
    }

    // ---- requires_confirmation tests ----

    #[test]
    fn test_requires_confirmation_rm() {
        assert!(requires_confirmation("rm somefile").is_some());
    }

    #[test]
    fn test_requires_confirmation_safe() {
        assert!(requires_confirmation("ls -la").is_none());
        assert!(requires_confirmation("echo hello").is_none());
    }

    #[test]
    fn test_requires_confirmation_mv_absolute() {
        assert!(requires_confirmation("mv foo /tmp/bar").is_some());
    }

    // ---- approval gate tests ----

    #[tokio::test]
    async fn test_approval_gate_blocks_without_env() {
        let _g = crate::sandbox::test_lock();
        std::env::remove_var("SPECTYN_AUTO_APPROVE");
        let args = serde_json::json!({"command": "rm somefile"});
        let result = run(&args).await;
        assert!(result.starts_with("APPROVAL_REQUIRED:"), "got: {}", result);
    }

    #[tokio::test]
    async fn test_approval_gate_allows_with_env() {
        let _g = crate::sandbox::test_lock();
        std::env::set_var("SPECTYN_AUTO_APPROVE", "1");
        let args = serde_json::json!({"command": "rm /tmp/__spectyn_test_nonexistent_file__"});
        let result = run(&args).await;
        std::env::remove_var("SPECTYN_AUTO_APPROVE");
        assert!(!result.starts_with("APPROVAL_REQUIRED:"), "got: {}", result);
    }

    // ---- blocked command tests ----

    #[tokio::test]
    async fn test_blocked_rm_rf() {
        let args = serde_json::json!({"command": "rm -rf /"});
        let result = run(&args).await;
        assert!(result.starts_with("Error: blocked"), "got: {}", result);
    }

    // ---- Audit C-1: blocklist bypass regressions ----
    //
    // Each of these would have succeeded pre-fix because the previous
    // implementation used raw `cmd.contains("rm -rf /")` substring matching
    // and never normalised whitespace or Unicode dashes.

    #[tokio::test]
    async fn test_blocked_rm_rf_double_space() {
        let args = serde_json::json!({"command": "rm  -rf  /"});
        let result = run(&args).await;
        assert!(
            result.starts_with("Error: blocked"),
            "double-space bypass should be caught, got: {}",
            result
        );
    }

    #[tokio::test]
    async fn test_blocked_rm_rf_tab() {
        let args = serde_json::json!({"command": "rm\t-rf\t/"});
        let result = run(&args).await;
        assert!(
            result.starts_with("Error: blocked"),
            "tab-separator bypass should be caught, got: {}",
            result
        );
    }

    #[tokio::test]
    async fn test_blocked_rm_rf_unicode_hyphen() {
        // U+2010 Unicode hyphen — visually identical to ASCII '-'.
        let args = serde_json::json!({"command": "rm \u{2010}rf /"});
        let result = run(&args).await;
        assert!(
            result.starts_with("Error:"),
            "Unicode-hyphen bypass should be caught, got: {}",
            result
        );
    }

    #[tokio::test]
    async fn test_blocked_rm_rf_non_breaking_space() {
        // U+00A0 non-breaking space.
        let args = serde_json::json!({"command": "rm\u{00A0}-rf /"});
        let result = run(&args).await;
        assert!(
            result.starts_with("Error:"),
            "non-breaking-space bypass should be caught, got: {}",
            result
        );
    }

    #[tokio::test]
    async fn test_blocked_newline_bypass() {
        // The audit calls out `rm \<newline>-rf /` as a bypass via line
        // continuation. We reject newlines outright now.
        let args = serde_json::json!({"command": "rm \\\n-rf /"});
        let result = run(&args).await;
        assert!(
            result.starts_with("Error:"),
            "newline injection should be rejected, got: {}",
            result
        );
    }

    #[tokio::test]
    async fn test_blocked_zero_width_joiner() {
        // U+200B zero-width space embedded inside the command.
        let args = serde_json::json!({"command": "rm\u{200B} -rf /"});
        let result = run(&args).await;
        assert!(
            result.starts_with("Error:"),
            "zero-width joiner should be rejected, got: {}",
            result
        );
    }

    #[tokio::test]
    async fn test_blocked_via_native_shell_pipe() {
        // Audit C-1 (the trickiest bypass): commands with pipes/redirects
        // route through `run_via_native_shell` straight to `sh -c`, bypassing
        // the post-parse re-check. The pre-parse check at the top of `run`
        // must still catch the blocklist on this path.
        let args = serde_json::json!({"command": "echo x | rm -rf /"});
        let result = run(&args).await;
        assert!(
            result.starts_with("Error: blocked"),
            "native-shell route must apply blocklist, got: {}",
            result
        );
    }

    // ---- Legitimate commands must still work ----

    #[tokio::test]
    async fn test_legit_echo_still_works() {
        let args = serde_json::json!({"command": "echo hello"});
        let result = run(&args).await;
        assert!(
            !result.starts_with("Error:"),
            "echo should not be blocked, got: {}",
            result
        );
        assert!(result.contains("hello"), "got: {}", result);
    }

    #[tokio::test]
    async fn test_legit_command_with_dash_arg() {
        // The blocklist must not false-positive on legitimate uses of `-`
        // or short flags. `ls -la` is the canonical example.
        let args = serde_json::json!({"command": "ls -la /tmp"});
        let result = run(&args).await;
        assert!(
            !result.starts_with("Error:"),
            "ls -la should not be blocked, got: {}",
            result
        );
    }

    #[tokio::test]
    async fn test_blocked_subshell() {
        let args = serde_json::json!({"command": "echo $(whoami)"});
        let result = run(&args).await;
        assert!(result.contains("subshell"), "got: {}", result);
    }

    #[tokio::test]
    async fn test_blocked_backtick() {
        let args = serde_json::json!({"command": "echo `whoami`"});
        let result = run(&args).await;
        assert!(result.contains("backtick"), "got: {}", result);
    }

    // ---- background job tests ----

    #[tokio::test]
    async fn test_run_bg_missing_command() {
        let args = serde_json::json!({});
        let result = run_bg(&args).await;
        assert!(result.starts_with("Error:"), "got: {}", result);
    }

    #[tokio::test]
    async fn test_run_bg_returns_pid() {
        let args = serde_json::json!({"command": "sleep 60", "label": "test_sleep"});
        let result = run_bg(&args).await;
        assert!(
            result.contains("PID="),
            "expected PID in output, got: {}",
            result
        );
        assert!(
            result.contains("test_sleep"),
            "expected label in output, got: {}",
            result
        );
        // Clean up: extract PID and kill it
        if let Some(pid_str) = result
            .split("PID=")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
        {
            if let Ok(pid) = pid_str.parse::<u32>() {
                kill_pid(Some(pid));
            }
        }
    }

    // ── [C7/T76] V9 H-3 approval-gate parity tests for run_bg ────────────

    /// V9 H-3 regression: `shell::run_bg` must refuse `requires_confirmation`
    /// matches without `SPECTYN_AUTO_APPROVE`. Pre-fix the background entry
    /// point spawned anything not on the hard blocklist — letting `sudo …`
    /// or `kill …` slip past the approval gate that `run` enforces.
    #[tokio::test]
    async fn test_run_bg_requires_confirmation_blocks_sudo() {
        let _g = crate::sandbox::test_lock();
        std::env::remove_var("SPECTYN_AUTO_APPROVE");
        let args = serde_json::json!({"command": "sudo systemctl restart sshd"});
        let result = run_bg(&args).await;
        assert!(
            result.starts_with("APPROVAL_REQUIRED:"),
            "expected APPROVAL_REQUIRED prefix, got: {}",
            result
        );
    }

    /// V9 H-3 regression: harmless commands must still launch.
    #[tokio::test]
    async fn test_run_bg_safe_command_still_launches() {
        let _g = crate::sandbox::test_lock();
        std::env::remove_var("SPECTYN_AUTO_APPROVE");
        // Use a long-running command so we get a real PID before exit.
        let args = serde_json::json!({"command": "sleep 60", "label": "c7_t76_safe"});
        let result = run_bg(&args).await;
        assert!(
            !result.starts_with("APPROVAL_REQUIRED:") && !result.starts_with("Error:"),
            "safe command should not be gated, got: {}",
            result
        );
        assert!(result.contains("PID="), "expected PID, got: {}", result);
        // Cleanup.
        if let Some(pid_str) = result
            .split("PID=")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
        {
            if let Ok(pid) = pid_str.parse::<u32>() {
                kill_pid(Some(pid));
            }
        }
    }

    #[tokio::test]
    async fn test_check_bg_no_pid_arg() {
        let args = serde_json::json!({});
        let _result = check_bg(&args).await;
    }

    #[tokio::test]
    async fn test_check_bg_nonexistent_pid() {
        let args = serde_json::json!({"pid": 4294967295_u64});
        let result = check_bg(&args).await;
        assert!(
            result.contains("finished") || result.contains("4294967295"),
            "got: {}",
            result
        );
    }

    // ---- new param tests ----

    #[cfg(unix)] // POSIX-only command (pwd/env/true/cat); skipped on Windows
    #[tokio::test]
    async fn test_cwd_valid() {
        let args = serde_json::json!({"command": "pwd", "cwd": "/tmp"});
        let result = run(&args).await;
        assert!(
            result.contains("/tmp") || result.contains("exit code: 0"),
            "got: {}",
            result
        );
    }

    #[tokio::test]
    async fn test_cwd_invalid() {
        let args = serde_json::json!({"command": "pwd", "cwd": "/nonexistent_spectyn_dir_xyz"});
        let result = run(&args).await;
        assert!(result.starts_with("Error: cwd"), "got: {}", result);
    }

    #[tokio::test]
    async fn test_cwd_tilde_expands_to_home() {
        // LLMs frequently emit `cwd: "~"`. Both alone and as a prefix
        // (~/something) it must resolve to dirs::home_dir() — without
        // this tilde would be passed verbatim to PathBuf::from("~"),
        // existing nowhere on either OS, and the agent would burn a
        // retry round just to switch to "/" or "./".
        let args = serde_json::json!({"command": "echo home", "cwd": "~"});
        let result = run(&args).await;
        assert!(
            !result.starts_with("Error: cwd"),
            "tilde-only cwd should expand, got: {}",
            result
        );
        assert!(
            result.contains("home") || result.contains("exit code: 0"),
            "got: {}",
            result
        );
    }

    #[tokio::test]
    async fn test_cwd_tilde_slash_variants_expand_to_home() {
        // The prefix forms must also expand: `~/` (POSIX) and `~\` (the
        // Windows form an LLM emits on Windows). Both strip to an empty
        // remainder, so they resolve to home itself. The "~\\" branch is
        // Windows-specific and was previously untested.
        for cwd in ["~/", "~\\"] {
            let args = serde_json::json!({"command": "echo home", "cwd": cwd});
            let result = run(&args).await;
            assert!(
                !result.starts_with("Error: cwd"),
                "cwd '{}' should expand to home, got: {}",
                cwd,
                result
            );
            assert!(
                result.contains("home") || result.contains("exit code: 0"),
                "cwd '{}' got: {}",
                cwd,
                result
            );
        }
    }

    #[cfg(unix)] // POSIX-only command (pwd/env/true/cat); skipped on Windows
    #[tokio::test]
    async fn test_env_injected() {
        let args = serde_json::json!({
            "command": "env",
            "env": {"SPECTYN_TEST_VAR": "hello_from_spectyn"}
        });
        let result = run(&args).await;
        assert!(
            result.contains("SPECTYN_TEST_VAR=hello_from_spectyn"),
            "got: {}",
            result
        );
    }

    // Audit C-2 (2026-05-15 tools-audit): the `env` parameter must NOT
    // accept dynamic-loader / config-redirect / SSL trust hijacks. The
    // detailed rule list lives in `tools::env_filter` — these
    // integration tests just confirm the wiring blocks the attack at
    // the `shell::run` boundary BEFORE the child is spawned (i.e. no
    // process is even created with `LD_PRELOAD` set).

    #[tokio::test]
    async fn env_injection_ld_preload_is_blocked() {
        let args = serde_json::json!({
            "command": "echo hello",
            "env": {"LD_PRELOAD": "/tmp/evil.so"}
        });
        let result = run(&args).await;
        assert!(
            result.starts_with("Error: blocked env var 'LD_PRELOAD'"),
            "LD_PRELOAD must be rejected, got: {}",
            result,
        );
        // Must NOT have actually run the command (no exit code line).
        assert!(
            !result.contains("[exit code:"),
            "command should not have spawned, got: {}",
            result
        );
    }

    #[tokio::test]
    async fn env_injection_dyld_insert_libraries_is_blocked() {
        let args = serde_json::json!({
            "command": "echo hello",
            "env": {"DYLD_INSERT_LIBRARIES": "/tmp/evil.dylib"}
        });
        let result = run(&args).await;
        assert!(
            result.contains("DYLD_INSERT_LIBRARIES"),
            "DYLD_INSERT_LIBRARIES must be rejected, got: {}",
            result,
        );
        assert!(
            result.starts_with("Error: blocked env var"),
            "got: {}",
            result
        );
    }

    #[tokio::test]
    async fn env_injection_path_override_is_blocked() {
        let args = serde_json::json!({
            "command": "echo hello",
            "env": {"PATH": "/tmp/evil:/usr/bin"}
        });
        let result = run(&args).await;
        assert!(
            result.starts_with("Error: blocked env var 'PATH'"),
            "got: {}",
            result
        );
    }

    #[tokio::test]
    async fn env_injection_git_ssh_command_is_blocked() {
        // GIT_SSH_COMMAND is the canonical "exec arbitrary code on
        // every git push/pull" knob — must be blocked even though it
        // looks innocuous (a string, not a binary path).
        let args = serde_json::json!({
            "command": "git --version",
            "env": {"GIT_SSH_COMMAND": "/tmp/evil-ssh"}
        });
        let result = run(&args).await;
        assert!(
            result.starts_with("Error: blocked env var 'GIT_SSH_COMMAND'"),
            "got: {}",
            result
        );
    }

    #[tokio::test]
    async fn env_injection_does_not_leak_into_subprocess() {
        // End-to-end guarantee: even with the var in the request, the
        // child process must NOT see it. We can't easily inspect what
        // the (rejected) child would have seen, so we verify the
        // command never ran by asserting no exit code in the output.
        let args = serde_json::json!({
            "command": "env",
            "env": {"LD_LIBRARY_PATH": "/tmp/evil"}
        });
        let result = run(&args).await;
        assert!(
            result.starts_with("Error: blocked env var"),
            "got: {}",
            result
        );
        assert!(
            !result.contains("LD_LIBRARY_PATH=/tmp/evil"),
            "leaked into child: {}",
            result
        );
    }

    #[cfg(unix)] // POSIX-only command (pwd/env/true/cat); skipped on Windows
    #[tokio::test]
    async fn env_injection_safe_vars_still_pass_through() {
        // Regression guard: blocking LD_PRELOAD must NOT block legit
        // app-config vars. RUST_LOG / PYTHONUNBUFFERED / RUST_BACKTRACE
        // are routinely set by real workflows.
        let args = serde_json::json!({
            "command": "env",
            "env": {
                "RUST_LOG": "debug",
                "PYTHONUNBUFFERED": "1",
                "SPECTYN_TEST_SAFE": "yes"
            }
        });
        let result = run(&args).await;
        assert!(
            !result.starts_with("Error: blocked"),
            "safe vars should pass, got: {}",
            result
        );
        // At least one of the vars should make it through to env(1)'s output
        // (running on Windows/macOS/Linux all emit the K=V format).
        assert!(
            result.contains("SPECTYN_TEST_SAFE=yes")
                || result.contains("RUST_LOG=debug")
                || result.contains("PYTHONUNBUFFERED=1"),
            "expected one of the safe vars to reach the child, got: {}",
            result,
        );
    }

    #[cfg(unix)] // POSIX-only command (pwd/env/true/cat); skipped on Windows
    #[tokio::test]
    async fn test_stdin_piped() {
        let args = serde_json::json!({
            "command": "cat",
            "stdin": "hello stdin"
        });
        let result = run(&args).await;
        assert!(result.contains("hello stdin"), "got: {}", result);
    }

    #[tokio::test]
    async fn test_output_format_stdout_only() {
        let args = serde_json::json!({"command": "echo hello"});
        let result = run(&args).await;
        // Should NOT have STDOUT: prefix when only stdout present
        assert!(!result.contains("STDOUT:"), "got: {}", result);
        assert!(result.contains("hello"), "got: {}", result);
        assert!(result.contains("[exit code: 0]"), "got: {}", result);
    }

    #[tokio::test]
    async fn test_output_format_stderr_only() {
        let args = serde_json::json!({"command": "ls /nonexistent_spectyn_path_xyz"});
        let result = run(&args).await;
        // ls to a nonexistent path writes to stderr and exits non-zero
        assert!(result.contains("STDERR:"), "got: {}", result);
    }

    #[cfg(unix)] // POSIX-only command (pwd/env/true/cat); skipped on Windows
    #[tokio::test]
    async fn test_exit_code_in_output() {
        let args = serde_json::json!({"command": "true"});
        let result = run(&args).await;
        assert!(result.contains("[exit code: 0]"), "got: {}", result);
    }

    #[tokio::test]
    async fn test_schema_structure() {
        let s = schema();
        assert_eq!(s["function"]["name"], "shell");
        let props = &s["function"]["parameters"]["properties"];
        assert!(props["cwd"].is_object(), "missing cwd");
        assert!(props["env"].is_object(), "missing env");
        assert!(props["stdin"].is_object(), "missing stdin");
    }
}
