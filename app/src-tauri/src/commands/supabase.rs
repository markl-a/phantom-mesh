use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::State;

// ── Supabase config ────────────────────────────────────────────────────────
const SUPABASE_URL: &str = "https://tqvrykaomrnyssuypnlq.supabase.co";
const SUPABASE_ANON_KEY: &str = "sb_publishable_m35VA2NnAFYHQvRdl5-83Q_kHDQFG6S";

// ── Types ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupabaseSession {
    pub access_token: String,
    pub refresh_token: String,
    pub user_id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub expires_at: u64,
}

/// Managed state: holds the current Supabase session (if logged in).
pub struct SupabaseState {
    pub session: Mutex<Option<SupabaseSession>>,
}

impl Default for SupabaseState {
    fn default() -> Self {
        Self {
            session: Mutex::new(None),
        }
    }
}

// ── Internal helpers ───────────────────────────────────────────────────────

fn api_headers(access_token: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "apikey",
        HeaderValue::from_str(SUPABASE_ANON_KEY).unwrap(),
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(token) = access_token {
        if let Ok(val) = HeaderValue::from_str(&format!("Bearer {}", token)) {
            headers.insert(AUTHORIZATION, val);
        }
    }
    headers
}

fn rest_url(table: &str) -> String {
    format!("{}/rest/v1/{}", SUPABASE_URL, table)
}

fn auth_url(path: &str) -> String {
    format!("{}/auth/v1/{}", SUPABASE_URL, path)
}

// ── Commands ───────────────────────────────────────────────────────────────

/// Exchange an OAuth id_token with Supabase Auth to get a session.
/// Called after Google/Apple OAuth succeeds locally.
#[tauri::command]
pub async fn supabase_sign_in(
    provider: String,
    id_token: String,
    state: State<'_, SupabaseState>,
) -> Result<SupabaseSession, String> {
    if SUPABASE_URL.is_empty() || SUPABASE_ANON_KEY.is_empty() {
        return Err("Supabase not configured yet".into());
    }

    let client = reqwest::Client::new();

    // Supabase Auth: sign in with ID token
    // https://supabase.com/docs/reference/javascript/auth-signinwithidtoken
    let body = serde_json::json!({
        "provider": provider,  // "google" or "apple"
        "token": id_token,
    });

    let resp = client
        .post(auth_url("token?grant_type=id_token"))
        .headers(api_headers(None))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Supabase auth request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Supabase auth error {}: {}", status, text));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse auth response: {}", e))?;

    let session = SupabaseSession {
        access_token: json["access_token"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        refresh_token: json["refresh_token"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        user_id: json["user"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        email: json["user"]["email"].as_str().map(|s| s.to_string()),
        display_name: json["user"]["user_metadata"]["full_name"]
            .as_str()
            .or(json["user"]["user_metadata"]["name"].as_str())
            .map(|s| s.to_string()),
        expires_at: json["expires_at"].as_u64().unwrap_or(0),
    };

    // Store session in managed state
    {
        let mut guard = state.session.lock().map_err(|e| e.to_string())?;
        *guard = Some(session.clone());
    }

    Ok(session)
}

/// Get the current Supabase session (if any).
#[tauri::command]
pub fn supabase_get_session(
    state: State<'_, SupabaseState>,
) -> Result<Option<SupabaseSession>, String> {
    let guard = state.session.lock().map_err(|e| e.to_string())?;
    Ok(guard.clone())
}

/// Log API usage to Supabase.
#[tauri::command]
pub async fn supabase_log_usage(
    provider: String,
    model: String,
    tokens_in: i32,
    tokens_out: i32,
    cost_usd: f64,
    state: State<'_, SupabaseState>,
) -> Result<(), String> {
    let (access_token, user_id) = {
        let guard = state.session.lock().map_err(|e| e.to_string())?;
        let session = guard.as_ref().ok_or("Not signed in to Supabase")?;
        (session.access_token.clone(), session.user_id.clone())
    };

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "user_id": user_id,
        "provider": provider,
        "model": model,
        "tokens_in": tokens_in,
        "tokens_out": tokens_out,
        "cost_usd": cost_usd,
    });

    let resp = client
        .post(rest_url("usage_logs"))
        .headers(api_headers(Some(&access_token)))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Failed to log usage: {}", e))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Usage log error: {}", text));
    }

    Ok(())
}

