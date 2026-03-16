// ai_code tool — delegate complex coding/reasoning tasks to external AI CLIs
// Supports: claude, gemini, codex (auto-detected at startup)

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{info, warn};

use super::{SecurityConfig, Tool, ToolResult};

/// Configuration for ai_code tool (deserializes from [ai_code] in agents.toml)
#[derive(Debug, Clone, Deserialize)]
pub struct AiCodeConfig {
    #[serde(default = "default_tool")]
    pub default_tool: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_output")]
    pub max_output_bytes: usize,
    #[serde(default)]
    pub models: HashMap<String, String>,
}

fn default_tool() -> String { "claude".to_string() }
fn default_timeout() -> u64 { 120 }
fn default_max_output() -> usize { 12000 }

impl Default for AiCodeConfig {
    fn default() -> Self {
        Self {
            default_tool: default_tool(),
            timeout_secs: default_timeout(),
            max_output_bytes: default_max_output(),
            models: HashMap::new(),
        }
    }
}

/// Supported AI CLI backends
const SUPPORTED_TOOLS: &[&str] = &["claude", "gemini", "codex"];

pub struct AiCodeTool {
    config: AiCodeConfig,
    security: SecurityConfig,
    available_tools: Vec<String>,
}

impl AiCodeTool {
    pub fn new(config: AiCodeConfig, security: SecurityConfig) -> Self {
        let available_tools = detect_available_tools();
        if available_tools.is_empty() {
            warn!("ai_code: no AI CLI tools detected (checked: {})", SUPPORTED_TOOLS.join(", "));
        } else {
            info!("ai_code: available backends: {}", available_tools.join(", "));
        }
        Self {
            config,
            security,
            available_tools,
        }
    }
}

/// Probe which AI CLIs are installed using `where` (Windows) or `which` (Unix)
fn detect_available_tools() -> Vec<String> {
    let mut available = Vec::new();
    for &tool_name in SUPPORTED_TOOLS {
        let probe = if cfg!(target_os = "windows") {
            std::process::Command::new("where")
                .arg(tool_name)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
        } else {
            std::process::Command::new("which")
                .arg(tool_name)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
        };
        if let Ok(status) = probe {
            if status.success() {
                available.push(tool_name.to_string());
            }
        }
    }
    available
}

/// Build the command arguments for each AI CLI backend
fn build_command(tool_name: &str, prompt: &str, model: Option<&str>, working_dir: &PathBuf) -> tokio::process::Command {
    match tool_name {
        "claude" => {
            let mut cmd = tokio::process::Command::new("claude");
            cmd.arg("-p").arg(prompt).arg("--output-format").arg("json");
            if let Some(m) = model {
                cmd.arg("--model").arg(m);
            }
            cmd.current_dir(working_dir);
            cmd
        }
        "gemini" => {
            let mut cmd = tokio::process::Command::new("gemini");
            cmd.arg("-p").arg(prompt).arg("--sandbox");
            if let Some(m) = model {
                cmd.arg("--model").arg(m);
            }
            cmd.current_dir(working_dir);
            cmd
        }
        "codex" => {
            let mut cmd = tokio::process::Command::new("codex");
            cmd.arg("exec").arg(prompt).arg("--full-auto");
            if let Some(m) = model {
                cmd.arg("--model").arg(m);
            }
            cmd.current_dir(working_dir);
            cmd
        }
        _ => unreachable!("unsupported tool validated before build_command"),
    }
}

/// Parse output from AI CLI — Claude returns JSON with a `result` field, others return plain text
fn parse_output(tool_name: &str, raw: &str) -> String {
    if tool_name == "claude" {
        // Claude --output-format json returns JSON; try to extract the result field
        if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
            if let Some(result) = parsed.get("result").and_then(|v| v.as_str()) {
                return result.to_string();
            }
            // Fallback: some versions use "content" or return the text directly
            if let Some(content) = parsed.get("content").and_then(|v| v.as_str()) {
                return content.to_string();
            }
        }
        // If JSON parsing fails, return raw output
        raw.to_string()
    } else {
        raw.to_string()
    }
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

