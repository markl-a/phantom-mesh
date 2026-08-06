use reqwest::header;
use serde_json::Value;
use std::time::Duration;

const MAX_CHARS_DEFAULT: usize = 8_000;
const MAX_CHARS_LIMIT: usize = 50_000;
const TIMEOUT_DEFAULT_SECS: u64 = 15;

pub async fn fetch_url(args: &Value) -> String {
    let url = match args.get("url").and_then(|v| v.as_str()) {
        Some(u) => u.to_string(),
        None => return "Error: missing required argument 'url'".to_string(),
    };

    let max_length = args
        .get("max_length")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).min(MAX_CHARS_LIMIT))
        // fall back to legacy max_chars param
        .or_else(|| {
            args.get("max_chars")
                .and_then(|v| v.as_u64())
                .map(|n| (n as usize).min(MAX_CHARS_LIMIT))
        })
        .unwrap_or(MAX_CHARS_DEFAULT);

    let timeout_secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(TIMEOUT_DEFAULT_SECS);

    let raw = args.get("raw").and_then(|v| v.as_bool()).unwrap_or(false);

    let selector = args
        .get("selector")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    if let Err(e) = crate::tools::urlguard::validate_url(&url) {
        return format!("Error: {}", e);
    }

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("Mozilla/5.0 (compatible; spectyn-mesh/0.1)")
        .build()
    {
        Ok(c) => c,
        Err(e) => return format!("Error: failed to build HTTP client: {}", e),
    };

    let response = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("timed out") || msg.contains("timeout") {
                return format!(
                    "Error: Request timed out after {}s fetching <{}>",
                    timeout_secs, url
                );
            }
            return format!("Error: network error fetching <{}>: {}", url, e);
        }
    };

    let status = response.status();
    if !status.is_success() {
        return format!(
            "Error: HTTP {} {} for <{}>",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Unknown"),
            url
        );
    }

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    let is_json = content_type.contains("application/json");
    let is_text_html = content_type.contains("text/html");
    let is_text_plain = content_type.contains("text/plain");

    if !is_json && !is_text_html && !is_text_plain {
        return format!(
            "Error: unsupported content type '{}' for <{}> (only text/html, text/plain, application/json accepted)",
            content_type, url
        );
    }

    let body = match response.text().await {
        Ok(b) => b,
        Err(e) => return format!("Error: failed to read response body from <{}>: {}", url, e),
    };

    if is_json {
        let pretty = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| serde_json::to_string_pretty(&v).ok())
            .unwrap_or(body);
        return truncate_with_marker(pretty, max_length);
    }

    // Raw mode — return body as-is
    if raw {
        return truncate_with_marker(body, max_length);
    }

    // HTML / plain text processing
    let (title, text) = if is_text_html {
        extract_html(&body, selector.as_deref())
    } else {
        (String::new(), body)
    };

    let cleaned = collapse_whitespace(&text);
    let content = truncate_with_marker(cleaned, max_length);

    if title.is_empty() {
        format!("URL: {}\n---\n{}", url, content)
    } else {
        format!("Title: {}\nURL: {}\n---\n{}", title, url, content)
    }
}

// ── Truncation ────────────────────────────────────────────────────────────────

fn truncate_with_marker(s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    // Truncate at a char boundary
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[... truncated]", &s[..end])
}

// ── URL validation ────────────────────────────────────────────────────────────
// Moved to `crate::tools::urlguard` (T7b: shared with web_fetch + http_client).

// ── HTML extraction ───────────────────────────────────────────────────────────

fn extract_html(html: &str, selector: Option<&str>) -> (String, String) {
    let title = extract_title(html);

    // Remove block-level noise tags entirely (including content)
    let stripped = remove_block_tags(html, &["script", "style", "nav", "footer", "header"]);

    // Remove HTML comments <!-- ... -->
    let stripped = remove_html_comments(&stripped);

    // If a selector hint was provided, try to narrow to that block
    let body_html = if let Some(sel) = selector {
        extract_by_tag_hint(&stripped, sel).unwrap_or(stripped)
    } else {
        stripped
    };

    let text = html_to_text(&body_html);
    let decoded = decode_entities(&text);

    (title, decoded)
}

fn extract_title(html: &str) -> String {
    let lower = html.to_lowercase();
    if let Some(start) = lower.find("<title") {
        if let Some(end_open) = lower[start..].find('>') {
            let content_start = start + end_open + 1;
            if let Some(end_tag) = lower[content_start..].find("</title>") {
                let raw = &html[content_start..content_start + end_tag];
                return decode_entities(raw).trim().to_string();
            }
        }
    }
    String::new()
}

