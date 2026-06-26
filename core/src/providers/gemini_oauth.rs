//! Gemini / Antigravity subscription provider via the **Gemini Code Assist**
//! backend (`cloudcode-pa.googleapis.com`).
//!
//! Reuses the credential the official Gemini CLI cached at
//! `~/.gemini/oauth_creds.json`, resolves the bound GCP project, wraps a
//! native-Gemini request in the Code Assist envelope, and does a non-streaming
//! `:generateContent` call. The agent layer adapts the result into its
//! synthetic response + on_token sink (see the `gemini_oauth` branch in
//! agent.rs), mirroring the claude_agent integration.
//!
//! Token lifecycle: the official `gemini` CLI refreshes `oauth_creds.json` on
//! use, so we simply read the current access token; if it's expired we ask the
//! user to run `gemini` once rather than refreshing it ourselves (no client
//! secret embedded, and we never rewrite the CLI's file).
//!
//! NOTE: using the Gemini CLI's credentials from a third-party client is a
//! Google-ToS gray area, like the other subscription providers.
//!
//! ⚠️ DEPRECATION (verified 2026-06-01): Google announced that the Gemini Code
//! Assist IDE extensions + Gemini CLI **stop serving requests for the
//! individual / AI Pro / AI Ultra tiers on 2026-06-18**, with the agent quota
//! migrating to **Antigravity / Antigravity CLI**. After that date this
//! provider's `cloudcode-pa` calls will fail for those tiers. The successor is
//! an Antigravity backend provider (same `cloudcode-pa` host + different headers
//! and an Antigravity-sourced token); deferred because Antigravity stores its
//! token in a SQLite `state.vscdb` protobuf blob that's awkward to extract.
//! Until then the clean, durable Gemini path remains a BYO API key (the `gemini`
//! provider).
//!
//! Protocol mapped from Block's Goose `gemini_oauth.rs` reference.

use crate::providers::traits::ProviderError;
use serde_json::{json, Value};

const CODE_ASSIST: &str = "https://cloudcode-pa.googleapis.com/v1internal";

fn err(msg: impl Into<String>) -> ProviderError {
    ProviderError::Unknown(msg.into())
}

struct GAuth {
    access_token: String,
    expiry_ms: Option<i64>,
    project: Option<String>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Read `~/.gemini/oauth_creds.json` (+ project hint from `projects.json`).
fn read_creds() -> Option<GAuth> {
    let home = super::credential_scanner::home_dir_lenient()?;
    let creds: Value = serde_json::from_str(
        &std::fs::read_to_string(home.join(".gemini").join("oauth_creds.json")).ok()?,
    )
    .ok()?;
    let access_token = creds
        .get("access_token")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?
        .to_string();
    let project = super::gemini_cli::find_gemini_auth().and_then(|a| a.project_id);
    Some(GAuth {
        access_token,
        expiry_ms: creds.get("expiry_date").and_then(|v| v.as_i64()),
        project,
    })
}

/// Return the access token, or an error if it's expired (60s buffer).
fn current_token(auth: &GAuth) -> Result<String, ProviderError> {
    if auth.expiry_ms.map(|e| e <= now_ms() + 60_000).unwrap_or(false) {
        return Err(err(
            "Gemini token expired — run `gemini` once to refresh ~/.gemini/oauth_creds.json",
        ));
    }
    Ok(auth.access_token.clone())
}

/// Resolve the Code Assist project id: use the hint, else `loadCodeAssist`.
async fn resolve_project(access_token: &str, hint: Option<&str>) -> Result<String, ProviderError> {
    if let Some(p) = hint.filter(|s| !s.is_empty()) {
        return Ok(p.to_string());
    }
    let body = json!({"metadata": {"ideType": "IDE_UNSPECIFIED", "platform": "PLATFORM_UNSPECIFIED", "pluginType": "GEMINI"}});
    let resp = reqwest::Client::new()
        .post(format!("{CODE_ASSIST}:loadCodeAssist"))
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| err(format!("loadCodeAssist failed: {e}")))?;
    let v: Value = resp
        .json()
        .await
        .map_err(|e| err(format!("loadCodeAssist json: {e}")))?;
    v.get("cloudaicompanionProject")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| err("could not resolve a Gemini Code Assist project (run the Gemini CLI once to onboard)"))
}

/// Convert OpenAI-style messages into a native-Gemini request body.
fn to_gemini_request(messages: &[Value], system_extra: Option<&str>) -> Value {
    let text_of = |m: &Value| -> String {
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
    };
    let mut system = system_extra.unwrap_or("").to_string();
    let mut contents = Vec::new();
    for m in messages {
        let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        let text = text_of(m);
        match role {
            "system" => {
                if !system.is_empty() {
                    system.push_str("\n\n");
                }
                system.push_str(&text);
            }
            "assistant" => contents.push(json!({"role": "model", "parts": [{"text": text}]})),
            _ => contents.push(json!({"role": "user", "parts": [{"text": text}]})),
        }
    }
    let mut req = json!({ "contents": contents });
    if !system.is_empty() {
        req["systemInstruction"] = json!({"parts": [{"text": system}]});
    }
    req
}

/// Extract the assistant text from a Code Assist generateContent response,
/// which wraps the standard Gemini response under a `response` key.
fn extract_text(v: &Value) -> String {
    let root = v.get("response").unwrap_or(v);
    root.get("candidates")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// Run one non-streaming Gemini Code Assist completion. Returns the answer text.
pub async fn run_gemini_code_assist(
    messages: &[Value],
    model: &str,
    system: Option<&str>,
) -> Result<String, ProviderError> {
    let auth = read_creds().ok_or_else(|| {
        err("no Gemini CLI credentials — run `gemini` (Google login) once; expected ~/.gemini/oauth_creds.json")
    })?;
    let token = current_token(&auth)?;
    let project = resolve_project(&token, auth.project.as_deref()).await?;
    let model = if model.is_empty() { "gemini-2.5-flash" } else { model };

    let wrapped = json!({
        "model": model,
        "project": project,
        "request": to_gemini_request(messages, system),
    });
    let resp = reqwest::Client::new()
        .post(format!("{CODE_ASSIST}:generateContent"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .json(&wrapped)
        .send()
        .await
        .map_err(|e| err(format!("Code Assist generateContent failed: {e}")))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(err(format!(
            "Code Assist HTTP {}: {}",
            status,
            body.chars().take(200).collect::<String>()
        )));
    }
    let v: Value = serde_json::from_str(&body).map_err(|e| err(format!("Code Assist json: {e}")))?;
    let text = extract_text(&v);
    if text.is_empty() {
        return Err(err("Code Assist returned no text"));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_gemini_request_with_system() {
        let msgs = vec![
            json!({"role": "system", "content": "be terse"}),
            json!({"role": "user", "content": "hi"}),
            json!({"role": "assistant", "content": "hello"}),
        ];
        let req = to_gemini_request(&msgs, None);
        assert_eq!(req["systemInstruction"]["parts"][0]["text"], "be terse");
        assert_eq!(req["contents"][0]["role"], "user");
        assert_eq!(req["contents"][0]["parts"][0]["text"], "hi");
        assert_eq!(req["contents"][1]["role"], "model");
    }

    #[test]
    fn extracts_wrapped_and_unwrapped() {
        let wrapped = json!({"response": {"candidates": [{"content": {"parts": [{"text": "pong"}]}}]}});
        assert_eq!(extract_text(&wrapped), "pong");
        let bare = json!({"candidates": [{"content": {"parts": [{"text": "hi"}, {"text": "!"}]}}]});
        assert_eq!(extract_text(&bare), "hi!");
    }
}
