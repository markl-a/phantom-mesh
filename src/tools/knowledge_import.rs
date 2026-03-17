//! knowledge_import tool — imports knowledge from files, directories, or URLs into the memory system.
//! Actions: "import_file" (single file), "import_directory" (batch), "import_url" (web page).
//! Supports .txt, .md, .pdf, .csv, .json. Chunks text with configurable overlap.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{debug, warn};

use super::{Tool, ToolResult};

pub struct KnowledgeImportTool;

impl KnowledgeImportTool {
    pub fn new() -> Self {
        Self
    }

    /// Get the default database path for knowledge storage.
    fn default_db_path() -> std::path::PathBuf {
        dirs::home_dir()
            .unwrap_or_default()
            .join(".clawtex")
            .join("knowledge_import.db")
    }

    /// Initialize the SQLite database for knowledge chunks.
    fn init_db(db_path: &std::path::Path) -> Result<()> {
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = rusqlite::Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS knowledge_chunks (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                category TEXT NOT NULL DEFAULT 'imported',
                chunk_index INTEGER NOT NULL,
                content TEXT NOT NULL,
                char_count INTEGER NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_knowledge_source ON knowledge_chunks(source);
            CREATE INDEX IF NOT EXISTS idx_knowledge_category ON knowledge_chunks(category);"
        )?;
        Ok(())
    }

    /// Store a chunk into the knowledge database.
    fn store_chunk(
        db_path: &std::path::Path,
        source: &str,
        category: &str,
        chunk_index: usize,
        content: &str,
    ) -> Result<String> {
        let conn = rusqlite::Connection::open(db_path)?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO knowledge_chunks (id, source, category, chunk_index, content, char_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![id, source, category, chunk_index as i64, content, content.len() as i64, now],
        )?;
        Ok(id)
    }

    /// Chunk text into overlapping segments by paragraph boundaries or fixed size.
    fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<String> {
        if text.is_empty() {
            return vec![];
        }

        // If text is smaller than chunk_size, return as single chunk
        if text.len() <= chunk_size {
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                return vec![];
            }
            return vec![trimmed];
        }

        let mut chunks = Vec::new();
        let mut start = 0;
        let text_len = text.len();

        // Ensure start is always on a char boundary
        start = find_char_boundary(text, start);

        while start < text_len {
            // Calculate end position (respecting char boundaries)
            let raw_end = std::cmp::min(start + chunk_size, text_len);
            let end = find_char_boundary(text, raw_end);

            // Safety check: if start == end, advance to the next char boundary
            if start >= end {
                // Move to next char boundary
                let mut next = start + 1;
                while next < text_len && !text.is_char_boundary(next) {
                    next += 1;
                }
                start = next;
                continue;
            }

            // Try to break at paragraph boundary within the chunk
            let chunk_slice = &text[start..end];
            let min_break = chunk_size / 4;
            let actual_end = if let Some(para_break) = chunk_slice.rfind("\n\n") {
                // Only break at paragraph if it's not too close to the start
                if para_break > min_break {
                    let candidate = start + para_break + 2;
                    find_char_boundary(text, std::cmp::min(candidate, text_len))
                } else {
                    end
                }
            } else if let Some(line_break) = chunk_slice.rfind('\n') {
                if line_break > min_break {
                    let candidate = start + line_break + 1;
                    find_char_boundary(text, std::cmp::min(candidate, text_len))
                } else {
                    end
                }
            } else {
                end
            };

            let chunk = text[start..actual_end].trim().to_string();
            if !chunk.is_empty() {
                chunks.push(chunk);
            }

            // Move start forward, accounting for overlap
            if actual_end >= text_len {
                break;
            }
            let advance = if actual_end - start > overlap {
                actual_end - start - overlap
            } else {
                actual_end - start
            };
            // Ensure new start is on a char boundary
            let new_start = start + advance;
            start = find_char_boundary(text, new_start);
            // If we didn't actually advance, force move forward
            if start <= new_start.saturating_sub(advance) {
                start = actual_end;
            }
        }

