//! JSON transform tool — query and transform JSON using JSON pointer paths.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolResult};

pub struct JsonTransformTool;

impl JsonTransformTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl Tool for JsonTransformTool {
    fn name(&self) -> &str { "json_transform" }

    fn description(&self) -> &str {
        "Query and transform JSON data. Operations: get, keys, values, flatten, count, filter, pretty."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "json_input": { "type": "string", "description": "JSON string to transform" },
                "operation": { "type": "string", "description": "One of: get, keys, values, flatten, count, filter, pretty" },
                "path": { "type": "string", "description": "JSON pointer path (e.g. /data/items/0/name)" },
                "filter_key": { "type": "string", "description": "Key to filter by (for 'filter' operation)" },
                "filter_value": { "type": "string", "description": "Value to match (for 'filter' operation)" }
            },
            "required": ["json_input", "operation"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let json_str = args["json_input"].as_str().unwrap_or("").trim();
        let operation = args["operation"].as_str().unwrap_or("").trim();
        let path = args["path"].as_str().unwrap_or("").trim();

        if json_str.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing required parameter: json_input".into() });
        }
        if operation.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing required parameter: operation".into() });
        }

        let data: Value = match serde_json::from_str(json_str) {
            Ok(v) => v,
            Err(e) => return Ok(ToolResult { success: false, output: format!("Invalid JSON: {}", e) }),
        };

        // Resolve path
        let target = if path.is_empty() || path == "/" {
            &data
        } else {
            match data.pointer(path) {
                Some(v) => v,
                None => return Ok(ToolResult { success: false, output: format!("Path '{}' not found", path) }),
            }
        };

        let result = match operation {
            "get" => {
                serde_json::to_string(target)?
            }
            "keys" => {
                match target.as_object() {
                    Some(obj) => {
                        let keys: Vec<_> = obj.keys().collect();
                        serde_json::to_string(&keys)?
                    }
                    None => return Ok(ToolResult { success: false, output: "Target is not an object (cannot get keys)".into() }),
                }
            }
            "values" => {
                match target.as_object() {
                    Some(obj) => {
                        let vals: Vec<_> = obj.values().collect();
                        serde_json::to_string(&vals)?
                    }
                    None => return Ok(ToolResult { success: false, output: "Target is not an object (cannot get values)".into() }),
                }
            }
            "count" => {
                let count = match target {
                    Value::Array(arr) => arr.len(),
                    Value::Object(obj) => obj.len(),
                    _ => return Ok(ToolResult { success: false, output: "Target is not an array or object".into() }),
                };
                count.to_string()
            }
            "flatten" => {
                let mut flat = serde_json::Map::new();
                flatten_value(&data, String::new(), &mut flat);
                serde_json::to_string(&Value::Object(flat))?
            }
            "filter" => {
                let filter_key = args["filter_key"].as_str().unwrap_or("").trim();
                let filter_value = args["filter_value"].as_str().unwrap_or("").trim();
                if filter_key.is_empty() {
                    return Ok(ToolResult { success: false, output: "filter operation requires filter_key".into() });
                }
                match target.as_array() {
                    Some(arr) => {
                        let filtered: Vec<_> = arr.iter()
                            .filter(|item| {
                                item.get(filter_key)
                                    .map(|v| match v {
                                        Value::String(s) => s == filter_value,
                                        other => other.to_string().trim_matches('"') == filter_value,
                                    })
                                    .unwrap_or(false)
                            })
                            .collect();
                        serde_json::to_string(&filtered)?
                    }
                    None => return Ok(ToolResult { success: false, output: "Target is not an array (cannot filter)".into() }),
                }
            }
            "pretty" => {
                serde_json::to_string_pretty(target)?
            }
            _ => return Ok(ToolResult { success: false, output: format!("Unknown operation: '{}'. Use: get, keys, values, flatten, count, filter, pretty", operation) }),
        };

        Ok(ToolResult { success: true, output: result })
    }
}

/// Recursively flatten a JSON value into dot-notation keys
fn flatten_value(value: &Value, prefix: String, out: &mut serde_json::Map<String, Value>) {
    match value {
        Value::Object(obj) => {
            for (key, val) in obj {
                let new_key = if prefix.is_empty() { key.clone() } else { format!("{}.{}", prefix, key) };
                flatten_value(val, new_key, out);
            }
        }
        Value::Array(arr) => {
            for (i, val) in arr.iter().enumerate() {
                let new_key = if prefix.is_empty() { i.to_string() } else { format!("{}.{}", prefix, i) };
                flatten_value(val, new_key, out);
            }
        }
        _ => {
            out.insert(prefix, value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        assert_eq!(JsonTransformTool::new().name(), "json_transform");
    }

    #[test]
    fn test_schema() {
        let tool = JsonTransformTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["json_input"].is_object());
        assert!(schema["properties"]["operation"].is_object());
    }

    #[tokio::test]
    async fn test_get_path() {
        let tool = JsonTransformTool::new();
        let input = json!({"json_input": r#"{"data":{"name":"test"}}"#, "operation": "get", "path": "/data/name"});
        let result = tool.execute(input).await.unwrap();
        assert!(result.success);
        assert_eq!(result.output, "\"test\"");
    }

    #[tokio::test]
    async fn test_keys() {
        let tool = JsonTransformTool::new();
        let input = json!({"json_input": r#"{"a":1,"b":2,"c":3}"#, "operation": "keys"});
        let result = tool.execute(input).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("a"));
        assert!(result.output.contains("b"));
    }

    #[tokio::test]
    async fn test_count_array() {
        let tool = JsonTransformTool::new();
        let input = json!({"json_input": "[1,2,3,4,5]", "operation": "count"});
        let result = tool.execute(input).await.unwrap();
        assert!(result.success);
        assert_eq!(result.output, "5");
    }

    #[tokio::test]
    async fn test_count_object() {
        let tool = JsonTransformTool::new();
        let input = json!({"json_input": r#"{"a":1,"b":2}"#, "operation": "count"});
        let result = tool.execute(input).await.unwrap();
        assert!(result.success);
        assert_eq!(result.output, "2");
    }

    #[tokio::test]
    async fn test_flatten() {
        let tool = JsonTransformTool::new();
        let input = json!({"json_input": r#"{"a":{"b":1},"c":2}"#, "operation": "flatten"});
        let result = tool.execute(input).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("a.b"));
    }

    #[tokio::test]
    async fn test_pretty() {
        let tool = JsonTransformTool::new();
        let input = json!({"json_input": r#"{"a":1}"#, "operation": "pretty"});
        let result = tool.execute(input).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains('\n'));
    }

    #[tokio::test]
    async fn test_invalid_json() {
        let tool = JsonTransformTool::new();
        let input = json!({"json_input": "not json", "operation": "get"});
        let result = tool.execute(input).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Invalid JSON"));
    }

    #[tokio::test]
    async fn test_filter() {
        let tool = JsonTransformTool::new();
        let input = json!({
            "json_input": r#"[{"name":"alice","age":30},{"name":"bob","age":25}]"#,
            "operation": "filter",
            "filter_key": "name",
            "filter_value": "bob"
        });
        let result = tool.execute(input).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("bob"));
        assert!(!result.output.contains("alice"));
    }

    #[tokio::test]
    async fn test_unknown_operation() {
        let tool = JsonTransformTool::new();
        let input = json!({"json_input": "{}", "operation": "nope"});
        let result = tool.execute(input).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown operation"));
    }
}
