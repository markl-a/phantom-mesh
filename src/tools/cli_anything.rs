// cli_anything tool — generate and execute CLI interfaces for any software
// Uses CLI-Anything (https://github.com/HKUDS/CLI-Anything) to create CLI wrappers
// for desktop applications like GIMP, Blender, FFmpeg, etc.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{info, warn};

use super::{Tool, ToolResult};

/// Maximum output length before truncation (bytes)
const MAX_OUTPUT_BYTES: usize = 8000;

/// Default timeout for execute actions (seconds)
const EXECUTE_TIMEOUT_SECS: u64 = 60;

/// Timeout for generate actions — CLI generation can take longer
const GENERATE_TIMEOUT_SECS: u64 = 120;

pub struct CliAnythingTool {
    cli_available: bool,
}

impl CliAnythingTool {
    pub fn new() -> Self {
        let cli_available = detect_cli_anything();
        if cli_available {
            info!("cli_anything: CLI-Anything detected");
        } else {
            warn!("cli_anything: CLI-Anything not found (install with: pip install cli-anything)");
        }
        Self { cli_available }
    }
}

/// Check if cli-anything is installed
fn detect_cli_anything() -> bool {
    let probe = if cfg!(target_os = "windows") {
        std::process::Command::new("where")
            .arg("cli-anything")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
    } else {
        std::process::Command::new("which")
            .arg("cli-anything")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
    };
    matches!(probe, Ok(status) if status.success())
}

/// Truncate output using head+tail strategy (80% head / 20% tail)
fn truncate_output(output: &str, max_bytes: usize) -> String {
    if output.len() <= max_bytes {
        return output.to_string();
    }

    let head_bytes = (max_bytes as f64 * 0.8) as usize;
    let tail_bytes = max_bytes - head_bytes;

    // Find safe char boundaries
    let head_end = output.char_indices()
        .take_while(|(i, _)| *i < head_bytes)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);

    let tail_start = output.char_indices()
        .rev()
        .take_while(|(i, _)| output.len() - *i <= tail_bytes)
        .last()
        .map(|(i, _)| i)
        .unwrap_or(output.len());

    let omitted = output.len() - head_end - (output.len() - tail_start);
    format!(
        "{}\n\n... ({} bytes omitted) ...\n\n{}",
        &output[..head_end],
        omitted,
        &output[tail_start..]
    )
}

/// Build the subprocess command for CLI-Anything
fn build_generate_command(software: &str, task: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("cli-anything");
    cmd.arg("generate")
        .arg("--software").arg(software);
    if !task.is_empty() {
        cmd.arg("--task").arg(task);
    }
    cmd
}

fn build_execute_command(software: &str, command: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("cli-anything");
    cmd.arg("exec")
        .arg("--software").arg(software)
        .arg("--command").arg(command);
    cmd
}

#[async_trait]
impl Tool for CliAnythingTool {
    fn name(&self) -> &str { "cli_anything" }

