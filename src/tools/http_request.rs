use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::debug;

use super::{Tool, ToolResult};

/// HTTP request tool — GET/POST/PUT/DELETE with domain allowlist
pub struct HttpRequestTool {
    client: Client,
    allowed_domains: Vec<String>,
}

impl HttpRequestTool {
    pub fn new(allowed_domains: Vec<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");
        Self { client, allowed_domains }
    }

    fn is_domain_allowed(&self, url: &str) -> bool {
        if self.allowed_domains.is_empty() || self.allowed_domains.contains(&"*".to_string()) {
            return true;
        }
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                return self.allowed_domains.iter().any(|d| {
                    host == d.as_str() || host.ends_with(&format!(".{}", d))
                });
            }
        }
        false
    }
}

#[async_trait]
impl Tool for HttpRequestTool {
    fn name(&self) -> &str {
        "http_request"
    }

    fn description(&self) -> &str {
        "Make HTTP requests (GET, POST, PUT, DELETE). Supports JSON bodies and custom headers."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to request"
                },
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "DELETE"],
                    "description": "HTTP method (default: GET)"
                },
                "body": {
                    "description": "Request body (JSON object or string)"
                },
                "headers": {
                    "type": "object",
                    "description": "Additional headers as key-value pairs"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("GET").to_uppercase();

        if url.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing 'url' argument".into() });
        }

        if !self.is_domain_allowed(url) {
            return Ok(ToolResult {
                success: false,
                output: format!("Domain not in allowlist. Allowed: {:?}", self.allowed_domains),
            });
        }

        debug!("http_request: {} {}", method, url);

        let mut req = match method.as_str() {
            "GET" => self.client.get(url),
            "POST" => self.client.post(url),
            "PUT" => self.client.put(url),
            "DELETE" => self.client.delete(url),
            other => return Ok(ToolResult { success: false, output: format!("Unsupported method: {}", other) }),
        };

        // Add custom headers
        if let Some(headers) = args.get("headers").and_then(|v| v.as_object()) {
            for (key, value) in headers {
                if let Some(v) = value.as_str() {
                    req = req.header(key.as_str(), v);
                }
            }
        }

        // Add body for POST/PUT
        if let Some(body) = args.get("body") {
            if body.is_string() {
                req = req.body(body.as_str().unwrap_or("").to_string());
            } else {
                req = req.json(body);
            }
        }

        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                // Truncate response to 10KB
                let truncated = if body.len() > 10240 {
                    format!("{}...\n[truncated, {} bytes total]", &body[..10240], body.len())
                } else {
                    body
                };
                Ok(ToolResult {
                    success: status.is_success(),
                    output: format!("HTTP {} {}\n\n{}", status.as_u16(), status.canonical_reason().unwrap_or(""), truncated),
                })
            }
            Err(e) => Ok(ToolResult { success: false, output: format!("Request failed: {}", e) }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_domain_allowed_wildcard() {
        let tool = HttpRequestTool::new(vec!["*".into()]);
        assert!(tool.is_domain_allowed("https://example.com/api"));
    }

    #[test]
    fn test_domain_allowed_empty() {
        let tool = HttpRequestTool::new(vec![]);
        assert!(tool.is_domain_allowed("https://example.com/api"));
    }

    #[test]
    fn test_domain_allowed_specific() {
        let tool = HttpRequestTool::new(vec!["api.example.com".into()]);
        assert!(tool.is_domain_allowed("https://api.example.com/v1/data"));
        assert!(!tool.is_domain_allowed("https://evil.com/api"));
    }

    #[test]
    fn test_domain_allowed_subdomain() {
        let tool = HttpRequestTool::new(vec!["example.com".into()]);
        assert!(tool.is_domain_allowed("https://api.example.com/v1"));
        assert!(tool.is_domain_allowed("https://example.com/v1"));
    }

    #[tokio::test]
    async fn test_missing_url() {
        let tool = HttpRequestTool::new(vec![]);
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing 'url'"));
    }

    #[tokio::test]
    async fn test_blocked_domain() {
        let tool = HttpRequestTool::new(vec!["allowed.com".into()]);
        let result = tool.execute(json!({"url": "https://blocked.com/api"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("allowlist"));
    }
}
