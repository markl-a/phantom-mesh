//! ChatGPT (Codex) subscription provider via the **Codex backend**
//! (`chatgpt.com/backend-api/codex/responses`, OpenAI Responses API).
//!
//! Reuses the token the official Codex CLI cached (`~/.codex/auth.json`), builds
//! a Responses-API request, and streams the result, accumulating
//! `response.output_text.delta` events. The agent layer adapts the text into its
//! synthetic response + on_token sink (see the `codex_oauth` branch in
//! agent.rs), like claude_agent / gemini_oauth.
//!
//! ⚠️ Most fragile of the subscription providers: it talks to OpenAI's private
//! ChatGPT backend (undocumented, may be Cloudflare-gated, frequently changed)
//! and is a clear OpenAI-ToS gray area. We only read the token the official CLI
//! cached. Protocol mapped from Block's Goose `chatgpt_codex.rs` reference.

use crate::providers::traits::ProviderError;
use futures::StreamExt;
use serde_json::{json, Value};

const CODEX_RESPONSES: &str = "https://chatgpt.com/backend-api/codex/responses";
/// ChatGPT-account Codex model list. Gates by account; requires a
/// `client_version` query param (400s if absent — the *value* only needs to be a
/// plausible codex version, models are account-gated not value-gated).
const CODEX_MODELS_URL: &str = "https://chatgpt.com/backend-api/codex/models";
const CODEX_CLIENT_VERSION: &str = "0.140.0";
/// Re-fetch the discovered model list at most once per this window.
const MODELS_CACHE_TTL_SECS: u64 = 86_400;
/// Last-resort default when discovery fails entirely. Verified working against
/// the ChatGPT-account Codex backend and matches the official `codex` CLI's
/// default for a ChatGPT login. NOTE: `gpt-5.x-codex` ids are rejected (HTTP
/// 400) on ChatGPT accounts — never use one as a static default here.
const FALLBACK_MODEL: &str = "gpt-5.5";

fn err(msg: impl Into<String>) -> ProviderError {
    ProviderError::Unknown(msg.into())
}

