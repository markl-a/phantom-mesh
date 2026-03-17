//! RSS/Atom feed reader tool — fetch, list, and search feed entries.
//! Uses simple string parsing to handle RSS 2.0 and Atom XML feeds without heavy XML crates.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolResult};

pub struct RssReaderTool;

impl RssReaderTool {
    pub fn new() -> Self {
        Self
    }

    /// Extract all occurrences of a tag's text content from XML.
    /// Returns a Vec of the inner text for each `<tag>...</tag>` found.
    fn extract_tags(xml: &str, tag: &str) -> Vec<String> {
        let open = format!("<{}", tag);
        let close = format!("</{}>", tag);
        let mut results = Vec::new();
        let mut search_from = 0;

        while let Some(start_tag_pos) = xml[search_from..].find(&open) {
            let abs_start = search_from + start_tag_pos;
            // Find the end of the opening tag (handle attributes)
            if let Some(tag_end) = xml[abs_start..].find('>') {
                let content_start = abs_start + tag_end + 1;
                // Check for self-closing tag
                if xml[abs_start..content_start].ends_with("/>") {
                    search_from = content_start;
                    continue;
                }
                if let Some(close_pos) = xml[content_start..].find(&close) {
                    let content = &xml[content_start..content_start + close_pos];
                    // Strip CDATA wrapper if present
                    let cleaned = strip_cdata(content.trim());
                    // Strip any remaining HTML tags for plain text
                    results.push(strip_html_tags(&cleaned));
                    search_from = content_start + close_pos + close.len();
                } else {
                    search_from = content_start;
                }
            } else {
                break;
            }
        }
        results
    }

    /// Extract the first occurrence of a tag's text content.
    fn extract_first_tag(xml: &str, tag: &str) -> Option<String> {
        Self::extract_tags(xml, tag).into_iter().next()
    }

    /// Parse feed entries from XML (supports both RSS and Atom).
    fn parse_entries(xml: &str) -> Vec<FeedEntry> {
        let is_atom = xml.contains("<feed") && xml.contains("xmlns=\"http://www.w3.org/2005/Atom\"")
            || xml.contains("<entry>");

        if is_atom {
            Self::parse_atom_entries(xml)
        } else {
            Self::parse_rss_entries(xml)
        }
    }

    /// Parse RSS 2.0 <item> elements.
    fn parse_rss_entries(xml: &str) -> Vec<FeedEntry> {
        let mut entries = Vec::new();
        let mut search_from = 0;

        while let Some(item_start) = xml[search_from..].find("<item") {
            let abs_start = search_from + item_start;
            if let Some(item_end) = xml[abs_start..].find("</item>") {
                let item_xml = &xml[abs_start..abs_start + item_end + 7];
                entries.push(FeedEntry {
                    title: Self::extract_first_tag(item_xml, "title").unwrap_or_default(),
                    link: Self::extract_first_tag(item_xml, "link").unwrap_or_default(),
                    description: Self::extract_first_tag(item_xml, "description").unwrap_or_default(),
                    published: Self::extract_first_tag(item_xml, "pubDate")
                        .or_else(|| Self::extract_first_tag(item_xml, "dc:date"))
                        .unwrap_or_default(),
                    author: Self::extract_first_tag(item_xml, "author")
                        .or_else(|| Self::extract_first_tag(item_xml, "dc:creator"))
                        .unwrap_or_default(),
                });
                search_from = abs_start + item_end + 7;
            } else {
                break;
            }
        }
        entries
    }

