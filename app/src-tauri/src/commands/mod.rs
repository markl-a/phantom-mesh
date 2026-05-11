pub mod agent;
pub mod broker_login;
pub mod cluster;
pub mod local_keys;
pub mod hardware;
pub mod health;
pub mod memory;
pub mod networking;
pub mod oauth;
pub mod onboarding;
pub mod provider;
pub mod security;
pub mod settings;
pub mod supabase;
pub mod tasks;
pub mod goals;
pub mod browser;
pub mod pages;

/// Shared HTTP client for connection pooling across all commands.
pub struct HttpClient(pub reqwest::Client);

impl Default for HttpClient {
    fn default() -> Self {
        Self(
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120)) // LLM inference can take > 30s
                .build()
                .unwrap_or_default(),
        )
    }
}
