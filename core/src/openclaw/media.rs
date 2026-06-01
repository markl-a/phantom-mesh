//! OpenClaw media handling — photo / voice / document downloads, bridged into
//! the existing `multimodal::prompt_to_content_value` sentinel format.
//!
//! In Remote Control terms (BIG-GOAL §P3): a Telegram photo or WhatsApp
//! voice memo is *attached input* on a cluster command — "look at this
//! screenshot and tell me what's wrong" — not a chat exchange. This
//! module standardises the download + size-cap + MIME handling so every
//! channel's remote feeds the cluster the same shape of multimodal
//! input, no matter which messenger took the picture.
//!
//! Channel adapters (`telegram`, `whatsapp`, `slack`) all need to:
//!
//!   1. Pick the *largest* representation when the platform gives an ascending
//!      list of thumbnails (Telegram's `Vec<PhotoSize>`).
//!   2. Stream the bytes from the upstream API, but **cap the download** so a
//!      hostile or buggy peer can't OOM the process by handing us a 5 GB
//!      "voice memo".
//!   3. Hand the bytes + MIME to `agent.rs` in the same shape the screenshot
//!      pipeline already uses (a `<phantom-image …/>` sentinel that
//!      [`crate::multimodal::prompt_to_content_value`] turns into an
//!      OpenAI/Anthropic-compatible `image_url` content part).
//!
//! This module is intentionally **channel-agnostic**: it knows nothing about
//! Telegram's `getFile`, Meta's Graph API, or Slack's `files.info`. Each
//! channel impl computes its own download URL and calls
//! [`download_with_limit`] — the size cap, MIME bookkeeping, and sentinel
//! emission are uniform across all three so they cannot drift apart.
//!
//! # Size cap
//!
//! [`MAX_MEDIA_BYTES`] is **5 MiB**, deliberately tighter than
//! `multimodal::MAX_IMAGE_BYTES` (20 MiB). The latter handles user-attached
//! local files where the path is trusted; here the bytes arrive over the
//! public internet from a third-party API, so we err on the smaller side.
//!
//! See `docs/superpowers/specs/2026-05-15-weekend-multi-agent-push-design.md`
//! §6 row [B8].

use crate::multimodal::image_mime_for_path;

use base64::Engine;

/// Hard cap on the body size of any single media download. Anything past this
/// length is rejected with [`MediaError::TooLarge`] without the remaining
/// bytes ever being buffered in memory.
///
/// 5 MiB comfortably covers a full-resolution Telegram `photo` (~1.5 MiB for a
/// 1280x1280 JPEG), a 60 s WhatsApp voice note (Opus, ~0.5 MiB), and a typical
/// Slack PDF (<2 MiB). Heavier attachments fall through to a graceful "media
/// too large" reply instead of risking process death.
pub const MAX_MEDIA_BYTES: usize = 5 * 1024 * 1024;

/// Errors a media fetch can return.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    /// Source list was empty — no photo size / file entry to pick.
    #[error("no media items to choose from")]
    Empty,

    /// Network / API error fetching the media bytes. The string carries the
    /// underlying error message but **never** the API token (callers are
    /// responsible for redacting before constructing this variant).
    #[error("transport error fetching media: {0}")]
    Transport(String),

    /// Body exceeded [`MAX_MEDIA_BYTES`]. The `actual` value is the count of
    /// bytes seen before the limit tripped (may equal the advertised
    /// `Content-Length` when we refused up front before reading any bytes).
    #[error("media too large: {actual} bytes (limit {limit})")]
    TooLarge { actual: usize, limit: usize },
}

/// Minimal description of a Telegram-style photo thumbnail. Channels build
/// this from their wire-protocol types; keeping it standalone makes
/// [`select_largest_photo`] testable without pulling teloxide into this
/// crate's build graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoSizeRef {
    pub file_id: String,
    pub width: u32,
    pub height: u32,
}

/// Pick the highest-resolution photo from a list.
///
/// Telegram delivers `photo: Vec<PhotoSize>` in *ascending* size order, so a
/// naive `photos[0]` yields a 64x64 thumbnail. We can't trust the upstream
/// ordering blindly (a future Bot API revision could change it), so this
/// helper picks by `width * height` and falls back to the last element on
/// ties — which matches the documented "biggest is last" convention.
///
/// Returns `MediaError::Empty` for an empty slice.
pub fn select_largest_photo(photos: &[PhotoSizeRef]) -> Result<&PhotoSizeRef, MediaError> {
    if photos.is_empty() {
        return Err(MediaError::Empty);
    }
    // `max_by_key` is stable on ties → keeps the last (= biggest, per Telegram
    // convention).
    Ok(photos
        .iter()
        .max_by_key(|p| (p.width as u64) * (p.height as u64))
        .expect("non-empty slice"))
}

