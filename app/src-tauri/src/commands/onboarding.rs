use serde::{Deserialize, Serialize};
use std::net::UdpSocket;
use tauri::Manager;

pub use super::hardware::{GpuInfo, NpuInfo};
use super::hardware;

// ── Response Types ──────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct HardwareScanResult {
    pub gpu: String,
    pub vram_mb: u64,
    pub gpus: Vec<GpuInfo>,
    pub npus: Vec<NpuInfo>,
    pub ram_mb: u64,
    pub ollama_status: String,
    pub ollama_models: Vec<String>,
    pub daemon_binary_path: Option<String>,
    pub available_port: u16,
}

#[derive(Debug, Serialize)]
pub struct OllamaProbeResult {
    pub ok: bool,
    pub models: Vec<String>,
    pub latency_ms: u64,
    pub speed_tier: String,
}

#[derive(Debug, Serialize)]
pub struct ValidationResult {
    pub ok: bool,
    pub models: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DaemonStatus {
    pub ok: bool,
    pub pid: Option<u32>,
    pub port: u16,
}

#[derive(Debug, Serialize)]
pub struct QrPayload {
    #[serde(rename = "type")]
    pub payload_type: String,
    pub version: u32,
    pub hub_url: String,
    pub auth_key: String,
    pub node_id: String,
}

#[derive(Debug, Serialize)]
pub struct CopilotTokenStatus {
    pub found: bool,
    pub user: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GcloudAdcStatus {
    pub found: bool,
    pub project: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ClaudeCliStatus {
    pub found: bool,
}

#[derive(Debug, Serialize)]
pub struct CodexCliStatus {
    pub found: bool,
    pub is_oauth: bool,
}

#[derive(Debug, Deserialize)]
pub struct OnboardingConfig {
    pub port: u16,
    pub discovered_providers: Vec<DiscoveredProviderEntry>,
    pub manual_providers: Vec<ManualProviderEntry>,
    pub ollama_endpoint: Option<String>,
    pub default_agent_provider: String,
    pub default_agent_model: String,
    pub auth_key: String,
    pub telegram_token: Option<String>,
    pub identity_provider: Option<String>,
    pub identity_sub: Option<String>,
    pub identity_email: Option<String>,
    pub is_primary: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct DiscoveredProviderEntry {
    pub name: String,
    pub provider_type: String,
    pub tier: String,
    pub token_source: String,
    pub base_url: Option<String>,
    pub env_key_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ManualProviderEntry {
    pub name: String,
    pub provider_type: String,
    pub api_key: String,
    pub tier: String,
    pub base_url: Option<String>,
    pub endpoint: Option<String>,
    pub region: Option<String>,
}

// ── Commands ────────────────────────────────────────────────

#[tauri::command]
pub async fn scan_hardware() -> Result<HardwareScanResult, String> {
    let hw = hardware::detect_all(7878);

    let primary = hw.gpus.first().cloned().unwrap_or(GpuInfo {
        name: "CPU-only".to_string(),
        dedicated_mb: 0,
        shared_mb: 0,
    });

    let (ollama_status, ollama_models) = probe_ollama_quick().await;
    let daemon_binary_path = find_daemon_binary();

    Ok(HardwareScanResult {
        gpu: primary.name.clone(),
        vram_mb: primary.dedicated_mb,
        gpus: hw.gpus,
        npus: hw.npus,
        ram_mb: hw.ram_mb,
        ollama_status,
        ollama_models,
        daemon_binary_path,
        available_port: hw.available_port,
    })
}

async fn probe_ollama_quick() -> (String, Vec<String>) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    match client.get("http://localhost:11434/api/tags").send().await {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                let models = body["models"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| m["name"].as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                ("online".to_string(), models)
            } else {
                ("online".to_string(), vec![])
            }
        }
        _ => ("offline".to_string(), vec![]),
    }
}

fn find_daemon_binary() -> Option<String> {
    let bin_name = if cfg!(windows) { "spectyn-mesh.exe" } else { "spectyn-mesh" };

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(bin_name);
            if candidate.exists() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }

    // Search exFAT workaround dirs first (most likely to be fresh), then standard
    // ../core/ is the sibling directory (both under LLM-Cluster-Project/)
    for target_dir in ["target2", "target3", "target"] {
        for profile in ["release", "debug"] {
            let candidate = std::path::PathBuf::from(format!(
                "../core/{}/{}/{}",
                target_dir, profile, bin_name
            ));
            if candidate.exists() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }

    which::which("spectyn-mesh")
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn test_ollama(endpoint: String) -> Result<OllamaProbeResult, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let start = std::time::Instant::now();
    let resp = client
        .get(format!("{}/api/tags", endpoint))
        .send()
        .await
        .map_err(|e| format!("Cannot reach Ollama: {}", e))?;

    let latency_ms = start.elapsed().as_millis() as u64;

    if !resp.status().is_success() {
        return Ok(OllamaProbeResult {
            ok: false,
            models: vec![],
            latency_ms,
            speed_tier: "Unknown".to_string(),
        });
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let models = body["models"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let speed_tier = match latency_ms {
        0..=500 => "Fast",
        501..=3000 => "Medium",
        _ => "Slow",
    }
    .to_string();

    Ok(OllamaProbeResult {
        ok: true,
        models,
        latency_ms,
        speed_tier,
    })
}

#[tauri::command]
pub async fn scan_credentials() -> Result<Vec<spectyn_mesh::providers::DiscoveredProviderInfo>, String> {
    let discovered = spectyn_mesh::providers::credential_scanner::scan_all().await;
    Ok(discovered.iter().map(|d| d.to_frontend_info()).collect())
}

#[tauri::command]
pub async fn read_copilot_token() -> Result<CopilotTokenStatus, String> {
    let paths = spectyn_mesh::providers::credential_scanner::copilot_token_paths();
    for path in &paths {
        if let Ok(content) = tokio::fs::read_to_string(path).await {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(obj) = json.as_object() {
                    for (_host, val) in obj {
                        if val["oauth_token"].as_str().is_some() {
                            let user = val["user"].as_str().map(String::from);
                            return Ok(CopilotTokenStatus { found: true, user });
                        }
                    }
                }
            }
        }
    }
    Ok(CopilotTokenStatus { found: false, user: None })
}

#[tauri::command]
pub async fn read_gcloud_adc() -> Result<GcloudAdcStatus, String> {
    let adc_path = dirs::config_dir()
        .map(|c| c.join("gcloud").join("application_default_credentials.json"));
    if let Some(path) = adc_path {
        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                let project = json["quota_project_id"].as_str().map(String::from);
                return Ok(GcloudAdcStatus { found: true, project });
            }
        }
    }
    Ok(GcloudAdcStatus { found: false, project: None })
}

#[tauri::command]
pub async fn read_claude_cli_token() -> Result<ClaudeCliStatus, String> {
    // Unified lookup: credentials files + (on macOS) the Claude Code Keychain
    // item, parsing the modern nested OAuth shape. Runs on a blocking thread
    // because the Keychain read shells out to `security`.
    let found = tokio::task::spawn_blocking(|| {
        spectyn_mesh::providers::claude_cli::find_claude_token().is_some()
    })
    .await
    .unwrap_or(false);
    Ok(ClaudeCliStatus { found })
}

/// Detect a usable ChatGPT (Codex) credential from `~/.codex/auth.json`.
/// `is_oauth` distinguishes a ChatGPT subscription login (→ codex_oauth
/// provider) from a plain `OPENAI_API_KEY` (→ openai provider).
#[tauri::command]
pub async fn read_codex_token() -> Result<CodexCliStatus, String> {
    let auth = spectyn_mesh::providers::codex_cli::find_codex_auth();
    Ok(CodexCliStatus {
        found: auth.is_some(),
        is_oauth: auth.map(|a| a.is_oauth).unwrap_or(false),
    })
}

/// Probe the standard local OpenAI-compatible servers (Ollama / LM Studio /
/// Lemonade) and return the ones currently running, with their models.
#[tauri::command]
pub async fn detect_local_servers(
) -> Result<Vec<spectyn_mesh::providers::local_servers::LocalServer>, String> {
    Ok(spectyn_mesh::providers::local_servers::detect_local_servers().await)
}

/// One free-tier cloud provider, flattened for the onboarding picker.
#[derive(Debug, Serialize)]
pub struct FreeProviderInfo {
    pub slug: String,
    pub display: String,
    pub provider_type: String,
    pub base_url: String,
    pub api_key_env: String,
    pub default_model: String,
    pub get_key_url: String,
    pub no_credit_card: bool,
}

/// The free-plugin suggestion the onboarding D5 step renders: the curated
/// registry, the recommended default (Groq), and whether a free key is ALREADY
/// in the environment (→ zero-config, no paste needed).
#[derive(Debug, Serialize)]
pub struct FreeProviderSuggestion {
    pub registry: Vec<FreeProviderInfo>,
    pub recommended: FreeProviderInfo,
    pub detected_from_env: Option<String>,
}

/// Surface the default-on free-API plugin to the onboarding UI. The frontend
/// shows `recommended` (with `get_key_url`) when nothing else is configured, and
/// skips the paste entirely when `detected_from_env` is Some (the key already
/// lives in the environment). Reads only env-var PRESENCE — never a key value.
#[tauri::command]
pub async fn detect_free_provider() -> Result<FreeProviderSuggestion, String> {
    use spectyn_mesh::providers::free_plugin;
    fn to_info(p: &free_plugin::FreeProvider) -> FreeProviderInfo {
        FreeProviderInfo {
            slug: p.slug.to_string(),
            display: p.display.to_string(),
            provider_type: p.provider_type.to_string(),
            base_url: p.base_url.to_string(),
            api_key_env: p.api_key_env.to_string(),
            default_model: p.default_model.to_string(),
            get_key_url: p.get_key_url.to_string(),
            no_credit_card: p.no_credit_card,
        }
    }
    Ok(FreeProviderSuggestion {
        registry: free_plugin::FREE_PROVIDERS.iter().map(to_info).collect(),
        recommended: to_info(free_plugin::default_free_provider()),
        detected_from_env: free_plugin::detect_free_from_env().map(|p| p.slug.to_string()),
    })
}

/// Input for `finalize_onboarding_config` — what the GUI's Provider step chose.
#[derive(Debug, Deserialize)]
pub struct FinalizeOnboardingInput {
    /// Subscription block names in priority order
    /// (`claude_cli` / `codex_oauth` / `gemini_oauth`).
    #[serde(default)]
    pub ordered: Vec<String>,
    /// Chosen free-plugin slug (`groq` / `cerebras` / `openrouter` / `gemini`), if any.
    pub free_slug: Option<String>,
    /// A free key the user pasted during onboarding. Persisted via
    /// `keys::set_api_key` (properly escaped) — never logged, never string-built.
    pub free_key: Option<String>,
    /// Detected local Ollama base URL, if any.
    pub ollama_url: Option<String>,
}

/// Write the first-run `agents.toml` for the GUI onboarding flow — the streamlined
/// flow's missing config-write (unified onboarding design §8.1). Calls the SAME
/// core writer the CLI uses (`spectyn_mesh::onboarding_config::write_onboarding_config`)
/// so the generated config is identical across surfaces, then persists a pasted
/// free key via the escaping `keys::set_api_key` path. Returns the active
/// (primary) provider slug. No secret is string-built into the file here.
#[tauri::command]
pub async fn finalize_onboarding_config(
    app: tauri::AppHandle,
    input: FinalizeOnboardingInput,
) -> Result<String, String> {
    use spectyn_mesh::providers::free_plugin;
    use spectyn_mesh::providers::local_servers::LocalServer;

    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e: tauri::Error| e.to_string())?;
    std::fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
    let cfg_path = config_dir.join("agents.toml");

