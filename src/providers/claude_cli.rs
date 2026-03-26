use anyhow::{anyhow, Result};
use std::path::PathBuf;
use tokio::sync::Mutex;
use tracing::debug;

// ── Token Extraction ───────────────────────────────────────

/// Try multiple known JSON field names for Claude CLI auth files
pub fn extract_claude_token(json: &serde_json::Value) -> Option<String> {
    for key in &["sessionKey", "token", "access_token", "apiKey"] {
        if let Some(val) = json[key].as_str() {
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

// ── Token Manager ──────────────────────────────────────────

#[derive(Debug)]
pub struct ClaudeCliCredential {
    pub token: String,
    pub source_path: PathBuf,
}

pub struct ClaudeCliTokenManager {
    credential: Mutex<Option<ClaudeCliCredential>>,
    auth_file_paths: Vec<PathBuf>,
}

impl ClaudeCliTokenManager {
    pub fn new(auth_file_paths: Vec<PathBuf>) -> Self {
        Self {
            credential: Mutex::new(None),
            auth_file_paths,
        }
    }

    pub async fn get_token(&self) -> Result<String> {
        // Check cached
        {
            let cached = self.credential.lock().await;
            if let Some(ref cred) = *cached {
                return Ok(cred.token.clone());
            }
        }

        // Scan files
        for path in &self.auth_file_paths {
            if let Ok(content) = tokio::fs::read_to_string(path).await {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(token) = extract_claude_token(&json) {
                        debug!("Claude CLI token found at: {}", path.display());
                        let cred = ClaudeCliCredential {
                            token: token.clone(),
                            source_path: path.clone(),
                        };
                        *self.credential.lock().await = Some(cred);
                        return Ok(token);
                    }
                }
            }
        }

        Err(anyhow!("No Claude CLI token found"))
    }

    pub async fn invalidate(&self) {
        *self.credential.lock().await = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_session_key_format() {
        let json = r#"{ "sessionKey": "sk-ant-session-test123" }"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let token = extract_claude_token(&parsed);
        assert_eq!(token, Some("sk-ant-session-test123".to_string()));
    }

    #[test]
    fn parse_token_format() {
        let json = r#"{ "token": "clt_test456" }"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let token = extract_claude_token(&parsed);
        assert_eq!(token, Some("clt_test456".to_string()));
    }

    #[test]
    fn parse_access_token_format() {
        let json = r#"{ "access_token": "acc_test789" }"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let token = extract_claude_token(&parsed);
        assert_eq!(token, Some("acc_test789".to_string()));
    }

    #[test]
    fn empty_token_returns_none() {
        let json = r#"{ "sessionKey": "" }"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let token = extract_claude_token(&parsed);
        assert!(token.is_none());
    }

    #[test]
    fn no_known_fields_returns_none() {
        let json = r#"{ "unknown_field": "value" }"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let token = extract_claude_token(&parsed);
        assert!(token.is_none());
    }
}
