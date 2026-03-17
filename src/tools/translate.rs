//! Translation tool — translates text using Google Translate free endpoint.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolResult};

pub struct TranslateTool;

impl TranslateTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl Tool for TranslateTool {
    fn name(&self) -> &str { "translate" }

    fn description(&self) -> &str {
        "Translate text between languages. Supports 100+ languages via Google Translate."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "Text to translate" },
                "target_lang": { "type": "string", "description": "Target language code (e.g. 'en', 'zh-TW', 'ja', 'ko', 'fr')" },
                "source_lang": { "type": "string", "description": "Source language code (auto-detect if omitted)" }
            },
            "required": ["text", "target_lang"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let text = args["text"].as_str().unwrap_or("").trim();
        let target = args["target_lang"].as_str().unwrap_or("").trim();
        let source = args["source_lang"].as_str().unwrap_or("auto").trim();

        if text.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing required parameter: text".into() });
        }
        if target.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing required parameter: target_lang".into() });
        }

        // URL-encode text manually (percent-encoding)
        let encoded: String = text.bytes().map(|b| {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    format!("{}", b as char)
                }
                _ => format!("%{:02X}", b),
            }
        }).collect();

        let url = format!(
            "https://translate.googleapis.com/translate_a/single?client=gtx&sl={}&tl={}&dt=t&q={}",
            source, target, encoded
        );

        let client = reqwest::Client::new();
        let resp = client.get(&url)
            .header("User-Agent", "Mozilla/5.0")
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Ok(ToolResult {
                success: false,
                output: format!("Translation API error: {}", status),
            });
        }

        let body: Value = resp.json().await?;
        // Google Translate response: [[["translated text","original text",null,null,10]],null,"en"]
        let translated = body[0].as_array()
            .map(|sentences| {
                sentences.iter()
                    .filter_map(|s| s[0].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();

        let detected = body[2].as_str().unwrap_or("unknown");

        if translated.is_empty() {
            return Ok(ToolResult { success: false, output: "Translation returned empty result".into() });
        }

        Ok(ToolResult {
            success: true,
            output: json!({
                "translated": translated,
                "source_lang": detected,
                "target_lang": target,
            }).to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        assert_eq!(TranslateTool::new().name(), "translate");
    }

    #[test]
    fn test_schema() {
        let tool = TranslateTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["text"].is_object());
        assert!(schema["properties"]["target_lang"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 2);
    }

    #[tokio::test]
    async fn test_empty_text() {
        let tool = TranslateTool::new();
        let result = tool.execute(json!({"text": "", "target_lang": "en"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("text"));
    }

    #[tokio::test]
    async fn test_empty_target() {
        let tool = TranslateTool::new();
        let result = tool.execute(json!({"text": "hello", "target_lang": ""})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("target_lang"));
    }

    #[test]
    fn test_description() {
        let tool = TranslateTool::new();
        assert!(tool.description().contains("Translate"));
    }
}
