use serde::Serialize;
use std::path::PathBuf;

use super::auth_profile::{CredentialType, ProviderTier};

// ── Data Structures ────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum CredentialSource {
    TokenFile(PathBuf),
    EnvVar(String),
    LocalProbe(String),
    CliTool(String),
}

impl CredentialSource {
    pub fn as_str(&self) -> &str {
        match self {
            Self::TokenFile(_) => "token_file",
            Self::EnvVar(_) => "env_var",
            Self::LocalProbe(_) => "local_probe",
            Self::CliTool(_) => "cli_tool",
        }
    }
}

/// Internal — contains secrets, never sent to frontend
#[derive(Debug, Clone)]
pub struct DiscoveredCredential {
    pub provider_name: String,
    pub provider_type: String,
    pub source: CredentialSource,
    pub credential: CredentialType,
    pub tier: ProviderTier,
    pub display_label: String,
    pub available_models: Vec<String>,
}

/// Frontend-safe — no secrets
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredProviderInfo {
    pub name: String,
    pub provider_type: String,
    pub source: String,
    pub tier: String,
    pub display_label: String,
    pub models: Vec<String>,
}

impl DiscoveredCredential {
    pub fn to_frontend_info(&self) -> DiscoveredProviderInfo {
        let tier_str = match self.tier {
            ProviderTier::Local => "local",
            ProviderTier::FreeApi => "free",
            ProviderTier::Subscription => "subscription",
            ProviderTier::PayAsYouGo => "payg",
        };
        DiscoveredProviderInfo {
            name: self.provider_name.clone(),
            provider_type: self.provider_type.clone(),
            source: self.source.as_str().to_string(),
            tier: tier_str.to_string(),
            display_label: self.display_label.clone(),
            models: self.available_models.clone(),
        }
    }
}

// ── Scan Engine ────────────────────────────────────────────

pub async fn scan_all() -> Vec<DiscoveredCredential> {
    let (ollama, codex, copilot, claude, gcloud, opencode, env_vars, aws) = tokio::join!(
        scan_ollama(),
        scan_codex(),
        scan_copilot(),
        scan_claude_cli(),
        scan_gcloud(),
        scan_opencode(),
        scan_env_vars(),
        scan_aws(),
    );
    let mut results = Vec::new();
    results.extend(ollama);
    results.extend(codex);
    results.extend(copilot);
    results.extend(claude);
    results.extend(gcloud);
    results.extend(opencode);
    results.extend(env_vars);
    results.extend(aws);
    results
}

// ── Per-Source Scanners ────────────────────────────────────

async fn scan_ollama() -> Vec<DiscoveredCredential> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    match client.get("http://localhost:11434/api/tags").send().await {
        Ok(resp) if resp.status().is_success() => {
            let models = if let Ok(body) = resp.json::<serde_json::Value>().await {
                body["models"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| m["name"].as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                vec![]
            };
            vec![DiscoveredCredential {
                provider_name: "ollama".to_string(),
                provider_type: "ollama".to_string(),
                source: CredentialSource::LocalProbe("http://localhost:11434".to_string()),
                credential: CredentialType::ApiKey {
                    key: String::new(),
                },
                tier: ProviderTier::Local,
                display_label: "Ollama (本地)".to_string(),
                available_models: models,
            }]
        }
        _ => vec![],
    }
}

async fn scan_codex() -> Vec<DiscoveredCredential> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return vec![],
    };

    let paths = [
        home.join(".codex").join("auth.json"),
        home.join(".codex-cli").join("auth.json"),
    ];

    for path in &paths {
        if let Ok(content) = tokio::fs::read_to_string(path).await {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                // Nested format: { tokens: { access_token: "..." } }
                // Flat format: { access_token: "..." }
                let token = json["tokens"]["access_token"]
                    .as_str()
                    .or_else(|| json["access_token"].as_str());

                if let Some(tok) = token {
                    if !tok.is_empty() {
                        return vec![DiscoveredCredential {
                            provider_name: "codex".to_string(),
                            provider_type: "codex".to_string(),
                            source: CredentialSource::TokenFile(path.clone()),
                            credential: CredentialType::TokenFile {
                                token: tok.to_string(),
                                source_path: path.clone(),
                            },
                            tier: ProviderTier::Subscription,
                            display_label: "Codex (ChatGPT Plus)".to_string(),
                            available_models: vec![],
                        }];
                    }
                }
            }
        }
    }
    vec![]
}

