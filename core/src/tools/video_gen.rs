//! `video_generate` — text/image → mp4 via async-job providers.
//!
//! Closes the rest of the doc 28 §5 v0.6.0 media-gen gap. Where
//! `image_generate` is one synchronous POST, video generation
//! universally uses **create-job + poll-status + download-url**
//! because frame inference takes 10s–2min. This module bundles a
//! generic polling loop with provider-specific wire formats so an
//! agent can say "10-second 16:9 clip of a phantom mesh forming"
//! and get an mp4 path back the same way `image_generate` returns
//! a PNG path.
//!
//! ## Supported providers
//!
//! Selected via `PHANTOM_VIDEO_GEN_PROVIDER`:
//!
//! | value (default `replicate`) | provider | default model | notes |
//! |---|---|---|---|
//! | `replicate` | Replicate `/v1/predictions` | `lightricks/ltx-video` | $0.05/clip, fastest; 7+ models via `model` arg |
//! | `openai-sora` (alias `sora`) | OpenAI Sora `/v1/videos` | `sora-2` | Same auth shape as `image_generate`; $$ but cleanest |
//! | `luma` | Luma Dream Machine `/dream-machine/v1/generations` | `ray-2` | Text-to-video only (i2v needs prior upload) |
//! | `fal` | Fal.ai `/queue/{model}` | `fal-ai/ltx-video` | Sub-minute queue API |
//!
//! Env overrides (all optional except API key):
//! - `PHANTOM_VIDEO_GEN_PROVIDER` — see table
//! - `PHANTOM_VIDEO_GEN_BASE_URL` — override the provider base URL
//! - `PHANTOM_VIDEO_GEN_API_KEY`  — fallback per-provider env (REPLICATE_API_TOKEN / OPENAI_API_KEY / LUMA_API_KEY / FAL_KEY)
//! - `PHANTOM_VIDEO_GEN_MODEL`    — override the model id
//! - `PHANTOM_VIDEO_GEN_TIMEOUT_SECS` — default 600 (= 10 min)
//!
//! ## Output
//!
//! mp4 lands in `~/.phantom-mesh/generated/<unix-ts>.mp4`. Returns
//! the path so the agent can `file_read` or `@`-attach for a
//! follow-up vision turn (frame-by-frame description, e.g.).

use base64::Engine;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_TIMEOUT_SECS: u64 = 600;
const POLL_INTERVAL_SECS: u64 = 3;
const HTTP_TIMEOUT_SECS: u64 = 60;

/// Hard cap on image bytes for i2v starting-frame input. Same rationale
/// as `multimodal::MAX_IMAGE_BYTES`: anything over 20 MiB is almost
/// certainly a mistake.
const MAX_I2V_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Replicate,
    OpenAISora,
    Luma,
    Fal,
}

impl Provider {
    fn from_env() -> Self {
        match std::env::var("PHANTOM_VIDEO_GEN_PROVIDER")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "openai-sora" | "sora" | "openai_sora" => Self::OpenAISora,
            "luma" | "lumalabs" => Self::Luma,
            "fal" | "fal-ai" | "fal.ai" => Self::Fal,
            _ => Self::Replicate, // default
        }
    }

    fn default_base_url(self) -> &'static str {
        match self {
            Self::Replicate => "https://api.replicate.com/v1",
            Self::OpenAISora => "https://api.openai.com/v1",
            Self::Luma => "https://api.lumalabs.ai/dream-machine/v1",
            Self::Fal => "https://queue.fal.run",
        }
    }

    fn default_model(self) -> &'static str {
        match self {
            Self::Replicate => "lightricks/ltx-video",
            Self::OpenAISora => "sora-2",
            Self::Luma => "ray-2",
            Self::Fal => "fal-ai/ltx-video",
        }
    }

    /// Env vars to try IN ORDER for the API key.
    fn api_key_envs(self) -> &'static [&'static str] {
        match self {
            Self::Replicate => &["PHANTOM_VIDEO_GEN_API_KEY", "REPLICATE_API_TOKEN"],
            Self::OpenAISora => &["PHANTOM_VIDEO_GEN_API_KEY", "OPENAI_API_KEY"],
            Self::Luma => &["PHANTOM_VIDEO_GEN_API_KEY", "LUMA_API_KEY"],
            Self::Fal => &["PHANTOM_VIDEO_GEN_API_KEY", "FAL_KEY"],
        }
    }

    fn auth_header_value(self, key: &str) -> String {
        match self {
            Self::Replicate => format!("Token {key}"),
            Self::Fal => format!("Key {key}"),
            _ => format!("Bearer {key}"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Replicate => "replicate",
            Self::OpenAISora => "openai-sora",
            Self::Luma => "luma",
            Self::Fal => "fal",
        }
    }
}

