//! `image_generate` — text → image via OpenAI-shape `/v1/images/generations`.
//!
//! Closes the v0.6.0 gap noted in doc 28 §5: spectyn can *consume* images
//! (via the multimodal sentinel pipeline in `core/src/multimodal.rs`) but
//! cannot *produce* them. With this tool an agent can say "generate a
//! 1024×1024 isometric illustration of a Rust crab" and get a PNG path
//! back, which it can then `file_read` or `@`-attach to a vision provider
//! for follow-up edits.
//!
//! ## Provider routing
//!
//! Calls the OpenAI Images endpoint by default. Any provider with an
//! OpenAI-compatible `/images/generations` route works (e.g. Together
//! AI's `black-forest-labs/FLUX.1-schnell`, Stability AI's compat
//! endpoint). Override via env:
//!
//! - `SPECTYN_IMAGE_GEN_BASE_URL` (default `https://api.openai.com/v1`)
//! - `SPECTYN_IMAGE_GEN_API_KEY`  (falls back to `OPENAI_API_KEY`)
//! - `SPECTYN_IMAGE_GEN_MODEL`    (default `dall-e-3`)
//!
//! ## Output
//!
//! The decoded PNG bytes are written to
//! `<home>/.spectyn-mesh/generated/<unix-ts>-<n>.png` and the path is
//! returned to the caller. The path can be fed straight back to the agent
//! via the multimodal `@`-attach sentinel — closes the round-trip
//! (generate → look at it → describe → edit).

use base64::Engine;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "dall-e-3";
const TIMEOUT_SECS: u64 = 120;

/// Max prompt length we'll pass through. OpenAI's DALL-E 3 limit is
/// 4000 chars; we cap a little under to leave room for system additions.
const MAX_PROMPT_CHARS: usize = 3800;

/// Where generated PNGs land. `<home>/.spectyn-mesh/generated/`.
fn output_dir() -> std::io::Result<PathBuf> {
    let data = crate::cli_config::spectyn_data_dir()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, e.to_string()))?;
    let dir = data.join("generated");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// MCP tool entry-point.
