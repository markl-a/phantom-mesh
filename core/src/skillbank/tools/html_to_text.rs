//! `skill_html_to_text` — strip HTML tags + decode common entities.
//!
//! Minimal regex-free state machine. Drops `<script>` and `<style>`
//! contents, decodes named (&amp; &lt; &gt; &quot; &apos; &nbsp;) and
//! numeric (&#NNN; &#xHH;) character references, collapses whitespace.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{SkillTool, ToolError, ToolResult};

pub struct HtmlToText;

#[async_trait]
impl SkillTool for HtmlToText {
    fn name(&self) -> &'static str {
        "skill_html_to_text"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "skill_html_to_text",
                "description": "Strip HTML tags from `html`, decode common entities \
                    (named: amp/lt/gt/quot/apos/nbsp; numeric: &#N; and &#xH;), \
                    and collapse whitespace.",
                "parameters": {
                    "type": "object",
                    "properties": { "html": {"type": "string"} },
                    "required": ["html"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let html = args
            .get("html")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("html required".into()))?;
        let stripped = strip_tags(html);
        let decoded = decode_entities(&stripped);
        let collapsed = collapse_ws(&decoded);
        Ok(json!({ "text": collapsed }))
    }
}

fn strip_tags(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let bytes = html.as_bytes();
    let lbytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Is it <script ...> or <style ...>?
            let skip_block = ["<script", "<style"].iter().find_map(|tag| {
                let tb = tag.as_bytes();
                if lbytes[i..].starts_with(tb) {
                    Some(*tag)
                } else {
                    None
                }
            });
            if let Some(tag) = skip_block {
                // Find matching </script> or </style>.
                let close = format!("</{}>", &tag[1..]);
                if let Some(end_rel) = lbytes[i..]
                    .windows(close.len())
                    .position(|w| w == close.as_bytes())
                {
                    i += end_rel + close.len();
                    // No filler char: adjacent text should not gain a space.
                    continue;
                } else {
                    // No close → drop rest.
                    break;
                }
            }
            // Generic tag: scan to '>'.
            if let Some(end_rel) = bytes[i..].iter().position(|&b| b == b'>') {
                i += end_rel + 1;
                // No filler char: subsequent `collapse_ws` will keep any
                // whitespace that surrounded the tag in the source.
                continue;
            } else {
                break;
            }
        } else {
            // Push one UTF-8 char.
            let ch_end = next_char_boundary(bytes, i);
            out.push_str(&html[i..ch_end]);
            i = ch_end;
        }
    }
    out
}

fn next_char_boundary(bytes: &[u8], i: usize) -> usize {
    // Returns the byte index of the start of the next char after `i`.
    let first = bytes[i];
    let width = if first < 0x80 {
        1
    } else if first < 0xC0 {
        1
    }
    // continuation; should not start a char, but be defensive
    else if first < 0xE0 {
        2
    } else if first < 0xF0 {
        3
    } else {
        4
    };
    (i + width).min(bytes.len())
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        let mut buf = String::new();
        let mut found_semi = false;
        while let Some(&nc) = chars.peek() {
            if nc == ';' {
                chars.next();
                found_semi = true;
                break;
            }
            if buf.len() >= 8 {
                break;
            }
            buf.push(nc);
            chars.next();
        }
        if !found_semi {
            // Not an entity; restore.
            out.push('&');
            out.push_str(&buf);
            continue;
        }
        let decoded = match buf.as_str() {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            other if other.starts_with('#') => {
                let rest = &other[1..];
                let codepoint =
                    if let Some(hex) = rest.strip_prefix('x').or_else(|| rest.strip_prefix('X')) {
                        u32::from_str_radix(hex, 16).ok()
                    } else {
                        rest.parse::<u32>().ok()
                    };
                codepoint.and_then(char::from_u32)
            }
            _ => None,
        };
        match decoded {
            Some(ch) => out.push(ch),
            None => {
                out.push('&');
                out.push_str(&buf);
                out.push(';');
            }
        }
    }
    out
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_ws = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_ws && !out.is_empty() {
                out.push(' ');
            }
            prev_ws = true;
        } else {
            out.push(c);
            prev_ws = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn strips_tags_and_decodes_named_entities() {
        let tool = HtmlToText;
        let r = tool
            .call(&json!({
                "html": "<p>Hello &amp; <b>world</b>!</p>"
            }))
            .await
            .unwrap();
        assert_eq!(r["text"], "Hello & world!");
    }

    #[tokio::test]
    async fn drops_script_and_style_content() {
        let tool = HtmlToText;
        let r = tool
            .call(&json!({
                "html": "<style>.x{color:red}</style>visible<script>alert(1)</script>"
            }))
            .await
            .unwrap();
        assert_eq!(r["text"], "visible");
    }

    #[tokio::test]
    async fn decodes_numeric_entities_decimal_and_hex() {
        let tool = HtmlToText;
        let r = tool
            .call(&json!({"html": "&#65;&#x42;&#x43;"}))
            .await
            .unwrap();
        assert_eq!(r["text"], "ABC");
    }
}
