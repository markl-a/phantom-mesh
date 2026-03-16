// web_search tool — multi-backend web search with fallback chain
// Backend 1: Serper API (Google search results as JSON) — primary
// Backend 2: Tavily API (AI-focused search) — fallback
// Backend 3: Google News RSS (always works, for news queries)
// Backend 4: Direct URL fetch (agent can read specific pages)

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{debug, info, warn};

use super::{Tool, ToolResult};

/// Search backend API keys loaded from config
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchConfig {
    #[serde(default)]
    pub serper_api_key: String,
    #[serde(default)]
    pub tavily_api_key: String,
    #[serde(default)]
    pub brave_api_key: String,
    #[serde(default)]
    pub exa_api_key: String,
    #[serde(default)]
    pub langsearch_api_key: String,
    #[serde(default)]
    pub searchapi_api_key: String,
}

pub struct WebSearchTool {
    client: Client,
    config: SearchConfig,
}

impl WebSearchTool {
    pub fn new(config: SearchConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { client, config }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str { "web_search" }

    fn description(&self) -> &str {
        "Search the web or fetch a URL. Modes: 'search' (general web search via Google), 'news' (Google News RSS), 'fetch' (read a specific URL)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query (for mode=search/news) or URL (for mode=fetch)"
                },
                "mode": {
                    "type": "string",
                    "description": "Either 'search' (default, general web), 'news' (Google News), or 'fetch' (read a URL directly)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if query.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Error: empty query".to_string(),
            });
        }

        let mode = args
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("search");

        match mode {
            "fetch" => self.fetch_url(query).await,
            "news" => self.search_news(query).await,
            _ => self.search(query).await,
        }
    }
}

impl WebSearchTool {
    /// General web search with fallback chain: Serper → Tavily → Google News RSS
    async fn search(&self, query: &str) -> Result<ToolResult> {
        info!("Web search: '{}'", query);

        // Try Serper first (Google results as JSON)
        if !self.config.serper_api_key.is_empty() {
            match self.search_serper(query).await {
                Ok(result) if result.success => return Ok(result),
                Ok(result) => {
                    warn!("Serper returned failure: {}", result.output);
                }
                Err(e) => {
                    warn!("Serper error: {}", e);
                }
            }
        }

        // Fallback to Tavily
        if !self.config.tavily_api_key.is_empty() {
            match self.search_tavily(query).await {
                Ok(result) if result.success => return Ok(result),
                Ok(result) => {
                    warn!("Tavily returned failure: {}", result.output);
                }
                Err(e) => {
                    warn!("Tavily error: {}", e);
                }
            }
        }

        // Final fallback: Google News RSS
        info!("All API backends failed or unavailable, falling back to Google News RSS");
        self.search_news(query).await
    }

    /// Serper API — returns Google search results as structured JSON
    async fn search_serper(&self, query: &str) -> Result<ToolResult> {
        debug!("Trying Serper API for: '{}'", query);

        let resp = self.client
            .post("https://google.serper.dev/search")
            .header("X-API-KEY", &self.config.serper_api_key)
            .header("Content-Type", "application/json")
            .json(&json!({
                "q": query,
                "gl": "tw",
                "hl": "zh-tw",
                "num": 8
            }))
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Ok(ToolResult {
                success: false,
                output: format!("Serper API error (HTTP {}): {}", status.as_u16(), body),
            });
        }

        let data: Value = resp.json().await?;

        let mut output = String::new();

        // Knowledge graph (if present)
        if let Some(kg) = data.get("knowledgeGraph") {
            if let Some(title) = kg.get("title").and_then(|v| v.as_str()) {
                output.push_str(&format!("## {}\n", title));
            }
            if let Some(desc) = kg.get("description").and_then(|v| v.as_str()) {
                output.push_str(&format!("{}\n\n", desc));
            }
        }

        // Answer box (if present)
        if let Some(answer) = data.get("answerBox") {
            if let Some(snippet) = answer.get("snippet").and_then(|v| v.as_str()) {
                output.push_str(&format!("Answer: {}\n\n", snippet));
            } else if let Some(answer_text) = answer.get("answer").and_then(|v| v.as_str()) {
                output.push_str(&format!("Answer: {}\n\n", answer_text));
            }
        }

        // Organic results
        if let Some(organic) = data.get("organic").and_then(|v| v.as_array()) {
            if !organic.is_empty() {
                output.push_str(&format!("Search results for '{}' ({} results):\n", query, organic.len()));
                for (i, item) in organic.iter().enumerate() {
                    let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    let link = item.get("link").and_then(|v| v.as_str()).unwrap_or("");
                    let snippet = item.get("snippet").and_then(|v| v.as_str()).unwrap_or("");

                    output.push_str(&format!("\n{}. {}\n", i + 1, title));
                    if !link.is_empty() {
                        output.push_str(&format!("   {}\n", link));
                    }
                    if !snippet.is_empty() {
                        output.push_str(&format!("   {}\n", snippet));
                    }
                }
            }
        }

