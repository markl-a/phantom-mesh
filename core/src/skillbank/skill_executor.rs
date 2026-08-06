//! Skill Document execution runtime.
//!
//! Consumes a [`crate::skillbank::skill::SkillDocument`] (parsed by H2) and
//! executes its Markdown body as a flat sequence of steps. Three step kinds:
//! `Bash` (fenced ```bash / ```sh / ```shell blocks), `Note` (prose), and
//! `Prompt` (`## Prompt:` sections — returned to caller for LLM dispatch).
//!
//! Gated behind the `experimental-curator` cargo feature. Default
//! `cargo build` does not compile this file.
//!
//! ## Known gaps (T10 v0)
//! - Bash steps run via `sh -c <code>` (Unix) or `pwsh -NoProfile -Command <code>`
//!   (Windows). This is the simplest contract but means shell metacharacters in
//!   the *skill source* are interpreted by the shell. T10 v0 is for first-party
//!   authored skills only. A future revision should switch to structured
//!   argument arrays once a skill DSL exists for that.
//! - No per-skill timeout override knob; only a per-step cap in `ExecutionOpts`.
//! - No SkillbankRuntime/T2 integration yet — T10 is standalone.

#![cfg(feature = "experimental-curator")]

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::skillbank::skill::SkillDocument;

/// A single executable step extracted from a SkillDocument body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillStep {
    Bash { code: String },
    Note { text: String },
    Prompt { text: String },
}

/// How a Bash step's body is dispatched. See module docs for semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionMode {
    /// T10 v0 behavior: pass the bash body to `sh -c` / `pwsh -Command`.
    Trusted,
    /// T29: parse the body into an argv list and spawn the program directly,
    /// only if the program's name appears in `allowed_commands`.
    Sandboxed { allowed_commands: Vec<String> },
}

impl Default for ExecutionMode {
    fn default() -> Self {
        ExecutionMode::Trusted
    }
}

/// Caller-supplied execution options.
#[derive(Debug, Clone)]
pub struct ExecutionOpts {
    pub dry_run: bool,
    pub cwd: Option<PathBuf>,
    pub bash_timeout_secs: u64,
    pub env: BTreeMap<String, String>,
    /// T29: chooses how Bash steps are dispatched. Default = Trusted (T10 v0).
    pub mode: ExecutionMode,
}

impl Default for ExecutionOpts {
    fn default() -> Self {
        Self {
            dry_run: false,
            cwd: None,
            bash_timeout_secs: 60,
            env: BTreeMap::new(),
            mode: ExecutionMode::Trusted,
        }
    }
}

/// Per-step result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    BashRan {
        exit_code: i32,
        stdout: String,
        stderr: String,
    },
    BashDryRun {
        code: String,
    },
    NoteLogged {
        text: String,
    },
    PromptDeferred {
        text: String,
    },
    BashError {
        message: String,
    },
    /// T29: sandboxed-mode parser rejected this bash step. No process was
    /// spawned. `reason` is a human-readable message (no shell interposed).
    BashRejected {
        reason: String,
    },
}

/// Aggregate result of executing a skill.
#[derive(Debug, Clone)]
pub struct SkillExecutionResult {
    pub steps_run: usize,
    pub steps_skipped: usize,
    pub errors: Vec<String>,
    pub outcomes: Vec<StepOutcome>,
    pub output: String,
}

