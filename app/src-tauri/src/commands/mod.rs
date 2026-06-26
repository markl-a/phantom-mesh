pub mod agent;
pub mod broker_login;
pub mod capture_focus_wire;
pub mod capture_food_wire;
pub mod capture_habit_wire;
pub mod cluster;
pub mod cluster_dispatch_wire;
pub mod conversation;
pub mod cluster_peers;
pub mod daily_review_wire;
pub mod dispatch;
pub mod identity_status;
pub mod recall_wire;
pub mod life_stats;
pub mod note_wire;
pub mod partner_wire;
pub mod event_storage_wire;
pub mod event_detail;
pub mod local_keys;
pub mod mobile_settings;
pub mod hardware;
pub mod health;
pub mod memory;
pub mod miui;
pub mod networking;
pub mod oauth;
pub mod onboarding;
pub mod onboarding_wire;
pub mod provider;
pub mod providers_wire;
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
