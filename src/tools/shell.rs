// ShellTool — execute shell commands with allowlist + argument validation
// Security: deny-by-default, only allowed_commands can run.
// Defence-in-depth: even allowed commands have their arguments validated
// to block dangerous patterns like --exec, eval, and destructive subcommands.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use super::{SecurityConfig, Tool, ToolResult};
use crate::shell_filter::clean_shell_output;
use crate::tools::shell_session::ShellSessionManager;

/// Dangerous argument patterns that should be blocked regardless of the base command.
/// Each entry is (pattern, description) where pattern is matched case-insensitively
/// against individual arguments or the full argument string.
const DANGEROUS_ARG_PATTERNS: &[(&str, &str)] = &[
    // Execution flags that allow arbitrary code via arguments
    ("--exec", "arbitrary command execution via --exec"),
    ("-exec", "arbitrary command execution via -exec (find-style)"),
    ("--exec-path", "arbitrary command execution via --exec-path"),
    // Eval-style flags (matched as exact arg or prefix)
    ("-e", "eval/execute flag"),
    // Shell metacharacters that should never appear in arguments
    // (defence-in-depth — input_sanitizer also blocks these, but we
    //  validate here in case the sanitizer is bypassed or misconfigured)
    ("`", "backtick command substitution"),
    ("$(", "dollar-paren command substitution"),
    ("; rm", "chained rm after semicolon"),
    ("| rm", "piped rm"),
    ("&& rm", "chained rm after &&"),
    ("|| rm", "chained rm after ||"),
    ("; curl", "chained curl after semicolon"),
    ("| curl", "piped curl"),
    ("&& curl", "chained curl after &&"),
    ("; wget", "chained wget after semicolon"),
    ("| wget", "piped wget"),
    ("&& wget", "chained wget after &&"),
    ("; sh", "chained shell after semicolon"),
    ("| sh", "piped shell"),
    ("&& sh", "chained shell after &&"),
    ("; bash", "chained bash after semicolon"),
    ("| bash", "piped bash"),
    ("&& bash", "chained bash after &&"),
    ("; powershell", "chained powershell after semicolon"),
    ("| powershell", "piped powershell"),
    ("&& powershell", "chained powershell after &&"),
    ("; cmd", "chained cmd after semicolon"),
    ("| cmd", "piped cmd"),
    ("&& cmd", "chained cmd after &&"),
];

/// Dangerous subcommands / arguments that specific allowed commands should never receive.
/// Format: (base_command, blocked_arg, description)
const DANGEROUS_SUBCOMMANDS: &[(&str, &str, &str)] = &[
    // Python/Node eval — allows arbitrary code execution from CLI
    ("python", "-c", "inline code execution via python -c"),
    ("python3", "-c", "inline code execution via python3 -c"),
    ("node", "-e", "inline code execution via node -e"),
    ("node", "--eval", "inline code execution via node --eval"),
    ("node", "--input-type", "input type override can enable eval"),
    // npm/npx dangerous operations
    ("npm", "exec", "npm exec can run arbitrary packages"),
    ("npx", "--package", "npx can download and execute arbitrary packages"),
    // Git dangerous options
    ("git", "--exec", "git --exec allows arbitrary command execution"),
    ("git", "--upload-pack", "git upload-pack can execute commands"),
    ("git", "--receive-pack", "git receive-pack can execute commands"),
    ("git", "-c", "git -c can override any config including hooks"),
    // find -exec / -execdir
    ("find", "-exec", "find -exec executes arbitrary commands"),
    ("find", "-execdir", "find -execdir executes arbitrary commands"),
    ("find", "-ok", "find -ok executes arbitrary commands"),
    ("find", "-okdir", "find -okdir executes arbitrary commands"),
    ("find", "-delete", "find -delete can remove files"),
    // grep --exec doesn't exist but -e with crafted patterns could be abused
    // sqlite3 can run shell commands via .shell / .system
    ("sqlite3", "-cmd", "sqlite3 -cmd can execute dot-commands including .shell"),
    // Sort / head / tail are generally safe but block --exec-like patterns
    // cp/mv dangerous flags
    ("cp", "--remove-destination", "cp --remove-destination can destroy targets"),
    ("mv", "-f", "mv -f forces overwrite without confirmation"),
    // jq can execute shell commands in some versions
    ("jq", "--rawfile", "jq --rawfile can read arbitrary files"),
    ("jq", "--jsonargs", "jq --jsonargs can be used for injection"),
];