    /// Parse Atom <entry> elements.
    fn parse_atom_entries(xml: &str) -> Vec<FeedEntry> {
        let mut entries = Vec::new();
        let mut search_from = 0;

        while let Some(entry_start) = xml[search_from..].find("<entry") {
            let abs_start = search_from + entry_start;
            if let Some(entry_end) = xml[abs_start..].find("</entry>") {
                let entry_xml = &xml[abs_start..abs_start + entry_end + 8];
                // Atom <link> is self-closing with href attribute
                let link = extract_atom_link(entry_xml);
                entries.push(FeedEntry {
                    title: Self::extract_first_tag(entry_xml, "title").unwrap_or_default(),
                    link,
                    description: Self::extract_first_tag(entry_xml, "summary")
                        .or_else(|| Self::extract_first_tag(entry_xml, "content"))
                        .unwrap_or_default(),
                    published: Self::extract_first_tag(entry_xml, "published")
                        .or_else(|| Self::extract_first_tag(entry_xml, "updated"))
                        .unwrap_or_default(),
                    author: Self::extract_first_tag(entry_xml, "name").unwrap_or_default(),
                });
                search_from = abs_start + entry_end + 8;
            } else {
                break;
            }
        }
        entries
    }

    /// Get feed title and description.
    fn parse_feed_meta(xml: &str) -> (String, String) {
        // For RSS: title is in <channel><title>
        // For Atom: title is a direct child of <feed>
        let title = Self::extract_first_tag(xml, "title").unwrap_or_default();
        let description = Self::extract_first_tag(xml, "description")
            .or_else(|| Self::extract_first_tag(xml, "subtitle"))
            .unwrap_or_default();
        (title, description)
    }
}

/// A parsed feed entry.
#[derive(Debug, Clone)]
struct FeedEntry {
    title: String,
    link: String,
    description: String,
    published: String,
    author: String,
}

impl FeedEntry {
    fn to_json(&self) -> Value {
        json!({
            "title": self.title,
            "link": self.link,
            "description": truncate_str(&self.description, 500),
            "published": self.published,
            "author": self.author,
        })
    }

    /// Check if entry matches a search query (case-insensitive).
    fn matches(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        self.title.to_lowercase().contains(&q)
            || self.description.to_lowercase().contains(&q)
            || self.author.to_lowercase().contains(&q)
    }
}

/// Extract href from Atom <link> element (self-closing with attributes).
fn extract_atom_link(xml: &str) -> String {
    // Look for <link ... href="..." ... />
    // Prefer rel="alternate" or no rel attribute
    let mut best_link = String::new();
    let mut search_from = 0;

    while let Some(pos) = xml[search_from..].find("<link") {
        let abs_pos = search_from + pos;
        if let Some(end) = xml[abs_pos..].find('>') {
            let tag = &xml[abs_pos..abs_pos + end + 1];
            if let Some(href) = extract_attribute(tag, "href") {
                let rel = extract_attribute(tag, "rel").unwrap_or_default();
                if rel.is_empty() || rel == "alternate" {
                    return href;
                }
                if best_link.is_empty() {
                    best_link = href;
                }
            }
            search_from = abs_pos + end + 1;
        } else {
            break;
        }
    }
    best_link
}

/// Extract an attribute value from an XML tag string.
fn extract_attribute(tag: &str, attr: &str) -> Option<String> {
    let patterns = [
        format!("{}=\"", attr),
        format!("{}='", attr),
    ];

    for pattern in &patterns {
        if let Some(start) = tag.find(pattern.as_str()) {
            let value_start = start + pattern.len();
            let quote = if pattern.ends_with('"') { '"' } else { '\'' };
            if let Some(end) = tag[value_start..].find(quote) {
                return Some(tag[value_start..value_start + end].to_string());
            }
        }
    }
    None
}

/// Strip CDATA wrappers from content.
fn strip_cdata(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.starts_with("<![CDATA[") && trimmed.ends_with("]]>") {
        trimmed[9..trimmed.len() - 3].to_string()
    } else {
        trimmed.to_string()
    }
}

/// Strip HTML tags from a string (simple regex-free approach).
fn strip_html_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    // Decode common HTML entities
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

/// Truncate a string at a safe char boundary.
fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
        format!("{}...", &s[..end])
    }
}

#[async_trait]
impl Tool for RssReaderTool {
    fn name(&self) -> &str {
        "rss_reader"
    }

