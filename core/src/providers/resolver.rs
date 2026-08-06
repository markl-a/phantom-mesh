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
        // Sanctioned subscription path via the official `claude -p` CLI
        // (Agent SDK credits). Non-streaming — agent.rs routes it to complete().
        "claude_agent" => Arc::new(crate::providers::claude_agent::ClaudeAgentProvider::new()),
        // Sanctioned-CLI path for the OTHER local AI CLIs: shell out to the
        // official codex/opencode/agy CLI via the L0 cli_session substrate (one
        // shared CliSessionProvider). Named `*_agent` like `claude_agent` — the
        // existing `codex_cli`/`claude_cli` modules are OAuth-token DISCOVERY, a
        // different (gray-zone) thing. The provider KEY carries the target because
        // ProviderEntry has no free-form options field.
        "codex_agent" => Arc::new(crate::providers::cli_session_provider::CliSessionProvider::new(
            crate::cli_session::CliKind::Codex,
        )),
        "opencode_agent" => Arc::new(
            crate::providers::cli_session_provider::CliSessionProvider::new(
                crate::cli_session::CliKind::Opencode,
            ),
        ),
        "agy_agent" => Arc::new(crate::providers::cli_session_provider::CliSessionProvider::new(
            crate::cli_session::CliKind::Agy,
        )),
        // claude via the L0 cli_session substrate — the GOVERNED path (distinct from
        // `claude_agent` above, which is the ungoverned `claude -p` complete()).
        // A dispatched agent on this key runs under run_govern_folded when
        // SPECTYN_GOVERN_CLI=1, where claude gets its true PreToolUse pre-action gate
        // (PreActionDelegated) — the only path that raises a pre-action approval, and
        // the one the dispatch↔approval correlation rides. apex-④ flagship loop.
        "claude_session" => Arc::new(
            crate::providers::cli_session_provider::CliSessionProvider::new(
                crate::cli_session::CliKind::Claude,
            ),
        ),
        // External one-shot gateway CLIs registered in the external_gateway registry.
        // The registry is the single place that names the upstream programs.
        key if crate::cli_session::external_gateway::lookup(key).is_some() => {
            Arc::new(crate::providers::cli_session_provider::CliSessionProvider::new(
                crate::cli_session::CliKind::External(
                    crate::cli_session::external_gateway::lookup(key).unwrap(),
                ),
            ))
        }
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

// ── Gemini native (generateContent) helpers ──────────────────────────────
//
// Google 的 OpenAI 相容墊片（OpenAI-compat shim，位於
// `/v1beta/openai/chat/completions`）不會穩定回傳結構化的 `tool_calls`——
// 它常把工具呼叫當成純文字輸出，導致工具靜默不執行。改用 Gemini 原生
// API（native API，`:generateContent`）+ `functionDeclarations`，再把回應
// 的 `functionCall` part 轉成 OpenAI 形狀的 `tool_calls`，工具才會可靠執行。

/// Build the Gemini native `:generateContent` URL. Defaults to the v1beta
/// base (NOT the `/openai` shim). Honours an explicit `base_url` override,
/// stripping a trailing `/openai`, `/v1beta`, or `/chat/completions` suffix
/// so an operator config pointing at the old shim still resolves correctly.
fn gemini_native_url(base_url: Option<&str>, model: &str) -> String {
    let default_base = "https://generativelanguage.googleapis.com/v1beta";
    let raw = base_url.unwrap_or(default_base).trim_end_matches('/');
    // Peel off any OpenAI-compat / shim suffixes the operator may have stored.
    let base = raw
        .trim_end_matches("/chat/completions")
        .trim_end_matches('/')
        .trim_end_matches("/openai")
        .trim_end_matches('/');
    // Ensure the base carries a version segment; if the override dropped it,
    // re-add `/v1beta` so `:generateContent` resolves.
    let base = if base.contains("/v1beta") || base.contains("/v1") {
        base.to_string()
    } else {
        format!("{}/v1beta", base)
    };
    format!("{}/models/{}:generateContent", base, model)
}