/// Shell metacharacters that should never appear unquoted in the argument portion
/// of a command (everything after the base command).
const SHELL_METACHAR_PATTERNS: &[(&str, &str)] = &[
    (";", "semicolon — command chaining"),
    ("&&", "double ampersand — command chaining"),
    ("||", "double pipe — command chaining"),
    ("$(", "dollar-paren — command substitution"),
    ("`", "backtick — command substitution"),
    ("\n", "newline — command injection"),
    ("\r", "carriage return — command injection"),
];

pub struct ShellTool {
    security: SecurityConfig,
    session_manager: Option<Arc<ShellSessionManager>>,
}

impl ShellTool {
    pub fn new(security: SecurityConfig) -> Self {
        Self { security, session_manager: None }
    }

    pub fn new_with_sessions(security: SecurityConfig, session_manager: Arc<ShellSessionManager>) -> Self {
        Self { security, session_manager: Some(session_manager) }
    }

    fn is_command_allowed(&self, command: &str) -> bool {
        // Extract the base command (first word)
        let base = command.split_whitespace().next().unwrap_or("");
        // Strip path prefixes
        let base = base.rsplit('/').next().unwrap_or(base);
        let base = base.rsplit('\\').next().unwrap_or(base);
        // Strip .exe suffix on Windows
        let base = base.strip_suffix(".exe").unwrap_or(base);

        self.security.allowed_commands.iter().any(|a| a == base)
    }

    /// Validate the arguments of an allowed command.
    ///
    /// This is a defence-in-depth layer: even if the base command is on the
    /// allowlist, its arguments are checked for dangerous patterns that could
    /// lead to arbitrary code execution or destructive operations.
    ///
    /// Returns `Ok(())` if arguments are safe, or `Err(reason)` if blocked.
    fn validate_arguments(&self, command: &str) -> Result<(), String> {
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(());
        }

        let base_raw = parts[0];
        // Normalise: strip path prefix and .exe suffix
        let base = base_raw.rsplit('/').next().unwrap_or(base_raw);
        let base = base.rsplit('\\').next().unwrap_or(base);
        let base = base.strip_suffix(".exe").unwrap_or(base);

        let args_str = if command.len() > base_raw.len() {
            &command[base_raw.len()..]
        } else {
            ""
        };
        let args_lower = args_str.to_lowercase();

        // ── 1. Check shell metacharacters in the argument portion ────────
        for (meta, desc) in SHELL_METACHAR_PATTERNS {
            if args_str.contains(meta) {
                warn!(
                    command_prefix = %&command[..command.len().min(80)],
                    pattern = %meta,
                    "shell metacharacter in arguments"
                );
                return Err(format!(
                    "blocked: arguments contain shell metacharacter: {} ({})",
                    meta, desc
                ));
            }
        }

        // ── 2. Check dangerous argument patterns (global) ────────────────
        for (pattern, desc) in DANGEROUS_ARG_PATTERNS {
            let pat_lower = pattern.to_lowercase();
            if args_lower.contains(&pat_lower) {
                // Special handling for `-e`: only block if it appears as a
                // standalone flag (not as part of a longer flag like `-euf`
                // or a filename like `file-extra.txt`).
                if *pattern == "-e" {
                    let is_standalone = parts[1..].iter().any(|arg| *arg == "-e");
                    if !is_standalone {
                        continue;
                    }
                }
                warn!(
                    command_prefix = %&command[..command.len().min(80)],
                    pattern = %pattern,
                    "dangerous argument pattern detected"
                );
                return Err(format!(
                    "blocked: dangerous argument pattern '{}' ({})",
                    pattern, desc
                ));
            }
        }

        // ── 3. Check command-specific dangerous subcommands ──────────────
        for (cmd, blocked_arg, desc) in DANGEROUS_SUBCOMMANDS {
            if base != *cmd {
                continue;
            }

            // Check if the blocked argument appears as a standalone token
            for arg in &parts[1..] {
                let arg_lower = arg.to_lowercase();
                // Exact match or prefix match (e.g., `-c` matches `-c`, `--exec` matches `--exec=...`)
                if arg_lower == *blocked_arg
                    || arg_lower.starts_with(&format!("{}=", blocked_arg))
                {
                    warn!(
                        command_prefix = %&command[..command.len().min(80)],
                        base_cmd = %cmd,
                        blocked = %blocked_arg,
                        "dangerous subcommand/flag for allowed command"
                    );
                    return Err(format!(
                        "blocked: '{}' does not allow argument '{}' ({})",
                        cmd, blocked_arg, desc
                    ));
                }
            }
        }

