use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolResult, SecurityConfig};

/// Glob search tool — find files by pattern within the workspace
pub struct GlobSearchTool {
    security: SecurityConfig,
}

impl GlobSearchTool {
    pub fn new(security: SecurityConfig) -> Self {
        Self { security }
    }

    fn walk_dir(&self, dir: &std::path::Path, pattern: &str, results: &mut Vec<String>, max: usize) {
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
                // Skip hidden directories and common large directories
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with('.') || name == "node_modules" || name == "target" || name == "__pycache__" {
                    continue;
                }
                self.walk_dir(&path, pattern, results, max);
            } else if path.is_file() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if matches_glob(name, pattern) {
                    results.push(path.to_string_lossy().to_string());
                }
            }
        }
    }
}

/// Simple glob matching (supports * and ?)
pub fn matches_glob(name: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    let mut name_chars = name.chars().peekable();
    let mut pattern_chars = pattern.chars().peekable();

    while let Some(pc) = pattern_chars.next() {
        match pc {
            '*' => {
                // Match zero or more characters
                if pattern_chars.peek().is_none() {
                    return true; // Trailing * matches everything
                }
                // Try matching rest of pattern from each position
                let rest: String = pattern_chars.collect();
                let remaining: String = name_chars.collect();
                for i in 0..=remaining.len() {
                    if matches_glob(&remaining[i..], &rest) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                if name_chars.next().is_none() {
                    return false;
                }
            }
            c => {
                if name_chars.next() != Some(c) {
                    return false;
                }
            }
        }
    }

    name_chars.next().is_none()
}

#[async_trait]
impl Tool for GlobSearchTool {
    fn name(&self) -> &str { "glob_search" }
    fn description(&self) -> &str {
        "Search for files by name pattern within the workspace. Supports * and ? wildcards."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match file names (e.g., '*.rs', 'test_*')"
                },
                "path": {
                    "type": "string",
                    "description": "Subdirectory to search in (relative to workspace, default: workspace root)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results (default: 50)"
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
            return Ok(ToolResult { success: false, output: format!("Directory not found: {}", search_dir.display()) });
        }

        let max = args.get("max_results").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
        let mut results = Vec::new();
        self.walk_dir(&search_dir, pattern, &mut results, max);

        // Make paths relative to workspace
        let relative: Vec<String> = results.iter().map(|p| {
            std::path::Path::new(p)
                .strip_prefix(&workspace)
                .map(|r| r.to_string_lossy().to_string())
                .unwrap_or_else(|_| p.clone())
        }).collect();

        if relative.is_empty() {
            Ok(ToolResult { success: true, output: format!("No files matching '{}' found", pattern) })
        } else {
            Ok(ToolResult {
                success: true,
                output: format!("Found {} files:\n{}", relative.len(), relative.join("\n")),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_glob_star() {
        assert!(matches_glob("hello.rs", "*.rs"));
        assert!(matches_glob("test.py", "*.py"));
        assert!(!matches_glob("hello.rs", "*.py"));
    }

    #[test]
    fn test_matches_glob_question() {
        assert!(matches_glob("a.rs", "?.rs"));
        assert!(!matches_glob("ab.rs", "?.rs"));
    }

    #[test]
    fn test_matches_glob_wildcard_all() {
        assert!(matches_glob("anything", "*"));
    }

    #[test]
    fn test_matches_glob_prefix() {
        assert!(matches_glob("test_hello.rs", "test_*"));
        assert!(!matches_glob("hello_test.rs", "test_*"));
    }

    #[test]
    fn test_matches_glob_exact() {
        assert!(matches_glob("Cargo.toml", "Cargo.toml"));
        assert!(!matches_glob("cargo.toml", "Cargo.toml"));
    }

    #[tokio::test]
    async fn test_glob_missing_pattern() {
        let tool = GlobSearchTool::new(SecurityConfig::default());
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
    }
}