async fn scan_copilot() -> Vec<DiscoveredCredential> {
    let paths = copilot_token_paths();

    for path in &paths {
        if let Ok(content) = tokio::fs::read_to_string(path).await {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                // hosts.json: { "github.com": { "oauth_token": "gho_xxx", "user": "..." } }
                // apps.json: similar structure
                if let Some(obj) = json.as_object() {
                    for (_host, val) in obj {
                        if let Some(token) = val["oauth_token"].as_str() {
                            if !token.is_empty() {
                                return vec![DiscoveredCredential {
                                    provider_name: "copilot".to_string(),
                                    provider_type: "copilot".to_string(),
                                    source: CredentialSource::TokenFile(path.clone()),
                                    credential: CredentialType::TokenFile {
                                        token: token.to_string(),
                                        source_path: path.clone(),
                                    },
                                    tier: ProviderTier::Subscription,
                                    display_label: "GitHub Copilot".to_string(),
                                    available_models: vec![],
                                }];
                            }
                        }
                    }
                }
            }
        }
    }
    vec![]
}

pub fn copilot_token_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // Linux/macOS
    if let Some(config) = dirs::config_dir() {
        paths.push(config.join("github-copilot").join("hosts.json"));
        paths.push(config.join("github-copilot").join("apps.json"));
    }

    // Windows: %LOCALAPPDATA%
    #[cfg(target_os = "windows")]
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let dir = PathBuf::from(local_app_data).join("github-copilot");
        paths.push(dir.join("hosts.json"));
        paths.push(dir.join("apps.json"));
    }

    paths
}

async fn scan_claude_cli() -> Vec<DiscoveredCredential> {
    let paths = claude_cli_paths();

    for path in &paths {
        if let Ok(content) = tokio::fs::read_to_string(path).await {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                // Try multiple known schemas
                let token = json["sessionKey"]
                    .as_str()
                    .or_else(|| json["token"].as_str())
                    .or_else(|| json["access_token"].as_str())
                    .or_else(|| json["apiKey"].as_str());

                if let Some(tok) = token {
                    if !tok.is_empty() {
                        tracing::debug!("Claude CLI token found at: {}", path.display());
                        return vec![DiscoveredCredential {
                            provider_name: "claude_cli".to_string(),
                            provider_type: "claude_cli".to_string(),
                            source: CredentialSource::TokenFile(path.clone()),
                            credential: CredentialType::TokenFile {
                                token: tok.to_string(),
                                source_path: path.clone(),
                            },
                            tier: ProviderTier::Subscription,
                            display_label: "Claude CLI".to_string(),
                            available_models: vec![],
                        }];
                    }
                }
            }
        }
    }
    vec![]
}

pub fn claude_cli_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(home) = dirs::home_dir() {
        let claude_dir = home.join(".claude");
        paths.push(claude_dir.join(".credentials.json"));
        paths.push(claude_dir.join("credentials.json"));
        paths.push(claude_dir.join("auth.json"));
    }

    // Windows: %APPDATA%\claude\
    if let Some(config) = dirs::config_dir() {
        let claude_dir = config.join("claude");
        paths.push(claude_dir.join(".credentials.json"));
        paths.push(claude_dir.join("credentials.json"));
        paths.push(claude_dir.join("auth.json"));
    }

    paths
}

