//! WhatsApp notification tool — sends messages via WhatsApp Business Cloud API.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolResult};

/// WhatsApp tool configuration (from agents.toml [whatsapp] section)
#[derive(Debug, Clone, serde::Deserialize)]
pub struct WhatsAppConfig {
    #[serde(default)]
    pub phone_number_id: String,
    #[serde(default)]
    pub access_token: String,
}

impl Default for WhatsAppConfig {
    fn default() -> Self {
        Self { phone_number_id: String::new(), access_token: String::new() }
    }
}

pub struct WhatsAppTool {
    config: WhatsAppConfig,
}

impl WhatsAppTool {
    pub fn new(config: WhatsAppConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for WhatsAppTool {
    fn name(&self) -> &str { "whatsapp_send" }

    fn description(&self) -> &str {
        "Send a WhatsApp message via Business Cloud API"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "to": { "type": "string", "description": "Recipient phone number (with country code, e.g. +886912345678)" },
                "text": { "type": "string", "description": "Message text" }
            },
            "required": ["to", "text"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let to = args["to"].as_str().unwrap_or("").trim();
        let text = args["text"].as_str().unwrap_or("").trim();

        if to.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing required parameter: to".into() });
        }
        if text.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing required parameter: text".into() });
        }
        if self.config.phone_number_id.is_empty() || self.config.access_token.is_empty() {
            return Ok(ToolResult { success: false, output: "WhatsApp Business API not configured".into() });
        }

        let url = format!(
            "https://graph.facebook.com/v21.0/{}/messages",
            self.config.phone_number_id
        );

        let payload = json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": "text",
            "text": { "body": text }
        });

        let client = reqwest::Client::new();
        let resp = client.post(&url)
            .bearer_auth(&self.config.access_token)
            .json(&payload)
            .send()
            .await?;

        if resp.status().is_success() {
            Ok(ToolResult { success: true, output: format!("WhatsApp message sent to {}", to) })
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Ok(ToolResult { success: false, output: format!("WhatsApp API error ({}): {}", status, body) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        assert_eq!(WhatsAppTool::new(WhatsAppConfig::default()).name(), "whatsapp_send");
    }

    #[test]
    fn test_schema() {
        let tool = WhatsAppTool::new(WhatsAppConfig::default());
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["to"].is_object());
        assert!(schema["properties"]["text"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 2);
    }

    #[tokio::test]
    async fn test_empty_to() {
        let tool = WhatsAppTool::new(WhatsAppConfig::default());
        let result = tool.execute(json!({"to": "", "text": "hi"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("to"));
    }

    #[tokio::test]
    async fn test_empty_text() {
        let tool = WhatsAppTool::new(WhatsAppConfig::default());
        let result = tool.execute(json!({"to": "+886912345678", "text": ""})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("text"));
    }

    #[tokio::test]
    async fn test_no_config() {
        let tool = WhatsAppTool::new(WhatsAppConfig::default());
        let result = tool.execute(json!({"to": "+886912345678", "text": "hello"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("not configured"));
    }

    #[test]
    fn test_config_default() {
        let config = WhatsAppConfig::default();
        assert!(config.phone_number_id.is_empty());
        assert!(config.access_token.is_empty());
    }

    #[test]
    fn test_description() {
        let tool = WhatsAppTool::new(WhatsAppConfig::default());
        assert!(tool.description().contains("WhatsApp"));
    }
}
