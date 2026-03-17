//! Clipboard tool — copy text to clipboard and paste text from clipboard.
//! Uses platform-specific commands: clip.exe/PowerShell (Windows), pbcopy/pbpaste (macOS),
//! xclip/xsel (Linux).

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolResult};

pub struct ClipboardTool;

impl ClipboardTool {
    pub fn new() -> Self {
        Self
    }

    /// Write text to the system clipboard using platform-specific commands.
    async fn write_clipboard(text: &str) -> Result<String> {
        #[cfg(target_os = "windows")]
        {
            // Use clip.exe via echo piped through PowerShell to handle Unicode correctly
            let mut child = tokio::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Set-Clipboard -Value $input",
                ])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()?;

            if let Some(stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let mut stdin = stdin;
                stdin.write_all(text.as_bytes()).await?;
                drop(stdin);
            }

            let output = child.wait_with_output().await?;
            if output.status.success() {
                Ok(format!("Copied {} characters to clipboard.", text.len()))
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // Fallback: try clip.exe
                Self::write_clipboard_clip_exe(text).await
                    .map_err(|_| anyhow::anyhow!("PowerShell clipboard failed: {}", stderr))
            }
        }

        #[cfg(target_os = "macos")]
        {
            let mut child = tokio::process::Command::new("pbcopy")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()?;

            if let Some(stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let mut stdin = stdin;
                stdin.write_all(text.as_bytes()).await?;
                drop(stdin);
            }

            let output = child.wait_with_output().await?;
            if output.status.success() {
                Ok(format!("Copied {} characters to clipboard.", text.len()))
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(anyhow::anyhow!("pbcopy failed: {}", stderr))
            }
        }

        #[cfg(target_os = "linux")]
        {
            // Try xclip first, then xsel
            let programs = [
                ("xclip", vec!["-selection", "clipboard"]),
                ("xsel", vec!["--clipboard", "--input"]),
            ];

            for (program, args) in &programs {
                let mut child = tokio::process::Command::new(program)
                    .args(args)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn();

                if let Ok(mut child) = child {
                    if let Some(stdin) = child.stdin.take() {
                        use tokio::io::AsyncWriteExt;
                        let mut stdin = stdin;
                        let _ = stdin.write_all(text.as_bytes()).await;
                        drop(stdin);
                    }
                    if let Ok(output) = child.wait_with_output().await {
                        if output.status.success() {
                            return Ok(format!("Copied {} characters to clipboard via {}.", text.len(), program));
                        }
                    }
                }
            }
            Err(anyhow::anyhow!("No clipboard command found. Install xclip or xsel."))
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            Err(anyhow::anyhow!("Clipboard not supported on this platform."))
        }
    }

    /// Fallback: write to clipboard using clip.exe (Windows only, ASCII-safe).
    #[cfg(target_os = "windows")]
    async fn write_clipboard_clip_exe(text: &str) -> Result<String> {
        let mut child = tokio::process::Command::new("cmd")
            .args(["/c", "clip"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        if let Some(stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let mut stdin = stdin;
            stdin.write_all(text.as_bytes()).await?;
            drop(stdin);
        }

        let output = child.wait_with_output().await?;
        if output.status.success() {
            Ok(format!("Copied {} characters to clipboard via clip.exe.", text.len()))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(anyhow::anyhow!("clip.exe failed: {}", stderr))
        }
    }

    /// Read text from the system clipboard.
    async fn read_clipboard() -> Result<String> {
        #[cfg(target_os = "windows")]
        {
            let output = tokio::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Get-Clipboard",
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .await?;

            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                Ok(text)
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(anyhow::anyhow!("PowerShell Get-Clipboard failed: {}", stderr))
            }
        }

        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("pbpaste")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .await?;

            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(anyhow::anyhow!("pbpaste failed: {}", stderr))
            }
        }

        #[cfg(target_os = "linux")]
        {
            let programs = [
                ("xclip", vec!["-selection", "clipboard", "-out"]),
                ("xsel", vec!["--clipboard", "--output"]),
            ];

            for (program, args) in &programs {
                if let Ok(output) = tokio::process::Command::new(program)
                    .args(args)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .output()
                    .await
                {
                    if output.status.success() {
                        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
                    }
                }
            }
            Err(anyhow::anyhow!("No clipboard command found. Install xclip or xsel."))
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            Err(anyhow::anyhow!("Clipboard not supported on this platform."))
        }
    }

    /// Check if clipboard commands are available on the current platform.
    fn available_commands() -> Vec<&'static str> {
        #[cfg(target_os = "windows")]
        return vec!["powershell (Set-Clipboard/Get-Clipboard)", "clip.exe"];

        #[cfg(target_os = "macos")]
        return vec!["pbcopy", "pbpaste"];

        #[cfg(target_os = "linux")]
        return vec!["xclip", "xsel"];

        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        return vec![];
    }
}

#[async_trait]
impl Tool for ClipboardTool {
    fn name(&self) -> &str {
        "clipboard"
    }

