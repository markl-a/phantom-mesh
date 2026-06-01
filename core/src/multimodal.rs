//! Multimodal (image attachment) support for chat prompts.
//!
//! When a user types `@/path/to/image.png` in the REPL, we want the LLM to
//! actually *see* the image rather than receive a bag of bytes. This module
//! provides helpers to:
//!
//!   1. Encode a local image file into a `data:image/<mime>;base64,<...>`
//!      URL string, wrapped in a sentinel marker that downstream code can
//!      recognise without changing the `expand_at_files` String return type.
//!   2. Convert a prompt string (which may contain such sentinels) into the
//!      OpenAI multipart `content` shape — either a plain `String` (no images)
//!      or an array of `{type: "text", ...}` and `{type: "image_url", ...}`
//!      parts. Anthropic and Gemini's OpenAI-compat endpoints accept the same
//!      shape.
//!
//! The sentinel form is intentionally regex-friendly:
//!
//! ```text
//! <phantom-image mime="image/png" data="<base64>"/>
//! ```
//!
//! Anything outside the sentinels remains plain text.

use base64::Engine;
use serde_json::{json, Value};

/// Recognised image extensions (lowercased, no leading dot).
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

/// Hard cap on image file size accepted by [`encode_image_sentinel`].
///
/// At 20 MiB raw bytes, the base64 payload is ~27 MiB and the in-memory peak
/// during encoding briefly hits ~47 MiB — well within reason for a CLI/TUI
/// that may be running alongside the user's shell on a phone or laptop.
///
/// This covers any realistic screenshot or photograph (a typical 4K PNG is
/// 1-4 MiB; a 100 MP RAW conversion sits well under 20 MiB). Anything
/// larger is almost certainly either a mistake (user `@`-attached a video
/// or archive that happens to have an image extension) or hostile input
/// designed to OOM the process. Per issue #71, an unbounded `fs::read` on
/// a multi-GB file was a real panic vector.
pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

/// Returns `Some(mime)` if `path` ends with a recognised image extension.
pub fn image_mime_for_path(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    if !IMAGE_EXTS.contains(&ext.as_str()) {
        return None;
    }
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    })
}

/// Read `path`, base64-encode the bytes, and return the sentinel string that
/// later gets parsed into an `image_url` part. Returns `Err` if the file
/// cannot be read, or if the file exceeds [`MAX_IMAGE_BYTES`].
///
/// The size check is done via `fs::metadata` *before* `fs::read`, so a
/// multi-GB file never gets pulled into memory — fixing the OOM-panic
/// vector flagged in issue #71. The error message is intentionally
/// user-friendly (printed verbatim by `expand_at_files` as
/// `[error reading image <path>: <msg>]`).
pub fn encode_image_sentinel(path: &str) -> std::io::Result<String> {
    let mime = image_mime_for_path(path).unwrap_or("application/octet-stream");

    // Stat first; metadata() is cheap and avoids allocating for the
    // refused-too-large case entirely.
    let meta = std::fs::metadata(path)?;
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "image too large: {} bytes (limit {} bytes / {} MiB)",
                meta.len(),
                MAX_IMAGE_BYTES,
                MAX_IMAGE_BYTES / (1024 * 1024),
            ),
        ));
    }

    let bytes = std::fs::read(path)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!(
        r#"<phantom-image mime="{}" data="{}"/>"#,
        mime, b64
    ))
}

/// If `prompt` contains at least one image sentinel, return a JSON array of
/// content parts; otherwise return the original string as a JSON `String`.
///
/// The output is suitable to drop directly into `{"role":"user","content": ...}`.
pub fn prompt_to_content_value(prompt: &str) -> Value {
    let needle = "<phantom-image ";
    if !prompt.contains(needle) {
        return Value::String(prompt.to_string());
    }

    let mut parts: Vec<Value> = Vec::new();
    let mut cursor: usize = 0;
    let bytes = prompt.as_bytes();

    while cursor < prompt.len() {
        // Find next sentinel start.
        let Some(rel_start) = prompt[cursor..].find(needle) else {
            // No more sentinels — push remaining text and stop.
            let tail = &prompt[cursor..];
            if !tail.is_empty() {
                push_text(&mut parts, tail);
            }
            break;
        };
        let start = cursor + rel_start;

        // Push any text before the sentinel.
        if start > cursor {
            push_text(&mut parts, &prompt[cursor..start]);
        }

        // Locate sentinel end ("/>").
        let Some(rel_end) = prompt[start..].find("/>") else {
            // Malformed — just emit remainder as text.
            push_text(&mut parts, &prompt[start..]);
            break;
        };
        let end = start + rel_end + 2;
        let sentinel = &prompt[start..end];

        match parse_sentinel(sentinel) {
            Some((mime, data)) => {
                parts.push(json!({
                    "type": "image_url",
                    "image_url": { "url": format!("data:{};base64,{}", mime, data) }
                }));
            }
            None => {
                // Unparseable — keep the raw sentinel as text so we don't
                // silently drop content.
                push_text(&mut parts, sentinel);
            }
        }

        cursor = end;
        // Skip a single trailing space if present, to avoid stray blanks.
        if cursor < bytes.len() && bytes[cursor] == b' ' {
            cursor += 1;
        }
    }

    if parts.is_empty() {
        return Value::String(prompt.to_string());
    }
    // OpenAI requires at least one text part in some implementations; ensure one.
    let has_text = parts.iter().any(|p| p["type"] == "text");
    if !has_text {
        parts.insert(0, json!({"type": "text", "text": ""}));
    }
    Value::Array(parts)
}