async fn scan_gcloud() -> Vec<DiscoveredCredential> {
    let adc_path = if let Some(config) = dirs::config_dir() {
        config
            .join("gcloud")
            .join("application_default_credentials.json")
    } else {
        return vec![];
    };

    if let Ok(content) = tokio::fs::read_to_string(&adc_path).await {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if json["client_id"].as_str().is_some() || json["type"].as_str().is_some() {
                return vec![DiscoveredCredential {
                    provider_name: "gemini".to_string(),
                    provider_type: "gemini".to_string(),
                    source: CredentialSource::TokenFile(adc_path),
                    credential: CredentialType::OAuth {
                        access_token: String::new(), // Resolved at runtime by gcloud SDK
                        refresh_token: json["refresh_token"]
                            .as_str()
                            .map(String::from),
                        expires_at: None,
                    },
                    tier: ProviderTier::FreeApi,
                    display_label: "Google Gemini (gcloud ADC)".to_string(),
                    available_models: vec![],
                }];
            }
        }
    }
    vec![]
}

async fn scan_opencode() -> Vec<DiscoveredCredential> {
    let found = tokio::task::spawn_blocking(|| which::which("opencode").is_ok())
        .await
        .unwrap_or(false);

    if found {
        vec![DiscoveredCredential {
            provider_name: "opencode".to_string(),
            provider_type: "opencode".to_string(),
            source: CredentialSource::CliTool("opencode".to_string()),
            credential: CredentialType::ApiKey {
                key: String::new(),
            },
            tier: ProviderTier::Subscription,
            display_label: "OpenCode".to_string(),
            available_models: vec![],
        }]
    } else {
        vec![]
    }
}

// Async for interface consistency with scan_all() tokio::join! — no await points needed
async fn scan_env_vars() -> Vec<DiscoveredCredential> {
    let checks = [
        ("OPENAI_API_KEY", "openai", "openai", "OpenAI"),
        ("ANTHROPIC_API_KEY", "anthropic", "anthropic", "Anthropic"),
        ("GEMINI_API_KEY", "gemini", "gemini", "Gemini"),
        ("GROQ_API_KEY", "groq", "groq", "Groq"),
        ("OPENROUTER_API_KEY", "openrouter", "openai_compat", "OpenRouter"),
        ("DEEPSEEK_API_KEY", "deepseek", "openai_compat", "DeepSeek"),
        ("MISTRAL_API_KEY", "mistral", "openai_compat", "Mistral"),
        ("XAI_API_KEY", "xai", "openai_compat", "xAI (Grok)"),
    ];

    checks
        .iter()
        .filter_map(|(env_key, name, ptype, label)| {
            scan_single_env_var(env_key, name, ptype, label)
        })
        .collect()
}

fn scan_single_env_var(
    env_key: &str,
    name: &str,
    provider_type: &str,
    label: &str,
) -> Option<DiscoveredCredential> {
    match std::env::var(env_key) {
        Ok(val) if !val.is_empty() => Some(DiscoveredCredential {
            provider_name: name.to_string(),
            provider_type: provider_type.to_string(),
            source: CredentialSource::EnvVar(env_key.to_string()),
            credential: CredentialType::ApiKey { key: val },
            tier: ProviderTier::PayAsYouGo,
            display_label: label.to_string(),
            available_models: vec![],
        }),
        _ => None,
    }
}

