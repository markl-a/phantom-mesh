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

/// Returns `Some(mime)` if `path` ends with a recognised image extension.
pub fn image_mime_for_path(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    if !IMAGE_EXTS.contains(&ext.as_str()) {
        return None;
    }
    Some(match ext.as_str() {
        "png"          => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif"          => "image/gif",
        "webp"         => "image/webp",
        _              => "application/octet-stream",
    })
}

/// Read `path`, base64-encode the bytes, and return the sentinel string that
/// later gets parsed into an `image_url` part. Returns `Err` if the file
/// cannot be read.
pub fn encode_image_sentinel(path: &str) -> std::io::Result<String> {
    let mime = image_mime_for_path(path).unwrap_or("application/octet-stream");
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

    let new_parts: Vec<Value> = arr.iter().map(|p| {
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
    }).collect();

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
        assert!(sentinel.contains(r#"data="3q2+7w==""#),
            "expected base64 of [de ad be ef]; got: {}", sentinel);

        let prompt = format!("describe this image: {}", sentinel);
        let content = prompt_to_content_value(&prompt);
        let arr = content.as_array().expect("expected multipart array");
        // Should contain at least one text and one image_url part.
        let has_text = arr.iter().any(|p| p["type"] == "text");
        let img = arr.iter().find(|p| p["type"] == "image_url")
            .expect("missing image_url part");
        assert!(has_text);
        let url = img["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/png;base64,"),
            "url should be a data URL: {}", url);
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
}