    fn description(&self) -> &str {
        "Read from or write to the system clipboard. Operations: copy (write text to clipboard), paste (read text from clipboard), info (show available clipboard commands)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "One of: copy, paste, info",
                    "enum": ["copy", "paste", "info"]
                },
                "text": {
                    "type": "string",
                    "description": "Text to copy to clipboard (required for 'copy' operation)"
                }
            },
            "required": ["operation"]
        })
    }

    fn preflight(&self, args: &Value) -> Result<()> {
        let operation = args.get("operation").and_then(|v| v.as_str()).unwrap_or("");
        if operation.is_empty() {
            anyhow::bail!("Preflight: 'operation' is required");
        }
        if !["copy", "paste", "info"].contains(&operation) {
            anyhow::bail!("Preflight: unknown operation '{}'. Use: copy, paste, info", operation);
        }
        if operation == "copy" {
            let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
            if text.is_empty() {
                anyhow::bail!("Preflight: 'text' is required for copy operation");
            }
        }
        Ok(())
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let operation = args["operation"].as_str().unwrap_or("").trim();

        if operation.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Missing required parameter: operation".into(),
            });
        }

        match operation {
            "copy" => {
                let text = args["text"].as_str().unwrap_or("").trim();
                if text.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: "Missing required parameter: text (for copy operation)".into(),
                    });
                }
                match Self::write_clipboard(text).await {
                    Ok(msg) => Ok(ToolResult { success: true, output: msg }),
                    Err(e) => Ok(ToolResult {
                        success: false,
                        output: format!("Failed to copy to clipboard: {}", e),
                    }),
                }
            }
            "paste" => {
                match Self::read_clipboard().await {
                    Ok(text) => {
                        let len = text.len();
                        let result = json!({
                            "length": len,
                            "text": text,
                        });
                        Ok(ToolResult {
                            success: true,
                            output: serde_json::to_string_pretty(&result)?,
                        })
                    }
                    Err(e) => Ok(ToolResult {
                        success: false,
                        output: format!("Failed to read from clipboard: {}", e),
                    }),
                }
            }
            "info" => {
                let cmds = Self::available_commands();
                let result = json!({
                    "platform": std::env::consts::OS,
                    "available_commands": cmds,
                });
                Ok(ToolResult {
                    success: true,
                    output: serde_json::to_string_pretty(&result)?,
                })
            }
            _ => Ok(ToolResult {
                success: false,
                output: format!("Unknown operation: '{}'. Use: copy, paste, info", operation),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_name() {
        let tool = ClipboardTool::new();
        assert_eq!(tool.name(), "clipboard");
    }

    #[test]
    fn test_description_not_empty() {
        let tool = ClipboardTool::new();
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn test_schema() {
        let tool = ClipboardTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["operation"].is_object());
        assert!(schema["properties"]["text"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("operation")));
    }

    #[test]
    fn test_schema_enum_values() {
        let tool = ClipboardTool::new();
        let schema = tool.parameters_schema();
        let ops = schema["properties"]["operation"]["enum"].as_array().unwrap();
        let op_strings: Vec<&str> = ops.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(op_strings.contains(&"copy"));
        assert!(op_strings.contains(&"paste"));
        assert!(op_strings.contains(&"info"));
    }

    #[test]
    fn test_preflight_missing_operation() {
        let tool = ClipboardTool::new();
        let result = tool.preflight(&json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("operation"));
    }

    #[test]
    fn test_preflight_invalid_operation() {
        let tool = ClipboardTool::new();
        let result = tool.preflight(&json!({"operation": "clear"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown operation"));
    }

    #[test]
    fn test_preflight_copy_missing_text() {
        let tool = ClipboardTool::new();
        let result = tool.preflight(&json!({"operation": "copy"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("text"));
    }

    #[test]
    fn test_preflight_copy_with_text() {
        let tool = ClipboardTool::new();
        let result = tool.preflight(&json!({"operation": "copy", "text": "hello"}));
        assert!(result.is_ok());
    }

    #[test]
    fn test_preflight_paste_valid() {
        let tool = ClipboardTool::new();
        let result = tool.preflight(&json!({"operation": "paste"}));
        assert!(result.is_ok());
    }

    #[test]
    fn test_preflight_info_valid() {
        let tool = ClipboardTool::new();
        let result = tool.preflight(&json!({"operation": "info"}));
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_missing_operation() {
        let tool = ClipboardTool::new();
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_execute_unknown_operation() {
        let tool = ClipboardTool::new();
        let result = tool.execute(json!({"operation": "clear"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown operation"));
    }

    #[tokio::test]
    async fn test_execute_copy_missing_text() {
        let tool = ClipboardTool::new();
        let result = tool.execute(json!({"operation": "copy"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_execute_info() {
        let tool = ClipboardTool::new();
        let result = tool.execute(json!({"operation": "info"})).await.unwrap();
        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert!(parsed["platform"].is_string());
        assert!(parsed["available_commands"].is_array());
    }

    #[test]
    fn test_available_commands_not_empty() {
        let cmds = ClipboardTool::available_commands();
        // Should have at least one command on any supported platform
        assert!(!cmds.is_empty());
    }

    #[tokio::test]
    async fn test_execute_copy_and_paste_roundtrip() {
        // Note: This test requires a display/clipboard server to be available.
        // It will succeed on Windows (powershell) and macOS (pbcopy/pbpaste),
        // but may fail on headless Linux without xclip/xsel.
        let tool = ClipboardTool::new();
        let test_text = "clawtex clipboard test 12345";

        let copy_result = tool
            .execute(json!({"operation": "copy", "text": test_text}))
            .await
            .unwrap();

        if copy_result.success {
            let paste_result = tool
                .execute(json!({"operation": "paste"}))
                .await
                .unwrap();

            if paste_result.success {
                let parsed: Value = serde_json::from_str(&paste_result.output).unwrap();
                let pasted_text = parsed["text"].as_str().unwrap_or("").trim();
                assert_eq!(pasted_text, test_text);
            }
            // If paste fails (e.g., no display), that's acceptable in CI
        }
        // If copy fails (e.g., no display server), skip without failing
    }
}