async fn scan_aws() -> Vec<DiscoveredCredential> {
    // Check env var first
    if std::env::var("AWS_ACCESS_KEY_ID").map(|v| !v.is_empty()).unwrap_or(false) {
        return vec![DiscoveredCredential {
            provider_name: "bedrock".to_string(),
            provider_type: "bedrock".to_string(),
            source: CredentialSource::EnvVar("AWS_ACCESS_KEY_ID".to_string()),
            credential: CredentialType::ApiKey { key: String::new() },
            tier: ProviderTier::PayAsYouGo,
            display_label: "AWS Bedrock".to_string(),
            available_models: vec![],
        }];
    }

    // Check ~/.aws/credentials
    if let Some(home) = dirs::home_dir() {
        let aws_creds = home.join(".aws").join("credentials");
        if aws_creds.exists() {
            return vec![DiscoveredCredential {
                provider_name: "bedrock".to_string(),
                provider_type: "bedrock".to_string(),
                source: CredentialSource::TokenFile(aws_creds),
                credential: CredentialType::ApiKey { key: String::new() },
                tier: ProviderTier::PayAsYouGo,
                display_label: "AWS Bedrock".to_string(),
                available_models: vec![],
            }];
        }
    }

    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovered_credential_to_frontend_info_strips_secrets() {
        let cred = DiscoveredCredential {
            provider_name: "openai".to_string(),
            provider_type: "openai".to_string(),
            source: CredentialSource::EnvVar("OPENAI_API_KEY".to_string()),
            credential: CredentialType::ApiKey {
                key: "sk-secret-key-12345".to_string(),
            },
            tier: ProviderTier::PayAsYouGo,
            display_label: "OpenAI".to_string(),
            available_models: vec![],
        };

        let info = cred.to_frontend_info();
        assert_eq!(info.name, "openai");
        assert_eq!(info.provider_type, "openai");
        assert_eq!(info.source, "env_var");
        assert_eq!(info.tier, "payg");
        assert_eq!(info.display_label, "OpenAI");
        // Verify no secret key in the info struct fields
        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains("sk-secret-key-12345"));
    }

    #[test]
    fn discovered_credential_to_frontend_info_token_file() {
        let cred = DiscoveredCredential {
            provider_name: "copilot".to_string(),
            provider_type: "copilot".to_string(),
            source: CredentialSource::TokenFile("/home/user/.config/github-copilot/hosts.json".into()),
            credential: CredentialType::TokenFile {
                token: "gho_secret".to_string(),
                source_path: "/home/user/.config/github-copilot/hosts.json".into(),
            },
            tier: ProviderTier::Subscription,
            display_label: "GitHub Copilot".to_string(),
            available_models: vec!["gpt-4o".to_string()],
        };

        let info = cred.to_frontend_info();
        assert_eq!(info.source, "token_file");
        assert_eq!(info.tier, "subscription");
        assert_eq!(info.models, vec!["gpt-4o"]);
    }

    #[test]
    fn scan_env_vars_detects_set_keys() {
        // Set a test env var
        std::env::set_var("PHANTOM_MESH_TEST_OPENAI_API_KEY", "sk-test-123");

        let results = scan_single_env_var(
            "PHANTOM_MESH_TEST_OPENAI_API_KEY",
            "test_openai",
            "openai",
            "Test OpenAI",
        );

        assert!(results.is_some());
        let cred = results.unwrap();
        assert_eq!(cred.provider_name, "test_openai");
        assert!(matches!(cred.tier, ProviderTier::PayAsYouGo));

        std::env::remove_var("PHANTOM_MESH_TEST_OPENAI_API_KEY");
    }

    #[test]
    fn scan_env_vars_skips_unset_keys() {
        std::env::remove_var("PHANTOM_MESH_TEST_MISSING_KEY");
        let results = scan_single_env_var(
            "PHANTOM_MESH_TEST_MISSING_KEY",
            "missing",
            "openai_compat",
            "Missing",
        );
        assert!(results.is_none());
    }

    #[test]
    fn scan_env_vars_skips_empty_keys() {
        std::env::set_var("PHANTOM_MESH_TEST_EMPTY_KEY", "");
        let results = scan_single_env_var(
            "PHANTOM_MESH_TEST_EMPTY_KEY",
            "empty",
            "openai_compat",
            "Empty",
        );
        assert!(results.is_none());
        std::env::remove_var("PHANTOM_MESH_TEST_EMPTY_KEY");
    }

    #[test]
    fn credential_source_display() {
        assert_eq!(
            CredentialSource::EnvVar("KEY".to_string()).as_str(),
            "env_var"
        );
        assert_eq!(
            CredentialSource::TokenFile("/path".into()).as_str(),
            "token_file"
        );
        assert_eq!(
            CredentialSource::LocalProbe("http://localhost".to_string()).as_str(),
            "local_probe"
        );
        assert_eq!(
            CredentialSource::CliTool("opencode".to_string()).as_str(),
            "cli_tool"
        );
    }
}
