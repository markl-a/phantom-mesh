//! `skill_word_freq` — count word frequencies and return the top-N.
//!
//! Tokenisation: split on any char that is NOT alphanumeric or apostrophe;
//! lowercase via `str::to_lowercase()`. Returns words sorted by count
//! descending, ties broken by lexicographic order (deterministic).

use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

use super::{SkillTool, ToolError, ToolResult};

pub struct WordFreq;

#[async_trait]
impl SkillTool for WordFreq {
    fn name(&self) -> &'static str {
        "skill_word_freq"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "skill_word_freq",
                "description": "Tokenise `text` (lowercase, split on non-alphanumeric except `'`) \
                    and return the top `top_n` words by frequency. \
                    Ties broken lexicographically. `top_n` defaults to 10.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text":  {"type": "string"},
                        "top_n": {"type": "integer", "minimum": 1}
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
        let top_n = args.get("top_n").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        if top_n == 0 {
            return Err(ToolError::BadArgs("top_n must be >= 1".into()));
        }

        let lower = text.to_lowercase();
        let mut counts: HashMap<String, usize> = HashMap::new();
        let mut current = String::new();
        for c in lower.chars() {
            if c.is_alphanumeric() || c == '\'' {
                current.push(c);
            } else if !current.is_empty() {
                *counts.entry(std::mem::take(&mut current)).or_insert(0) += 1;
            }
        }
        if !current.is_empty() {
            *counts.entry(current).or_insert(0) += 1;
        }

        let mut pairs: Vec<(String, usize)> = counts.into_iter().collect();
        // Sort by count desc, then word asc.
        pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let top: Vec<Value> = pairs
            .into_iter()
            .take(top_n)
            .map(|(w, c)| json!({"word": w, "count": c}))
            .collect();
        Ok(json!({ "top": top }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn counts_and_orders_by_descending_frequency() {
        let tool = WordFreq;
        let r = tool
            .call(&json!({
                "text":  "the quick brown fox the lazy dog the",
                "top_n": 3
            }))
            .await
            .unwrap();
        let top = r["top"].as_array().unwrap();
        assert_eq!(top.len(), 3);
        assert_eq!(top[0]["word"], "the");
        assert_eq!(top[0]["count"], 3);
        // Remaining 1-counts tied → lexicographic order.
        assert_eq!(top[1]["word"], "brown");
        assert_eq!(top[2]["word"], "dog");
    }

    #[tokio::test]
    async fn punctuation_is_stripped_and_apostrophes_kept() {
        let tool = WordFreq;
        let r = tool
            .call(&json!({"text": "Don't! Don't. Don't?", "top_n": 5}))
            .await
            .unwrap();
        let top = r["top"].as_array().unwrap();
        assert_eq!(top.len(), 1);
        assert_eq!(top[0]["word"], "don't");
        assert_eq!(top[0]["count"], 3);
    }

    #[tokio::test]
    async fn zero_top_n_is_bad_args() {
        let tool = WordFreq;
        let err = tool
            .call(&json!({"text": "a b", "top_n": 0}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::BadArgs(_)));
    }
}
