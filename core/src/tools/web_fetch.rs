//! `web_fetch` — fetch a URL and return body converted to readable plain text.
//!
//! Unlike `http_get` (which returns raw response with status/headers), this
//! tool strips HTML tags, decodes common entities, collapses whitespace, and
//! returns just the readable text. Useful for feeding web pages to an LLM.

use reqwest::ClientBuilder;
use serde_json::Value;
use std::time::Duration;

const MAX_CHARS_DEFAULT: usize = 50_000;
const MAX_CHARS_LIMIT: usize = 200_000;
const TIMEOUT_SECS: u64 = 30;

/// Strip HTML tags and decode a small set of common HTML entities.
/// This is intentionally simple — for v0.1.0 a regex-style stripper is
/// sufficient. We do NOT pull in a full HTML parser.
fn html_to_text(html: &str) -> String {
    // 1. Strip <script>...</script> and <style>...</style> blocks.
    let mut s = strip_block(html, "<script", "</script>");
    s = strip_block(&s, "<style", "</style>");
    // Also drop HTML comments.
    s = strip_block(&s, "<!--", "-->");

    // 2. Replace block-level closing tags with newlines so paragraphs survive.
    let block_closes = [
        "</p>",
        "</div>",
        "</section>",
        "</article>",
        "</header>",
        "</footer>",
        "</li>",
        "</ul>",
        "</ol>",
        "</tr>",
        "</table>",
        "</h1>",
        "</h2>",
        "</h3>",
        "</h4>",
        "</h5>",
        "</h6>",
        "</blockquote>",
        "</pre>",
    ];
    for close in &block_closes {
        s = s.replace(close, "\n");
        // Case-insensitive: also try upper-case variant.
        s = s.replace(&close.to_uppercase(), "\n");
    }
    // <br> and <br/> → newline
    for br in &["<br>", "<br/>", "<br />", "<BR>", "<BR/>", "<BR />"] {
        s = s.replace(br, "\n");
    }

    // 3. Strip every remaining <...> tag.
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut in_tag = false;
    while i < bytes.len() {
        let b = bytes[i];
        if !in_tag && b == b'<' {
            in_tag = true;
        } else if in_tag && b == b'>' {
            in_tag = false;
        } else if !in_tag {
            out.push(char::from(b));
        }
        i += 1;
    }

    // 4. Decode common HTML entities.
    let decoded = decode_entities(&out);

    // 5. Collapse whitespace runs.
    collapse_whitespace(&decoded)
}