    fn description(&self) -> &str {
        "Read and parse RSS/Atom feeds. Operations: fetch (get full feed), list_entries (list entry titles and links), search (find entries matching a query)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "One of: fetch, list_entries, search",
                    "enum": ["fetch", "list_entries", "search"]
                },
                "url": {
                    "type": "string",
                    "description": "URL of the RSS/Atom feed"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (for 'search' operation)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of entries to return (default: 20)"
                }
            },
            "required": ["operation", "url"]
        })
    }

    fn preflight(&self, args: &Value) -> Result<()> {
        let operation = args.get("operation").and_then(|v| v.as_str()).unwrap_or("");
        if operation.is_empty() {
            anyhow::bail!("Preflight: 'operation' is required");
        }
        if !["fetch", "list_entries", "search"].contains(&operation) {
            anyhow::bail!("Preflight: unknown operation '{}'. Use: fetch, list_entries, search", operation);
        }

        let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if url.is_empty() {
            anyhow::bail!("Preflight: 'url' is required");
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            anyhow::bail!("Preflight: url must start with http:// or https://");
        }

        if operation == "search" {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            if query.is_empty() {
                anyhow::bail!("Preflight: 'query' is required for search operation");
            }
        }

        Ok(())
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let operation = args["operation"].as_str().unwrap_or("").trim();
        let url = args["url"].as_str().unwrap_or("").trim();
        let limit = args["limit"].as_u64().unwrap_or(20) as usize;

        if operation.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing required parameter: operation".into() });
        }
        if url.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing required parameter: url".into() });
        }

        // Validate operation early, before making any network requests
        if !["fetch", "list_entries", "search"].contains(&operation) {
            return Ok(ToolResult {
                success: false,
                output: format!("Unknown operation: '{}'. Use: fetch, list_entries, search", operation),
            });
        }

        // Fetch the feed XML
        let xml = match fetch_feed(url).await {
            Ok(xml) => xml,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Failed to fetch feed from {}: {}", url, e),
                });
            }
        };

        let entries = Self::parse_entries(&xml);
        let (feed_title, feed_description) = Self::parse_feed_meta(&xml);

        match operation {
            "fetch" => {
                let entry_jsons: Vec<Value> = entries.iter().take(limit).map(|e| e.to_json()).collect();
                let result = json!({
                    "feed_title": feed_title,
                    "feed_description": feed_description,
                    "entry_count": entries.len(),
                    "entries": entry_jsons,
                });
                Ok(ToolResult {
                    success: true,
                    output: serde_json::to_string_pretty(&result)?,
                })
            }
            "list_entries" => {
                let items: Vec<Value> = entries.iter().take(limit).map(|e| {
                    json!({
                        "title": e.title,
                        "link": e.link,
                        "published": e.published,
                    })
                }).collect();
                let result = json!({
                    "feed_title": feed_title,
                    "total": entries.len(),
                    "showing": items.len(),
                    "entries": items,
                });
                Ok(ToolResult {
                    success: true,
                    output: serde_json::to_string_pretty(&result)?,
                })
            }
            "search" => {
                let query = args["query"].as_str().unwrap_or("").trim();
                if query.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: "Missing required parameter: query (for search operation)".into(),
                    });
                }
                let matched: Vec<Value> = entries.iter()
                    .filter(|e| e.matches(query))
                    .take(limit)
                    .map(|e| e.to_json())
                    .collect();
                let result = json!({
                    "feed_title": feed_title,
                    "query": query,
                    "matched_count": matched.len(),
                    "total_entries": entries.len(),
                    "entries": matched,
                });
                Ok(ToolResult {
                    success: true,
                    output: serde_json::to_string_pretty(&result)?,
                })
            }
            _ => Ok(ToolResult {
                success: false,
                output: format!("Unknown operation: '{}'. Use: fetch, list_entries, search", operation),
            }),
        }
    }
}

