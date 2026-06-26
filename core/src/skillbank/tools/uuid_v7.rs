//! `skill_uuid_v7` — generate a UUID v7 (Unix timestamp + random).
//!
//! v7 is time-ordered: leading 48 bits are Unix ms, then version (4 bits)
//! and 12 random bits, then variant (2 bits) and 62 random bits. The
//! `uuid` crate's `v7` feature gives us a turnkey impl using the `rand`
//! generator already in workspace deps.

use async_trait::async_trait;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{SkillTool, ToolResult};

pub struct UuidV7;

#[async_trait]
impl SkillTool for UuidV7 {
    fn name(&self) -> &'static str {
        "skill_uuid_v7"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "skill_uuid_v7",
                "description": "Generate a time-ordered UUID v7. Returns hyphenated lowercase form. \
                    Unlike v4, v7s sort by creation time when compared lexically.",
                "parameters": { "type": "object", "properties": {} }
            }
        })
    }

    async fn call(&self, _args: &Value) -> ToolResult {
        Ok(json!({ "uuid": Uuid::now_v7().to_string() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn produces_well_formed_v7_uuid() {
        let tool = UuidV7;
        let r = tool.call(&json!({})).await.unwrap();
        let s = r["uuid"].as_str().unwrap();
        assert_eq!(s.len(), 36);
        // v7 has '7' as the 14th char (version nibble) and one of [89ab] as the 19th
        // (variant nibble), same positions as v4.
        let chars: Vec<char> = s.chars().collect();
        assert_eq!(chars[14], '7', "version nibble should be 7 in {}", s);
        assert!(matches!(chars[19], '8' | '9' | 'a' | 'b'));
    }

    #[tokio::test]
    async fn consecutive_uuids_are_monotonically_ordered() {
        let tool = UuidV7;
        let a = tool.call(&json!({})).await.unwrap();
        // Force a sub-ms delay so the timestamp prefix differs.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let b = tool.call(&json!({})).await.unwrap();
        let sa = a["uuid"].as_str().unwrap();
        let sb = b["uuid"].as_str().unwrap();
        assert!(sa < sb, "v7 should sort by time: {} vs {}", sa, sb);
    }

    #[tokio::test]
    async fn produces_distinct_uuids_each_call() {
        let tool = UuidV7;
        let a = tool.call(&json!({})).await.unwrap();
        let b = tool.call(&json!({})).await.unwrap();
        assert_ne!(a["uuid"], b["uuid"]);
    }
}