/// Remove every `start..=end` block (case-insensitive on the start tag prefix).
fn strip_block(input: &str, start_tag: &str, end_tag: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let start_lower = start_tag.to_ascii_lowercase();
    let end_lower = end_tag.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;
    while cursor < input.len() {
        match lower[cursor..].find(&start_lower) {
            Some(rel_start) => {
                let abs_start = cursor + rel_start;
                out.push_str(&input[cursor..abs_start]);
                match lower[abs_start..].find(&end_lower) {
                    Some(rel_end) => {
                        cursor = abs_start + rel_end + end_tag.len();
                    }
                    None => {
                        // Unterminated — drop the rest.
                        return out;
                    }
                }
            }
            None => {
                out.push_str(&input[cursor..]);
                break;
            }
        }
    }
    out
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            // Find the closing ';' within next 10 bytes.
            let max_end = (i + 10).min(bytes.len());
            if let Some(rel) = bytes[i + 1..max_end].iter().position(|&b| b == b';') {
                let entity = &s[i + 1..i + 1 + rel];
                let replaced = match entity {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some(' '),
                    "copy" => Some('\u{00A9}'),
                    "reg" => Some('\u{00AE}'),
                    "mdash" => Some('\u{2014}'),
                    "ndash" => Some('\u{2013}'),
                    "hellip" => Some('\u{2026}'),
                    "lsquo" => Some('\u{2018}'),
                    "rsquo" => Some('\u{2019}'),
                    "ldquo" => Some('\u{201C}'),
                    "rdquo" => Some('\u{201D}'),
                    e if e.starts_with("#x") || e.starts_with("#X") => {
                        u32::from_str_radix(&e[2..], 16)
                            .ok()
                            .and_then(char::from_u32)
                    }
                    e if e.starts_with('#') => e[1..].parse::<u32>().ok().and_then(char::from_u32),
                    _ => None,
                };
                if let Some(c) = replaced {
                    out.push(c);
                    i += 1 + rel + 1;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_newline = false;
    let mut last_was_space = false;
    let mut at_line_start = true;
    for ch in s.chars() {
        if ch == '\n' {
            if !last_was_newline {
                // Trim trailing space before newline.
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push('\n');
            } else if !out.ends_with("\n\n") {
                out.push('\n');
            }
            last_was_newline = true;
            last_was_space = false;
            at_line_start = true;
        } else if ch == ' ' || ch == '\t' || ch == '\r' {
            if at_line_start {
                continue;
            }
            if !last_was_space {
                out.push(' ');
            }
            last_was_space = true;
            last_was_newline = false;
        } else {
            out.push(ch);
            last_was_space = false;
            last_was_newline = false;
            at_line_start = false;
        }
    }
    // Trim trailing whitespace.
    while out.ends_with('\n') || out.ends_with(' ') {
        out.pop();
    }
    out
}

pub async fn fetch(args: &Value) -> String {
    let url = match args.get("url").and_then(|v| v.as_str()) {
        Some(u) => u.to_string(),
        None => return "ERROR: missing required parameter 'url'".to_string(),
    };
    // T7b T13-N6: SSRF guard. Blocks loopback / private / link-local hosts
    // unless PHANTOM_FETCH_ALLOW_LOCAL=1 is set.
    if let Err(e) = crate::tools::urlguard::validate_url(&url) {
        return format!("ERROR: {}", e);
    }
    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).min(MAX_CHARS_LIMIT))
        .unwrap_or(MAX_CHARS_DEFAULT);

    let client = match ClientBuilder::new()
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .use_rustls_tls()
        .user_agent("phantom-mesh/web_fetch")
        .build()
    {
        Ok(c) => c,
        Err(e) => return format!("ERROR: failed to build HTTP client: {}", e),
    };

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => return format!("ERROR: request failed: {}", e),
    };

    let status = resp.status();
    if !status.is_success() {
        return format!(
            "ERROR: HTTP {} {}\nURL: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            url
        );
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => return format!("ERROR: failed to read body: {}", e),
    };

    let text = if content_type.contains("html") || body.trim_start().starts_with('<') {
        html_to_text(&body)
    } else {
        // Already plain text / JSON / markdown — just normalize whitespace.
        collapse_whitespace(&body)
    };

    if text.chars().count() > max_chars {
        let mut truncated: String = text.chars().take(max_chars).collect();
        truncated.push_str(&format!(
            "\n\n[... truncated, {} of {} chars shown ...]",
            max_chars,
            text.chars().count()
        ));
        truncated
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_basic_tags() {
        let html = "<p>hello <b>world</b></p>";
        let out = html_to_text(html);
        assert!(out.contains("hello"));
        assert!(out.contains("world"));
        assert!(!out.contains("<"));
    }

    #[test]
    fn strips_script_block() {
        let html = "before<script>alert('x')</script>after";
        let out = html_to_text(html);
        assert!(!out.contains("alert"));
        assert!(out.contains("before"));
        assert!(out.contains("after"));
    }

    #[test]
    fn strips_style_block() {
        let html = "<style>body{color:red}</style>visible";
        let out = html_to_text(html);
        assert!(!out.contains("color:red"));
        assert!(out.contains("visible"));
    }

    #[test]
    fn decodes_entities() {
        let html = "Tom &amp; Jerry &lt;3 &#65;";
        let out = html_to_text(html);
        assert!(out.contains("Tom & Jerry <3 A"), "got: {}", out);
    }

    #[test]
    fn collapses_whitespace() {
        let html = "a   b\n\n\n\nc";
        let out = html_to_text(html);
        assert_eq!(out, "a b\n\nc");
    }

    #[tokio::test]
    async fn missing_url() {
        let args = serde_json::json!({});
        let res = fetch(&args).await;
        assert!(res.starts_with("ERROR:"), "got: {}", res);
    }
}