        chunks
    }

    /// Detect the file type from extension.
    fn detect_file_type(path: &str) -> &str {
        let lower = path.to_lowercase();
        if lower.ends_with(".txt") {
            "txt"
        } else if lower.ends_with(".md") || lower.ends_with(".markdown") {
            "md"
        } else if lower.ends_with(".pdf") {
            "pdf"
        } else if lower.ends_with(".csv") {
            "csv"
        } else if lower.ends_with(".json") {
            "json"
        } else {
            "unknown"
        }
    }

    /// Read file content based on its type.
    fn read_file_content(path: &str) -> Result<String> {
        let file_type = Self::detect_file_type(path);
        let expanded = super::expand_home(path);

        match file_type {
            "txt" | "md" => {
                Ok(std::fs::read_to_string(&expanded)?)
            }
            "csv" => {
                // Read CSV and convert each row to a readable line
                let content = std::fs::read_to_string(&expanded)?;
                let mut reader = csv::ReaderBuilder::new()
                    .has_headers(true)
                    .from_reader(content.as_bytes());

                let headers: Vec<String> = reader.headers()
                    .map(|h| h.iter().map(|s| s.to_string()).collect())
                    .unwrap_or_default();

                let mut lines = Vec::new();
                if !headers.is_empty() {
                    lines.push(format!("Headers: {}", headers.join(", ")));
                }

                for record in reader.records().flatten() {
                    let row: Vec<String> = headers.iter().zip(record.iter())
                        .map(|(h, v)| format!("{}: {}", h, v))
                        .collect();
                    lines.push(row.join(", "));
                }

                Ok(lines.join("\n"))
            }
            "json" => {
                let content = std::fs::read_to_string(&expanded)?;
                // Pretty-print JSON for better chunking
                let parsed: Value = serde_json::from_str(&content)?;
                Ok(serde_json::to_string_pretty(&parsed)?)
            }
            "pdf" => {
                // PDF extraction via python subprocess (pdfplumber or PyPDF2)
                // We return an error suggesting user convert first, since we can't
                // bundle a PDF library without adding a heavy dependency
                anyhow::bail!(
                    "PDF import requires Python: pip install pdfplumber. \
                     Alternatively, convert to .txt first. \
                     Path: {}", expanded
                )
            }
            _ => {
                // Try reading as plain text
                Ok(std::fs::read_to_string(&expanded)?)
            }
        }
    }

    /// Strip HTML tags from content (simple regex-based approach).
    fn strip_html_tags(html: &str) -> String {
        // Remove script and style blocks
        let re_script = regex::Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap_or_else(|_| regex::Regex::new("$^").unwrap());
        let no_script = re_script.replace_all(html, "");
        let re_style = regex::Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap_or_else(|_| regex::Regex::new("$^").unwrap());
        let no_style = re_style.replace_all(&no_script, "");

        // Remove all HTML tags
        let re_tags = regex::Regex::new(r"<[^>]+>").unwrap_or_else(|_| regex::Regex::new("$^").unwrap());
        let text = re_tags.replace_all(&no_style, "");

        // Decode common HTML entities
        let text = text
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&#39;", "'")
            .replace("&nbsp;", " ");

        // Collapse multiple whitespace/newlines
        let re_ws = regex::Regex::new(r"\n{3,}").unwrap_or_else(|_| regex::Regex::new("$^").unwrap());
        let text = re_ws.replace_all(&text, "\n\n");

        text.trim().to_string()
    }

    /// Execute import_file action.
    async fn execute_import_file(
        &self,
        source: &str,
        category: &str,
        chunk_size: usize,
        overlap: usize,
    ) -> Result<ToolResult> {
        let content = match Self::read_file_content(source) {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Failed to read file '{}': {}", source, e),
                });
            }
        };

        if content.trim().is_empty() {
            return Ok(ToolResult {
                success: false,
                output: format!("File '{}' is empty or contains only whitespace", source),
            });
        }

        let chunks = Self::chunk_text(&content, chunk_size, overlap);
        let db_path = Self::default_db_path();
        Self::init_db(&db_path)?;

        let mut stored = 0;
        for (i, chunk) in chunks.iter().enumerate() {
            match Self::store_chunk(&db_path, source, category, i, chunk) {
                Ok(_) => stored += 1,
                Err(e) => {
                    warn!("Failed to store chunk {} from {}: {}", i, source, e);
                }
            }
        }

        debug!("Imported {} chunks from file '{}'", stored, source);

        Ok(ToolResult {
            success: true,
            output: format!(
                "Knowledge imported successfully!\nSource: {}\nFile type: {}\nChunks stored: {}\nCategory: {}\nChunk size: {} chars\nOverlap: {} chars\nDB: {}",
                source,
                Self::detect_file_type(source),
                stored,
                category,
                chunk_size,
                overlap,
                db_path.display()
            ),
        })
    }

    /// Execute import_directory action.
    async fn execute_import_directory(
        &self,
        source: &str,
        category: &str,
        chunk_size: usize,
        overlap: usize,
    ) -> Result<ToolResult> {
        let expanded = super::expand_home(source);
        let dir_path = std::path::Path::new(&expanded);

        if !dir_path.is_dir() {
            return Ok(ToolResult {
                success: false,
                output: format!("'{}' is not a directory", source),
            });
        }

        let supported_extensions = ["txt", "md", "markdown", "csv", "json"];
        let mut total_chunks = 0;
        let mut files_processed = 0;
        let mut errors = Vec::new();

        let entries: Vec<_> = std::fs::read_dir(dir_path)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| supported_extensions.contains(&ext.to_lowercase().as_str()))
                    .unwrap_or(false)
            })
            .collect();

        let db_path = Self::default_db_path();
        Self::init_db(&db_path)?;

        for entry in &entries {
            let file_path = entry.path();
            let file_str = file_path.to_string_lossy().to_string();

            match Self::read_file_content(&file_str) {
                Ok(content) => {
                    if content.trim().is_empty() {
                        continue;
                    }
                    let chunks = Self::chunk_text(&content, chunk_size, overlap);
                    for (i, chunk) in chunks.iter().enumerate() {
                        match Self::store_chunk(&db_path, &file_str, category, i, chunk) {
                            Ok(_) => total_chunks += 1,
                            Err(e) => {
                                errors.push(format!("{}: chunk {}: {}", file_str, i, e));
                            }
                        }
                    }
                    files_processed += 1;
                }
                Err(e) => {
                    errors.push(format!("{}: {}", file_str, e));
                }
            }
        }

        let mut output = format!(
            "Directory import complete!\nSource: {}\nFiles processed: {}/{}\nTotal chunks stored: {}\nCategory: {}",
            source, files_processed, entries.len(), total_chunks, category
        );

        if !errors.is_empty() {
            output.push_str(&format!("\nErrors ({}):\n{}", errors.len(), errors.join("\n")));
        }

        Ok(ToolResult {
            success: files_processed > 0,
            output,
        })
    }

    /// Execute import_url action.
    async fn execute_import_url(
        &self,
        source: &str,
        category: &str,
        chunk_size: usize,
        overlap: usize,
    ) -> Result<ToolResult> {
        debug!("Fetching URL for knowledge import: {}", source);

        let client = reqwest::Client::new();
        let resp = client.get(source)
            .header("User-Agent", "Clawtex-KnowledgeImport/1.0")
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await;

        let html = match resp {
            Ok(response) => {
                let status = response.status();
                if !status.is_success() {
                    return Ok(ToolResult {
                        success: false,
                        output: format!("HTTP error fetching '{}': status {}", source, status),
                    });
                }
                response.text().await.unwrap_or_default()
            }
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Failed to fetch URL '{}': {}", source, e),
                });
            }
        };

        if html.trim().is_empty() {
            return Ok(ToolResult {
                success: false,
                output: format!("URL '{}' returned empty content", source),
            });
        }

        let text = Self::strip_html_tags(&html);

        if text.trim().is_empty() {
            return Ok(ToolResult {
                success: false,
                output: format!("No text content extracted from URL '{}'", source),
            });
        }

        let chunks = Self::chunk_text(&text, chunk_size, overlap);
        let db_path = Self::default_db_path();
        Self::init_db(&db_path)?;

        let mut stored = 0;
        for (i, chunk) in chunks.iter().enumerate() {
            match Self::store_chunk(&db_path, source, category, i, chunk) {
                Ok(_) => stored += 1,
                Err(e) => {
                    warn!("Failed to store chunk {} from URL {}: {}", i, source, e);
                }
            }
        }

        debug!("Imported {} chunks from URL '{}'", stored, source);

        Ok(ToolResult {
            success: true,
            output: format!(
                "Knowledge imported from URL!\nSource: {}\nText extracted: {} chars\nChunks stored: {}\nCategory: {}\nDB: {}",
                source,
                text.len(),
                stored,
                category,
                db_path.display()
            ),
        })
    }
}

