//! `skill_grep` — regex search across a string, returning each match
//! with its 1-based line number and the line text.
//!
//! Uses the existing `regex` dep (no new Cargo cost).

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};

use super::{SkillTool, ToolError, ToolResult};

pub struct Grep;

#[async_trait]
impl SkillTool for Grep {
    fn name(&self) -> &'static str {
        "skill_grep"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "skill_grep",
                "description": "Search `text` for regex `pattern`. Returns every match \
                    as `{line: <1-based>, text: <line>, match: <substring>}`. \
                    If `case_insensitive=true`, the regex is compiled with `(?i)`.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern":          {"type": "string"},
                        "text":             {"type": "string"},
                        "case_insensitive": {"type": "boolean"}
                    },
                    "required": ["pattern", "text"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("pattern required".into()))?;
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("text required".into()))?;
        let ci = args
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let effective = if ci {
            format!("(?i){}", pattern)
        } else {
            pattern.to_string()
        };
        let re = Regex::new(&effective).map_err(|e| ToolError::Invalid(e.to_string()))?;
        let mut matches = Vec::new();
        for (idx, line) in text.lines().enumerate() {
            for m in re.find_iter(line) {
                matches.push(json!({
                    "line":  idx + 1,
                    "text":  line,
                    "match": m.as_str(),
                }));
            }
        }
        Ok(json!({ "matches": matches, "count": matches.len() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn matches_have_line_numbers() {
        let tool = Grep;
        let r = tool
            .call(&json!({
                "pattern": r"\bfoo\b",
                "text":    "alpha\nfoo bar\nbaz foo\nqux"
            }))
            .await
            .unwrap();
        assert_eq!(r["count"], 2);
        assert_eq!(r["matches"][0]["line"], 2);
        assert_eq!(r["matches"][0]["match"], "foo");
        assert_eq!(r["matches"][1]["line"], 3);
    }

    #[tokio::test]
    async fn case_insensitive_flag_is_honoured() {
        let tool = Grep;
        let r = tool
            .call(&json!({
                "pattern":          "foo",
                "text":             "FOO\nbar",
                "case_insensitive": true
            }))
            .await
            .unwrap();
        assert_eq!(r["count"], 1);
        assert_eq!(r["matches"][0]["line"], 1);
    }

    #[tokio::test]
    async fn bad_regex_is_invalid_error() {
        let tool = Grep;
        let err = tool
            .call(&json!({"pattern": "(", "text": "x"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Invalid(_)));
    }
}
