//! `music_generate` — text → audio via Replicate / Fal / ElevenLabs Music.
//!
//! Completes the media-gen trinity (image_gen / video_gen / this).
//! Two wire patterns under one tool:
//!
//! - **Async-job** (Replicate, Fal) — POST creates job, poll for
//!   completion, download URL. Same shape as `video_gen`.
//! - **Sync POST** (ElevenLabs Music) — single request returns audio
//!   bytes directly.
//!
//! ## Supported providers
//!
//! Selected via `SPECTYN_MUSIC_GEN_PROVIDER`:
//!
//! | value (default `replicate`) | provider          | default model                  | vocals? |
//! |---|---|---|---|
//! | `replicate` | Replicate `/v1/predictions`            | `meta/musicgen`                | no (instrumental) |
//! | `fal`       | Fal.ai queue                           | `fal-ai/musicgen-medium`       | no (instrumental) |
//! | `elevenlabs` | ElevenLabs Music `/v1/music`          | (server default — chirp-class) | YES (lyrics supported) |
//!
//! Env overrides:
//! - `SPECTYN_MUSIC_GEN_PROVIDER` — see table
//! - `SPECTYN_MUSIC_GEN_BASE_URL` — override the provider base URL
//! - `SPECTYN_MUSIC_GEN_API_KEY`  — fallback per-provider env
//!   (`REPLICATE_API_TOKEN` / `FAL_KEY` / `ELEVENLABS_API_KEY`)
//! - `SPECTYN_MUSIC_GEN_MODEL`    — override the model id
//! - `SPECTYN_MUSIC_GEN_TIMEOUT_SECS` — default 300 (= 5 min)
//!
//! ## Output
//!
//! Audio lands in `~/.spectyn-mesh/generated/<unix-ts>.mp3` (or
//! whichever extension the provider URL/content-type suggests).
//! Returns the path so the agent can `file_read` / `@`-attach for a
//! follow-up turn (transcribe / describe lyrics / etc).

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_TIMEOUT_SECS: u64 = 300;
const POLL_INTERVAL_SECS: u64 = 3;
const HTTP_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Replicate,
    Fal,
    ElevenLabs,
}

impl Provider {
    fn from_env() -> Self {
        match std::env::var("SPECTYN_MUSIC_GEN_PROVIDER")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "fal" | "fal-ai" | "fal.ai" => Self::Fal,
            "elevenlabs" | "eleven" | "eleven-labs" => Self::ElevenLabs,
            _ => Self::Replicate,
        }
    }

    fn default_base_url(self) -> &'static str {
        match self {
            Self::Replicate => "https://api.replicate.com/v1",
            Self::Fal => "https://queue.fal.run",
            Self::ElevenLabs => "https://api.elevenlabs.io/v1",
        }
    }

    fn default_model(self) -> &'static str {
        match self {
            Self::Replicate => "meta/musicgen",
            Self::Fal => "fal-ai/musicgen-medium",
            // ElevenLabs picks server-side default; pass empty.
            Self::ElevenLabs => "",
        }
    }

    fn api_key_envs(self) -> &'static [&'static str] {
        match self {
            Self::Replicate => &["SPECTYN_MUSIC_GEN_API_KEY", "REPLICATE_API_TOKEN"],
            Self::Fal => &["SPECTYN_MUSIC_GEN_API_KEY", "FAL_KEY"],
            Self::ElevenLabs => &["SPECTYN_MUSIC_GEN_API_KEY", "ELEVENLABS_API_KEY"],
        }
    }

    /// Wire pattern: ElevenLabs is one synchronous POST that returns
    /// audio bytes; the others are async create-job + poll.
    fn is_sync(self) -> bool {
        matches!(self, Self::ElevenLabs)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Replicate => "replicate",
            Self::Fal => "fal",
            Self::ElevenLabs => "elevenlabs",
        }
    }
}