/// Recursively sanitize a JSON schema into the subset Gemini's `parameters`
/// validator accepts. Beyond dropping rejected keys (`additionalProperties`,
/// `$schema`, `$ref`, `$defs`, `definitions`, `default`, `examples`,
/// `anyOf`/`oneOf`/`allOf`), it fixes the two constructs that 400 in practice:
///   1. a JSON-Schema `type` array (e.g. `["string","null"]`) → first non-null
///      scalar type, and
///   2. an `OBJECT` node with no/empty `properties` (e.g. a free-form map
///      declared via `additionalProperties`) — Gemini requires OBJECT to carry
///      non-empty properties, so we demote such a node to `STRING`. The model
///      then passes JSON text and the (optional) arg degrades gracefully.
/// The top-level parameters object is handled by the caller, which omits it
/// entirely when it has no properties left.
fn strip_gemini_schema(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            for k in [
                "additionalProperties",
                "$schema",
                "$ref",
                "$defs",
                "definitions",
                "default",
                "examples",
                "title",
                "anyOf",
                "oneOf",
                "allOf",
            ] {
                map.remove(k);
            }
            // Normalize a `type` array (["string","null"]) to its first
            // non-null scalar type.
            if let Some(arr) = map.get("type").and_then(|t| t.as_array()).cloned() {
                if let Some(first) = arr.iter().filter_map(|x| x.as_str()).find(|s| *s != "null") {
                    map.insert("type".into(), serde_json::Value::String(first.to_string()));
                }
            }
            // Sanitize children first so the empty-object check below sees the
            // post-strip shape.
            for (_k, child) in map.iter_mut() {
                strip_gemini_schema(child);
            }
            // Demote an OBJECT with no usable properties to STRING.
            let is_object = map.get("type").and_then(|t| t.as_str()) == Some("object");
            let empty_props = map
                .get("properties")
                .and_then(|p| p.as_object())
                .map(|o| o.is_empty())
                .unwrap_or(true);
            if is_object && empty_props {
                map.remove("properties");
                map.remove("required");
                map.insert("type".into(), serde_json::Value::String("string".into()));
            }
        }
        serde_json::Value::Array(arr) => {
            for child in arr.iter_mut() {
                strip_gemini_schema(child);
            }
        }
        _ => {}
    }
}