#[async_trait]
impl Tool for KnowledgeImportTool {
    fn name(&self) -> &str {
        "knowledge_import"
    }

    fn description(&self) -> &str {
        "Import knowledge from files, directories, or URLs into the memory system. Supports .txt, .md, .csv, .json. Chunks text with configurable overlap for retrieval."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Import action: 'import_file' (single file), 'import_directory' (batch), 'import_url' (web page)",
                    "enum": ["import_file", "import_directory", "import_url"],
                    "default": "import_file"
                },
                "source": {
                    "type": "string",
                    "description": "File path, directory path, or URL to import from"
                },
                "category": {
                    "type": "string",
                    "description": "Category label for the imported knowledge (default: 'imported')",
                    "default": "imported"
                },
                "chunk_size": {
                    "type": "integer",
                    "description": "Maximum chunk size in characters (default: 500)",
                    "default": 500
                },
                "overlap": {
                    "type": "integer",
                    "description": "Overlap between chunks in characters (default: 50)",
                    "default": 50
                }
            },
            "required": ["source"]
        })
    }

    fn preflight(&self, args: &Value) -> Result<()> {
        let source = args.get("source").and_then(|v| v.as_str()).unwrap_or("");
        if source.trim().is_empty() {
            anyhow::bail!("Preflight: 'source' cannot be empty");
        }

        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("import_file");
        match action {
            "import_file" => {
                let expanded = super::expand_home(source);
                if !std::path::Path::new(&expanded).exists() {
                    anyhow::bail!("Preflight: file '{}' does not exist", source);
                }
            }
            "import_directory" => {
                let expanded = super::expand_home(source);
                if !std::path::Path::new(&expanded).is_dir() {
                    anyhow::bail!("Preflight: '{}' is not a directory", source);
                }
            }
            "import_url" => {
                if !source.starts_with("http://") && !source.starts_with("https://") {
                    anyhow::bail!("Preflight: URL must start with http:// or https://");
                }
            }
            other => {
                anyhow::bail!("Preflight: unknown action '{}'. Use 'import_file', 'import_directory', or 'import_url'", other);
            }
        }

        let chunk_size = args.get("chunk_size").and_then(|v| v.as_u64()).unwrap_or(500);
        if chunk_size == 0 {
            anyhow::bail!("Preflight: chunk_size must be greater than 0");
        }
        if chunk_size > 100_000 {
            anyhow::bail!("Preflight: chunk_size cannot exceed 100000");
        }

        let overlap = args.get("overlap").and_then(|v| v.as_u64()).unwrap_or(50);
        if overlap >= chunk_size {
            anyhow::bail!("Preflight: overlap ({}) must be less than chunk_size ({})", overlap, chunk_size);
        }

        Ok(())
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let source = args.get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if source.trim().is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Error: 'source' is required and cannot be empty".to_string(),
            });
        }

        let action = args.get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("import_file");

        let category = args.get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("imported");

        let chunk_size = args.get("chunk_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(500) as usize;

        let overlap = args.get("overlap")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as usize;

        if chunk_size == 0 {
            return Ok(ToolResult {
                success: false,
                output: "Error: chunk_size must be greater than 0".to_string(),
            });
        }

        if overlap >= chunk_size {
            return Ok(ToolResult {
                success: false,
                output: format!("Error: overlap ({}) must be less than chunk_size ({})", overlap, chunk_size),
            });
        }

        match action {
            "import_file" => self.execute_import_file(&source, category, chunk_size, overlap).await,
            "import_directory" => self.execute_import_directory(&source, category, chunk_size, overlap).await,
            "import_url" => self.execute_import_url(&source, category, chunk_size, overlap).await,
            other => Ok(ToolResult {
                success: false,
                output: format!("Unknown action '{}'. Use 'import_file', 'import_directory', or 'import_url'.", other),
            }),
        }
    }
}

