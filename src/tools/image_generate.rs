//! image_generate tool — generates images using Gemini Imagen API.
//! Config-gated: requires gemini_api_key.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, warn};

use super::{Tool, ToolResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenerateConfig {
    pub gemini_api_key: String,
}

pub struct ImageGenerateTool {
    config: ImageGenerateConfig,
}

impl ImageGenerateTool {
    pub fn new(config: ImageGenerateConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ImageGenerateTool {
    fn name(&self) -> &str {
        "image_generate"
    }

    fn description(&self) -> &str {
        "Generate an image from a text prompt using Gemini Imagen API. Returns the saved file path."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Text description of the image to generate"
                },
                "style": {
                    "type": "string",
                    "description": "Optional style hint (e.g. 'photorealistic', 'cartoon', 'watercolor')"
                },
                "output_path": {
                    "type": "string",
                    "description": "Optional output file path. Defaults to workspace/{timestamp}.png"
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
        if self.config.gemini_api_key.is_empty() {
            anyhow::bail!("Preflight: gemini_api_key is not configured");
        }
        Ok(())
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

        let style = args.get("style").and_then(|v| v.as_str()).unwrap_or("");
        let full_prompt = if style.is_empty() {
            prompt.clone()
        } else {
            format!("{}, {} style", prompt, style)
        };

        // Determine output path
        let output_path = if let Some(p) = args.get("output_path").and_then(|v| v.as_str()) {
            std::path::PathBuf::from(p)
        } else {
            let workspace = dirs::home_dir()
                .unwrap_or_default()
                .join(".phantom-mesh")
                .join("workspace");
            let _ = std::fs::create_dir_all(&workspace);
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            workspace.join(format!("image_{}.png", timestamp))
        };

        debug!("Generating image with prompt: {}", full_prompt);

        // Call Gemini generateContent with responseModalities: ["IMAGE"]
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash-exp:generateContent?key={}",
            self.config.gemini_api_key
        );

        let body = json!({
            "contents": [{
                "parts": [{
                    "text": full_prompt
                }]
            }],
            "generationConfig": {
                "responseModalities": ["TEXT", "IMAGE"]
            }
        });

        let client = reqwest::Client::new();
        let resp = client.post(&url)
            .json(&body)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await;

        match resp {
            Ok(response) => {
                let status = response.status();
                if !status.is_success() {
                    let err_text = response.text().await.unwrap_or_default();
                    warn!("Gemini Imagen API error {}: {}", status, err_text);
                    return Ok(ToolResult {
                        success: false,
                        output: format!("Gemini API error ({}): {}", status, truncate(&err_text, 500)),
                    });
                }

                let json_resp: Value = response.json().await?;

                // Look for inline_data in response parts
                if let Some(parts) = json_resp
                    .pointer("/candidates/0/content/parts")
                    .and_then(|v| v.as_array())
                {
                    for part in parts {
                        if let Some(inline_data) = part.get("inlineData") {
                            if let Some(b64) = inline_data.get("data").and_then(|v| v.as_str()) {
                                // Decode base64 and save
                                use base64::Engine;
                                let decoder = base64::engine::general_purpose::STANDARD;
                                match decoder.decode(b64) {
                                    Ok(bytes) => {
                                        if let Some(parent) = output_path.parent() {
                                            let _ = std::fs::create_dir_all(parent);
                                        }
                                        std::fs::write(&output_path, &bytes)?;
                                        let mime = inline_data.get("mimeType")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("image/png");
                                        return Ok(ToolResult {
                                            success: true,
                                            output: format!(
                                                "Image generated successfully!\nPath: {}\nSize: {} bytes\nMIME: {}\nPrompt: {}",
                                                output_path.display(), bytes.len(), mime, full_prompt
                                            ),
                                        });
                                    }
                                    Err(e) => {
                                        return Ok(ToolResult {
                                            success: false,
                                            output: format!("Failed to decode base64 image: {}", e),
                                        });
                                    }
                                }
                            }
                        }
                    }

                    // No image in response, check for text description
                    let text_parts: Vec<&str> = parts.iter()
                        .filter_map(|p| p.get("text").and_then(|v| v.as_str()))
                        .collect();
                    if !text_parts.is_empty() {
                        return Ok(ToolResult {
                            success: true,
                            output: format!(
                                "Model returned text instead of image (model may not support image generation):\n{}",
                                text_parts.join("\n")
                            ),
                        });
                    }
                }

                Ok(ToolResult {
                    success: false,
                    output: "No image or text found in Gemini response".to_string(),
                })
            }
            Err(e) => {
                Ok(ToolResult {
                    success: false,
                    output: format!("HTTP request failed: {}", e),
                })
            }
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ImageGenerateConfig {
        ImageGenerateConfig {
            gemini_api_key: "test-key-123".to_string(),
        }
    }

    #[test]
    fn test_schema() {
        let tool = ImageGenerateTool::new(test_config());
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"][0], "prompt");
        assert!(schema["properties"]["prompt"].is_object());
        assert!(schema["properties"]["style"].is_object());
    }

    #[test]
    fn test_name_and_description() {
        let tool = ImageGenerateTool::new(test_config());
        assert_eq!(tool.name(), "image_generate");
        assert!(tool.description().contains("image"));
    }

    #[test]
    fn test_preflight_empty_prompt() {
        let tool = ImageGenerateTool::new(test_config());
        let result = tool.preflight(&json!({"prompt": ""}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_preflight_missing_key() {
        let config = ImageGenerateConfig { gemini_api_key: String::new() };
        let tool = ImageGenerateTool::new(config);
        let result = tool.preflight(&json!({"prompt": "a cat"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("gemini_api_key"));
    }

    #[test]
    fn test_preflight_ok() {
        let tool = ImageGenerateTool::new(test_config());
        assert!(tool.preflight(&json!({"prompt": "a beautiful sunset"})).is_ok());
    }

    #[tokio::test]
    async fn test_execute_empty_prompt() {
        let tool = ImageGenerateTool::new(test_config());
        let result = tool.execute(json!({"prompt": ""})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("required"));
    }

    #[test]
    fn test_config_serialize() {
        let config = test_config();
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("test-key-123"));
        let back: ImageGenerateConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.gemini_api_key, "test-key-123");
    }

    #[test]
    fn test_truncate_fn() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello...");
    }
}
