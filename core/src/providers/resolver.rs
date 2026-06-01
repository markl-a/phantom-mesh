//! `DefaultProviderResolver` — Phase 2 of the DEMO-1 gap 1 plan.
//!
//! Adds the resolver shim + 4 internal `LlmProvider` impls
//! (`AnthropicProvider`, `OpenAICompatProvider`, `GeminiProvider`,
//! `ClaudeCliProvider`). Each impl wraps the corresponding branch of the
//! `provider_url` string-switch in `agent.rs` so behaviour is byte-identical
//! with the legacy dispatch path. **No call-site changes** — `agent.rs` and
//! `streaming.rs` keep using their existing string-switch; Phase 3+ migrate
//! the call sites onto the trait.
//!
//! See `docs/superpowers/specs/2026-05-17-demo1-gap1-llmprovider-design.md`
//! for the 5-phase plan.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::config::{AgentsConfig, ProviderEntry};
use crate::providers::llm_provider::{
    default_openai_compat_body, BuildRequestOpts, BuildRequestParts, LlmProvider,
};
use crate::providers::traits::{classify_error, ChatMessage, ProviderError};

// ── Phase 4 helpers (shared body-shaping) ─────────────────────────────────

/// `true` for Claude Opus 4.7+ — gated `thinking.display = "omitted"`
/// support. Mirrors `streaming.rs::model_supports_thinking_display_omitted`
/// exactly; older Claude models 400 on `display`.
fn model_supports_thinking_display_omitted(model: &str) -> bool {
    let m = model.to_lowercase();
    m.starts_with("claude-opus-4-7")
        || m.starts_with("claude-opus-4-8")
        || m.starts_with("claude-opus-4-9")
        || m.starts_with("claude-opus-5")
}

/// Anthropic native `/v1/messages` URL builder. Strips any
/// chat/completions / messages suffix so explicit `provider.url`
/// pointing at the legacy OpenAI-compat proxy still resolves correctly.
fn anthropic_messages_url(explicit: Option<&str>) -> String {
    if let Some(explicit) = explicit {
        let base = explicit.trim_end_matches('/');
        if base.ends_with("/v1/messages") {
            return base.to_string();
        }
        let base = base
            .trim_end_matches("/v1/chat/completions")
            .trim_end_matches('/');
        return format!("{}/v1/messages", base);
    }
    "https://api.anthropic.com/v1/messages".into()
}

/// Build the native Anthropic Messages-API body with cache_control on
/// system + last tool, adaptive `thinking.display = "omitted"` for Opus
/// 4.7+, and multimodal `content_block` conversion. Mirrors
/// `streaming.rs::build_request_body` (anthropic branch) byte-for-byte so
/// any future tweak there must land here too.
fn build_anthropic_body(opts: &BuildRequestOpts<'_>) -> serde_json::Value {
    use serde_json::Value;

    let user_messages: Vec<Value> = opts
        .messages
        .iter()
        .filter(|m| m["role"].as_str() != Some("system"))
        .map(crate::multimodal::convert_message_for_anthropic)
        .collect();

    let mut body = serde_json::json!({
        "model": opts.model,
        "max_tokens": opts.max_tokens,
        "messages": user_messages,
    });
    if opts.stream {
        body["stream"] = Value::Bool(true);
    }

    // [F1] Auto prompt caching (GA 2026-02-19): system as a single
    // cacheable text block.
    if !opts.system.is_empty() {
        body["system"] = serde_json::json!([{
            "type": "text",
            "text": opts.system,
            "cache_control": {"type": "ephemeral"},
        }]);
    }

    // Anthropic tool schema + cache_control breakpoint on the LAST tool
    // so tools + system are cached together (render order: tools → system).
    if !opts.tools.is_empty() {
        let last_idx = opts.tools.len() - 1;
        let anthropic_tools: Vec<Value> = opts
            .tools
            .iter()
            .enumerate()
            .map(|(i, td)| {
                let mut tool = serde_json::json!({
                    "name": td["function"]["name"],
                    "description": td["function"]["description"],
                    "input_schema": td["function"]["parameters"],
                });
                if i == last_idx {
                    tool["cache_control"] = serde_json::json!({"type": "ephemeral"});
                }
                tool
            })
            .collect();
        body["tools"] = Value::Array(anthropic_tools);
    }

    // [F1] thinking.display "omitted" (Anthropic 2026-03-16): gate
    // strictly on the model prefix — older models 400 on `display`.
    if model_supports_thinking_display_omitted(opts.model) {
        body["thinking"] = serde_json::json!({
            "type": "adaptive",
            "display": "omitted",
        });
    }
    body
}

