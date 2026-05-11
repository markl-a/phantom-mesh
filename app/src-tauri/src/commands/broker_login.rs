// Broker login — phantom-mesh iOS / Tauri equivalent of `phantom login`.
//
// Desktop CLI's login_broker() at core/src/bin/phantom.rs:5540 starts a
// localhost HTTP server on :48181 to catch the OAuth callback, but iOS
// sandbox blocks loopback. Instead the iOS app:
//
//   1. broker_login_start(broker_url) generates a device_id + a
//      `phantom://oauth/callback` redirect URI, returns the Safari URL
//      to navigate to (`<broker>/auth/cli/start?...`).
//   2. JS layer opens it via tauri-plugin-shell::open() — that hands off
//      to Mobile Safari / system browser.
//   3. User completes Google / Apple / email login on phantommesh.io.
//      Broker meta-refreshes browser to phantom://oauth/callback?p=<b64>.
//   4. iOS routes that URL to the app via tauri-plugin-deep-link's
//      onOpenUrl handler (registered in lib.rs setup()), which emits a
//      `deep-link://oauth-callback` event.
//   5. JS layer's listener extracts the `p=<b64>` query, calls
//      broker_login_finish(b64) which decodes UTF-8 base64 → identity
//      JSON → AuthState → phantom_mesh::auth::save().
//
// Server side accepts `phantom://oauth/callback` thanks to PR #15
// (REDIRECT_RE extension on phantommesh-io/src/routes/oauth.ts:15).

use base64::Engine;
use phantom_mesh::auth;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct BrokerLoginStartResponse {
    /// URL the front-end should open in the system browser. The user
    /// completes OAuth there; the broker meta-refreshes back to
    /// phantom://oauth/callback when done.
    pub auth_url: String,
    /// Persisted in case the front-end wants to display "linking
    /// device <X>…" or for diagnostics.
    pub device_id: String,
    /// The redirect URI we registered with the broker (`phantom://...`)
    /// — informational; the broker validates it against REDIRECT_RE.
    pub redirect: String,
}

#[tauri::command]
pub fn broker_login_start(broker_url: String) -> Result<BrokerLoginStartResponse, String> {
    let broker_url = broker_url
        .trim()
        .trim_end_matches('/')
        .to_string();
    if broker_url.is_empty() {
        return Err("broker_url must not be empty".into());
    }

    // Reuse an existing device_id if one is already saved (so re-logging
    // in on the same device doesn't fragment the broker's device list).
    let device_id = auth::load()
        .map(|s| s.device_id)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(auth::random_device_id);

    let redirect = "phantom://oauth/callback".to_string();
    let auth_url = format!(
        "{}/auth/cli/start?device_id={}&port=0&redirect={}",
        broker_url,
        urlencoding::encode(&device_id),
        urlencoding::encode(&redirect),
    );

    Ok(BrokerLoginStartResponse {
        auth_url,
        device_id,
        redirect,
    })
}

/// Decode the broker's `?p=<base64-payload>` query, build an AuthState,
/// persist via phantom_mesh::auth::save(). Front-end should call this
/// after extracting the `p` value from the `phantom://oauth/callback`
/// URL the deep-link handler emitted.
///
/// Format of the decoded JSON is the CliPayload defined on
/// phantommesh-io/src/types.ts:CliPayload.
#[derive(Deserialize)]
struct BrokerPayload {
    provider: String,
    email: String,
    sub: Option<String>,
    name: Option<String>,
    picture: Option<String>,
    #[serde(default)]
    broker_token: String,
    #[serde(default)]
    broker_token_expires_at_ms: i64,
}

#[derive(Serialize)]
pub struct BrokerLoginFinishResponse {
    pub email: String,
    pub provider: String,
    pub display_name: Option<String>,
    pub broker_token_expires_at_ms: i64,
    pub auth_path: String,
}

