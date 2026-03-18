use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tracing::{debug, info, warn};

use super::traits::*;
use super::ollama::OllamaProvider;
use super::openai_compat::OpenAiCompatProvider;
use super::anthropic::AnthropicProvider;
use super::openai::OpenAiProvider;
use super::gemini::GeminiProvider;
use super::groq::GroqProvider;
use super::reliable::classify_error;
use super::reliable::ErrorClass;
use super::rotation::ProviderRotation;
use super::codex::{CodexAwareProvider, CodexTokenManager, CodexUsageSnapshot, ModelInfo};

/// Parsed providers section from agents.toml
#[derive(Debug, Deserialize)]
struct AgentsToml {
    #[serde(default)]
    providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    routing: Vec<RouteHint>,
    #[serde(default)]
    smart_routing: Option<SmartRoutingConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct SmartRoutingConfig {
    #[allow(dead_code)]
    pub classifier_provider: Option<String>,
    #[allow(dead_code)]
    pub classifier_model: Option<String>,
    #[serde(default)]
    pub simple_providers: Vec<String>,
    #[serde(default)]
    pub medium_providers: Vec<String>,
    #[serde(default)]
    pub complex_providers: Vec<String>,
}

/// Provider Router — manages multiple LLM providers with hint-based routing.
/// Replacement for the monolithic LlmRouter.
pub struct ProviderRouter {
    providers: HashMap<String, Box<dyn Provider>>,
    /// Hint routes: "reasoning" → (provider_name, optional_model_override)
    routes: HashMap<String, (String, Option<String>)>,
    /// Ordered list for auto-routing fallback
    auto_order: Vec<String>,
    /// Optional rotation engine for rate-limit-aware provider selection
    rotation: Option<Arc<ProviderRotation>>,
    /// Codex token manager (shared with CodexAwareProvider)
    codex_token_manager: Option<Arc<CodexTokenManager>>,
    /// Codex base URL for model listing / usage queries
    codex_base_url: Option<String>,
    /// Request classifier for smart tiered routing
    classifier: Option<Arc<super::classifier::RequestClassifier>>,
    /// Provider names for simple requests
    simple_providers: Vec<String>,
    /// Provider names for medium requests
    medium_providers: Vec<String>,
    /// Provider names for complex requests
    complex_providers: Vec<String>,
    /// Budget ratio (0.0 - 1.0) for automatic provider downgrade.
    /// Updated externally via set_budget_ratio(). 0.0 = no spend, 1.0 = budget exhausted.
    budget_ratio: std::sync::atomic::AtomicU32,
}

// Default provider configs (used if agents.toml not found or provider missing)
fn default_provider_configs() -> HashMap<String, ProviderConfig> {
    let mut map = HashMap::new();
    map.insert(
        "ollama".to_string(),
        ProviderConfig {
            provider_type: "ollama".to_string(),
            url: Some("http://localhost:11434".to_string()),
            default_model: Some("qwen3:8b".to_string()),
            api_key: None,
        },
    );
    map.insert(
        "lmstudio".to_string(),
        ProviderConfig {
            provider_type: "openai_compat".to_string(),
            url: Some("http://localhost:1234".to_string()),
            default_model: None,
            api_key: None,
        },
    );
    map.insert(
        "lemonade".to_string(),
        ProviderConfig {
            provider_type: "openai_compat".to_string(),
            url: Some("http://localhost:8000/api/v0".to_string()),
            default_model: None,
            api_key: None,
        },
    );
    map
}

/// Resolve an API key: config value → env var → Codex OAuth token → empty.
/// `env_vars` is a list of env var names to check in order.
fn resolve_api_key(config_key: &Option<String>, env_vars: &[&str]) -> String {
    // 1. Explicit config value (non-empty)
    if let Some(ref key) = config_key {
        if !key.is_empty() {
            return key.clone();
        }
    }
    // 2. Environment variable fallback
    for var in env_vars {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                debug!("API key resolved from env var {}", var);
                return val;
            }
        }
    }
    String::new()
}

// resolve_codex_credential() removed — replaced by CodexTokenManager in codex.rs