/// Anthropic auth + version headers (caller adds `x-api-key` from its
/// own api_key source after `build_stream_request` returns).
fn anthropic_headers() -> Vec<(&'static str, String)> {
    vec![
        ("anthropic-version", "2023-06-01".into()),
        ("content-type", "application/json".into()),
    ]
}

// ── Resolver ──────────────────────────────────────────────────────────────

/// Holds the provider blocks from `[providers.*]` and hands back a trait
/// object for the requested provider name.
///
/// The spec called this `from_config(cfg: &AgentConfig)`; the actual type
/// in this crate is `AgentsConfig` (plural) — same data, named differently.
pub struct DefaultProviderResolver {
    providers: HashMap<String, ProviderEntry>,
}

impl DefaultProviderResolver {
    /// Snapshot the `[providers.*]` table from an `AgentsConfig`.
    pub fn from_config(cfg: &AgentsConfig) -> Self {
        Self {
            providers: cfg.providers.clone(),
        }
    }

    /// Return the trait object for `name` if it exists in the config.
    ///
    /// Dispatch rule: if the entry's `provider_type` is empty (legacy configs
    /// keyed only by name), fall back to matching on `name` itself — same as
    /// `agent.rs::provider_url`.
    pub fn resolve(&self, name: &str) -> Option<Arc<dyn LlmProvider>> {
        let entry = self.providers.get(name)?;
        let key = if entry.provider_type.is_empty() {
            name
        } else {
            entry.provider_type.as_str()
        };
        Some(build_provider(key, entry))
    }
}

fn build_provider(key: &str, entry: &ProviderEntry) -> Arc<dyn LlmProvider> {
    match key {
        "anthropic" => Arc::new(AnthropicProvider::new(entry.url.clone())),
        "gemini" => Arc::new(GeminiProvider::new(entry.url.clone())),
        "claude_cli" => Arc::new(ClaudeCliProvider::new(entry.url.clone())),
        // openai / openai_compat / groq / opencode / openrouter / cerebras /
        // deepseek and any unknown type all share the OpenAI-compat
        // `chat/completions` wire format — same fallthrough as the
        // string-switch in `agent.rs::provider_url`.
        _ => Arc::new(OpenAICompatProvider::new(
            key.to_string(),
            entry.url.clone(),
        )),
    }
}

// ── Shared URL helpers ────────────────────────────────────────────────────

/// Apply the same OpenAI-compat URL normalisation as
/// `agent.rs::provider_url`: honour explicit `url` (don't double-add `/v1`),
/// otherwise build from a default base.
fn openai_compat_url(explicit: Option<&str>, default_base: &str) -> String {
    let base = explicit.unwrap_or(default_base).trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        return base.to_string();
    }
    let already_versioned =
        base.ends_with("/v1") || base.contains("/v1/") || base.contains("/v1beta");
    if already_versioned {
        return format!("{}/chat/completions", base);
    }
    format!("{}/v1/chat/completions", base)
}

/// Build a minimal OpenAI-compat body — `stream` is set per caller.
fn build_body(
    model: &str,
    messages: &[ChatMessage],
    tools: &[serde_json::Value],
    stream: bool,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": stream,
    });
    if !tools.is_empty() {
        body["tools"] = serde_json::Value::Array(tools.to_vec());
        body["tool_choice"] = serde_json::Value::String("auto".into());
    }
    body
}

/// Extract the OpenAI-compat assistant message from a one-shot response JSON.
fn extract_openai_message(json: &serde_json::Value) -> ChatMessage {
    let msg = &json["choices"][0]["message"];
    ChatMessage {
        role: msg["role"].as_str().unwrap_or("assistant").into(),
        content: msg["content"].as_str().unwrap_or("").into(),
        tool_calls: msg.get("tool_calls").cloned(),
    }
}

async fn send_openai_compat(
    url: &str,
    api_key: &str,
    body: &serde_json::Value,
) -> Result<reqwest::Response, ProviderError> {
    reqwest::Client::new()
        .post(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| classify_error(0, &e.to_string()))
}