/// Find the nearest valid UTF-8 char boundary at or before `pos`.
fn find_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut p = pos;
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        let tool = KnowledgeImportTool::new();
        assert_eq!(tool.name(), "knowledge_import");
    }

    #[test]
    fn test_description() {
        let tool = KnowledgeImportTool::new();
        let desc = tool.description();
        assert!(desc.contains("knowledge"), "Description should mention knowledge: {}", desc);
        assert!(desc.contains("file"), "Description should mention file: {}", desc);
        assert!(desc.contains("URL"), "Description should mention URL: {}", desc);
    }

    #[test]
    fn test_schema() {
        let tool = KnowledgeImportTool::new();
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"][0], "source");
        assert!(schema["properties"]["source"].is_object());
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["category"].is_object());
        assert!(schema["properties"]["chunk_size"].is_object());
        assert!(schema["properties"]["overlap"].is_object());
    }

    #[test]
    fn test_preflight_empty_source() {
        let tool = KnowledgeImportTool::new();
        let args = json!({"source": ""});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("empty"), "Error should mention empty: {}", err);
    }

    #[test]
    fn test_preflight_missing_source() {
        let tool = KnowledgeImportTool::new();
        let args = json!({"action": "import_file"});
        let result = tool.preflight(&args);
        assert!(result.is_err());
    }

    #[test]
    fn test_preflight_nonexistent_file() {
        let tool = KnowledgeImportTool::new();
        let args = json!({"source": "/nonexistent/file.txt", "action": "import_file"});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("does not exist"), "Error should mention does not exist: {}", err);
    }

    #[test]
    fn test_preflight_invalid_url() {
        let tool = KnowledgeImportTool::new();
        let args = json!({"source": "not-a-url", "action": "import_url"});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("http"), "Error should mention http: {}", err);
    }

    #[test]
    fn test_preflight_valid_url() {
        let tool = KnowledgeImportTool::new();
        let args = json!({"source": "https://example.com", "action": "import_url"});
        let result = tool.preflight(&args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_preflight_invalid_action() {
        let tool = KnowledgeImportTool::new();
        let args = json!({"source": "https://example.com", "action": "invalid"});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unknown action"), "Error should mention unknown action: {}", err);
    }

    #[test]
    fn test_preflight_zero_chunk_size() {
        let tool = KnowledgeImportTool::new();
        let args = json!({"source": "https://example.com", "action": "import_url", "chunk_size": 0});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("greater than 0"), "Error should mention > 0: {}", err);
    }

    #[test]
    fn test_preflight_overlap_exceeds_chunk() {
        let tool = KnowledgeImportTool::new();
        let args = json!({"source": "https://example.com", "action": "import_url", "chunk_size": 100, "overlap": 100});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("less than chunk_size"), "Error should mention overlap < chunk_size: {}", err);
    }

    #[test]
    fn test_preflight_overlap_greater_than_chunk() {
        let tool = KnowledgeImportTool::new();
        let args = json!({"source": "https://example.com", "action": "import_url", "chunk_size": 50, "overlap": 200});
        let result = tool.preflight(&args);
        assert!(result.is_err());
    }

    // ── Chunking Tests ────────────────────────────────────────────────────────

    #[test]
    fn test_chunk_empty_text() {
        let chunks = KnowledgeImportTool::chunk_text("", 500, 50);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_small_text() {
        let text = "This is a short text.";
        let chunks = KnowledgeImportTool::chunk_text(text, 500, 50);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "This is a short text.");
    }

    #[test]
    fn test_chunk_whitespace_only() {
        let chunks = KnowledgeImportTool::chunk_text("   \n\n  \t  ", 500, 50);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_exact_size() {
        let text = "a".repeat(500);
        let chunks = KnowledgeImportTool::chunk_text(&text, 500, 50);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn test_chunk_multiple_chunks() {
        let text = "a".repeat(1200);
        let chunks = KnowledgeImportTool::chunk_text(&text, 500, 50);
        assert!(chunks.len() >= 2, "Should produce at least 2 chunks, got {}", chunks.len());
        // All chunks should be non-empty
        for chunk in &chunks {
            assert!(!chunk.is_empty(), "Chunk should not be empty");
        }
    }

    #[test]
    fn test_chunk_paragraph_boundary() {
        let text = "First paragraph with some content here.\n\nSecond paragraph with different content.\n\nThird paragraph at the end.";
        let chunks = KnowledgeImportTool::chunk_text(text, 60, 10);
        // Should prefer breaking at paragraph boundaries
        assert!(chunks.len() >= 2, "Should produce at least 2 chunks for paragraph text");
        for chunk in &chunks {
            assert!(!chunk.is_empty());
        }
    }

    #[test]
    fn test_chunk_unicode_safety() {
        let text = "你好世界。".repeat(200);
        let chunks = KnowledgeImportTool::chunk_text(&text, 100, 20);
        // Each chunk should be valid UTF-8 (Rust strings guarantee this)
        for chunk in &chunks {
            assert!(!chunk.is_empty());
            // Verify it's valid UTF-8 (accessing .len() would panic if not)
            let _ = chunk.len();
        }
    }

    #[test]
    fn test_chunk_overlap_works() {
        let text = "AAAA\n\nBBBB\n\nCCCC\n\nDDDD\n\nEEEE";
        let chunks_no_overlap = KnowledgeImportTool::chunk_text(text, 10, 0);
        let chunks_with_overlap = KnowledgeImportTool::chunk_text(text, 10, 3);
        // With overlap we should get at least as many chunks
        assert!(chunks_with_overlap.len() >= chunks_no_overlap.len(),
            "Overlap should produce >= chunks: {} vs {}", chunks_with_overlap.len(), chunks_no_overlap.len());
    }

    // ── File Type Detection Tests ─────────────────────────────────────────────

    #[test]
    fn test_detect_file_type() {
        assert_eq!(KnowledgeImportTool::detect_file_type("doc.txt"), "txt");
        assert_eq!(KnowledgeImportTool::detect_file_type("readme.md"), "md");
        assert_eq!(KnowledgeImportTool::detect_file_type("README.MD"), "md");
        assert_eq!(KnowledgeImportTool::detect_file_type("doc.markdown"), "md");
        assert_eq!(KnowledgeImportTool::detect_file_type("data.csv"), "csv");
        assert_eq!(KnowledgeImportTool::detect_file_type("config.json"), "json");
        assert_eq!(KnowledgeImportTool::detect_file_type("doc.pdf"), "pdf");
        assert_eq!(KnowledgeImportTool::detect_file_type("file.xyz"), "unknown");
    }

    // ── HTML Stripping Tests ──────────────────────────────────────────────────

    #[test]
    fn test_strip_html_tags_basic() {
        let html = "<p>Hello <b>world</b></p>";
        let text = KnowledgeImportTool::strip_html_tags(html);
        assert_eq!(text, "Hello world");
    }

    #[test]
    fn test_strip_html_tags_script() {
        let html = "<p>Text</p><script>alert('xss')</script><p>More</p>";
        let text = KnowledgeImportTool::strip_html_tags(html);
        assert!(!text.contains("alert"));
        assert!(text.contains("Text"));
        assert!(text.contains("More"));
    }

    #[test]
    fn test_strip_html_tags_entities() {
        let html = "Hello &amp; welcome &lt;user&gt;";
        let text = KnowledgeImportTool::strip_html_tags(html);
        assert_eq!(text, "Hello & welcome <user>");
    }

    #[test]
    fn test_strip_html_tags_style() {
        let html = "<style>body { color: red; }</style><p>Content</p>";
        let text = KnowledgeImportTool::strip_html_tags(html);
        assert!(!text.contains("color"));
        assert!(text.contains("Content"));
    }

    #[test]
    fn test_strip_html_tags_empty() {
        let text = KnowledgeImportTool::strip_html_tags("");
        assert_eq!(text, "");
    }

    // ── Database Tests ────────────────────────────────────────────────────────

    #[test]
    fn test_init_db() {
        let dir = std::env::temp_dir().join("clawtex_test_ki");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join("test_init.db");
        let _ = std::fs::remove_file(&db_path);

        let result = KnowledgeImportTool::init_db(&db_path);
        assert!(result.is_ok(), "DB init should succeed: {:?}", result.err());
        assert!(db_path.exists(), "DB file should be created");

        // Verify table exists by querying
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM knowledge_chunks", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 0);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_store_and_query_chunk() {
        let dir = std::env::temp_dir().join("clawtex_test_ki");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join("test_store.db");
        let _ = std::fs::remove_file(&db_path);

        KnowledgeImportTool::init_db(&db_path).unwrap();
        let id = KnowledgeImportTool::store_chunk(&db_path, "test.txt", "imported", 0, "Hello world").unwrap();
        assert!(!id.is_empty());

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let content: String = conn.query_row(
            "SELECT content FROM knowledge_chunks WHERE id = ?1", [&id], |row| row.get(0)
        ).unwrap();
        assert_eq!(content, "Hello world");

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_store_multiple_chunks() {
        let dir = std::env::temp_dir().join("clawtex_test_ki");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join("test_multi.db");
        let _ = std::fs::remove_file(&db_path);

        KnowledgeImportTool::init_db(&db_path).unwrap();
        KnowledgeImportTool::store_chunk(&db_path, "doc.txt", "imported", 0, "Chunk 0").unwrap();
        KnowledgeImportTool::store_chunk(&db_path, "doc.txt", "imported", 1, "Chunk 1").unwrap();
        KnowledgeImportTool::store_chunk(&db_path, "other.md", "notes", 0, "Other chunk").unwrap();

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM knowledge_chunks", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 3);

        let source_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM knowledge_chunks WHERE source = ?1", ["doc.txt"], |row| row.get(0)
        ).unwrap();
        assert_eq!(source_count, 2);

        let _ = std::fs::remove_file(&db_path);
    }

    // ── Import File Tests ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_import_txt_file() {
        let dir = std::env::temp_dir().join("clawtex_test_ki_import");
        let _ = std::fs::create_dir_all(&dir);
        let txt_path = dir.join("test_import.txt");
        std::fs::write(&txt_path, "This is test content.\nWith multiple lines.\nFor knowledge import.").unwrap();

        let tool = KnowledgeImportTool::new();
        let result = tool.execute(json!({
            "source": txt_path.to_string_lossy(),
            "action": "import_file",
            "category": "test"
        })).await.unwrap();

        assert!(result.success, "Import should succeed: {}", result.output);
        assert!(result.output.contains("imported successfully"));
        assert!(result.output.contains("txt"));

        let _ = std::fs::remove_file(&txt_path);
    }

    #[tokio::test]
    async fn test_import_json_file() {
        let dir = std::env::temp_dir().join("clawtex_test_ki_import");
        let _ = std::fs::create_dir_all(&dir);
        let json_path = dir.join("test_import.json");
        std::fs::write(&json_path, r#"{"key": "value", "items": [1, 2, 3]}"#).unwrap();

        let tool = KnowledgeImportTool::new();
        let result = tool.execute(json!({
            "source": json_path.to_string_lossy(),
            "action": "import_file",
            "category": "config"
        })).await.unwrap();

        assert!(result.success, "JSON import should succeed: {}", result.output);
        assert!(result.output.contains("json"));

        let _ = std::fs::remove_file(&json_path);
    }

    #[tokio::test]
    async fn test_import_csv_file() {
        let dir = std::env::temp_dir().join("clawtex_test_ki_import");
        let _ = std::fs::create_dir_all(&dir);
        let csv_path = dir.join("test_import.csv");
        std::fs::write(&csv_path, "name,age,city\nAlice,30,Taipei\nBob,25,Tokyo\n").unwrap();

        let tool = KnowledgeImportTool::new();
        let result = tool.execute(json!({
            "source": csv_path.to_string_lossy(),
            "action": "import_file"
        })).await.unwrap();

        assert!(result.success, "CSV import should succeed: {}", result.output);
        assert!(result.output.contains("csv"));

        let _ = std::fs::remove_file(&csv_path);
    }

    #[tokio::test]
    async fn test_import_nonexistent_file() {
        let tool = KnowledgeImportTool::new();
        let result = tool.execute(json!({
            "source": "/nonexistent/path/file.txt",
            "action": "import_file"
        })).await.unwrap();

        assert!(!result.success);
        assert!(result.output.contains("Failed to read"));
    }

    #[tokio::test]
    async fn test_execute_empty_source() {
        let tool = KnowledgeImportTool::new();
        let result = tool.execute(json!({"source": ""})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("required"));
    }

    #[tokio::test]
    async fn test_execute_invalid_action() {
        let tool = KnowledgeImportTool::new();
        let result = tool.execute(json!({"source": "test.txt", "action": "bad"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_execute_zero_chunk_size() {
        let tool = KnowledgeImportTool::new();
        let result = tool.execute(json!({"source": "test.txt", "chunk_size": 0})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("greater than 0"));
    }

    #[tokio::test]
    async fn test_execute_overlap_exceeds_chunk() {
        let tool = KnowledgeImportTool::new();
        let result = tool.execute(json!({"source": "test.txt", "chunk_size": 100, "overlap": 200})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("less than chunk_size"));
    }

    // ── find_char_boundary Tests ──────────────────────────────────────────────

    #[test]
    fn test_find_char_boundary_ascii() {
        let s = "hello world";
        assert_eq!(find_char_boundary(s, 5), 5);
        assert_eq!(find_char_boundary(s, 100), s.len());
    }

    #[test]
    fn test_find_char_boundary_unicode() {
        let s = "你好世界";
        // Each CJK char is 3 bytes in UTF-8
        assert_eq!(find_char_boundary(s, 3), 3); // After 你
        assert_eq!(find_char_boundary(s, 4), 3); // Mid-char, should snap back
        assert_eq!(find_char_boundary(s, 5), 3); // Mid-char, should snap back
        assert_eq!(find_char_boundary(s, 6), 6); // After 好
    }

    #[test]
    fn test_find_char_boundary_empty() {
        assert_eq!(find_char_boundary("", 0), 0);
        assert_eq!(find_char_boundary("", 10), 0);
    }
}