/// Backup user config (agents.toml as JSON) to Supabase.
#[tauri::command]
pub async fn supabase_backup_config(
    config_json: String,
    state: State<'_, SupabaseState>,
) -> Result<(), String> {
    let (access_token, user_id) = {
        let guard = state.session.lock().map_err(|e| e.to_string())?;
        let session = guard.as_ref().ok_or("Not signed in to Supabase")?;
        (session.access_token.clone(), session.user_id.clone())
    };

    let config_value: serde_json::Value =
        serde_json::from_str(&config_json).map_err(|e| format!("Invalid config JSON: {}", e))?;

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "user_id": user_id,
        "config": config_value,
        "updated_at": chrono::Utc::now().to_rfc3339(),
    });

    // Upsert (insert or update on conflict)
    let resp = client
        .post(rest_url("user_configs"))
        .headers(api_headers(Some(&access_token)))
        .header("Prefer", "resolution=merge-duplicates")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Failed to backup config: {}", e))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Config backup error: {}", text));
    }

    Ok(())
}

/// Restore user config from Supabase.
#[tauri::command]
pub async fn supabase_restore_config(
    state: State<'_, SupabaseState>,
) -> Result<String, String> {
    let (access_token, user_id) = {
        let guard = state.session.lock().map_err(|e| e.to_string())?;
        let session = guard.as_ref().ok_or("Not signed in to Supabase")?;
        (session.access_token.clone(), session.user_id.clone())
    };

    let client = reqwest::Client::new();
    let url = format!(
        "{}?user_id=eq.{}&select=config",
        rest_url("user_configs"),
        user_id
    );

    let resp = client
        .get(&url)
        .headers(api_headers(Some(&access_token)))
        .send()
        .await
        .map_err(|e| format!("Failed to restore config: {}", e))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Config restore error: {}", text));
    }

    let rows: Vec<serde_json::Value> = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse config response: {}", e))?;

    if let Some(row) = rows.first() {
        Ok(row["config"].to_string())
    } else {
        Err("No config backup found".into())
    }
}

/// Sign out — clear local session.
#[tauri::command]
pub fn supabase_sign_out(state: State<'_, SupabaseState>) -> Result<(), String> {
    let mut guard = state.session.lock().map_err(|e| e.to_string())?;
    *guard = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rest_url() {
        let url = rest_url("usage_logs");
        assert!(url.ends_with("/rest/v1/usage_logs"));
        assert!(url.starts_with("https://"));
    }

    #[test]
    fn test_auth_url() {
        let url = auth_url("token?grant_type=id_token");
        assert!(url.ends_with("/auth/v1/token?grant_type=id_token"));
        assert!(url.starts_with("https://"));
    }

    #[test]
    fn test_api_headers_without_token() {
        let headers = api_headers(None);
        assert!(headers.contains_key("apikey"));
        assert!(headers.contains_key(CONTENT_TYPE));
        assert!(!headers.contains_key(AUTHORIZATION));
    }

    #[test]
    fn test_api_headers_with_token() {
        let headers = api_headers(Some("test_token_123"));
        assert!(headers.contains_key(AUTHORIZATION));
        let auth = headers.get(AUTHORIZATION).unwrap().to_str().unwrap();
        assert_eq!(auth, "Bearer test_token_123");
    }

    #[test]
    fn test_supabase_state_default() {
        let state = SupabaseState::default();
        let guard = state.session.lock().unwrap();
        assert!(guard.is_none());
    }

    #[test]
    fn test_supabase_session_serialization() {
        let session = SupabaseSession {
            access_token: "at".to_string(),
            refresh_token: "rt".to_string(),
            user_id: "uid".to_string(),
            email: Some("test@example.com".to_string()),
            display_name: Some("Test User".to_string()),
            expires_at: 1234567890,
        };
        let json = serde_json::to_string(&session).unwrap();
        let deserialized: SupabaseSession = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.user_id, "uid");
        assert_eq!(deserialized.email.unwrap(), "test@example.com");
    }
}