/// MCP tool entry-point.
///
/// Required args:
/// - `prompt` (string) — describes the music (genre / mood / instruments)
///
/// Optional args:
/// - `model`         (string — overrides env)
/// - `duration_secs` (integer 1-300, default 30; MusicGen caps at 30s,
///   Stable Audio / ElevenLabs go further)
/// - `lyrics`        (string — only honoured by ElevenLabs; passed
///   through to enable vocal generation)
pub async fn generate(args: &Value) -> String {
    let prompt = match args.get("prompt").and_then(|v| v.as_str()) {
        Some(p) if !p.trim().is_empty() => p.trim().to_string(),
        _ => return "Error: missing required string argument 'prompt'".into(),
    };

    let provider = Provider::from_env();
    let api_key = read_api_key(provider);
    if api_key.is_empty() {
        return format!(
            "Error: no API key for provider '{}' — set SPECTYN_MUSIC_GEN_API_KEY or one of [{}]",
            provider.name(),
            provider.api_key_envs().join(", "),
        );
    }

    let base_url = std::env::var("SPECTYN_MUSIC_GEN_BASE_URL")
        .unwrap_or_else(|_| provider.default_base_url().to_string());
    let model = args
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            std::env::var("SPECTYN_MUSIC_GEN_MODEL")
                .unwrap_or_else(|_| provider.default_model().to_string())
        });
    let duration_secs = args
        .get("duration_secs")
        .and_then(|v| v.as_u64())
        .map(|v| v.clamp(1, 300))
        .unwrap_or(30);
    let lyrics = args
        .get("lyrics")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    if lyrics.is_some() && provider != Provider::ElevenLabs {
        return format!(
            "Error: 'lyrics' is only supported by elevenlabs provider; current provider is '{}'. \
             Set SPECTYN_MUSIC_GEN_PROVIDER=elevenlabs to use vocals.",
            provider.name(),
        );
    }

    let timeout_secs = std::env::var("SPECTYN_MUSIC_GEN_TIMEOUT_SECS")
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

    let params = MusicParams {
        prompt: &prompt,
        model: &model,
        duration_secs,
        lyrics: lyrics.as_deref(),
    };

    let dir = match output_dir() {
        Ok(d) => d,
        Err(e) => return format!("Error: preparing output dir: {e}"),
    };
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if provider.is_sync() {
        // ── ElevenLabs sync path ─────────────────────────────────────────────
        let dest = dir.join(format!("{ts}.mp3"));
        match elevenlabs_post_and_save(&client, &base_url, &api_key, &params, &dest).await {
            Ok(bytes) => format!(
                "Generated 1 music clip ({} bytes, elevenlabs provider):\n{}",
                bytes,
                dest.display(),
            ),
            Err(e) => format!("Error: elevenlabs request: {e}"),
        }
    } else {
        // ── Async job path (Replicate / Fal) ─────────────────────────────────
        let create = match create_job(provider, &client, &base_url, &api_key, &params).await {
            Ok(c) => c,
            Err(e) => return format!("Error: creating {} job: {e}", provider.name()),
        };
        let audio_url = match poll_until_done(
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
        let ext = guess_audio_ext(&audio_url);
        let dest = dir.join(format!("{ts}.{ext}"));
        match download_to_file(&client, &audio_url, &dest).await {
            Ok(bytes) => format!(
                "Generated 1 music clip ({} bytes, {} provider, model={}):\n{}",
                bytes,
                provider.name(),
                model,
                dest.display(),
            ),
            Err(e) => format!("Error: downloading {audio_url}: {e}"),
        }
    }
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn output_dir() -> std::io::Result<PathBuf> {
    let data = crate::cli_config::spectyn_data_dir()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, e.to_string()))?;
    let dir = data.join("generated");
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

/// Sniff the file extension from a URL or filename. Falls back to `mp3`.
fn guess_audio_ext(url: &str) -> &'static str {
    // Strip query / fragment.
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    if last.ends_with(".wav") {
        return "wav";
    }
    if last.ends_with(".flac") {
        return "flac";
    }
    if last.ends_with(".ogg") {
        return "ogg";
    }
    if last.ends_with(".m4a") {
        return "m4a";
    }
    if last.ends_with(".mp3") {
        return "mp3";
    }
    "mp3"
}

#[derive(Debug)]
struct MusicParams<'a> {
    prompt: &'a str,
    model: &'a str,
    duration_secs: u64,
    lyrics: Option<&'a str>,
}

#[derive(Debug)]
struct JobCreated {
    id: String,
    status_url: Option<String>,
}

// ─── Replicate / Fal create + poll (async) ───────────────────────────────────