///
/// Required args:
/// - `prompt` (string)
///
/// Optional args:
/// - `model`  (string, default from `SPECTYN_IMAGE_GEN_MODEL` or `dall-e-3`)
/// - `size`   (string, default `"1024x1024"` — must be one the provider supports)
/// - `n`      (integer, default 1, capped to 4)
/// - `style`  (string — provider-specific, e.g. `"vivid"` / `"natural"` for DALL-E 3)
pub async fn generate(args: &Value) -> String {
    let prompt = match args.get("prompt").and_then(|v| v.as_str()) {
        Some(p) if !p.trim().is_empty() => p.trim(),
        _ => return "Error: missing required string argument 'prompt'".into(),
    };
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        return format!(
            "Error: prompt is {} chars; cap is {} chars",
            prompt.chars().count(),
            MAX_PROMPT_CHARS,
        );
    }

    let base_url = std::env::var("SPECTYN_IMAGE_GEN_BASE_URL")
        .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
    let api_key = std::env::var("SPECTYN_IMAGE_GEN_API_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .unwrap_or_default();
    if api_key.is_empty() {
        return "Error: no API key — set SPECTYN_IMAGE_GEN_API_KEY or OPENAI_API_KEY".into();
    }

    let model = args
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            std::env::var("SPECTYN_IMAGE_GEN_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string())
        });
    let size = args
        .get("size")
        .and_then(|v| v.as_str())
        .unwrap_or("1024x1024");
    let n = args
        .get("n")
        .and_then(|v| v.as_u64())
        .map(|v| v.clamp(1, 4))
        .unwrap_or(1);

    let mut req_body = json!({
        "model":           model,
        "prompt":          prompt,
        "n":               n,
        "size":            size,
        "response_format": "b64_json",
    });
    if let Some(style) = args.get("style").and_then(|v| v.as_str()) {
        req_body["style"] = json!(style);
    }

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .build()
    {
        Ok(c) => c,
        Err(e) => return format!("Error: building HTTP client: {e}"),
    };
    let url = format!("{}/images/generations", base_url.trim_end_matches('/'));
    let resp = match client
        .post(&url)
        .bearer_auth(&api_key)
        .header("Content-Type", "application/json")
        .json(&req_body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return format!("Error: request to {url}: {e}"),
    };

    let status = resp.status();
    let body_text = match resp.text().await {
        Ok(t) => t,
        Err(e) => return format!("Error: reading response body: {e}"),
    };
    if !status.is_success() {
        return format!(
            "Error: provider returned {} — {}",
            status.as_u16(),
            body_text.chars().take(400).collect::<String>(),
        );
    }

    let parsed: Value = match serde_json::from_str(&body_text) {
        Ok(v) => v,
        Err(e) => return format!("Error: parsing JSON body: {e}\n{body_text}"),
    };
    let data = match parsed.get("data").and_then(|v| v.as_array()) {
        Some(arr) if !arr.is_empty() => arr,
        _ => return format!("Error: response has no 'data' array: {body_text}"),
    };

    let dir = match output_dir() {
        Ok(d) => d,
        Err(e) => return format!("Error: preparing output dir: {e}"),
    };
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut paths: Vec<String> = Vec::with_capacity(data.len());
    for (i, item) in data.iter().enumerate() {
        let b64 = match item.get("b64_json").and_then(|v| v.as_str()) {
            Some(b) => b,
            None => {
                // Some providers honour response_format weakly and return `url`
                // instead. If so, surface a clear error rather than silently
                // skipping the result.
                if let Some(url) = item.get("url").and_then(|v| v.as_str()) {
                    return format!(
                        "Error: provider returned URL instead of b64_json — \
                         '{url}'. Set response_format=b64_json on the provider \
                         or download manually."
                    );
                }
                return format!("Error: data[{i}] missing 'b64_json': {item}");
            }
        };
        let bytes = match base64::engine::general_purpose::STANDARD.decode(b64) {
            Ok(b) => b,
            Err(e) => return format!("Error: decoding base64 for data[{i}]: {e}"),
        };
        let path = dir.join(format!("{ts}-{i}.png"));
        if let Err(e) = std::fs::write(&path, &bytes) {
            return format!("Error: writing {}: {e}", path.display());
        }
        paths.push(path.display().to_string());
    }

    if paths.len() == 1 {
        format!("Generated 1 image:\n{}", paths[0])
    } else {
        format!("Generated {} images:\n{}", paths.len(), paths.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn missing_prompt_returns_error() {
        let out = generate(&json!({})).await;
        assert!(
            out.starts_with("Error: missing required string argument 'prompt'"),
            "got: {out}",
        );
    }

    #[tokio::test]
    async fn empty_prompt_returns_error() {
        let out = generate(&json!({"prompt": "   "})).await;
        assert!(out.starts_with("Error: missing required"), "got: {out}");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // generate() returns early on the key
    // check (no real I/O); the lock just serializes OPENAI_API_KEY mutation
    // against prompt_too_long_returns_error.
    async fn missing_api_key_returns_error() {
        let _g = crate::env_lock::acquire();
        // Make sure the env is clean for this test.
        let prev_spectyn = std::env::var("SPECTYN_IMAGE_GEN_API_KEY").ok();
        let prev_openai = std::env::var("OPENAI_API_KEY").ok();
        std::env::remove_var("SPECTYN_IMAGE_GEN_API_KEY");
        std::env::remove_var("OPENAI_API_KEY");

        let out = generate(&json!({"prompt": "a red crab"})).await;
        assert!(out.starts_with("Error: no API key"), "got: {out}");

        // Restore.
        if let Some(v) = prev_spectyn {
            std::env::set_var("SPECTYN_IMAGE_GEN_API_KEY", v);
        }
        if let Some(v) = prev_openai {
            std::env::set_var("OPENAI_API_KEY", v);
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // see missing_api_key_returns_error
    async fn prompt_too_long_returns_error() {
        let _g = crate::env_lock::acquire();
        let huge = "a".repeat(MAX_PROMPT_CHARS + 1);
        // API key needs to be present to get past the earlier guard.
        std::env::set_var("OPENAI_API_KEY", "test-stub");
        let out = generate(&json!({"prompt": huge})).await;
        std::env::remove_var("OPENAI_API_KEY");
        assert!(out.contains("cap is"), "got: {out}");
    }
}
