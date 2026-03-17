//! Discord notification tool — sends messages via Discord Webhook.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolResult};

/// Discord tool configuration (from agents.toml [discord] section)
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DiscordConfig {
    #[serde(default)]
    pub webhook_url: String,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self { webhook_url: String::new() }
    }
}

pub struct DiscordTool {
    config: DiscordConfig,
}

impl DiscordTool {
    pub fn new(config: DiscordConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for DiscordTool {
    fn name(&self) -> &str { "discord_send" }

    fn description(&self) -> &str {
        "Send a message to a Discord channel via Webhook"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "Message content (supports Discord markdown)" },
                "username": { "type": "string", "description": "Override bot username (optional)" },
                "avatar_url": { "type": "string", "description": "Override bot avatar URL (optional)" },
                "tts": { "type": "boolean", "description": "Text-to-speech (optional, default false)" }
            },
            "required": ["text"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let text = args["text"].as_str().unwrap_or("").trim();
        if text.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing required parameter: text".into() });
        }
        if self.config.webhook_url.is_empty() {
            return Ok(ToolResult { success: false, output: "Discord webhook URL not configured".into() });
        }

        let mut payload = json!({ "content": text });
        if let Some(u) = args["username"].as_str() { payload["username"] = json!(u); }
        if let Some(a) = args["avatar_url"].as_str() { payload["avatar_url"] = json!(a); }
        if let Some(tts) = args["tts"].as_bool() { payload["tts"] = json!(tts); }

        let client = reqwest::Client::new();
        let resp = client.post(&self.config.webhook_url)
            .json(&payload)
            .send()
            .await?;

        // Discord returns 204 No Content on success
        if resp.status().is_success() {
            Ok(ToolResult { success: true, output: "Discord message sent successfully".into() })
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Ok(ToolResult { success: false, output: format!("Discord API error ({}): {}", status, body) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        assert_eq!(DiscordTool::new(DiscordConfig::default()).name(), "discord_send");
    }

    #[test]
    fn test_schema() {
        let tool = DiscordTool::new(DiscordConfig::default());
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["text"].is_object());
        assert_eq!(schema["required"][0], "text");
    }

    #[tokio::test]
    async fn test_empty_text() {
        let tool = DiscordTool::new(DiscordConfig::default());
        let result = tool.execute(json!({"text": ""})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_no_webhook() {
        let tool = DiscordTool::new(DiscordConfig::default());
        let result = tool.execute(json!({"text": "hello"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("not configured"));
    }

    #[test]
    fn test_config_default() {
        let config = DiscordConfig::default();
        assert!(config.webhook_url.is_empty());
    }

    #[test]
    fn test_description() {
        let tool = DiscordTool::new(DiscordConfig::default());
        assert!(tool.description().contains("Discord"));
    }
}