/// Provider name → env var names for API key resolution
fn env_vars_for_provider(provider_type: &str, name: &str) -> Vec<&'static str> {
    match provider_type {
        "openai" | "openai_codex" => vec!["OPENAI_API_KEY"],
        "anthropic" => vec!["ANTHROPIC_API_KEY"],
        "gemini" => vec!["GEMINI_API_KEY", "GOOGLE_API_KEY"],
        "groq" => vec!["GROQ_API_KEY"],
        _ => match name {
            "deepseek" => vec!["DEEPSEEK_API_KEY"],
            "cerebras" => vec!["CEREBRAS_API_KEY"],
            "together" => vec!["TOGETHER_API_KEY"],
            "openrouter" => vec!["OPENROUTER_API_KEY"],
            _ => vec![],
        },
    }
}

/// Result of creating a provider — includes optional Codex metadata.
struct CreateProviderResult {
    provider: Box<dyn Provider>,
    codex_token_manager: Option<Arc<CodexTokenManager>>,
    codex_base_url: Option<String>,
}

/// Create a boxed Provider from a name and config.
/// API keys are resolved via config → env var → empty (OpenFang/OpenCrust pattern).
fn create_provider(name: &str, config: &ProviderConfig) -> CreateProviderResult {
    let env_vars = env_vars_for_provider(&config.provider_type, name);

    match config.provider_type.as_str() {
        "ollama" => CreateProviderResult {
            provider: Box::new(OllamaProvider::new(
                name.to_string(),
                config.url.clone().unwrap_or_else(|| "http://localhost:11434".to_string()),
                config.default_model.clone().unwrap_or_else(|| "qwen3:8b".to_string()),
            )),
            codex_token_manager: None,
            codex_base_url: None,
        },
        "anthropic" => {
            let api_key = resolve_api_key(&config.api_key, &env_vars);
            CreateProviderResult {
                provider: Box::new(AnthropicProvider::new(
                    name.to_string(),
                    config.url.clone().unwrap_or_else(|| "https://api.anthropic.com".to_string()),
                    config.default_model.clone().unwrap_or_else(|| "claude-sonnet-4-6".to_string()),
                    api_key,
                )),
                codex_token_manager: None,
                codex_base_url: None,
            }
        }
        "openai" => {
            let api_key = resolve_api_key(&config.api_key, &env_vars);
            let model = config.default_model.clone().unwrap_or_else(|| "gpt-4o".to_string());
            CreateProviderResult {
                provider: if let Some(url) = &config.url {
                    Box::new(OpenAiProvider::with_base_url(api_key, model, url.clone()))
                } else {
                    Box::new(OpenAiProvider::new(api_key, model))
                },
                codex_token_manager: None,
                codex_base_url: None,
            }
        }
        "openai_codex" => {
            let model = config.default_model.clone().unwrap_or_else(|| "gpt-4o".to_string());
            let base_url = config.url.clone().unwrap_or_else(|| "https://api.openai.com".to_string());
            let token_manager = Arc::new(CodexTokenManager::new());

            // Check if auth.json exists; if not, fall back to static API key via OpenAiProvider
            if token_manager.read_auth_file_sync().is_some() {
                info!("Codex token manager initialized (auth.json found)");
                CreateProviderResult {
                    provider: Box::new(CodexAwareProvider::new(
                        base_url.clone(),
                        model,
                        Arc::clone(&token_manager),
                    )),
                    codex_token_manager: Some(token_manager),
                    codex_base_url: Some(base_url),
                }
            } else {
                // No auth.json — fall back to static API key
                let api_key = resolve_api_key(&config.api_key, &env_vars);
                info!("Codex: no auth.json found, using static API key");
                CreateProviderResult {
                    provider: Box::new(OpenAiProvider::with_base_url(api_key, model, base_url)),
                    codex_token_manager: None,
                    codex_base_url: None,
                }
            }
        }
        "chatgpt_backend" => {
            let tm = Arc::new(CodexTokenManager::new());
            let has_auth = tm.read_auth_file().is_some();
            if has_auth {
                tracing::info!("chatgpt_backend: OAuth credentials found");
            } else {
                tracing::warn!("chatgpt_backend: no ~/.codex/auth.json found, provider will need login");
            }
            let provider = super::chatgpt_backend::ChatGptBackendProvider::new(tm.clone());
            CreateProviderResult {
                provider: Box::new(provider),
                codex_token_manager: Some(tm),
                codex_base_url: None,
            }
        }
        "chatgpt_ws" => {
            let tm = Arc::new(CodexTokenManager::new());
            let provider = super::chatgpt_ws::ChatGptWsProvider::new(tm.clone());
            CreateProviderResult {
                provider: Box::new(provider),
                codex_token_manager: Some(tm),
                codex_base_url: None,
            }
        }
        "opencode_backend" => {
            let model = config.default_model.clone().unwrap_or_default();
            let provider = if model.is_empty() {
                super::opencode_backend::OpenCodeBackendProvider::new()
            } else {
                super::opencode_backend::OpenCodeBackendProvider::with_model(&model)
            };
            info!("opencode_backend: default_model={}", provider.default_model());
            CreateProviderResult {
                provider: Box::new(provider),
                codex_token_manager: None,
                codex_base_url: None,
            }
        }
        "gemini" => {
            let api_key = resolve_api_key(&config.api_key, &env_vars);
            CreateProviderResult {
                provider: Box::new(GeminiProvider::new(
                    api_key,
                    config.default_model.clone(),
                )),
                codex_token_manager: None,
                codex_base_url: None,
            }
        }
        "groq" => {
            let api_key = resolve_api_key(&config.api_key, &env_vars);
            CreateProviderResult {
                provider: Box::new(GroqProvider::new(
                    api_key,
                    config.default_model.clone(),
                )),
                codex_token_manager: None,
                codex_base_url: None,
            }
        }
        _ => {
            // Default to OpenAI-compat for unknown types
            let api_key = resolve_api_key(&config.api_key, &env_vars);
            let api_key_opt = if api_key.is_empty() { None } else { Some(api_key) };
            CreateProviderResult {
                provider: Box::new(OpenAiCompatProvider::new(
                    name.to_string(),
                    config.url.clone().unwrap_or_else(|| "http://localhost:1234".to_string()),
                    config.default_model.clone().unwrap_or_else(|| "default".to_string()),
                    api_key_opt,
                )),
                codex_token_manager: None,
                codex_base_url: None,
            }
        }
    }
}

