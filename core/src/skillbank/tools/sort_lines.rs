//! `skill_sort_lines` — sort the lines of a string with optional
//! numeric / unique / desc modes.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{SkillTool, ToolError, ToolResult};

pub struct SortLines;

#[async_trait]
impl SkillTool for SortLines {
    fn name(&self) -> &'static str {
        "skill_sort_lines"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "skill_sort_lines",
                "description": "Sort the lines of `text`. `order` is 'asc' or 'desc' (default 'asc'). \
                    `numeric=true` sorts by f64 value (non-numeric lines first, stable). \
                    `unique=true` collapses adjacent duplicates after sorting.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text":    {"type": "string"},
                        "order":   {"type": "string", "enum": ["asc", "desc"]},
                        "numeric": {"type": "boolean"},
                        "unique":  {"type": "boolean"}
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
        let order = args.get("order").and_then(|v| v.as_str()).unwrap_or("asc");
        if order != "asc" && order != "desc" {
            return Err(ToolError::BadArgs(format!(
                "order must be asc|desc, got {}",
                order
            )));
        }
        let numeric = args
            .get("numeric")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let unique = args
            .get("unique")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();

        if numeric {
            lines.sort_by(|a, b| {
                let na = a.trim().parse::<f64>().ok();
                let nb = b.trim().parse::<f64>().ok();
                match (na, nb) {
                    (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
                    (None, Some(_)) => std::cmp::Ordering::Less,
                    (Some(_), None) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            });
        } else {
            lines.sort();
        }
        if order == "desc" {
            lines.reverse();
        }
        if unique {
            lines.dedup();
        }

        Ok(json!({ "lines": lines, "text": lines.join("\n") }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sorts_lexicographically_ascending_by_default() {
        let tool = SortLines;
        let r = tool
            .call(&json!({"text": "banana\napple\ncherry"}))
            .await
            .unwrap();
        assert_eq!(r["lines"], json!(["apple", "banana", "cherry"]));
        assert_eq!(r["text"], "apple\nbanana\ncherry");
    }

    #[tokio::test]
    async fn descending_numeric_with_unique() {
        let tool = SortLines;
        let r = tool
            .call(&json!({
                "text":    "10\n2\n10\n1\n2",
                "order":   "desc",
                "numeric": true,
                "unique":  true
            }))
            .await
            .unwrap();
        assert_eq!(r["lines"], json!(["10", "2", "1"]));
    }

    #[tokio::test]
    async fn bad_order_is_bad_args() {
        let tool = SortLines;
        let err = tool
            .call(&json!({"text": "x", "order": "sideways"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::BadArgs(_)));
    }
}