/// Fetch feed content from a URL using reqwest.
async fn fetch_feed(url: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("clawtex-core/0.1 RSS Reader")
        .build()?;

    let resp = client.get(url).send().await?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {} from {}", status, url);
    }
    let body = resp.text().await?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSS_SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Test Feed</title>
    <description>A test RSS feed</description>
    <link>https://example.com</link>
    <item>
      <title>First Post</title>
      <link>https://example.com/1</link>
      <description>This is the first post content</description>
      <pubDate>Mon, 01 Jan 2026 00:00:00 GMT</pubDate>
      <author>alice@example.com</author>
    </item>
    <item>
      <title>Second Post about Rust</title>
      <link>https://example.com/2</link>
      <description>Rust programming language news</description>
      <pubDate>Tue, 02 Jan 2026 00:00:00 GMT</pubDate>
      <author>bob@example.com</author>
    </item>
    <item>
      <title>Third Post</title>
      <link>https://example.com/3</link>
      <description><![CDATA[<p>HTML content in CDATA</p>]]></description>
      <pubDate>Wed, 03 Jan 2026 00:00:00 GMT</pubDate>
    </item>
  </channel>
</rss>"#;

    const ATOM_SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Atom Test Feed</title>
  <subtitle>An Atom test feed</subtitle>
  <link href="https://example.com" rel="alternate"/>
  <entry>
    <title>Atom Entry One</title>
    <link href="https://example.com/atom/1" rel="alternate"/>
    <summary>Summary of entry one</summary>
    <published>2026-01-01T00:00:00Z</published>
    <author><name>Charlie</name></author>
  </entry>
  <entry>
    <title>Atom Entry Two</title>
    <link href="https://example.com/atom/2" rel="alternate"/>
    <summary>Summary about Rust development</summary>
    <updated>2026-01-02T00:00:00Z</updated>
    <author><name>Diana</name></author>
  </entry>