async fn send_anthropic(
    url: &str,
    api_key: &str,
    body: &serde_json::Value,
) -> Result<reqwest::Response, ProviderError> {
    reqwest::Client::new()
        .post(url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| classify_error(0, &e.to_string()))
}

async fn one_shot_openai_compat(
    url: &str,
    api_key: &str,
    model: &str,
    messages: &[ChatMessage],
    tools: &[serde_json::Value],
) -> Result<(ChatMessage, serde_json::Value), ProviderError> {
    let body = build_body(model, messages, tools, false);
    let resp = send_openai_compat(url, api_key, &body).await?;
    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| classify_error(0, &e.to_string()))?;
    if !(200..300).contains(&status) {
        return Err(classify_error(status, &text));
    }
    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| ProviderError::Unknown(format!("invalid JSON: {}", e)))?;
    let msg = extract_openai_message(&json);
    Ok((msg, json))
}

// ── Provider impls ────────────────────────────────────────────────────────

pub(crate) struct AnthropicProvider {
    base_url: Option<String>,
}
impl AnthropicProvider {
    pub fn new(base_url: Option<String>) -> Self {
        Self { base_url }
    }
    fn url(&self) -> String {
        // agent.rs uses /v1/chat/completions for anthropic (OpenAI-compat
        // proxy). We mirror that exactly so dispatch behaviour is identical.
        openai_compat_url(self.base_url.as_deref(), "https://api.anthropic.com")
    }
}
#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn stream(
        &self,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<reqwest::Response, ProviderError> {
        let body = build_body(model, messages, tools, true);
        send_anthropic(&self.url(), api_key, &body).await
    }
    async fn complete(
        &self,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<(ChatMessage, serde_json::Value), ProviderError> {
        // Use anthropic auth + OpenAI-compat response shape (matches the
        // /v1/chat/completions proxy path used by agent.rs).
        let body = build_body(model, messages, tools, false);
        let resp = send_anthropic(&self.url(), api_key, &body).await?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| classify_error(0, &e.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(classify_error(status, &text));
        }
        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Unknown(format!("invalid JSON: {}", e)))?;
        Ok((extract_openai_message(&json), json))
    }
    fn provider_type(&self) -> &'static str {
        "anthropic"
    }

    /// Phase 4: native Anthropic Messages-API request — preserves
    /// cache_control on system + last tool, adaptive `thinking.display`
    /// for Opus 4.7+, and multimodal `content_block` conversion. Mirrors
    /// the streaming.rs anthropic branch byte-for-byte.
    fn build_stream_request(
        &self,
        opts: &BuildRequestOpts<'_>,
    ) -> Result<BuildRequestParts, ProviderError> {
        let url = anthropic_messages_url(opts.base_url_override.or(self.base_url.as_deref()));
        let body = build_anthropic_body(opts);
        Ok(BuildRequestParts {
            url,
            body,
            headers: anthropic_headers(),
        })
    }
}