impl ProviderRouter {
    /// Create a new ProviderRouter, loading config from the given TOML path.
    /// Falls back to defaults if the file doesn't exist.
    pub fn new(config_path: &str) -> Result<Self> {
        let (provider_configs, routes, smart_routing_config) = if std::path::Path::new(config_path).exists() {
            let content = std::fs::read_to_string(config_path)?;

            // Decrypt enc2: secrets in config before parsing
            let content = {
                let clawtex_dir = config_path
                    .rsplit_once('/')
                    .or_else(|| config_path.rsplit_once('\\'))
                    .map(|(dir, _)| dir.to_string())
                    .unwrap_or_else(|| ".".to_string());
                match crate::SecretManager::new(&clawtex_dir) {
                    Ok(sm) => {
                        let mut s = content;
                        while let Some(start) = s.find("enc2:") {
                            let rest = &s[start..];
                            let end = rest.find('"')
                                .or_else(|| rest.find('\''))
                                .or_else(|| rest.find('\n'))
                                .unwrap_or(rest.len());
                            let enc_val = s[start..start + end].to_string();
                            match sm.decrypt(&enc_val) {
                                Ok(plain) => {
                                    s = format!("{}{}{}", &s[..start], plain, &s[start + end..]);
                                }
                                Err(_) => break,
                            }
                        }
                        s
                    }
                    Err(_) => content,
                }
            };

            let parsed: AgentsToml = toml::from_str(&content)?;
            let configs = if parsed.providers.is_empty() {
                default_provider_configs()
            } else {
                parsed.providers
            };
            let routes: HashMap<String, (String, Option<String>)> = parsed.routing
                .into_iter()
                .map(|r| (r.hint, (r.provider, r.model)))
                .collect();
            (configs, routes, parsed.smart_routing)
        } else {
            warn!("Config not found at {}, using defaults", config_path);
            (default_provider_configs(), HashMap::new(), None)
        };

        if !routes.is_empty() {
            info!("Loaded {} route hints: {:?}", routes.len(), routes.keys().collect::<Vec<_>>());
        }

        // Build auto-routing order: ollama first, then alphabetical
        let mut auto_order: Vec<String> = provider_configs.keys().cloned().collect();
        auto_order.sort();
        // Move "ollama" to front if present
        if let Some(pos) = auto_order.iter().position(|n| n == "ollama") {
            auto_order.remove(pos);
            auto_order.insert(0, "ollama".to_string());
        }

        // Create provider instances
        let mut providers: HashMap<String, Box<dyn Provider>> = HashMap::new();
        let mut codex_token_manager: Option<Arc<CodexTokenManager>> = None;
        let mut codex_base_url: Option<String> = None;

        for (name, config) in &provider_configs {
            let result = create_provider(name, config);
            providers.insert(name.clone(), result.provider);
            if result.codex_token_manager.is_some() {
                codex_token_manager = result.codex_token_manager;
                codex_base_url = result.codex_base_url;
            }
        }

        info!("Initialized {} providers: {:?}", providers.len(), providers.keys().collect::<Vec<_>>());

        // Parse smart routing tiers from config
        let mut smart_simple = Vec::new();
        let mut smart_medium = Vec::new();
        let mut smart_complex = Vec::new();
        if let Some(ref sr) = smart_routing_config {
            smart_simple = sr.simple_providers.clone();
            smart_medium = sr.medium_providers.clone();
            smart_complex = sr.complex_providers.clone();
            tracing::info!("Smart routing configured: simple={:?}, medium={:?}, complex={:?}",
                sr.simple_providers, sr.medium_providers, sr.complex_providers);
        }

        Ok(Self {
            providers,
            routes,
            auto_order,
            rotation: None,
            codex_token_manager,
            codex_base_url,
            classifier: None,  // Set via set_classifier() after construction
            simple_providers: smart_simple,
            medium_providers: smart_medium,
            complex_providers: smart_complex,
            budget_ratio: std::sync::atomic::AtomicU32::new(0),
        })
    }

