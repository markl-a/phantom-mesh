//! docx_export tool — converts markdown content to DOCX via pandoc subprocess.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::debug;

use super::{Tool, ToolResult};

pub struct DocxExportTool;

impl DocxExportTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for DocxExportTool {
    fn name(&self) -> &str {
        "docx_export"
    }

    fn description(&self) -> &str {
        "Export markdown content to a DOCX file using pandoc. Requires pandoc installed."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Markdown content to convert to DOCX"
                },
                "output_path": {
                    "type": "string",
                    "description": "Output .docx file path. Defaults to workspace/{timestamp}.docx"
                },
                "title": {
                    "type": "string",
                    "description": "Optional document title (added as YAML front matter)"
                }
            },
            "required": ["content"]
        })
    }

    fn preflight(&self, args: &Value) -> Result<()> {
        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
        if content.trim().is_empty() {
            anyhow::bail!("Preflight: 'content' cannot be empty");
        }
        // Check pandoc availability
        match std::process::Command::new("pandoc").arg("--version").output() {
            Ok(output) if output.status.success() => Ok(()),
            _ => anyhow::bail!("Preflight: pandoc is not installed or not in PATH"),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let content = args.get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if content.trim().is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Error: 'content' is required".to_string(),
            });
        }

        let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");

        // Build markdown with optional title
        let md_content = if title.is_empty() {
            content.clone()
        } else {
            format!("---\ntitle: \"{}\"\n---\n\n{}", title, content)
        };

        // Output path
        let output_path = if let Some(p) = args.get("output_path").and_then(|v| v.as_str()) {
            std::path::PathBuf::from(p)
        } else {
            let workspace = dirs::home_dir()
                .unwrap_or_default()
                .join(".phantom-mesh")
                .join("workspace");
            let _ = std::fs::create_dir_all(&workspace);
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            workspace.join(format!("export_{}.docx", timestamp))
        };

        // Write temp markdown file
        let temp_dir = std::env::temp_dir();
        let temp_md = temp_dir.join("phantom_mesh_docx_input.md");
        std::fs::write(&temp_md, &md_content)?;

        debug!("Converting markdown to DOCX: {} -> {}", temp_md.display(), output_path.display());

        if let Some(parent) = output_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // Run pandoc
        let output = std::process::Command::new("pandoc")
            .arg(&temp_md)
            .arg("-o")
            .arg(&output_path)
            .arg("--from=markdown")
            .arg("--to=docx")
            .output();

        // Clean up temp
        let _ = std::fs::remove_file(&temp_md);

        match output {
            Ok(result) if result.status.success() => {
                let size = std::fs::metadata(&output_path)
                    .map(|m| m.len())
                    .unwrap_or(0);
                Ok(ToolResult {
                    success: true,
                    output: format!(
                        "DOCX exported successfully!\nPath: {}\nSize: {} bytes\nTitle: {}",
                        output_path.display(), size, if title.is_empty() { "(none)" } else { title }
                    ),
                })
            }
            Ok(result) => {
                let stderr = String::from_utf8_lossy(&result.stderr);
                Ok(ToolResult {
                    success: false,
                    output: format!("pandoc failed: {}", stderr),
                })
            }
            Err(e) => {
                Ok(ToolResult {
                    success: false,
                    output: format!("Failed to run pandoc: {}", e),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema() {
        let tool = DocxExportTool::new();
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"][0], "content");
    }

    #[test]
    fn test_name_description() {
        let tool = DocxExportTool::new();
        assert_eq!(tool.name(), "docx_export");
        assert!(tool.description().contains("DOCX"));
    }

    #[test]
    fn test_preflight_empty_content() {
        let tool = DocxExportTool::new();
        let result = tool.preflight(&json!({"content": ""}));
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_empty_content() {
        let tool = DocxExportTool::new();
        let result = tool.execute(json!({"content": ""})).await.unwrap();
        assert!(!result.success);
    }

    #[test]
    fn test_preflight_with_content() {
        let tool = DocxExportTool::new();
        // This will fail if pandoc not installed, which is expected in CI
        let result = tool.preflight(&json!({"content": "# Hello"}));
        // We just verify it doesn't panic; whether it passes depends on pandoc
        let _ = result;
    }

    #[tokio::test]
    async fn test_execute_with_pandoc() {
        let tool = DocxExportTool::new();
        // Only test if pandoc is available
        if std::process::Command::new("pandoc").arg("--version").output().is_err() {
            return; // Skip test
        }
        let temp = std::env::temp_dir().join("phantom_mesh_test_export.docx");
        let result = tool.execute(json!({
            "content": "# Test\n\nHello world",
            "output_path": temp.to_str().unwrap(),
            "title": "Test Doc"
        })).await.unwrap();
        assert!(result.success);
        assert!(temp.exists());
        let _ = std::fs::remove_file(&temp);
    }
}
