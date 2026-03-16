//! Vision analysis tool — sends images to Gemini/Groq for visual understanding.
//! Used by the agent to understand screenshots, web pages, and UI elements.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;
use tracing::debug;

use super::{Tool, ToolResult};

/// Vision tool — analyzes images using free-tier vision APIs.
pub struct VisionTool {
    /// Gemini API key (primary)
    gemini_api_key: Option<String>,
    /// Groq API key (fallback)
    groq_api_key: Option<String>,
    client: reqwest::Client,
}

impl VisionTool {
    pub fn new(gemini_api_key: Option<String>, groq_api_key: Option<String>) -> Self {
        Self {
            gemini_api_key,
            groq_api_key,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Analyze image via Gemini Vision API
    async fn analyze_gemini(&self, b64_image: &str, prompt: &str) -> Result<String> {
        let api_key = self.gemini_api_key.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No Gemini API key"))?;

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-lite:generateContent?key={}",
            api_key
        );

        let body = json!({
            "contents": [{
                "parts": [
                    {"text": prompt},
                    {
                        "inline_data": {
                            "mime_type": "image/png",
                            "data": b64_image
                        }
                    }
                ]
            }],
            "generationConfig": {
                "temperature": 0.3,
                "maxOutputTokens": 2048
            }
        });

        let resp = self.client.post(&url).json(&body).send().await?;
        let status = resp.status();

        if !status.is_success() {
            let err = resp.text().await.unwrap_or_default();
            let err_preview: String = err.chars().take(300).collect();
            return Err(anyhow::anyhow!("Gemini vision error ({}): {}", status, err_preview));
        }

        let json: Value = resp.json().await?;
        let text = json
            .pointer("/candidates/0/content/parts/0/text")
            .and_then(|v| v.as_str())
            .unwrap_or("(no response)")
            .to_string();

        Ok(text)
    }

    /// Analyze image via Groq Vision API (Llama 4 Scout)
    async fn analyze_groq(&self, b64_image: &str, prompt: &str) -> Result<String> {
        let api_key = self.groq_api_key.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No Groq API key"))?;

        let body = json!({
            "model": "llama-4-scout-17b-16e-instruct",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": prompt},
                    {
                        "type": "image_url",
                        "image_url": {
                            "url": format!("data:image/png;base64,{}", b64_image)
                        }
                    }
                ]
            }],
            "temperature": 0.3,
            "max_tokens": 2048
        });

        let resp = self.client
            .post("https://api.groq.com/openai/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err = resp.text().await.unwrap_or_default();
            let err_preview: String = err.chars().take(300).collect();
            return Err(anyhow::anyhow!("Groq vision error ({}): {}", status, err_preview));
        }

        let json: Value = resp.json().await?;
        let text = json
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .unwrap_or("(no response)")
            .to_string();

        Ok(text)
    }
}

#[async_trait]
impl Tool for VisionTool {
    fn name(&self) -> &str {
        "vision_analyze"
    }

    fn description(&self) -> &str {
        "Analyze an image using AI vision. Send a screenshot or image file for visual understanding. \
         Useful for understanding web pages, UI elements, charts, documents, etc."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "image_path": {
                    "type": "string",
                    "description": "Path to the image file to analyze"
                },
                "prompt": {
                    "type": "string",
                    "description": "What to look for or describe in the image (default: 'Describe this image in detail')"
                }
            },
            "required": ["image_path"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let image_path = args.get("image_path").and_then(|v| v.as_str()).unwrap_or("");
        if image_path.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Missing 'image_path' parameter".into(),
            });
        }

        let prompt = args.get("prompt").and_then(|v| v.as_str())
            .unwrap_or("Describe this image in detail. If it's a web page, list the key elements, text content, buttons, and links visible.");

        // Read and encode image
        let path = Path::new(image_path);
        if !path.exists() {
            return Ok(ToolResult {
                success: false,
                output: format!("Image file not found: {}", image_path),
            });
        }

        let image_bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Failed to read image: {}", e),
                });
            }
        };

        let b64_image = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &image_bytes);

        debug!("vision_analyze: image={} ({}KB), prompt={}...",
            image_path, image_bytes.len() / 1024, &prompt[..prompt.len().min(50)]);

        // Try Gemini first, then Groq
        if self.gemini_api_key.is_some() {
            match self.analyze_gemini(&b64_image, prompt).await {
                Ok(result) => {
                    return Ok(ToolResult { success: true, output: result });
                }
                Err(e) => {
                    debug!("Gemini vision failed, trying Groq: {}", e);
                }
            }
        }

        if self.groq_api_key.is_some() {
            match self.analyze_groq(&b64_image, prompt).await {
                Ok(result) => {
                    return Ok(ToolResult { success: true, output: result });
                }
                Err(e) => {
                    return Ok(ToolResult {
                        success: false,
                        output: format!("Vision analysis failed (all providers): {}", e),
                    });
                }
            }
        }

        Ok(ToolResult {
            success: false,
            output: "No vision API keys configured. Set gemini_api_key or groq_api_key in config.".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vision_tool_name() {
        let tool = VisionTool::new(None, None);
        assert_eq!(tool.name(), "vision_analyze");
    }

    #[test]
    fn test_vision_tool_schema() {
        let tool = VisionTool::new(None, None);
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("image_path").is_some());
        assert!(props.get("prompt").is_some());
    }

    #[tokio::test]
    async fn test_vision_missing_path() {
        let tool = VisionTool::new(None, None);
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_vision_nonexistent_file() {
        let tool = VisionTool::new(Some("test-key".into()), None);
        let result = tool.execute(json!({"image_path": "/nonexistent/image.png"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("not found"));
    }

    #[tokio::test]
    async fn test_vision_no_api_keys() {
        // Create a temp file to use as image
        let dir = tempfile::tempdir().unwrap();
        let img_path = dir.path().join("test.png");
        std::fs::write(&img_path, &[0x89, 0x50, 0x4E, 0x47]).unwrap(); // PNG header

        let tool = VisionTool::new(None, None);
        let result = tool.execute(json!({"image_path": img_path.to_str().unwrap()})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("No vision API keys"));
    }
}