    /// Attach a rotation engine for rate-limit-aware provider selection.
    pub fn set_rotation(&mut self, rotation: Arc<ProviderRotation>) {
        self.rotation = Some(rotation);
    }

    /// Get rotation engine reference (if attached).
    pub fn rotation(&self) -> Option<&Arc<ProviderRotation>> {
        self.rotation.as_ref()
    }

    /// Attach a request classifier for smart tiered routing.
    pub fn set_classifier(&mut self, classifier: Arc<super::classifier::RequestClassifier>) {
        self.classifier = Some(classifier);
    }

    /// Set the provider tiers for smart routing.
    pub fn set_tiers(&mut self, simple: Vec<String>, medium: Vec<String>, complex: Vec<String>) {
        self.simple_providers = simple;
        self.medium_providers = medium;
        self.complex_providers = complex;
    }

    /// Check if smart routing is configured (classifier + at least one tier).
    pub fn has_smart_routing(&self) -> bool {
        self.classifier.is_some() && (!self.simple_providers.is_empty()
            || !self.medium_providers.is_empty()
            || !self.complex_providers.is_empty())
    }

    /// Update the budget usage ratio (0.0 = no spend, 1.0 = fully spent).
    /// When ratio >= 0.5, routing prefers medium-tier providers.
    /// When ratio >= 0.8, routing prefers local/cheap providers only.
    pub fn set_budget_ratio(&self, ratio: f32) {
        let clamped = ratio.clamp(0.0, 1.0);
        self.budget_ratio.store(
            clamped.to_bits(),
            std::sync::atomic::Ordering::Relaxed,
        );
        if clamped >= 0.8 {
            info!("Budget at {:.0}% — routing to local providers only", clamped * 100.0);
        } else if clamped >= 0.5 {
            info!("Budget at {:.0}% — routing to medium-tier providers", clamped * 100.0);
        }
    }