    fn description(&self) -> &str {
        "Generate and execute CLI interfaces for any software using CLI-Anything. \
         Supports generating CLI wrappers for desktop apps (GIMP, Blender, FFmpeg, etc.) \
         and executing commands through them."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["generate", "execute"],
                    "description": "generate = create CLI interface for software, execute = run a CLI command"
                },
                "software": {
                    "type": "string",
                    "description": "Target software name (e.g. gimp, ffmpeg, blender)"
                },
                "command": {
                    "type": "string",
                    "description": "CLI command to execute (required for action=execute)"
                },
                "task": {
                    "type": "string",
                    "description": "Task description for CLI generation (optional, for action=generate)"
                }
            },
            "required": ["action", "software"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        // 1. Extract action
        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => {
                return Ok(ToolResult {
                    success: false,
                    output: "Error: 'action' is required (generate or execute)".to_string(),
                });
            }
        };

        if action != "generate" && action != "execute" {
            return Ok(ToolResult {
                success: false,
                output: format!("Error: invalid action '{}'. Must be 'generate' or 'execute'", action),
            });
        }

        // 2. Extract software
        let software = match args.get("software").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => {
                return Ok(ToolResult {
                    success: false,
                    output: "Error: 'software' is required and must not be empty".to_string(),
                });
            }
        };

        // 3. Check if cli-anything is installed
        if !self.cli_available {
            return Ok(ToolResult {
                success: false,
                output: "Error: cli-anything is not installed. Install with: pip install cli-anything (or git clone https://github.com/HKUDS/CLI-Anything && pip install -e .)".to_string(),
            });
        }

        // 4. Build and execute command based on action
        match action {
            "generate" => {
                let task = args.get("task").and_then(|v| v.as_str()).unwrap_or("");
                info!("cli_anything: generating CLI for '{}' (task: {})", software,
                    if task.is_empty() { "default" } else { task });

                let mut cmd = build_generate_command(software, task);
                let timeout = Duration::from_secs(GENERATE_TIMEOUT_SECS);

                match tokio::time::timeout(timeout, cmd.output()).await {
                    Ok(Ok(output)) => {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let stderr = String::from_utf8_lossy(&output.stderr);

                        if !output.status.success() {
                            let error_msg = if stderr.is_empty() { stdout.to_string() } else { stderr.to_string() };
                            Ok(ToolResult {
                                success: false,
                                output: format!("CLI generation failed for '{}':\n{}", software,
                                    truncate_output(&error_msg, MAX_OUTPUT_BYTES)),
                            })
                        } else {
                            let result = truncate_output(&stdout, MAX_OUTPUT_BYTES);
                            info!("cli_anything: generated CLI for '{}' ({} bytes)", software, stdout.len());
                            Ok(ToolResult {
                                success: true,
                                output: format!("CLI interface generated for '{}':\n{}", software, result),
                            })
                        }
                    }
                    Ok(Err(e)) => Ok(ToolResult {
                        success: false,
                        output: format!("Error: failed to execute cli-anything: {}", e),
                    }),
                    Err(_) => {
                        warn!("cli_anything: generate timed out after {}s", GENERATE_TIMEOUT_SECS);
                        Ok(ToolResult {
                            success: false,
                            output: format!("Error: CLI generation timed out after {} seconds", GENERATE_TIMEOUT_SECS),
                        })
                    }
                }
            }
            "execute" => {
                let command = match args.get("command").and_then(|v| v.as_str()) {
                    Some(c) if !c.is_empty() => c,
                    _ => {
                        return Ok(ToolResult {
                            success: false,
                            output: "Error: 'command' is required for action=execute".to_string(),
                        });
                    }
                };

                info!("cli_anything: executing '{}' command for '{}'", command, software);

                let mut cmd = build_execute_command(software, command);
                let timeout = Duration::from_secs(EXECUTE_TIMEOUT_SECS);

                match tokio::time::timeout(timeout, cmd.output()).await {
                    Ok(Ok(output)) => {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let stderr = String::from_utf8_lossy(&output.stderr);

                        if !output.status.success() {
                            let error_msg = if stderr.is_empty() { stdout.to_string() } else { stderr.to_string() };
                            Ok(ToolResult {
                                success: false,
                                output: format!("Command execution failed:\n{}", truncate_output(&error_msg, MAX_OUTPUT_BYTES)),
                            })
                        } else {
                            let combined = if stderr.is_empty() {
                                stdout.to_string()
                            } else {
                                format!("{}\n[stderr]: {}", stdout, stderr)
                            };
                            let result = truncate_output(&combined, MAX_OUTPUT_BYTES);
                            info!("cli_anything: executed command for '{}' ({} bytes)", software, combined.len());
                            Ok(ToolResult {
                                success: true,
                                output: result,
                            })
                        }
                    }
                    Ok(Err(e)) => Ok(ToolResult {
                        success: false,
                        output: format!("Error: failed to execute cli-anything: {}", e),
                    }),
                    Err(_) => {
                        warn!("cli_anything: execute timed out after {}s", EXECUTE_TIMEOUT_SECS);
                        Ok(ToolResult {
                            success: false,
                            output: format!("Error: command execution timed out after {} seconds", EXECUTE_TIMEOUT_SECS),
                        })
                    }
                }
            }
            _ => unreachable!("action validated above"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool(available: bool) -> CliAnythingTool {
        CliAnythingTool { cli_available: available }
    }

    #[tokio::test]
    async fn test_cli_not_installed() {
        let tool = make_tool(false);
        let result = tool.execute(json!({
            "action": "generate",
            "software": "ffmpeg"
        })).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("not installed"));
        assert!(result.output.contains("pip install"));
    }

    #[tokio::test]
    async fn test_missing_action() {
        let tool = make_tool(true);
        let result = tool.execute(json!({
            "software": "ffmpeg"
        })).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("action"));
    }

    #[tokio::test]
    async fn test_invalid_action() {
        let tool = make_tool(true);
        let result = tool.execute(json!({
            "action": "invalid",
            "software": "ffmpeg"
        })).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("invalid"));
    }

    #[tokio::test]
    async fn test_missing_software() {
        let tool = make_tool(true);
        let result = tool.execute(json!({
            "action": "generate"
        })).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("software"));
    }

    #[tokio::test]
    async fn test_empty_software() {
        let tool = make_tool(true);
        let result = tool.execute(json!({
            "action": "generate",
            "software": ""
        })).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("software"));
    }

    #[tokio::test]
    async fn test_execute_missing_command() {
        let tool = make_tool(true);
        // cli_available=true but cli-anything is not actually installed — the error
        // about missing command should come before attempting subprocess
        let result = tool.execute(json!({
            "action": "execute",
            "software": "ffmpeg"
        })).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("command"));
    }

    #[test]
    fn test_generate_command_construction() {
        let cmd = build_generate_command("gimp", "resize an image");
        let prog = cmd.as_std().get_program().to_string_lossy().to_string();
        let args: Vec<String> = cmd.as_std().get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(prog, "cli-anything");
        assert!(args.contains(&"generate".to_string()));
        assert!(args.contains(&"--software".to_string()));
        assert!(args.contains(&"gimp".to_string()));
        assert!(args.contains(&"--task".to_string()));
        assert!(args.contains(&"resize an image".to_string()));
    }

    #[test]
    fn test_execute_command_construction() {
        let cmd = build_execute_command("ffmpeg", "convert input.mp4 output.gif");
        let prog = cmd.as_std().get_program().to_string_lossy().to_string();
        let args: Vec<String> = cmd.as_std().get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(prog, "cli-anything");
        assert!(args.contains(&"exec".to_string()));
        assert!(args.contains(&"--software".to_string()));
        assert!(args.contains(&"ffmpeg".to_string()));
        assert!(args.contains(&"--command".to_string()));
        assert!(args.contains(&"convert input.mp4 output.gif".to_string()));
    }

    #[test]
    fn test_generate_command_no_task() {
        let cmd = build_generate_command("blender", "");
        let args: Vec<String> = cmd.as_std().get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert!(args.contains(&"generate".to_string()));
        assert!(args.contains(&"--software".to_string()));
        assert!(args.contains(&"blender".to_string()));
        // No --task flag when task is empty
        assert!(!args.contains(&"--task".to_string()));
    }

    #[test]
    fn test_output_truncation() {
        let long_output = "A".repeat(20000);
        let truncated = truncate_output(&long_output, MAX_OUTPUT_BYTES);
        assert!(truncated.len() < MAX_OUTPUT_BYTES + 100, "Truncated output too long: {}", truncated.len());
        assert!(truncated.contains("bytes omitted"));
        assert!(truncated.starts_with("AAAA"));
        assert!(truncated.ends_with("AAAA"));
    }

    #[test]
    fn test_no_truncation_when_short() {
        let short = "Hello world";
        let result = truncate_output(short, MAX_OUTPUT_BYTES);
        assert_eq!(result, short);
    }

    #[test]
    fn test_tool_metadata() {
        let tool = make_tool(false);
        assert_eq!(tool.name(), "cli_anything");
        assert!(tool.description().contains("CLI"));
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("action").is_some());
        assert!(props.get("software").is_some());
        assert!(props.get("command").is_some());
        assert!(props.get("task").is_some());
    }
}
