//! PDF Export tool — converts Markdown reports to PDF using pandoc or weasyprint.
//! Falls back to a basic Python-based HTML→PDF conversion if neither is available.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use crate::tools::Tool;

pub struct PdfExportTool {
    workspace_dir: String,
}

impl PdfExportTool {
    pub fn new(workspace_dir: &str) -> Self {
        Self { workspace_dir: workspace_dir.to_string() }
    }
}

#[async_trait]
impl Tool for PdfExportTool {
    fn name(&self) -> &str { "pdf_export" }

    fn description(&self) -> &str {
        "Convert a Markdown file to a professional PDF document"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "input_file": {
                    "type": "string",
                    "description": "Path to the Markdown file (relative to workspace)"
                },
                "output_file": {
                    "type": "string",
                    "description": "Output PDF filename (relative to workspace)"
                },
                "title": {
                    "type": "string",
                    "description": "Document title for the PDF header"
                },
                "author": {
                    "type": "string",
                    "description": "Author name for the PDF metadata"
                }
            },
            "required": ["input_file", "output_file"]
        })
    }

    async fn execute(&self, params: Value) -> Result<crate::tools::ToolResult> {
        let input_file = params["input_file"].as_str()
            .ok_or_else(|| anyhow::anyhow!("input_file is required"))?;
        let output_file = params["output_file"].as_str()
            .ok_or_else(|| anyhow::anyhow!("output_file is required"))?;
        let title = params["title"].as_str().unwrap_or("Report");
        let author = params["author"].as_str().unwrap_or("Clawtex");

        let input_path = std::path::Path::new(&self.workspace_dir).join(input_file);
        let output_path = std::path::Path::new(&self.workspace_dir).join(output_file);

        if !input_path.exists() {
            return Ok(crate::tools::ToolResult {
                success: false,
                output: format!("Input file not found: {}", input_path.display()),
            });
        }

        // Try pandoc first, then weasyprint, then Python fallback
        let result = try_pandoc(&input_path, &output_path, title, author).await
            .or_else(|_| {
                warn!("pandoc not available, trying Python fallback");
                Ok::<_, anyhow::Error>(false)
            })?;

        if !result {
            let result = try_python_pdf(&input_path, &output_path, title, author).await?;
            if !result {
                return Ok(crate::tools::ToolResult {
                    success: false,
                    output: "PDF export failed: no converter available. Install pandoc (recommended) or Python markdown+weasyprint.".to_string(),
                });
            }
        }

        info!("PDF exported: {} → {}", input_path.display(), output_path.display());
        Ok(crate::tools::ToolResult {
            success: true,
            output: format!("PDF created: {}", output_path.display()),
        })
    }
}

/// Try converting with pandoc (best quality)
async fn try_pandoc(
    input: &std::path::Path,
    output: &std::path::Path,
    title: &str,
    author: &str,
) -> Result<bool> {
    let result = tokio::process::Command::new("pandoc")
        .arg(input)
        .arg("-o")
        .arg(output)
        .arg("--pdf-engine=xelatex")
        .arg("-V")
        .arg(format!("title={}", title))
        .arg("-V")
        .arg(format!("author={}", author))
        .arg("-V")
        .arg("geometry:margin=1in")
        .arg("--highlight-style=tango")
        .output()
        .await;

    match result {
        Ok(output_result) => {
            if output_result.status.success() && output.exists() {
                Ok(true)
            } else {
                let stderr = String::from_utf8_lossy(&output_result.stderr);
                debug!("pandoc failed: {}", stderr);
                Ok(false)
            }
        }
        Err(_) => Ok(false), // pandoc not found
    }
}

/// Fallback: Python-based markdown→HTML→PDF conversion
async fn try_python_pdf(
    input: &std::path::Path,
    output: &std::path::Path,
    title: &str,
    author: &str,
) -> Result<bool> {
    let python_script = format!(
        r#"
import sys
try:
    import markdown
except ImportError:
    print("ERROR: pip install markdown")
    sys.exit(1)

input_path = sys.argv[1]
output_path = sys.argv[2]
title = sys.argv[3]
author = sys.argv[4]

with open(input_path, 'r', encoding='utf-8') as f:
    md_content = f.read()

html_content = markdown.markdown(md_content, extensions=['tables', 'fenced_code', 'toc'])

full_html = f"""<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>{{title}}</title>
<style>
body {{ font-family: 'Segoe UI', Arial, sans-serif; max-width: 800px; margin: 40px auto; padding: 0 20px; line-height: 1.6; color: #333; }}
h1 {{ color: #2c3e50; border-bottom: 2px solid #3498db; padding-bottom: 10px; }}
h2 {{ color: #2980b9; }}
h3 {{ color: #7f8c8d; }}
table {{ border-collapse: collapse; width: 100%; margin: 20px 0; }}
th, td {{ border: 1px solid #ddd; padding: 8px; text-align: left; }}
th {{ background-color: #3498db; color: white; }}
tr:nth-child(even) {{ background-color: #f2f2f2; }}
code {{ background-color: #f4f4f4; padding: 2px 6px; border-radius: 3px; }}
pre {{ background-color: #2c3e50; color: #ecf0f1; padding: 15px; border-radius: 5px; overflow-x: auto; }}
blockquote {{ border-left: 4px solid #3498db; margin: 0; padding: 10px 20px; background-color: #ebf5fb; }}
.header {{ text-align: center; margin-bottom: 40px; }}
.footer {{ text-align: center; margin-top: 40px; font-size: 0.8em; color: #999; }}
</style>
</head>
<body>
<div class="header">
<h1>{{title}}</h1>
<p>By {{author}}</p>
</div>
{{html_content}}
<div class="footer">
<p>Generated by Clawtex</p>
</div>
</body>
</html>"""

# Try weasyprint for PDF
try:
    from weasyprint import HTML
    HTML(string=full_html).write_pdf(output_path)
    print(f"PDF created with weasyprint: {{output_path}}")
except ImportError:
    # Fallback: save as HTML (still useful)
    html_path = output_path.replace('.pdf', '.html')
    with open(html_path, 'w', encoding='utf-8') as f:
        f.write(full_html)
    print(f"HTML created (install weasyprint for PDF): {{html_path}}")
"#
    );

    let result = tokio::process::Command::new("python")
        .arg("-c")
        .arg(&python_script)
        .arg(input.to_str().unwrap_or(""))
        .arg(output.to_str().unwrap_or(""))
        .arg(title)
        .arg(author)
        .output()
        .await;

    match result {
        Ok(output_result) => {
            let stdout = String::from_utf8_lossy(&output_result.stdout);
            debug!("Python PDF: {}", stdout);
            Ok(output_result.status.success())
        }
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pdf_export_tool_name() {
        let tool = PdfExportTool::new("/tmp");
        assert_eq!(tool.name(), "pdf_export");
    }

    #[test]
    fn test_pdf_export_schema() {
        let tool = PdfExportTool::new("/tmp");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["input_file"].is_object());
        assert!(schema["properties"]["output_file"].is_object());
    }

    #[tokio::test]
    async fn test_pdf_export_missing_input() {
        let dir = tempfile::tempdir().unwrap();
        let tool = PdfExportTool::new(dir.path().to_str().unwrap());
        let result = tool.execute(json!({
            "input_file": "nonexistent.md",
            "output_file": "output.pdf"
        })).await.unwrap();
        assert!(result.output.contains("not found"));
    }
}