/// MCP tool entry-point.
///
/// Required args:
/// - `prompt` (string)
///
/// Optional args:
/// - `model`         (string — overrides env)
/// - `duration_secs` (integer 1-10, default 5)
/// - `aspect_ratio`  (string, default `"16:9"` — provider-dependent)
/// - `image`         (string — local path OR https URL to use as the
///   starting frame for image-to-video. Local files are base64-encoded
///   to a `data:` URL. Not supported on `luma` — needs CDN upload.)
pub async fn generate(args: &Value) -> String {
    let prompt = match args.get("prompt").and_then(|v| v.as_str()) {
        Some(p) if !p.trim().is_empty() => p.trim().to_string(),
        _ => return "Error: missing required string argument 'prompt'".into(),
    };

    let provider = Provider::from_env();
    let api_key = read_api_key(provider);
    if api_key.is_empty() {
        return format!(
            "Error: no API key for provider '{}' — set PHANTOM_VIDEO_GEN_API_KEY or one of [{}]",
            provider.name(),
            provider.api_key_envs().join(", "),
        );
    }

    let base_url = std::env::var("PHANTOM_VIDEO_GEN_BASE_URL")
        .unwrap_or_else(|_| provider.default_base_url().to_string());
    let model = args
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            std::env::var("PHANTOM_VIDEO_GEN_MODEL")
                .unwrap_or_else(|_| provider.default_model().to_string())
        });
    let duration_secs = args
        .get("duration_secs")
        .and_then(|v| v.as_u64())
        .map(|v| v.clamp(1, 10))
        .unwrap_or(5);
    let aspect = args
        .get("aspect_ratio")
        .and_then(|v| v.as_str())
        .unwrap_or("16:9")
        .to_string();

    let image_input: Option<String> = match args.get("image").and_then(|v| v.as_str()) {
        None => None,
        Some(s) if s.starts_with("http://") || s.starts_with("https://") => Some(s.to_string()),
        Some(local) => match image_path_to_data_url(local).await {
            Ok(url) => Some(url),
            Err(e) => return format!("Error: i2v image '{local}': {e}"),
        },
    };
    if provider == Provider::Luma && image_input.is_some() {
        return "Error: Luma i2v needs a prior CDN upload — not supported in this tool. \
                Use replicate / openai-sora / fal for image-to-video."
            .into();
    }

    let timeout_secs = std::env::var("PHANTOM_VIDEO_GEN_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_TIMEOUT_SECS);

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build()
    {
        Ok(c) => c,
        Err(e) => return format!("Error: building HTTP client: {e}"),
    };

    let params = VideoParams {
        prompt: &prompt,
        model: &model,
        duration_secs,
        aspect: &aspect,
        image_data: image_input.as_deref(),
    };

    // 1. Create the job.
    let create = match create_job(provider, &client, &base_url, &api_key, &params).await {
        Ok(c) => c,
        Err(e) => return format!("Error: creating {} job: {e}", provider.name()),
    };

    // 2. Poll.
    let video_url = match poll_until_done(
        provider,
        &client,
        &base_url,
        &api_key,
        &create,
        Duration::from_secs(timeout_secs),
    )
    .await
    {
        Ok(u) => u,
        Err(e) => {
            return format!(
                "Error: polling {} job '{}': {e}",
                provider.name(),
                create.id
            )
        }
    };

    // 3. Download to local file.
    let dir = match output_dir() {
        Ok(d) => d,
        Err(e) => return format!("Error: preparing output dir: {e}"),
    };
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dest = dir.join(format!("{ts}.mp4"));
    match download_to_file(&client, &video_url, &dest).await {
        Ok(bytes) => format!(
            "Generated 1 video ({} bytes, {} provider, model={}):\n{}",
            bytes,
            provider.name(),
            model,
            dest.display(),
        ),
        Err(e) => format!("Error: downloading {video_url}: {e}"),
    }
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn output_dir() -> std::io::Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "home dir not found"))?;
    let dir = home.join(".phantom-mesh").join("generated");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn read_api_key(provider: Provider) -> String {
    for env in provider.api_key_envs() {
        if let Ok(v) = std::env::var(env) {
            if !v.is_empty() {
                return v;
            }
        }
    }
    String::new()
}

