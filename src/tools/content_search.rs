use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolResult, SecurityConfig};

/// Content search tool — grep-like search within workspace files
pub struct ContentSearchTool {
    security: SecurityConfig,
}

impl ContentSearchTool {
    pub fn new(security: SecurityConfig) -> Self {
        Self { security }
    }

    fn search_file(
        &self,
        path: &std::path::Path,
        pattern: &str,
        case_insensitive: bool,
        results: &mut Vec<SearchMatch>,
        max: usize,
    ) {
        if results.len() >= max {
            return;
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return, // Skip binary/unreadable files
        };

        let pattern_lower = if case_insensitive { pattern.to_lowercase() } else { String::new() };

        for (line_num, line) in content.lines().enumerate() {
            if results.len() >= max {
                return;
            }
            let matches = if case_insensitive {
                line.to_lowercase().contains(&pattern_lower)
            } else {
                line.contains(pattern)
            };
            if matches {
                results.push(SearchMatch {
                    file: path.to_string_lossy().to_string(),
                    line_number: line_num + 1,
                    line: line.to_string(),
                });
            }
        }
    }

    fn walk_and_search(
        &self,
        dir: &std::path::Path,
        pattern: &str,
        case_insensitive: bool,
        file_pattern: Option<&str>,
        results: &mut Vec<SearchMatch>,
        max: usize,
    ) {
        if results.len() >= max {
            return;
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries {
            if results.len() >= max {
                return;
            }
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with('.') || name == "node_modules" || name == "target" || name == "__pycache__" {
                    continue;
                }
                self.walk_and_search(&path, pattern, case_insensitive, file_pattern, results, max);
            } else if path.is_file() {
                // Apply file pattern filter
                if let Some(fp) = file_pattern {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if !super::glob_search::matches_glob(name, fp) {
                        continue;
                    }
                }
                self.search_file(&path, pattern, case_insensitive, results, max);
            }
        }
    }
}

#[derive(Debug)]
struct SearchMatch {
    file: String,
    line_number: usize,
    line: String,
}

#[async_trait]
impl Tool for ContentSearchTool {
    fn name(&self) -> &str { "content_search" }
    fn description(&self) -> &str {
        "Search for text content in files within the workspace. Like grep."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Text pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "Subdirectory to search in (relative to workspace)"
                },
                "file_pattern": {
                    "type": "string",
                    "description": "Only search files matching this glob (e.g., '*.rs')"
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "Case-insensitive search (default: false)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of matching lines (default: 50)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
        if pattern.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing 'pattern'".into() });
        }

        let workspace = self.security.workspace_path();
        let search_dir = if let Some(sub) = args.get("path").and_then(|v| v.as_str()) {
            workspace.join(sub)
        } else {
            workspace.clone()
        };

        if !search_dir.exists() {
            return Ok(ToolResult {
                success: false,
                output: format!("Directory not found: {}", search_dir.display()),
            });
        }

        let case_insensitive = args.get("case_insensitive").and_then(|v| v.as_bool()).unwrap_or(false);
        let file_pattern = args.get("file_pattern").and_then(|v| v.as_str());
        let max = args.get("max_results").and_then(|v| v.as_u64()).unwrap_or(50) as usize;

        let mut results = Vec::new();
        self.walk_and_search(&search_dir, pattern, case_insensitive, file_pattern, &mut results, max);

        if results.is_empty() {
            return Ok(ToolResult { success: true, output: format!("No matches for '{}' found", pattern) });
        }

        // Format results, making paths relative
        let formatted: Vec<String> = results.iter().map(|m| {
            let rel_path = std::path::Path::new(&m.file)
                .strip_prefix(&workspace)
                .map(|r| r.to_string_lossy().to_string())
                .unwrap_or_else(|_| m.file.clone());
            let line_preview = if m.line.len() > 120 { &m.line[..120] } else { &m.line };
            format!("{}:{}: {}", rel_path, m.line_number, line_preview.trim())
        }).collect();

        Ok(ToolResult {
            success: true,
            output: format!("Found {} matches:\n{}", results.len(), formatted.join("\n")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_content_search_missing_pattern() {
        let tool = ContentSearchTool::new(SecurityConfig::default());
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_content_search_nonexistent_dir() {
        let tool = ContentSearchTool::new(SecurityConfig {
            workspace_only: false,
            ..Default::default()
        });
        let result = tool.execute(json!({
            "pattern": "test",
            "path": "/tmp/clawtex_nonexistent_dir_12345"
        })).await.unwrap();
        assert!(!result.success);
    }
}
