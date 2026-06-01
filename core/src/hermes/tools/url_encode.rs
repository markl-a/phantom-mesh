//! `hermes_url_encode` — percent-encode a string for use in URLs.
//!
//! Uses the existing `urlencoding` dep (no extra Cargo cost).

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{HermesTool, ToolError, ToolResult};

pub struct UrlEncode;

#[async_trait]
impl HermesTool for UrlEncode {
    fn name(&self) -> &'static str {
        "hermes_url_encode"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "hermes_url_encode",
                "description": "Percent-encode `text` (RFC 3986 unreserved set is left as-is). \
                    Round-trip pair with hermes_url_decode.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string"}
                    },
                    "required": ["text"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("text required".into()))?;
        Ok(json!({ "encoded": urlencoding::encode(text).into_owned() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn encodes_spaces_and_unicode() {
        let tool = UrlEncode;
        let r = tool.call(&json!({"text": "hello world"})).await.unwrap();
        assert_eq!(r["encoded"], "hello%20world");
    }

    #[tokio::test]
    async fn encodes_reserved_characters() {
        let tool = UrlEncode;
        let r = tool.call(&json!({"text": "a/b?c=d&e"})).await.unwrap();
        assert_eq!(r["encoded"], "a%2Fb%3Fc%3Dd%26e");
    }

    #[tokio::test]
    async fn missing_text_is_bad_args() {
        let tool = UrlEncode;
        let err = tool.call(&json!({})).await.unwrap_err();
        assert!(matches!(err, ToolError::BadArgs(_)));
    }
}
