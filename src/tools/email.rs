//! Email send tool — sends emails via SMTP using Python subprocess.
//! Requires approval gate for all send operations.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{debug, warn};

use super::{Tool, ToolResult};

/// Email tool configuration (from agents.toml [email] section)
#[derive(Debug, Clone, serde::Deserialize)]
pub struct EmailConfig {
    #[serde(default = "default_smtp_host")]
    pub smtp_host: String,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub from_address: String,
    /// Use TLS (default: true)
    #[serde(default = "default_true")]
    pub use_tls: bool,
}

fn default_smtp_host() -> String { "smtp.gmail.com".to_string() }
fn default_smtp_port() -> u16 { 587 }
fn default_true() -> bool { true }

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            smtp_host: default_smtp_host(),
            smtp_port: default_smtp_port(),
            username: String::new(),
            password: String::new(),
            from_address: String::new(),
            use_tls: true,
        }
    }
}

pub struct EmailTool {
    config: EmailConfig,
}

impl EmailTool {
    pub fn new(config: EmailConfig) -> Self {
        Self { config }
    }

    /// Deploy the Python email helper script
    fn deploy_helper(&self) -> Result<String> {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".to_string());
        let helper_path = format!("{}/.clawtex/email_helper.py", home);

        let script = r#"#!/usr/bin/env python3
"""Clawtex email helper — sends emails via SMTP."""
import sys, json, smtplib
from email.mime.text import MIMEText
from email.mime.multipart import MIMEMultipart

def send_email(config):
    msg = MIMEMultipart("alternative")
    msg["From"] = config["from"]
    msg["To"] = config["to"]
    msg["Subject"] = config["subject"]

    # Plain text part
    text_part = MIMEText(config.get("body", ""), "plain", "utf-8")
    msg.attach(text_part)

    # HTML part (optional)
    if config.get("html"):
        html_part = MIMEText(config["html"], "html", "utf-8")
        msg.attach(html_part)

    # CC
    if config.get("cc"):
        msg["Cc"] = config["cc"]

    try:
        if config.get("use_tls", True):
            server = smtplib.SMTP(config["smtp_host"], config["smtp_port"])
            server.ehlo()
            server.starttls()
        else:
            server = smtplib.SMTP(config["smtp_host"], config["smtp_port"])

        if config.get("username") and config.get("password"):
            server.login(config["username"], config["password"])

        recipients = [config["to"]]
        if config.get("cc"):
            recipients.extend([a.strip() for a in config["cc"].split(",")])

        server.sendmail(config["from"], recipients, msg.as_string())
        server.quit()
        return {"success": True, "message": f"Email sent to {config['to']}"}
    except Exception as e:
        return {"success": False, "error": str(e)}

if __name__ == "__main__":
    config = json.loads(sys.stdin.read())
    result = send_email(config)
    print(json.dumps(result))
"#;

        std::fs::write(&helper_path, script)?;
        Ok(helper_path)
    }
}

#[async_trait]
impl Tool for EmailTool {
    fn name(&self) -> &str {
        "email_send"
    }