    // Free slug → registry entry (reject an unknown slug rather than write a
    // dead provider block).
    let free = match input.free_slug.as_deref() {
        Some(slug) => Some(
            free_plugin::free_provider_by_slug(slug)
                .ok_or_else(|| format!("unknown free provider slug: {slug}"))?,
        ),
        None => None,
    };

    // Ollama URL → a minimal LocalServer (model list unknown at this seam → the
    // writer falls back to a sane default tag).
    let ollama = input.ollama_url.as_ref().map(|url| LocalServer {
        name: "ollama".to_string(),
        base_url: url.clone(),
        models: vec![],
    });

    let ordered_refs: Vec<&str> = input.ordered.iter().map(|s| s.as_str()).collect();

    let active = spectyn_mesh::onboarding_config::write_onboarding_config(
        &cfg_path,
        &ordered_refs,
        ollama.as_ref(),
        free,
    )
    .map_err(|e| e.to_string())?;

    // Persist a pasted free key (if any) via the escaping keys API.
    if let (Some(slug), Some(key)) = (input.free_slug.as_deref(), input.free_key.as_deref()) {
        if !key.trim().is_empty() {
            spectyn_mesh::keys::set_api_key(&cfg_path, slug, key).map_err(|e| e.to_string())?;
        }
    }

