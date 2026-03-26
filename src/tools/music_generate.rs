//! music_generate tool — generates music via external APIs (Suno, Udio, or local AudioCraft).
//! Actions: "generate" (create from prompt), "extend" (extend existing track), "remix" (remix from audio file).

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{debug, warn};

use super::{Tool, ToolResult};

pub struct MusicGenerateTool;

impl MusicGenerateTool {
    pub fn new() -> Self {
        Self
    }

    /// Build the output file path (in workspace).
    fn build_output_path(output_path: Option<&str>) -> std::path::PathBuf {
        if let Some(p) = output_path {
            std::path::PathBuf::from(p)
        } else {
            let workspace = dirs::home_dir()
                .unwrap_or_default()
                .join(".phantom-mesh")
                .join("workspace");
            let _ = std::fs::create_dir_all(&workspace);
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            workspace.join(format!("music_{}.mp3", timestamp))
        }
    }

    /// Build a full prompt from the base prompt and optional style.
    fn build_full_prompt(prompt: &str, style: Option<&str>) -> String {
        match style {
            Some(s) if !s.is_empty() => format!("{}, {} style", prompt, s),
            _ => prompt.to_string(),
        }
    }

    /// Validate the action parameter.
    fn validate_action(action: &str) -> Result<(), String> {
        match action {
            "generate" | "extend" | "remix" => Ok(()),
            other => Err(format!("Unknown action '{}'. Use 'generate', 'extend', or 'remix'.", other)),
        }
    }

    /// Validate the provider parameter.
    fn validate_provider(provider: &str) -> Result<(), String> {
        match provider {
            "suno" | "udio" | "local" => Ok(()),
            other => Err(format!("Unknown provider '{}'. Use 'suno', 'udio', or 'local'.", other)),
        }
    }

    /// Validate duration_secs is within reasonable bounds.
    fn validate_duration(duration_secs: u64) -> Result<(), String> {
        if duration_secs == 0 {
            return Err("duration_secs must be greater than 0".to_string());
        }
        if duration_secs > 600 {
            return Err("duration_secs cannot exceed 600 (10 minutes)".to_string());
        }
        Ok(())
    }

    /// Execute Suno provider via HTTP API.
    async fn execute_suno(
        &self,
        action: &str,
        prompt: &str,
        duration_secs: u64,
        output: &std::path::Path,
    ) -> Result<ToolResult> {
        let api_key = match std::env::var("SUNO_API_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => {
                return Ok(ToolResult {
                    success: false,
                    output: "SUNO_API_KEY environment variable not set".to_string(),
                });
            }
        };

        let url = "https://api.suno.ai/v1/generate";

        let body = json!({
            "action": action,
            "prompt": prompt,
            "duration_secs": duration_secs,
        });

        debug!("Calling Suno API: action={}, duration={}s", action, duration_secs);

        let client = reqwest::Client::new();
        let resp = client.post(url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .timeout(std::time::Duration::from_secs(300))
            .send()
            .await;

        match resp {
            Ok(response) => {
                let status = response.status();
                if !status.is_success() {
                    let err_text = response.text().await.unwrap_or_default();
                    warn!("Suno API error {}: {}", status, err_text);
                    return Ok(ToolResult {
                        success: false,
                        output: format!("Suno API error ({}): {}", status, truncate(&err_text, 500)),
                    });
                }

                let bytes = response.bytes().await?;

                if let Some(parent) = output.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(output, &bytes)?;

                Ok(ToolResult {
                    success: true,
                    output: format!(
                        "Music generated successfully!\nPath: {}\nSize: {} bytes\nProvider: suno\nAction: {}\nDuration: {}s\nPrompt: {}",
                        output.display(), bytes.len(), action, duration_secs, prompt
                    ),
                })
            }
            Err(e) => {
                Ok(ToolResult {
                    success: false,
                    output: format!("HTTP request to Suno failed: {}", e),
                })
            }
        }
    }