        // People also ask
        if let Some(paa) = data.get("peopleAlsoAsk").and_then(|v| v.as_array()) {
            if !paa.is_empty() {
                output.push_str("\nRelated questions:\n");
                for item in paa.iter().take(3) {
                    if let Some(q) = item.get("question").and_then(|v| v.as_str()) {
                        output.push_str(&format!("  - {}\n", q));
                    }
                }
            }
        }

        if output.is_empty() {
            return Ok(ToolResult {
                success: true,
                output: format!("No results found for '{}'.", query),
            });
        }

        Ok(ToolResult {
            success: true,
            output,
        })
    }

    /// Tavily API — AI-focused search with extracted content
    async fn search_tavily(&self, query: &str) -> Result<ToolResult> {
        debug!("Trying Tavily API for: '{}'", query);

        let resp = self.client
            .post("https://api.tavily.com/search")
            .header("Content-Type", "application/json")
            .json(&json!({
                "api_key": self.config.tavily_api_key,
                "query": query,
                "max_results": 8,
                "include_answer": true,
                "search_depth": "basic"
            }))
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Ok(ToolResult {
                success: false,
                output: format!("Tavily API error (HTTP {}): {}", status.as_u16(), body),
            });
        }

        let data: Value = resp.json().await?;
        let mut output = String::new();

        // Tavily's AI answer (if present)
        if let Some(answer) = data.get("answer").and_then(|v| v.as_str()) {
            if !answer.is_empty() {
                output.push_str(&format!("Summary: {}\n\n", answer));
            }
        }

        // Search results
        if let Some(results) = data.get("results").and_then(|v| v.as_array()) {
            if !results.is_empty() {
                output.push_str(&format!("Search results for '{}' ({} results):\n", query, results.len()));
                for (i, item) in results.iter().enumerate() {
                    let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    let content = item.get("content").and_then(|v| v.as_str()).unwrap_or("");

                    output.push_str(&format!("\n{}. {}\n", i + 1, title));
                    if !url.is_empty() {
                        output.push_str(&format!("   {}\n", url));
                    }
                    if !content.is_empty() {
                        // Truncate long content
                        let snippet = if content.len() > 300 {
                            format!("{}...", &content[..content.char_indices().nth(300).map(|(i,_)|i).unwrap_or(content.len())])
                        } else {
                            content.to_string()
                        };
                        output.push_str(&format!("   {}\n", snippet));
                    }
                }
            }
        }

        if output.is_empty() {
            return Ok(ToolResult {
                success: true,
                output: format!("No results found for '{}'.", query),
            });
        }

        Ok(ToolResult {
            success: true,
            output,
        })
    }

    /// Search using Google News RSS feed (reliable, no API key needed)
    async fn search_news(&self, query: &str) -> Result<ToolResult> {
        info!("News search: '{}'", query);

        let url = format!(
            "https://news.google.com/rss/search?q={}&hl=zh-TW&gl=TW&ceid=TW:zh-Hant",
            urlencoding::encode(query)
        );

        let resp = match self.client
            .get(&url)
            .header("Accept", "application/rss+xml, application/xml, text/xml")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("News search request failed: {}", e);
                return Ok(ToolResult {
                    success: false,
                    output: format!("News search failed: {}", e),
                });
            }
        };

        let xml = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Failed to read response: {}", e),
                });
            }
        };

        let results = parse_rss_items(&xml, 8);

        if results.is_empty() {
            return Ok(ToolResult {
                success: true,
                output: format!("No news results found for '{}'.", query),
            });
        }

        let mut output = format!("News results for '{}' ({} articles):\n", query, results.len());
        for (i, item) in results.iter().enumerate() {
            output.push_str(&format!("\n{}. {}", i + 1, item.title));
            if !item.source.is_empty() {
                output.push_str(&format!(" [{}]", item.source));
            }
            if !item.pub_date.is_empty() {
                output.push_str(&format!("\n   {}", item.pub_date));
            }
            output.push('\n');
        }

        Ok(ToolResult {
            success: true,
            output,
        })
    }

    /// Fetch and extract text from a URL
    async fn fetch_url(&self, url: &str) -> Result<ToolResult> {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Ok(ToolResult {
                success: false,
                output: "Error: URL must start with http:// or https://".to_string(),
            });
        }

        info!("Fetching URL: {}", url);

        let resp = match self.client.get(url).send().await {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Failed to fetch URL: {}", e),
                });
            }
        };

        let status = resp.status();
        if !status.is_success() {
            return Ok(ToolResult {
                success: false,
                output: format!("HTTP {}: {}", status.as_u16(), url),
            });
        }

        let html = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Failed to read response: {}", e),
                });
            }
        };

        // Extract text content from HTML
        let text = extract_text_from_html(&html);

        // Truncate if too long
        let max_chars = 3000;
        let output = if text.chars().count() > max_chars {
            let end = text.char_indices().nth(max_chars).map(|(i, _)| i).unwrap_or(text.len());
            format!("{}...\n\n(truncated, {} chars total)", &text[..end], text.chars().count())
        } else {
            text
        };

        Ok(ToolResult {
            success: true,
            output,
        })
    }
}