#[tauri::command]
pub fn broker_login_finish(payload_b64: String) -> Result<BrokerLoginFinishResponse, String> {
    // base64url → base64 standard with padding
    let std_b64 = payload_b64.replace('-', "+").replace('_', "/");
    let pad = (4 - std_b64.len() % 4) % 4;
    let padded = format!("{}{}", std_b64, "=".repeat(pad));

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&padded)
        .map_err(|e| format!("base64 decode failed: {e}"))?;

    let json = String::from_utf8(bytes).map_err(|e| format!("payload not valid UTF-8: {e}"))?;
    let payload: BrokerPayload =
        serde_json::from_str(&json).map_err(|e| format!("payload not valid JSON: {e}"))?;

    if payload.email.is_empty() {
        return Err("broker payload had no email — refusing to save".into());
    }

    let now = auth::now_ms();
    let prior = auth::load();
    let device_id = prior
        .as_ref()
        .map(|s| s.device_id.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(auth::random_device_id);

    let state = auth::AuthState {
        provider: payload.provider.clone(),
        email: payload.email.clone(),
        display_name: payload.name.clone(),
        sub: payload.sub.clone(),
        avatar_url: payload.picture.clone(),
        device_id,
        created_at_ms: prior.as_ref().map(|s| s.created_at_ms).unwrap_or(now),
        last_login_ms: now,
        password_hash: String::new(),
        salt: String::new(),
        id_token: String::new(),
        access_token: String::new(),
        broker_token: payload.broker_token.clone(),
        broker_token_expires_at_ms: payload.broker_token_expires_at_ms,
    };

    auth::save(&state).map_err(|e| format!("auth::save failed: {e}"))?;

    Ok(BrokerLoginFinishResponse {
        email: payload.email,
        provider: payload.provider,
        display_name: payload.name,
        broker_token_expires_at_ms: payload.broker_token_expires_at_ms,
        auth_path: auth::auth_path().display().to_string(),
    })
}

/// Diagnostic — return whether we have a saved AuthState and a brief
/// human summary. Used by the front-end to decide whether to show
/// "Sign in" or "Logged in as X" in the UI.
#[tauri::command]
pub fn broker_login_status() -> Option<BrokerLoginFinishResponse> {
    let s = auth::load()?;
    Some(BrokerLoginFinishResponse {
        email: s.email,
        provider: s.provider,
        display_name: s.display_name,
        broker_token_expires_at_ms: s.broker_token_expires_at_ms,
        auth_path: auth::auth_path().display().to_string(),
    })
}

/// Wipe local broker auth — useful when broker_token is rotated server
/// side or when user wants to switch accounts.
#[tauri::command]
pub fn broker_login_logout() -> Result<(), String> {
    auth::delete().map_err(|e| format!("auth::delete failed: {e}"))?;
    Ok(())
}

// ── Post-login: pull LLM keys + cluster peers from the broker vault ──────
//
// Mirrors the desktop CLI's `phantom config pull` step (which lives in
// core/src/cli_config.rs::config_pull_lines on platform/macos but isn't
// in iOS's branch yet — inlined here so the iOS app has feature parity
// without needing a deeper merge).

#[derive(Serialize, Deserialize, Clone)]
pub struct ClusterPeer {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Serialize)]
pub struct BrokerSyncResponse {
    pub keys_written: Vec<String>,
    pub env_path: String,
    pub peers_count: usize,
    pub peers_path: Option<String>,
    /// Full peer list — front-end uses this to show a coordinator picker
    /// after sync, so the user can pick which peer the WebView should
    /// load `<coord>/m` from.
    #[serde(default)]
    pub peers: Vec<ClusterPeer>,
}

fn phantom_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".phantom-mesh")
}

fn env_file_path() -> std::path::PathBuf {
    phantom_dir().join("env")
}

fn peers_json_path() -> std::path::PathBuf {
    phantom_dir().join("peers.json")
}

fn read_env_file(path: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return out;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

fn write_env_file(
    path: &std::path::Path,
    env: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {parent:?}: {e}"))?;
    }
    let mut buf = String::new();
    for (k, v) in env {
        buf.push_str(k);
        buf.push('=');
        buf.push_str(v);
        buf.push('\n');
    }
    std::fs::write(path, buf).map_err(|e| format!("write {path:?}: {e}"))?;
    Ok(())
}

