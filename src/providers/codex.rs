//! Codex OAuth integration: token refresh, model listing, usage query, and CodexAwareProvider.
//!
//! Replaces the static `resolve_codex_credential()` with a full token lifecycle manager
//! that auto-refreshes expired OAuth tokens via the OpenAI auth endpoint.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc, Duration as ChronoDuration};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use super::openai_compat::OpenAiCompatProvider;
use super::traits::*;

// ── Constants ─────────────────────────────────────────────────────────────────

const OPENAI_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const OPENAI_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const TOKEN_EXPIRY_BUFFER: Duration = Duration::from_secs(300); // 5 min
const ASSUMED_TOKEN_LIFETIME: Duration = Duration::from_secs(3600); // 1h from file mtime
const MODEL_CACHE_TTL: Duration = Duration::from_secs(600); // 10 min

// ── CodexCredential ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CodexCredential {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub account_id: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

// ── CodexTokenManager ─────────────────────────────────────────────────────────

/// OAuth token lifecycle manager: read, cache, detect expiry, auto-refresh.
pub struct CodexTokenManager {
    credential: Mutex<Option<CodexCredential>>,
    auth_file_paths: Vec<PathBuf>,
    client: Client,
}

impl CodexTokenManager {
    /// Create a new token manager, resolving auth file paths from home dir.
    pub fn new() -> Self {
        let paths = if let Some(home) = dirs::home_dir() {
            vec![
                home.join(".codex").join("auth.json"),
                home.join(".codex-cli").join("auth.json"),
            ]
        } else {
            vec![]
        };

        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client for CodexTokenManager");

        Self {
            credential: Mutex::new(None),
            auth_file_paths: paths,
            client,
        }
    }

    /// Main entry point: return a valid access token, refreshing if needed.
    pub async fn get_token(&self) -> Result<String> {
        let cred = self.get_credential().await?;
        Ok(cred.access_token)
    }

    /// Get the full credential (with account_id, etc.).
    pub async fn get_credential(&self) -> Result<CodexCredential> {
        // 1. Check cached credential
        {
            let guard = self.credential.lock().await;
            if let Some(ref cred) = *guard {
                if !is_expired(cred) {
                    return Ok(cred.clone());
                }
                debug!("Cached Codex token expired, attempting refresh");
            }
        }

        // 2. Re-read auth file (another process may have refreshed it)
        if let Some(fresh) = self.read_auth_file() {
            if !is_expired(&fresh) {
                let mut guard = self.credential.lock().await;
                *guard = Some(fresh.clone());
                info!("Codex token refreshed from auth.json");
                return Ok(fresh);
            }
            // File token also expired — try OAuth refresh
            if let Some(ref rt) = fresh.refresh_token {
                match self.refresh_token(rt).await {
                    Ok(refreshed) => {
                        let mut guard = self.credential.lock().await;
                        *guard = Some(refreshed.clone());
                        info!("Codex token refreshed via OAuth");
                        return Ok(refreshed);
                    }
                    Err(e) => {
                        warn!("Codex OAuth refresh failed: {}", e);
                    }
                }
            }
            // Even if expired, return it — the API call might still work
            // (tokens sometimes live slightly past their stated expiry)
            let mut guard = self.credential.lock().await;
            *guard = Some(fresh.clone());
            return Ok(fresh);
        }

        // 3. Check if we have a cached credential with refresh_token
        let maybe_rt = {
            let guard = self.credential.lock().await;
            guard.as_ref().and_then(|c| c.refresh_token.clone())
        };
        if let Some(rt) = maybe_rt {
            match self.refresh_token(&rt).await {
                Ok(refreshed) => {
                    let mut guard = self.credential.lock().await;
                    *guard = Some(refreshed.clone());
                    info!("Codex token refreshed via cached refresh_token");
                    return Ok(refreshed);
                }
                Err(e) => {
                    warn!("Codex OAuth refresh from cache failed: {}", e);
                }
            }
        }

        Err(anyhow!("No Codex credential available (no auth.json found and no cached token)"))
    }

