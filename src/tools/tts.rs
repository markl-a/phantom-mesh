//! TTS (text-to-speech) tool — generates speech audio from text.
//! Two providers: "edge" (free, via edge-tts Python CLI) and "elevenlabs" (paid API).

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{debug, warn};

use super::{Tool, ToolResult};

pub struct TtsTool;

impl TtsTool {
    pub fn new() -> Self {
        Self
    }

    /// Detect whether text is primarily Chinese (CJK Unified Ideographs).
    fn is_chinese(text: &str) -> bool {
        let cjk_count = text.chars().filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c)).count();
        let alpha_count = text.chars().filter(|c| c.is_alphabetic()).count();
        if alpha_count == 0 {
            return cjk_count > 0;
        }
        (cjk_count as f64 / alpha_count as f64) > 0.3
    }

    /// Select default voice based on text language.
    fn default_voice(text: &str) -> &'static str {
        if Self::is_chinese(text) {
            "zh-TW-HsiaoChenNeural"
        } else {
            "en-US-AriaNeural"
        }
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
            workspace.join(format!("tts_{}.mp3", timestamp))
        }
    }

    /// Build the edge-tts command arguments.
    fn build_edge_command(text: &str, voice: &str, output: &std::path::Path) -> Vec<String> {
        vec![
            "--text".to_string(),
            text.to_string(),
            "--voice".to_string(),
            voice.to_string(),
            "--write-media".to_string(),
            output.to_string_lossy().to_string(),
        ]
    }

    /// Execute edge-tts provider (free, subprocess).
    async fn execute_edge(&self, text: &str, voice: &str, output: &std::path::Path) -> Result<ToolResult> {
        let args = Self::build_edge_command(text, voice, output);

        debug!("Running edge-tts with voice={}, output={}", voice, output.display());

        if let Some(parent) = output.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let result = tokio::process::Command::new("edge-tts")
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let child = match result {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Failed to spawn edge-tts: {}. Install with: pip install edge-tts", e),
                });
            }
        };

        let output_result = tokio::time::timeout(
            std::time::Duration::from_secs(120),
            child.wait_with_output(),
        ).await
            .map_err(|_| anyhow::anyhow!("edge-tts timed out after 120s"))?
            .map_err(|e| anyhow::anyhow!("edge-tts process error: {}", e))?;

        let stderr = String::from_utf8_lossy(&output_result.stderr).to_string();

        if !output_result.status.success() {
            warn!("edge-tts failed: {}", stderr);
            return Ok(ToolResult {
                success: false,
                output: format!("edge-tts failed: {}", stderr),
            });
        }

        // Verify output file exists
        if output.exists() {
            let metadata = std::fs::metadata(output)?;
            Ok(ToolResult {
                success: true,
                output: format!(
                    "TTS audio generated successfully!\nPath: {}\nSize: {} bytes\nVoice: {}\nProvider: edge",
                    output.display(), metadata.len(), voice
                ),
            })
        } else {
            Ok(ToolResult {
                success: false,
                output: format!("edge-tts completed but output file not found at {}", output.display()),
            })
        }
    }

    /// Execute ElevenLabs provider (paid API).
    async fn execute_elevenlabs(&self, text: &str, voice_id: &str, output: &std::path::Path) -> Result<ToolResult> {
        let api_key = match std::env::var("ELEVENLABS_API_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => {
                return Ok(ToolResult {
                    success: false,
                    output: "ELEVENLABS_API_KEY environment variable not set".to_string(),
                });
            }
        };

        let url = format!("https://api.elevenlabs.io/v1/text-to-speech/{}", voice_id);

        let body = json!({
            "text": text,
            "model_id": "eleven_monolingual_v1",
            "voice_settings": {
                "stability": 0.5,
                "similarity_boost": 0.5
            }
        });

        debug!("Calling ElevenLabs TTS API with voice_id={}", voice_id);

        let client = reqwest::Client::new();
        let resp = client.post(&url)
            .header("xi-api-key", &api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "audio/mpeg")
            .json(&body)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await;

        match resp {
            Ok(response) => {
                let status = response.status();
                if !status.is_success() {
                    let err_text = response.text().await.unwrap_or_default();
                    warn!("ElevenLabs API error {}: {}", status, err_text);
                    return Ok(ToolResult {
                        success: false,
                        output: format!("ElevenLabs API error ({}): {}", status, truncate(&err_text, 500)),
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
                        "TTS audio generated successfully!\nPath: {}\nSize: {} bytes\nVoice ID: {}\nProvider: elevenlabs",
                        output.display(), bytes.len(), voice_id
                    ),
                })
            }
            Err(e) => {
                Ok(ToolResult {
                    success: false,
                    output: format!("HTTP request to ElevenLabs failed: {}", e),
                })
            }
        }
    }
}

