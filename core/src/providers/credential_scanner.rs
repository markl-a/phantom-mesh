use super::DiscoveredProviderInfo;
use std::path::PathBuf;

pub struct DiscoveredCredential {
    pub name: String,
    pub provider_type: String,
    pub source: String,
}

impl DiscoveredCredential {
    pub fn to_frontend_info(&self) -> DiscoveredProviderInfo {
        DiscoveredProviderInfo {
            name: self.name.clone(),
            provider_type: self.provider_type.clone(),
            source: self.source.clone(),
        }
    }
}

pub async fn scan_all() -> Vec<DiscoveredCredential> {
    let mut found = Vec::new();

    if std::env::var("OPENAI_API_KEY")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        found.push(DiscoveredCredential {
            name: "openai".into(),
            provider_type: "openai".into(),
            source: "env".into(),
        });
    }
    if std::env::var("ANTHROPIC_API_KEY")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        found.push(DiscoveredCredential {
            name: "anthropic".into(),
            provider_type: "anthropic".into(),
            source: "env".into(),
        });
    }
    if std::env::var("OPENROUTER_API_KEY")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        found.push(DiscoveredCredential {
            name: "openrouter".into(),
            provider_type: "openrouter".into(),
            source: "env".into(),
        });
    }

    found
}

pub fn copilot_token_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs_home() {
        paths.push(
            home.join(".config")
                .join("github-copilot")
                .join("hosts.json"),
        );
        paths.push(
            home.join(".config")
                .join("github-copilot")
                .join("apps.json"),
        );
    }
    paths
}

pub fn claude_cli_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs_home() {
        paths.push(home.join(".claude").join("credentials.json"));
        paths.push(home.join(".config").join("claude").join("credentials.json"));
    }
    paths
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}
