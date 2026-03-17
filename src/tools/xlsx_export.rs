//! xlsx_export tool — converts CSV data to XLSX via Python openpyxl subprocess.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::debug;

use super::{Tool, ToolResult};

pub struct XlsxExportTool;

impl XlsxExportTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for XlsxExportTool {
    fn name(&self) -> &str {
        "xlsx_export"
    }

    fn description(&self) -> &str {
        "Export CSV data to an XLSX spreadsheet using Python openpyxl. Requires Python + openpyxl."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "data": {
                    "type": "string",
                    "description": "CSV-formatted data to convert to XLSX"
                },
                "output_path": {
                    "type": "string",
                    "description": "Output .xlsx file path. Defaults to workspace/{timestamp}.xlsx"
                },
                "sheet_name": {
                    "type": "string",
                    "description": "Worksheet name. Defaults to 'Sheet1'"
                }
            },
            "required": ["data"]
        })
    }

    fn preflight(&self, args: &Value) -> Result<()> {
        let data = args.get("data").and_then(|v| v.as_str()).unwrap_or("");
        if data.trim().is_empty() {
            anyhow::bail!("Preflight: 'data' (CSV) cannot be empty");
        }
        // Check Python + openpyxl availability
        let python = find_python();
        match std::process::Command::new(&python)
            .args(["-c", "import openpyxl; print('ok')"])
            .output()
        {
            Ok(output) if output.status.success() => Ok(()),
            _ => anyhow::bail!("Preflight: Python with openpyxl not available (tried: {})", python),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let data = args.get("data")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if data.trim().is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Error: 'data' (CSV) is required".to_string(),
            });
        }

        let sheet_name = args.get("sheet_name")
            .and_then(|v| v.as_str())
            .unwrap_or("Sheet1");

        // Output path
        let output_path = if let Some(p) = args.get("output_path").and_then(|v| v.as_str()) {
            std::path::PathBuf::from(p)
        } else {
            let workspace = dirs::home_dir()
                .unwrap_or_default()
                .join(".clawtex")
                .join("workspace");
            let _ = std::fs::create_dir_all(&workspace);
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            workspace.join(format!("export_{}.xlsx", timestamp))
        };

        // Write CSV to temp file
        let temp_dir = std::env::temp_dir();
        let temp_csv = temp_dir.join("clawtex_xlsx_input.csv");
        std::fs::write(&temp_csv, &data)?;

        // Generate Python script
        let output_str = output_path.display().to_string().replace('\\', "/");
        let csv_str = temp_csv.display().to_string().replace('\\', "/");
        let py_script = format!(
            r#"import csv, openpyxl, sys
wb = openpyxl.Workbook()
ws = wb.active
ws.title = "{sheet_name}"
with open("{csv_str}", "r", encoding="utf-8") as f:
    reader = csv.reader(f)
    for row in reader:
        ws.append(row)
wb.save("{output_str}")
print(f"Saved to {output_str}")
"#
        );

        let temp_py = temp_dir.join("clawtex_xlsx_gen.py");
        std::fs::write(&temp_py, &py_script)?;

        debug!("Converting CSV to XLSX: {} -> {}", temp_csv.display(), output_path.display());

        if let Some(parent) = output_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let python = find_python();
        let output = std::process::Command::new(&python)
            .arg(&temp_py)
            .output();

        // Cleanup
        let _ = std::fs::remove_file(&temp_csv);
        let _ = std::fs::remove_file(&temp_py);

        match output {
            Ok(result) if result.status.success() => {
                let size = std::fs::metadata(&output_path)
                    .map(|m| m.len())
                    .unwrap_or(0);
                Ok(ToolResult {
                    success: true,
                    output: format!(
                        "XLSX exported successfully!\nPath: {}\nSize: {} bytes\nSheet: {}",
                        output_path.display(), size, sheet_name
                    ),
                })
            }
            Ok(result) => {
                let stderr = String::from_utf8_lossy(&result.stderr);
                Ok(ToolResult {
                    success: false,
                    output: format!("Python script failed: {}", stderr),
                })
            }
            Err(e) => {
                Ok(ToolResult {
                    success: false,
                    output: format!("Failed to run Python: {}", e),
                })
            }
        }
    }
}

/// Find available Python executable
fn find_python() -> String {
    for cmd in &["python3", "python", "C:\\Python314\\python.exe"] {
        if let Ok(output) = std::process::Command::new(cmd).arg("--version").output() {
            if output.status.success() {
                return cmd.to_string();
            }
        }
    }
    "python3".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema() {
        let tool = XlsxExportTool::new();
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"][0], "data");
    }

    #[test]
    fn test_name_description() {
        let tool = XlsxExportTool::new();
        assert_eq!(tool.name(), "xlsx_export");
        assert!(tool.description().contains("XLSX"));
    }

    #[test]
    fn test_preflight_empty_data() {
        let tool = XlsxExportTool::new();
        let result = tool.preflight(&json!({"data": ""}));
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_empty_data() {
        let tool = XlsxExportTool::new();
        let result = tool.execute(json!({"data": ""})).await.unwrap();
        assert!(!result.success);
    }

    #[test]
    fn test_find_python() {
        let py = find_python();
        assert!(!py.is_empty());
    }

    #[tokio::test]
    async fn test_execute_with_openpyxl() {
        let tool = XlsxExportTool::new();
        // Only test if Python + openpyxl is available
        let py = find_python();
        if std::process::Command::new(&py)
            .args(["-c", "import openpyxl"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            let temp = std::env::temp_dir().join("clawtex_test_export.xlsx");
            let result = tool.execute(json!({
                "data": "Name,Age,City\nAlice,30,Tokyo\nBob,25,Taipei",
                "output_path": temp.to_str().unwrap(),
                "sheet_name": "People"
            })).await.unwrap();
            assert!(result.success, "Failed: {}", result.output);
            assert!(temp.exists());
            let _ = std::fs::remove_file(&temp);
        }
    }
}
