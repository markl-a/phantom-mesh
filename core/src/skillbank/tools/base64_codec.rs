//! `skill_base64_codec` — encode/decode base64 strings.
//!
//! Uses the existing `base64` dep (no extra Cargo cost).

use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};

use super::{SkillTool, ToolError, ToolResult};

pub struct Base64Codec;

#[async_trait]
impl SkillTool for Base64Codec {
    fn name(&self) -> &'static str {
        "skill_base64_codec"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "skill_base64_codec",
                "description": "Encode (`op=encode`) or decode (`op=decode`) base64. \
                    Decode returns UTF-8 text or an error if the input is not valid UTF-8.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "op":   { "type": "string", "enum": ["encode", "decode"] },
                        "data": { "type": "string" }
                    },
                    "required": ["op", "data"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let op = args
            .get("op")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("op required".into()))?;
        let data = args
            .get("data")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("data required".into()))?;
        let engine = base64::engine::general_purpose::STANDARD;
        match op {
            "encode" => Ok(json!({ "result": engine.encode(data.as_bytes()) })),
            "decode" => {
                let bytes = engine
                    .decode(data)
                    .map_err(|e| ToolError::Invalid(e.to_string()))?;
                let s = String::from_utf8(bytes)
                    .map_err(|e| ToolError::Invalid(format!("not utf8: {}", e)))?;
                Ok(json!({ "result": s }))
            }
            other => Err(ToolError::BadArgs(format!("unknown op: {}", other))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn encode_then_decode_roundtrips() {
        let tool = Base64Codec;
        let enc = tool
            .call(&json!({"op": "encode", "data": "hello"}))
            .await
            .unwrap();
        assert_eq!(enc["result"], "aGVsbG8=");
        let dec = tool
            .call(&json!({"op": "decode", "data": "aGVsbG8="}))
            .await
            .unwrap();
        assert_eq!(dec["result"], "hello");
    }

    #[tokio::test]
    async fn bad_decode_is_invalid_error() {
        let tool = Base64Codec;
        let err = tool
            .call(&json!({"op": "decode", "data": "!!!not-base64!!!"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Invalid(_)));
    }
}