async fn image_path_to_data_url(path: &str) -> Result<String, String> {
    let p = Path::new(path);
    let mime = match p
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
    {
        Some(ref e) if e == "png" => "image/png",
        Some(ref e) if e == "jpg" || e == "jpeg" => "image/jpeg",
        Some(ref e) if e == "webp" => "image/webp",
        Some(ref e) if e == "gif" => "image/gif",
        other => return Err(format!("unsupported image extension: {other:?}")),
    };
    let meta = std::fs::metadata(p).map_err(|e| format!("metadata: {e}"))?;
    if meta.len() > MAX_I2V_IMAGE_BYTES {
        return Err(format!(
            "i2v image is {} bytes; cap is {} bytes",
            meta.len(),
            MAX_I2V_IMAGE_BYTES,
        ));
    }
    let bytes = tokio::fs::read(p).await.map_err(|e| format!("read: {e}"))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{mime};base64,{b64}"))
}

// ─── per-provider create + poll ──────────────────────────────────────────────

#[derive(Debug)]
struct VideoParams<'a> {
    prompt: &'a str,
    model: &'a str,
    duration_secs: u64,
    aspect: &'a str,
    image_data: Option<&'a str>,
}

#[derive(Debug)]
struct JobCreated {
    /// Provider's job id.
    id: String,
    /// Optional status URL given by the provider (Fal uses this).
    status_url: Option<String>,
}

async fn create_job(
    provider: Provider,
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    params: &VideoParams<'_>,
) -> Result<JobCreated, String> {
    let base = base_url.trim_end_matches('/');
    let auth = provider.auth_header_value(api_key);
    let (url, body) = match provider {
        Provider::Replicate => {
            // Use the model-name route: POST /v1/models/{owner}/{name}/predictions
            // — newer than the version-hash route and doesn't require the agent
            // to know the latest version hash.
            let model = params.model;
            let url = format!("{base}/models/{model}/predictions");
            let mut input = json!({
                "prompt":   params.prompt,
                "duration": params.duration_secs,
                "aspect_ratio": params.aspect,
            });
            if let Some(img) = params.image_data {
                input["image"] = json!(img);
            }
            (url, json!({ "input": input }))
        }
        Provider::OpenAISora => {
            let url = format!("{base}/videos");
            let mut body = json!({
                "model":   params.model,
                "prompt":  params.prompt,
                "seconds": params.duration_secs,
                "aspect_ratio": params.aspect,
            });
            if let Some(img) = params.image_data {
                body["input_image"] = json!(img);
            }
            (url, body)
        }
        Provider::Luma => {
            // Luma's `duration` is a string like "5s" / "10s".
            let url = format!("{base}/generations");
            let body = json!({
                "model":        params.model,
                "prompt":       params.prompt,
                "aspect_ratio": params.aspect,
                "duration":     format!("{}s", params.duration_secs),
            });
            (url, body)
        }
        Provider::Fal => {
            // Fal queue API: POST /queue/{model}
            let model = params.model;
            let url = format!("{base}/{model}");
            let mut body = json!({
                "prompt":        params.prompt,
                "aspect_ratio":  params.aspect,
                "duration":      params.duration_secs,
            });
            if let Some(img) = params.image_data {
                body["image_url"] = json!(img);
            }
            (url, body)
        }
    };

    let resp = client
        .post(&url)
        .header("Authorization", &auth)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "{} returned {} — {}",
            url,
            status.as_u16(),
            body_text.chars().take(400).collect::<String>(),
        ));
    }
    let parsed: Value = serde_json::from_str(&body_text)
        .map_err(|e| format!("non-JSON create response: {e} — {body_text}"))?;
    parse_create_response(provider, &parsed)
}

