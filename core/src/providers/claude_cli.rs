use serde_json::Value;

pub fn extract_claude_token(json: &Value) -> Option<String> {
    if let Some(obj) = json.as_object() {
        for (_key, val) in obj {
            if let Some(token) = val.as_str() {
                if token.starts_with("sk-ant-") {
                    return Some(token.to_string());
                }
            }
            if let Some(token) = val.get("token").and_then(|t| t.as_str()) {
                return Some(token.to_string());
            }
        }
    }
    None
}