/// Remove `<!-- ... -->` comments (single-pass, handles multi-line).
fn remove_html_comments(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut pos = 0;
    let bytes = html.as_bytes();
    let len = html.len();

    while pos < len {
        // Look for comment start
        if pos + 3 < len && &bytes[pos..pos + 4] == b"<!--" {
            // Find closing -->
            if let Some(rel) = html[pos + 4..].find("-->") {
                pos = pos + 4 + rel + 3; // skip past -->
            } else {
                // Unclosed comment — skip to end
                break;
            }
        } else {
            // Find next potential comment start
            if let Some(rel) = html[pos..].find("<!--") {
                out.push_str(&html[pos..pos + rel]);
                pos += rel;
            } else {
                out.push_str(&html[pos..]);
                break;
            }
        }
    }
    out
}

/// Remove entire `<tag ...>...</tag>` blocks (case-insensitive, handles nesting naively).
fn remove_block_tags(html: &str, tags: &[&str]) -> String {
    let mut result = html.to_string();
    for tag in tags {
        result = remove_one_block_tag(&result, tag);
    }
    result
}

fn remove_one_block_tag(html: &str, tag: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let lower = html.to_lowercase();
    let open_pat = format!("<{}", tag);
    let close_pat = format!("</{}>", tag);
    let mut pos = 0;

    while pos < html.len() {
        if let Some(rel) = lower[pos..].find(open_pat.as_str()) {
            let tag_start = pos + rel;
            // Ensure next char is '>' or whitespace (prevents <navigate> matching <nav>)
            let after = &lower[tag_start + open_pat.len()..];
            if !after.starts_with('>')
                && !after.starts_with(' ')
                && !after.starts_with('\t')
                && !after.starts_with('\n')
                && !after.starts_with('/')
            {
                out.push_str(&html[pos..tag_start + 1]);
                pos = tag_start + 1;
                continue;
            }
            out.push_str(&html[pos..tag_start]);
            if let Some(rel_close) = lower[tag_start..].find(close_pat.as_str()) {
                pos = tag_start + rel_close + close_pat.len();
            } else {
                break;
            }
        } else {
            out.push_str(&html[pos..]);
            break;
        }
    }
    out
}

/// Try to find the first element matching a simple tag name hint (e.g. "article", "main").
fn extract_by_tag_hint(html: &str, tag_hint: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let open_pat = format!("<{}", tag_hint);
    let close_pat = format!("</{}>", tag_hint);

    let start = lower.find(open_pat.as_str())?;
    let after = &lower[start + open_pat.len()..];
    if !after.starts_with('>')
        && !after.starts_with(' ')
        && !after.starts_with('\t')
        && !after.starts_with('\n')
    {
        return None;
    }
    let gt = lower[start..].find('>')?;
    let content_start = start + gt + 1;

    let close_pos = lower[content_start..].find(close_pat.as_str())?;
    Some(html[content_start..content_start + close_pos].to_string())
}

/// Convert HTML tags to readable text with semantic newlines and heading markers.
///
/// Rules (applied left-to-right while scanning):
/// - `<br>`, `<p>`, `<div>`, `<li>` opening tags  → newline before content
/// - `</p>`, `</div>`, `</li>`                     → newline after content
/// - `<h1>`–`<h6>` opening                         → `\n## ` prefix
/// - `</h1>`–`</h6>`                               → `\n` suffix
/// - All other tags stripped (replaced with space)
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut pos = 0;
    let len = html.len();
    let lower = html.to_lowercase();

    while pos < len {
        if let Some(rel) = lower[pos..].find('<') {
            // Emit text before the tag
            out.push_str(&html[pos..pos + rel]);
            let tag_start = pos + rel;

            // Find end of tag
            let close = match lower[tag_start..].find('>') {
                Some(r) => tag_start + r,
                None => {
                    // No closing '>' — emit rest as text
                    out.push_str(&html[tag_start..]);
                    break;
                }
            };

            let tag_inner = &lower[tag_start + 1..close]; // e.g. "br", "/p", "h2 class=..."
            let tag_name = tag_inner
                .trim_start_matches('/')
                .split(|c: char| c.is_whitespace() || c == '>')
                .next()
                .unwrap_or("")
                .trim();

            let is_closing = tag_inner.starts_with('/');

            match tag_name {
                "br" => out.push('\n'),
                "p" | "div" | "li" => {
                    if !is_closing {
                        out.push('\n');
                    } else {
                        out.push('\n');
                    }
                }
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    if !is_closing {
                        out.push_str("\n## ");
                    } else {
                        out.push('\n');
                    }
                }
                _ => {
                    // All other tags — emit a space to avoid word-merging
                    out.push(' ');
                }
            }

            pos = close + 1;
        } else {
            // No more tags
            out.push_str(&html[pos..]);
            break;
        }
    }

    out
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_newline = false;
    let mut last_was_space = false;
    let mut newline_count = 0usize;

    for ch in s.chars() {
        match ch {
            '\n' | '\r' => {
                newline_count += 1;
                last_was_space = false;
                if newline_count <= 2 {
                    out.push('\n');
                    last_was_newline = true;
                }
            }
            ' ' | '\t' => {
                newline_count = 0;
                if !last_was_newline && !last_was_space {
                    out.push(' ');
                    last_was_space = true;
                }
            }
            _ => {
                newline_count = 0;
                last_was_newline = false;
                last_was_space = false;
                out.push(ch);
            }
        }
    }
    out.trim().to_string()
}