    fn description(&self) -> &str {
        "Send an email via SMTP. Requires approval. Args: to, subject, body, html (optional), cc (optional)"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Recipient email address"
                },
                "subject": {
                    "type": "string",
                    "description": "Email subject line"
                },
                "body": {
                    "type": "string",
                    "description": "Plain text email body"
                },
                "html": {
                    "type": "string",
                    "description": "Optional HTML email body"
                },
                "cc": {
                    "type": "string",
                    "description": "Optional CC addresses (comma-separated)"
                }
            },
            "required": ["to", "subject", "body"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let to = args.get("to").and_then(|v| v.as_str()).unwrap_or("");
        let subject = args.get("subject").and_then(|v| v.as_str()).unwrap_or("");
        let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");

        if to.is_empty() || subject.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Missing required fields: to, subject".to_string(),
            });
        }

        // Validate email format (basic check)
        if !to.contains('@') || !to.contains('.') {
            return Ok(ToolResult {
                success: false,
                output: format!("Invalid email address: {}", to),
            });
        }

        // Check SMTP configuration
        if self.config.username.is_empty() || self.config.password.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "SMTP not configured. Set [email] username and password in agents.toml".to_string(),
            });
        }

        // Deploy helper script
        let helper_path = match self.deploy_helper() {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult {
                success: false,
                output: format!("Failed to deploy email helper: {}", e),
            }),
        };

        // Build config for Python helper
        let config = json!({
            "smtp_host": self.config.smtp_host,
            "smtp_port": self.config.smtp_port,
            "username": self.config.username,
            "password": self.config.password,
            "from": if self.config.from_address.is_empty() { &self.config.username } else { &self.config.from_address },
            "to": to,
            "subject": subject,
            "body": body,
            "html": args.get("html").and_then(|v| v.as_str()).unwrap_or(""),
            "cc": args.get("cc").and_then(|v| v.as_str()).unwrap_or(""),
            "use_tls": self.config.use_tls,
        });

        debug!("Sending email to {} via {}:{}", to, self.config.smtp_host, self.config.smtp_port);

        // Execute Python helper
        let mut child = tokio::process::Command::new("python")
            .arg(&helper_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn email helper: {}", e))?;

        // Write config to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let config_str = serde_json::to_string(&config)?;
            stdin.write_all(config_str.as_bytes()).await?;
            drop(stdin);
        }

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            child.wait_with_output(),
        ).await
            .map_err(|_| anyhow::anyhow!("Email send timed out after 30s"))?
            .map_err(|e| anyhow::anyhow!("Email helper error: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            warn!("Email helper failed: {}", stderr);
            return Ok(ToolResult {
                success: false,
                output: format!("Email send failed: {}", if stderr.is_empty() { &stdout } else { &stderr }),
            });
        }

        // Parse JSON result from helper
        match serde_json::from_str::<Value>(&stdout) {
            Ok(result) => {
                let success = result.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
                let message = if success {
                    result.get("message").and_then(|v| v.as_str()).unwrap_or("Email sent").to_string()
                } else {
                    result.get("error").and_then(|v| v.as_str()).unwrap_or("Unknown error").to_string()
                };
                Ok(ToolResult { success, output: message })
            }
            Err(_) => Ok(ToolResult {
                success: false,
                output: format!("Unexpected helper output: {}", stdout),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_tool_name() {
        let tool = EmailTool::new(EmailConfig::default());
        assert_eq!(tool.name(), "email_send");
    }

    #[test]
    fn test_email_config_defaults() {
        let config = EmailConfig::default();
        assert_eq!(config.smtp_host, "smtp.gmail.com");
        assert_eq!(config.smtp_port, 587);
        assert!(config.use_tls);
        assert!(config.username.is_empty());
    }

    #[test]
    fn test_email_tool_schema() {
        let tool = EmailTool::new(EmailConfig::default());
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("to").is_some());
        assert!(props.get("subject").is_some());
        assert!(props.get("body").is_some());
        assert!(props.get("html").is_some());
    }

    #[tokio::test]
    async fn test_email_missing_to() {
        let tool = EmailTool::new(EmailConfig::default());
        let result = tool.execute(json!({"subject": "test", "body": "hello"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing required"));
    }

    #[tokio::test]
    async fn test_email_invalid_address() {
        let tool = EmailTool::new(EmailConfig::default());
        let result = tool.execute(json!({"to": "invalid", "subject": "test", "body": "hello"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Invalid email"));
    }

    #[tokio::test]
    async fn test_email_no_smtp_config() {
        let tool = EmailTool::new(EmailConfig::default());
        let result = tool.execute(json!({
            "to": "test@example.com",
            "subject": "test",
            "body": "hello"
        })).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("SMTP not configured"));
    }
}