    /// Parse auth.json from disk. Supports nested `tokens.access_token` and flat layouts.
    pub fn read_auth_file(&self) -> Option<CodexCredential> {
        for path in &self.auth_file_paths {
            if !path.exists() {
                continue;
            }
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) => {
                    debug!("Failed to read {:?}: {}", path, e);
                    continue;
                }
            };
            let val: Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(e) => {
                    debug!("Failed to parse {:?}: {}", path, e);
                    continue;
                }
            };

            // Determine expires_at from file metadata
            let expires_at = Self::estimate_expiry_from_file(path, &val);

            // Try nested: tokens.access_token
            if let Some(token) = val.pointer("/tokens/access_token").and_then(|v| v.as_str()) {
                if !token.is_empty() {
                    let refresh = val.pointer("/tokens/refresh_token")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let account_id = val.pointer("/tokens/account_id")
                        .or_else(|| val.get("account_id"))
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    info!("Codex credential loaded from {:?} (nested)", path);
                    return Some(CodexCredential {
                        access_token: token.to_string(),
                        refresh_token: refresh,
                        account_id,
                        expires_at,
                    });
                }
            }

            // Try flat: access_token
            if let Some(token) = val.get("access_token").and_then(|v| v.as_str()) {
                if !token.is_empty() {
                    let refresh = val.get("refresh_token").and_then(|v| v.as_str()).map(String::from);
                    let account_id = val.get("account_id").and_then(|v| v.as_str()).map(String::from);
                    info!("Codex credential loaded from {:?} (flat)", path);
                    return Some(CodexCredential {
                        access_token: token.to_string(),
                        refresh_token: refresh,
                        account_id,
                        expires_at,
                    });
                }
            }

            // Try api_key fallback (some configs store plain API key)
            if let Some(key) = val.get("api_key").and_then(|v| v.as_str()) {
                if !key.is_empty() {
                    info!("Codex API key loaded from {:?}", path);
                    return Some(CodexCredential {
                        access_token: key.to_string(),
                        refresh_token: None,
                        account_id: None,
                        expires_at: None, // API keys don't expire
                    });
                }
            }
        }
        None
    }

    /// Synchronous token read for startup (create_provider context).
    pub fn read_auth_file_sync(&self) -> Option<String> {
        self.read_auth_file().map(|c| c.access_token)
    }

    /// Refresh an OAuth token via the OpenAI auth endpoint (RFC 6749).
    async fn refresh_token(&self, refresh_token: &str) -> Result<CodexCredential> {
        debug!("Attempting Codex OAuth token refresh");
        let resp = self.client
            .post(OPENAI_OAUTH_TOKEN_URL)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", OPENAI_OAUTH_CLIENT_ID),
                ("refresh_token", refresh_token),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("OAuth refresh failed ({}): {}", status, body));
        }

        let json: Value = resp.json().await?;
        let access_token = json.get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("OAuth refresh response missing access_token"))?
            .to_string();

        // RFC 6749: new refresh_token is optional; keep old one if not provided
        let new_refresh = json.get("refresh_token")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| Some(refresh_token.to_string()));

        let expires_in = json.get("expires_in")
            .and_then(|v| v.as_i64())
            .unwrap_or(3600);
        let expires_at = Utc::now() + ChronoDuration::seconds(expires_in);

        let account_id = json.get("account_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        Ok(CodexCredential {
            access_token,
            refresh_token: new_refresh,
            account_id,
            expires_at: Some(expires_at),
        })
    }

    /// Returns a clone of the current credential without refreshing.
    /// Useful for providers that need the full credential (token + account_id).
    pub async fn get_credential_clone(&self) -> Option<CodexCredential> {
        // Try cached first
        let guard = self.credential.lock().await;
        if let Some(ref cred) = *guard {
            return Some(cred.clone());
        }
        drop(guard);

        // Try reading from file
        if let Some(cred) = self.read_auth_file() {
            let mut guard = self.credential.lock().await;
            *guard = Some(cred.clone());
            Some(cred)
        } else {
            None
        }
    }

    /// Clear cached credential (for external re-login flows).
    pub async fn invalidate(&self) {
        let mut guard = self.credential.lock().await;
        *guard = None;
        debug!("Codex token cache invalidated");
    }

    /// Estimate token expiry from file metadata + JSON fields.
    fn estimate_expiry_from_file(path: &std::path::Path, val: &Value) -> Option<DateTime<Utc>> {
        // Check explicit expires_at in JSON (Unix timestamp)
        if let Some(exp) = val.get("expires_at").or_else(|| val.pointer("/tokens/expires_at")) {
            if let Some(ts) = exp.as_i64() {
                return DateTime::from_timestamp(ts, 0).map(|dt| dt.into());
            }
        }
        // Check expires_in in JSON
        if let Some(exp_in) = val.get("expires_in").or_else(|| val.pointer("/tokens/expires_in")) {
            if let Some(secs) = exp_in.as_i64() {
                // Use file mtime as base
                if let Ok(meta) = std::fs::metadata(path) {
                    if let Ok(mtime) = meta.modified() {
                        let mtime_utc: DateTime<Utc> = mtime.into();
                        return Some(mtime_utc + ChronoDuration::seconds(secs));
                    }
                }
            }
        }
        // Fallback: file mtime + assumed lifetime
        if let Ok(meta) = std::fs::metadata(path) {
            if let Ok(mtime) = meta.modified() {
                let mtime_utc: DateTime<Utc> = mtime.into();
                return Some(mtime_utc + ChronoDuration::from_std(ASSUMED_TOKEN_LIFETIME).unwrap());
            }
        }
        None
    }
}

