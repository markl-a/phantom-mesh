use crate::config::ToolsConfig;
use serde_json::Value;

pub async fn search(args: &Value, config: &ToolsConfig) -> String {
    let query = match args["query"].as_str() {
        Some(q) => q,
        None => return "Error: missing 'query' argument".into(),
    };
    let num_results = args["num_results"]
        .as_u64()
        .map(|n| (n as usize).min(10))
        .unwrap_or(5);

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; spectyn-mesh/0.1)")
        .build()
        .unwrap_or_default();

    if let Some(brave_key) = &config.brave_search_api_key {
        brave_search(&client, query, brave_key, num_results).await
    } else {
        ddg_search(&client, query, num_results).await
    }
}

fn format_results(results: &[(String, String, String)]) -> String {
    results
        .iter()
        .enumerate()
        .map(|(i, (title, url, snippet))| {
            let snippet_trimmed = if snippet.chars().count() > 200 {
                let truncated: String = snippet.chars().take(200).collect();
                format!("{}…", truncated)
            } else {
                snippet.clone()
            };
            format!(
                "[{}] {}\n    URL: {}\n    Snippet: {}",
                i + 1,
                title,
                url,
                snippet_trimmed
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

async fn brave_search(
    client: &reqwest::Client,
    query: &str,
    key: &str,
    num_results: usize,
) -> String {
    let encoded = urlencoding::encode(query);
    let url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
        encoded, num_results
    );
    match client
        .get(&url)
        .header("Accept", "application/json")
        .header("Accept-Encoding", "gzip")
        .header("X-Subscription-Token", key)
        .send()
        .await
    {
        Ok(resp) => match resp.json::<Value>().await {
            Ok(data) => {
                let mut results: Vec<(String, String, String)> = Vec::new();
                if let Some(web) = data["web"]["results"].as_array() {
                    for r in web.iter().take(num_results) {
                        let title = r["title"].as_str().unwrap_or("").to_string();
                        let url_str = r["url"].as_str().unwrap_or("").to_string();
                        let desc = r["description"].as_str().unwrap_or("").to_string();
                        results.push((title, url_str, desc));
                    }
                }
                if results.is_empty() {
                    format!("No results for: {}", query)
                } else {
                    format_results(&results)
                }
            }
            Err(_) => format!("Brave search returned no structured results for: {}", query),
        },
        Err(e) => format!("Brave search error: {}", e),
    }
}

async fn ddg_search(client: &reqwest::Client, query: &str, num_results: usize) -> String {
    // First try the Instant Answer API
    if let Some(results) = ddg_instant_answer(client, query, num_results).await {
        return results;
    }
    // Fallback to DuckDuckGo HTML search
    ddg_html_search(client, query, num_results).await
}

async fn ddg_instant_answer(
    client: &reqwest::Client,
    query: &str,
    num_results: usize,
) -> Option<String> {
    let encoded = urlencoding::encode(query);
    let url = format!(
        "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
        encoded
    );
    let resp = client.get(&url).send().await.ok()?;
    let data: Value = resp.json().await.ok()?;

    let mut results: Vec<(String, String, String)> = Vec::new();

    // Check Abstract (direct answer)
    if let (Some(text), Some(url_str)) = (
        data["Abstract"].as_str().filter(|s| !s.is_empty()),
        data["AbstractURL"].as_str().filter(|s| !s.is_empty()),
    ) {
        let source = data["AbstractSource"].as_str().unwrap_or("DuckDuckGo");
        results.push((source.to_string(), url_str.to_string(), text.to_string()));
    }

    // Check RelatedTopics
    if let Some(related) = data["RelatedTopics"].as_array() {
        for topic in related.iter() {
            if results.len() >= num_results {
                break;
            }
            // Some topics are groups with nested Topics
            if let Some(sub_topics) = topic["Topics"].as_array() {
                for sub in sub_topics.iter() {
                    if results.len() >= num_results {
                        break;
                    }
                    push_ddg_topic(&mut results, sub);
                }
            } else {
                push_ddg_topic(&mut results, topic);
            }
        }
    }

    if results.is_empty() {
        None
    } else {
        Some(format_results(&results))
    }
}

fn push_ddg_topic(results: &mut Vec<(String, String, String)>, topic: &Value) {
    let text = topic["Text"].as_str().unwrap_or("").to_string();
    let url_str = topic["FirstURL"].as_str().unwrap_or("").to_string();
    if text.is_empty() && url_str.is_empty() {
        return;
    }
    // Use the last path segment of the URL as a rough title, or truncate text
    let title = if !url_str.is_empty() {
        url_str
            .rsplit('/')
            .next()
            .unwrap_or(&url_str)
            .replace('_', " ")
    } else {
        text.chars().take(60).collect::<String>()
    };
    results.push((title, url_str, text));
}

async fn ddg_html_search(client: &reqwest::Client, query: &str, num_results: usize) -> String {
    let encoded = urlencoding::encode(query);
    let url = format!("https://html.duckduckgo.com/html/?q={}", encoded);
    let html = match client
        .get(&url)
        .header("Accept", "text/html,application/xhtml+xml")
        .send()
        .await
    {
        Ok(resp) => match resp.text().await {
            Ok(text) => text,
            Err(e) => return format!("Web search error reading response: {}", e),
        },
        Err(e) => return format!("Web search error: {}", e),
    };

    let results = parse_ddg_html(&html, num_results);
    if results.is_empty() {
        format!(
            "No results for: {} (tip: add brave_search_api_key in agents.toml for better results)",
            query
        )
    } else {
        format_results(&results)
    }
}

/// Parse DuckDuckGo HTML search results.
/// Looks for result links (`<a class="result__a"`) and snippets (`<a class="result__snippet"`).
fn parse_ddg_html(html: &str, num_results: usize) -> Vec<(String, String, String)> {
    let mut results: Vec<(String, String, String)> = Vec::new();

    // We'll scan for result__a anchors to get title+url, then result__snippet for description.
    // Pattern: <a class="result__a" href="...">title</a>
    // and:     <a class="result__snippet" ...>snippet</a>
    //
    // Simple state-machine parser — no regex or external crate needed.

    let mut pos = 0;
    let bytes = html.as_bytes();

    while pos < bytes.len() && results.len() < num_results {
        // Find next result__a
        let marker = b"result__a\"";
        if let Some(found) = find_bytes(&html[pos..], marker) {
            let abs = pos + found;
            // find href="..."
            if let Some((href, title, after_a)) = extract_anchor(&html[abs..]) {
                // Resolve DuckDuckGo redirect: /l/?uddg=<encoded-url>
                let resolved_url = resolve_ddg_url(&href);

                // Look for snippet after this anchor
                let snippet_search_start = abs + after_a;
                let snippet_end = snippet_search_start
                    + html[snippet_search_start..]
                        .find("result__a\"")
                        .unwrap_or(html.len() - snippet_search_start);

                let snippet = if let Some(snip_pos) = find_bytes(
                    &html[snippet_search_start..snippet_end],
                    b"result__snippet\"",
                ) {
                    let snip_abs = snippet_search_start + snip_pos;
                    extract_anchor_text(&html[snip_abs..]).unwrap_or_default()
                } else {
                    String::new()
                };

                results.push((title, resolved_url, snippet));
                pos = abs + after_a;
            } else {
                pos = abs + marker.len();
            }
        } else {
            break;
        }
    }

    results
}

/// Search for `needle` bytes in `haystack`, return position of first occurrence.
fn find_bytes(haystack: &str, needle: &[u8]) -> Option<usize> {
    let h = haystack.as_bytes();
    let n = needle.len();
    if n == 0 || h.len() < n {
        return None;
    }
    h.windows(n).position(|w| w == needle)
}

/// Extract href and inner text from an `<a ...>text</a>` anchor starting at `src`.
/// Returns (href, inner_text, bytes_consumed).
fn extract_anchor(src: &str) -> Option<(String, String, usize)> {
    // Find href="
    let href_start = src.find("href=\"")?;
    let href_content_start = href_start + 6;
    let href_end = src[href_content_start..].find('"')?;
    let href = html_decode(&src[href_content_start..href_content_start + href_end]);

    // Find > (end of opening tag)
    let tag_close = src.find('>')?;
    let inner_start = tag_close + 1;

    // Find </a>
    let inner_end = src[inner_start..].find("</a>")?;
    let inner_text = strip_tags(&src[inner_start..inner_start + inner_end]);

    Some((href, inner_text, inner_start + inner_end + 4))
}

/// Extract just the inner text of the next `<a ...>text</a>` anchor in src.
fn extract_anchor_text(src: &str) -> Option<String> {
    let tag_close = src.find('>')?;
    let inner_start = tag_close + 1;
    let inner_end = src[inner_start..].find("</a>")?;
    Some(strip_tags(&src[inner_start..inner_start + inner_end]))
}

/// Remove HTML tags from a string slice and decode basic entities.
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    html_decode(&out)
}

/// Decode basic HTML entities.
fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

/// Resolve DuckDuckGo redirect URLs of the form `/l/?uddg=<encoded>` or `/l/?kh=-1&uddg=<encoded>`.
fn resolve_ddg_url(href: &str) -> String {
    if href.starts_with("/l/?") || href.starts_with("https://duckduckgo.com/l/?") {
        // Extract uddg parameter
        if let Some(uddg_pos) = href.find("uddg=") {
            let encoded = &href[uddg_pos + 5..];
            // Stop at next &
            let end = encoded.find('&').unwrap_or(encoded.len());
            if let Ok(decoded) = urlencoding::decode(&encoded[..end]) {
                return decoded.into_owned();
            }
        }
    }
    href.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_results_truncates_on_char_boundary_for_cjk() {
        // Regression: byte-slicing on CJK input panicked at src/tools/web.rs:32
        // when len() > 200 landed inside a multi-byte char (e.g. '能').
        let long_cjk: String = "根據".repeat(200); // 200 chars × 3 bytes = 600 bytes
        let results = vec![("t".to_string(), "u".to_string(), long_cjk)];
        let out = format_results(&results); // must not panic
        assert!(out.contains('…'));
    }

    #[test]
    fn format_results_passes_short_snippet_through() {
        let results = vec![("t".into(), "u".into(), "short".into())];
        let out = format_results(&results);
        assert!(out.contains("short"));
        assert!(!out.contains('…'));
    }
}