/// Convert OpenAI-shaped `messages` + `tools` into a Gemini native
/// `generateContent` request body.
///
/// - system messages → `systemInstruction.parts[].text` (concatenated)
/// - user → `{role:"user", parts:[{text}]}`
/// - assistant/model text → `{role:"model", parts:[{text}]}`
/// - assistant carrying `tool_calls` → `{role:"model", parts:[{functionCall}]}`
/// - tool result → `{role:"user", parts:[{functionResponse}]}`
/// - tools → `[{functionDeclarations: [...]}]` (schema stripped)
fn build_gemini_body(
    model: &str,
    messages: &[ChatMessage],
    tools: &[serde_json::Value],
) -> serde_json::Value {
    let _ = model; // model goes in the URL for native generateContent.

    let mut system_text = String::new();
    let mut contents: Vec<serde_json::Value> = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                if !msg.content.is_empty() {
                    if !system_text.is_empty() {
                        system_text.push_str("\n\n");
                    }
                    system_text.push_str(&msg.content);
                }
            }
            "tool" => {
                // OpenAI tool-result message → Gemini functionResponse.
                // ChatMessage has no `name`/`tool_call_id` field, so we
                // best-effort recover the name from a `tool_calls` blob if
                // present, else default to "tool".
                let name = msg
                    .tool_calls
                    .as_ref()
                    .and_then(|tc| tc.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("tool")
                    .to_string();
                contents.push(serde_json::json!({
                    "role": "user",
                    "parts": [{
                        "functionResponse": {
                            "name": name,
                            "response": { "result": msg.content },
                        }
                    }]
                }));
            }
            "assistant" | "model" => {
                let mut parts: Vec<serde_json::Value> = Vec::new();
                if !msg.content.is_empty() {
                    parts.push(serde_json::json!({ "text": msg.content }));
                }
                // Assistant turn carrying tool calls → functionCall parts.
                if let Some(tcs) = msg.tool_calls.as_ref().and_then(|v| v.as_array()) {
                    for tc in tcs {
                        let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                        // OpenAI `arguments` is a JSON string; parse to object.
                        let args: serde_json::Value = tc["function"]["arguments"]
                            .as_str()
                            .and_then(|s| serde_json::from_str(s).ok())
                            .unwrap_or_else(|| serde_json::json!({}));
                        parts.push(serde_json::json!({
                            "functionCall": { "name": name, "args": args }
                        }));
                    }
                }
                if parts.is_empty() {
                    parts.push(serde_json::json!({ "text": "" }));
                }
                contents.push(serde_json::json!({ "role": "model", "parts": parts }));
            }
            // user (and any unknown role) → user text.
            _ => {
                contents.push(serde_json::json!({
                    "role": "user",
                    "parts": [{ "text": msg.content }],
                }));
            }
        }
    }

    let mut body = serde_json::json!({ "contents": contents });

    if !system_text.is_empty() {
        body["systemInstruction"] = serde_json::json!({
            "parts": [{ "text": system_text }],
        });
    }

    if !tools.is_empty() {
        let decls: Vec<serde_json::Value> = tools
            .iter()
            .map(|td| {
                let func = &td["function"];
                let mut decl = serde_json::Map::new();
                if let Some(name) = func["name"].as_str() {
                    decl.insert("name".into(), serde_json::Value::String(name.into()));
                }
                if let Some(desc) = func["description"].as_str() {
                    decl.insert(
                        "description".into(),
                        serde_json::Value::String(desc.into()),
                    );
                }
                // parameters: sanitize, then keep ONLY if it remains an object
                // with at least one property. Gemini rejects an OBJECT with no
                // properties, and a no-arg function may legally omit parameters.
                if let Some(params) = func.get("parameters") {
                    if !params.is_null() {
                        let mut p = params.clone();
                        strip_gemini_schema(&mut p);
                        let has_props = p
                            .get("properties")
                            .and_then(|x| x.as_object())
                            .map(|o| !o.is_empty())
                            .unwrap_or(false);
                        if has_props {
                            decl.insert("parameters".into(), p);
                        }
                    }
                }
                serde_json::Value::Object(decl)
            })
            .collect();
        body["tools"] = serde_json::json!([{ "functionDeclarations": decls }]);
    }

    body
}

/// Map a Gemini `finishReason` to a user-facing notice when the response was
/// truncated (`MAX_TOKENS`) or filtered (`SAFETY`). Returns `None` for normal
/// completions (`STOP`, `stop`, tool calls, etc.) so the happy path is
/// untouched. The leading newline keeps the marker on its own line after the
/// model's (possibly partial) text.
fn gemini_finish_notice(finish_reason: &str) -> Option<&'static str> {
    match finish_reason {
        "MAX_TOKENS" => Some(
            "\n\n⚠ Response truncated by Gemini: output hit the max-tokens cap \
             (finishReason=MAX_TOKENS). The answer above is incomplete — raise \
             `SPECTYN_MAX_TOKENS` or split the prompt and re-run.",
        ),
        "SAFETY" => Some(
            "\n\n⚠ Response blocked by Gemini safety filter \
             (finishReason=SAFETY). The answer above may be partial or withheld; \
             rephrase the request and re-run.",
        ),
        _ => None,
    }
}

