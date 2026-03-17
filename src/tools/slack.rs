//! Slack notification tool — sends messages via Incoming Webhook.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolResult};

/// Slack tool configuration (from agents.toml [slack] section)
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SlackConfig {
    #[serde(default)]
    pub webhook_url: String,
}

impl Default for SlackConfig {
    fn default() -> Self {
        Self { webhook_url: String::new() }
    }
}

pub struct SlackTool {
    config: SlackConfig,
}

impl SlackTool {
    pub fn new(config: SlackConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for SlackTool {
    fn name(&self) -> &str { "slack_send" }

    fn description(&self) -> &str {
        "Send a message to a Slack channel via Incoming Webhook"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "Message text (supports Slack mrkdwn)" },
                "channel": { "type": "string", "description": "Override channel (optional)" },
                "username": { "type": "string", "description": "Override bot username (optional)" },
                "icon_emoji": { "type": "string", "description": "Override bot icon emoji (optional)" }
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
            return Ok(ToolResult { success: false, output: "Slack webhook URL not configured".into() });
        }

        let mut payload = json!({ "text": text });
        if let Some(ch) = args["channel"].as_str() { payload["channel"] = json!(ch); }
        if let Some(u) = args["username"].as_str() { payload["username"] = json!(u); }
        if let Some(e) = args["icon_emoji"].as_str() { payload["icon_emoji"] = json!(e); }

        let client = reqwest::Client::new();
        let resp = client.post(&self.config.webhook_url)
            .json(&payload)
            .send()
            .await?;

        if resp.status().is_success() {
            Ok(ToolResult { success: true, output: "Slack message sent successfully".into() })
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Ok(ToolResult { success: false, output: format!("Slack API error ({}): {}", status, body) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        assert_eq!(SlackTool::new(SlackConfig::default()).name(), "slack_send");
    }

    #[test]
    fn test_schema() {
        let tool = SlackTool::new(SlackConfig::default());
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["text"].is_object());
        assert_eq!(schema["required"][0], "text");
    }

    #[tokio::test]
    async fn test_empty_text() {
        let tool = SlackTool::new(SlackConfig::default());
        let result = tool.execute(json!({"text": ""})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_no_webhook() {
        let tool = SlackTool::new(SlackConfig::default());
        let result = tool.execute(json!({"text": "hello"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("not configured"));
    }

    #[test]
    fn test_config_default() {
        let config = SlackConfig::default();
        assert!(config.webhook_url.is_empty());
    }

    #[test]
    fn test_description() {
        let tool = SlackTool::new(SlackConfig::default());
        assert!(tool.description().contains("Slack"));
    }
}
