//! `skill_regex_extract` — extract regex matches from text.
//!
//! Uses the existing `regex` dep (no extra Cargo cost).

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};

use super::{SkillTool, ToolError, ToolResult};

pub struct RegexExtract;

#[async_trait]
impl SkillTool for RegexExtract {
    fn name(&self) -> &'static str {
        "skill_regex_extract"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "skill_regex_extract",
                "description": "Extract regex matches from `text`. With `all=true`, returns every match; \
                    otherwise returns the first match (or null).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": {"type": "string"},
                        "text":    {"type": "string"},
                        "all":     {"type": "boolean", "description": "Return all matches if true."}
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
        let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
        let re = Regex::new(pattern).map_err(|e| ToolError::Invalid(e.to_string()))?;
        if all {
            let matches: Vec<&str> = re.find_iter(text).map(|m| m.as_str()).collect();
            Ok(json!({ "matches": matches }))
        } else {
            let first = re.find(text).map(|m| m.as_str());
            Ok(json!({ "match": first }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn extracts_first_match_by_default() {
        let tool = RegexExtract;
        let r = tool
            .call(&json!({"pattern": r"\d+", "text": "a1 b22 c333"}))
            .await
            .unwrap();
        assert_eq!(r["match"], "1");
    }

    #[tokio::test]
    async fn extracts_all_matches_when_requested() {
        let tool = RegexExtract;
        let r = tool
            .call(&json!({"pattern": r"\d+", "text": "a1 b22 c333", "all": true}))
            .await
            .unwrap();
        assert_eq!(r["matches"], json!(["1", "22", "333"]));
    }

    #[tokio::test]
    async fn bad_regex_is_invalid_error() {
        let tool = RegexExtract;
        let err = tool
            .call(&json!({"pattern": "(", "text": "x"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Invalid(_)));
    }
}