/// Parse a Gemini native `generateContent` response into the OpenAI-shaped
/// `(ChatMessage, synthetic_json)` pair the agent loop consumes.
///
/// - `candidates[0].content.parts[].text` → concatenated `content`
/// - each `functionCall` part → an OpenAI `tool_call`
/// - `usageMetadata` → `usage.prompt_tokens` / `usage.completion_tokens`
fn parse_gemini_response(json: &serde_json::Value, model: &str) -> (ChatMessage, serde_json::Value) {
    let candidate = &json["candidates"][0];
    let parts = candidate["content"]["parts"].as_array();

    let mut content = String::new();
    let mut tool_calls: Vec<serde_json::Value> = Vec::new();

    if let Some(parts) = parts {
        let mut call_idx = 0usize;
        for part in parts {
            if let Some(text) = part["text"].as_str() {
                content.push_str(text);
            }
            if let Some(fc) = part.get("functionCall") {
                let name = fc["name"].as_str().unwrap_or("").to_string();
                // Gemini `args` is a JSON object; OpenAI wants the arguments
                // as a JSON *string*.
                let args_str = serde_json::to_string(fc.get("args").unwrap_or(&serde_json::json!({})))
                    .unwrap_or_else(|_| "{}".into());
                tool_calls.push(serde_json::json!({
                    "id": format!("call_{}", call_idx),
                    "type": "function",
                    "function": { "name": name, "arguments": args_str },
                }));
                call_idx += 1;
            }
        }
    }

    let finish_reason = candidate["finishReason"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "stop".into());

    // Truncation / safety notice: Gemini reports MAX_TOKENS (output cap hit) and
    // SAFETY (response filtered) via `finishReason`, but returns HTTP 200 with a
    // (usually) non-empty `content`. Without surfacing it, the caller treats the
    // partial/filtered text as a normal completion and the user never learns the
    // answer was cut off or censored. Append a clear marker to the returned
    // content so it reaches the user through the existing token path.
    //
    // We only append when there IS content — the EMPTY-candidates dead-end
    // (content="" + no tool calls) is handled by the sibling failover guard in
    // agent.rs, and adding a marker there would mask the empty case from it.
    if !content.is_empty() {
        if let Some(notice) = gemini_finish_notice(&finish_reason) {
            content.push_str(notice);
        }
    }

    let prompt_tokens = json["usageMetadata"]["promptTokenCount"]
        .as_u64()
        .unwrap_or(0);
    let completion_tokens = json["usageMetadata"]["candidatesTokenCount"]
        .as_u64()
        .unwrap_or(0);

    let tool_calls_val = if tool_calls.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::Array(tool_calls.clone())
    };

    let synthetic = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": content,
                "tool_calls": tool_calls_val,
            },
            "finish_reason": finish_reason,
        }],
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
        },
        "model": model,
    });

    let chat = ChatMessage {
        role: "assistant".into(),
        content,
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(serde_json::Value::Array(tool_calls))
        },
    };

    (chat, synthetic)
}