</feed>"#;

    #[test]
    fn test_name() {
        let tool = RssReaderTool::new();
        assert_eq!(tool.name(), "rss_reader");
    }

    #[test]
    fn test_schema() {
        let tool = RssReaderTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["operation"].is_object());
        assert!(schema["properties"]["url"].is_object());
        assert!(schema["properties"]["query"].is_object());
        assert!(schema["properties"]["limit"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("operation")));
        assert!(required.contains(&json!("url")));
    }

    #[test]
    fn test_parse_rss_entries() {
        let entries = RssReaderTool::parse_rss_entries(RSS_SAMPLE);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].title, "First Post");
        assert_eq!(entries[0].link, "https://example.com/1");
        assert_eq!(entries[1].title, "Second Post about Rust");
        assert!(entries[0].published.contains("Mon"));
    }

    #[test]
    fn test_parse_atom_entries() {
        let entries = RssReaderTool::parse_atom_entries(ATOM_SAMPLE);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "Atom Entry One");
        assert_eq!(entries[0].link, "https://example.com/atom/1");
        assert_eq!(entries[1].author, "Diana");
    }

    #[test]
    fn test_parse_entries_auto_detect_rss() {
        let entries = RssReaderTool::parse_entries(RSS_SAMPLE);
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_parse_entries_auto_detect_atom() {
        let entries = RssReaderTool::parse_entries(ATOM_SAMPLE);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_feed_meta_rss() {
        let (title, desc) = RssReaderTool::parse_feed_meta(RSS_SAMPLE);
        assert_eq!(title, "Test Feed");
        assert_eq!(desc, "A test RSS feed");
    }

    #[test]
    fn test_feed_meta_atom() {
        let (title, desc) = RssReaderTool::parse_feed_meta(ATOM_SAMPLE);
        assert_eq!(title, "Atom Test Feed");
        assert_eq!(desc, "An Atom test feed");
    }

    #[test]
    fn test_search_matches() {
        let entry = FeedEntry {
            title: "Rust Programming Tips".into(),
            link: "https://example.com/rust".into(),
            description: "Learn Rust".into(),
            published: "2026-01-01".into(),
            author: "Alice".into(),
        };
        assert!(entry.matches("rust"));
        assert!(entry.matches("Rust"));
        assert!(entry.matches("alice"));
        assert!(!entry.matches("python"));
    }

    #[test]
    fn test_strip_cdata() {
        assert_eq!(strip_cdata("<![CDATA[Hello World]]>"), "Hello World");
        assert_eq!(strip_cdata("No CDATA here"), "No CDATA here");
        assert_eq!(strip_cdata("  <![CDATA[Trimmed]]>  "), "Trimmed");
    }

    #[test]
    fn test_strip_html_tags() {
        assert_eq!(strip_html_tags("<p>Hello</p>"), "Hello");
        assert_eq!(strip_html_tags("<b>Bold</b> and <i>italic</i>"), "Bold and italic");
        assert_eq!(strip_html_tags("No tags here"), "No tags here");
        assert_eq!(strip_html_tags("&amp; &lt; &gt;"), "& < >");
    }

    #[test]
    fn test_extract_attribute() {
        assert_eq!(
            extract_attribute(r#"<link href="https://example.com" rel="alternate"/>"#, "href"),
            Some("https://example.com".to_string())
        );
        assert_eq!(
            extract_attribute(r#"<link href='https://example.com'/>"#, "href"),
            Some("https://example.com".to_string())
        );
        assert_eq!(
            extract_attribute(r#"<link rel="self"/>"#, "href"),
            None
        );
    }

    #[test]
    fn test_extract_tags() {
        let xml = "<root><title>A</title><title>B</title></root>";
        let tags = RssReaderTool::extract_tags(xml, "title");
        assert_eq!(tags, vec!["A", "B"]);
    }

    #[test]
    fn test_preflight_missing_operation() {
        let tool = RssReaderTool::new();
        let result = tool.preflight(&json!({"url": "https://example.com/feed.xml"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("operation"));
    }

    #[test]
    fn test_preflight_invalid_operation() {
        let tool = RssReaderTool::new();
        let result = tool.preflight(&json!({"operation": "delete", "url": "https://x.com/feed"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown operation"));
    }

    #[test]
    fn test_preflight_missing_url() {
        let tool = RssReaderTool::new();
        let result = tool.preflight(&json!({"operation": "fetch"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("url"));
    }

    #[test]
    fn test_preflight_bad_url() {
        let tool = RssReaderTool::new();
        let result = tool.preflight(&json!({"operation": "fetch", "url": "ftp://bad.com/feed"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("http"));
    }

    #[test]
    fn test_preflight_search_missing_query() {
        let tool = RssReaderTool::new();
        let result = tool.preflight(&json!({"operation": "search", "url": "https://x.com/feed"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("query"));
    }

    #[test]
    fn test_preflight_valid() {
        let tool = RssReaderTool::new();
        assert!(tool.preflight(&json!({"operation": "fetch", "url": "https://example.com/rss"})).is_ok());
        assert!(tool.preflight(&json!({"operation": "list_entries", "url": "https://example.com/rss"})).is_ok());
        assert!(tool.preflight(&json!({"operation": "search", "url": "https://example.com/rss", "query": "rust"})).is_ok());
    }

    #[tokio::test]
    async fn test_execute_missing_operation() {
        let tool = RssReaderTool::new();
        let result = tool.execute(json!({"url": "https://example.com/rss"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_execute_missing_url() {
        let tool = RssReaderTool::new();
        let result = tool.execute(json!({"operation": "fetch"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_execute_unknown_operation() {
        let tool = RssReaderTool::new();
        let result = tool.execute(json!({"operation": "delete", "url": "https://example.com/rss"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown operation"));
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world foo", 5), "hello...");
    }

    #[test]
    fn test_entry_to_json() {
        let entry = FeedEntry {
            title: "Test".into(),
            link: "https://example.com".into(),
            description: "Desc".into(),
            published: "2026-01-01".into(),
            author: "Author".into(),
        };
        let j = entry.to_json();
        assert_eq!(j["title"], "Test");
        assert_eq!(j["link"], "https://example.com");
    }

    #[test]
    fn test_cdata_in_rss() {
        let entries = RssReaderTool::parse_rss_entries(RSS_SAMPLE);
        // Third entry has CDATA-wrapped HTML content
        assert_eq!(entries[2].description, "HTML content in CDATA");
    }

    #[test]
    fn test_empty_xml() {
        let entries = RssReaderTool::parse_entries("");
        assert!(entries.is_empty());
    }
}