fn text_of(m: &Value) -> String {
    if let Some(s) = m.get("content").and_then(|c| c.as_str()) {
        return s.to_string();
    }
    m.get("content")
        .and_then(|c| c.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

/// Split OpenAI-style messages into (instructions, Responses `input` items).
fn build_input(messages: &[Value], system_extra: Option<&str>) -> (String, Vec<Value>) {
    let mut instructions = system_extra.unwrap_or("").to_string();
    let mut input = Vec::new();
    for m in messages {
        let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        let text = text_of(m);
        match role {
            "system" => {
                if !instructions.is_empty() {
                    instructions.push_str("\n\n");
                }
                instructions.push_str(&text);
            }
            "assistant" => input.push(json!({
                "role": "assistant",
                "content": [{"type": "output_text", "text": text}]
            })),
            _ => input.push(json!({
                "role": "user",
                "content": [{"type": "input_text", "text": text}]
            })),
        }
    }
    (instructions, input)
}

/// Pull `response.output_text.delta` text out of one SSE `data:` payload.
fn delta_from_event(data: &str) -> Result<Option<String>, ProviderError> {
    let v: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    match v.get("type").and_then(|t| t.as_str()) {
        Some("response.output_text.delta") => {
            Ok(v.get("delta").and_then(|d| d.as_str()).map(String::from))
        }
        Some("response.failed") => Err(err(format!(
            "Codex response failed: {}",
            data.chars().take(200).collect::<String>()
        ))),
        _ => Ok(None),
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn models_cache_path(home: &std::path::Path) -> std::path::PathBuf {
    home.join(".phantom-mesh").join("codex_models_cache.json")
}

/// Pick the best default model slug from a codex `/models`-shaped JSON body:
/// the listed (`visibility == "list"`) model with the lowest `priority`
/// (lower number = higher rank — `gpt-5.5` is `priority` 9). `exclude` lets the
/// self-heal path skip a slug the backend just rejected. None if nothing fits.
fn pick_best_model(models_json: &Value, exclude: Option<&str>) -> Option<String> {
    let arr = models_json.get("models").and_then(|m| m.as_array())?;
    let mut cands: Vec<(i64, String)> = arr
        .iter()
        .filter_map(|m| {
            let slug = m.get("slug").and_then(|s| s.as_str())?;
            if Some(slug) == exclude {
                return None;
            }
            let vis = m
                .get("visibility")
                .and_then(|v| v.as_str())
                .unwrap_or("list");
            if vis != "list" {
                return None;
            }
            let prio = m
                .get("priority")
                .and_then(|p| p.as_i64())
                .unwrap_or(i64::MAX);
            Some((prio, slug.to_string()))
        })
        .collect();
    cands.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    cands.into_iter().next().map(|(_, s)| s)
}

/// GET the account's available codex models from the ChatGPT backend.
async fn fetch_models(token: &str, account_id: Option<&str>) -> Result<Value, ProviderError> {
    let url = format!("{CODEX_MODELS_URL}?client_version={CODEX_CLIENT_VERSION}");
    let mut req = reqwest::Client::new()
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .header("OpenAI-Beta", "responses=experimental");
    if let Some(acct) = account_id.filter(|s| !s.is_empty()) {
        req = req.header("chatgpt-account-id", acct);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| err(format!("codex models request failed: {e}")))?;
    if !resp.status().is_success() {
        let st = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return Err(err(format!(
            "codex models HTTP {st}: {}",
            txt.chars().take(160).collect::<String>()
        )));
    }
    resp.json::<Value>()
        .await
        .map_err(|e| err(format!("codex models parse: {e}")))
}

fn read_fresh_cache(home: &std::path::Path) -> Option<Value> {
    let raw = std::fs::read_to_string(models_cache_path(home)).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let fetched = v.get("fetched_at").and_then(|t| t.as_u64()).unwrap_or(0);
    (now_secs().saturating_sub(fetched) <= MODELS_CACHE_TTL_SECS).then_some(v)
}

fn write_cache(home: &std::path::Path, models: &Value) {
    let obj = json!({
        "fetched_at": now_secs(),
        "models": models.get("models").cloned().unwrap_or(Value::Array(vec![])),
    });
    let path = models_cache_path(home);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(s) = serde_json::to_string(&obj) {
        let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Read the official `codex` CLI's own model cache (maintained whenever the user
/// runs `codex`) as a last network-free source before the static fallback.
fn read_codex_cli_cache_model(home: &std::path::Path, exclude: Option<&str>) -> Option<String> {
    let raw = std::fs::read_to_string(home.join(".codex").join("models_cache.json")).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    pick_best_model(&v, exclude)
}

/// Resolve the default model dynamically: fresh phantom cache → live backend
/// fetch (cached) → the official `codex` CLI's cache → the static fallback.
/// `exclude` skips a slug the backend just rejected; `force_refresh` bypasses
/// the phantom cache (used by the self-heal retry).
async fn resolve_default_model(
    home: Option<&std::path::Path>,
    token: &str,
    account_id: Option<&str>,
    exclude: Option<&str>,
    force_refresh: bool,
) -> String {
    if !force_refresh {
        if let Some(h) = home {
            if let Some(v) = read_fresh_cache(h) {
                if let Some(slug) = pick_best_model(&v, exclude) {
                    return slug;
                }
            }
        }
    }
    if let Ok(v) = fetch_models(token, account_id).await {
        if let Some(h) = home {
            write_cache(h, &v);
        }
        if let Some(slug) = pick_best_model(&v, exclude) {
            return slug;
        }
    }
    if let Some(h) = home {
        if let Some(slug) = read_codex_cli_cache_model(h, exclude) {
            return slug;
        }
    }
    FALLBACK_MODEL.to_string()
}

/// One attempt's failure: a model-gate 400 (recoverable by switching model) vs
/// anything else (terminal).
enum AttemptErr {
    ModelNotSupported(String),
    Other(String),
}

/// Issue one Codex completion with a fixed model, streaming the Responses API.
async fn attempt_codex(
    auth: &super::codex_cli::CodexAuth,
    model: &str,
    instructions: &str,
    input: &[Value],
) -> Result<String, AttemptErr> {
    let body = json!({
        "model": model,
        "input": input,
        "store": false,
        "reasoning": {"effort": "medium"},
        "instructions": instructions,
        "stream": true,
    });

    let mut req = reqwest::Client::new()
        .post(CODEX_RESPONSES)
        .header("Authorization", format!("Bearer {}", auth.token))
        .header("Content-Type", "application/json")
        .header("OpenAI-Beta", "responses=experimental");
    if let Some(acct) = auth.account_id.as_deref().filter(|s| !s.is_empty()) {
        req = req.header("chatgpt-account-id", acct);
    }

    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| AttemptErr::Other(format!("Codex request failed: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        let txt = resp.text().await.unwrap_or_default();
        let snippet = txt.chars().take(200).collect::<String>();
        if status == reqwest::StatusCode::BAD_REQUEST && txt.contains("not supported") {
            return Err(AttemptErr::ModelNotSupported(format!(
                "Codex backend rejected model '{model}': {snippet}"
            )));
        }
        return Err(AttemptErr::Other(format!(
            "Codex backend HTTP {status} (ChatGPT backend is undocumented/gated): {snippet}"
        )));
    }

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut out = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| AttemptErr::Other(format!("Codex stream error: {e}")))?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(nl) = buf.find('\n') {
            let line: String = buf.drain(..=nl).collect();
            let line = line.trim_end_matches(['\r', '\n']);
            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" {
                    continue;
                }
                if let Some(d) =
                    delta_from_event(data).map_err(|e| AttemptErr::Other(format!("{e:?}")))?
                {
                    out.push_str(&d);
                }
            }
        }
    }
    if out.is_empty() {
        return Err(AttemptErr::Other("Codex returned no text".to_string()));
    }
    Ok(out)
}

/// Run one Codex (ChatGPT subscription) completion. Honours an explicitly
/// configured model; otherwise auto-discovers the account's best available model
/// from the backend (cached). If the backend gates the chosen model (HTTP 400
/// "not supported"), the live list is refreshed and a different model is tried
/// once — so a removed/renamed model self-heals instead of needing a manual fix.
pub async fn run_codex(
    messages: &[Value],
    model: &str,
    system: Option<&str>,
) -> Result<String, ProviderError> {
    let home = crate::cli_config::resolve_home_dir().ok();
    // Refresh-on-use: a phantom-minted "Sign in with ChatGPT" token (opt-in
    // OAuth) is refreshed here if its JWT exp is near — the official `codex` CLI
    // refreshes its own cache, but a phantom-minted one has no other refresher.
    if let Some(h) = home.as_deref() {
        super::openai_oauth::ensure_fresh_if_present(h, now_secs()).await;
    }
    let auth = super::codex_cli::find_codex_auth()
        .ok_or_else(|| err("no Codex credentials — run `codex` (Sign in with ChatGPT) once, or `phantom auth chatgpt`; expected ~/.codex/auth.json"))?;
    if !auth.is_oauth {
        return Err(err(
            "~/.codex/auth.json is in API-key mode — use the `openai` provider with that key instead of codex_oauth",
        ));
    }
    let (instructions, input) = build_input(messages, system);

    // Honour an explicit model; otherwise discover the account's best model.
    let configured = model.trim();
    let mut model_slug = if configured.is_empty() {
        resolve_default_model(
            home.as_deref(),
            &auth.token,
            auth.account_id.as_deref(),
            None,
            false,
        )
        .await
    } else {
        configured.to_string()
    };

    let mut last_err: Option<String> = None;
    for attempt in 0..2 {
        match attempt_codex(&auth, &model_slug, &instructions, &input).await {
            Ok(text) => return Ok(text),
            Err(AttemptErr::Other(msg)) => return Err(err(msg)),
            Err(AttemptErr::ModelNotSupported(msg)) => {
                last_err = Some(msg);
                if attempt == 0 {
                    // Refresh the live list and pick a *different* model.
                    let next = resolve_default_model(
                        home.as_deref(),
                        &auth.token,
                        auth.account_id.as_deref(),
                        Some(&model_slug),
                        true,
                    )
                    .await;
                    if next != model_slug {
                        model_slug = next;
                        continue;
                    }
                }
                break;
            }
        }
    }
    Err(err(last_err.unwrap_or_else(|| "Codex: no usable model".to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_input_and_instructions() {
        let msgs = vec![
            json!({"role": "system", "content": "be terse"}),
            json!({"role": "user", "content": "hi"}),
            json!({"role": "assistant", "content": "hello"}),
        ];
        let (instr, input) = build_input(&msgs, None);
        assert_eq!(instr, "be terse");
        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[1]["content"][0]["type"], "output_text");
    }

    #[test]
    fn parses_output_text_delta() {
        let ev = r#"{"type":"response.output_text.delta","delta":"po"}"#;
        assert_eq!(delta_from_event(ev).unwrap().as_deref(), Some("po"));
        let other = r#"{"type":"response.created"}"#;
        assert_eq!(delta_from_event(other).unwrap(), None);
    }

    #[test]
    fn flags_failed_event() {
        let ev = r#"{"type":"response.failed","response":{"error":"x"}}"#;
        assert!(delta_from_event(ev).is_err());
    }

    fn models_fixture() -> Value {
        // Shape returned by /backend-api/codex/models (lower priority = better).
        json!({"models": [
            {"slug": "gpt-5.4",             "visibility": "list", "priority": 16},
            {"slug": "gpt-5.5",             "visibility": "list", "priority": 9},
            {"slug": "gpt-5.4-mini",        "visibility": "list", "priority": 23},
            {"slug": "codex-auto-review",   "visibility": "hide", "priority": 5},
        ]})
    }

    #[test]
    fn picks_lowest_priority_listed_model() {
        // gpt-5.5 (prio 9) wins; codex-auto-review (prio 5) is hidden → skipped.
        assert_eq!(
            pick_best_model(&models_fixture(), None).as_deref(),
            Some("gpt-5.5")
        );
    }

    #[test]
    fn exclude_skips_rejected_model() {
        // Self-heal: excluding gpt-5.5 falls to the next listed (gpt-5.4).
        assert_eq!(
            pick_best_model(&models_fixture(), Some("gpt-5.5")).as_deref(),
            Some("gpt-5.4")
        );
    }

    #[test]
    fn hidden_only_yields_none() {
        let j = json!({"models": [{"slug": "x", "visibility": "hide", "priority": 1}]});
        assert!(pick_best_model(&j, None).is_none());
        assert!(pick_best_model(&json!({}), None).is_none());
    }

    #[test]
    fn never_defaults_to_a_codex_suffixed_model() {
        // Guards the regression that started this: gpt-5.x-codex ids 400 on
        // ChatGPT accounts, so the static fallback must not be one.
        assert!(!FALLBACK_MODEL.contains("-codex"));
    }
}
