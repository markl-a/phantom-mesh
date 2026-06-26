use serde_json::Value;

/// Extract a Claude token from a parsed credentials JSON.
///
/// Handles the shapes seen across Claude Code versions:
///   - the modern file/Keychain shape
///     `{ "claudeAiOauth": { "accessToken": "sk-ant-oat…" } }`,
///   - a bare `{ "accessToken": "…" }`,
///   - a `{ "<provider>": { "token": "…" } }` wrapper,
///   - a top-level value that is itself an `sk-ant-*` string.
pub fn extract_claude_token(json: &Value) -> Option<String> {
    // Modern Claude Code: nested OAuth object (file or macOS Keychain).
    for key in ["claudeAiOauth", "claude_ai_oauth"] {
        if let Some(tok) = json
            .get(key)
            .and_then(|o| o.get("accessToken").or_else(|| o.get("access_token")))
            .and_then(|t| t.as_str())
        {
            return Some(tok.to_string());
        }
    }
    // A bare access token at the top level.
    if let Some(tok) = json
        .get("accessToken")
        .or_else(|| json.get("access_token"))
        .and_then(|t| t.as_str())
    {
        return Some(tok.to_string());
    }
    // Older shapes: any top-level `sk-ant-*` string or a nested `.token` field.
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

/// True when `token` is a Claude **OAuth** access token (subscription login via
/// Claude Code, `sk-ant-oat…`) rather than a console **API key** (`sk-ant-api…`).
///
/// OAuth tokens must be sent as `Authorization: Bearer` + the oauth beta header;
/// API keys use `x-api-key`.
pub fn is_oauth_token(token: &str) -> bool {
    token.starts_with("sk-ant-oat")
}

/// Locate the current Claude Code token from the local CLI cache.
///
/// Tries each credentials file ([`credential_scanner::claude_cli_paths`]), then
/// (on macOS) the login-Keychain item the Claude Code CLI writes. Returns the
/// live token so callers always use the auto-refreshed value rather than a
/// persisted copy.
///
/// NOTE: reading the Keychain item from a different binary can prompt the user
/// to authorise access the first time — that is expected.
pub fn find_claude_token() -> Option<String> {
    use super::credential_scanner;
    for path in credential_scanner::claude_cli_paths() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(json) = serde_json::from_str::<Value>(&content) {
                if let Some(tok) = extract_claude_token(&json) {
                    return Some(tok);
                }
            }
        }
    }
    if let Some(content) = credential_scanner::claude_cli_keychain_json() {
        if let Ok(json) = serde_json::from_str::<Value>(&content) {
            if let Some(tok) = extract_claude_token(&json) {
                return Some(tok);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_modern_oauth_shape() {
        let j = serde_json::json!({
            "claudeAiOauth": { "accessToken": "sk-ant-oat01-abc", "refreshToken": "sk-ant-ort01-x" }
        });
        assert_eq!(extract_claude_token(&j).as_deref(), Some("sk-ant-oat01-abc"));
    }

    #[test]
    fn extracts_legacy_top_level_key() {
        let j = serde_json::json!({ "anthropic": "sk-ant-api03-xyz" });
        assert_eq!(extract_claude_token(&j).as_deref(), Some("sk-ant-api03-xyz"));
    }

    #[test]
    fn classifies_oauth_vs_api_key() {
        assert!(is_oauth_token("sk-ant-oat01-abc"));
        assert!(!is_oauth_token("sk-ant-api03-xyz"));
    }
}
