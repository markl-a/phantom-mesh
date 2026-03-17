//! LINE Notify tool — sends messages via LINE Notify API.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolResult};

/// LINE Notify configuration (from agents.toml [line] section)
#[derive(Debug, Clone, serde::Deserialize)]
pub struct LineConfig {
    #[serde(default)]
    pub notify_token: String,
}

impl Default for LineConfig {
    fn default() -> Self {
        Self { notify_token: String::new() }
    }
}

pub struct LineTool {
    config: LineConfig,
}

impl LineTool {
    pub fn new(config: LineConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for LineTool {
    fn name(&self) -> &str { "line_send" }

    fn description(&self) -> &str {
        "Send a notification via LINE Notify API"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "message": { "type": "string", "description": "Notification message" },
                "image_url": { "type": "string", "description": "Image URL to attach (optional)" },
                "sticker_package_id": { "type": "integer", "description": "LINE sticker package ID (optional)" },
                "sticker_id": { "type": "integer", "description": "LINE sticker ID (optional)" }
            },
            "required": ["message"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let message = args["message"].as_str().unwrap_or("").trim();
        if message.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing required parameter: message".into() });
        }
        if self.config.notify_token.is_empty() {
            return Ok(ToolResult { success: false, output: "LINE Notify token not configured".into() });
        }

        let mut params = vec![("message", message.to_string())];
        if let Some(img) = args["image_url"].as_str() {
            params.push(("imageFullsize", img.to_string()));
            params.push(("imageThumbnail", img.to_string()));
        }
        if let Some(pkg) = args["sticker_package_id"].as_i64() {
            params.push(("stickerPackageId", pkg.to_string()));
        }
        if let Some(stk) = args["sticker_id"].as_i64() {
            params.push(("stickerId", stk.to_string()));
        }

        let client = reqwest::Client::new();
        let resp = client.post("https://notify-api.line.me/api/notify")
            .bearer_auth(&self.config.notify_token)
            .form(&params)
            .send()
            .await?;

        if resp.status().is_success() {
            Ok(ToolResult { success: true, output: "LINE notification sent successfully".into() })
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Ok(ToolResult { success: false, output: format!("LINE Notify API error ({}): {}", status, body) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        assert_eq!(LineTool::new(LineConfig::default()).name(), "line_send");
    }

    #[test]
    fn test_schema() {
        let tool = LineTool::new(LineConfig::default());
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["message"].is_object());
        assert_eq!(schema["required"][0], "message");
    }

    #[tokio::test]
    async fn test_empty_message() {
        let tool = LineTool::new(LineConfig::default());
        let result = tool.execute(json!({"message": ""})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_no_token() {
        let tool = LineTool::new(LineConfig::default());
        let result = tool.execute(json!({"message": "hello"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("not configured"));
    }

    #[test]
    fn test_config_default() {
        let config = LineConfig::default();
        assert!(config.notify_token.is_empty());
    }

    #[test]
    fn test_description() {
        let tool = LineTool::new(LineConfig::default());
        assert!(tool.description().contains("LINE"));
    }
}
