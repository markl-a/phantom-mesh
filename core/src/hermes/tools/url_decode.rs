//! `hermes_url_decode` — reverse of `hermes_url_encode`.
//!
//! Uses the existing `urlencoding` dep (no extra Cargo cost).

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{HermesTool, ToolError, ToolResult};

pub struct UrlDecode;

#[async_trait]
impl HermesTool for UrlDecode {
    fn name(&self) -> &'static str {
        "hermes_url_decode"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "hermes_url_decode",
                "description": "Decode percent-encoded `text`. Round-trip pair with hermes_url_encode. \
                    Returns Invalid on non-UTF-8 byte sequences.",
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
        let decoded = urlencoding::decode(text)
            .map_err(|e| ToolError::Invalid(e.to_string()))?
            .into_owned();
        Ok(json!({ "decoded": decoded }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn decodes_percent_encoded_spaces() {
        let tool = UrlDecode;
        let r = tool.call(&json!({"text": "hello%20world"})).await.unwrap();
        assert_eq!(r["decoded"], "hello world");
    }

    #[tokio::test]
    async fn decodes_reserved_characters() {
        let tool = UrlDecode;
        let r = tool
            .call(&json!({"text": "a%2Fb%3Fc%3Dd%26e"}))
            .await
            .unwrap();
        assert_eq!(r["decoded"], "a/b?c=d&e");
    }

    #[tokio::test]
    async fn round_trip_with_encode() {
        // Encoded then decoded should match the original.
        let tool = UrlDecode;
        let original = "key=value & weird/?chars";
        let encoded = urlencoding::encode(original).into_owned();
        let r = tool.call(&json!({"text": encoded})).await.unwrap();
        assert_eq!(r["decoded"], original);
    }
}