fn parse_create_response(provider: Provider, resp: &Value) -> Result<JobCreated, String> {
    let id = match provider {
        Provider::Replicate | Provider::OpenAISora | Provider::Luma => resp
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("missing 'id': {resp}"))?
            .to_string(),
        Provider::Fal => resp
            .get("request_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("missing 'request_id': {resp}"))?
            .to_string(),
    };
    let status_url = resp
        .get("status_url")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Ok(JobCreated { id, status_url })
}

async fn poll_until_done(
    provider: Provider,
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    job: &JobCreated,
    overall_timeout: Duration,
) -> Result<String, String> {
    let base = base_url.trim_end_matches('/');
    let auth = provider.auth_header_value(api_key);
    let start = std::time::Instant::now();

    let status_url = match provider {
        Provider::Replicate => format!("{base}/predictions/{}", job.id),
        Provider::OpenAISora => format!("{base}/videos/{}", job.id),
        Provider::Luma => format!("{base}/generations/{}", job.id),
        Provider::Fal => job
            .status_url
            .clone()
            .ok_or_else(|| "Fal create response missing status_url".to_string())?,
    };

    loop {
        if start.elapsed() > overall_timeout {
            return Err(format!(
                "timeout after {} s (set PHANTOM_VIDEO_GEN_TIMEOUT_SECS to extend)",
                overall_timeout.as_secs()
            ));
        }
        let resp = client
            .get(&status_url)
            .header("Authorization", &auth)
            .send()
            .await
            .map_err(|e| format!("GET {status_url}: {e}"))?;
        let status_code = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        if !status_code.is_success() {
            return Err(format!(
                "{status_url} returned {} — {}",
                status_code.as_u16(),
                body_text.chars().take(400).collect::<String>(),
            ));
        }
        let parsed: Value = serde_json::from_str(&body_text)
            .map_err(|e| format!("non-JSON status response: {e} — {body_text}"))?;
        match parse_status(provider, &parsed) {
            JobStatus::Done(url) => return Ok(url),
            JobStatus::Failed(msg) => return Err(format!("provider failed: {msg}")),
            JobStatus::Running => {
                tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
            }
        }
    }
}

#[derive(Debug)]
enum JobStatus {
    Running,
    Done(String),
    Failed(String),
}

fn parse_status(provider: Provider, resp: &Value) -> JobStatus {
    match provider {
        Provider::Replicate => {
            let status = resp.get("status").and_then(|v| v.as_str()).unwrap_or("");
            match status {
                "succeeded" => {
                    // `output` is either a string URL or an array of URLs.
                    let url = resp
                        .get("output")
                        .and_then(|v| {
                            v.as_str().map(str::to_string).or_else(|| {
                                v.as_array()
                                    .and_then(|a| a.first())
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string)
                            })
                        })
                        .unwrap_or_default();
                    if url.is_empty() {
                        JobStatus::Failed("succeeded but no output URL".into())
                    } else {
                        JobStatus::Done(url)
                    }
                }
                "failed" | "canceled" => JobStatus::Failed(
                    resp.get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or(status)
                        .to_string(),
                ),
                _ => JobStatus::Running,
            }
        }
        Provider::OpenAISora => {
            let status = resp.get("status").and_then(|v| v.as_str()).unwrap_or("");
            match status {
                "completed" | "succeeded" => {
                    // OpenAI Sora returns a `url` field or content under `assets.video`.
                    let url = resp
                        .get("url")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or_else(|| {
                            resp.get("assets")
                                .and_then(|a| a.get("video"))
                                .and_then(|v| v.as_str())
                                .map(str::to_string)
                        })
                        .or_else(|| {
                            resp.get("video")
                                .and_then(|v| v.as_str())
                                .map(str::to_string)
                        })
                        .unwrap_or_default();
                    if url.is_empty() {
                        JobStatus::Failed("completed but no video URL".into())
                    } else {
                        JobStatus::Done(url)
                    }
                }
                "failed" | "canceled" | "cancelled" => JobStatus::Failed(
                    resp.get("error")
                        .and_then(|v| v.get("message"))
                        .and_then(|v| v.as_str())
                        .unwrap_or(status)
                        .to_string(),
                ),
                _ => JobStatus::Running,
            }
        }
        Provider::Luma => {
            let state = resp.get("state").and_then(|v| v.as_str()).unwrap_or("");
            match state {
                "completed" => {
                    let url = resp
                        .get("assets")
                        .and_then(|a| a.get("video"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if url.is_empty() {
                        JobStatus::Failed("completed but no assets.video".into())
                    } else {
                        JobStatus::Done(url)
                    }
                }
                "failed" => JobStatus::Failed(
                    resp.get("failure_reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or(state)
                        .to_string(),
                ),
                _ => JobStatus::Running,
            }
        }
        Provider::Fal => {
            let status = resp.get("status").and_then(|v| v.as_str()).unwrap_or("");
            match status {
                "COMPLETED" => {
                    // Fal completed-status response carries the full payload —
                    // including the video URL under `video.url` or `output.video.url`.
                    let url = resp
                        .get("video")
                        .and_then(|v| v.get("url"))
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            resp.get("output")
                                .and_then(|o| o.get("video"))
                                .and_then(|v| v.get("url"))
                                .and_then(|v| v.as_str())
                        })
                        .unwrap_or("")
                        .to_string();
                    if url.is_empty() {
                        JobStatus::Failed("COMPLETED but no video.url".into())
                    } else {
                        JobStatus::Done(url)
                    }
                }
                "FAILED" | "CANCELLED" => JobStatus::Failed(
                    resp.get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or(status)
                        .to_string(),
                ),
                _ => JobStatus::Running,
            }
        }
    }
}