/// Errors that prevent the executor from even starting.
#[derive(Debug, thiserror::Error)]
pub enum SkillExecError {
    #[error("skill body contains no executable steps")]
    NoSteps,
    #[error("io error while preparing executor: {0}")]
    Io(#[from] std::io::Error),
}

/// The executor itself is stateless.
pub struct SkillExecutor;

impl SkillExecutor {
    /// Execute a parsed Skill Document. See module docs for semantics.
    pub fn execute(
        skill: &SkillDocument,
        opts: ExecutionOpts,
    ) -> Result<SkillExecutionResult, SkillExecError> {
        let steps = extract_steps(&skill.body);
        if steps.is_empty() {
            return Err(SkillExecError::NoSteps);
        }

        let mut outcomes: Vec<StepOutcome> = Vec::with_capacity(steps.len());
        let mut errors: Vec<String> = Vec::new();
        let mut output_chunks: Vec<String> = Vec::new();
        let mut steps_run: usize = 0;
        let steps_skipped: usize = 0;

        for (idx, step) in steps.into_iter().enumerate() {
            match step {
                SkillStep::Note { text } => {
                    outcomes.push(StepOutcome::NoteLogged { text: text.clone() });
                    steps_run += 1;
                }
                SkillStep::Prompt { text } => {
                    outcomes.push(StepOutcome::PromptDeferred { text });
                    steps_run += 1;
                }
                SkillStep::Bash { code } => {
                    if opts.dry_run {
                        outcomes.push(StepOutcome::BashDryRun { code: code.clone() });
                        steps_run += 1;
                        continue;
                    }
                    // T29: in sandboxed mode, parse + allowlist-check BEFORE spawning.
                    if let ExecutionMode::Sandboxed { allowed_commands } = &opts.mode {
                        match parse_sandboxed_argv(&code, allowed_commands) {
                            Ok(argv) => {
                                match run_argv(&argv, &opts) {
                                    Ok((exit_code, stdout, stderr)) => {
                                        if !stdout.is_empty() {
                                            output_chunks
                                                .push(format!("[step {idx}] stdout:\n{stdout}"));
                                        }
                                        if !stderr.is_empty() {
                                            output_chunks
                                                .push(format!("[step {idx}] stderr:\n{stderr}"));
                                        }
                                        if exit_code != 0 {
                                            errors
                                                .push(format!("step {idx} exit code {exit_code}"));
                                        }
                                        outcomes.push(StepOutcome::BashRan {
                                            exit_code,
                                            stdout,
                                            stderr,
                                        });
                                        steps_run += 1;
                                    }
                                    Err(msg) => {
                                        errors.push(format!("step {idx}: {msg}"));
                                        outcomes.push(StepOutcome::BashError { message: msg });
                                        steps_run += 1;
                                    }
                                }
                                continue;
                            }
                            Err(reason) => {
                                errors.push(format!("step {idx} rejected: {reason}"));
                                outcomes.push(StepOutcome::BashRejected { reason });
                                steps_run += 1;
                                continue;
                            }
                        }
                    }
                    // Trusted path — unchanged.
                    match run_bash(&code, &opts) {
                        Ok((exit_code, stdout, stderr)) => {
                            if !stdout.is_empty() {
                                output_chunks.push(format!("[step {idx}] stdout:\n{stdout}"));
                            }
                            if !stderr.is_empty() {
                                output_chunks.push(format!("[step {idx}] stderr:\n{stderr}"));
                            }
                            if exit_code != 0 {
                                errors.push(format!("step {idx} exit code {exit_code}"));
                            }
                            outcomes.push(StepOutcome::BashRan {
                                exit_code,
                                stdout,
                                stderr,
                            });
                            steps_run += 1;
                        }
                        Err(msg) => {
                            errors.push(format!("step {idx}: {msg}"));
                            outcomes.push(StepOutcome::BashError { message: msg });
                            steps_run += 1;
                        }
                    }
                }
            }
        }

        Ok(SkillExecutionResult {
            steps_run,
            steps_skipped,
            errors,
            outcomes,
            output: output_chunks.join("\n\n"),
        })
    }
}

/// Spawn a single bash step. Returns (exit_code, stdout, stderr) on success
/// (where "success" means we got an exit status back, NOT that exit_code == 0),
/// or an error string if the process could not be spawned / timed out.
fn run_bash(code: &str, opts: &ExecutionOpts) -> Result<(i32, String, String), String> {
    use std::process::{Command, Stdio};

    // Route Trusted-mode skill bash through the same permission/trust gate as the
    // `shell` tool — otherwise a skill document body is an arbitrary-shell-exec
    // surface that bypasses SPECTYN_PERM=deny / plan-mode / observe / project-trust.
    if let Err(reason) = crate::tools::gate_allows("shell", &serde_json::json!({ "command": code }))
    {
        return Err(format!("denied by permission/trust gate: {reason}"));
    }

    #[cfg(target_os = "windows")]
    let mut cmd = {
        // Prefer `pwsh` (PowerShell 7+) when available; fall back to the
        // built-in `powershell.exe` if not. `where.exe` is the canonical
        // resolver and is always present on Windows.
        let prefer_pwsh = std::process::Command::new("where.exe")
            .arg("pwsh")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        let shell = if prefer_pwsh { "pwsh" } else { "powershell" };
        let mut c = Command::new(shell);
        c.arg("-NoProfile").arg("-Command").arg(code);
        c
    };
    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(code);
        c
    };

    if let Some(cwd) = &opts.cwd {
        cmd.current_dir(cwd);
    }
    for (k, v) in &opts.env {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;

    // Crude timeout via wait_timeout if available — but we don't have that
    // crate. Use a thread-based wait with a deadline. For v0 we accept that
    // bash_timeout_secs is a wall-clock cap on the WAIT, not a guaranteed kill
    // of the child if it's already past the deadline. A kill is attempted.
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(opts.bash_timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // Read stdout/stderr from the (now-closed) pipes.
                let mut stdout = String::new();
                let mut stderr = String::new();
                use std::io::Read;
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_string(&mut stdout);
                }
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_string(&mut stderr);
                }
                let code = status.code().unwrap_or(-1);
                return Ok((code, stdout, stderr));
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    return Err(format!(
                        "bash step exceeded {}s timeout, killed",
                        opts.bash_timeout_secs
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => return Err(format!("wait failed: {e}")),
        }
    }
}