async fn create_job(
    provider: Provider,
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    params: &MusicParams<'_>,
) -> Result<JobCreated, String> {
    let base = base_url.trim_end_matches('/');
    let auth = match provider {
        Provider::Replicate => format!("Token {api_key}"),
        Provider::Fal => format!("Key {api_key}"),
        _ => unreachable!("create_job only for async providers"),
    };
    let (url, body) = match provider {
        Provider::Replicate => {
            let model = params.model;
            let url = format!("{base}/models/{model}/predictions");
            let input = json!({
                "prompt":   params.prompt,
                "duration": params.duration_secs,
            });
            (url, json!({ "input": input }))
        }
        Provider::Fal => {
            let model = params.model;
            let url = format!("{base}/{model}");
            let body = json!({
                "prompt":   params.prompt,
                "duration": params.duration_secs,
            });
            (url, body)
        }
        _ => unreachable!(),
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
            "{url} returned {} — {}",
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
        Provider::Replicate => resp
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("missing 'id': {resp}"))?
            .to_string(),
        Provider::Fal => resp
            .get("request_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("missing 'request_id': {resp}"))?
            .to_string(),
        _ => return Err("parse_create_response not applicable for sync providers".into()),
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
    let auth = match provider {
        Provider::Replicate => format!("Token {api_key}"),
        Provider::Fal => format!("Key {api_key}"),
        _ => unreachable!(),
    };
    let start = std::time::Instant::now();

    let status_url = match provider {
        Provider::Replicate => format!("{base}/predictions/{}", job.id),
        Provider::Fal => job
            .status_url
            .clone()
            .ok_or_else(|| "Fal create response missing status_url".to_string())?,
        _ => unreachable!(),
    };

    loop {
        if start.elapsed() > overall_timeout {
            return Err(format!(
                "timeout after {} s (set SPECTYN_MUSIC_GEN_TIMEOUT_SECS to extend)",
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
                    // `output` is either a string URL or an array.
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
        Provider::Fal => {
            let status = resp.get("status").and_then(|v| v.as_str()).unwrap_or("");
            match status {
                "COMPLETED" => {
                    let url = resp
                        .get("audio")
                        .and_then(|a| a.get("url"))
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            resp.get("output")
                                .and_then(|o| o.get("audio"))
                                .and_then(|a| a.get("url"))
                                .and_then(|v| v.as_str())
                        })
                        .or_else(|| {
                            // Some Fal music models return `audio_url` flat.
                            resp.get("audio_url").and_then(|v| v.as_str())
                        })
                        .unwrap_or("")
                        .to_string();
                    if url.is_empty() {
                        JobStatus::Failed("COMPLETED but no audio.url".into())
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
        Provider::ElevenLabs => unreachable!("ElevenLabs is sync — never polls"),
    }
}

// ─── ElevenLabs sync ─────────────────────────────────────────────────────────

async fn elevenlabs_post_and_save(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    params: &MusicParams<'_>,
    dest: &Path,
) -> Result<u64, String> {
    let base = base_url.trim_end_matches('/');
    let url = format!("{base}/music");
    let mut body = json!({
        "prompt":           params.prompt,
        "music_length_ms":  (params.duration_secs as i64) * 1000,
    });
    // ElevenLabs uses server-side default model when none supplied.
    if !params.model.is_empty() {
        body["model_id"] = json!(params.model);
    }
    if let Some(lyrics) = params.lyrics {
        body["lyrics"] = json!(lyrics);
    }

    let resp = client
        .post(&url)
        .header("xi-api-key", api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "{url} returned {} — {}",
            status.as_u16(),
            body_text.chars().take(400).collect::<String>(),
        ));
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
            "SPECTYN_MUSIC_GEN_PROVIDER",
            "SPECTYN_MUSIC_GEN_API_KEY",
            "SPECTYN_MUSIC_GEN_BASE_URL",
            "SPECTYN_MUSIC_GEN_MODEL",
            "SPECTYN_MUSIC_GEN_TIMEOUT_SECS",
            "REPLICATE_API_TOKEN",
            "FAL_KEY",
            "ELEVENLABS_API_KEY",
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
        std::env::set_var("SPECTYN_MUSIC_GEN_PROVIDER", "fal");
        assert_eq!(Provider::from_env(), Provider::Fal);
        std::env::set_var("SPECTYN_MUSIC_GEN_PROVIDER", "Fal.ai");
        assert_eq!(Provider::from_env(), Provider::Fal);
        std::env::set_var("SPECTYN_MUSIC_GEN_PROVIDER", "elevenlabs");
        assert_eq!(Provider::from_env(), Provider::ElevenLabs);
        std::env::set_var("SPECTYN_MUSIC_GEN_PROVIDER", "eleven");
        assert_eq!(Provider::from_env(), Provider::ElevenLabs);
        clean_env();
    }

    #[test]
    fn is_sync_only_elevenlabs() {
        assert!(!Provider::Replicate.is_sync());
        assert!(!Provider::Fal.is_sync());
        assert!(Provider::ElevenLabs.is_sync());
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
        let out = generate(&json!({"prompt": "lo-fi hip-hop with rain"})).await;
        assert!(out.starts_with("Error: no API key"), "got: {out}");
        assert!(out.contains("REPLICATE_API_TOKEN"), "got: {out}");
    }

    #[tokio::test]
    async fn lyrics_on_non_elevenlabs_rejected() {
        let _g = crate::sandbox::test_lock();
        clean_env();
        std::env::set_var("SPECTYN_MUSIC_GEN_PROVIDER", "replicate");
        std::env::set_var("REPLICATE_API_TOKEN", "stub-key");
        let out = generate(&json!({
            "prompt": "synthwave",
            "lyrics": "I am a stranger in this world",
        }))
        .await;
        clean_env();
        assert!(
            out.contains("'lyrics' is only supported by elevenlabs"),
            "got: {out}"
        );
    }

    #[test]
    fn guess_audio_ext_strips_query() {
        assert_eq!(
            guess_audio_ext("https://cdn.replicate.com/x.wav?sig=abc"),
            "wav"
        );
        assert_eq!(guess_audio_ext("https://fal.media/y.mp3"), "mp3");
        assert_eq!(guess_audio_ext("https://q/z.flac"), "flac");
        assert_eq!(guess_audio_ext("https://q/no-ext"), "mp3");
        assert_eq!(guess_audio_ext("https://q/song.OGG"), "ogg");
    }

    #[test]
    fn parse_status_replicate_succeeded_string_output() {
        let r = parse_status(
            Provider::Replicate,
            &json!({"status": "succeeded", "output": "https://cdn.replicate.com/a.wav"}),
        );
        assert!(matches!(r, JobStatus::Done(ref u) if u.ends_with("a.wav")));
    }

    #[test]
    fn parse_status_replicate_succeeded_array_output() {
        let r = parse_status(
            Provider::Replicate,
            &json!({"status": "succeeded", "output": ["https://cdn.replicate.com/b.wav"]}),
        );
        assert!(matches!(r, JobStatus::Done(ref u) if u.ends_with("b.wav")));
    }

    #[test]
    fn parse_status_replicate_failed() {
        let r = parse_status(
            Provider::Replicate,
            &json!({"status": "failed", "error": "OOM"}),
        );
        assert!(matches!(r, JobStatus::Failed(ref m) if m == "OOM"));
    }

    #[test]
    fn parse_status_replicate_running() {
        let r = parse_status(Provider::Replicate, &json!({"status": "processing"}));
        assert!(matches!(r, JobStatus::Running));
    }

    #[test]
    fn parse_status_fal_completed_nested_url() {
        let r = parse_status(
            Provider::Fal,
            &json!({"status": "COMPLETED", "audio": {"url": "https://fal.media/m.mp3"}}),
        );
        assert!(matches!(r, JobStatus::Done(ref u) if u.ends_with("m.mp3")));
    }

    #[test]
    fn parse_status_fal_completed_flat_url() {
        let r = parse_status(
            Provider::Fal,
            &json!({"status": "COMPLETED", "audio_url": "https://fal.media/n.mp3"}),
        );
        assert!(matches!(r, JobStatus::Done(ref u) if u.ends_with("n.mp3")));
    }

    #[test]
    fn parse_create_response_replicate_id() {
        let r = parse_create_response(Provider::Replicate, &json!({"id": "rp_abc"})).unwrap();
        assert_eq!(r.id, "rp_abc");
    }

    #[test]
    fn parse_create_response_fal_request_id() {
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
}
