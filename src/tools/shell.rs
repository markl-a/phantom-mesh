// ShellTool — execute shell commands with allowlist
// Security: deny-by-default, only allowed_commands can run

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{info, warn};

use super::{SecurityConfig, Tool, ToolResult};
use crate::shell_filter::clean_shell_output;

pub struct ShellTool {
    security: SecurityConfig,
}

impl ShellTool {
    pub fn new(security: SecurityConfig) -> Self {
        Self { security }
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
                    "description": "The shell command to execute. Use absolute Windows paths (C:/Users/...) or ~/ which auto-expands. Example: sqlite3 C:/Users/m4932/.clawtex/costs.db \"SELECT * FROM cost_records;\""
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 30, max: 300)"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Working directory (must be in workspace or allowed_paths)"
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

        // Security check
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

        // Parse timeout (default 30s, max 300s)
        let timeout_secs = args.get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(30)
            .min(300);

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
        } else {
            workspace
        };

        info!("Executing (timeout {}s, cwd {}): {}", timeout_secs, cwd.display(), truncate_str(command, 100));

        // Use cmd on Windows, sh on Unix
        let output = if cfg!(target_os = "windows") {
            tokio::time::timeout(
                Duration::from_secs(timeout_secs),
                tokio::process::Command::new("cmd")
                    .args(["/C", command])
                    .current_dir(&cwd)
                    .output(),
            )
            .await
        } else {
            tokio::time::timeout(
                Duration::from_secs(timeout_secs),
                tokio::process::Command::new("sh")
                    .args(["-c", command])
                    .current_dir(&cwd)
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
                let stdout_raw = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr_raw = String::from_utf8_lossy(&out.stderr).to_string();
                let exit_code = out.status.code().unwrap_or(-1);

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
        let dir = std::env::temp_dir().join("clawtex_test_shell");
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
        let script_dir = std::env::temp_dir().join("clawtex_test_shell");
        let _ = std::fs::create_dir_all(&script_dir);
        let script_path = script_dir.join("exit42.py");
        std::fs::write(&script_path, "import sys\nsys.exit(42)\n").unwrap();
        let cmd = format!("python {}", script_path.to_string_lossy().replace('\\', "/"));
        let result = tool.execute(json!({"command": cmd})).await.unwrap();
        assert!(!result.success);
        let v = parse_output(&result);
        assert_eq!(v["exit_code"].as_i64().unwrap(), 42);
    }

    #[tokio::test]
    async fn test_stderr_only_command() {
        let tool = make_tool();
        // Write a temp python script that writes only to stderr.
        let script_dir = std::env::temp_dir().join("clawtex_test_shell");
        let _ = std::fs::create_dir_all(&script_dir);
        let script_path = script_dir.join("stderr_only.py");
        std::fs::write(&script_path, "import sys\nsys.stderr.write('err_only\\n')\n").unwrap();
        let cmd = format!("python {}", script_path.to_string_lossy().replace('\\', "/"));
        let result = tool.execute(json!({"command": cmd})).await.unwrap();
        let v = parse_output(&result);
        assert!(v["stderr"].as_str().unwrap().contains("err_only"), "stderr should contain 'err_only'");
        assert_eq!(v["stdout"].as_str().unwrap(), "");
    }

    #[tokio::test]
    async fn test_both_stdout_and_stderr() {
        let tool = make_tool();
        // Write a temp python script that writes to both stdout and stderr.
        let script_dir = std::env::temp_dir().join("clawtex_test_shell");
        let _ = std::fs::create_dir_all(&script_dir);
        let script_path = script_dir.join("both_streams.py");
        std::fs::write(&script_path, "import sys\nprint('out_msg')\nsys.stderr.write('err_msg\\n')\n").unwrap();
        let cmd = format!("python {}", script_path.to_string_lossy().replace('\\', "/"));
        let result = tool.execute(json!({"command": cmd})).await.unwrap();
        let v = parse_output(&result);
        assert!(v["stdout"].as_str().unwrap().contains("out_msg"), "stdout should contain 'out_msg'");
        assert!(v["stderr"].as_str().unwrap().contains("err_msg"), "stderr should contain 'err_msg'");
        assert_eq!(v["exit_code"].as_i64().unwrap(), 0);
    }
}