    Ok(active)
}

/// Validates a URL string for the `open_external_url` command.
///
/// Accepts only:
///   - `https://<host>...` with any host
///   - `http://<host>...` where host is exactly `localhost` or `127.0.0.1`
///
/// Rejects:
///   - Any other scheme (file://, javascript:, vscode://, spectyn://, ...)
///   - URLs with a userinfo component (e.g. `http://localhost@attacker.com`)
///     because browsers route those to the userinfo-stripped authority.
///   - Hosts that *look* like localhost but aren't, e.g. `localhost.attacker.com`,
///     `localhost.evil`, or any subdomain of localhost. The previous
///     `starts_with("http://localhost")` check matched these.
fn validate_external_url(url: &str) -> Result<(), &'static str> {
    // Split scheme.
    let (scheme, rest) = match url.split_once("://") {
        Some(parts) => parts,
        None => return Err("URL missing scheme"),
    };

    // Take the authority (everything before the first '/', '?', or '#').
    let authority_end = rest
        .find(|c: char| c == '/' || c == '?' || c == '#')
        .unwrap_or(rest.len());
    let authority = &rest[..authority_end];

    // Reject userinfo (anything before '@' in the authority).
    if authority.contains('@') {
        return Err("URLs with userinfo are not allowed");
    }