/// Rewrite a single chat message so any OpenAI-style `image_url` parts are
/// converted into Anthropic's native `image` / `source.base64` shape.
///
/// Pass-through behaviour:
///   - `content` is a plain string → unchanged.
///   - `content` is an array but contains no `image_url` parts → unchanged.
///   - Other top-level fields (role, tool_calls, etc.) → preserved.
pub fn convert_message_for_anthropic(msg: &Value) -> Value {
    let Some(content) = msg.get("content") else {
        return msg.clone();
    };
    let Some(arr) = content.as_array() else {
        return msg.clone();
    };
    let needs_convert = arr.iter().any(|p| p["type"] == "image_url");
    if !needs_convert {
        return msg.clone();
    }

    let new_parts: Vec<Value> = arr
        .iter()
        .map(|p| {
            if p["type"] == "image_url" {
                let url = p["image_url"]["url"].as_str().unwrap_or("");
                // Expect `data:<mime>;base64,<data>`.
                if let Some(rest) = url.strip_prefix("data:") {
                    if let Some((meta, data)) = rest.split_once(",") {
                        let mime = meta.trim_end_matches(";base64");
                        return json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": mime,
                                "data": data,
                            }
                        });
                    }
                }
                // Fallback: pass through.
                p.clone()
            } else {
                p.clone()
            }
        })
        .collect();

    let mut out = msg.clone();
    if let Some(obj) = out.as_object_mut() {
        obj.insert("content".to_string(), Value::Array(new_parts));
    }
    out
}

fn push_text(parts: &mut Vec<Value>, text: &str) {
    let trimmed_check = text.trim();
    if trimmed_check.is_empty() {
        return;
    }
    parts.push(json!({"type": "text", "text": text}));
}

/// Parse `<phantom-image mime="..." data="..."/>` into `(mime, data)`.
fn parse_sentinel(s: &str) -> Option<(String, String)> {
    let mime = extract_attr(s, "mime")?;
    let data = extract_attr(s, "data")?;
    Some((mime, data))
}