        Ok(())
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str { "shell" }

    fn preflight(&self, args: &Value) -> Result<()> {
        let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
        if command.is_empty() {
            return Err(anyhow::anyhow!("Preflight: missing 'command' parameter"));
        }
        if !self.is_command_allowed(command) {
            let base = command.split_whitespace().next().unwrap_or("");
            return Err(anyhow::anyhow!("Preflight: command '{}' is not in the allowed list", base));
        }
        // Validate arguments for dangerous patterns even on allowed commands
        if let Err(reason) = self.validate_arguments(command) {
            return Err(anyhow::anyhow!("Preflight: {}", reason));
        }
        Ok(())
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its output. Only allowed commands can be run. Paths with ~/ are auto-expanded to the home directory."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute. Use absolute Windows paths (C:/Users/...) or ~/ which auto-expands. Example: sqlite3 ~/.phantom-mesh/costs.db \"SELECT * FROM cost_records;\""
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 30, max: 300)"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Working directory (must be in workspace or allowed_paths)"
                },
                "session_id": {
                    "type": "string",
                    "description": "Shell session ID for persistent env/cwd state. Default: stateless."
                },
                "reset_session": {
                    "type": "boolean",
                    "description": "Reset the named session to clean state"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let raw_command = args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if raw_command.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: json!({"stdout": "", "stderr": "Error: empty command", "exit_code": 1}).to_string(),
            });
        }

        // ACI poka-yoke: normalize paths in the command (expand ~/, fix /home/→Windows)
        let command = &super::normalize_shell_command(raw_command);

        // Security check: allowlist
        if !self.is_command_allowed(command) {
            let base = command.split_whitespace().next().unwrap_or(command);
            warn!("Blocked command: {}", base);
            return Ok(ToolResult {
                success: false,
                output: json!({
                    "stdout": "",
                    "stderr": format!(
                        "Error: command '{}' is not in the allowed list. Allowed: {}",
                        base,
                        self.security.allowed_commands.join(", ")
                    ),
                    "exit_code": 1
                }).to_string(),
            });
        }

        // Security check: argument validation (defence-in-depth)
        if let Err(reason) = self.validate_arguments(command) {
            warn!("Blocked arguments: {}", reason);
            return Ok(ToolResult {
                success: false,
                output: json!({
                    "stdout": "",
                    "stderr": format!("Error: {}", reason),
                    "exit_code": 1
                }).to_string(),
            });
        }

        // Parse timeout (default 30s, max 300s)
        let timeout_secs = args.get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(30)
            .min(300);

        // Handle session state
        let session_id = args.get("session_id").and_then(|v| v.as_str());
        let reset_session = args.get("reset_session").and_then(|v| v.as_bool()).unwrap_or(false);

        let (session_cwd, session_env, use_session) = if let (Some(sid), Some(ref mgr)) = (session_id, &self.session_manager) {
            if reset_session {
                mgr.reset(sid);
            }
            let session = mgr.get_or_create(sid);
            (Some(session.working_dir.clone()), session.env_vars.clone(), true)
        } else {
            (None, HashMap::new(), false)
        };

        // Determine working directory
        let workspace = self.security.workspace_path();
        let cwd = if let Some(dir) = args.get("working_dir").and_then(|v| v.as_str()) {
            let dir_path = std::path::PathBuf::from(dir);
            let canonical = dir_path.canonicalize().unwrap_or(dir_path.clone());
            if self.security.is_allowed_path(&canonical) {
                canonical
            } else {
                return Ok(ToolResult {
                    success: false,
                    output: json!({
                        "stdout": "",
                        "stderr": format!("Error: working_dir '{}' is outside workspace and allowed paths", dir),
                        "exit_code": 1
                    }).to_string(),
                });
            }
        } else if let Some(ref session_dir) = session_cwd {
            session_dir.clone()
        } else {
            workspace
        };

        // Build actual command: if using session, append state capture suffix
        let actual_command = if use_session {
            format!("{}{}", command, ShellSessionManager::state_capture_suffix())
        } else {
            command.to_string()
        };

        info!("Executing (timeout {}s, cwd {}): {}", timeout_secs, cwd.display(), truncate_str(command, 100));

        // Use cmd on Windows, sh on Unix
        let output = if cfg!(target_os = "windows") {
            tokio::time::timeout(
                Duration::from_secs(timeout_secs),
                tokio::process::Command::new("cmd")
                    .args(["/C", &actual_command])
                    .current_dir(&cwd)
                    .envs(&session_env)
                    .output(),
            )
            .await
        } else {
            tokio::time::timeout(
                Duration::from_secs(timeout_secs),
                tokio::process::Command::new("sh")
                    .args(["-c", &actual_command])
                    .current_dir(&cwd)
                    .envs(&session_env)
                    .output(),
            )
            .await
        };

        // Handle timeout
        let output = match output {
            Ok(inner) => inner,
            Err(_) => {
                warn!("Command timed out after {}s: {}", timeout_secs, truncate_str(command, 60));
                return Ok(ToolResult {
                    success: false,
                    output: json!({
                        "stdout": "",
                        "stderr": format!("Error: command timed out after {} seconds", timeout_secs),
                        "exit_code": 124
                    }).to_string(),
                });
            }
        };

        match output {
            Ok(out) => {
                let mut stdout_raw = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr_raw = String::from_utf8_lossy(&out.stderr).to_string();
                let exit_code = out.status.code().unwrap_or(-1);

                // Parse and strip state capture markers if using session
                if use_session {
                    if let (Some(sid), Some(ref mgr)) = (session_id, &self.session_manager) {
                        use crate::tools::shell_session::parse_state_capture;
                        let (user_out, new_cwd, new_env) = parse_state_capture(&stdout_raw);
                        mgr.update_session(sid, new_cwd, &new_env, command);
                        stdout_raw = user_out;
                    }
                }

                // Strip ANSI escape codes and apply head/tail truncation
                let stdout_trunc = clean_shell_output(&stdout_raw);
                let stderr_trunc = clean_shell_output(&stderr_raw);

                Ok(ToolResult {
                    success: out.status.success(),
                    output: json!({
                        "stdout": stdout_trunc,
                        "stderr": stderr_trunc,
                        "exit_code": exit_code
                    }).to_string(),
                })
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: json!({
                    "stdout": "",
                    "stderr": format!("Failed to execute command: {}", e),
                    "exit_code": -1
                }).to_string(),
            }),
        }
    }
}