    // Strip port for host comparison.
    let host = authority.split(':').next().unwrap_or("");

    match scheme {
        "https" => {
            if host.is_empty() {
                return Err("https URL missing host");
            }
            Ok(())
        }
        "http" => {
            // Exact-match localhost / 127.0.0.1 only.
            if host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" {
                Ok(())
            } else {
                Err("Only HTTPS or http://localhost (exact host) URLs are allowed")
            }
        }
        _ => Err("Only HTTPS or http://localhost URLs are allowed"),
    }
}

#[tauri::command]
pub async fn open_external_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    tracing::info!("[open_external_url] Called with url={}", url);
    if let Err(reason) = validate_external_url(&url) {
        tracing::error!("[open_external_url] Rejected: {}", reason);
        return Err(reason.to_string());
    }
    // Use tauri-plugin-opener (cross-platform). On iOS this routes through
    // UIApplication.openURL: so the URL opens in Safari — the previous `open`
    // crate spawned a subprocess that the iOS sandbox blocks, which made the
    // JS-side openExternal() fall back to navigating the embedded WKWebView,
    // and Google rejects embedded-webview OAuth (403 disallowed_useragent).
    match app.opener().open_url(url.clone(), None::<&str>) {
        Ok(()) => {
            tracing::info!("[open_external_url] Browser opened successfully");
            Ok(())
        }
        Err(e) => {
            tracing::error!("[open_external_url] Failed: {}", e);
            Err(format!("Cannot open browser: {}", e))
        }
    }
}

#[cfg(test)]
mod open_external_url_tests {
    use super::validate_external_url;

    #[test]
    fn accepts_https() {
        assert!(validate_external_url("https://example.com").is_ok());
        assert!(validate_external_url("https://example.com/path?q=1").is_ok());
        assert!(validate_external_url("https://api.openai.com/v1/models").is_ok());
    }

    #[test]
    fn accepts_http_localhost_and_loopback() {
        assert!(validate_external_url("http://localhost").is_ok());
        assert!(validate_external_url("http://localhost/").is_ok());
        assert!(validate_external_url("http://localhost:7878/oauth/google").is_ok());
        assert!(validate_external_url("http://127.0.0.1:7878/").is_ok());
        assert!(validate_external_url("http://LOCALHOST/").is_ok()); // case-insensitive
    }

