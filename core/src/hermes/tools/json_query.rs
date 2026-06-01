//! `hermes_json_query` — pull a value from JSON via dotted path.
//!
//! Path grammar: segments are `name` for object keys and `[N]` for
//! array indices, joined by `.` (e.g. `users.[0].name` or `users[0].name`).

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{HermesTool, ToolError, ToolResult};

pub struct JsonQuery;

#[async_trait]
impl HermesTool for JsonQuery {
    fn name(&self) -> &'static str {
        "hermes_json_query"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "hermes_json_query",
                "description": "Extract a value from a JSON document using a dotted path. \
                    Path uses `key` for object access and `[N]` for array index.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "json": {"description": "JSON value to query (object/array/etc.)."},
                        "path": {"type": "string", "description": "Dotted path, e.g. 'users[0].name'."}
                    },
                    "required": ["json", "path"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let doc = args
            .get("json")
            .ok_or_else(|| ToolError::BadArgs("json required".into()))?;
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("path required".into()))?;
        let value = lookup(doc, path).map_err(ToolError::Invalid)?;
        Ok(json!({ "value": value }))
    }
}

pub(crate) fn lookup<'a>(doc: &'a Value, path: &str) -> Result<&'a Value, String> {
    let mut current = doc;
    for raw in split_path(path) {
        match raw {
            Segment::Key(k) => {
                current = current
                    .get(&k)
                    .ok_or_else(|| format!("missing key: {}", k))?;
            }
            Segment::Index(i) => {
                current = current
                    .get(i)
                    .ok_or_else(|| format!("missing index: {}", i))?;
            }
        }
    }
    Ok(current)
}

enum Segment {
    Key(String),
    Index(usize),
}

fn split_path(path: &str) -> Vec<Segment> {
    // Example: "users[0].name" → [Key("users"), Index(0), Key("name")]
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '.' => {
                if !buf.is_empty() {
                    out.push(Segment::Key(std::mem::take(&mut buf)));
                }
            }
            '[' => {
                if !buf.is_empty() {
                    out.push(Segment::Key(std::mem::take(&mut buf)));
                }
                let mut num = String::new();
                for nc in chars.by_ref() {
                    if nc == ']' {
                        break;
                    }
                    num.push(nc);
                }
                if let Ok(i) = num.parse() {
                    out.push(Segment::Index(i));
                }
            }
            other => buf.push(other),
        }
    }
    if !buf.is_empty() {
        out.push(Segment::Key(buf));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn extracts_nested_object_value() {
        let tool = JsonQuery;
        let r = tool
            .call(&json!({
                "json": {"users": [{"name": "alice"}, {"name": "bob"}]},
                "path": "users[1].name"
            }))
            .await
            .unwrap();
        assert_eq!(r["value"], "bob");
    }

    #[tokio::test]
    async fn missing_key_is_invalid_error() {
        let tool = JsonQuery;
        let err = tool
            .call(&json!({
                "json": {"a": 1},
                "path": "b"
            }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Invalid(_)));
    }
}