#[async_trait]
impl Tool for TtsTool {
    fn name(&self) -> &str {
        "tts"
    }

    fn description(&self) -> &str {
        "Convert text to speech audio (MP3). Providers: 'edge' (free, edge-tts CLI) or 'elevenlabs' (paid API). Returns path to generated .mp3 file."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "The text to convert to speech"
                },
                "provider": {
                    "type": "string",
                    "description": "TTS provider: 'edge' (free, default) or 'elevenlabs' (paid)",
                    "enum": ["edge", "elevenlabs"],
                    "default": "edge"
                },
                "voice": {
                    "type": "string",
                    "description": "Voice name (edge) or voice ID (elevenlabs). Defaults: zh-TW-HsiaoChenNeural for Chinese, en-US-AriaNeural for English"
                },
                "output_path": {
                    "type": "string",
                    "description": "Optional output file path. Defaults to ~/.phantom-mesh/workspace/tts_{timestamp}.mp3"
                }
            },
            "required": ["text"]
        })
    }

    fn preflight(&self, args: &Value) -> Result<()> {
        let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
        if text.trim().is_empty() {
            anyhow::bail!("Preflight: 'text' cannot be empty");
        }

        let provider = args.get("provider").and_then(|v| v.as_str()).unwrap_or("edge");

        match provider {
            "edge" => {
                // Check if edge-tts is installed
                let check = std::process::Command::new("edge-tts")
                    .arg("--version")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
                match check {
                    Ok(status) if status.success() => Ok(()),
                    _ => {
                        // Also try `edge-tts --help` as some versions don't have --version
                        let check2 = std::process::Command::new("edge-tts")
                            .arg("--help")
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .status();
                        match check2 {
                            Ok(_) => Ok(()),
                            Err(_) => anyhow::bail!("Preflight: edge-tts is not installed. Install with: pip install edge-tts"),
                        }
                    }
                }
            }
            "elevenlabs" => {
                match std::env::var("ELEVENLABS_API_KEY") {
                    Ok(k) if !k.is_empty() => Ok(()),
                    _ => anyhow::bail!("Preflight: ELEVENLABS_API_KEY environment variable not set"),
                }
            }
            other => {
                anyhow::bail!("Preflight: unknown provider '{}'. Use 'edge' or 'elevenlabs'", other);
            }
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let text = args.get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if text.trim().is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Error: 'text' is required and cannot be empty".to_string(),
            });
        }

        let provider = args.get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("edge");

        let default_voice = Self::default_voice(&text);
        let voice = args.get("voice")
            .and_then(|v| v.as_str())
            .unwrap_or(default_voice);

        let output_path_str = args.get("output_path").and_then(|v| v.as_str());
        let output = Self::build_output_path(output_path_str);

        match provider {
            "edge" => self.execute_edge(&text, voice, &output).await,
            "elevenlabs" => self.execute_elevenlabs(&text, voice, &output).await,
            other => Ok(ToolResult {
                success: false,
                output: format!("Unknown provider '{}'. Use 'edge' or 'elevenlabs'.", other),
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
        let tool = TtsTool::new();
        assert_eq!(tool.name(), "tts");
    }

    #[test]
    fn test_schema() {
        let tool = TtsTool::new();
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"][0], "text");
        assert!(schema["properties"]["text"].is_object());
        assert!(schema["properties"]["provider"].is_object());
        assert!(schema["properties"]["voice"].is_object());
        assert!(schema["properties"]["output_path"].is_object());
    }

    #[test]
    fn test_preflight_edge_not_installed() {
        // This test checks that preflight validates edge-tts availability.
        // On CI/systems without edge-tts, it should fail with a helpful message.
        let tool = TtsTool::new();
        let args = json!({"text": "hello", "provider": "edge"});
        let result = tool.preflight(&args);
        // We can't guarantee edge-tts is or isn't installed, so just check the error message format
        if result.is_err() {
            let err = result.unwrap_err().to_string();
            assert!(err.contains("edge-tts"), "Error should mention edge-tts: {}", err);
            assert!(err.contains("pip install"), "Error should suggest pip install: {}", err);
        }
        // If it's Ok, edge-tts is installed — that's also valid
    }

    #[test]
    fn test_preflight_elevenlabs_no_key() {
        let tool = TtsTool::new();
        // Temporarily ensure the env var is unset for this test
        let original = std::env::var("ELEVENLABS_API_KEY").ok();
        std::env::remove_var("ELEVENLABS_API_KEY");

        let args = json!({"text": "hello", "provider": "elevenlabs"});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("ELEVENLABS_API_KEY"), "Error should mention API key: {}", err);

        // Restore env var if it was set
        if let Some(val) = original {
            std::env::set_var("ELEVENLABS_API_KEY", val);
        }
    }

    #[test]
    fn test_default_voice_selection() {
        // Chinese text should get Chinese voice
        assert_eq!(TtsTool::default_voice("你好世界"), "zh-TW-HsiaoChenNeural");
        assert_eq!(TtsTool::default_voice("這是測試"), "zh-TW-HsiaoChenNeural");

        // English text should get English voice
        assert_eq!(TtsTool::default_voice("Hello world"), "en-US-AriaNeural");
        assert_eq!(TtsTool::default_voice("This is a test"), "en-US-AriaNeural");

        // Mixed text with more CJK should get Chinese voice
        assert_eq!(TtsTool::default_voice("你好 hello 世界 test 測試"), "zh-TW-HsiaoChenNeural");
    }

    #[test]
    fn test_command_construction() {
        let output = std::path::PathBuf::from("/tmp/test_output.mp3");
        let args = TtsTool::build_edge_command("Hello world", "en-US-AriaNeural", &output);
        assert_eq!(args.len(), 6);
        assert_eq!(args[0], "--text");
        assert_eq!(args[1], "Hello world");
        assert_eq!(args[2], "--voice");
        assert_eq!(args[3], "en-US-AriaNeural");
        assert_eq!(args[4], "--write-media");
        assert_eq!(args[5], "/tmp/test_output.mp3");
    }

    #[test]
    fn test_output_path_generation() {
        // Default path should be in workspace
        let path = TtsTool::build_output_path(None);
        let path_str = path.to_string_lossy().to_string();
        assert!(path_str.contains(".phantom-mesh"), "Path should contain .phantom-mesh: {}", path_str);
        assert!(path_str.contains("workspace"), "Path should contain workspace: {}", path_str);
        assert!(path_str.contains("tts_"), "Path should contain tts_ prefix: {}", path_str);
        assert!(path_str.ends_with(".mp3"), "Path should end with .mp3: {}", path_str);

        // Custom path should be used as-is
        let custom = TtsTool::build_output_path(Some("/custom/path/audio.mp3"));
        assert_eq!(custom.to_string_lossy(), "/custom/path/audio.mp3");
    }

    #[test]
    fn test_invalid_provider() {
        let tool = TtsTool::new();
        let args = json!({"text": "hello", "provider": "invalid_provider"});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unknown provider"), "Error should mention unknown provider: {}", err);
        assert!(err.contains("invalid_provider"), "Error should include the bad provider name: {}", err);
    }

    #[test]
    fn test_preflight_empty_text() {
        let tool = TtsTool::new();
        let args = json!({"text": "", "provider": "edge"});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("empty"), "Error should mention empty: {}", err);
    }

    #[test]
    fn test_preflight_missing_text() {
        let tool = TtsTool::new();
        let args = json!({"provider": "edge"});
        let result = tool.preflight(&args);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_empty_text() {
        let tool = TtsTool::new();
        let result = tool.execute(json!({"text": ""})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("required"));
    }

    #[tokio::test]
    async fn test_execute_invalid_provider() {
        let tool = TtsTool::new();
        let result = tool.execute(json!({"text": "hello", "provider": "badprovider"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown provider"));
    }

    #[test]
    fn test_is_chinese() {
        assert!(TtsTool::is_chinese("你好世界"));
        assert!(TtsTool::is_chinese("這是一個測試"));
        assert!(!TtsTool::is_chinese("Hello world"));
        assert!(!TtsTool::is_chinese("This is English text only"));
        // Empty string
        assert!(!TtsTool::is_chinese(""));
        // Numbers only
        assert!(!TtsTool::is_chinese("12345"));
    }

    #[test]
    fn test_truncate_fn() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello...");
        // Unicode safety
        assert_eq!(truncate("你好世界測試", 3), "你好世...");
    }
}
