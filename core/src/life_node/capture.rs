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
    let resp = client
        .post(&url)
        .multipart(form)
        .send()
        .await
        .with_context(|| format!("POST {}", url))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("server returned {}: {}", status, text));
    }
    println!("{}", text);
    Ok(())
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
        })
        .await
        .unwrap();
    }
}