// ── JSON schema (tool definition) ────────────────────────────────────────────

pub fn schema() -> serde_json::Value {
    serde_json::json!({
        "name": "fetch",
        "description": "Fetch a URL and return its content as clean text (HTML stripped). Supports HTML, plain text, and JSON responses.",
        "input_schema": {
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch (must start with http:// or https://)."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Request timeout in seconds (default: 15).",
                    "default": 15
                },
                "max_length": {
                    "type": "integer",
                    "description": "Maximum characters to return (default: 8000, max: 50000). Content beyond this limit is truncated with '[... truncated]'.",
                    "default": 8000
                },
                "raw": {
                    "type": "boolean",
                    "description": "If true, return the raw HTML/text without any stripping or conversion (default: false).",
                    "default": false
                },
                "selector": {
                    "type": "string",
                    "description": "Optional tag name hint (e.g. 'article', 'main') to narrow extraction to a specific element."
                }
            },
            "required": ["url"]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: validate_url + is_private_ipv4 tests moved to
    // `crate::tools::urlguard::tests` (T7b shared SSRF guard).

    #[test]
    fn test_decode_entities() {
        assert_eq!(decode_entities("a &amp; b &lt;c&gt;"), "a & b <c>");
        assert_eq!(decode_entities("&quot;hello&quot;"), "\"hello\"");
        assert_eq!(decode_entities("it&#39;s"), "it's");
    }

    #[test]
    fn test_collapse_whitespace() {
        assert_eq!(collapse_whitespace("  foo   bar  "), "foo bar");
        assert_eq!(collapse_whitespace("a\n\n\n\nb"), "a\n\nb");
    }

    #[test]
    fn test_extract_title() {
        let html = "<html><head><title>Hello World</title></head></html>";
        assert_eq!(extract_title(html), "Hello World");
    }

    #[test]
    fn test_remove_script_tag() {
        let html = "before<script>evil()</script>after";
        let result = remove_block_tags(html, &["script"]);
        assert!(!result.contains("evil"));
        assert!(result.contains("before"));
        assert!(result.contains("after"));
    }

    #[test]
    fn test_remove_style_tag() {
        let html = "text<style>body { color: red; }</style>more";
        let result = remove_block_tags(html, &["style"]);
        assert!(!result.contains("color"));
        assert!(result.contains("text"));
        assert!(result.contains("more"));
    }

    #[test]
    fn test_remove_nav_footer_header() {
        let html =
            "<header>site header</header><p>content</p><nav>menu</nav><footer>site footer</footer>";
        let result = remove_block_tags(html, &["nav", "footer", "header"]);
        assert!(!result.contains("site header"));
        assert!(!result.contains("menu"));
        assert!(!result.contains("site footer"));
        assert!(result.contains("content"));
    }

    #[test]
    fn test_remove_html_comments() {
        let html = "before<!-- this is a comment -->after";
        let result = remove_html_comments(html);
        assert!(!result.contains("this is a comment"));
        assert!(result.contains("before"));
        assert!(result.contains("after"));
    }

    #[test]
    fn test_remove_multiline_comment() {
        let html = "a<!-- \n multi\n line\n -->b";
        let result = remove_html_comments(html);
        assert_eq!(result, "ab");
    }

    #[test]
    fn test_html_to_text_headings() {
        let html = "<h1>Title</h1><p>Para</p>";
        let result = html_to_text(html);
        assert!(result.contains("## Title"));
        assert!(result.contains("Para"));
    }

    #[test]
    fn test_html_to_text_br_newline() {
        let html = "line1<br>line2";
        let result = html_to_text(html);
        assert!(result.contains("line1\nline2"));
    }

    #[test]
    fn test_truncate_with_marker() {
        let s = "hello world".to_string();
        let truncated = truncate_with_marker(s, 5);
        assert!(truncated.starts_with("hello"));
        assert!(truncated.contains("[... truncated]"));
    }

    #[test]
    fn test_truncate_no_op_when_short() {
        let s = "short".to_string();
        assert_eq!(truncate_with_marker(s, 100), "short");
    }

    #[test]
    fn test_nav_tag_not_matching_navigate() {
        // <navigate> should NOT be stripped by the <nav> remover
        let html = "x<navigate to='foo'>content</navigate>y";
        let result = remove_block_tags(html, &["nav"]);
        assert!(result.contains("content"));
    }
}
