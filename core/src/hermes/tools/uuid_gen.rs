//! `hermes_uuid_gen` — generate a UUID v4.
//!
//! Uses the existing `uuid` dep (no extra Cargo cost).

use async_trait::async_trait;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{HermesTool, ToolResult};

pub struct UuidGen;

#[async_trait]
impl HermesTool for UuidGen {
    fn name(&self) -> &'static str {
        "hermes_uuid_gen"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "hermes_uuid_gen",
                "description": "Generate a UUID v4 (random). Returns hyphenated lowercase form.",
                "parameters": { "type": "object", "properties": {} }
            }
        })
    }

    async fn call(&self, _args: &Value) -> ToolResult {
        Ok(json!({ "uuid": Uuid::new_v4().to_string() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn produces_well_formed_v4_uuid() {
        let tool = UuidGen;
        let r = tool.call(&json!({})).await.unwrap();
        let s = r["uuid"].as_str().unwrap();
        assert_eq!(s.len(), 36);
        // v4 has '4' as the 14th char and one of [89ab] as the 19th (hyphenated form).
        let chars: Vec<char> = s.chars().collect();
        assert_eq!(chars[14], '4');
        assert!(matches!(chars[19], '8' | '9' | 'a' | 'b'));
    }

    #[tokio::test]
    async fn produces_distinct_uuids_each_call() {
        let tool = UuidGen;
        let a = tool.call(&json!({})).await.unwrap();
        let b = tool.call(&json!({})).await.unwrap();
        assert_ne!(a["uuid"], b["uuid"]);
    }
}
