use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::debug;

use super::openai_compat::OpenAiCompatProvider;

const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const COPILOT_BASE_URL: &str = "https://api.githubcopilot.com";
const TOKEN_EXPIRY_BUFFER_SECS: i64 = 120; // 2 minutes

// ── Token Types ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CopilotApiToken {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

impl CopilotApiToken {
    pub fn is_expired(&self) -> bool {
        Utc::now() + chrono::Duration::seconds(TOKEN_EXPIRY_BUFFER_SECS) >= self.expires_at
    }
}

#[derive(Debug, Deserialize)]
struct TokenExchangeResponse {
    token: String,
    expires_at: i64,
}

// ── Token Manager ──────────────────────────────────────────

pub struct CopilotTokenManager {
    oauth_token: Mutex<Option<String>>,
    api_token: Mutex<Option<CopilotApiToken>>,
    token_file_paths: Vec<PathBuf>,
    client: Client,
}

impl CopilotTokenManager {
    pub fn new(token_file_paths: Vec<PathBuf>) -> Self {
        Self {
            oauth_token: Mutex::new(None),
            api_token: Mutex::new(None),
            token_file_paths,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn with_oauth_token(token: String) -> Self {
        Self {
            oauth_token: Mutex::new(Some(token)),
            api_token: Mutex::new(None),
            token_file_paths: vec![],
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Get a valid API token, refreshing if needed
    pub async fn get_token(&self) -> Result<String> {
        // Check cached API token
        {
            let cached = self.api_token.lock().await;
            if let Some(ref token) = *cached {
                if !token.is_expired() {
                    return Ok(token.token.clone());
                }
            }
        }

        // Need to exchange OAuth token for API token
        let oauth_token = self.get_oauth_token().await?;
        let api_token = self.exchange_token(&oauth_token).await?;

        let result = api_token.token.clone();
        *self.api_token.lock().await = Some(api_token);
        Ok(result)
    }

    async fn get_oauth_token(&self) -> Result<String> {
        // Check cached
        {
            let cached = self.oauth_token.lock().await;
            if let Some(ref token) = *cached {
                return Ok(token.clone());
            }
        }

        // Try reading from files
        for path in &self.token_file_paths {
            if let Ok(content) = tokio::fs::read_to_string(path).await {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(token) = extract_oauth_token_from_hosts(&json) {
                        debug!("Copilot OAuth token found at: {}", path.display());
                        *self.oauth_token.lock().await = Some(token.clone());
                        return Ok(token);
                    }
                }
            }
        }

        Err(anyhow!("No GitHub Copilot OAuth token found"))
    }

    async fn exchange_token(&self, oauth_token: &str) -> Result<CopilotApiToken> {
        // GitHub Copilot token exchange uses GET with token auth
        let resp = self
            .client
            .get(COPILOT_TOKEN_URL)
            .header("Authorization", format!("token {}", oauth_token))
            .header("User-Agent", "phantom-mesh")
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| anyhow!("Copilot token exchange failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Copilot token exchange HTTP {}: {}",
                status,
                body
            ));
        }

        let exchange: TokenExchangeResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse Copilot token response: {}", e))?;

        Ok(CopilotApiToken {
            token: exchange.token,
            expires_at: DateTime::from_timestamp(exchange.expires_at, 0)
                .unwrap_or_else(Utc::now),
        })
    }

    pub async fn invalidate(&self) {
        *self.api_token.lock().await = None;
    }
}

pub fn extract_oauth_token_from_hosts(json: &serde_json::Value) -> Option<String> {
    // hosts.json / apps.json: { "github.com": { "oauth_token": "gho_xxx" } }
    if let Some(obj) = json.as_object() {
        for (_host, val) in obj {
            if let Some(token) = val["oauth_token"].as_str() {
                if !token.is_empty() {
                    return Some(token.to_string());
                }
            }
        }
    }
    None
}

// ── Provider ───────────────────────────────────────────────

pub struct CopilotAwareProvider {
    inner: OpenAiCompatProvider,
    token_manager: Arc<CopilotTokenManager>,
}

impl CopilotAwareProvider {
    pub fn new(token_manager: Arc<CopilotTokenManager>) -> Self {
        Self {
            inner: OpenAiCompatProvider::new(
                "copilot".to_string(),
                COPILOT_BASE_URL.to_string(),
                "gpt-4o".to_string(),
                None, // Token injected per-call
            ),
            token_manager,
        }
    }

    pub async fn chat(
        &self,
        messages: &[crate::providers::ChatMessage],
        tools: &[serde_json::Value],
        model: &str,
    ) -> Result<crate::providers::ChatResponse> {
        let token = self.token_manager.get_token().await?;
        self.inner.chat_with_token(messages, tools, model, &token).await
    }

    pub async fn stream_chat(
        &self,
        messages: &[crate::providers::ChatMessage],
        tools: &[serde_json::Value],
        model: &str,
    ) -> Result<std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<crate::providers::StreamChunk>> + Send>>>
    {
        let token = self.token_manager.get_token().await?;
        self.inner
            .stream_chat_with_token(messages, tools, model, &token)
            .await
    }

    pub fn name(&self) -> &str {
        "copilot"
    }

    pub fn default_model(&self) -> &str {
        "gpt-4o"
    }

    pub async fn is_alive(&self) -> bool {
        self.token_manager.get_token().await.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copilot_api_token_expired() {
        let token = CopilotApiToken {
            token: "test".to_string(),
            expires_at: Utc::now() - chrono::Duration::minutes(5),
        };
        assert!(token.is_expired());
    }

    #[test]
    fn copilot_api_token_not_expired() {
        let token = CopilotApiToken {
            token: "test".to_string(),
            expires_at: Utc::now() + chrono::Duration::minutes(30),
        };
        assert!(!token.is_expired());
    }

    #[test]
    fn copilot_api_token_near_expiry() {
        // Within 2-minute buffer
        let token = CopilotApiToken {
            token: "test".to_string(),
            expires_at: Utc::now() + chrono::Duration::seconds(60),
        };
        assert!(token.is_expired()); // Should be "expired" due to buffer
    }

    #[test]
    fn parse_copilot_hosts_json() {
        let json = r#"{
            "github.com": {
                "oauth_token": "gho_test123",
                "user": "testuser"
            }
        }"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let token = extract_oauth_token_from_hosts(&parsed);
        assert_eq!(token, Some("gho_test123".to_string()));
    }

    #[test]
    fn parse_copilot_apps_json() {
        let json = r#"{
            "github.com": {
                "oauth_token": "ghu_apps_token",
                "user": "testuser"
            }
        }"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let token = extract_oauth_token_from_hosts(&parsed);
        assert_eq!(token, Some("ghu_apps_token".to_string()));
    }

    #[test]
    fn copilot_token_paths_not_empty() {
        let paths = super::super::credential_scanner::copilot_token_paths();
        assert!(!paths.is_empty());
    }
}