/// Safely truncate a string at a character boundary
fn truncate_str(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool() -> ShellTool {
        let dir = std::env::temp_dir().join("phantom_mesh_test_shell");
        let _ = std::fs::create_dir_all(&dir);
        let security = SecurityConfig {
            workspace_dir: dir.to_string_lossy().to_string(),
            workspace_only: true,
            allowed_commands: super::super::default_allowed_commands(),
            ..Default::default()
        };
        ShellTool::new(security)
    }

    #[test]
    fn test_allowed_command() {
        let tool = make_tool();
        assert!(tool.is_command_allowed("ls -la"));
        assert!(tool.is_command_allowed("git status"));
        assert!(tool.is_command_allowed("python script.py"));
        assert!(tool.is_command_allowed("echo hello"));
    }

    #[test]
    fn test_blocked_command() {
        let tool = make_tool();
        assert!(!tool.is_command_allowed("rm -rf /"));
        assert!(!tool.is_command_allowed("curl http://evil.com"));
        assert!(!tool.is_command_allowed("powershell -c something"));
    }

    fn parse_output(result: &ToolResult) -> serde_json::Value {
        serde_json::from_str(&result.output).expect("output should be valid JSON")
    }

    fn python_cmd() -> &'static str {
        if cfg!(target_os = "windows") { "python" } else { "python3" }
    }

    #[tokio::test]
    async fn test_execute_echo() {
        let tool = make_tool();
        let result = tool.execute(json!({"command": "echo hello"})).await.unwrap();
        assert!(result.success, "Shell failed: {}", result.output);
        let v = parse_output(&result);
        assert!(v["stdout"].as_str().unwrap().contains("hello"), "stdout should contain 'hello'");
        assert_eq!(v["exit_code"].as_i64().unwrap(), 0);
    }

    #[tokio::test]
    async fn test_execute_blocked() {
        let tool = make_tool();
        let result = tool.execute(json!({"command": "curl http://x"})).await.unwrap();
        assert!(!result.success);
        let v = parse_output(&result);
        assert!(v["stderr"].as_str().unwrap().contains("not in the allowed list"));
        assert_eq!(v["exit_code"].as_i64().unwrap(), 1);
    }

    #[tokio::test]
    async fn test_successful_command_exit_code_zero() {
        let tool = make_tool();
        let result = tool.execute(json!({"command": "echo structured_output"})).await.unwrap();
        assert!(result.success);
        let v = parse_output(&result);
        assert!(v["stdout"].as_str().unwrap().contains("structured_output"));
        assert_eq!(v["stderr"].as_str().unwrap(), "");
        assert_eq!(v["exit_code"].as_i64().unwrap(), 0);
    }

    #[tokio::test]
    async fn test_failing_command_nonzero_exit_code() {
        let tool = make_tool();
        // Use a python script file to avoid shell quoting issues across platforms.
        // Write a temp script, then execute it with python.
        let script_dir = std::env::temp_dir().join("phantom_mesh_test_shell");
        let _ = std::fs::create_dir_all(&script_dir);
        let script_path = script_dir.join("exit42.py");
        std::fs::write(&script_path, "import sys\nsys.exit(42)\n").unwrap();
        let cmd = format!("{} {}", python_cmd(), script_path.to_string_lossy().replace('\\', "/"));
        let result = tool.execute(json!({"command": cmd})).await.unwrap();
        assert!(!result.success);
        let v = parse_output(&result);
        assert_eq!(v["exit_code"].as_i64().unwrap(), 42);
    }

    #[tokio::test]
    async fn test_stderr_only_command() {
        let tool = make_tool();
        // Write a temp python script that writes only to stderr.
        let script_dir = std::env::temp_dir().join("phantom_mesh_test_shell");
        let _ = std::fs::create_dir_all(&script_dir);
        let script_path = script_dir.join("stderr_only.py");
        std::fs::write(&script_path, "import sys\nsys.stderr.write('err_only\\n')\n").unwrap();
        let cmd = format!("{} {}", python_cmd(), script_path.to_string_lossy().replace('\\', "/"));
        let result = tool.execute(json!({"command": cmd})).await.unwrap();
        let v = parse_output(&result);
        assert!(v["stderr"].as_str().unwrap().contains("err_only"), "stderr should contain 'err_only'");
        assert_eq!(v["stdout"].as_str().unwrap(), "");
    }

    #[tokio::test]
    async fn test_both_stdout_and_stderr() {
        let tool = make_tool();
        // Write a temp python script that writes to both stdout and stderr.
        let script_dir = std::env::temp_dir().join("phantom_mesh_test_shell");
        let _ = std::fs::create_dir_all(&script_dir);
        let script_path = script_dir.join("both_streams.py");
        std::fs::write(&script_path, "import sys\nprint('out_msg')\nsys.stderr.write('err_msg\\n')\n").unwrap();
        let cmd = format!("{} {}", python_cmd(), script_path.to_string_lossy().replace('\\', "/"));
        let result = tool.execute(json!({"command": cmd})).await.unwrap();
        let v = parse_output(&result);
        assert!(v["stdout"].as_str().unwrap().contains("out_msg"), "stdout should contain 'out_msg'");
        assert!(v["stderr"].as_str().unwrap().contains("err_msg"), "stderr should contain 'err_msg'");
        assert_eq!(v["exit_code"].as_i64().unwrap(), 0);
    }

    // ── Argument Validation Tests ─────────────────────────────────────────

    // -- Safe commands that must still pass --

    #[test]
    fn test_validate_args_safe_git_commands() {
        let tool = make_tool();
        assert!(tool.validate_arguments("git status").is_ok());
        assert!(tool.validate_arguments("git log --oneline -10").is_ok());
        assert!(tool.validate_arguments("git diff HEAD~1").is_ok());
        assert!(tool.validate_arguments("git branch -a").is_ok());
        assert!(tool.validate_arguments("git add file.rs").is_ok());
        assert!(tool.validate_arguments("git commit -m \"fix: update version\"").is_ok());
        assert!(tool.validate_arguments("git push origin main").is_ok());
        assert!(tool.validate_arguments("git clone https://github.com/user/repo.git").is_ok());
    }

    #[test]
    fn test_validate_args_safe_python_script() {
        let tool = make_tool();
        assert!(tool.validate_arguments("python script.py").is_ok());
        assert!(tool.validate_arguments("python3 manage.py runserver").is_ok());
        assert!(tool.validate_arguments("python -m pytest tests/").is_ok());
        assert!(tool.validate_arguments("python --version").is_ok());
    }

    #[test]
    fn test_validate_args_safe_cargo_commands() {
        let tool = make_tool();
        assert!(tool.validate_arguments("cargo build --release").is_ok());
        assert!(tool.validate_arguments("cargo test --lib").is_ok());
        assert!(tool.validate_arguments("cargo fmt").is_ok());
        assert!(tool.validate_arguments("cargo clippy").is_ok());
    }

    #[test]
    fn test_validate_args_safe_basic_commands() {
        let tool = make_tool();
        assert!(tool.validate_arguments("ls -la").is_ok());
        assert!(tool.validate_arguments("cat file.txt").is_ok());
        assert!(tool.validate_arguments("head -n 20 output.log").is_ok());
        assert!(tool.validate_arguments("grep -r pattern src/").is_ok());
        assert!(tool.validate_arguments("echo hello world").is_ok());
        assert!(tool.validate_arguments("mkdir -p new/directory").is_ok());
        assert!(tool.validate_arguments("wc -l file.txt").is_ok());
        assert!(tool.validate_arguments("sort data.csv").is_ok());
        assert!(tool.validate_arguments("tree src/").is_ok());
    }

    #[test]
    fn test_validate_args_safe_find_without_exec() {
        let tool = make_tool();
        assert!(tool.validate_arguments("find . -name \"*.rs\"").is_ok());
        assert!(tool.validate_arguments("find src -type f -name \"*.py\"").is_ok());
    }

    #[test]
    fn test_validate_args_safe_sqlite3_query() {
        let tool = make_tool();
        assert!(tool.validate_arguments("sqlite3 db.sqlite \"SELECT * FROM users\"").is_ok());
    }

    #[test]
    fn test_validate_args_safe_npm_commands() {
        let tool = make_tool();
        assert!(tool.validate_arguments("npm install").is_ok());
        assert!(tool.validate_arguments("npm run build").is_ok());
        assert!(tool.validate_arguments("npm test").is_ok());
        assert!(tool.validate_arguments("npm list --depth=0").is_ok());
    }

    #[test]
    fn test_validate_args_safe_jq() {
        let tool = make_tool();
        assert!(tool.validate_arguments("jq .name package.json").is_ok());
        assert!(tool.validate_arguments("jq -r .version package.json").is_ok());
    }

    // -- Dangerous patterns that must be blocked --

    #[test]
    fn test_validate_args_blocks_python_c() {
        let tool = make_tool();
        // The semicolon inside the quoted string is detected by metachar check first,
        // but even without it, the -c subcommand check would block this.
        let result = tool.validate_arguments("python -c \"import os; os.system('rm -rf /')\"");
        assert!(result.is_err(), "python -c should be blocked");
        assert!(result.unwrap_err().contains("blocked"));

        // Also test python -c without shell metacharacters in the payload
        let result2 = tool.validate_arguments("python -c \"print('hello')\"");
        assert!(result2.is_err(), "python -c should always be blocked");
        assert!(result2.unwrap_err().contains("python"));
    }

    #[test]
    fn test_validate_args_blocks_python3_c() {
        let tool = make_tool();
        let result = tool.validate_arguments("python3 -c \"import shutil; shutil.rmtree('/')\"");
        assert!(result.is_err(), "python3 -c should be blocked");
    }

    #[test]
    fn test_validate_args_blocks_node_eval() {
        let tool = make_tool();
        assert!(tool.validate_arguments("node -e \"require('child_process').exec('rm -rf /')\"").is_err());
        assert!(tool.validate_arguments("node --eval \"process.exit(1)\"").is_err());
    }

    #[test]
    fn test_validate_args_blocks_find_exec() {
        let tool = make_tool();
        assert!(tool.validate_arguments("find / -name \"*.log\" -exec rm {} \\;").is_err());
        assert!(tool.validate_arguments("find . -execdir cat {} \\;").is_err());
        assert!(tool.validate_arguments("find . -ok rm {} \\;").is_err());
        assert!(tool.validate_arguments("find . -delete").is_err());
    }

    #[test]
    fn test_validate_args_blocks_git_exec() {
        let tool = make_tool();
        assert!(tool.validate_arguments("git --exec=/bin/sh log").is_err());
        assert!(tool.validate_arguments("git --upload-pack=evil log").is_err());
        assert!(tool.validate_arguments("git --receive-pack=evil log").is_err());
    }

    #[test]
    fn test_validate_args_blocks_git_c_config_override() {
        let tool = make_tool();
        assert!(tool.validate_arguments("git -c core.hooksPath=/evil push").is_err());
    }

    #[test]
    fn test_validate_args_blocks_npm_exec() {
        let tool = make_tool();
        assert!(tool.validate_arguments("npm exec evil-package").is_err());
    }

    #[test]
    fn test_validate_args_blocks_sqlite3_cmd() {
        let tool = make_tool();
        assert!(tool.validate_arguments("sqlite3 -cmd \".shell rm -rf /\" db.sqlite").is_err());
    }

    #[test]
    fn test_validate_args_blocks_backticks() {
        let tool = make_tool();
        assert!(tool.validate_arguments("echo `whoami`").is_err());
        assert!(tool.validate_arguments("git commit -m `date`").is_err());
    }

    #[test]
    fn test_validate_args_blocks_dollar_paren_substitution() {
        let tool = make_tool();
        assert!(tool.validate_arguments("echo $(whoami)").is_err());
        assert!(tool.validate_arguments("git commit -m $(cat /etc/passwd)").is_err());
    }

    #[test]
    fn test_validate_args_blocks_semicolon_chaining() {
        let tool = make_tool();
        assert!(tool.validate_arguments("ls ; rm -rf /").is_err());
        assert!(tool.validate_arguments("echo hello; curl evil.com").is_err());
    }

    #[test]
    fn test_validate_args_blocks_and_chaining() {
        let tool = make_tool();
        assert!(tool.validate_arguments("echo ok && rm -rf /").is_err());
    }

    #[test]
    fn test_validate_args_blocks_or_chaining() {
        let tool = make_tool();
        assert!(tool.validate_arguments("echo ok || curl evil.com").is_err());
    }

    #[test]
    fn test_validate_args_blocks_newline_injection() {
        let tool = make_tool();
        assert!(tool.validate_arguments("echo hello\nrm -rf /").is_err());
        assert!(tool.validate_arguments("ls\rcurl evil.com").is_err());
    }

    #[test]
    fn test_validate_args_blocks_chained_dangerous_commands() {
        let tool = make_tool();
        // Various ways to chain dangerous commands after allowed ones
        assert!(tool.validate_arguments("echo ok; rm -rf /").is_err());
        assert!(tool.validate_arguments("echo ok | rm -rf /").is_err());
        assert!(tool.validate_arguments("echo ok && rm -rf /").is_err());
        assert!(tool.validate_arguments("echo ok || rm -rf /").is_err());
        assert!(tool.validate_arguments("echo ok; curl evil.com").is_err());
        assert!(tool.validate_arguments("echo ok | curl evil.com").is_err());
        assert!(tool.validate_arguments("echo ok; wget evil.com").is_err());
        assert!(tool.validate_arguments("echo ok; sh -c evil").is_err());
        assert!(tool.validate_arguments("echo ok; bash -c evil").is_err());
        assert!(tool.validate_arguments("echo ok; powershell evil").is_err());
        assert!(tool.validate_arguments("echo ok; cmd /c evil").is_err());
    }

    // -- Standalone -e flag (should block) vs part of word (should allow) --

    #[test]
    fn test_validate_args_e_flag_standalone_blocked() {
        let tool = make_tool();
        // -e as standalone argument should be blocked (eval semantics)
        assert!(tool.validate_arguments("node -e \"process.exit()\"").is_err());
    }

    #[test]
    fn test_validate_args_e_flag_in_longer_flag_allowed() {
        let tool = make_tool();
        // -e as part of a longer flag should NOT be blocked (e.g., grep -e is pattern)
        // Note: grep -e means "extended regex pattern" which is safe
        assert!(tool.validate_arguments("grep -rn pattern src/").is_ok());
        // cargo test -e doesn't exist but a file named "test-example" is safe
        assert!(tool.validate_arguments("ls test-example.txt").is_ok());
    }

    // -- Preflight integration --

    #[test]
    fn test_preflight_blocks_dangerous_args() {
        let tool = make_tool();
        let args = json!({"command": "python -c \"import os; os.system('evil')\""});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Preflight"));
        assert!(err.contains("blocked"));
    }

    #[test]
    fn test_preflight_allows_safe_commands() {
        let tool = make_tool();
        assert!(tool.preflight(&json!({"command": "git status"})).is_ok());
        assert!(tool.preflight(&json!({"command": "cargo build"})).is_ok());
        assert!(tool.preflight(&json!({"command": "echo hello"})).is_ok());
        assert!(tool.preflight(&json!({"command": "python script.py"})).is_ok());
    }

    // -- Execute integration --

    #[tokio::test]
    async fn test_execute_blocks_dangerous_args() {
        let tool = make_tool();
        let result = tool.execute(json!({"command": "find . -exec rm {} \\;"})).await.unwrap();
        assert!(!result.success);
        let v = parse_output(&result);
        assert!(v["stderr"].as_str().unwrap().contains("blocked"), "stderr should indicate blocking: {}", v["stderr"]);
    }

    #[tokio::test]
    async fn test_execute_blocks_backtick_injection() {
        let tool = make_tool();
        let result = tool.execute(json!({"command": "echo `whoami`"})).await.unwrap();
        assert!(!result.success);
        let v = parse_output(&result);
        assert!(v["stderr"].as_str().unwrap().contains("blocked"));
    }

    #[tokio::test]
    async fn test_execute_blocks_command_substitution() {
        let tool = make_tool();
        let result = tool.execute(json!({"command": "echo $(cat /etc/passwd)"})).await.unwrap();
        assert!(!result.success);
        let v = parse_output(&result);
        assert!(v["stderr"].as_str().unwrap().contains("blocked"));
    }

    // -- Empty / edge cases --

    #[test]
    fn test_validate_args_empty_command() {
        let tool = make_tool();
        assert!(tool.validate_arguments("").is_ok()); // empty handled elsewhere
    }

    #[test]
    fn test_validate_args_single_word_command() {
        let tool = make_tool();
        assert!(tool.validate_arguments("ls").is_ok());
        assert!(tool.validate_arguments("pwd").is_ok());
    }

    // -- Session integration tests --

    #[tokio::test]
    async fn test_shell_with_session_id_schema() {
        let tool = make_tool();
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("session_id").is_some(), "schema should have session_id");
        assert!(props.get("reset_session").is_some(), "schema should have reset_session");
    }

    #[tokio::test]
    async fn test_shell_without_session_stateless() {
        // When no session_id is provided, behavior is identical to current
        let tool = make_tool();
        let result = tool.execute(json!({"command": "echo hello"})).await.unwrap();
        assert!(result.success);
        // No state capture markers should appear in output
        assert!(!result.output.contains("PHANTOM_MESH_CWD"));
    }

    #[tokio::test]
    async fn test_state_capture_not_visible_to_user() {
        use crate::tools::shell_session::ShellSessionManager;
        use std::sync::Arc;

        let dir = std::env::temp_dir().join("phantom_mesh_test_shell_e2e");
        let _ = std::fs::create_dir_all(&dir);

        let mgr = Arc::new(ShellSessionManager::new(dir.clone()));
        let security = SecurityConfig {
            workspace_dir: dir.to_string_lossy().to_string(),
            workspace_only: false,
            allowed_commands: super::super::default_allowed_commands(),
            ..Default::default()
        };
        let tool = ShellTool::new_with_sessions(security, mgr);

        let result = tool.execute(json!({
            "command": "echo hello",
            "session_id": "test_vis"
        })).await.unwrap();
        assert!(result.success, "command should succeed: {}", result.output);
        assert!(!result.output.contains("PHANTOM_MESH_CWD"), "CWD marker should not appear in output");
        assert!(!result.output.contains("PHANTOM_MESH_ENV"), "ENV marker should not appear in output");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