    /// Get current budget ratio
    pub fn budget_ratio(&self) -> f32 {
        f32::from_bits(self.budget_ratio.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Get the budget-adjusted provider tier.
    /// Returns which tier to prefer based on budget pressure.
    pub fn budget_tier(&self) -> &str {
        let ratio = self.budget_ratio();
        if ratio >= 0.8 {
            "local"  // L3: use only local providers (ollama, lmstudio)
        } else if ratio >= 0.5 {
            "medium"  // L2: prefer medium-cost providers
        } else {
            "full"  // L1: all providers available
        }
    }

    /// Check if a provider with the given name exists
    pub fn has_provider(&self, name: &str) -> bool {
        self.providers.contains_key(name)
    }

    /// Get all provider names
    pub fn provider_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.providers.keys().cloned().collect();
        names.sort();
        names
    }

    /// Get a provider by name
    pub fn get_provider(&self, name: &str) -> Option<&dyn Provider> {
        self.providers.get(name).map(|p| p.as_ref())
    }

    /// Resolve a provider string that may contain a hint prefix.
    /// "hint:reasoning" → resolves to the configured provider+model for "reasoning"
    /// "ollama" → passes through unchanged
    fn resolve_hint(&self, provider: &str) -> (String, Option<String>) {
        if let Some(hint_name) = provider.strip_prefix("hint:") {
            if let Some((target_provider, model_override)) = self.routes.get(hint_name) {
                debug!("Resolved hint '{}' → provider='{}', model={:?}", hint_name, target_provider, model_override);
                return (target_provider.clone(), model_override.clone());
            }
            warn!("Unknown route hint '{}', falling back to auto", hint_name);
            return ("auto".to_string(), None);
        }
        (provider.to_string(), None)
    }

    /// Find the first alive provider (auto-routing), skipping those in cooldown.
    /// Respects budget_ratio: at >=80% prefer local, at >=50% prefer medium tier.
    async fn find_alive_provider(&self) -> Option<&dyn Provider> {
        let tier = self.budget_tier();
        let local_names: Vec<&str> = vec!["ollama", "lmstudio"];

        // Budget L3: try local-only first
        if tier == "local" {
            for name in &self.auto_order {
                if !local_names.contains(&name.as_str()) {
                    continue;
                }
                if let Some(ref rotation) = self.rotation {
                    if rotation.is_cooling_down(name) { continue; }
                }
                if let Some(p) = self.providers.get(name) {
                    if p.is_alive().await {
                        debug!("Auto-route (budget L3 local): using '{}'", name);
                        return Some(p.as_ref());
                    }
                }
            }
            // Fall through to any provider if no local available
        }

        // Budget L2: try medium tier first (if configured)
        if tier == "medium" && !self.medium_providers.is_empty() {
            for name in &self.medium_providers {
                if let Some(ref rotation) = self.rotation {
                    if rotation.is_cooling_down(name) { continue; }
                }
                if let Some(p) = self.providers.get(name) {
                    if p.is_alive().await {
                        debug!("Auto-route (budget L2 medium): using '{}'", name);
                        return Some(p.as_ref());
                    }
                }
            }
        }

        // Normal fallback: try all providers in order
        for name in &self.auto_order {
            if let Some(ref rotation) = self.rotation {
                if rotation.is_cooling_down(name) {
                    debug!("Auto-route: skipping '{}' (cooling down)", name);
                    continue;
                }
            }
            if let Some(p) = self.providers.get(name) {
                if p.is_alive().await {
                    debug!("Auto-route: using provider '{}'", name);
                    return Some(p.as_ref());
                }
            }
        }
        None
    }

    /// Route a simple prompt to a provider (no tool calling)
    pub async fn route(&self, prompt: &str, provider: &str) -> Result<String> {
        let (resolved, model_override) = self.resolve_hint(provider);

        let (provider_ref, model) = if resolved == "auto" {
            let p = self.find_alive_provider().await
                .ok_or_else(|| anyhow!("No LLM provider available"))?;
            (p, model_override.unwrap_or_default())
        } else {
            let p = self.providers.get(&resolved)
                .ok_or_else(|| anyhow!("Unknown provider: {}", resolved))?
                .as_ref();
            (p, model_override.unwrap_or_default())
        };

        let messages = vec![ChatMessage {
            role: "user".into(),
            content: prompt.to_string(),
            tool_calls: None,
            tool_call_id: None,
        }];

        let resp = provider_ref.chat(&messages, &[], &model).await?;
        Ok(resp.message.content)
    }

    /// Chat with tools support.
    /// Supports "hint:<name>" for route-based dispatch with model override.
    /// On rate-limit errors, records to rotation and retries with next available provider.
    pub async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        provider: &str,
    ) -> Result<ChatResponse> {
        let (resolved, model_override) = self.resolve_hint(provider);

        // Smart tiered routing: if hint is "auto" or empty and classifier is configured
        if (resolved.is_empty() || resolved == "auto") && self.classifier.is_some() {
            let classifier = self.classifier.as_ref().unwrap();
            let complexity = classifier.classify(messages).await;

            let tier_candidates = match complexity {
                super::classifier::RequestComplexity::Simple => &self.simple_providers,
                super::classifier::RequestComplexity::Medium => &self.medium_providers,
                super::classifier::RequestComplexity::Complex => &self.complex_providers,
            };

            if !tier_candidates.is_empty() {
                tracing::debug!("Classified as {:?}, candidates: {:?}", complexity, tier_candidates);

                for candidate in tier_candidates {
                    if let Some(p) = self.providers.get(candidate) {
                        let model_to_use = p.default_model().to_string();
                        match p.chat(messages, tools, &model_to_use).await {
                            Ok(resp) => {
                                if let Some(ref rot) = self.rotation {
                                    rot.record_success(candidate);
                                }
                                return Ok(resp);
                            }
                            Err(e) => {
                                tracing::warn!("Tier provider {} failed: {}, trying next", candidate, e);
                                if let Some(ref rot) = self.rotation {
                                    rot.record_rate_limit(candidate);  // sync, no .await
                                }
                                continue;
                            }
                        }
                    }
                }
                tracing::warn!("All tier candidates failed, falling back to normal routing");
            }
        }

        let (provider_ref, model) = if resolved == "auto" {
            let p = self.find_alive_provider().await
                .ok_or_else(|| anyhow!("No LLM provider available"))?;
            (p, model_override.clone().unwrap_or_default())
        } else {
            let p = self.providers.get(&resolved)
                .ok_or_else(|| anyhow!("Unknown provider: {}", resolved))?
                .as_ref();
            (p, model_override.clone().unwrap_or_default())
        };

        match provider_ref.chat(messages, tools, &model).await {
            Ok(resp) => {
                if let Some(ref rotation) = self.rotation {
                    rotation.record_success(provider_ref.name());
                }
                Ok(resp)
            }
            Err(e) => {
                // Check if this is a rate-limit error and try rotation
                if let Some(ref rotation) = self.rotation {
                    let error_class = classify_error(&e);
                    if error_class == ErrorClass::RateLimited {
                        rotation.record_rate_limit(provider_ref.name());

                        // Try ALL available providers in order (not just one)
                        let candidates: Vec<String> = self.auto_order.clone();
                        let mut last_err = e;
                        for candidate_name in &candidates {
                            if candidate_name == provider_ref.name() { continue; }
                            if rotation.is_cooling_down(candidate_name) { continue; }
                            if let Some(fallback) = self.providers.get(candidate_name) {
                                let fb_model = fallback.default_model().to_string();
                                info!(
                                    "Rotation: '{}' rate-limited, trying '{}'",
                                    provider_ref.name(), candidate_name
                                );
                                match fallback.chat(messages, tools, &fb_model).await {
                                    Ok(resp) => {
                                        rotation.record_success(candidate_name);
                                        return Ok(resp);
                                    }
                                    Err(e2) => {
                                        let e2_class = classify_error(&e2);
                                        if e2_class == ErrorClass::RateLimited {
                                            rotation.record_rate_limit(candidate_name);
                                        }
                                        warn!(
                                            "Rotation: fallback '{}' also failed ({:?}): {}",
                                            candidate_name, e2_class, e2
                                        );
                                        last_err = e2;
                                        continue; // Try next provider
                                    }
                                }
                            }
                        }
                        return Err(last_err);
                    }
                }
                Err(e)
            }
        }
    }