pub(crate) struct OpenAICompatProvider {
    type_id: String,
    base_url: Option<String>,
}
impl OpenAICompatProvider {
    pub fn new(type_id: String, base_url: Option<String>) -> Self {
        Self { type_id, base_url }
    }
    fn url(&self) -> String {
        // Mirror `agent.rs::provider_url` defaults for every OpenAI-compat
        // provider type the legacy switch handles.
        let default = match self.type_id.as_str() {
            "openai" | "openai_compat" => "https://api.openai.com",
            "groq" => "https://api.groq.com/openai",
            "opencode" => "https://opencode.ai/zen",
            "openrouter" => "https://openrouter.ai/api",
            "cerebras" => "https://api.cerebras.ai",
            "deepseek" => "https://api.deepseek.com",
            // V1 (2026-05-21): canonical native endpoints per provider.
            // Before these arms, agents with `provider = "mistral"` (etc.)
            // and a valid MISTRAL_API_KEY were hitting openrouter.ai and
            // 401-ing because the Bearer token isn't valid there. URLs
            // mirror each `providers/<name>.rs::DEFAULT_BASE_URL`; the
            // `openai_compat_url` helper below appends `/v1/chat/completions`
            // when no path suffix is present (and is a no-op if the URL
            // already ends in `/chat/completions`).
            "mistral" => "https://api.mistral.ai",
            "together" => "https://api.together.xyz",
            "nvidia" => "https://integrate.api.nvidia.com",
            "fireworks" => "https://api.fireworks.ai/inference",
            "xai" => "https://api.x.ai",
            "ai21" => "https://api.ai21.com/studio",
            // Perplexity's endpoint is bare `/chat/completions` (no /v1/).
            // Pre-formatted full URL so openai_compat_url returns it as-is.
            "perplexity" => "https://api.perplexity.ai/chat/completions",
            // Cohere's OpenAI-compat endpoint is at /compatibility/v1/...
            // (the bare /v1/chat path is Cohere's NATIVE chat API with a
            // different body shape, so OpenAICompatProvider must NOT use it).
            "cohere" => "https://api.cohere.com/compatibility",
            // Unknown type — keep the historical openrouter fallback so
            // existing operator configs don't silently break.
            _ => "https://openrouter.ai/api",
        };
        openai_compat_url(self.base_url.as_deref(), default)
    }
}
#[async_trait]
impl LlmProvider for OpenAICompatProvider {
    async fn stream(
        &self,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<reqwest::Response, ProviderError> {
        let body = build_body(model, messages, tools, true);
        send_openai_compat(&self.url(), api_key, &body).await
    }
    async fn complete(
        &self,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<(ChatMessage, serde_json::Value), ProviderError> {
        one_shot_openai_compat(&self.url(), api_key, model, messages, tools).await
    }
    fn provider_type(&self) -> &'static str {
        "openai"
    }

    /// Phase 4: OpenAI-compat — honours the stored `base_url` (per-type
    /// defaults wrapped in `OpenAICompatProvider::url`). Body shape is the
    /// shared `default_openai_compat_body` helper (flat `messages` array,
    /// `stream_options.include_usage` when streaming with tools).
    fn build_stream_request(
        &self,
        opts: &BuildRequestOpts<'_>,
    ) -> Result<BuildRequestParts, ProviderError> {
        let url = match opts.base_url_override {
            Some(o) => openai_compat_url(Some(o), "https://api.openai.com"),
            None => self.url(),
        };
        Ok(BuildRequestParts {
            url,
            body: default_openai_compat_body(opts),
            headers: vec![("content-type", "application/json".into())],
        })
    }
}

pub(crate) struct GeminiProvider {
    base_url: Option<String>,
}
impl GeminiProvider {
    pub fn new(base_url: Option<String>) -> Self {
        Self { base_url }
    }
    fn url(&self) -> String {
        // agent.rs uses Gemini's OpenAI-compat endpoint at
        // /v1beta/openai/chat/completions. openai_compat_url already
        // detects `/v1beta` and skips double-adding `/v1`.
        openai_compat_url(
            self.base_url.as_deref(),
            "https://generativelanguage.googleapis.com/v1beta/openai",
        )
    }
}
#[async_trait]
impl LlmProvider for GeminiProvider {
    async fn stream(
        &self,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<reqwest::Response, ProviderError> {
        let body = build_body(model, messages, tools, true);
        send_openai_compat(&self.url(), api_key, &body).await
    }
    async fn complete(
        &self,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<(ChatMessage, serde_json::Value), ProviderError> {
        one_shot_openai_compat(&self.url(), api_key, model, messages, tools).await
    }
    fn provider_type(&self) -> &'static str {
        "gemini"
    }

    /// Phase 4: Gemini — routed through Google's OpenAI-compat shim at
    /// `/v1beta/openai/chat/completions` (NOT the native
    /// `:streamGenerateContent` endpoint). This mirrors what
    /// `agent.rs::provider_url` did before Phase 4 and what
    /// `streaming.rs::streaming_url_openai` still does for `gemini` —
    /// switching to native `contents`/`parts` would be a behaviour change
    /// outside Phase 4's scope (see PR body: "design gap surfaced").
    fn build_stream_request(
        &self,
        opts: &BuildRequestOpts<'_>,
    ) -> Result<BuildRequestParts, ProviderError> {
        let url = match opts.base_url_override {
            Some(o) => openai_compat_url(
                Some(o),
                "https://generativelanguage.googleapis.com/v1beta/openai",
            ),
            None => self.url(),
        };
        Ok(BuildRequestParts {
            url,
            body: default_openai_compat_body(opts),
            headers: vec![("content-type", "application/json".into())],
        })
    }
}

pub(crate) struct ClaudeCliProvider {
    base_url: Option<String>,
}
impl ClaudeCliProvider {
    pub fn new(base_url: Option<String>) -> Self {
        Self { base_url }
    }
    fn url(&self) -> String {
        // The `claude_cli` provider_type is sourced from a CLI-cached
        // sk-ant-* token (see providers::claude_cli + credential_scanner)
        // but speaks the Anthropic wire protocol — so the URL/auth shape
        // is identical to AnthropicProvider.
        openai_compat_url(self.base_url.as_deref(), "https://api.anthropic.com")
    }
}
#[async_trait]
impl LlmProvider for ClaudeCliProvider {
    async fn stream(
        &self,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<reqwest::Response, ProviderError> {
        let body = build_body(model, messages, tools, true);
        send_anthropic(&self.url(), api_key, &body).await
    }
    async fn complete(
        &self,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<(ChatMessage, serde_json::Value), ProviderError> {
        let body = build_body(model, messages, tools, false);
        let resp = send_anthropic(&self.url(), api_key, &body).await?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| classify_error(0, &e.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(classify_error(status, &text));
        }
        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Unknown(format!("invalid JSON: {}", e)))?;
        Ok((extract_openai_message(&json), json))
    }
    fn provider_type(&self) -> &'static str {
        "claude_cli"
    }

