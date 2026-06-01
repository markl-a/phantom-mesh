//! `hermes_datetime` — UTC clock + RFC3339 parse/format + duration diff.
//!
//! Concept ported from hermes-agent's datetime helper. Uses chrono
//! (only compiled when the feature is on).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use super::{HermesTool, ToolError, ToolResult};

pub struct DateTimeTool;

#[async_trait]
impl HermesTool for DateTimeTool {
    fn name(&self) -> &'static str {
        "hermes_datetime"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "hermes_datetime",
                "description": "Datetime helpers. Op = 'now' returns current UTC timestamp. \
                    Op = 'diff' returns seconds between two RFC3339 timestamps `a` and `b`.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "op": { "type": "string", "enum": ["now", "diff"] },
                        "a":  { "type": "string", "description": "RFC3339 timestamp (op=diff)." },
                        "b":  { "type": "string", "description": "RFC3339 timestamp (op=diff)." }
                    },
                    "required": ["op"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let op = args
            .get("op")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("op required".into()))?;
        match op {
            "now" => {
                let now: DateTime<Utc> = Utc::now();
                Ok(json!({ "iso": now.to_rfc3339(), "epoch_secs": now.timestamp() }))
            }
            "diff" => {
                let a = args
                    .get("a")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::BadArgs("a required for diff".into()))?;
                let b = args
                    .get("b")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::BadArgs("b required for diff".into()))?;
                let ta = DateTime::parse_from_rfc3339(a)
                    .map_err(|e| ToolError::Invalid(format!("a: {}", e)))?;
                let tb = DateTime::parse_from_rfc3339(b)
                    .map_err(|e| ToolError::Invalid(format!("b: {}", e)))?;
                Ok(json!({ "diff_secs": (tb - ta).num_seconds() }))
            }
            other => Err(ToolError::BadArgs(format!("unknown op: {}", other))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn now_returns_iso_and_epoch() {
        let tool = DateTimeTool;
        let r = tool.call(&json!({"op": "now"})).await.unwrap();
        assert!(r["iso"].as_str().unwrap().contains('T'));
        assert!(r["epoch_secs"].as_i64().unwrap() > 1_700_000_000);
    }

    #[tokio::test]
    async fn diff_in_seconds_is_positive_when_b_after_a() {
        let tool = DateTimeTool;
        let r = tool
            .call(&json!({
                "op": "diff",
                "a": "2026-01-01T00:00:00Z",
                "b": "2026-01-01T00:00:42Z"
            }))
            .await
            .unwrap();
        assert_eq!(r["diff_secs"], 42);
    }

    #[tokio::test]
    async fn diff_with_bad_input_is_invalid() {
        let tool = DateTimeTool;
        let err = tool
            .call(&json!({"op": "diff", "a": "garbage", "b": "2026-01-01T00:00:00Z"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Invalid(_)));
    }
}
