//! `hermes_text_stats` — word / line / char counts.
//!
//! Concept ported from hermes-agent's text-analytics helper.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{HermesTool, ToolError, ToolResult};

pub struct TextStats;

#[async_trait]
impl HermesTool for TextStats {
    fn name(&self) -> &'static str {
        "hermes_text_stats"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "hermes_text_stats",
                "description": "Return word, line, and char counts for `text`.",
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
        let words = text.split_whitespace().count();
        let lines = if text.is_empty() {
            0
        } else {
            text.lines().count()
        };
        let chars = text.chars().count();
        Ok(json!({ "words": words, "lines": lines, "chars": chars }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn counts_words_lines_chars() {
        let tool = TextStats;
        let r = tool
            .call(&json!({"text": "hello world\nfoo bar baz"}))
            .await
            .unwrap();
        assert_eq!(r["words"], 5);
        assert_eq!(r["lines"], 2);
        assert_eq!(r["chars"], 23);
    }

    #[tokio::test]
    async fn empty_text_returns_zeros() {
        let tool = TextStats;
        let r = tool.call(&json!({"text": ""})).await.unwrap();
        assert_eq!(r["words"], 0);
        assert_eq!(r["lines"], 0);
        assert_eq!(r["chars"], 0);
    }
}