    /// Streaming chat — returns a stream of chunks.
    pub async fn stream_chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        provider: &str,
    ) -> Result<Pin<Box<dyn futures_util::Stream<Item = Result<StreamChunk>> + Send>>> {
        let (resolved, model_override) = self.resolve_hint(provider);

        let (provider_ref, model) = if resolved == "auto" {
            let p = self.find_alive_provider().await
                .ok_or_else(|| anyhow!("No LLM provider available"))?;
            (p, model_override.unwrap_or_default())
        } else {
            let p = self.providers.get(&resolved)
                .ok_or_else(|| anyhow!("Unknown provider: {}", resolved))?
                .as_ref();
            (p, model_override.unwrap_or_default())
        };

        provider_ref.stream_chat(messages, tools, &model).await
    }

    /// Check if a named provider is alive
    pub async fn is_alive(&self, name: &str) -> bool {
        if let Some(p) = self.providers.get(name) {
            p.is_alive().await
        } else {
            false
        }
    }

    /// Check if any provider is alive
    pub async fn any_alive(&self) -> bool {
        self.find_alive_provider().await.is_some()
    }

    /// Get Codex usage snapshot (if a Codex provider is configured with OAuth).
    pub async fn codex_usage(&self) -> Option<CodexUsageSnapshot> {
        let tm = self.codex_token_manager.as_ref()?;
        let cred = tm.get_credential().await.ok()?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .ok()?;
        super::codex::fetch_codex_usage(
            &client,
            &cred.access_token,
            cred.account_id.as_deref(),
        ).await.ok()
    }

    /// List models available via the Codex provider (if configured with OAuth).
    pub async fn list_codex_models(&self) -> Option<Vec<ModelInfo>> {
        let tm = self.codex_token_manager.as_ref()?;
        let base_url = self.codex_base_url.as_ref()?;
        let token = tm.get_token().await.ok()?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .ok()?;
        let cache = super::codex::ModelListCache::new(client);
        cache.list_models(base_url, &token).await.ok()
    }

    /// Access the Codex token manager (if available).
    pub fn codex_token_manager(&self) -> Option<&Arc<CodexTokenManager>> {
        self.codex_token_manager.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_providers_loaded() {
        let router = ProviderRouter::new("/nonexistent/path.toml").unwrap();
        assert!(router.has_provider("ollama"));
        assert!(router.has_provider("lmstudio"));
        assert!(router.has_provider("lemonade"));
    }

    #[test]
    fn test_provider_names() {
        let router = ProviderRouter::new("/nonexistent/path.toml").unwrap();
        let names = router.provider_names();
        assert!(names.contains(&"ollama".to_string()));
        assert!(names.contains(&"lmstudio".to_string()));
        assert!(names.contains(&"lemonade".to_string()));
    }

    #[test]
    fn test_auto_order_ollama_first() {
        let router = ProviderRouter::new("/nonexistent/path.toml").unwrap();
        assert_eq!(router.auto_order[0], "ollama");
    }

    #[test]
    fn test_unknown_provider() {
        let router = ProviderRouter::new("/nonexistent/path.toml").unwrap();
        assert!(!router.has_provider("nonexistent"));
    }

    #[test]
    fn test_resolve_hint_passthrough() {
        let router = ProviderRouter::new("/nonexistent/path.toml").unwrap();
        let (name, model) = router.resolve_hint("ollama");
        assert_eq!(name, "ollama");
        assert!(model.is_none());
    }

    #[test]
    fn test_resolve_hint_unknown_falls_back_to_auto() {
        let router = ProviderRouter::new("/nonexistent/path.toml").unwrap();
        let (name, _) = router.resolve_hint("hint:nonexistent");
        assert_eq!(name, "auto");
    }

    #[test]
    fn test_get_provider() {
        let router = ProviderRouter::new("/nonexistent/path.toml").unwrap();
        let p = router.get_provider("ollama").unwrap();
        assert_eq!(p.name(), "ollama");
        assert_eq!(p.default_model(), "qwen3:8b");
    }

    #[test]
    fn test_create_ollama_provider() {
        let config = ProviderConfig {
            provider_type: "ollama".into(),
            url: Some("http://localhost:11434".into()),
            default_model: Some("llama3:8b".into()),
            api_key: None,
        };
        let result = create_provider("test-ollama", &config);
        assert_eq!(result.provider.name(), "test-ollama");
        assert_eq!(result.provider.default_model(), "llama3:8b");
        assert!(result.codex_token_manager.is_none());
    }

    #[test]
    fn test_create_anthropic_provider() {
        let config = ProviderConfig {
            provider_type: "anthropic".into(),
            url: None,
            default_model: Some("claude-sonnet-4-6".into()),
            api_key: Some("sk-test".into()),
        };
        let result = create_provider("test-anthropic", &config);
        assert_eq!(result.provider.name(), "test-anthropic");
        assert_eq!(result.provider.default_model(), "claude-sonnet-4-6");
    }

    #[test]
    fn test_create_openai_provider() {
        let config = ProviderConfig {
            provider_type: "openai".into(),
            url: None,
            default_model: Some("gpt-4o".into()),
            api_key: Some("sk-test".into()),
        };
        let result = create_provider("test-openai", &config);
        assert_eq!(result.provider.name(), "openai"); // OpenAiProvider always returns "openai"
        assert_eq!(result.provider.default_model(), "gpt-4o");
    }

    #[test]
    fn test_create_openai_compat_provider() {
        let config = ProviderConfig {
            provider_type: "openai_compat".into(),
            url: Some("http://localhost:1234".into()),
            default_model: Some("local-model".into()),
            api_key: None,
        };
        let result = create_provider("lmstudio", &config);
        assert_eq!(result.provider.name(), "lmstudio");
    }

    #[test]
    fn test_default_provider_configs() {
        let configs = default_provider_configs();
        assert!(configs.contains_key("ollama"));
        assert!(configs.contains_key("lmstudio"));
        assert!(configs.contains_key("lemonade"));
        assert_eq!(configs["ollama"].provider_type, "ollama");
        assert_eq!(configs["lmstudio"].provider_type, "openai_compat");
    }

    // ── resolve_api_key tests ──────────────────────────────────────────

    #[test]
    fn test_resolve_api_key_from_config() {
        let result = resolve_api_key(&Some("sk-from-config".into()), &["NONEXISTENT_VAR_12345"]);
        assert_eq!(result, "sk-from-config");
    }

    #[test]
    fn test_resolve_api_key_empty_config_falls_through() {
        let result = resolve_api_key(&Some("".into()), &[]);
        assert_eq!(result, "");
    }

    #[test]
    fn test_resolve_api_key_none_config_falls_through() {
        let result = resolve_api_key(&None, &[]);
        assert_eq!(result, "");
    }

    #[test]
    fn test_resolve_api_key_env_var_fallback() {
        // PATH always exists on all platforms
        let result = resolve_api_key(&None, &["PATH"]);
        assert!(!result.is_empty());
    }

    #[test]
    fn test_resolve_api_key_config_wins_over_env() {
        let result = resolve_api_key(&Some("config-key".into()), &["PATH"]);
        assert_eq!(result, "config-key");
    }

    // ── env_vars_for_provider tests ────────────────────────────────────

    #[test]
    fn test_env_vars_openai() {
        let vars = env_vars_for_provider("openai", "openai");
        assert_eq!(vars, vec!["OPENAI_API_KEY"]);
    }

    #[test]
    fn test_env_vars_anthropic() {
        let vars = env_vars_for_provider("anthropic", "anthropic");
        assert_eq!(vars, vec!["ANTHROPIC_API_KEY"]);
    }

    #[test]
    fn test_env_vars_gemini() {
        let vars = env_vars_for_provider("gemini", "gemini");
        assert_eq!(vars, vec!["GEMINI_API_KEY", "GOOGLE_API_KEY"]);
    }

    #[test]
    fn test_env_vars_groq() {
        let vars = env_vars_for_provider("groq", "groq");
        assert_eq!(vars, vec!["GROQ_API_KEY"]);
    }

    #[test]
    fn test_env_vars_codex() {
        let vars = env_vars_for_provider("openai_codex", "codex");
        assert_eq!(vars, vec!["OPENAI_API_KEY"]);
    }

    #[test]
    fn test_env_vars_name_based_deepseek() {
        let vars = env_vars_for_provider("openai_compat", "deepseek");
        assert_eq!(vars, vec!["DEEPSEEK_API_KEY"]);
    }

    #[test]
    fn test_env_vars_unknown() {
        let vars = env_vars_for_provider("openai_compat", "custom");
        assert!(vars.is_empty());
    }

    // ── Codex provider tests ─────────────────────────────────────────────

    #[test]
    fn test_create_openai_codex_creates_codex_aware_or_fallback() {
        let config = ProviderConfig {
            provider_type: "openai_codex".into(),
            url: None,
            default_model: Some("gpt-4o".into()),
            api_key: Some("sk-test-codex".into()),
        };
        let result = create_provider("codex", &config);
        // Without auth.json, falls back to OpenAiProvider ("openai")
        // With auth.json, creates CodexAwareProvider ("codex")
        let name = result.provider.name();
        assert!(name == "openai" || name == "codex");
        assert_eq!(result.provider.default_model(), "gpt-4o");
    }

    #[test]
    fn test_router_codex_fields_none_by_default() {
        let router = ProviderRouter::new("/nonexistent/path.toml").unwrap();
        assert!(router.codex_token_manager.is_none());
        assert!(router.codex_base_url.is_none());
    }
}