    #[test]
    fn rejects_localhost_lookalikes() {
        // H-2: the old `starts_with("http://localhost")` check let these through.
        assert!(validate_external_url("http://localhost.attacker.com/").is_err());
        assert!(validate_external_url("http://localhost.evil/").is_err());
        assert!(validate_external_url("http://localhostX/").is_err());
    }

    #[test]
    fn rejects_userinfo() {
        // H-2: `http://localhost@attacker.com/` routes to attacker.com.
        assert!(validate_external_url("http://localhost@attacker.com/").is_err());
        assert!(validate_external_url("https://user:pass@example.com/").is_err());
    }

    #[test]
    fn rejects_other_schemes() {
        assert!(validate_external_url("file:///etc/passwd").is_err());
        assert!(validate_external_url("javascript:alert(1)").is_err());
        assert!(validate_external_url("vscode://path").is_err());
        assert!(validate_external_url("spectyn://oauth/callback").is_err());
        assert!(validate_external_url("ftp://example.com/").is_err());
    }

    #[test]
    fn rejects_malformed() {
        assert!(validate_external_url("not a url").is_err());
        assert!(validate_external_url("https://").is_err());
    }

    #[test]
    fn rejects_non_loopback_http() {
        assert!(validate_external_url("http://example.com/").is_err());
        assert!(validate_external_url("http://10.0.0.1/").is_err());
    }
}

#[tauri::command]
pub async fn validate_api_key(
    http: tauri::State<'_, super::HttpClient>,
    provider: String,
    key: String,
) -> Result<ValidationResult, String> {
    let client = &http.0;

    let result = match provider.as_str() {
        "openai" => {
            let resp = client
                .get("https://api.openai.com/v1/models")
                .bearer_auth(&key)
                .send()
                .await;
            parse_model_list_response(resp, "data", "id").await
        }
        "anthropic" => {
            let resp = client
                .get("https://api.anthropic.com/v1/models")
                .header("x-api-key", &key)
                .header("anthropic-version", "2023-06-01")
                .send()
                .await;
            parse_model_list_response(resp, "data", "id").await
        }
        "gemini" => {
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models?key={}",
                key
            );
            let resp = client.get(&url).send().await;
            parse_model_list_response(resp, "models", "name").await
        }
        "groq" => {
            let resp = client
                .get("https://api.groq.com/openai/v1/models")
                .bearer_auth(&key)
                .send()
                .await;
            parse_model_list_response(resp, "data", "id").await
        }
        "cerebras" => {
            // Free-plugin tier (OpenAI-compatible). Bearer + GET /models.
            let resp = client
                .get("https://api.cerebras.ai/v1/models")
                .bearer_auth(&key)
                .send()
                .await;
            parse_model_list_response(resp, "data", "id").await
        }
        "openrouter" => {
            let resp = client
                .get("https://openrouter.ai/api/v1/models")
                .bearer_auth(&key)
                .send()
                .await;
            parse_model_list_response(resp, "data", "id").await
        }
        "codex" => {
            // OpenAI Codex uses the same OpenAI API
            let resp = client
                .get("https://api.openai.com/v1/models")
                .bearer_auth(&key)
                .send()
                .await;
            parse_model_list_response(resp, "data", "id").await
        }
        "opencode" => {
            // OpenCode — OpenAI-compatible API
            let resp = client
                .get("https://api.open-code.dev/v1/models")
                .bearer_auth(&key)
                .send()
                .await;
            parse_model_list_response(resp, "data", "id").await
        }
        "deepseek" => {
            let resp = client
                .get("https://api.deepseek.com/v1/models")
                .bearer_auth(&key)
                .send()
                .await;
            parse_model_list_response(resp, "data", "id").await
        }
        "mistral" => {
            let resp = client
                .get("https://api.mistral.ai/v1/models")
                .bearer_auth(&key)
                .send()
                .await;
            parse_model_list_response(resp, "data", "id").await
        }
        "xai" => {
            let resp = client
                .get("https://api.x.ai/v1/models")
                .bearer_auth(&key)
                .send()
                .await;
            parse_model_list_response(resp, "data", "id").await
        }
        "azure" => {
            // Azure uses api-key header, not Bearer. Key format: endpoint|api_key
            let parts: Vec<&str> = key.splitn(2, '|').collect();
            if parts.len() != 2 {
                return Ok(ValidationResult {
                    ok: false, models: vec![],
                    error: Some("Format: endpoint|api_key".to_string()),
                });
            }
            let endpoint = parts[0].trim_end_matches('/');
            let api_key = parts[1];
            let url = format!("{}/openai/deployments?api-version=2024-02-01", endpoint);
            let resp = client.get(&url).header("api-key", api_key).send().await;
            match resp {
                Ok(r) if r.status().is_success() => Ok(ValidationResult {
                    ok: true, models: vec!["gpt-4o".to_string()], error: None,
                }),
                Ok(r) => Ok(ValidationResult {
                    ok: false, models: vec![],
                    error: Some(format!("HTTP {}", r.status())),
                }),
                Err(e) => Ok(ValidationResult {
                    ok: false, models: vec![], error: Some(e.to_string()),
                }),
            }
        }
        "bedrock" => {
            // Bedrock uses AWS IAM credentials — validate by checking if credentials exist
            let has_env = std::env::var("AWS_ACCESS_KEY_ID").map(|v| !v.is_empty()).unwrap_or(false);
            let has_file = dirs::home_dir()
                .map(|h| h.join(".aws").join("credentials").exists())
                .unwrap_or(false);
            Ok(ValidationResult {
                ok: has_env || has_file,
                models: vec!["anthropic.claude-3-sonnet".to_string()],
                error: if !has_env && !has_file {
                    Some("No AWS credentials found".to_string())
                } else {
                    None
                },
            })
        }
        _ => Err(format!("Unknown provider: {}", provider)),
    };

    result
}

