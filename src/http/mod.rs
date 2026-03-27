//! HTTP config structs and request types extracted from main.rs.
//!
//! These types are used by the daemon's HTTP API handlers.

use serde::Deserialize;

use crate::{
    AiCodeConfig, ClusterConfig, ComputerUseConfig, EmailConfig, EvalConfig, ImapConfig,
    MemoryConfig, PrivacyConfig, SearchConfig, SecurityConfig, TelegramConfig, TwitterConfig,
    BlogConfig, SlackConfig, DiscordConfig, LineConfig, WhatsAppConfig,
};

// ── Config ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct CoreConfig {
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub hub_api_key: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub core: Option<CoreConfig>,
    #[serde(default)]
    pub telegram: Option<TelegramConfig>,
    #[serde(default)]
    pub security: Option<SecurityConfig>,
    #[serde(default)]
    pub search: Option<SearchConfig>,
    #[serde(default)]
    pub ai_code: Option<AiCodeConfig>,
    #[serde(default)]
    pub computer_use: Option<ComputerUseConfig>,
    #[serde(default)]
    pub memory: Option<MemoryConfig>,
    #[serde(default)]
    pub eval: Option<EvalConfig>,
    #[serde(default)]
    pub email: Option<EmailConfig>,
    #[serde(default)]
    pub imap: Option<ImapConfig>,
    #[serde(default)]
    pub twitter: Option<TwitterConfig>,
    #[serde(default)]
    pub blog: Option<BlogConfig>,
    #[serde(default)]
    pub stripe: Option<StripeConfig>,
    #[serde(default)]
    pub render: Option<RenderConfig>,
    #[serde(default)]
    pub cluster: Option<ClusterConfig>,
    #[serde(default)]
    pub privacy: Option<PrivacyConfig>,
    #[serde(default)]
    pub slack: Option<SlackConfig>,
    #[serde(default)]
    pub discord: Option<DiscordConfig>,
    #[serde(default)]
    pub line: Option<LineConfig>,
    #[serde(default)]
    pub whatsapp: Option<WhatsAppConfig>,
    #[serde(default)]
    pub image_generate: Option<ImageGenerateAppConfig>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct ImageGenerateAppConfig {
    #[serde(default)]
    pub gemini_api_key: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct StripeConfig {
    #[serde(default)]
    pub secret_key: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct RenderConfig {
    #[serde(default)]
    pub api_key: String,
}

// ── Request types ──────────────────────────────────────────────────────────────

pub fn default_load_factor() -> f64 {
    1.0
}

#[derive(Debug, Deserialize)]
pub struct PowerEstimateRequest {
    pub node_id: String,
    pub duration_secs: f64,
    #[serde(default = "default_load_factor")]
    pub load_factor: f64,
}

#[derive(Debug, Deserialize)]
pub struct PowerProfitabilityRequest {
    pub node_id: String,
    pub expected_revenue_per_hour_usd: f64,
    #[serde(default)]
    pub api_cost_per_hour_usd: f64,
    #[serde(default = "default_load_factor")]
    pub load_factor: f64,
}

#[derive(Debug, Deserialize)]
pub struct PowerProfileUpsertRequest {
    pub idle_watts: f64,
    pub active_watts: f64,
    pub electricity_usd_per_kwh: f64,
    #[serde(default)]
    pub depreciation_usd_per_hour: f64,
    #[serde(default)]
    pub cooling_usd_per_hour: f64,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PricingRuleUpsertRequest {
    pub provider: String,
    pub model_pattern: String,
    pub input_usd_per_1m_tokens: f64,
    pub output_usd_per_1m_tokens: f64,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PricingEstimateRequest {
    pub provider: String,
    pub model: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
}
