//! `phantom event capture` CLI subcommand handler.
//!
//! Reads files from disk (image/audio) and command-line text, then
//! POSTs them as multipart to the local phantom serve daemon's
//! `/api/events` endpoint. Prints the resulting AnalysisResult.

use crate::life_node::multimodal::Modality;
use anyhow::{anyhow, Context, Result};
use std::path::Path;

pub struct CaptureArgs {
    pub kind: String,
    pub image_paths: Vec<String>,
    pub audio_paths: Vec<String>,
    pub text: Option<String>,
    pub goal_tags: Vec<String>,
    pub coord_url: String, // e.g., http://127.0.0.1:17878
    /// Print the raw daemon JSON response (for pipelines). When false (default)
    /// we render a concise human summary instead of dumping the full payload.
    pub json: bool,
}

pub async fn run(args: CaptureArgs) -> Result<()> {
    let modalities = collect_modalities(&args)?;
    if modalities.is_empty() {
        return Err(anyhow!(
            "at least one of --image / --audio / --text must be supplied"
        ));
    }

    let client = reqwest::Client::new();
    let mut form = reqwest::multipart::Form::new()
        .text("kind", args.kind.clone())
        .text("goal_tags", args.goal_tags.join(","));

    if let Some(t) = &args.text {
        form = form.text("text", t.clone());
    }
    for (i, p) in args.image_paths.iter().enumerate() {
        let bytes = std::fs::read(p).with_context(|| format!("read image {}", p))?;
        let fname = Path::new(p)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("image")
            .to_string();
        form = form.part(
            format!("image_{}", i),
            reqwest::multipart::Part::bytes(bytes)
                .file_name(fname)
                .mime_str("image/jpeg")?,
        );
    }
    for (i, p) in args.audio_paths.iter().enumerate() {
        let bytes = std::fs::read(p).with_context(|| format!("read audio {}", p))?;
        let fname = Path::new(p)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("audio")
            .to_string();
        form = form.part(
            format!("audio_{}", i),
            reqwest::multipart::Part::bytes(bytes)
                .file_name(fname)
                .mime_str("audio/wav")?,
        );
    }

    let url = format!("{}/api/events", args.coord_url.trim_end_matches('/'));
    let resp = match client.post(&url).multipart(form).send().await {
        Ok(r) => r,
        // The capture pipeline lives inside `phantom serve`. A new user running
        // `phantom food ...` before starting the daemon would otherwise see a raw
        // "connection refused" — surface an actionable hint instead (E006 path).
        Err(e) if e.is_connect() => {
            use crate::i18n::tr;
            return Err(anyhow!(
                "{}\n\n    phantom serve\n\n{} {}",
                tr(
                    "Could not reach the phantom daemon — the capture pipeline runs inside `phantom serve`. Start it first (in another terminal):",
                    "連不到 phantom daemon（常駐服務）——擷取管線跑在 `phantom serve` 裡。請先在另一個終端機啟動："
                ),
                tr(
                    "then re-run this command. Underlying error:",
                    "再重跑這個指令；底層錯誤："
                ),
                e
            ));
        }
        Err(e) => return Err(anyhow::Error::new(e).context(format!("POST {}", url))),
    };
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("server returned {}: {}", status, text));
    }
    if args.json {
        println!("{}", text);
    } else {
        print_summary(&args.kind, &args.goal_tags, &text);
    }
    Ok(())
}

/// Render a concise, human-readable capture confirmation instead of dumping the
/// raw daemon JSON. Falls back to printing the raw body if it can't be parsed,
/// so we never swallow an unexpected response shape.
fn print_summary(kind: &str, goal_tags: &[String], body: &str) {
    use crate::i18n::tr;
    use crate::util::term::colored;
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => {
            println!("{}", body);
            return;
        }
    };
    let event_id = v.get("event_id").and_then(|x| x.as_str()).unwrap_or("");
    let short_id = event_id.split('-').next().unwrap_or(event_id);
    println!(
        "{} {} · {} {}",
        colored("✓", 32),
        tr("captured", "已擷取"),
        kind,
        short_id,
    );
    let tags: Vec<&str> = goal_tags
        .iter()
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .collect();
    if !tags.is_empty() {
        println!("  {}: {}", tr("tags", "標籤"), tags.join(", "));
    }
    let analysis = v.get("analysis");
    if let Some(summary) = analysis.and_then(|a| a.get("summary")).and_then(|x| x.as_str()) {
        if !summary.trim().is_empty() {
            println!("  {}", summary.trim());
        }
    }
    let impact = analysis
        .and_then(|a| a.get("goal_impact"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let suggestion = analysis
        .and_then(|a| a.get("suggestion"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if !suggestion.trim().is_empty() {
        let label = tr("suggestion", "建議");
        if impact.is_empty() {
            println!("  {}: {}", label, suggestion.trim());
        } else {
            println!("  {} ({}): {}", label, impact, suggestion.trim());
        }
    }
    let model = analysis
        .and_then(|a| a.get("model_id"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let latency = analysis
        .and_then(|a| a.get("latency_ms"))
        .and_then(|x| x.as_i64());
    if !model.is_empty() || latency.is_some() {
        let lat = latency.map(|l| format!(" · {}ms", l)).unwrap_or_default();
        println!("  {}", colored(&format!("{}{}", model, lat), 90));
    }
}

fn collect_modalities(_args: &CaptureArgs) -> Result<Vec<Modality>> {
    // Modalities are collected server-side from the multipart form.
    // This helper exists so future caching / dry-run modes have a hook.
    Ok(vec![Modality::Text(String::new())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn cli_subcommand_uploads_multipart() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/events"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"event_id":"abc","analysis":{"summary":"ok","model_id":"m","latency_ms":1,"raw_response":{}}}"#
            ))
            .mount(&mock)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let img = tmp.path().join("food.jpg");
        std::fs::write(&img, b"\xff\xd8\xff\xd9").unwrap();

        run(CaptureArgs {
            kind: "food_log".into(),
            image_paths: vec![img.to_string_lossy().to_string()],
            audio_paths: vec![],
            text: Some("lunch".into()),
            goal_tags: vec!["fat_loss".into()],
            coord_url: mock.uri(),
            json: false,
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn capture_gives_actionable_hint_when_daemon_down() {
        // No daemon listening (port 1 refuses) → the error must point the user at
        // `phantom serve` rather than surfacing a raw connection-refused. Guards
        // the E006 "30-second Life Hello" first-run ergonomics.
        let err = run(CaptureArgs {
            kind: "food_log".into(),
            image_paths: vec![],
            audio_paths: vec![],
            text: Some("lunch".into()),
            goal_tags: vec![],
            coord_url: "http://127.0.0.1:1".into(),
            json: false,
        })
        .await
        .expect_err("must fail when no daemon is listening");
        let msg = format!("{err}");
        assert!(
            msg.contains("phantom serve"),
            "hint should mention `phantom serve`, got: {msg}"
        );
    }
}