/// Sandboxed-mode spawner. Takes a pre-parsed argv vector — argv[0] is the
/// program, argv[1..] are positional args. NO shell is interposed. Returns
/// (exit_code, stdout, stderr) on success, or an error string if spawn /
/// wait failed.
fn run_argv(argv: &[String], opts: &ExecutionOpts) -> Result<(i32, String, String), String> {
    use std::process::{Command, Stdio};

    if argv.is_empty() {
        return Err("internal: empty argv passed to run_argv".to_string());
    }
    let mut cmd = Command::new(&argv[0]);
    if argv.len() > 1 {
        cmd.args(&argv[1..]);
    }

    if let Some(cwd) = &opts.cwd {
        cmd.current_dir(cwd);
    }
    for (k, v) in &opts.env {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;

    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(opts.bash_timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                use std::io::Read;
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_string(&mut stdout);
                }
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_string(&mut stderr);
                }
                let code = status.code().unwrap_or(-1);
                return Ok((code, stdout, stderr));
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    return Err(format!(
                        "sandboxed step exceeded {}s timeout, killed",
                        opts.bash_timeout_secs
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => return Err(format!("wait failed: {e}")),
        }
    }
}

/// Parse a sandboxed-mode bash step body into an argv vector.
///
/// Returns `Err(reason)` if the body violates any sandbox rule (see module
/// docs for the full list). Pure — no I/O, no spawning.
///
/// v0 parsing rules:
///   1. Trim leading/trailing whitespace; reject if empty.
///   2. Reject if the trimmed body contains any newline (multi-line).
///   3. Reject if it contains any of:
///        - `|`           (pipe)
///        - `>` or `<`    (redirect)
///        - `` ` ``       (backtick — command substitution)
///        - `$(`          (POSIX command substitution)
///        - `$` followed immediately by an ASCII alpha or `_` (env-var expansion)
///        - `;` `&&` `||` `&`  (statement chaining)
///        - `"` or `'`    (quoting — not supported in v0; reject rather than mis-parse)
///   4. Whitespace-split into tokens; tokens[0] must be in `allowed`.
///   5. Return `tokens.into_iter().map(String::from).collect()`.
pub(crate) fn parse_sandboxed_argv(code: &str, allowed: &[String]) -> Result<Vec<String>, String> {
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return Err("empty bash step in sandboxed mode".to_string());
    }
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return Err("multi-line bash step in sandboxed mode".to_string());
    }

    // Char-scan for forbidden constructs. Order matches the reason-string table
    // in the plan so test substring assertions remain stable.
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '|' => {
                // `||` is statement chaining (or-or), bare `|` is a pipe.
                if matches!(bytes.get(i + 1), Some(b'|')) {
                    return Err(
                        "statement chaining (||) is not allowed in sandboxed mode".to_string()
                    );
                }
                return Err("pipe is not allowed in sandboxed mode".to_string());
            }
            '>' | '<' => return Err("redirect is not allowed in sandboxed mode".to_string()),
            '`' => {
                return Err(
                    "command substitution (backtick) is not allowed in sandboxed mode".to_string(),
                )
            }
            '$' => {
                // `$(...)` or `$IDENT` — both rejected.
                if let Some(&next) = bytes.get(i + 1) {
                    let nc = next as char;
                    if nc == '(' {
                        return Err(
                            "command substitution ($(...)) is not allowed in sandboxed mode"
                                .to_string(),
                        );
                    }
                    if nc.is_ascii_alphabetic() || nc == '_' || nc == '{' {
                        return Err(
                            "env var expansion is not allowed in sandboxed mode".to_string()
                        );
                    }
                }
            }
            ';' => {
                return Err("statement chaining (;) is not allowed in sandboxed mode".to_string())
            }
            '&' => {
                // `&&` and bare `&` both forbidden.
                return Err("statement chaining (&) is not allowed in sandboxed mode".to_string());
            }
            '"' | '\'' => return Err("quoting not supported in sandboxed mode (v0)".to_string()),
            _ => {}
        }
        i += 1;
    }

    // Whitespace-split (no quoting in v0 — already rejected above).
    let tokens: Vec<String> = trimmed.split_whitespace().map(String::from).collect();
    if tokens.is_empty() {
        // Defensive — we already rejected empty trimmed above.
        return Err("empty bash step in sandboxed mode".to_string());
    }
    let prog = &tokens[0];
    if !allowed.iter().any(|c| c == prog) {
        return Err(format!(
            "command not on allowlist in sandboxed mode: {prog}"
        ));
    }
    Ok(tokens)
}

