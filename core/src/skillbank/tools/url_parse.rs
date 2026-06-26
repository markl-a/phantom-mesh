//! `skill_url_parse` — split URL into components.
//!
//! Uses the `url` crate (only compiled when the feature is on).

use async_trait::async_trait;
use serde_json::{json, Value};
use url::Url;

use super::{SkillTool, ToolError, ToolResult};

pub struct UrlParse;

#[async_trait]
impl SkillTool for UrlParse {
    fn name(&self) -> &'static str {
        "skill_url_parse"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "skill_url_parse",
                "description": "Parse a URL into its components: scheme, host, port, path, query, fragment.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {"type": "string"}
                    },
                    "required": ["url"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let raw = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("url required".into()))?;
        let parsed = Url::parse(raw).map_err(|e| ToolError::Invalid(e.to_string()))?;
        Ok(json!({
            "scheme":   parsed.scheme(),
            "host":     parsed.host_str(),
            "port":     parsed.port_or_known_default(),
            "path":     parsed.path(),
            "query":    parsed.query(),
            "fragment": parsed.fragment()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parses_full_url() {
        let tool = UrlParse;
        let r = tool
            .call(&json!({"url": "https://api.example.com:8443/v1/items?id=42#x"}))
            .await
            .unwrap();
        assert_eq!(r["scheme"], "https");
        assert_eq!(r["host"], "api.example.com");
        assert_eq!(r["port"], 8443);
        assert_eq!(r["path"], "/v1/items");
        assert_eq!(r["query"], "id=42");
        assert_eq!(r["fragment"], "x");
    }

    #[tokio::test]
    async fn invalid_url_is_error() {
        let tool = UrlParse;
        let err = tool.call(&json!({"url": "not a url"})).await.unwrap_err();
        assert!(matches!(err, ToolError::Invalid(_)));
    }
}