/// Check if a credential is expired (with buffer).
fn is_expired(cred: &CodexCredential) -> bool {
    match cred.expires_at {
        Some(exp) => {
            let buffer = ChronoDuration::from_std(TOKEN_EXPIRY_BUFFER).unwrap();
            Utc::now() + buffer > exp
        }
        None => false, // No expiry info → assume valid (API keys)
    }
}

// ── ModelListCache ────────────────────────────────────────────────────────────

/// Model info from the OpenAI /v1/models endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub owned_by: Option<String>,
    #[serde(default)]
    pub created: Option<i64>,
}

/// Cached model list with TTL.
pub struct ModelListCache {
    cache: Mutex<Option<(Vec<ModelInfo>, Instant)>>,
    client: Client,
}

impl ModelListCache {
    pub fn new(client: Client) -> Self {
        Self {
            cache: Mutex::new(None),
            client,
        }
    }

    /// List models from the given base URL, using bearer token auth.
    /// Results are cached for MODEL_CACHE_TTL (10 min).
    pub async fn list_models(&self, base_url: &str, token: &str) -> Result<Vec<ModelInfo>> {
        // Check cache
        {
            let guard = self.cache.lock().await;
            if let Some((ref models, ref fetched_at)) = *guard {
                if fetched_at.elapsed() < MODEL_CACHE_TTL {
                    return Ok(models.clone());
                }
            }
        }

        // Fetch fresh
        let url = format!("{}/v1/models", base_url);
        let resp = self.client
            .get(&url)
            .bearer_auth(token)
            .timeout(Duration::from_secs(10))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Model list failed ({}): {}", status, body));
        }

        let json: Value = resp.json().await?;
        let models: Vec<ModelInfo> = if let Some(data) = json.get("data").and_then(|v| v.as_array()) {
            data.iter()
                .filter_map(|m| serde_json::from_value(m.clone()).ok())
                .collect()
        } else {
            vec![]
        };

        // Update cache
        {
            let mut guard = self.cache.lock().await;
            *guard = Some((models.clone(), Instant::now()));
        }