/// Validate that a working directory is within the workspace
fn validate_working_dir(requested: &str, security: &SecurityConfig) -> Result<PathBuf, String> {
    let workspace = security.workspace_path();
    let workspace_canonical = workspace.canonicalize()
        .unwrap_or_else(|_| workspace.clone());

    let requested_path = PathBuf::from(requested);
    let resolved = if requested_path.is_absolute() {
        requested_path
    } else {
        workspace.join(&requested_path)
    };

    let resolved_canonical = resolved.canonicalize()
        .map_err(|e| format!("Cannot resolve working directory '{}': {}", requested, e))?;

    if resolved_canonical.starts_with(&workspace_canonical) {
        Ok(resolved_canonical)
    } else {
        Err(format!(
            "Working directory '{}' is outside workspace '{}'",
            requested,
            security.workspace_dir
        ))
    }
}

#[async_trait]
impl Tool for AiCodeTool {
    fn name(&self) -> &str { "ai_code" }

    fn description(&self) -> &str {
        "Delegate complex coding/reasoning tasks to an external AI CLI (Claude Code, Gemini, Codex). Use for code generation, deep analysis, or tasks needing a more powerful model."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The coding/reasoning prompt to send to the external AI tool"
                },
                "tool": {
                    "type": "string",
                    "description": "Which AI CLI to use: 'claude', 'gemini', or 'codex' (default: from config)",
                    "enum": ["claude", "gemini", "codex"]
                },
                "model": {
                    "type": "string",
                    "description": "Optional model override for the selected tool"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Optional working directory (must be within workspace)"
                }
            },
            "required": ["prompt"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        // 1. Extract and validate prompt
        let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
        if prompt.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Error: 'prompt' is required and must not be empty".to_string(),
            });
        }

        // 2. Resolve tool name
        let tool_name = args.get("tool")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.config.default_tool);

        if !SUPPORTED_TOOLS.contains(&tool_name) {
            return Ok(ToolResult {
                success: false,
                output: format!(
                    "Error: unsupported tool '{}'. Supported: {}",
                    tool_name,
                    SUPPORTED_TOOLS.join(", ")
                ),
            });
        }

        if !self.available_tools.contains(&tool_name.to_string()) {
            return Ok(ToolResult {
                success: false,
                output: format!(
                    "Error: '{}' is not installed on this system. Available: {}",
                    tool_name,
                    if self.available_tools.is_empty() {
                        "none".to_string()
                    } else {
                        self.available_tools.join(", ")
                    }
                ),
            });
        }

        // 3. Resolve model (explicit arg > config models map > none)
        let model = args.get("model")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| self.config.models.get(tool_name).cloned());

        // 4. Resolve working directory
        let working_dir = if let Some(dir) = args.get("working_dir").and_then(|v| v.as_str()) {
            match validate_working_dir(dir, &self.security) {
                Ok(path) => path,
                Err(e) => {
                    return Ok(ToolResult {
                        success: false,
                        output: format!("Error: {}", e),
                    });
                }
            }
        } else {
            self.security.workspace_path()
        };

        // Ensure working dir exists
        let _ = std::fs::create_dir_all(&working_dir);

        info!(
            "ai_code: running {} (model: {}) — prompt: {}...",
            tool_name,
            model.as_deref().unwrap_or("default"),
            truncate_str(prompt, 80)
        );

        // 5. Build and execute command with timeout
        let mut cmd = build_command(tool_name, prompt, model.as_deref(), &working_dir);
        let timeout = Duration::from_secs(self.config.timeout_secs);

        let output = match tokio::time::timeout(timeout, cmd.output()).await {
            Ok(inner) => inner,
            Err(_) => {
                warn!("ai_code: {} timed out after {}s", tool_name, self.config.timeout_secs);
                return Ok(ToolResult {
                    success: false,
                    output: format!(
                        "Error: {} timed out after {} seconds. Try a simpler prompt or increase timeout.",
                        tool_name, self.config.timeout_secs
                    ),
                });
            }
        };

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);

                if !out.status.success() {
                    let error_msg = if stderr.is_empty() {
                        stdout.to_string()
                    } else {
                        stderr.to_string()
                    };
                    return Ok(ToolResult {
                        success: false,
                        output: format!("Error: {} exited with error:\n{}", tool_name, truncate_output(&error_msg, self.config.max_output_bytes)),
                    });
                }

                // 6. Parse output (extract from Claude JSON, plain text for others)
                let parsed = parse_output(tool_name, &stdout);

                // 7. Truncate if needed
                let final_output = truncate_output(&parsed, self.config.max_output_bytes);

                info!(
                    "ai_code: {} completed ({} bytes output)",
                    tool_name, parsed.len()
                );

                Ok(ToolResult {
                    success: true,
                    output: final_output,
                })
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: format!("Error: failed to execute {}: {}", tool_name, e),
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

    fn make_config() -> AiCodeConfig {
        AiCodeConfig {
            default_tool: "claude".to_string(),
            timeout_secs: 120,
            max_output_bytes: 12000,
            models: HashMap::new(),
        }
    }

    fn make_security() -> SecurityConfig {
        let dir = std::env::temp_dir().join("clawtex_test_aicode");
        let _ = std::fs::create_dir_all(&dir);
        SecurityConfig {
            workspace_dir: dir.to_string_lossy().to_string(),
            workspace_only: true,
            allowed_commands: super::super::default_allowed_commands(),
            ..Default::default()
        }
    }

    fn make_tool_with_available(available: Vec<String>) -> AiCodeTool {
        AiCodeTool {
            config: make_config(),
            security: make_security(),
            available_tools: available,
        }
    }

    #[tokio::test]
    async fn test_empty_prompt_rejected() {
        let tool = make_tool_with_available(vec!["claude".to_string()]);
        let result = tool.execute(json!({"prompt": ""})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("required"));
    }

    #[tokio::test]
    async fn test_missing_prompt_rejected() {
        let tool = make_tool_with_available(vec!["claude".to_string()]);
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("required"));
    }

    #[tokio::test]
    async fn test_unsupported_tool_rejected() {
        let tool = make_tool_with_available(vec!["claude".to_string()]);
        let result = tool.execute(json!({"prompt": "hello", "tool": "gpt4"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("unsupported"));
    }

    #[tokio::test]
    async fn test_unavailable_tool_rejected() {
        let tool = make_tool_with_available(vec![]); // nothing available
        let result = tool.execute(json!({"prompt": "hello", "tool": "claude"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("not installed"));
    }

    #[test]
    fn test_output_truncation() {
        let long_output = "A".repeat(20000);
        let truncated = truncate_output(&long_output, 12000);
        // Should be around max_output_bytes plus the omission notice
        assert!(truncated.len() < 13000, "Truncated output too long: {}", truncated.len());
        assert!(truncated.contains("bytes omitted"));
        assert!(truncated.starts_with("AAAA"));
        assert!(truncated.ends_with("AAAA"));
    }

    #[test]
    fn test_no_truncation_when_short() {
        let short = "Hello world";
        let result = truncate_output(short, 12000);
        assert_eq!(result, short);
    }

    #[test]
    fn test_parse_claude_json() {
        let json_output = r#"{"result": "def hello():\n    print('hello')"}"#;
        let parsed = parse_output("claude", json_output);
        assert_eq!(parsed, "def hello():\n    print('hello')");
    }

    #[test]
    fn test_parse_claude_json_with_content_field() {
        let json_output = r#"{"content": "some content here"}"#;
        let parsed = parse_output("claude", json_output);
        assert_eq!(parsed, "some content here");
    }

    #[test]
    fn test_parse_claude_plain_fallback() {
        let plain = "This is not JSON";
        let parsed = parse_output("claude", plain);
        assert_eq!(parsed, plain);
    }

    #[test]
    fn test_parse_gemini_plain() {
        let plain = "Generated code here";
        let parsed = parse_output("gemini", plain);
        assert_eq!(parsed, plain);
    }

    #[test]
    fn test_default_config() {
        let config = AiCodeConfig::default();
        assert_eq!(config.default_tool, "claude");
        assert_eq!(config.timeout_secs, 120);
        assert_eq!(config.max_output_bytes, 12000);
    }
}