async fn parse_model_list_response(
    resp: Result<reqwest::Response, reqwest::Error>,
    array_key: &str,
    name_key: &str,
) -> Result<ValidationResult, String> {
    match resp {
        Ok(r) if r.status().is_success() => {
            let body: serde_json::Value = r.json().await.unwrap_or_default();
            let models = body[array_key]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m[name_key].as_str().map(String::from))
                        .take(20)
                        .collect()
                })
                .unwrap_or_default();
            Ok(ValidationResult {
                ok: true,
                models,
                error: None,
            })
        }
        Ok(r) => Ok(ValidationResult {
            ok: false,
            models: vec![],
            error: Some(format!("HTTP {}", r.status())),
        }),
        Err(e) => Ok(ValidationResult {
            ok: false,
            models: vec![],
            error: Some(e.to_string()),
        }),
    }
}

#[tauri::command]
pub async fn write_config(
    app: tauri::AppHandle,
    app_config: tauri::State<'_, super::settings::AppConfigState>,
    data: OnboardingConfig,
) -> Result<(), String> {
    // Update in-memory AppConfig so subsequent commands use the correct auth key & port
    {
        let mut cfg = app_config.write();
        cfg.auth_key = data.auth_key.clone();
        cfg.daemon_port = data.port;
        cfg.hub_url = format!("http://localhost:{}", data.port);
    }
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e: tauri::Error| e.to_string())?;
    std::fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;

    let toml_path = config_dir.join("agents.toml");

    // If agents.toml already has real API keys, preserve [providers.*]/[agent.*]
    // and update only [core]/[auth]/[identity]/[sync] in-place. The previous
    // implementation matched the literal substring "api_key" (which appears in
    // template comments and in `api_key_env` field names) and then silently
    // returned Ok(()) without writing anything — losing all changes.
    if toml_path.exists() {
        let backup = config_dir.join("agents.toml.bak");
        std::fs::copy(&toml_path, &backup).ok();
        if let Ok(existing) = std::fs::read_to_string(&toml_path) {
            if let Ok(mut doc) = existing.parse::<toml::Table>() {
                let has_real_keys = doc.get("providers")
                    .and_then(|v| v.as_table())
                    .map(|providers| providers.values().any(|v| {
                        v.as_table().map(|t| {
                            t.get("api_key").and_then(|x| x.as_str())
                                .is_some_and(|s| !s.is_empty())
                            || t.get("api_key_env").and_then(|x| x.as_str())
                                .is_some_and(|s| !s.is_empty())
                        }).unwrap_or(false)
                    }))
                    .unwrap_or(false);

                if has_real_keys {
                    let mut core = toml::map::Map::new();
                    core.insert("host".into(), toml::Value::String("0.0.0.0".into()));
                    core.insert("port".into(), toml::Value::Integer(data.port as i64));
                    core.insert("hub_api_key".into(), toml::Value::String(data.auth_key.clone()));
                    doc.insert("core".into(), toml::Value::Table(core));

                    let mut auth = toml::map::Map::new();
                    auth.insert("bearer_token".into(), toml::Value::String(data.auth_key.clone()));
                    doc.insert("auth".into(), toml::Value::Table(auth));

                    if let (Some(provider), Some(sub), Some(email)) =
                        (&data.identity_provider, &data.identity_sub, &data.identity_email)
                    {
                        if !provider.is_empty() {
                            let mut id = toml::map::Map::new();
                            id.insert("provider".into(), toml::Value::String(provider.clone()));
                            id.insert("sub".into(), toml::Value::String(sub.clone()));
                            id.insert("email".into(), toml::Value::String(email.clone()));
                            doc.insert("identity".into(), toml::Value::Table(id));
                        }
                    }

                    if data.is_primary.unwrap_or(false) {
                        let mut sync = toml::map::Map::new();
                        sync.insert("is_primary".into(), toml::Value::Boolean(true));
                        doc.insert("sync".into(), toml::Value::Table(sync));
                    }

                    let serialized = toml::to_string(&doc).map_err(|e| e.to_string())?;
                    std::fs::write(&toml_path, &serialized).map_err(|e| e.to_string())?;
                    tracing::info!("Updated [core]/[auth] in existing agents.toml (preserved providers)");
                    return Ok(());
                }
            }
        }
    }

    let mut toml = format!(
        "[core]\nhost = \"0.0.0.0\"\nport = {}\nhub_api_key = \"{}\"\n\n",
        data.port, data.auth_key
    );

    // Ollama (handled separately)
    if let Some(ref endpoint) = data.ollama_endpoint {
        toml.push_str(&format!(
            "[providers.ollama]\ntype = \"ollama\"\nurl = \"{}\"\ntier = \"local\"\n\n",
            endpoint
        ));
    }

    // Discovered providers (token_source = "auto" or "env")
    // Skip ollama if already written above via ollama_endpoint
    for p in &data.discovered_providers {
        if p.name == "ollama" && data.ollama_endpoint.is_some() {
            continue; // Already written above with explicit URL
        }
        toml.push_str(&format!("[providers.{}]\ntype = \"{}\"\ntier = \"{}\"\n",
            p.name, p.provider_type, p.tier));
        if p.token_source == "auto" {
            toml.push_str("token_source = \"auto\"\n");
        } else if let Some(ref env_key) = p.env_key_name {
            toml.push_str(&format!("api_key_env = \"{}\"\n", env_key));
        }
        if let Some(ref base_url) = p.base_url {
            toml.push_str(&format!("base_url = \"{}\"\n", base_url));
        }
        toml.push('\n');
    }

    // Manual providers (API key in .env)
    for p in &data.manual_providers {
        let env_key = format!("{}_API_KEY", p.name.to_uppercase());
        toml.push_str(&format!("[providers.{}]\ntype = \"{}\"\ntier = \"{}\"\napi_key_env = \"{}\"\n",
            p.name, p.provider_type, p.tier, env_key));
        if let Some(ref base_url) = p.base_url {
            toml.push_str(&format!("base_url = \"{}\"\n", base_url));
        }
        // Azure-specific
        if let Some(ref endpoint) = p.endpoint {
            toml.push_str(&format!("endpoint = \"{}\"\n", endpoint));
            toml.push_str("api_version = \"2024-02-01\"\n");
        }
        // Bedrock-specific
        if let Some(ref region) = p.region {
            toml.push_str(&format!("region = \"{}\"\n", region));
        }
        toml.push('\n');
    }

    // Default agent — full tool set for self-iteration capability
    toml.push_str(&format!(
        "[agent.master]\nprovider = \"{}\"\nmodel = \"{}\"\ntools = [\"shell\", \"file_read\", \"file_write\", \"file_edit\", \"content_search\", \"glob_search\", \"web_search\", \"memory_store\", \"memory_recall\"]\ninstructions = \"\"\"\nYou are a senior software engineer AI assistant running on the user's machine. You have direct access to the filesystem and shell via tools. ALWAYS use your tools to accomplish tasks — never guess or hallucinate file contents.\n\nKey behaviors:\n- Use shell tool to run commands (git, cargo, npm, etc.)\n- Use file_read to read files before editing them\n- Use file_edit for precise string replacements in existing files\n- Use file_write to create new files\n- Use content_search (ripgrep) to search code\n- Use glob_search to find files by pattern\n- Always respond in the user's language\n- Be concise. Show results, not explanations.\n\"\"\"\n\n",
        data.default_agent_provider, data.default_agent_model
    ));

    // Auth
    toml.push_str(&format!("[auth]\nbearer_token = \"{}\"\n", data.auth_key));

    // Identity
    if let (Some(ref provider), Some(ref sub), Some(ref email)) =
        (&data.identity_provider, &data.identity_sub, &data.identity_email)
    {
        if !provider.is_empty() {
            toml.push_str(&format!(
                "\n[identity]\nprovider = \"{}\"\nsub = \"{}\"\nemail = \"{}\"\n",
                provider, sub, email
            ));
        }
    }

    // Sync
    if data.is_primary.unwrap_or(false) {
        toml.push_str("\n[sync]\nis_primary = true\n");
    }

    std::fs::write(&toml_path, &toml).map_err(|e| e.to_string())?;

    // .env — only manual providers (discovered env vars already exist)
    let env_path = config_dir.join(".env");
    let mut env_content = String::new();
    for p in &data.manual_providers {
        env_content.push_str(&format!(
            "{}_API_KEY={}\n",
            p.name.to_uppercase(),
            p.api_key
        ));
    }
    if let Some(ref token) = data.telegram_token {
        env_content.push_str(&format!("TELEGRAM_BOT_TOKEN={}\n", token));
    }
    std::fs::write(&env_path, &env_content).map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub fn generate_qr_data(hub_url: String, auth_key: String, node_id: String) -> QrPayload {
    QrPayload {
        payload_type: "spectyn-mesh-hub".to_string(),
        version: 1,
        hub_url,
        auth_key,
        node_id,
    }
}