/// Stream a body from `url`, refusing if it would exceed [`MAX_MEDIA_BYTES`].
///
/// The cap is enforced **incrementally** via `bytes_stream`: we never call
/// `Response::bytes()` (which would buffer the entire body, regardless of
/// size, before we got a chance to inspect it). The upstream's
/// `Content-Length`, if present and over the limit, short-circuits the
/// download before any bytes are read.
pub async fn download_with_limit(
    client: &reqwest::Client,
    url: &str,
    limit: usize,
) -> Result<Vec<u8>, MediaError> {
    use futures::StreamExt;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| MediaError::Transport(e.to_string()))?
        .error_for_status()
        .map_err(|e| MediaError::Transport(e.to_string()))?;

    // Cheap pre-check: refuse loudly when the upstream advertises a body
    // bigger than the cap. (Saves us from buffering up to `limit` bytes only
    // to throw them away.)
    if let Some(len) = resp.content_length() {
        if len as usize > limit {
            return Err(MediaError::TooLarge {
                actual: len as usize,
                limit,
            });
        }
    }

    let mut buf: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| MediaError::Transport(e.to_string()))?;
        if buf.len().saturating_add(chunk.len()) > limit {
            return Err(MediaError::TooLarge {
                actual: buf.len() + chunk.len(),
                limit,
            });
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Wrap downloaded bytes + MIME into the `<phantom-image …/>` sentinel that
/// [`crate::multimodal::prompt_to_content_value`] turns into a multipart
/// content array. Non-image MIME types (voice, generic document) currently
/// route through the same sentinel — the model receives the bytes as an
/// `image_url`-shaped payload, which Anthropic/OpenAI vision endpoints will
/// reject if non-visual, surfacing as an upstream error rather than a silent
/// drop. A future revision can introduce an `<phantom-audio …/>` sentinel
/// once the streaming-multimodal layer supports it; the surface here stays
/// stable.
pub fn media_to_image_sentinel(mime: &str, bytes: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!(r#"<phantom-image mime="{}" data="{}"/>"#, mime, b64)
}

/// Best-effort MIME inference from a filename (e.g. Telegram's `file_path`,
/// Slack's `name`, WhatsApp's `filename`). Falls back to
/// `application/octet-stream` when the extension is unknown, mirroring
/// `multimodal::image_mime_for_path` behaviour.
pub fn mime_from_filename(name: &str) -> &'static str {
    image_mime_for_path(name).unwrap_or_else(|| {
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "ogg" | "oga" => "audio/ogg",
            "mp3" => "audio/mpeg",
            "m4a" => "audio/mp4",
            "wav" => "audio/wav",
            "pdf" => "application/pdf",
            _ => "application/octet-stream",
        }
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn p(file_id: &str, w: u32, h: u32) -> PhotoSizeRef {
        PhotoSizeRef {
            file_id: file_id.into(),
            width: w,
            height: h,
        }
    }

    #[test]
    fn select_largest_photo_picks_full_size_not_thumb() {
        // Telegram's documented order: thumb -> medium -> full.
        let photos = vec![
            p("thumb_64", 64, 64),
            p("med_320", 320, 320),
            p("full_1280", 1280, 1280),
        ];
        let picked = select_largest_photo(&photos).expect("non-empty");
        assert_eq!(picked.file_id, "full_1280");
        assert_eq!(picked.width, 1280);
    }

    #[test]
    fn select_largest_photo_handles_unordered_input() {
        // Even if a future Bot API revision delivers the biggest first, we
        // still pick it correctly (we sort by area, not by position).
        let photos = vec![p("big", 1280, 1280), p("small", 64, 64), p("med", 320, 320)];
        let picked = select_largest_photo(&photos).expect("non-empty");
        assert_eq!(picked.file_id, "big");
    }

    #[test]
    fn select_largest_photo_empty_errors() {
        let err = select_largest_photo(&[]).unwrap_err();
        assert!(matches!(err, MediaError::Empty));
    }

    #[test]
    fn select_largest_photo_single_returns_it() {
        let photos = vec![p("only", 100, 100)];
        let picked = select_largest_photo(&photos).expect("non-empty");
        assert_eq!(picked.file_id, "only");
    }

    #[test]
    fn mime_from_filename_known_types() {
        assert_eq!(mime_from_filename("photo.png"), "image/png");
        assert_eq!(mime_from_filename("voice.ogg"), "audio/ogg");
        assert_eq!(mime_from_filename("doc.pdf"), "application/pdf");
        assert_eq!(mime_from_filename("voice.m4a"), "audio/mp4");
    }

    #[test]
    fn mime_from_filename_unknown_falls_back() {
        assert_eq!(mime_from_filename("weird.xyz"), "application/octet-stream");
        assert_eq!(mime_from_filename("noext"), "application/octet-stream");
    }

    #[test]
    fn media_to_image_sentinel_round_trips_via_multimodal() {
        // The sentinel must parse cleanly back through the existing
        // multimodal layer — otherwise downstream LLM routing breaks.
        let sentinel = media_to_image_sentinel("image/png", &[0xde, 0xad, 0xbe, 0xef]);
        assert!(sentinel.contains(r#"mime="image/png""#));
        assert!(sentinel.contains(r#"data="3q2+7w==""#));

        let prompt = format!("look at this: {}", sentinel);
        let value = crate::multimodal::prompt_to_content_value(&prompt);
        let arr = value.as_array().expect("multipart array");
        let img = arr
            .iter()
            .find(|p| p["type"] == "image_url")
            .expect("image part");
        let url = img["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
        assert!(url.contains("3q2+7w=="));
    }

    #[tokio::test]
    async fn download_with_limit_accepts_small_body() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let mock = MockServer::start().await;
        let body = vec![0x42u8; 1024]; // 1 KiB — well under 5 MiB
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&mock)
            .await;

        let client = reqwest::Client::new();
        let bytes = download_with_limit(&client, &mock.uri(), MAX_MEDIA_BYTES)
            .await
            .expect("under-limit download should succeed");
        assert_eq!(bytes.len(), 1024);
        assert!(bytes.iter().all(|b| *b == 0x42));
    }

    #[tokio::test]
    async fn download_with_limit_one_mib_photo_forwarded_via_sentinel() {
        // 1 MiB synthetic photo body → download succeeds → we wrap it into
        // the sentinel format the agent layer consumes.
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let mock = MockServer::start().await;
        let body = vec![0xa5u8; 1024 * 1024];
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&mock)
            .await;

        let client = reqwest::Client::new();
        let bytes = download_with_limit(&client, &mock.uri(), MAX_MEDIA_BYTES)
            .await
            .expect("1 MiB body should download");
        assert_eq!(bytes.len(), 1024 * 1024);

        let sentinel = media_to_image_sentinel("image/jpeg", &bytes);
        let prompt = format!("describe: {}", sentinel);
        let value = crate::multimodal::prompt_to_content_value(&prompt);
        let arr = value
            .as_array()
            .expect("array — sentinel must produce multipart");
        assert!(arr.iter().any(|p| p["type"] == "image_url"));
    }

    #[tokio::test]
    async fn download_with_limit_rejects_oversize_content_length() {
        // Upstream advertises a 6 MiB Content-Length → we refuse without
        // reading the body at all.
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let mock = MockServer::start().await;
        let big = vec![0u8; 6 * 1024 * 1024];
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(big))
            .mount(&mock)
            .await;

        let client = reqwest::Client::new();
        let err = download_with_limit(&client, &mock.uri(), MAX_MEDIA_BYTES)
            .await
            .expect_err("6 MiB body must be rejected");
        match err {
            MediaError::TooLarge { actual, limit } => {
                assert_eq!(limit, MAX_MEDIA_BYTES);
                assert!(
                    actual > MAX_MEDIA_BYTES,
                    "actual={} should exceed limit",
                    actual
                );
            }
            other => panic!("expected TooLarge, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn download_with_limit_rejects_streamed_over_cap() {
        // Bytes-tracked enforcement: even if the Content-Length header is
        // present, the incremental check is what catches a chunked transfer
        // (or a spoofed under-size header). Body is exactly limit+1 to
        // pin the strict ">" boundary.
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let mock = MockServer::start().await;
        let tight = vec![0u8; MAX_MEDIA_BYTES + 1];
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(tight))
            .mount(&mock)
            .await;

        let client = reqwest::Client::new();
        let err = download_with_limit(&client, &mock.uri(), MAX_MEDIA_BYTES)
            .await
            .expect_err("limit+1 byte body must be rejected");
        assert!(matches!(err, MediaError::TooLarge { .. }));
    }

    #[tokio::test]
    async fn download_with_limit_transport_error_classified() {
        // 500 from upstream → Transport error (not TooLarge / Empty).
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock)
            .await;

        let client = reqwest::Client::new();
        let err = download_with_limit(&client, &mock.uri(), MAX_MEDIA_BYTES)
            .await
            .expect_err("HTTP 500 must error");
        assert!(matches!(err, MediaError::Transport(_)));
    }
}
