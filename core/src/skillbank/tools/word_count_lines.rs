//! `skill_word_count_lines` — combined `wc`-style report on a string.
//!
//! Goes beyond `skill_text_stats` by additionally returning:
//!   * `bytes`: UTF-8 byte length
//!   * `chars_no_ws`: char count excluding ASCII whitespace
//!   * `avg_line_len`: mean characters per non-empty line (0 for empty input)
//!   * `longest_line`: longest line length in characters (0 for empty input)

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{SkillTool, ToolError, ToolResult};

pub struct WordCountLines;

#[async_trait]
impl SkillTool for WordCountLines {
    fn name(&self) -> &'static str {
        "skill_word_count_lines"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "skill_word_count_lines",
                "description": "wc-style combined report: lines, words, chars, bytes, chars_no_ws, \
                    avg_line_len (over non-empty lines), longest_line.",
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
        let bytes = text.len();
        let chars_no_ws = text.chars().filter(|c| !c.is_whitespace()).count();
        let mut nonempty = 0usize;
        let mut total = 0usize;
        let mut longest = 0usize;
        for line in text.lines() {
            let len = line.chars().count();
            if len > longest {
                longest = len;
            }
            if !line.trim().is_empty() {
                nonempty += 1;
                total += len;
            }
        }
        let avg_line_len = if nonempty == 0 {
            0.0
        } else {
            total as f64 / nonempty as f64
        };
        Ok(json!({
            "lines": lines,
            "words": words,
            "chars": chars,
            "bytes": bytes,
            "chars_no_ws": chars_no_ws,
            "avg_line_len": avg_line_len,
            "longest_line": longest
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn counts_basic_text() {
        let tool = WordCountLines;
        let r = tool
            .call(&json!({"text": "hello world\nfoo bar baz"}))
            .await
            .unwrap();
        assert_eq!(r["lines"], 2);
        assert_eq!(r["words"], 5);
        assert_eq!(r["chars"], 23);
        assert_eq!(r["bytes"], 23);
        assert_eq!(r["chars_no_ws"], 19); // 23 chars - 4 whitespace (3 spaces + 1 newline)
        assert_eq!(r["longest_line"], 11); // "hello world"
    }

    #[tokio::test]
    async fn empty_input_zeros_everything() {
        let tool = WordCountLines;
        let r = tool.call(&json!({"text": ""})).await.unwrap();
        assert_eq!(r["lines"], 0);
        assert_eq!(r["words"], 0);
        assert_eq!(r["chars"], 0);
        assert_eq!(r["bytes"], 0);
        assert_eq!(r["chars_no_ws"], 0);
        assert_eq!(r["avg_line_len"], 0.0);
        assert_eq!(r["longest_line"], 0);
    }

    #[tokio::test]
    async fn unicode_chars_distinct_from_bytes() {
        let tool = WordCountLines;
        let r = tool.call(&json!({"text": "café"})).await.unwrap();
        assert_eq!(r["chars"], 4);
        assert_eq!(r["bytes"], 5); // é = 2 bytes in UTF-8
    }
}
