// ShellTool — execute shell commands with allowlist
// Security: deny-by-default, only allowed_commands can run

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{info, warn};

use super::{SecurityConfig, Tool, ToolResult};

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
                output: "Error: empty command".to_string(),
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
                output: format!(
                    "Error: command '{}' is not in the allowed list. Allowed: {}",
                    base,
                    self.security.allowed_commands.join(", ")
                ),
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
                    output: format!("Error: working_dir '{}' is outside workspace and allowed paths", dir),
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
                    output: format!("Error: command timed out after {} seconds", timeout_secs),
                });
            }
        };

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let combined = if stderr.is_empty() {
                    stdout.to_string()
                } else if stdout.is_empty() {
                    stderr.to_string()
                } else {
                    format!("{}\n--- stderr ---\n{}", stdout, stderr)
                };

                // Truncate very long output (safe for multi-byte UTF-8)
                let truncated = if combined.len() > 4000 {
                    let end = floor_char_boundary(&combined, 4000);
                    format!("{}...\n(truncated, {} bytes total)", &combined[..end], combined.len())
                } else {
                    combined
                };

                Ok(ToolResult {
                    success: out.status.success(),
                    output: truncated,
                })
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: format!("Failed to execute command: {}", e),
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

/// Find the largest valid char boundary <= byte index (safe for multi-byte UTF-8)
fn floor_char_boundary(s: &str, byte_index: usize) -> usize {
    if byte_index >= s.len() {
        return s.len();
    }
    let mut i = byte_index;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
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

    #[tokio::test]
    async fn test_execute_echo() {
        let tool = make_tool();
        let result = tool.execute(json!({"command": "echo hello"})).await.unwrap();
        assert!(result.success, "Shell failed: {}", result.output);
        assert!(result.output.contains("hello"));
    }

    #[tokio::test]
    async fn test_execute_blocked() {
        let tool = make_tool();
        let result = tool.execute(json!({"command": "curl http://x"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("not in the allowed list"));
    }
}