struct RssItem {
    title: String,
    source: String,
    pub_date: String,
}

/// Parse RSS XML and extract items
fn parse_rss_items(xml: &str, max: usize) -> Vec<RssItem> {
    let mut results = Vec::new();

    for item_chunk in xml.split("<item>").skip(1) {
        if results.len() >= max { break; }

        let title = extract_xml_tag(item_chunk, "title");
        let pub_date = extract_xml_tag(item_chunk, "pubDate");
        let source_tag = extract_xml_tag(item_chunk, "source");

        if title.is_empty() { continue; }

        results.push(RssItem {
            title: xml_decode(&title),
            source: xml_decode(&source_tag),
            pub_date,
        });
    }

    results
}

/// Extract content between XML tags (simple, no nested support)
fn extract_xml_tag(xml: &str, tag: &str) -> String {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);

    if let Some(start_pos) = xml.find(&open) {
        let after_open = &xml[start_pos + open.len()..];
        // Skip past the > (handles attributes)
        if let Some(gt) = after_open.find('>') {
            let content = &after_open[gt + 1..];
            if let Some(end) = content.find(&close) {
                let raw = &content[..end];
                // Handle CDATA
                if raw.starts_with("<![CDATA[") && raw.ends_with("]]>") {
                    return raw[9..raw.len()-3].to_string();
                }
                return raw.to_string();
            }
        }
    }
    String::new()
}

/// Decode XML entities
fn xml_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Simple HTML to text extractor
fn extract_text_from_html(html: &str) -> String {
    // Remove script and style blocks
    let mut clean = html.to_string();

    // Remove <script>...</script> and <style>...</style>
    for tag in &["script", "style", "noscript"] {
        loop {
            let open = format!("<{}", tag);
            let close = format!("</{}>", tag);
            if let Some(start) = clean.to_lowercase().find(&open) {
                if let Some(end_pos) = clean.to_lowercase()[start..].find(&close) {
                    let end = start + end_pos + close.len();
                    clean.replace_range(start..end, " ");
                    continue;
                }
            }
            break;
        }
    }

    // Strip all remaining HTML tags
    let mut result = String::with_capacity(clean.len());
    let mut in_tag = false;
    for c in clean.chars() {
        match c {
            '<' => in_tag = true,
            '>' => { in_tag = false; result.push(' '); }
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }

    // Decode HTML entities
    let result = result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ");

    // Collapse whitespace
    let mut collapsed = String::with_capacity(result.len());
    let mut prev_newline = false;
    let mut prev_space = false;
    for c in result.chars() {
        match c {
            '\n' | '\r' => {
                if !prev_newline {
                    collapsed.push('\n');
                    prev_newline = true;
                    prev_space = false;
                }
            }
            ' ' | '\t' => {
                if !prev_space && !prev_newline {
                    collapsed.push(' ');
                    prev_space = true;
                }
            }
            _ => {
                collapsed.push(c);
                prev_newline = false;
                prev_space = false;
            }
        }
    }

    collapsed.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_text_basic() {
        let html = "<html><body><h1>Hello</h1><p>World</p></body></html>";
        let text = extract_text_from_html(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
    }

    #[test]
    fn test_extract_text_strips_script() {
        let html = "<p>Before</p><script>alert('x')</script><p>After</p>";
        let text = extract_text_from_html(html);
        assert!(text.contains("Before"));
        assert!(text.contains("After"));
        assert!(!text.contains("alert"));
    }

    #[test]
    fn test_extract_text_decodes_entities() {
        let html = "<p>A &amp; B &lt;C&gt;</p>";
        let text = extract_text_from_html(html);
        assert!(text.contains("A & B <C>"));
    }

    #[test]
    fn test_search_config_default() {
        let config = SearchConfig::default();
        assert!(config.serper_api_key.is_empty());
        assert!(config.tavily_api_key.is_empty());
    }
}
