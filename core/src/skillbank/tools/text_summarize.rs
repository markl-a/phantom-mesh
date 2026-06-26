//! `skill_text_summarize` — naive extractive summary.
//!
//! Splits on `. ! ?` then keeps the first `head` and last `tail`
//! sentences. Deterministic, no LLM call. Useful as a context-trim
//! prelude when an LLM is downstream.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{SkillTool, ToolError, ToolResult};

pub struct TextSummarize;

#[async_trait]
impl SkillTool for TextSummarize {
    fn name(&self) -> &'static str {
        "skill_text_summarize"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "skill_text_summarize",
                "description": "Naive extractive summary: keep first `head` and last `tail` sentences.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string"},
                        "head": {"type": "integer", "description": "Sentences from the start (default 2)."},
                        "tail": {"type": "integer", "description": "Sentences from the end (default 1)."}
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
        let head = args.get("head").and_then(|v| v.as_u64()).unwrap_or(2) as usize;
        let tail = args.get("tail").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
        let sentences: Vec<&str> = split_sentences(text);
        let summary = if sentences.len() <= head + tail {
            sentences.join(" ")
        } else {
            let mut out: Vec<&str> = sentences.iter().take(head).copied().collect();
            out.extend(sentences.iter().rev().take(tail).rev().copied());
            out.join(" ")
        };
        Ok(json!({ "summary": summary, "sentence_count": sentences.len() }))
    }
}

pub(crate) fn split_sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if matches!(b, b'.' | b'!' | b'?') {
            let end = i + 1;
            let s = text[start..end].trim();
            if !s.is_empty() {
                out.push(s);
            }
            start = end;
        }
    }
    let trailing = text[start..].trim();
    if !trailing.is_empty() {
        out.push(trailing);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn keeps_head_and_tail() {
        let tool = TextSummarize;
        let r = tool
            .call(&json!({
                "text": "First. Second. Third. Fourth. Fifth.",
                "head": 1,
                "tail": 1
            }))
            .await
            .unwrap();
        assert_eq!(r["summary"], "First. Fifth.");
        assert_eq!(r["sentence_count"], 5);
    }

    #[tokio::test]
    async fn returns_full_text_when_short() {
        let tool = TextSummarize;
        let r = tool.call(&json!({"text": "Only one."})).await.unwrap();
        assert_eq!(r["summary"], "Only one.");
        assert_eq!(r["sentence_count"], 1);
    }
}