fn extract_attr(s: &str, name: &str) -> Option<String> {
    let pat = format!("{}=\"", name);
    let i = s.find(&pat)? + pat.len();
    let rest = &s[i..];
    let j = rest.find('"')?;
    Some(rest[..j].to_string())
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn detects_image_extension() {
        assert_eq!(image_mime_for_path("/tmp/foo.png"), Some("image/png"));
        assert_eq!(image_mime_for_path("/tmp/foo.JPG"), Some("image/jpeg"));
        assert_eq!(image_mime_for_path("/tmp/foo.jpeg"), Some("image/jpeg"));
        assert_eq!(image_mime_for_path("foo.WebP"), Some("image/webp"));
        assert_eq!(image_mime_for_path("foo.txt"), None);
        assert_eq!(image_mime_for_path("foo"), None);
    }

    #[test]
    fn test_at_image_detection() {
        // Write a tiny "image" file (content is not real PNG, but the
        // detection is purely extension-based).
        let dir = std::env::temp_dir();
        let path = dir.join("phantom_test_image.png");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&[0xde, 0xad, 0xbe, 0xef]).unwrap();

        let path_str = path.to_string_lossy().to_string();
        assert_eq!(image_mime_for_path(&path_str), Some("image/png"));

        let sentinel = encode_image_sentinel(&path_str).expect("encode ok");
        assert!(sentinel.contains(r#"mime="image/png""#));
        assert!(
            sentinel.contains(r#"data="3q2+7w==""#),
            "expected base64 of [de ad be ef]; got: {}",
            sentinel
        );

        let prompt = format!("describe this image: {}", sentinel);
        let content = prompt_to_content_value(&prompt);
        let arr = content.as_array().expect("expected multipart array");
        // Should contain at least one text and one image_url part.
        let has_text = arr.iter().any(|p| p["type"] == "text");
        let img = arr
            .iter()
            .find(|p| p["type"] == "image_url")
            .expect("missing image_url part");
        assert!(has_text);
        let url = img["image_url"]["url"].as_str().unwrap();
        assert!(
            url.starts_with("data:image/png;base64,"),
            "url should be a data URL: {}",
            url
        );
        assert!(url.contains("3q2+7w=="), "url should embed base64: {}", url);

        // Cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn no_image_returns_string() {
        let v = prompt_to_content_value("just text");
        assert_eq!(v, Value::String("just text".into()));
    }

    #[test]
    fn multiple_images_in_prompt() {
        let s1 = r#"<phantom-image mime="image/png" data="AAAA"/>"#;
        let s2 = r#"<phantom-image mime="image/jpeg" data="BBBB"/>"#;
        let prompt = format!("look at {} and {}", s1, s2);
        let v = prompt_to_content_value(&prompt);
        let arr = v.as_array().expect("array");
        let n_imgs = arr.iter().filter(|p| p["type"] == "image_url").count();
        assert_eq!(n_imgs, 2);
    }

    // ── Issue #71: file-size guard ────────────────────────────────────────

    #[test]
    fn encode_image_under_limit_ok() {
        // A small (4-byte) "image" stays well under MAX_IMAGE_BYTES, so
        // encoding succeeds — the limit must not regress normal usage.
        let dir = std::env::temp_dir();
        let path = dir.join("phantom_t49_small.png");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&[0xde, 0xad, 0xbe, 0xef]).unwrap();

        let path_str = path.to_string_lossy().to_string();
        let result = encode_image_sentinel(&path_str);
        assert!(
            result.is_ok(),
            "small image should encode: {:?}",
            result.err()
        );
        let sentinel = result.unwrap();
        assert!(sentinel.contains(r#"mime="image/png""#));
        assert!(sentinel.contains(r#"data="3q2+7w==""#));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn encode_image_over_limit_clean_error() {
        // Build a sparse file whose *metadata* reports a size larger than
        // MAX_IMAGE_BYTES, without actually allocating that many bytes on
        // disk. `set_len` extends the file logically; on every supported
        // platform the unwritten region reads as zeros. `encode_image_sentinel`
        // must reject this in the metadata stage, *before* `fs::read` would
        // pull the (logical) gigabytes into RAM.
        let dir = std::env::temp_dir();
        let path = dir.join("phantom_t49_huge.png");
        {
            let f = std::fs::File::create(&path).unwrap();
            // 1 byte over the limit — explicitly tests the strict ">" guard.
            f.set_len(MAX_IMAGE_BYTES + 1).unwrap();
        }

        let path_str = path.to_string_lossy().to_string();
        let result = encode_image_sentinel(&path_str);
        assert!(result.is_err(), "over-limit image should be rejected");
        let err = result.err().unwrap();
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::InvalidInput,
            "expected InvalidInput; got: {:?}",
            err.kind()
        );
        let msg = err.to_string();
        assert!(
            msg.contains("too large"),
            "expected user-friendly message; got: {}",
            msg
        );
        assert!(msg.contains("limit"), "expected limit hint; got: {}", msg);

        let _ = std::fs::remove_file(&path);
    }

    // ── T27: additive coverage — mime/type detection + path selection ─────

    #[test]
    fn mime_covers_all_recognised_exts() {
        // Every entry in IMAGE_EXTS must resolve to a concrete image/* mime,
        // so a newly-added extension that forgets a match arm fails this.
        for ext in IMAGE_EXTS {
            let path = format!("/tmp/asset.{}", ext);
            let mime = image_mime_for_path(&path)
                .unwrap_or_else(|| panic!("ext {ext} should be recognised"));
            assert!(
                mime.starts_with("image/"),
                "ext {ext} mapped to non-image mime {mime}"
            );
        }
    }

    #[test]
    fn mime_is_case_insensitive_and_uses_last_dot() {
        // Detection lowercases and keys off the final dot segment only.
        assert_eq!(image_mime_for_path("PHOTO.PNG"), Some("image/png"));
        assert_eq!(
            image_mime_for_path("archive.tar.gz.jpeg"),
            Some("image/jpeg")
        );
        // A dot in a directory name must not be mistaken for an extension.
        assert_eq!(image_mime_for_path("/home/v1.0/notes"), None);
    }

    #[test]
    fn mime_rejects_non_image_and_empty() {
        assert_eq!(image_mime_for_path(""), None);
        assert_eq!(image_mime_for_path("README.md"), None);
        assert_eq!(image_mime_for_path("script.sh"), None);
        // A bare ".png" with no stem still detects on its extension.
        assert_eq!(image_mime_for_path(".png"), Some("image/png"));
    }

    #[test]
    fn empty_prompt_selects_text_path() {
        // No sentinel ⇒ plain-string content path, not an array.
        let v = prompt_to_content_value("");
        assert_eq!(v, Value::String(String::new()));
        assert!(v.as_array().is_none());
    }

    #[test]
    fn text_only_prompt_with_angle_bracket_stays_string() {
        // A lone "<" or non-matching tag must not trip the image path.
        let v = prompt_to_content_value("compare a < b in <code>x</code>");
        assert!(
            matches!(v, Value::String(_)),
            "non-sentinel angle brackets should stay text: {v:?}"
        );
    }

    #[test]
    fn image_only_prompt_gets_synthetic_text_part() {
        // When the prompt is purely an image, an empty text part is prepended
        // so providers that require ≥1 text part stay happy.
        let s = r#"<phantom-image mime="image/png" data="AAAA"/>"#;
        let v = prompt_to_content_value(s);
        let arr = v.as_array().expect("image path returns array");
        assert_eq!(arr[0]["type"], "text", "first part should be text");
        assert_eq!(arr[0]["text"], "");
        assert!(arr.iter().any(|p| p["type"] == "image_url"));
    }

    #[test]
    fn malformed_sentinel_without_close_kept_as_text() {
        // No "/>" terminator ⇒ remainder emitted verbatim as text, never dropped.
        let prompt = r#"hi <phantom-image mime="image/png" data="AAAA""#;
        let v = prompt_to_content_value(prompt);
        let arr = v.as_array().expect("array because needle present");
        assert!(arr.iter().all(|p| p["type"] == "text"));
        let joined: String = arr
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect();
        assert!(joined.contains("phantom-image"));
    }

    #[test]
    fn data_url_mime_round_trips_through_content() {
        // The mime in the sentinel must survive into the emitted data: URL.
        let s = r#"<phantom-image mime="image/webp" data="ZZZZ"/>"#;
        let v = prompt_to_content_value(s);
        let arr = v.as_array().unwrap();
        let img = arr.iter().find(|p| p["type"] == "image_url").unwrap();
        let url = img["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/webp;base64,ZZZZ"));
    }

    #[test]
    fn anthropic_convert_passes_through_string_content() {
        // Plain-string content is untouched by the Anthropic rewrite.
        let msg = json!({"role": "user", "content": "hello"});
        assert_eq!(convert_message_for_anthropic(&msg), msg);
    }

    #[test]
    fn anthropic_convert_passes_through_text_only_array() {
        // Array content with no image_url part is returned unchanged.
        let msg = json!({
            "role": "user",
            "content": [{"type": "text", "text": "no images here"}]
        });
        assert_eq!(convert_message_for_anthropic(&msg), msg);
    }

    #[test]
    fn anthropic_convert_rewrites_image_url_to_source() {
        // image_url parts become Anthropic's native image/source.base64 shape,
        // preserving media_type and data; sibling text parts are kept as-is.
        let msg = json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "look"},
                {"type": "image_url",
                 "image_url": {"url": "data:image/png;base64,3q2+7w=="}}
            ]
        });
        let out = convert_message_for_anthropic(&msg);
        let arr = out["content"].as_array().expect("array content");
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "look");
        assert_eq!(arr[1]["type"], "image");
        assert_eq!(arr[1]["source"]["type"], "base64");
        assert_eq!(arr[1]["source"]["media_type"], "image/png");
        assert_eq!(arr[1]["source"]["data"], "3q2+7w==");
        // role is preserved.
        assert_eq!(out["role"], "user");
    }

    #[test]
    fn anthropic_convert_passes_through_non_data_url() {
        // A remote http(s) image_url has no "data:" prefix, so it falls back
        // to pass-through rather than producing a broken source block.
        let msg = json!({
            "role": "user",
            "content": [
                {"type": "image_url",
                 "image_url": {"url": "https://example.com/cat.png"}}
            ]
        });
        let out = convert_message_for_anthropic(&msg);
        let arr = out["content"].as_array().unwrap();
        assert_eq!(arr[0]["type"], "image_url");
        assert_eq!(
            arr[0]["image_url"]["url"],
            "https://example.com/cat.png"
        );
    }
}