/// GET /api/me/settings/raw + GET /api/me/cluster-peers using the
/// broker_token from saved AuthState; merge keys into ~/.phantom-mesh/env
/// (broker wins for keys it provides; locals untouched for keys it
/// doesn't), and write peers.json. Best-effort: peers fetch failure
/// doesn't break the keys sync.
#[tauri::command]
pub async fn broker_sync_from_vault(
    broker_url: Option<String>,
) -> Result<BrokerSyncResponse, String> {
    let broker_url = broker_url
        .unwrap_or_else(|| "https://phantommesh.io".to_string())
        .trim_end_matches('/')
        .to_string();
    let state = auth::load().ok_or("not logged in — run broker_login_start first")?;
    let token = state.broker_token.clone();
    if token.is_empty() {
        return Err("AuthState has no broker_token (login was skipped or older format)".into());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("client build: {e}"))?;

    // ── pull settings/raw ──
    let settings_url = format!("{broker_url}/api/me/settings/raw");
    let resp = client
        .get(&settings_url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("GET {settings_url}: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "broker {} returned HTTP {} — {}",
            settings_url,
            status.as_u16(),
            body.chars().take(200).collect::<String>()
        ));
    }
    let parsed: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("non-JSON response: {e} (body head: {})",
            body.chars().take(120).collect::<String>()))?;
    let env_obj = parsed
        .get("env")
        .and_then(|v| v.as_object())
        .ok_or("broker response missing `env` object")?;

    let env_path = env_file_path();
    let mut existing = read_env_file(&env_path);
    let mut keys_written: Vec<String> = Vec::new();
    for (k, v) in env_obj {
        if let Some(val) = v.as_str() {
            if val.is_empty() {
                continue;
            }
            existing.insert(k.clone(), val.to_string());
            // Push into the running process's env immediately — the agent
            // runtime reads provider keys via std::env::var() at every
            // chat request, so this makes the freshly-pulled keys usable
            // without an app restart. (Startup also re-loads the file in
            // lib.rs setup() for cold-launch case.)
            std::env::set_var(k, val);
            keys_written.push(k.clone());
        }
    }
    write_env_file(&env_path, &existing)?;
    keys_written.sort();

    // Seed agents.toml on first sync — same idempotency guard as the
    // lib.rs setup() startup hook, but here we catch the case where the
    // user did broker login at runtime (e.g. via the deep-link transfer
    // helper) and the app is already running, so they can chat without
    // restart.
    if let Err(e) = crate::commands::local_keys::seed_default_agents_toml_if_missing() {
        tracing::warn!("post-sync agents.toml seed failed: {e}");
    }

    // ── pull cluster peers (best-effort) ──
    let peers_url = format!("{broker_url}/api/me/cluster-peers");
    let mut peers: Vec<ClusterPeer> = Vec::new();
    let mut peers_path: Option<String> = None;
    if let Ok(resp) = client
        .get(&peers_url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(body) = resp.text().await {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                    if let Some(arr) = json.get("peers").and_then(|v| v.as_array()) {
                        peers = arr
                            .iter()
                            .filter_map(|p| {
                                let name = p.get("name")?.as_str()?.to_string();
                                let url = p.get("url")?.as_str()?.to_string();
                                let label = p
                                    .get("label")
                                    .and_then(|v| v.as_str())
                                    .map(String::from);
                                if name.is_empty() || url.is_empty() {
                                    None
                                } else {
                                    Some(ClusterPeer { name, url, label })
                                }
                            })
                            .collect();
                        let path = peers_json_path();
                        if let Some(parent) = path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if std::fs::write(
                            &path,
                            serde_json::to_string_pretty(&peers).unwrap_or_default(),
                        )
                        .is_ok()
                        {
                            peers_path = Some(path.display().to_string());
                        }
                    }
                }
            }
        }
    }

    Ok(BrokerSyncResponse {
        keys_written,
        env_path: env_path.display().to_string(),
        peers_count: peers.len(),
        peers_path,
        peers,
    })
}

/// Diagnostic / picker UI helper — returns the cluster peer list cached
/// in ~/.phantom-mesh/peers.json. Empty Vec when the file is missing or
/// unparseable. Front-end calls this on app boot to decide whether to
/// show "Pick a coordinator" before triggering thin-shell redirect.
#[tauri::command]
pub fn broker_list_cached_peers() -> Vec<ClusterPeer> {
    let path = peers_json_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<ClusterPeer>>(&text).unwrap_or_default()
}

/// POST /api/me/cluster-peers/upsert — register THIS device on the
/// user's cluster registry. Used by the iOS app after broker_login so
/// other peers can discover it. Best-effort; failure surfaces to UI.
#[tauri::command]
pub async fn broker_register_self_peer(
    name: String,
    url: String,
    label: Option<String>,
    broker_url: Option<String>,
) -> Result<usize, String> {
    let broker_url = broker_url
        .unwrap_or_else(|| "https://phantommesh.io".to_string())
        .trim_end_matches('/')
        .to_string();
    let state = auth::load().ok_or("not logged in")?;
    if state.broker_token.is_empty() {
        return Err("no broker_token saved".into());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("client: {e}"))?;
    let body = serde_json::json!({
        "name": name,
        "url": url,
        "label": label.unwrap_or_default(),
    });
    let resp = client
        .post(format!("{broker_url}/api/me/cluster-peers/upsert"))
        .header("Authorization", format!("Bearer {}", state.broker_token))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("POST upsert: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("HTTP {}: {body}", status.as_u16()));
    }
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    let count = parsed
        .get("peers")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    Ok(count)
}