    /// Execute Udio provider via HTTP API.
    async fn execute_udio(
        &self,
        action: &str,
        prompt: &str,
        duration_secs: u64,
        output: &std::path::Path,
    ) -> Result<ToolResult> {
        let api_key = match std::env::var("UDIO_API_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => {
                return Ok(ToolResult {
                    success: false,
                    output: "UDIO_API_KEY environment variable not set".to_string(),
                });
            }
        };

        let url = "https://api.udio.com/v1/generate";

        let body = json!({
            "action": action,
            "prompt": prompt,
            "duration_secs": duration_secs,
        });

        debug!("Calling Udio API: action={}, duration={}s", action, duration_secs);

        let client = reqwest::Client::new();
        let resp = client.post(url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .timeout(std::time::Duration::from_secs(300))
            .send()
            .await;

        match resp {
            Ok(response) => {
                let status = response.status();
                if !status.is_success() {
                    let err_text = response.text().await.unwrap_or_default();
                    warn!("Udio API error {}: {}", status, err_text);
                    return Ok(ToolResult {
                        success: false,
                        output: format!("Udio API error ({}): {}", status, truncate(&err_text, 500)),
                    });
                }

                let bytes = response.bytes().await?;

                if let Some(parent) = output.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(output, &bytes)?;

                Ok(ToolResult {
                    success: true,
                    output: format!(
                        "Music generated successfully!\nPath: {}\nSize: {} bytes\nProvider: udio\nAction: {}\nDuration: {}s\nPrompt: {}",
                        output.display(), bytes.len(), action, duration_secs, prompt
                    ),
                })
            }
            Err(e) => {
                Ok(ToolResult {
                    success: false,
                    output: format!("HTTP request to Udio failed: {}", e),
                })
            }
        }
    }

    /// Execute local AudioCraft provider via Python subprocess.
    async fn execute_local(
        &self,
        action: &str,
        prompt: &str,
        duration_secs: u64,
        output: &std::path::Path,
    ) -> Result<ToolResult> {
        let output_str = output.to_string_lossy().to_string();

        let python_code = format!(
            r#"
from audiocraft.models import MusicGen
import torchaudio
model = MusicGen.get_pretrained('facebook/musicgen-small')
model.set_generation_params(duration={duration})
descriptions = ["{prompt}"]
wav = model.generate(descriptions)
torchaudio.save("{output}", wav[0].cpu(), sample_rate=32000)
print("OK")
"#,
            duration = duration_secs,
            prompt = prompt.replace('"', r#"\""#),
            output = output_str.replace('\\', "\\\\").replace('"', r#"\""#),
        );

        debug!("Running local AudioCraft: action={}, duration={}s, output={}", action, duration_secs, output.display());

        if let Some(parent) = output.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let result = tokio::process::Command::new("python")
            .args(&["-c", &python_code])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let child = match result {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: format!(
                        "Failed to spawn python for AudioCraft: {}. Install with: pip install audiocraft",
                        e
                    ),
                });
            }
        };

        let output_result = tokio::time::timeout(
            std::time::Duration::from_secs(600),
            child.wait_with_output(),
        ).await
            .map_err(|_| anyhow::anyhow!("AudioCraft timed out after 600s"))?
            .map_err(|e| anyhow::anyhow!("AudioCraft process error: {}", e))?;

        let stderr = String::from_utf8_lossy(&output_result.stderr).to_string();

        if !output_result.status.success() {
            warn!("AudioCraft failed: {}", stderr);
            return Ok(ToolResult {
                success: false,
                output: format!("AudioCraft failed: {}", truncate(&stderr, 500)),
            });
        }

        // Verify output file exists
        if output.exists() {
            let metadata = std::fs::metadata(output)?;
            Ok(ToolResult {
                success: true,
                output: format!(
                    "Music generated successfully!\nPath: {}\nSize: {} bytes\nProvider: local (AudioCraft)\nAction: {}\nDuration: {}s\nPrompt: {}",
                    output.display(), metadata.len(), action, duration_secs, prompt
                ),
            })
        } else {
            Ok(ToolResult {
                success: false,
                output: format!("AudioCraft completed but output file not found at {}", output.display()),
            })
        }
    }
}