/// POST a Gemini native request with the `x-goog-api-key` header.
async fn send_gemini_native(
    url: &str,
    api_key: &str,
    body: &serde_json::Value,
) -> Result<reqwest::Response, ProviderError> {
    reqwest::Client::new()
        .post(url)
        .header("x-goog-api-key", api_key)
        .header("Content-Type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| classify_error(0, &e.to_string()))
}

/// Run a one-shot Gemini native `generateContent` call and return the
/// OpenAI-shaped `(ChatMessage, synthetic_json)`.
async fn one_shot_gemini_native(
    base_url: Option<&str>,
    api_key: &str,
    model: &str,
    messages: &[ChatMessage],
    tools: &[serde_json::Value],
) -> Result<(ChatMessage, serde_json::Value), ProviderError> {
    let url = gemini_native_url(base_url, model);
    let body = build_gemini_body(model, messages, tools);
    let resp = send_gemini_native(&url, api_key, &body).await?;
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
    Ok(parse_gemini_response(&json, model))
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
}
#[async_trait]
impl LlmProvider for GeminiProvider {
    /// Gemini 原生串流（native SSE）與 agent 的 OpenAI-SSE 解析器不相容，
    /// 因此 agent 對 gemini 一律短路（short-circuit）走非串流的 `complete`
    /// 原生路徑（見 `agent.rs`）。此 `stream` 仍指向原生 `generateContent`
    /// 端點以維持型別一致；正常路徑不會走到這裡。
    async fn stream(
        &self,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<reqwest::Response, ProviderError> {
        let url = gemini_native_url(self.base_url.as_deref(), model);
        let body = build_gemini_body(model, messages, tools);
        send_gemini_native(&url, api_key, &body).await
    }
    /// Native `:generateContent` with `functionDeclarations`. Parses
    /// `functionCall` parts into OpenAI-shaped `tool_calls` so tool-use works
    /// reliably (the OpenAI-compat shim does NOT reliably emit `tool_calls`).
    async fn complete(
        &self,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<(ChatMessage, serde_json::Value), ProviderError> {
        one_shot_gemini_native(self.base_url.as_deref(), api_key, model, messages, tools).await
    }
    fn provider_type(&self) -> &'static str {
        "gemini"
    }

    /// Gemini native `generateContent` request shaping. The agent
    /// short-circuits gemini to the non-streaming `complete` path (native
    /// SSE is incompatible with the OpenAI-SSE parser), so this method's
    /// body is the native shape for any caller that does shape-then-send.
    fn build_stream_request(
        &self,
        opts: &BuildRequestOpts<'_>,
    ) -> Result<BuildRequestParts, ProviderError> {
        // Convert the agent's OpenAI-shaped Value messages → ChatMessage so
        // we can reuse build_gemini_body.
        let chat_messages = value_messages_to_chat(opts.messages);
        let base = opts.base_url_override.or(self.base_url.as_deref());
        let url = gemini_native_url(base, opts.model);
        let body = build_gemini_body(opts.model, &chat_messages, opts.tools);
        Ok(BuildRequestParts {
            url,
            body,
            headers: vec![("content-type", "application/json".into())],
        })
    }
}

/// Convert the agent's OpenAI-shaped `Value` messages into `ChatMessage`s.
/// Used to bridge `build_stream_request` (Value input) → `build_gemini_body`
/// (ChatMessage input).
fn value_messages_to_chat(messages: &[serde_json::Value]) -> Vec<ChatMessage> {
    messages
        .iter()
        .map(|m| ChatMessage {
            role: m["role"].as_str().unwrap_or("user").to_string(),
            content: m["content"].as_str().unwrap_or("").to_string(),
            // Preserve assistant tool_calls and tool-result `name` so the
            // native conversion can rebuild functionCall / functionResponse.
            tool_calls: m
                .get("tool_calls")
                .cloned()
                .filter(|v| !v.is_null())
                .or_else(|| {
                    // tool-result messages carry a top-level `name`; wrap it so
                    // build_gemini_body's tool branch can read it.
                    m.get("name").and_then(|n| n.as_str()).map(|n| {
                        serde_json::json!({ "name": n })
                    })
                }),
        })
        .collect()
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
    fn build_provider_maps_cli_session_agent_keys() {
        let entry = ProviderEntry::default();
        // The sanctioned-CLI keys all resolve to the shared CliSessionProvider.
        assert_eq!(build_provider("codex_agent", &entry).provider_type(), "cli_session");
        assert_eq!(build_provider("opencode_agent", &entry).provider_type(), "cli_session");
        assert_eq!(build_provider("agy_agent", &entry).provider_type(), "cli_session");
        // claude_session = the GOVERNED claude cli_session path (apex-④ pre-action
        // gate) — distinct from claude_agent (ungoverned `claude -p`).
        assert_eq!(build_provider("claude_session", &entry).provider_type(), "cli_session");
        assert_ne!(build_provider("claude_agent", &entry).provider_type(), "cli_session");
        assert_eq!(
            crate::providers::cli_session_provider::cli_for_provider_key("claude_session"),
            Some(crate::cli_session::CliKind::Claude),
        );
        // External gateway keys from the registry also resolve to cli_session.
        for gw in crate::cli_session::external_gateway::all() {
            assert_eq!(build_provider(gw.key, &entry).provider_type(), "cli_session",
                "expected cli_session for external gateway key {}", gw.key);
        }
        // An unknown type still falls through to the OpenAI-compat provider.
        assert_ne!(build_provider("groq", &entry).provider_type(), "cli_session");
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
    fn gemini_native_url_default() {
        // Gemini now uses the NATIVE generateContent endpoint (NOT the
        // /openai shim) so structured tool_calls are reliable.
        assert_eq!(
            gemini_native_url(None, "gemini-2.0-flash"),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent"
        );
    }

    #[test]
    fn gemini_native_url_strips_openai_shim_override() {
        // An operator config pointing at the old /openai shim still resolves
        // to the native endpoint.
        assert_eq!(
            gemini_native_url(
                Some("https://generativelanguage.googleapis.com/v1beta/openai"),
                "gemini-2.0-flash"
            ),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent"
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

    // ── Gemini native function-calling: body builder + response parser ──────

    fn msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.into(),
            content: content.into(),
            tool_calls: None,
        }
    }

    #[test]
    fn gemini_body_system_to_system_instruction() {
        let msgs = vec![msg("system", "You are a helpful agent."), msg("user", "hi")];
        let body = build_gemini_body("gemini-2.0-flash", &msgs, &[]);
        assert_eq!(
            body["systemInstruction"]["parts"][0]["text"],
            "You are a helpful agent."
        );
        // System message must NOT leak into contents.
        assert_eq!(body["contents"].as_array().unwrap().len(), 1);
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][0]["parts"][0]["text"], "hi");
    }

    #[test]
    fn gemini_body_user_assistant_roles() {
        let msgs = vec![
            msg("user", "what is 2+2?"),
            msg("assistant", "4"),
        ];
        let body = build_gemini_body("m", &msgs, &[]);
        let contents = body["contents"].as_array().unwrap();
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"][0]["text"], "4");
        // No tools → no systemInstruction either.
        assert!(body.get("systemInstruction").is_none());
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn gemini_body_assistant_tool_calls_to_function_call() {
        let mut assistant = msg("assistant", "");
        assistant.tool_calls = Some(serde_json::json!([{
            "id": "call_0",
            "type": "function",
            "function": {
                "name": "read_file",
                "arguments": "{\"path\":\"/tmp/x\"}"
            }
        }]));
        let msgs = vec![msg("user", "read it"), assistant];
        let body = build_gemini_body("m", &msgs, &[]);
        let model_turn = &body["contents"][1];
        assert_eq!(model_turn["role"], "model");
        let fc = &model_turn["parts"][0]["functionCall"];
        assert_eq!(fc["name"], "read_file");
        // args must be a parsed JSON object, not a string.
        assert_eq!(fc["args"]["path"], "/tmp/x");
    }

    #[test]
    fn gemini_body_tool_result_to_function_response() {
        let mut tool = msg("tool", "file contents here");
        tool.tool_calls = Some(serde_json::json!({ "name": "read_file" }));
        let msgs = vec![tool];
        let body = build_gemini_body("m", &msgs, &[]);
        let part = &body["contents"][0];
        assert_eq!(part["role"], "user");
        let fr = &part["parts"][0]["functionResponse"];
        assert_eq!(fr["name"], "read_file");
        assert_eq!(fr["response"]["result"], "file contents here");
    }

    #[test]
    fn gemini_body_tools_to_function_declarations_strips_schema() {
        let tools = vec![serde_json::json!({
            "type": "function",
            "function": {
                "name": "search",
                "description": "search the web",
                "parameters": {
                    "type": "object",
                    "additionalProperties": false,
                    "$schema": "http://json-schema.org/draft-07/schema#",
                    "properties": {
                        "q": { "type": "string" }
                    }
                }
            }
        })];
        let body = build_gemini_body("m", &[msg("user", "go")], &tools);
        let decls = &body["tools"][0]["functionDeclarations"];
        assert_eq!(decls[0]["name"], "search");
        assert_eq!(decls[0]["description"], "search the web");
        let params = &decls[0]["parameters"];
        // Rejected JSON-schema keys must be stripped recursively.
        assert!(params.get("additionalProperties").is_none());
        assert!(params.get("$schema").is_none());
        // Legitimate schema kept intact.
        assert_eq!(params["properties"]["q"]["type"], "string");
    }

    #[test]
    fn gemini_body_omits_empty_parameters() {
        let tools = vec![serde_json::json!({
            "type": "function",
            "function": { "name": "ping", "parameters": {} }
        })];
        let body = build_gemini_body("m", &[msg("user", "go")], &tools);
        let decl = &body["tools"][0]["functionDeclarations"][0];
        assert_eq!(decl["name"], "ping");
        assert!(decl.get("parameters").is_none());
    }

    #[test]
    fn gemini_body_demotes_freeform_map_object_to_string() {
        // Mirrors the real 400: a property typed as a free-form map
        // ({type:object, additionalProperties:{type:string}}) with no
        // `properties`. Gemini rejects OBJECT-with-no-properties, so it must be
        // demoted to STRING while the parent params (with a real prop) survive.
        let tools = vec![serde_json::json!({
            "type": "function",
            "function": {
                "name": "shell",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" },
                        "env": {
                            "type": "object",
                            "additionalProperties": { "type": "string" }
                        }
                    },
                    "required": ["command"]
                }
            }
        })];
        let body = build_gemini_body("m", &[msg("user", "go")], &tools);
        let params = &body["tools"][0]["functionDeclarations"][0]["parameters"];
        // Parent object kept (has a real property).
        assert_eq!(params["type"], "object");
        assert_eq!(params["properties"]["command"]["type"], "string");
        // Free-form map demoted to string; no bare empty OBJECT remains.
        assert_eq!(params["properties"]["env"]["type"], "string");
        assert!(params["properties"]["env"].get("additionalProperties").is_none());
    }

    #[test]
    fn gemini_body_omits_params_when_only_empty_object() {
        // A no-arg tool declared as {type:object, properties:{}} (e.g.
        // cluster_status) must drop parameters entirely, not send an empty
        // OBJECT.
        let tools = vec![serde_json::json!({
            "type": "function",
            "function": {
                "name": "cluster_status",
                "parameters": { "type": "object", "properties": {} }
            }
        })];
        let body = build_gemini_body("m", &[msg("user", "go")], &tools);
        let decl = &body["tools"][0]["functionDeclarations"][0];
        assert_eq!(decl["name"], "cluster_status");
        assert!(decl.get("parameters").is_none());
    }

    #[test]
    fn gemini_response_function_call_to_tool_calls() {
        let resp = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "read_file",
                            "args": { "path": "/etc/hosts" }
                        }
                    }]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 12,
                "candidatesTokenCount": 7
            }
        });
        let (chat, json) = parse_gemini_response(&resp, "gemini-2.0-flash");
        let tc = &json["choices"][0]["message"]["tool_calls"][0];
        assert_eq!(tc["type"], "function");
        assert_eq!(tc["function"]["name"], "read_file");
        // arguments must be a JSON *string* (OpenAI shape).
        let args_str = tc["function"]["arguments"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(args_str).unwrap();
        assert_eq!(parsed["path"], "/etc/hosts");
        assert_eq!(json["usage"]["prompt_tokens"], 12);
        assert_eq!(json["usage"]["completion_tokens"], 7);
        assert_eq!(json["model"], "gemini-2.0-flash");
        // ChatMessage carries the tool_calls too.
        assert!(chat.tool_calls.is_some());
        assert_eq!(chat.role, "assistant");
    }

    #[test]
    fn gemini_response_text_only_no_tool_calls() {
        let resp = serde_json::json!({
            "candidates": [{
                "content": { "parts": [{ "text": "Hello there." }] },
                "finishReason": "STOP"
            }],
            "usageMetadata": { "promptTokenCount": 3, "candidatesTokenCount": 2 }
        });
        let (chat, json) = parse_gemini_response(&resp, "gemini-2.0-flash");
        assert_eq!(json["choices"][0]["message"]["content"], "Hello there.");
        // tool_calls must be null (not an empty array) for a text-only reply.
        assert!(json["choices"][0]["message"]["tool_calls"].is_null());
        assert_eq!(chat.content, "Hello there.");
        assert!(chat.tool_calls.is_none());
    }

    #[test]
    fn gemini_response_max_tokens_surfaces_truncation_notice() {
        // finishReason=MAX_TOKENS with non-empty (partial) content: the user must
        // be told the answer was cut off, not handed the fragment silently.
        let resp = serde_json::json!({
            "candidates": [{
                "content": { "parts": [{ "text": "The answer is partia" }] },
                "finishReason": "MAX_TOKENS"
            }],
            "usageMetadata": { "promptTokenCount": 5, "candidatesTokenCount": 8 }
        });
        let (chat, json) = parse_gemini_response(&resp, "gemini-2.5-flash");
        let content = json["choices"][0]["message"]["content"].as_str().unwrap();
        // Original (partial) text is preserved...
        assert!(content.starts_with("The answer is partia"), "got: {content}");
        // ...and the truncation marker is appended.
        assert!(content.contains("truncated"), "got: {content}");
        assert!(content.contains("MAX_TOKENS"), "got: {content}");
        // The notice rides on the ChatMessage too (same content path).
        assert!(chat.content.contains("MAX_TOKENS"), "got: {}", chat.content);
        // finish_reason is preserved verbatim on the synthetic choice.
        assert_eq!(json["choices"][0]["finish_reason"], "MAX_TOKENS");
    }

    #[test]
    fn gemini_response_safety_surfaces_block_notice() {
        // finishReason=SAFETY with partial content: surface that it was filtered.
        let resp = serde_json::json!({
            "candidates": [{
                "content": { "parts": [{ "text": "Here is some" }] },
                "finishReason": "SAFETY"
            }],
            "usageMetadata": { "promptTokenCount": 4, "candidatesTokenCount": 3 }
        });
        let (chat, json) = parse_gemini_response(&resp, "gemini-2.5-flash");
        let content = json["choices"][0]["message"]["content"].as_str().unwrap();
        assert!(content.starts_with("Here is some"), "got: {content}");
        assert!(content.contains("safety"), "got: {content}");
        assert!(content.contains("SAFETY"), "got: {content}");
        assert!(chat.content.contains("SAFETY"), "got: {}", chat.content);
        assert_eq!(json["choices"][0]["finish_reason"], "SAFETY");
    }

    #[test]
    fn gemini_response_stop_has_no_notice() {
        // The happy path (finishReason=STOP) must be byte-for-byte unchanged: no
        // marker appended, content equals the model text exactly.
        let resp = serde_json::json!({
            "candidates": [{
                "content": { "parts": [{ "text": "All done." }] },
                "finishReason": "STOP"
            }],
            "usageMetadata": { "promptTokenCount": 2, "candidatesTokenCount": 2 }
        });
        let (_chat, json) = parse_gemini_response(&resp, "gemini-2.5-flash");
        assert_eq!(json["choices"][0]["message"]["content"], "All done.");
    }

    #[test]
    fn gemini_empty_content_max_tokens_stays_empty_for_failover() {
        // Empty candidate + MAX_TOKENS: must NOT gain a marker. The sibling
        // empty-candidates failover in agent.rs keys off content=="" to retry the
        // next provider; appending a notice here would mask the empty case.
        let resp = serde_json::json!({
            "candidates": [{
                "content": { "parts": [] },
                "finishReason": "MAX_TOKENS"
            }],
            "usageMetadata": { "promptTokenCount": 1, "candidatesTokenCount": 0 }
        });
        let (chat, json) = parse_gemini_response(&resp, "gemini-2.5-flash");
        assert_eq!(json["choices"][0]["message"]["content"], "");
        assert_eq!(chat.content, "");
    }
}