async fn download_to_file(client: &reqwest::Client, url: &str, dest: &Path) -> Result<u64, String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GET {url} returned {}", resp.status().as_u16()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("reading body: {e}"))?;
    tokio::fs::write(dest, &bytes)
        .await
        .map_err(|e| format!("writing {}: {e}", dest.display()))?;
    Ok(bytes.len() as u64)
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn clean_env() {
        for v in [
            "PHANTOM_VIDEO_GEN_PROVIDER",
            "PHANTOM_VIDEO_GEN_API_KEY",
            "PHANTOM_VIDEO_GEN_BASE_URL",
            "PHANTOM_VIDEO_GEN_MODEL",
            "PHANTOM_VIDEO_GEN_TIMEOUT_SECS",
            "REPLICATE_API_TOKEN",
            "OPENAI_API_KEY",
            "LUMA_API_KEY",
            "FAL_KEY",
        ] {
            std::env::remove_var(v);
        }
    }

    #[test]
    fn provider_from_env_default_is_replicate() {
        let _g = crate::sandbox::test_lock();
        clean_env();
        assert_eq!(Provider::from_env(), Provider::Replicate);
    }

    #[test]
    fn provider_from_env_aliases() {
        let _g = crate::sandbox::test_lock();
        clean_env();
        std::env::set_var("PHANTOM_VIDEO_GEN_PROVIDER", "sora");
        assert_eq!(Provider::from_env(), Provider::OpenAISora);
        std::env::set_var("PHANTOM_VIDEO_GEN_PROVIDER", "OpenAI-Sora");
        assert_eq!(Provider::from_env(), Provider::OpenAISora);
        std::env::set_var("PHANTOM_VIDEO_GEN_PROVIDER", "luma");
        assert_eq!(Provider::from_env(), Provider::Luma);
        std::env::set_var("PHANTOM_VIDEO_GEN_PROVIDER", "fal.ai");
        assert_eq!(Provider::from_env(), Provider::Fal);
        clean_env();
    }

    #[test]
    fn auth_header_per_provider() {
        assert_eq!(Provider::Replicate.auth_header_value("xyz"), "Token xyz");
        assert_eq!(Provider::OpenAISora.auth_header_value("xyz"), "Bearer xyz");
        assert_eq!(Provider::Luma.auth_header_value("xyz"), "Bearer xyz");
        assert_eq!(Provider::Fal.auth_header_value("xyz"), "Key xyz");
    }

    #[tokio::test]
    async fn missing_prompt_returns_error() {
        let out = generate(&json!({})).await;
        assert!(
            out.starts_with("Error: missing required string argument 'prompt'"),
            "got: {out}"
        );
    }

    #[tokio::test]
    async fn missing_api_key_returns_error() {
        let _g = crate::sandbox::test_lock();
        clean_env();
        let out = generate(&json!({"prompt": "a phantom mesh"})).await;
        assert!(out.starts_with("Error: no API key"), "got: {out}");
        assert!(
            out.contains("REPLICATE_API_TOKEN"),
            "should hint at REPLICATE_API_TOKEN: {out}"
        );
    }

    #[tokio::test]
    async fn luma_i2v_rejected() {
        let _g = crate::sandbox::test_lock();
        clean_env();
        std::env::set_var("PHANTOM_VIDEO_GEN_PROVIDER", "luma");
        std::env::set_var("LUMA_API_KEY", "stub-key");
        // Use an https URL so we skip the local-file read.
        let out = generate(&json!({
            "prompt": "test",
            "image":  "https://example.com/a.png",
        }))
        .await;
        clean_env();
        assert!(
            out.contains("Luma i2v needs a prior CDN upload"),
            "got: {out}",
        );
    }

    #[test]
    fn parse_status_replicate_succeeded_string_output() {
        let r = parse_status(
            Provider::Replicate,
            &json!({"status": "succeeded", "output": "https://cdn.replicate.com/x.mp4"}),
        );
        assert!(matches!(r, JobStatus::Done(ref u) if u.ends_with("x.mp4")));
    }

    #[test]
    fn parse_status_replicate_succeeded_array_output() {
        let r = parse_status(
            Provider::Replicate,
            &json!({"status": "succeeded", "output": ["https://cdn.replicate.com/y.mp4"]}),
        );
        assert!(matches!(r, JobStatus::Done(ref u) if u.ends_with("y.mp4")));
    }

    #[test]
    fn parse_status_replicate_failed() {
        let r = parse_status(
            Provider::Replicate,
            &json!({"status": "failed", "error": "OOM"}),
        );
        assert!(
            matches!(r, JobStatus::Failed(ref m) if m == "OOM"),
            "got: {r:?}"
        );
    }

    #[test]
    fn parse_status_replicate_running() {
        let r = parse_status(Provider::Replicate, &json!({"status": "processing"}));
        assert!(matches!(r, JobStatus::Running));
    }

    #[test]
    fn parse_status_sora_completed() {
        let r = parse_status(
            Provider::OpenAISora,
            &json!({"status": "completed", "url": "https://cdn.openai.com/v.mp4"}),
        );
        assert!(matches!(r, JobStatus::Done(ref u) if u.ends_with("v.mp4")));
    }

    #[test]
    fn parse_status_luma_completed() {
        let r = parse_status(
            Provider::Luma,
            &json!({"state": "completed", "assets": {"video": "https://cdn.lumalabs.ai/x.mp4"}}),
        );
        assert!(matches!(r, JobStatus::Done(ref u) if u.ends_with("x.mp4")));
    }

    #[test]
    fn parse_status_fal_completed_nested_url() {
        let r = parse_status(
            Provider::Fal,
            &json!({"status": "COMPLETED", "video": {"url": "https://fal.media/x.mp4"}}),
        );
        assert!(matches!(r, JobStatus::Done(ref u) if u.ends_with("x.mp4")));
    }

    #[test]
    fn parse_create_response_replicate_id() {
        let r = parse_create_response(Provider::Replicate, &json!({"id": "rp_abc"})).unwrap();
        assert_eq!(r.id, "rp_abc");
        assert!(r.status_url.is_none());
    }

    #[test]
    fn parse_create_response_fal_request_id_and_status_url() {
        let r = parse_create_response(
            Provider::Fal,
            &json!({"request_id": "fal_xyz", "status_url": "https://q.fal.run/status/fal_xyz"}),
        )
        .unwrap();
        assert_eq!(r.id, "fal_xyz");
        assert_eq!(
            r.status_url.as_deref(),
            Some("https://q.fal.run/status/fal_xyz")
        );
    }

    #[tokio::test]
    async fn data_url_for_nonexistent_image_path() {
        // Skip extension check by using a .png suffix that doesn't exist.
        let out = image_path_to_data_url("definitely-not-here.png").await;
        assert!(out.is_err(), "got: {out:?}");
    }

    #[tokio::test]
    async fn data_url_for_unsupported_extension() {
        let out = image_path_to_data_url("video.mkv").await;
        assert!(out.is_err(), "got: {out:?}");
        if let Err(e) = out {
            assert!(e.contains("unsupported"));
        }
    }
}