#[async_trait]
impl Tool for MusicGenerateTool {
    fn name(&self) -> &str {
        "music_generate"
    }

    fn description(&self) -> &str {
        "Generate music from a text prompt. Providers: 'suno' (Suno API), 'udio' (Udio API), or 'local' (AudioCraft subprocess). Actions: 'generate', 'extend', 'remix'. Returns path to generated .mp3 file."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Action to perform: 'generate' (create from prompt), 'extend' (extend existing track), 'remix' (remix from audio file)",
                    "enum": ["generate", "extend", "remix"],
                    "default": "generate"
                },
                "prompt": {
                    "type": "string",
                    "description": "Text description of desired music (e.g. 'upbeat lo-fi hip hop with piano')"
                },
                "duration_secs": {
                    "type": "integer",
                    "description": "Duration of the generated music in seconds (default: 30, max: 600)",
                    "default": 30
                },
                "style": {
                    "type": "string",
                    "description": "Optional style hint (e.g. 'lo-fi', 'rock', 'classical', 'electronic')"
                },
                "provider": {
                    "type": "string",
                    "description": "Music generation provider: 'suno' (default), 'udio', or 'local' (AudioCraft)",
                    "enum": ["suno", "udio", "local"],
                    "default": "suno"
                },
                "output_path": {
                    "type": "string",
                    "description": "Optional output file path. Defaults to ~/.phantom-mesh/workspace/music_{timestamp}.mp3"
                }
            },
            "required": ["prompt"]
        })
    }

    fn preflight(&self, args: &Value) -> Result<()> {
        let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
        if prompt.trim().is_empty() {
            anyhow::bail!("Preflight: 'prompt' cannot be empty");
        }

        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("generate");
        if let Err(msg) = Self::validate_action(action) {
            anyhow::bail!("Preflight: {}", msg);
        }

        let provider = args.get("provider").and_then(|v| v.as_str()).unwrap_or("suno");
        if let Err(msg) = Self::validate_provider(provider) {
            anyhow::bail!("Preflight: {}", msg);
        }

        let duration_secs = args.get("duration_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);
        if let Err(msg) = Self::validate_duration(duration_secs) {
            anyhow::bail!("Preflight: {}", msg);
        }

        match provider {
            "suno" => {
                match std::env::var("SUNO_API_KEY") {
                    Ok(k) if !k.is_empty() => Ok(()),
                    _ => anyhow::bail!("Preflight: SUNO_API_KEY environment variable not set"),
                }
            }
            "udio" => {
                match std::env::var("UDIO_API_KEY") {
                    Ok(k) if !k.is_empty() => Ok(()),
                    _ => anyhow::bail!("Preflight: UDIO_API_KEY environment variable not set"),
                }
            }
            "local" => {
                // Check if python is available
                let check = std::process::Command::new("python")
                    .arg("--version")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
                match check {
                    Ok(status) if status.success() => Ok(()),
                    _ => anyhow::bail!("Preflight: python is not available. Required for local AudioCraft provider."),
                }
            }
            _ => unreachable!(), // Already validated above
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let prompt = args.get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if prompt.trim().is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Error: 'prompt' is required and cannot be empty".to_string(),
            });
        }

        let action = args.get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("generate");

        if let Err(msg) = Self::validate_action(action) {
            return Ok(ToolResult {
                success: false,
                output: msg,
            });
        }

        let provider = args.get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("suno");

        if let Err(msg) = Self::validate_provider(provider) {
            return Ok(ToolResult {
                success: false,
                output: msg,
            });
        }

        let duration_secs = args.get("duration_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);

        if let Err(msg) = Self::validate_duration(duration_secs) {
            return Ok(ToolResult {
                success: false,
                output: msg,
            });
        }

        let style = args.get("style").and_then(|v| v.as_str());
        let full_prompt = Self::build_full_prompt(&prompt, style);

        let output_path_str = args.get("output_path").and_then(|v| v.as_str());
        let output = Self::build_output_path(output_path_str);

        match provider {
            "suno" => self.execute_suno(action, &full_prompt, duration_secs, &output).await,
            "udio" => self.execute_udio(action, &full_prompt, duration_secs, &output).await,
            "local" => self.execute_local(action, &full_prompt, duration_secs, &output).await,
            other => Ok(ToolResult {
                success: false,
                output: format!("Unknown provider '{}'. Use 'suno', 'udio', or 'local'.", other),
            }),
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // Respect char boundaries
        let end = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        let tool = MusicGenerateTool::new();
        assert_eq!(tool.name(), "music_generate");
    }

    #[test]
    fn test_description() {
        let tool = MusicGenerateTool::new();
        let desc = tool.description();
        assert!(desc.contains("music"), "Description should mention music: {}", desc);
        assert!(desc.contains("suno"), "Description should mention suno: {}", desc);
        assert!(desc.contains("udio"), "Description should mention udio: {}", desc);
        assert!(desc.contains("local"), "Description should mention local: {}", desc);
    }

    #[test]
    fn test_schema() {
        let tool = MusicGenerateTool::new();
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"][0], "prompt");
        assert!(schema["properties"]["prompt"].is_object());
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["duration_secs"].is_object());
        assert!(schema["properties"]["style"].is_object());
        assert!(schema["properties"]["provider"].is_object());
        assert!(schema["properties"]["output_path"].is_object());
    }

    #[test]
    fn test_preflight_empty_prompt() {
        let tool = MusicGenerateTool::new();
        let args = json!({"prompt": ""});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("empty"), "Error should mention empty: {}", err);
    }

    #[test]
    fn test_preflight_missing_prompt() {
        let tool = MusicGenerateTool::new();
        let args = json!({"action": "generate"});
        let result = tool.preflight(&args);
        assert!(result.is_err());
    }

    #[test]
    fn test_preflight_invalid_action() {
        let tool = MusicGenerateTool::new();
        let args = json!({"prompt": "hello", "action": "invalid_action"});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Unknown action"), "Error should mention unknown action: {}", err);
        assert!(err.contains("invalid_action"), "Error should include the bad action: {}", err);
    }

    #[test]
    fn test_preflight_invalid_provider() {
        let tool = MusicGenerateTool::new();
        let args = json!({"prompt": "hello", "provider": "badprovider"});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Unknown provider"), "Error should mention unknown provider: {}", err);
        assert!(err.contains("badprovider"), "Error should include the bad provider: {}", err);
    }

    #[test]
    fn test_preflight_suno_no_key() {
        let tool = MusicGenerateTool::new();
        let original = std::env::var("SUNO_API_KEY").ok();
        std::env::remove_var("SUNO_API_KEY");

        let args = json!({"prompt": "lo-fi beat", "provider": "suno"});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("SUNO_API_KEY"), "Error should mention API key: {}", err);

        if let Some(val) = original {
            std::env::set_var("SUNO_API_KEY", val);
        }
    }

    #[test]
    fn test_preflight_udio_no_key() {
        let tool = MusicGenerateTool::new();
        let original = std::env::var("UDIO_API_KEY").ok();
        std::env::remove_var("UDIO_API_KEY");

        let args = json!({"prompt": "rock anthem", "provider": "udio"});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("UDIO_API_KEY"), "Error should mention API key: {}", err);

        if let Some(val) = original {
            std::env::set_var("UDIO_API_KEY", val);
        }
    }

    #[test]
    fn test_preflight_duration_zero() {
        let tool = MusicGenerateTool::new();
        // Set a dummy key to pass provider check
        std::env::set_var("SUNO_API_KEY", "test-key");
        let args = json!({"prompt": "beat", "duration_secs": 0});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("greater than 0"), "Error should mention > 0: {}", err);
        std::env::remove_var("SUNO_API_KEY");
    }

    #[test]
    fn test_preflight_duration_too_long() {
        let tool = MusicGenerateTool::new();
        std::env::set_var("SUNO_API_KEY", "test-key");
        let args = json!({"prompt": "beat", "duration_secs": 9999});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("600"), "Error should mention max 600: {}", err);
        std::env::remove_var("SUNO_API_KEY");
    }

    #[test]
    fn test_validate_action() {
        assert!(MusicGenerateTool::validate_action("generate").is_ok());
        assert!(MusicGenerateTool::validate_action("extend").is_ok());
        assert!(MusicGenerateTool::validate_action("remix").is_ok());
        assert!(MusicGenerateTool::validate_action("invalid").is_err());
        assert!(MusicGenerateTool::validate_action("").is_err());
    }

    #[test]
    fn test_validate_provider() {
        assert!(MusicGenerateTool::validate_provider("suno").is_ok());
        assert!(MusicGenerateTool::validate_provider("udio").is_ok());
        assert!(MusicGenerateTool::validate_provider("local").is_ok());
        assert!(MusicGenerateTool::validate_provider("invalid").is_err());
    }

    #[test]
    fn test_validate_duration() {
        assert!(MusicGenerateTool::validate_duration(1).is_ok());
        assert!(MusicGenerateTool::validate_duration(30).is_ok());
        assert!(MusicGenerateTool::validate_duration(600).is_ok());
        assert!(MusicGenerateTool::validate_duration(0).is_err());
        assert!(MusicGenerateTool::validate_duration(601).is_err());
    }

    #[test]
    fn test_build_full_prompt() {
        assert_eq!(
            MusicGenerateTool::build_full_prompt("chill beat", Some("lo-fi")),
            "chill beat, lo-fi style"
        );
        assert_eq!(
            MusicGenerateTool::build_full_prompt("chill beat", None),
            "chill beat"
        );
        assert_eq!(
            MusicGenerateTool::build_full_prompt("chill beat", Some("")),
            "chill beat"
        );
    }

    #[test]
    fn test_output_path_generation() {
        let path = MusicGenerateTool::build_output_path(None);
        let path_str = path.to_string_lossy().to_string();
        assert!(path_str.contains(".phantom-mesh"), "Path should contain .phantom-mesh: {}", path_str);
        assert!(path_str.contains("workspace"), "Path should contain workspace: {}", path_str);
        assert!(path_str.contains("music_"), "Path should contain music_ prefix: {}", path_str);
        assert!(path_str.ends_with(".mp3"), "Path should end with .mp3: {}", path_str);

        let custom = MusicGenerateTool::build_output_path(Some("/custom/path/track.mp3"));
        assert_eq!(custom.to_string_lossy(), "/custom/path/track.mp3");
    }

    #[tokio::test]
    async fn test_execute_empty_prompt() {
        let tool = MusicGenerateTool::new();
        let result = tool.execute(json!({"prompt": ""})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("required"));
    }

    #[tokio::test]
    async fn test_execute_invalid_action() {
        let tool = MusicGenerateTool::new();
        let result = tool.execute(json!({"prompt": "beat", "action": "badaction"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_execute_invalid_provider() {
        let tool = MusicGenerateTool::new();
        let result = tool.execute(json!({"prompt": "beat", "provider": "badprovider"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown provider"));
    }

    #[tokio::test]
    async fn test_execute_invalid_duration() {
        let tool = MusicGenerateTool::new();
        let result = tool.execute(json!({"prompt": "beat", "duration_secs": 0})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("greater than 0"));
    }

    #[test]
    fn test_truncate_fn() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello...");
        // Unicode safety
        assert_eq!(truncate("你好世界測試", 3), "你好世...");
    }
}