#[tauri::command]
pub fn get_local_ip() -> String {
    // Connect to a public DNS to determine local network interface IP
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| {
            socket.connect("8.8.8.8:80")?;
            socket.local_addr()
        })
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string())
}

#[cfg(desktop)]
#[tauri::command]
pub async fn launch_daemon(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::daemon::DaemonState>,
    http: tauri::State<'_, super::HttpClient>,
    vault_pin: String,
    port: u16,
    binary_path: String,
) -> Result<DaemonStatus, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // Stop any existing daemon first (e.g. from auto-start) to avoid port conflict
    if state.is_running() {
        tracing::info!("Stopping existing daemon before onboarding launch");
        state.kill();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // Use the Tauri app config dir so daemon reads the config written by onboarding
    let config_dir = app.path().app_config_dir().map_err(|e: tauri::Error| e.to_string())?;
    let config_path = config_dir.join("agents.toml");
    tracing::info!("Launching daemon with config: {:?}", config_path);

    let child = Command::new(&binary_path)
        .arg("--host")
        .arg("0.0.0.0")
        .arg("--port")
        .arg(port.to_string())
        .arg("--config")
        .arg(config_path.to_string_lossy().as_ref())
        .arg("daemon")
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start daemon: {}", e))?;

    let pid = child.id();

    {
        let mut proc = state.process.lock().map_err(|e| e.to_string())?;
        *proc = Some(child);
    }

    // Wait for daemon to start (startup can take 15-17s due to LLM provider reachability checks)
    tokio::time::sleep(std::time::Duration::from_millis(3000)).await;

    let url = format!("http://localhost:{}/health", port);
    for i in 0..27 {
        match http.0.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!("Daemon healthy after ~{}s", 3 + i);
                return Ok(DaemonStatus {
                    ok: true,
                    pid: Some(pid),
                    port,
                });
            }
            _ => {
                if i < 26 {
                    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                }
            }
        }
    }

    Ok(DaemonStatus {
        ok: false,
        pid: Some(pid),
        port,
    })
}