/// Extract ordered steps from a Markdown body. `pub(crate)` so tests in this
/// module can exercise it without spawning processes.
pub(crate) fn extract_steps(body: &str) -> Vec<SkillStep> {
    let mut steps: Vec<SkillStep> = Vec::new();
    let mut note_buf: Vec<String> = Vec::new();
    let mut prompt_buf: Vec<String> = Vec::new();
    let mut in_prompt = false;

    // Helper closures don't borrow mutably across iterator, so inline-flush.
    let flush_note = |buf: &mut Vec<String>, out: &mut Vec<SkillStep>| {
        if !buf.is_empty() {
            let text = buf.join("\n").trim().to_string();
            if !text.is_empty() {
                out.push(SkillStep::Note { text });
            }
            buf.clear();
        }
    };
    let flush_prompt = |buf: &mut Vec<String>, out: &mut Vec<SkillStep>| {
        if !buf.is_empty() {
            let text = buf.join("\n").trim().to_string();
            if !text.is_empty() {
                out.push(SkillStep::Prompt { text });
            }
            buf.clear();
        }
    };

    let mut lines = body.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_end();

        // Detect fence-open.
        if let Some(rest) = trimmed.strip_prefix("```") {
            let lang = rest.trim();
            // Flush whatever buffer was building before this fence.
            if in_prompt {
                flush_prompt(&mut prompt_buf, &mut steps);
                in_prompt = false;
            } else {
                flush_note(&mut note_buf, &mut steps);
            }
            // Decide whether to keep or skip this block.
            let is_bash = matches!(lang, "bash" | "sh" | "shell");
            let mut block: Vec<&str> = Vec::new();
            // Consume until the closing fence (or EOF).
            while let Some(inner) = lines.next() {
                if inner.trim_end() == "```" {
                    break;
                }
                block.push(inner);
            }
            if is_bash {
                let code = block.join("\n");
                // Trim only trailing whitespace, preserve internal blank lines.
                let code = code.trim_end().to_string();
                if !code.is_empty() {
                    steps.push(SkillStep::Bash { code });
                }
            }
            continue;
        }

        // Detect `## Prompt:` heading (case-insensitive on "Prompt").
        // Must be at column 0, exactly `##` (not `###`), then space, then a
        // case-insensitive "prompt:" prefix.
        let is_prompt_heading = {
            let lc = trimmed.to_ascii_lowercase();
            lc.starts_with("## prompt:")
        };
        if is_prompt_heading {
            // Close out whatever we were buffering.
            if in_prompt {
                flush_prompt(&mut prompt_buf, &mut steps);
            } else {
                flush_note(&mut note_buf, &mut steps);
            }
            in_prompt = true;
            continue;
        }

        // Any other heading line `## ...` closes a prompt section.
        if trimmed.starts_with("## ") && in_prompt {
            flush_prompt(&mut prompt_buf, &mut steps);
            in_prompt = false;
            // Fall through so the heading itself becomes part of the next note.
        }

        // Blank line: flush note buffer; in prompt, blank lines are kept inside.
        if trimmed.is_empty() {
            if !in_prompt {
                flush_note(&mut note_buf, &mut steps);
            } else {
                // Preserve blank lines inside a prompt section.
                prompt_buf.push(String::new());
            }
            continue;
        }

        // Regular content line.
        if in_prompt {
            prompt_buf.push(line.to_string());
        } else {
            note_buf.push(line.to_string());
        }
    }

    // EOF — flush whatever's left.
    if in_prompt {
        flush_prompt(&mut prompt_buf, &mut steps);
    } else {
        flush_note(&mut note_buf, &mut steps);
    }

    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_opts_defaults_are_safe() {
        let o = ExecutionOpts::default();
        assert!(!o.dry_run);
        assert!(o.cwd.is_none());
        assert_eq!(o.bash_timeout_secs, 60);
        assert!(o.env.is_empty());
    }

    #[test]
    fn execution_opts_default_mode_is_trusted() {
        // T29: backwards-compat — default mode must be Trusted so every
        // pre-T29 caller of ExecutionOpts::default() keeps the current
        // shell-invocation behavior.
        let o = ExecutionOpts::default();
        assert!(matches!(o.mode, ExecutionMode::Trusted));
        // The other defaults are checked by `execution_opts_defaults_are_safe`;
        // we don't duplicate that here.
    }

    #[test]
    fn parse_sandboxed_argv_accepts_allowed_command() {
        // T29: allowed command + plain positional args → argv vector.
        let allowed = vec!["grep".to_string(), "ls".to_string()];
        let argv = parse_sandboxed_argv("grep -n needle README.md", &allowed)
            .expect("allowed command should parse");
        assert_eq!(argv, vec!["grep", "-n", "needle", "README.md"]);
    }

    #[test]
    fn parse_sandboxed_argv_rejects_pipe() {
        let allowed = vec!["grep".to_string(), "wc".to_string()];
        let err = parse_sandboxed_argv("grep needle file.txt | wc -l", &allowed)
            .expect_err("pipe must be rejected");
        assert!(
            err.contains("pipe"),
            "rejection reason should mention 'pipe', got: {err}"
        );
    }

    #[test]
    fn parse_sandboxed_argv_rejects_redirect() {
        let allowed = vec!["echo".to_string()];
        let err = parse_sandboxed_argv("echo hello > /tmp/out", &allowed)
            .expect_err("redirect must be rejected");
        assert!(
            err.contains("redirect"),
            "rejection reason should mention 'redirect', got: {err}"
        );
        // Also `<` and `>>`.
        let err2 =
            parse_sandboxed_argv("echo hi >> /tmp/out", &allowed).expect_err(">> must be rejected");
        assert!(err2.contains("redirect"), "got: {err2}");
        let err3 =
            parse_sandboxed_argv("echo < /tmp/in", &allowed).expect_err("< must be rejected");
        assert!(err3.contains("redirect"), "got: {err3}");
    }

    #[test]
    fn parse_sandboxed_argv_rejects_command_substitution() {
        let allowed = vec!["echo".to_string()];
        // Backtick form.
        let err =
            parse_sandboxed_argv("echo `whoami`", &allowed).expect_err("backtick must be rejected");
        assert!(err.contains("command substitution"), "got: {err}");
        // $(...) form.
        let err2 =
            parse_sandboxed_argv("echo $(whoami)", &allowed).expect_err("$(...) must be rejected");
        assert!(err2.contains("command substitution"), "got: {err2}");
    }

    #[test]
    fn parse_sandboxed_argv_rejects_env_var_expansion() {
        let allowed = vec!["echo".to_string()];
        let err = parse_sandboxed_argv("echo $HOME", &allowed).expect_err("$VAR must be rejected");
        assert!(err.contains("env var expansion"), "got: {err}");
        // ${VAR} form too.
        let err2 =
            parse_sandboxed_argv("echo ${HOME}", &allowed).expect_err("${VAR} must be rejected");
        assert!(err2.contains("env var expansion"), "got: {err2}");
    }

    #[test]
    fn parse_sandboxed_argv_rejects_statement_chaining() {
        let allowed = vec!["echo".to_string(), "ls".to_string()];
        for (input, label) in [
            ("echo a ; ls", "semicolon"),
            ("echo a && ls", "and-and"),
            ("echo a || ls", "or-or"),
            ("echo a &", "background"),
        ] {
            let err = parse_sandboxed_argv(input, &allowed)
                .expect_err(&format!("{label}: expected rejection"));
            assert!(err.contains("statement chaining"), "{label}: got: {err}");
        }
    }

    #[test]
    fn parse_sandboxed_argv_rejects_multiline_and_quotes() {
        let allowed = vec!["echo".to_string()];
        let err = parse_sandboxed_argv("echo a\necho b", &allowed)
            .expect_err("multi-line must be rejected");
        assert!(err.contains("multi-line"), "got: {err}");
        let err2 = parse_sandboxed_argv("echo \"hello world\"", &allowed)
            .expect_err("double-quote must be rejected in v0");
        assert!(err2.contains("quoting"), "got: {err2}");
        let err3 = parse_sandboxed_argv("echo 'hi'", &allowed)
            .expect_err("single-quote must be rejected in v0");
        assert!(err3.contains("quoting"), "got: {err3}");
    }

    #[test]
    fn execute_sandboxed_runs_allowed_command_via_argv() {
        // T29: end-to-end happy path. Sandboxed mode with an allowlist
        // containing a real binary; the bash step calls that binary with a
        // single arg; the executor must spawn it directly (no shell) and
        // capture stdout. We pick `cargo --version` on Unix and
        // `cmd.exe /c ver` on Windows — both ship with the toolchains we
        // build on.
        let (allowed, body, expect_substr): (Vec<String>, &str, &str) =
            if cfg!(target_os = "windows") {
                // `cmd.exe /c echo spectyn-T29-OK` is byte-deterministic and
                // doesn't depend on locale-specific `ver` output.
                (
                    vec!["cmd.exe".to_string()],
                    "cmd.exe /c echo spectyn-T29-OK",
                    "spectyn-T29-OK",
                )
            } else {
                (vec!["cargo".to_string()], "cargo --version", "cargo")
            };

        let doc = parse_doc(&format!(
            "---
name: sandbox-happy
version: 0.1.0
description: sandboxed dispatch happy path
triggers:
  - test
---
```bash
{body}
```
",
        ));
        let opts = ExecutionOpts {
            mode: ExecutionMode::Sandboxed {
                allowed_commands: allowed,
            },
            ..Default::default()
        };
        let result = SkillExecutor::execute(&doc, opts).expect("execute ok");
        let bash_outcomes: Vec<&StepOutcome> = result
            .outcomes
            .iter()
            .filter(|o| matches!(o, StepOutcome::BashRan { .. }))
            .collect();
        assert_eq!(
            bash_outcomes.len(),
            1,
            "expected 1 BashRan outcome, got: {:?}",
            result.outcomes
        );
        match bash_outcomes[0] {
            StepOutcome::BashRan {
                exit_code, stdout, ..
            } => {
                assert_eq!(*exit_code, 0, "sandboxed dispatch should succeed");
                assert!(
                    stdout
                        .to_lowercase()
                        .contains(&expect_substr.to_lowercase()),
                    "stdout {stdout:?} should mention {expect_substr:?}"
                );
            }
            _ => unreachable!(),
        }
        assert!(
            result.errors.is_empty(),
            "no errors expected for happy path: {:?}",
            result.errors
        );
        // And critically: NO BashRejected outcome.
        assert!(
            !result
                .outcomes
                .iter()
                .any(|o| matches!(o, StepOutcome::BashRejected { .. })),
            "happy path must not produce a BashRejected outcome"
        );
    }

    #[test]
    fn execute_trusted_mode_still_uses_shell() {
        // T29 regression guard: trusted mode (the default) MUST still pass
        // the body through a shell. Use `;` (statement separator) which the
        // sandboxed parser unconditionally rejects — if trusted mode somehow
        // routed through the sandboxed parser, this test would fail with a
        // BashRejected outcome. `;` works in both sh -c and powershell -Command
        // (unlike `&&`, which is not supported in Windows PowerShell 5.1).
        let doc = parse_doc(
            "---
name: trusted-shell
version: 0.1.0
description: trusted mode still uses sh -c / pwsh -Command
triggers:
  - test
---
```bash
echo first; echo second
```
",
        );
        // Use default opts — mode = Trusted is the default.
        let result = SkillExecutor::execute(&doc, ExecutionOpts::default())
            .expect("execute ok in trusted mode");
        // Must NOT be a rejection.
        assert!(
            !result
                .outcomes
                .iter()
                .any(|o| matches!(o, StepOutcome::BashRejected { .. })),
            "trusted mode must NOT route through the sandboxed parser"
        );
        // Must produce exactly one BashRan.
        let rans: Vec<&StepOutcome> = result
            .outcomes
            .iter()
            .filter(|o| matches!(o, StepOutcome::BashRan { .. }))
            .collect();
        assert_eq!(
            rans.len(),
            1,
            "expected exactly one BashRan, got: {:?}",
            result.outcomes
        );
        match rans[0] {
            StepOutcome::BashRan {
                exit_code, stdout, ..
            } => {
                assert_eq!(*exit_code, 0);
                let s = stdout.replace("\r\n", "\n");
                assert!(s.contains("first"), "stdout: {s:?}");
                assert!(s.contains("second"), "stdout: {s:?}");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn execute_sandboxed_rejects_unknown_command() {
        // T29: a bash step whose program isn't on the allowlist must produce
        // a BashRejected outcome AND not spawn anything. We pick a program
        // we know doesn't exist on the allowlist (because the allowlist is
        // empty).
        let doc = parse_doc(
            "---
name: sandbox-unknown
version: 0.1.0
description: sandboxed rejects unlisted command
triggers:
  - test
---
```bash
rm -rf /
```
",
        );
        let opts = ExecutionOpts {
            mode: ExecutionMode::Sandboxed {
                allowed_commands: vec![],
            },
            ..Default::default()
        };
        let result = SkillExecutor::execute(&doc, opts).expect("execute ok");
        let rejections: Vec<&StepOutcome> = result
            .outcomes
            .iter()
            .filter(|o| matches!(o, StepOutcome::BashRejected { .. }))
            .collect();
        assert_eq!(
            rejections.len(),
            1,
            "expected 1 BashRejected, got: {:?}",
            result.outcomes
        );
        match rejections[0] {
            StepOutcome::BashRejected { reason } => {
                assert!(
                    reason.contains("not on allowlist"),
                    "reason should mention allowlist, got: {reason}"
                );
            }
            _ => unreachable!(),
        }
        // And critically: NO BashRan outcome — nothing actually ran.
        assert!(
            !result
                .outcomes
                .iter()
                .any(|o| matches!(o, StepOutcome::BashRan { .. })),
            "MUST NOT spawn an unlisted command"
        );
        assert_eq!(result.errors.len(), 1, "rejection should record one error");
    }

    #[test]
    fn extract_steps_finds_single_bash_block() {
        let body = "\
Some intro prose.

```bash
echo hello
echo world
```
";
        let steps = extract_steps(body);
        let bash_steps: Vec<&SkillStep> = steps
            .iter()
            .filter(|s| matches!(s, SkillStep::Bash { .. }))
            .collect();
        assert_eq!(
            bash_steps.len(),
            1,
            "expected exactly one bash step, got {steps:#?}"
        );
        match bash_steps[0] {
            SkillStep::Bash { code } => {
                assert_eq!(code, "echo hello\necho world");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn extract_steps_accepts_sh_and_shell_fences() {
        let body = "\
```sh
ls
```

```shell
pwd
```
";
        let steps = extract_steps(body);
        let bash_codes: Vec<&str> = steps
            .iter()
            .filter_map(|s| match s {
                SkillStep::Bash { code } => Some(code.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(bash_codes, vec!["ls", "pwd"]);
    }

    #[test]
    fn extract_steps_skips_other_language_fences() {
        let body = "\
```python
print('nope')
```

```bash
echo yes
```

```
no language tag at all
```
";
        let steps = extract_steps(body);
        let bash_codes: Vec<&str> = steps
            .iter()
            .filter_map(|s| match s {
                SkillStep::Bash { code } => Some(code.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            bash_codes,
            vec!["echo yes"],
            "python and bare fences must NOT become bash steps"
        );
        // And the python / no-lang content MUST NOT leak into notes either:
        for s in &steps {
            if let SkillStep::Note { text } = s {
                assert!(
                    !text.contains("print('nope')"),
                    "python content leaked into a Note: {text:?}"
                );
                assert!(
                    !text.contains("no language tag at all"),
                    "bare fence content leaked into a Note: {text:?}"
                );
            }
        }
    }

    #[test]
    fn extract_steps_emits_note_for_plain_prose() {
        let body = "\
This is a paragraph.
It has two lines.

This is a second paragraph.
";
        let steps = extract_steps(body);
        let notes: Vec<&str> = steps
            .iter()
            .filter_map(|s| match s {
                SkillStep::Note { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            notes,
            vec![
                "This is a paragraph.\nIt has two lines.",
                "This is a second paragraph."
            ],
            "expected exactly two prose notes, blank line as separator"
        );
    }

    #[test]
    fn extract_steps_emits_prompt_for_prompt_heading() {
        let body = "\
Intro prose.

## Prompt: Summarize the diff

Given the diff above, produce a 2-line summary.
Keep it under 80 chars per line.

## Steps

1. Do the thing.
";
        let steps = extract_steps(body);
        let prompts: Vec<&str> = steps
            .iter()
            .filter_map(|s| match s {
                SkillStep::Prompt { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            prompts.len(),
            1,
            "expected exactly one prompt, got: {steps:#?}"
        );
        assert!(prompts[0].contains("Given the diff above"));
        assert!(prompts[0].contains("80 chars per line"));
    }

    #[test]
    fn extract_steps_prompt_heading_case_insensitive() {
        let body = "\
## PROMPT: shout

Do the loud thing.
";
        let steps = extract_steps(body);
        assert!(
            steps.iter().any(|s| matches!(s, SkillStep::Prompt { .. })),
            "PROMPT (uppercase) should still register as a prompt section"
        );
    }

    #[test]
    fn extract_steps_empty_body_returns_empty() {
        assert!(extract_steps("").is_empty());
        assert!(extract_steps("\n\n   \n").is_empty());
    }

    /// Build a `SkillDocument` from a literal full-file string (frontmatter + body).
    /// Centralized helper so tests stay short.
    fn parse_doc(input: &str) -> crate::skillbank::skill::SkillDocument {
        crate::skillbank::skill::parse_str(input).expect("parse test skill")
    }

    #[test]
    fn execute_dry_run_does_not_spawn_processes() {
        let doc = parse_doc(
            "---
name: dryrun-test
version: 0.1.0
description: ensure dry-run never spawns
triggers:
  - test
---
Some prose.

```bash
exit 7
```
",
        );
        let opts = ExecutionOpts {
            dry_run: true,
            ..Default::default()
        };
        let result = SkillExecutor::execute(&doc, opts).expect("execute ok");
        // We should see exactly one BashDryRun and one NoteLogged in outcomes.
        let dryruns = result
            .outcomes
            .iter()
            .filter(|o| matches!(o, StepOutcome::BashDryRun { .. }))
            .count();
        let ran = result
            .outcomes
            .iter()
            .filter(|o| matches!(o, StepOutcome::BashRan { .. }))
            .count();
        assert_eq!(dryruns, 1, "exactly one dry-run outcome");
        assert_eq!(ran, 0, "must NOT spawn any process in dry-run");
        assert_eq!(result.errors.len(), 0);
        // steps_run counts every step the executor processed (including notes
        // and dry-runs); steps_skipped is for steps the executor decided to
        // intentionally skip — currently always 0.
        assert!(
            result.steps_run >= 2,
            "got steps_run = {}",
            result.steps_run
        );
    }

    #[test]
    fn execute_bash_step_runs_and_captures_stdout() {
        // Use a command that exists on both Unix sh and PowerShell:
        //   echo X
        // Both shells produce "X\n" on stdout. The trailing newline form
        // differs (Unix \n, PowerShell \r\n) so we trim before asserting.
        let doc = parse_doc(
            "---
name: bash-success
version: 0.1.0
description: bash step succeeds
triggers:
  - test
---
```bash
echo spectyn-skill-exec-T10
```
",
        );
        let result = SkillExecutor::execute(&doc, ExecutionOpts::default()).expect("execute ok");
        let bash_outcomes: Vec<&StepOutcome> = result
            .outcomes
            .iter()
            .filter(|o| matches!(o, StepOutcome::BashRan { .. }))
            .collect();
        assert_eq!(bash_outcomes.len(), 1, "exactly one bash outcome");
        match bash_outcomes[0] {
            StepOutcome::BashRan {
                exit_code, stdout, ..
            } => {
                assert_eq!(*exit_code, 0, "echo should succeed");
                assert!(
                    stdout.trim().ends_with("spectyn-skill-exec-T10"),
                    "stdout was: {stdout:?}"
                );
            }
            _ => unreachable!(),
        }
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    }

    #[test]
    fn execute_bash_step_nonzero_exit_records_error() {
        // `exit 3` is portable across sh and pwsh (`exit 3` works in both).
        let doc = parse_doc(
            "---
name: bash-fail
version: 0.1.0
description: bash exits 3
triggers:
  - test
---
```bash
exit 3
```
",
        );
        let result = SkillExecutor::execute(&doc, ExecutionOpts::default())
            .expect("execute ok — non-zero exit is NOT a SkillExecError");
        assert_eq!(
            result.errors.len(),
            1,
            "expected 1 error, got {:?}",
            result.errors
        );
        assert!(
            result.errors[0].contains("exit code 3"),
            "got: {:?}",
            result.errors[0]
        );
        match &result.outcomes[0] {
            StepOutcome::BashRan { exit_code, .. } => assert_eq!(*exit_code, 3),
            other => panic!("expected BashRan, got {other:?}"),
        }
    }

    #[test]
    fn execute_note_step_records_note_logged() {
        let doc = parse_doc(
            "---
name: notes-only
version: 0.1.0
description: only prose
triggers:
  - test
---
First paragraph of guidance.

Second paragraph of guidance.
",
        );
        let result = SkillExecutor::execute(&doc, ExecutionOpts::default()).expect("execute ok");
        let notes: Vec<&str> = result
            .outcomes
            .iter()
            .filter_map(|o| match o {
                StepOutcome::NoteLogged { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(notes.len(), 2);
        assert!(notes[0].contains("First paragraph"));
        assert!(notes[1].contains("Second paragraph"));
        assert!(result.errors.is_empty());
    }

    #[test]
    fn execute_prompt_step_is_deferred_not_run() {
        let doc = parse_doc(
            "---
name: prompt-defer
version: 0.1.0
description: prompts defer to caller
triggers:
  - test
---
Intro.

## Prompt: classify the diff

Given input X, output a label.
",
        );
        let result = SkillExecutor::execute(&doc, ExecutionOpts::default()).expect("execute ok");
        let prompts: Vec<&str> = result
            .outcomes
            .iter()
            .filter_map(|o| match o {
                StepOutcome::PromptDeferred { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].contains("Given input X"));
        assert!(
            result.errors.is_empty(),
            "prompt deferral must NOT add an error"
        );
    }

    #[test]
    fn execute_returns_no_steps_error_for_empty_body() {
        let doc = parse_doc(
            "---
name: empty
version: 0.1.0
description: empty body
triggers:
  - test
---
",
        );
        let err = SkillExecutor::execute(&doc, ExecutionOpts::default())
            .expect_err("must reject empty body");
        assert!(matches!(err, SkillExecError::NoSteps), "got: {err:?}");
    }

    #[test]
    fn execute_sample_skill_from_docs_dry_run() {
        // Reads the H2 sample at docs/skills/sample-skill.md.
        // CARGO_MANIFEST_DIR is core/, so go one level up.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .join("docs/skills/sample-skill.md");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let doc = crate::skillbank::skill::parse_str(&raw).expect("parse sample-skill.md");

        let opts = ExecutionOpts {
            dry_run: true,
            ..Default::default()
        };
        let result = SkillExecutor::execute(&doc, opts).expect("execute sample skill dry-run");

        // The sample's body has prose + bullet lists, no fenced bash blocks.
        // So we expect notes only, no Bash outcomes (even as dry-runs).
        let bash = result
            .outcomes
            .iter()
            .filter(|o| {
                matches!(
                    o,
                    StepOutcome::BashRan { .. } | StepOutcome::BashDryRun { .. }
                )
            })
            .count();
        assert_eq!(bash, 0, "sample-skill.md has no fenced bash blocks");

        let notes = result
            .outcomes
            .iter()
            .filter(|o| matches!(o, StepOutcome::NoteLogged { .. }))
            .count();
        assert!(notes >= 1, "expected at least one Note from prose, got 0");
        assert!(
            result.errors.is_empty(),
            "dry-run of prose-only skill must not error"
        );
    }
}
