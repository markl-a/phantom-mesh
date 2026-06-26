//! Mac-side provider round-trip smoke tests.
//!
//! Each test reads its API key from the env. On a healthy dev Mac the
//! shell has already sourced `~/.phantom-mesh/env` before invoking
//! cargo, so the keys are visible. When a key is missing the test
//! eprintln!s and returns — non-fatal so partial-key dev hosts don't
//! get spurious red on `cargo test`.
//!
//! These bypass the gated `core/src/providers/{mistral,groq,...}.rs`
//! modules (`experimental-extra-providers`) and talk to each
//! provider's OpenAI-compatible endpoint directly. They catch the
//! class of regressions that matter day-to-day:
//!   - key rotated by the provider
//!   - endpoint moved / TLS cert expired
//!   - macOS host can't reach the provider (Tailscale ACL, proxy,
//!     corporate MITM)
//! NOT covered: phantom's resolver, retry middleware, streaming
//! codepath — those have dedicated wiremock tests inside
//! `core/src/providers/*.rs`.

#![cfg(target_os = "macos")]

use std::time::Duration;

async fn round_trip(endpoint: &str, key: &str, model: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("client build: {}", e))?;

    let body = serde_json::json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": "Reply with the literal word PONG. Nothing else."
        }],
        "max_tokens": 4,
        "temperature": 0.0,
    });

    let resp = client
        .post(endpoint)
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("POST {}: {}", endpoint, e))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("body read: {}", e))?;

    if !status.is_success() {
        return Err(format!("HTTP {} from {}: {}", status, endpoint, text));
    }

    let j: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("JSON parse: {}\nbody: {}", e, text))?;
    let content = j["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| format!("no choices[0].message.content in: {}", text))?;
    if content.is_empty() {
        return Err(format!("empty completion in: {}", text));
    }
    Ok(content.to_string())
}

/// MAC P0 — Groq round-trip via api.groq.com (OpenAI-compatible).
/// Skipped (eprintln + return) when GROQ_API_KEY unset.
#[tokio::test]
async fn round_trip_macos_groq() {
    let key = match std::env::var("GROQ_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            eprintln!(
                "SKIPPED: round_trip_macos_groq — GROQ_API_KEY unset \
                 (source ~/.phantom-mesh/env first)"
            );
            return;
        }
    };
    match round_trip(
        "https://api.groq.com/openai/v1/chat/completions",
        &key,
        "llama-3.1-8b-instant",
    )
    .await
    {
        Ok(out) => println!("Groq replied: {}", out.trim()),
        Err(e) => panic!("Groq round-trip failed: {}", e),
    }
}

/// MAC P0 — Mistral round-trip via api.mistral.ai (OpenAI-compatible).
/// Skipped when MISTRAL_API_KEY unset.
#[tokio::test]
async fn round_trip_macos_mistral() {
    let key = match std::env::var("MISTRAL_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            eprintln!(
                "SKIPPED: round_trip_macos_mistral — MISTRAL_API_KEY unset \
                 (source ~/.phantom-mesh/env first)"
            );
            return;
        }
    };
    match round_trip(
        "https://api.mistral.ai/v1/chat/completions",
        &key,
        "mistral-small-latest",
    )
    .await
    {
        Ok(out) => println!("Mistral replied: {}", out.trim()),
        Err(e) => panic!("Mistral round-trip failed: {}", e),
    }
}