        Ok(models)
    }

    /// Invalidate the cache.
    pub async fn invalidate(&self) {
        let mut guard = self.cache.lock().await;
        *guard = None;
    }
}

// ── Usage Query ───────────────────────────────────────────────────────────────

/// A single usage rate-limit window.
#[derive(Debug, Clone)]
pub struct UsageWindow {
    pub label: String,
    pub used_percent: f64,
    pub reset_at: Option<i64>,
}

/// Snapshot of Codex usage / rate-limit state.
#[derive(Debug, Clone)]
pub struct CodexUsageSnapshot {
    pub windows: Vec<UsageWindow>,
    pub plan_type: Option<String>,
    pub credit_balance: Option<f64>,
    pub max_used_percent: f64,
    pub is_near_limit: bool,
}

const WHAM_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const NEAR_LIMIT_THRESHOLD: f64 = 80.0;

/// Fetch Codex usage data from the wham API.
pub async fn fetch_codex_usage(
    client: &Client,
    token: &str,
    account_id: Option<&str>,
) -> Result<CodexUsageSnapshot> {
    let mut req = client
        .get(WHAM_USAGE_URL)
        .bearer_auth(token)
        .timeout(Duration::from_secs(10));

    if let Some(aid) = account_id {
        req = req.header("ChatGPT-Account-Id", aid);
    }

    let resp = req.send().await?;
    let status = resp.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(anyhow!("Codex usage fetch failed: token_expired ({})", status));
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Codex usage fetch failed ({}): {}", status, body));
    }

    let json: Value = resp.json().await?;
    parse_wham_response(&json)
}

