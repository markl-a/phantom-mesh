//! `LlmProvider` trait — abstract provider dispatch surface (DEMO-1 gap 1).
//!
//! Phase 1 of the 5-phase plan (`docs/superpowers/specs/
//! 2026-05-17-demo1-gap1-llmprovider-design.md`). Adds the trait surface ONLY;
//! no impls and no call-site changes. Phase 2 (`resolver.rs`) adds the
//! string-switch-passthrough default impls behind a `DefaultProviderResolver`.
//!
//! NOTE: this file may also be authored by the Phase 1 PR (#173 et al.) — the
//! contents are byte-identical with that PR so either-order landings are
//! conflict-free.
use async_trait::async_trait;

use crate::providers::traits::{ChatMessage, ProviderError};

// ── Phase 4: request-shaping types ────────────────────────────────────────
//
// DEMO-1 gap 1 Phase 4 adds `build_stream_request` to the trait so providers
// own URL + body + header shaping. This unblocks `agent.rs::call_with_fallback`
// and `agent.rs::call_with_streaming` migration off the `provider_url`
// string-switch (#185 left them on it because the trait's `stream` method
// builds its own body, which can't carry Anthropic prompt-caching /
// adaptive thinking / multimodal content_block conversion).
//
// The method is **synchronous** and **pure** — no HTTP, just shaping — so
// the caller can plug the result into its own retry / reconnect loop
// unchanged (T19 SSE retry middleware in `streaming.rs`, the per-attempt
// loop in `agent.rs::streaming_with_retry`).

/// Knobs the trait impl needs to shape its request. Captures the subset of
/// `AgentEntry` + per-call state that the 4 default impls actually consume.
/// Kept narrow so the trait stays embedder-friendly; if a future impl needs
/// more state, extend this struct rather than the trait method signature.
#[derive(Debug, Clone)]
pub struct BuildRequestOpts<'a> {
    /// Model id, post-alias-resolution. The Anthropic impl gates
    /// `thinking.display = "omitted"` on the `claude-opus-4-7+` prefix
    /// (see `model_supports_thinking_display_omitted` in streaming.rs).
    pub model: &'a str,
    /// System prompt. May be empty. Anthropic emits it as a cacheable
    /// `text` block with `cache_control: ephemeral`; OpenAI-compat impls
    /// fold it into the `messages` array as `role=system` (the caller is
    /// expected to do that — `messages` below already contains it).
    pub system: &'a str,
    /// User + assistant + tool messages as JSON Values. The OpenAI-compat
    /// shape is the canonical input; Anthropic impls strip the `system`
    /// entry + multimodal-convert each remaining message.
    pub messages: &'a [serde_json::Value],
    /// OpenAI-style tool definitions. Anthropic impls rewrite the schema
    /// + apply `cache_control` on the last tool entry.
    pub tools: &'a [serde_json::Value],
    /// Per-call base URL override from the matching `[providers.*]` entry.
    /// Each impl decides how to normalize (e.g. Anthropic appends
    /// `/v1/messages`; OpenAI-compat appends `/v1/chat/completions`).
    pub base_url_override: Option<&'a str>,
    /// `true` for streaming requests (sets `stream: true` + adds
    /// `stream_options.include_usage` on OpenAI-compat).
    pub stream: bool,
    /// `max_tokens` cap. Use `crate::config::default_max_tokens()` at the
    /// call site; the trait doesn't reach into config so embedders can
    /// override.
    pub max_tokens: u32,
}

/// What `build_stream_request` returns — the three pieces the caller needs
/// to build a `reqwest::RequestBuilder` (and rebuild it on each retry /
/// reconnect attempt without re-shaping the body).
///
/// We deliberately do NOT return a `reqwest::RequestBuilder` directly: it
/// isn't `Clone`, and both `agent.rs::streaming_with_retry` and
/// `streaming.rs::stream_one_round` need to rebuild the request per attempt
/// (e.g. to attach `Last-Event-ID` on SSE reconnect). Returning the parts
/// lets the caller own attempt-state without losing the trait's body shape.
#[derive(Debug, Clone)]
pub struct BuildRequestParts {
    /// Fully resolved request URL (POST target).
    pub url: String,
    /// JSON body, ready to `req.json(&body).send()`.
    pub body: serde_json::Value,
    /// `(name, value)` header pairs. The caller MUST also add an
    /// `Authorization` / `x-api-key` header from its own api_key source —
    /// header shape is provider-specific, owned by the impl.
    pub headers: Vec<(&'static str, String)>,
}

/// A pluggable LLM backend.
///
/// The trait is object-safe so callers can store providers as
/// `Arc<dyn LlmProvider>` — that's the indirection point that lets a test or
/// embedder swap the dispatch path without touching `agent.rs`.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Issue a streaming chat completion. Caller owns SSE parsing.
    async fn stream(
        &self,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<reqwest::Response, ProviderError>;

    /// One-shot non-streaming chat completion. Returns the parsed assistant
    /// message + the raw response JSON (so callers can fish tool_calls out of
    /// provider-specific response shapes).
    async fn complete(
        &self,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<(ChatMessage, serde_json::Value), ProviderError>;

    /// Stable identity. Used by metrics + `SPECTYN_RUNTIME_OVERRIDE` matching.
    fn provider_type(&self) -> &'static str;

    /// Phase 4: shape a streaming/non-streaming request without sending it.
    ///
    /// Returns `(url, body, headers)` so the caller can plug the result into
    /// its own retry / reconnect loop. The caller adds the `Authorization`
    /// (or `x-api-key`) header — the api_key isn't passed in because the
    /// shape of the auth header is itself provider-specific and lives in
    /// `headers` returned here for impls that need it (e.g. Anthropic's
    /// `anthropic-version`).
    ///
    /// Default impl matches the OpenAI-compat wire format — agent.rs's
    /// legacy fallback for unknown provider types. Specific impls
    /// (AnthropicProvider / ClaudeCliProvider) override to add cache_control
    /// + adaptive thinking + multimodal content_block.
    fn build_stream_request(
        &self,
        opts: &BuildRequestOpts<'_>,
    ) -> Result<BuildRequestParts, ProviderError> {
        // Default = OpenAI-compat — same body shape `agent.rs::call_with_*`
        // built inline before Phase 4. Override in specific impls for
        // provider-native wire formats.
        Ok(BuildRequestParts {
            url: opts
                .base_url_override
                .map(|s| s.to_string())
                .unwrap_or_default(),
            body: default_openai_compat_body(opts),
            headers: vec![("content-type", "application/json".into())],
        })
    }
}

/// OpenAI-compat body shape, exposed as a free function so per-impl
/// overrides (which patch the URL but keep the body shape) can reuse it
/// without going through `<dyn LlmProvider>` dispatch.
pub fn default_openai_compat_body(opts: &BuildRequestOpts<'_>) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": opts.model,
        "messages": opts.messages,
        "max_tokens": opts.max_tokens,
    });
    if opts.stream {
        body["stream"] = serde_json::Value::Bool(true);
    }
    if !opts.tools.is_empty() {
        body["tools"] = serde_json::Value::Array(opts.tools.to_vec());
        body["tool_choice"] = serde_json::Value::String("auto".into());
        if opts.stream {
            body["stream_options"] = serde_json::json!({"include_usage": true});
        }
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // Object-safety check: if the trait isn't object-safe this fails to
    // compile. Cheap canary against accidental future signature drift.
    #[allow(dead_code)]
    fn assert_object_safe(_: Arc<dyn LlmProvider>) {}
}