    /// Phase 4: claude_cli speaks Anthropic protocol (sk-ant-* token from
    /// the CLI cache, see providers::claude_cli + credential_scanner) so
    /// the request shape is identical to `AnthropicProvider`'s native
    /// Messages-API body — same cache_control, same adaptive thinking,
    /// same multimodal conversion. The Phase 3 streaming.rs comment
    /// flagged this as "Phase 4's job" — fixed here.
    fn build_stream_request(
        &self,
        opts: &BuildRequestOpts<'_>,
    ) -> Result<BuildRequestParts, ProviderError> {
        let url = anthropic_messages_url(opts.base_url_override.or(self.base_url.as_deref()));
        Ok(BuildRequestParts {
            url,
            body: build_anthropic_body(opts),
            headers: anthropic_headers(),
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(types: &[(&str, &str)]) -> AgentsConfig {
        let mut cfg = AgentsConfig::with_defaults();
        cfg.providers.clear();
        for (name, ptype) in types {
            cfg.providers.insert(
                (*name).into(),
                ProviderEntry {
                    provider_type: (*ptype).into(),
                    ..Default::default()
                },
            );
        }
        cfg
    }

    #[test]
    fn resolver_returns_anthropic_for_anthropic_type() {
        let cfg = cfg_with(&[("primary", "anthropic")]);
        let r = DefaultProviderResolver::from_config(&cfg);
        let p = r.resolve("primary").expect("provider resolved");
        assert_eq!(p.provider_type(), "anthropic");
    }

    #[test]
    fn resolver_returns_openai_for_openai_type() {
        let cfg = cfg_with(&[("p", "openai")]);
        let r = DefaultProviderResolver::from_config(&cfg);
        let p = r.resolve("p").expect("provider resolved");
        assert_eq!(p.provider_type(), "openai");
    }

    #[test]
    fn resolver_returns_gemini_for_gemini_type() {
        let cfg = cfg_with(&[("p", "gemini")]);
        let r = DefaultProviderResolver::from_config(&cfg);
        let p = r.resolve("p").expect("provider resolved");
        assert_eq!(p.provider_type(), "gemini");
    }

    #[test]
    fn resolver_returns_claude_cli_for_claude_cli_type() {
        let cfg = cfg_with(&[("p", "claude_cli")]);
        let r = DefaultProviderResolver::from_config(&cfg);
        let p = r.resolve("p").expect("provider resolved");
        assert_eq!(p.provider_type(), "claude_cli");
    }

    #[test]
    fn resolver_returns_none_for_unknown_type() {
        let cfg = cfg_with(&[("p", "anthropic")]);
        let r = DefaultProviderResolver::from_config(&cfg);
        assert!(r.resolve("not-a-configured-name").is_none());
    }

    // ── Bonus URL-shape canaries: confirm each impl picks the same base URL
    //     `agent.rs::provider_url` would for the same input. These are NOT in
    //     the spec's required test list but they're the cheapest way to
    //     guarantee the passthrough invariant doesn't drift.

    #[test]
    fn anthropic_url_matches_legacy_default() {
        let p = AnthropicProvider::new(None);
        assert_eq!(p.url(), "https://api.anthropic.com/v1/chat/completions");
    }

    #[test]
    fn openai_url_matches_legacy_default() {
        let p = OpenAICompatProvider::new("openai".into(), None);
        assert_eq!(p.url(), "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn gemini_url_matches_legacy_default() {
        let p = GeminiProvider::new(None);
        assert_eq!(
            p.url(),
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
        );
    }

    #[test]
    fn openai_compat_honours_explicit_v1() {
        let p =
            OpenAICompatProvider::new("custom".into(), Some("https://proxy.example.com/v1".into()));
        assert_eq!(p.url(), "https://proxy.example.com/v1/chat/completions");
    }

    // ── V1 (2026-05-21): pin each OpenAI-compat provider type to its
    //    canonical native endpoint. Before these arms were added to
    //    `OpenAICompatProvider::url`, an agent with `provider = "mistral"`
    //    silently fell through to "https://openrouter.ai/api/v1/chat/completions"
    //    and 401-ed because the Bearer key isn't valid there.

    #[test]
    fn mistral_url_routes_to_native() {
        let p = OpenAICompatProvider::new("mistral".into(), None);
        assert_eq!(p.url(), "https://api.mistral.ai/v1/chat/completions");
    }

    #[test]
    fn together_url_routes_to_native() {
        let p = OpenAICompatProvider::new("together".into(), None);
        assert_eq!(p.url(), "https://api.together.xyz/v1/chat/completions");
    }

    #[test]
    fn nvidia_url_routes_to_native() {
        let p = OpenAICompatProvider::new("nvidia".into(), None);
        assert_eq!(
            p.url(),
            "https://integrate.api.nvidia.com/v1/chat/completions"
        );
    }

    #[test]
    fn fireworks_url_routes_to_native() {
        let p = OpenAICompatProvider::new("fireworks".into(), None);
        assert_eq!(
            p.url(),
            "https://api.fireworks.ai/inference/v1/chat/completions"
        );
    }

    #[test]
    fn xai_url_routes_to_native() {
        let p = OpenAICompatProvider::new("xai".into(), None);
        assert_eq!(p.url(), "https://api.x.ai/v1/chat/completions");
    }

    #[test]
    fn ai21_url_routes_to_native() {
        let p = OpenAICompatProvider::new("ai21".into(), None);
        assert_eq!(p.url(), "https://api.ai21.com/studio/v1/chat/completions");
    }

    #[test]
    fn perplexity_url_routes_to_native_no_v1_segment() {
        let p = OpenAICompatProvider::new("perplexity".into(), None);
        // Perplexity's endpoint is at the bare /chat/completions path —
        // pin this explicitly so a future "always-add-/v1" refactor
        // doesn't silently break it.
        assert_eq!(p.url(), "https://api.perplexity.ai/chat/completions");
    }

    #[test]
    fn cohere_url_routes_to_openai_compat_endpoint() {
        let p = OpenAICompatProvider::new("cohere".into(), None);
        // Cohere's OpenAI-compat endpoint is /compatibility/v1/chat/completions
        // (the bare /v1/chat path is Cohere's NATIVE chat API, different
        // body shape; OpenAICompatProvider must not use it).
        assert_eq!(
            p.url(),
            "https://api.cohere.com/compatibility/v1/chat/completions"
        );
    }

    #[test]
    fn unknown_type_still_falls_back_to_openrouter() {
        let p = OpenAICompatProvider::new("some-new-provider".into(), None);
        assert_eq!(p.url(), "https://openrouter.ai/api/v1/chat/completions");
    }

    #[test]
    fn typed_provider_still_honours_explicit_override() {
        // If the operator sets an explicit proxy URL, it must override the
        // built-in match arms (e.g. self-hosted vLLM proxying Mistral).
        let p = OpenAICompatProvider::new(
            "mistral".into(),
            Some("https://proxy.example.com/v1".into()),
        );
        assert_eq!(p.url(), "https://proxy.example.com/v1/chat/completions");
    }
}