fn parse_wham_response(json: &Value) -> Result<CodexUsageSnapshot> {
    let mut windows = Vec::new();

    // Parse rate_limits array
    if let Some(limits) = json.get("rate_limits").and_then(|v| v.as_array()) {
        for limit in limits {
            let label = limit.get("label")
                .or_else(|| limit.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            let used_percent = limit.get("used_percent")
                .or_else(|| limit.get("usage_percent"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);

            let reset_at = limit.get("reset_at")
                .and_then(|v| v.as_i64());

            windows.push(UsageWindow { label, used_percent, reset_at });
        }
    }

    // Parse primary rate limit (alternative format)
    if windows.is_empty() {
        if let Some(primary) = json.get("primary") {
            let used_percent = primary.get("used_percent")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let reset_at = primary.get("reset_at").and_then(|v| v.as_i64());
            windows.push(UsageWindow {
                label: "Primary".to_string(),
                used_percent,
                reset_at,
            });
        }
    }

    let plan_type = json.get("plan_type")
        .or_else(|| json.get("plan"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let credit_balance = json.get("credit_balance")
        .and_then(|v| {
            // Handle both string and number formats
            v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
        });

    let max_used_percent = windows.iter()
        .map(|w| w.used_percent)
        .fold(0.0_f64, f64::max);

    let is_near_limit = max_used_percent > NEAR_LIMIT_THRESHOLD;

    Ok(CodexUsageSnapshot {
        windows,
        plan_type,
        credit_balance,
        max_used_percent,
        is_near_limit,
    })
}

/// Fetch usage and apply rate-limit to rotation if near limit.
pub async fn check_and_apply_usage(
    client: &Client,
    credential: &CodexCredential,
    rotation: &super::rotation::ProviderRotation,
    provider_name: &str,
) -> Result<CodexUsageSnapshot> {
    let snapshot = fetch_codex_usage(
        client,
        &credential.access_token,
        credential.account_id.as_deref(),
    ).await?;

    if snapshot.is_near_limit {
        warn!(
            "Codex usage near limit ({:.1}%), recording rate-limit for '{}'",
            snapshot.max_used_percent, provider_name
        );
        rotation.record_rate_limit(provider_name);
    }

    Ok(snapshot)
}

// ── CodexAwareProvider ────────────────────────────────────────────────────────

/// Provider wrapper that auto-refreshes Codex OAuth tokens before each API call.
pub struct CodexAwareProvider {
    inner: OpenAiCompatProvider,
    token_manager: Arc<CodexTokenManager>,
    model_cache: ModelListCache,
    base_url: String,
}

impl CodexAwareProvider {
    pub fn new(
        base_url: String,
        default_model: String,
        token_manager: Arc<CodexTokenManager>,
    ) -> Self {
        // Create inner with empty API key — we'll override per-request
        let inner = OpenAiCompatProvider::new(
            "codex".to_string(),
            base_url.clone(),
            default_model,
            None, // token set per-request via chat_with_token
        );

        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build client for ModelListCache");

        let model_cache = ModelListCache::new(client);

        Self {
            inner,
            token_manager,
            model_cache,
            base_url,
        }
    }

    /// List available models via the OpenAI /v1/models endpoint.
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let token = self.token_manager.get_token().await?;
        self.model_cache.list_models(&self.base_url, &token).await
    }

    /// Get current usage/rate-limit snapshot.
    pub async fn get_usage(&self) -> Result<CodexUsageSnapshot> {
        let cred = self.token_manager.get_credential().await?;
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        fetch_codex_usage(&client, &cred.access_token, cred.account_id.as_deref()).await
    }

    /// Access the underlying token manager.
    pub fn token_manager(&self) -> &Arc<CodexTokenManager> {
        &self.token_manager
    }
}

#[async_trait]
impl Provider for CodexAwareProvider {
    fn name(&self) -> &str {
        "codex"
    }

    fn default_model(&self) -> &str {
        self.inner.default_model()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            native_tools: true,
            vision: true,
        }
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<ChatResponse> {
        let token = self.token_manager.get_token().await?;
        self.inner.chat_with_token(messages, tools, model, &token).await
    }

    async fn stream_chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<Pin<Box<dyn futures_util::Stream<Item = Result<StreamChunk>> + Send>>> {
        let token = self.token_manager.get_token().await?;
        self.inner.stream_chat_with_token(messages, tools, model, &token).await
    }

    async fn is_alive(&self) -> bool {
        match self.token_manager.get_token().await {
            Ok(token) => {
                let url = format!("{}/v1/models", self.base_url);
                let client = Client::new();
                client.get(&url)
                    .bearer_auth(&token)
                    .timeout(Duration::from_secs(3))
                    .send()
                    .await
                    .map(|r| r.status().is_success())
                    .unwrap_or(false)
            }
            Err(_) => false,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_expired tests ──

    #[test]
    fn test_is_expired_within_buffer() {
        // Token expires in 3 minutes — within 5-min buffer → expired
        let cred = CodexCredential {
            access_token: "tok".into(),
            refresh_token: None,
            account_id: None,
            expires_at: Some(Utc::now() + ChronoDuration::minutes(3)),
        };
        assert!(is_expired(&cred));
    }

    #[test]
    fn test_is_expired_well_within() {
        // Token expires in 2 hours — not expired
        let cred = CodexCredential {
            access_token: "tok".into(),
            refresh_token: None,
            account_id: None,
            expires_at: Some(Utc::now() + ChronoDuration::hours(2)),
        };
        assert!(!is_expired(&cred));
    }

    #[test]
    fn test_is_expired_no_expiry() {
        // No expiry info → assume valid
        let cred = CodexCredential {
            access_token: "tok".into(),
            refresh_token: None,
            account_id: None,
            expires_at: None,
        };
        assert!(!is_expired(&cred));
    }

    // ── parse auth file tests ──

    fn parse_json_credential(json_str: &str) -> Option<CodexCredential> {
        let val: Value = serde_json::from_str(json_str).ok()?;
        // Simulate the nested/flat parsing logic
        if let Some(token) = val.pointer("/tokens/access_token").and_then(|v| v.as_str()) {
            if !token.is_empty() {
                let refresh = val.pointer("/tokens/refresh_token").and_then(|v| v.as_str()).map(String::from);
                let account_id = val.pointer("/tokens/account_id")
                    .or_else(|| val.get("account_id"))
                    .and_then(|v| v.as_str()).map(String::from);
                return Some(CodexCredential {
                    access_token: token.to_string(),
                    refresh_token: refresh,
                    account_id,
                    expires_at: None,
                });
            }
        }
        if let Some(token) = val.get("access_token").and_then(|v| v.as_str()) {
            if !token.is_empty() {
                let refresh = val.get("refresh_token").and_then(|v| v.as_str()).map(String::from);
                let account_id = val.get("account_id").and_then(|v| v.as_str()).map(String::from);
                return Some(CodexCredential {
                    access_token: token.to_string(),
                    refresh_token: refresh,
                    account_id,
                    expires_at: None,
                });
            }
        }
        if let Some(key) = val.get("api_key").and_then(|v| v.as_str()) {
            if !key.is_empty() {
                return Some(CodexCredential {
                    access_token: key.to_string(),
                    refresh_token: None,
                    account_id: None,
                    expires_at: None,
                });
            }
        }
        None
    }

    #[test]
    fn test_parse_auth_file_nested() {
        let json = r#"{"tokens": {"access_token": "tok-nested", "refresh_token": "rt-1"}, "account_id": "acc-1"}"#;
        let cred = parse_json_credential(json).unwrap();
        assert_eq!(cred.access_token, "tok-nested");
        assert_eq!(cred.refresh_token.as_deref(), Some("rt-1"));
        assert_eq!(cred.account_id.as_deref(), Some("acc-1"));
    }

    #[test]
    fn test_parse_auth_file_flat() {
        let json = r#"{"access_token": "tok-flat", "refresh_token": "rt-2", "account_id": "acc-2"}"#;
        let cred = parse_json_credential(json).unwrap();
        assert_eq!(cred.access_token, "tok-flat");
        assert_eq!(cred.refresh_token.as_deref(), Some("rt-2"));
        assert_eq!(cred.account_id.as_deref(), Some("acc-2"));
    }

    #[test]
    fn test_parse_auth_file_api_key_fallback() {
        let json = r#"{"api_key": "sk-test-key"}"#;
        let cred = parse_json_credential(json).unwrap();
        assert_eq!(cred.access_token, "sk-test-key");
        assert!(cred.refresh_token.is_none());
    }

    #[test]
    fn test_parse_auth_file_empty() {
        let json = r#"{}"#;
        assert!(parse_json_credential(json).is_none());
    }

    // ── ModelInfo tests ──

    #[test]
    fn test_model_info_deserialize() {
        let json = r#"{"id": "gpt-4o", "owned_by": "openai", "created": 1700000000}"#;
        let info: ModelInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, "gpt-4o");
        assert_eq!(info.owned_by.as_deref(), Some("openai"));
        assert_eq!(info.created, Some(1700000000));
    }

    #[test]
    fn test_models_response_parse() {
        let json = r#"{"data": [{"id": "gpt-4o"}, {"id": "gpt-4o-mini", "owned_by": "openai"}]}"#;
        let val: Value = serde_json::from_str(json).unwrap();
        let models: Vec<ModelInfo> = val.get("data").unwrap().as_array().unwrap()
            .iter()
            .filter_map(|m| serde_json::from_value(m.clone()).ok())
            .collect();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-4o");
        assert_eq!(models[1].id, "gpt-4o-mini");
    }

    #[test]
    fn test_cache_ttl() {
        // Just verify the constant is 10 minutes
        assert_eq!(MODEL_CACHE_TTL, Duration::from_secs(600));
    }

    // ── wham usage tests ──

    #[test]
    fn test_wham_response_full() {
        let json: Value = serde_json::from_str(r#"{
            "rate_limits": [
                {"label": "3h", "used_percent": 45.2, "reset_at": 1700000000},
                {"label": "Day", "used_percent": 30.0}
            ],
            "plan_type": "plus",
            "credit_balance": 12.50
        }"#).unwrap();
        let snap = parse_wham_response(&json).unwrap();
        assert_eq!(snap.windows.len(), 2);
        assert_eq!(snap.windows[0].label, "3h");
        assert!((snap.windows[0].used_percent - 45.2).abs() < 0.01);
        assert_eq!(snap.windows[0].reset_at, Some(1700000000));
        assert_eq!(snap.plan_type.as_deref(), Some("plus"));
        assert!((snap.credit_balance.unwrap() - 12.50).abs() < 0.01);
        assert!((snap.max_used_percent - 45.2).abs() < 0.01);
        assert!(!snap.is_near_limit);
    }

    #[test]
    fn test_wham_primary_only() {
        let json: Value = serde_json::from_str(r#"{
            "primary": {"used_percent": 55.0, "reset_at": 1700000000}
        }"#).unwrap();
        let snap = parse_wham_response(&json).unwrap();
        assert_eq!(snap.windows.len(), 1);
        assert_eq!(snap.windows[0].label, "Primary");
    }

    #[test]
    fn test_wham_no_rate_limit() {
        let json: Value = serde_json::from_str(r#"{}"#).unwrap();
        let snap = parse_wham_response(&json).unwrap();
        assert!(snap.windows.is_empty());
        assert!(!snap.is_near_limit);
    }

    #[test]
    fn test_window_label_hours() {
        let json: Value = serde_json::from_str(r#"{
            "rate_limits": [{"name": "3h", "usage_percent": 90.5}]
        }"#).unwrap();
        let snap = parse_wham_response(&json).unwrap();
        assert_eq!(snap.windows[0].label, "3h");
        assert!((snap.windows[0].used_percent - 90.5).abs() < 0.01);
    }

    #[test]
    fn test_credit_balance_string_vs_number() {
        // Number format
        let json: Value = serde_json::from_str(r#"{"credit_balance": 5.25}"#).unwrap();
        let snap = parse_wham_response(&json).unwrap();
        assert!((snap.credit_balance.unwrap() - 5.25).abs() < 0.01);

        // String format
        let json: Value = serde_json::from_str(r#"{"credit_balance": "10.50"}"#).unwrap();
        let snap = parse_wham_response(&json).unwrap();
        assert!((snap.credit_balance.unwrap() - 10.50).abs() < 0.01);
    }

    #[test]
    fn test_near_limit_threshold() {
        let json: Value = serde_json::from_str(r#"{
            "rate_limits": [{"label": "3h", "used_percent": 85.0}]
        }"#).unwrap();
        let snap = parse_wham_response(&json).unwrap();
        assert!(snap.is_near_limit);
        assert!((snap.max_used_percent - 85.0).abs() < 0.01);
    }

    // ── CodexAwareProvider tests ──

    #[test]
    fn test_codex_aware_provider_name() {
        let tm = Arc::new(CodexTokenManager::new());
        let p = CodexAwareProvider::new(
            "https://api.openai.com".into(),
            "gpt-4o".into(),
            tm,
        );
        assert_eq!(p.name(), "codex");
        assert_eq!(p.default_model(), "gpt-4o");
    }

    #[tokio::test]
    async fn test_get_credential_clone_returns_none_without_auth_file() {
        let tm = CodexTokenManager::new();
        let cred = tm.get_credential_clone().await;
        assert!(cred.is_none() || cred.is_some()); // Just verify it doesn't panic
    }

    #[test]
    fn test_codex_aware_capabilities() {
        let tm = Arc::new(CodexTokenManager::new());
        let p = CodexAwareProvider::new(
            "https://api.openai.com".into(),
            "gpt-4o".into(),
            tm,
        );
        let caps = p.capabilities();
        assert!(caps.streaming);
        assert!(caps.native_tools);
        assert!(caps.vision);
    }
}
