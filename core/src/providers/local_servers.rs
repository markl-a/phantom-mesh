//! Detection of local, OpenAI-compatible LLM servers (Ollama / LM Studio /
//! Lemonade). These are **per-machine, local-first** providers: each runs on
//! `localhost`, needs no API key, and is free. Onboarding probes the standard
//! ports so a machine auto-discovers whatever the operator already has running.
//!
//! All three speak the OpenAI `chat/completions` wire format, so once detected
//! they map to spectyn's default OpenAI-compat provider with a localhost
//! `base_url` (see [provider-routing]). We only *probe* `/models`; nothing is
//! started or installed.

use serde_json::Value;

/// A detected local server + the models it advertises.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LocalServer {
    /// Stable slug (`"ollama"` / `"lmstudio"` / `"lemonade"`).
    pub name: String,
    /// OpenAI-compatible base URL (e.g. `http://127.0.0.1:11434/v1`).
    pub base_url: String,
    /// Model ids the server's `/models` endpoint reports.
    pub models: Vec<String>,
}

/// (slug, base_url) candidates to probe. Lemonade ships on either port across
/// versions, so both are listed; the first that answers wins per slug.
const CANDIDATES: &[(&str, &str)] = &[
    ("ollama", "http://127.0.0.1:11434/v1"),
    ("lmstudio", "http://127.0.0.1:1234/v1"),
    ("lemonade", "http://127.0.0.1:8000/api/v1"),
    ("lemonade", "http://127.0.0.1:13305/api/v1"),
];

/// P0-7 SYS-B always-available offline model. Not a real server — a built-in
/// deterministic completion path the runtime recognises by `type = "stub"` /
/// `url = "stub://offline"`. Feature-gated so it never ships on by default; the
/// zero-config offline cold-start uses it as the final, always-usable tail so a
/// desktop with nothing installed never dead-ends (SPEC-03 §8 no-dead-end).
/// Tuple is `(block_name, provider_type, url)`.
#[cfg(feature = "offline-stub-model")]
pub const STUB_PROVIDER: (&str, &str, &str) = ("local-stub", "stub", "stub://offline");

/// The canned `default_model` written for the stub block — obviously-a-stub so
/// it can never be mistaken for a real model.
#[cfg(feature = "offline-stub-model")]
pub const STUB_DEFAULT_MODEL: &str = "stub-echo";

/// Parse model ids from an OpenAI-style `{"data":[{"id":..}]}` or an
/// Ollama-style `{"models":[..]}` / `{"data":[..]}` payload.
fn parse_models(body: &str) -> Option<Vec<String>> {
    let v: Value = serde_json::from_str(body).ok()?;
    let arr = v
        .get("data")
        .and_then(|d| d.as_array())
        .or_else(|| v.get("models").and_then(|m| m.as_array()))?;
    let ids: Vec<String> = arr
        .iter()
        .filter_map(|m| {
            m.get("id")
                .and_then(|i| i.as_str())
                .or_else(|| m.get("name").and_then(|n| n.as_str()))
                .map(String::from)
        })
        .collect();
    Some(ids)
}

/// Probe the standard local-server ports and return the ones that answer.
/// Each probe has a short timeout; unreachable servers are silently skipped.
pub async fn detect_local_servers() -> Vec<LocalServer> {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(1500))
        .build()
    {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut found: Vec<LocalServer> = Vec::new();
    for (name, base_url) in CANDIDATES {
        // One server per slug: skip a second Lemonade port if the first answered.
        if found.iter().any(|s| s.name == *name) {
            continue;
        }
        let url = format!("{}/models", base_url);
        let Ok(resp) = client.get(&url).send().await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(body) = resp.text().await else { continue };
        if let Some(models) = parse_models(&body) {
            found.push(LocalServer {
                name: (*name).to_string(),
                base_url: (*base_url).to_string(),
                models,
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_models_shape() {
        let body = r#"{"object":"list","data":[{"id":"llama3.1:8b"},{"id":"qwen2.5"}]}"#;
        assert_eq!(
            parse_models(body).unwrap(),
            vec!["llama3.1:8b".to_string(), "qwen2.5".to_string()]
        );
    }

    #[test]
    fn parses_ollama_models_shape() {
        let body = r#"{"models":[{"name":"mistral:latest"}]}"#;
        assert_eq!(parse_models(body).unwrap(), vec!["mistral:latest".to_string()]);
    }

    #[test]
    fn none_on_garbage() {
        assert!(parse_models("not json").is_none());
        assert!(parse_models(r#"{"oops":1}"#).is_none());
    }

    /// Live probe against this machine's localhost. Ignored by default (result
    /// is machine-dependent); run with `--ignored` to see what's detected.
    #[tokio::test]
    #[ignore]
    async fn live_detect() {
        let servers = detect_local_servers().await;
        eprintln!(
            "detected {} local server(s): {:?}",
            servers.len(),
            servers
                .iter()
                .map(|s| format!("{}@{} ({} models)", s.name, s.base_url, s.models.len()))
                .collect::<Vec<_>>()
        );
    }
}
