//! `hermes_yaml_to_json` — parse a YAML string into JSON.
//!
//! Round-trip pair with `hermes_json_to_yaml`. Uses the optional
//! `serde_yaml` dep that the `experimental-hermes-tools` feature
//! activates (already a workspace dep, just gated).

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{HermesTool, ToolError, ToolResult};

pub struct YamlToJson;

#[async_trait]
impl HermesTool for YamlToJson {
    fn name(&self) -> &'static str {
        "hermes_yaml_to_json"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "hermes_yaml_to_json",
                "description": "Parse a YAML document and return the equivalent JSON value.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "yaml": {"type": "string"}
                    },
                    "required": ["yaml"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let src = args
            .get("yaml")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("yaml required".into()))?;
        let parsed: serde_yaml::Value =
            serde_yaml::from_str(src).map_err(|e| ToolError::Invalid(e.to_string()))?;
        let json = yaml_to_json(parsed);
        Ok(json!({ "value": json }))
    }
}

pub(crate) fn yaml_to_json(v: serde_yaml::Value) -> Value {
    match v {
        serde_yaml::Value::Null => Value::Null,
        serde_yaml::Value::Bool(b) => Value::Bool(b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                return Value::Number(i.into());
            }
            if let Some(u) = n.as_u64() {
                return Value::Number(u.into());
            }
            if let Some(f) = n.as_f64() {
                if let Some(num) = serde_json::Number::from_f64(f) {
                    return Value::Number(num);
                }
            }
            Value::Null
        }
        serde_yaml::Value::String(s) => Value::String(s),
        serde_yaml::Value::Sequence(seq) => {
            Value::Array(seq.into_iter().map(yaml_to_json).collect())
        }
        serde_yaml::Value::Mapping(m) => {
            let mut out = serde_json::Map::new();
            for (k, val) in m {
                let key = match k {
                    serde_yaml::Value::String(s) => s,
                    other => serde_yaml::to_string(&other)
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                };
                out.insert(key, yaml_to_json(val));
            }
            Value::Object(out)
        }
        serde_yaml::Value::Tagged(t) => yaml_to_json(t.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parses_simple_mapping() {
        let tool = YamlToJson;
        let r = tool
            .call(&json!({"yaml": "name: alice\nage: 30"}))
            .await
            .unwrap();
        assert_eq!(r["value"], json!({"name": "alice", "age": 30}));
    }

    #[tokio::test]
    async fn parses_nested_sequence() {
        let tool = YamlToJson;
        let r = tool
            .call(&json!({"yaml": "items:\n  - a\n  - b"}))
            .await
            .unwrap();
        assert_eq!(r["value"], json!({"items": ["a", "b"]}));
    }

    #[tokio::test]
    async fn invalid_yaml_is_invalid_error() {
        let tool = YamlToJson;
        let err = tool
            .call(&json!({"yaml": "key: : value"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Invalid(_)));
    }
}
