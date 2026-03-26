//! Archive extraction tool — list and extract contents of .zip, .tar, .tar.gz, .7z archives.
//! Uses system commands (tar, unzip, 7z) for extraction.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;
use tracing::debug;

use super::{Tool, ToolResult};

pub struct ArchiveExtractTool;

impl ArchiveExtractTool {
    pub fn new() -> Self {
        Self
    }

    /// Detect archive type from file extension.
    fn detect_type(path: &str) -> Option<ArchiveType> {
        let lower = path.to_lowercase();
        if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            Some(ArchiveType::TarGz)
        } else if lower.ends_with(".tar.bz2") || lower.ends_with(".tbz2") {
            Some(ArchiveType::TarBz2)
        } else if lower.ends_with(".tar.xz") || lower.ends_with(".txz") {
            Some(ArchiveType::TarXz)
        } else if lower.ends_with(".tar") {
            Some(ArchiveType::Tar)
        } else if lower.ends_with(".zip") {
            Some(ArchiveType::Zip)
        } else if lower.ends_with(".7z") {
            Some(ArchiveType::SevenZip)
        } else {
            None
        }
    }

    /// Build command to list archive contents.
    fn list_command(archive_type: &ArchiveType, archive_path: &str) -> (String, Vec<String>) {
        match archive_type {
            ArchiveType::Tar => ("tar".into(), vec!["-tf".into(), archive_path.into()]),
            ArchiveType::TarGz => ("tar".into(), vec!["-tzf".into(), archive_path.into()]),
            ArchiveType::TarBz2 => ("tar".into(), vec!["-tjf".into(), archive_path.into()]),
            ArchiveType::TarXz => ("tar".into(), vec!["-tJf".into(), archive_path.into()]),
            ArchiveType::Zip => {
                if cfg!(target_os = "windows") {
                    // Use PowerShell on Windows
                    ("powershell".into(), vec![
                        "-NoProfile".into(),
                        "-Command".into(),
                        format!(
                            "[System.IO.Compression.ZipFile]::OpenRead('{}').Entries | ForEach-Object {{ $_.FullName }}",
                            archive_path.replace('\'', "''")
                        ),
                    ])
                } else {
                    ("unzip".into(), vec!["-l".into(), archive_path.into()])
                }
            }
            ArchiveType::SevenZip => ("7z".into(), vec!["l".into(), archive_path.into()]),
        }
    }

    /// Build command to extract archive contents.
    fn extract_command(archive_type: &ArchiveType, archive_path: &str, output_dir: &str) -> (String, Vec<String>) {
        match archive_type {
            ArchiveType::Tar => ("tar".into(), vec!["-xf".into(), archive_path.into(), "-C".into(), output_dir.into()]),
            ArchiveType::TarGz => ("tar".into(), vec!["-xzf".into(), archive_path.into(), "-C".into(), output_dir.into()]),
            ArchiveType::TarBz2 => ("tar".into(), vec!["-xjf".into(), archive_path.into(), "-C".into(), output_dir.into()]),
            ArchiveType::TarXz => ("tar".into(), vec!["-xJf".into(), archive_path.into(), "-C".into(), output_dir.into()]),
            ArchiveType::Zip => {
                if cfg!(target_os = "windows") {
                    ("powershell".into(), vec![
                        "-NoProfile".into(),
                        "-Command".into(),
                        format!(
                            "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                            archive_path.replace('\'', "''"),
                            output_dir.replace('\'', "''")
                        ),
                    ])
                } else {
                    ("unzip".into(), vec!["-o".into(), archive_path.into(), "-d".into(), output_dir.into()])
                }
            }
            ArchiveType::SevenZip => ("7z".into(), vec!["x".into(), archive_path.into(), format!("-o{}", output_dir), "-y".into()]),
        }
    }

    /// Run a command and capture output.
    async fn run_command(program: &str, args: &[String], timeout_secs: u64) -> Result<(bool, String, String)> {
        let result = tokio::process::Command::new(program)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let child = match result {
            Ok(c) => c,
            Err(e) => return Ok((false, String::new(), format!("Failed to spawn '{}': {}", program, e))),
        };

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            child.wait_with_output(),
        ).await
            .map_err(|_| anyhow::anyhow!("Command timed out after {}s", timeout_secs))?
            .map_err(|e| anyhow::anyhow!("Command error: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Ok((output.status.success(), stdout, stderr))
    }

    /// Apply a glob-style filter to file listing lines.
    fn apply_filter(lines: &[&str], filter: &str) -> Vec<String> {
        let filter_lower = filter.to_lowercase();
        // Support simple glob: *.ext or *pattern*
        let is_extension_filter = filter_lower.starts_with("*.");
        let is_contains_filter = filter_lower.starts_with('*') && filter_lower.ends_with('*');

        lines.iter().filter(|line| {
            let line_lower = line.to_lowercase().trim().to_string();
            if line_lower.is_empty() {
                return false;
            }
            if is_extension_filter {
                let ext = &filter_lower[1..]; // ".ext"
                line_lower.ends_with(ext)
            } else if is_contains_filter {
                let pattern = &filter_lower[1..filter_lower.len() - 1];
                line_lower.contains(pattern)
            } else {
                line_lower.contains(&filter_lower)
            }
        }).map(|s| s.to_string()).collect()
    }
}

#[derive(Debug, Clone)]
enum ArchiveType {
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    Zip,
    SevenZip,
}

impl std::fmt::Display for ArchiveType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchiveType::Tar => write!(f, "tar"),
            ArchiveType::TarGz => write!(f, "tar.gz"),
            ArchiveType::TarBz2 => write!(f, "tar.bz2"),
            ArchiveType::TarXz => write!(f, "tar.xz"),
            ArchiveType::Zip => write!(f, "zip"),
            ArchiveType::SevenZip => write!(f, "7z"),
        }
    }
}

#[async_trait]
impl Tool for ArchiveExtractTool {
    fn name(&self) -> &str {
        "archive_extract"
    }

    fn description(&self) -> &str {
        "List and extract archive contents. Supports .zip, .tar, .tar.gz, .tar.bz2, .tar.xz, .7z. Operations: list (view contents), extract (extract files)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "One of: list, extract",
                    "enum": ["list", "extract"]
                },
                "archive_path": {
                    "type": "string",
                    "description": "Path to the archive file"
                },
                "output_dir": {
                    "type": "string",
                    "description": "Directory to extract to (for 'extract' operation). Defaults to archive's parent directory."
                },
                "filter": {
                    "type": "string",
                    "description": "Optional glob filter for listing (e.g., '*.rs', '*config*')"
                }
            },
            "required": ["operation", "archive_path"]
        })
    }

    fn preflight(&self, args: &Value) -> Result<()> {
        let operation = args.get("operation").and_then(|v| v.as_str()).unwrap_or("");
        if operation.is_empty() {
            anyhow::bail!("Preflight: 'operation' is required");
        }
        if !["list", "extract"].contains(&operation) {
            anyhow::bail!("Preflight: unknown operation '{}'. Use: list, extract", operation);
        }

        let archive_path = args.get("archive_path").and_then(|v| v.as_str()).unwrap_or("");
        if archive_path.is_empty() {
            anyhow::bail!("Preflight: 'archive_path' is required");
        }

        // Check archive file exists
        let path = Path::new(archive_path);
        if !path.exists() {
            anyhow::bail!("Preflight: archive file does not exist: {}", archive_path);
        }

        // Check supported type
        if Self::detect_type(archive_path).is_none() {
            anyhow::bail!(
                "Preflight: unsupported archive type for '{}'. Supported: .zip, .tar, .tar.gz, .tar.bz2, .tar.xz, .7z",
                archive_path
            );
        }

        Ok(())
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let operation = args["operation"].as_str().unwrap_or("").trim();
        let archive_path = args["archive_path"].as_str().unwrap_or("").trim();
        let filter = args["filter"].as_str().unwrap_or("").trim();

        if operation.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing required parameter: operation".into() });
        }
        if archive_path.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing required parameter: archive_path".into() });
        }

        let archive_type = match Self::detect_type(archive_path) {
            Some(t) => t,
            None => {
                return Ok(ToolResult {
                    success: false,
                    output: format!(
                        "Unsupported archive type for '{}'. Supported: .zip, .tar, .tar.gz, .tar.bz2, .tar.xz, .7z",
                        archive_path
                    ),
                });
            }
        };

        match operation {
            "list" => {
                let (program, cmd_args) = Self::list_command(&archive_type, archive_path);
                debug!("Listing archive: {} {:?}", program, cmd_args);

                let (success, stdout, stderr) = Self::run_command(&program, &cmd_args, 60).await?;

                if !success {
                    return Ok(ToolResult {
                        success: false,
                        output: format!("Failed to list archive contents: {}", stderr),
                    });
                }

                let lines: Vec<&str> = stdout.lines().collect();
                let display_lines = if !filter.is_empty() {
                    Self::apply_filter(&lines, filter)
                } else {
                    lines.iter().map(|s| s.to_string()).collect()
                };

                let result = json!({
                    "archive": archive_path,
                    "type": archive_type.to_string(),
                    "total_lines": lines.len(),
                    "showing": display_lines.len(),
                    "filter": if filter.is_empty() { None } else { Some(filter) },
                    "entries": display_lines,
                });

                Ok(ToolResult {
                    success: true,
                    output: serde_json::to_string_pretty(&result)?,
                })
            }
            "extract" => {
                let output_dir = args["output_dir"].as_str().unwrap_or("").trim();
                let output_dir = if output_dir.is_empty() {
                    // Default to archive's parent directory
                    Path::new(archive_path)
                        .parent()
                        .unwrap_or(Path::new("."))
                        .to_string_lossy()
                        .to_string()
                } else {
                    output_dir.to_string()
                };

                // Ensure output directory exists
                let _ = std::fs::create_dir_all(&output_dir);

                let (program, cmd_args) = Self::extract_command(&archive_type, archive_path, &output_dir);
                debug!("Extracting archive: {} {:?}", program, cmd_args);

                let (success, stdout, stderr) = Self::run_command(&program, &cmd_args, 300).await?;

                if !success {
                    return Ok(ToolResult {
                        success: false,
                        output: format!("Failed to extract archive: {}", stderr),
                    });
                }

                // Count extracted files
                let file_count = if !stdout.is_empty() {
                    stdout.lines().count()
                } else {
                    // Walk the output directory to count files
                    walkdir_count(&output_dir)
                };

                Ok(ToolResult {
                    success: true,
                    output: format!(
                        "Archive extracted successfully!\nArchive: {}\nType: {}\nOutput: {}\nFiles: ~{}\n{}",
                        archive_path,
                        archive_type,
                        output_dir,
                        file_count,
                        if !stdout.is_empty() { truncate_output(&stdout, 2000) } else { String::new() }
                    ),
                })
            }
            _ => Ok(ToolResult {
                success: false,
                output: format!("Unknown operation: '{}'. Use: list, extract", operation),
            }),
        }
    }
}

/// Count files in a directory (non-recursive simple count).
fn walkdir_count(dir: &str) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| entries.filter_map(|e| e.ok()).count())
        .unwrap_or(0)
}

/// Truncate output at safe char boundary.
fn truncate_output(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
        format!("{}...\n[output truncated]", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        let tool = ArchiveExtractTool::new();
        assert_eq!(tool.name(), "archive_extract");
    }

    #[test]
    fn test_schema() {
        let tool = ArchiveExtractTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["operation"].is_object());
        assert!(schema["properties"]["archive_path"].is_object());
        assert!(schema["properties"]["output_dir"].is_object());
        assert!(schema["properties"]["filter"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("operation")));
        assert!(required.contains(&json!("archive_path")));
    }

    #[test]
    fn test_detect_type_tar_gz() {
        assert!(matches!(ArchiveExtractTool::detect_type("file.tar.gz"), Some(ArchiveType::TarGz)));
        assert!(matches!(ArchiveExtractTool::detect_type("file.tgz"), Some(ArchiveType::TarGz)));
        assert!(matches!(ArchiveExtractTool::detect_type("FILE.TAR.GZ"), Some(ArchiveType::TarGz)));
    }

    #[test]
    fn test_detect_type_tar() {
        assert!(matches!(ArchiveExtractTool::detect_type("file.tar"), Some(ArchiveType::Tar)));
    }

    #[test]
    fn test_detect_type_zip() {
        assert!(matches!(ArchiveExtractTool::detect_type("file.zip"), Some(ArchiveType::Zip)));
        assert!(matches!(ArchiveExtractTool::detect_type("file.ZIP"), Some(ArchiveType::Zip)));
    }

    #[test]
    fn test_detect_type_7z() {
        assert!(matches!(ArchiveExtractTool::detect_type("archive.7z"), Some(ArchiveType::SevenZip)));
    }

    #[test]
    fn test_detect_type_tar_bz2() {
        assert!(matches!(ArchiveExtractTool::detect_type("file.tar.bz2"), Some(ArchiveType::TarBz2)));
        assert!(matches!(ArchiveExtractTool::detect_type("file.tbz2"), Some(ArchiveType::TarBz2)));
    }

    #[test]
    fn test_detect_type_tar_xz() {
        assert!(matches!(ArchiveExtractTool::detect_type("file.tar.xz"), Some(ArchiveType::TarXz)));
        assert!(matches!(ArchiveExtractTool::detect_type("file.txz"), Some(ArchiveType::TarXz)));
    }

    #[test]
    fn test_detect_type_unknown() {
        assert!(ArchiveExtractTool::detect_type("file.pdf").is_none());
        assert!(ArchiveExtractTool::detect_type("file.txt").is_none());
        assert!(ArchiveExtractTool::detect_type("file").is_none());
    }

    #[test]
    fn test_list_command_tar() {
        let (prog, args) = ArchiveExtractTool::list_command(&ArchiveType::Tar, "/tmp/test.tar");
        assert_eq!(prog, "tar");
        assert!(args.contains(&"-tf".to_string()));
    }

    #[test]
    fn test_list_command_tar_gz() {
        let (prog, args) = ArchiveExtractTool::list_command(&ArchiveType::TarGz, "/tmp/test.tar.gz");
        assert_eq!(prog, "tar");
        assert!(args.contains(&"-tzf".to_string()));
    }

    #[test]
    fn test_list_command_7z() {
        let (prog, args) = ArchiveExtractTool::list_command(&ArchiveType::SevenZip, "/tmp/test.7z");
        assert_eq!(prog, "7z");
        assert!(args.contains(&"l".to_string()));
    }

    #[test]
    fn test_extract_command_tar_gz() {
        let (prog, args) = ArchiveExtractTool::extract_command(&ArchiveType::TarGz, "/tmp/test.tar.gz", "/tmp/out");
        assert_eq!(prog, "tar");
        assert!(args.contains(&"-xzf".to_string()));
        assert!(args.contains(&"-C".to_string()));
        assert!(args.contains(&"/tmp/out".to_string()));
    }

    #[test]
    fn test_extract_command_7z() {
        let (prog, args) = ArchiveExtractTool::extract_command(&ArchiveType::SevenZip, "/tmp/test.7z", "/tmp/out");
        assert_eq!(prog, "7z");
        assert!(args.contains(&"x".to_string()));
        assert!(args.contains(&"-y".to_string()));
    }

    #[test]
    fn test_apply_filter_extension() {
        let lines = vec!["src/main.rs", "src/lib.rs", "Cargo.toml", "README.md"];
        let filtered = ArchiveExtractTool::apply_filter(&lines, "*.rs");
        assert_eq!(filtered.len(), 2);
        assert!(filtered.contains(&"src/main.rs".to_string()));
        assert!(filtered.contains(&"src/lib.rs".to_string()));
    }

    #[test]
    fn test_apply_filter_contains() {
        let lines = vec!["src/main.rs", "config/settings.toml", "config/prod.toml", "README.md"];
        let filtered = ArchiveExtractTool::apply_filter(&lines, "*config*");
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_apply_filter_exact_substring() {
        let lines = vec!["src/main.rs", "src/lib.rs", "test/main_test.rs"];
        let filtered = ArchiveExtractTool::apply_filter(&lines, "main");
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_apply_filter_empty() {
        let lines = vec!["file1.txt", "file2.txt"];
        let filtered = ArchiveExtractTool::apply_filter(&lines, "");
        // Empty filter matches all non-empty lines
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_preflight_missing_operation() {
        let tool = ArchiveExtractTool::new();
        let result = tool.preflight(&json!({"archive_path": "/tmp/test.zip"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("operation"));
    }

    #[test]
    fn test_preflight_invalid_operation() {
        let tool = ArchiveExtractTool::new();
        let result = tool.preflight(&json!({"operation": "delete", "archive_path": "/tmp/test.zip"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown operation"));
    }

    #[test]
    fn test_preflight_missing_archive_path() {
        let tool = ArchiveExtractTool::new();
        let result = tool.preflight(&json!({"operation": "list"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("archive_path"));
    }

    #[test]
    fn test_preflight_nonexistent_file() {
        let tool = ArchiveExtractTool::new();
        let result = tool.preflight(&json!({"operation": "list", "archive_path": "/nonexistent/file.zip"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));
    }

    #[test]
    fn test_preflight_unsupported_type() {
        // Create a temp file with unsupported extension
        let tmp = std::env::temp_dir().join("test_archive_preflight.pdf");
        let _ = std::fs::write(&tmp, b"dummy");
        let result = ArchiveExtractTool::new().preflight(&json!({
            "operation": "list",
            "archive_path": tmp.to_string_lossy()
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unsupported"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn test_execute_missing_operation() {
        let tool = ArchiveExtractTool::new();
        let result = tool.execute(json!({"archive_path": "/tmp/test.zip"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_execute_missing_archive_path() {
        let tool = ArchiveExtractTool::new();
        let result = tool.execute(json!({"operation": "list"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_execute_unsupported_type() {
        let tool = ArchiveExtractTool::new();
        let result = tool.execute(json!({"operation": "list", "archive_path": "/tmp/file.docx"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unsupported"));
    }

    #[tokio::test]
    async fn test_execute_unknown_operation() {
        let tool = ArchiveExtractTool::new();
        let result = tool.execute(json!({"operation": "compress", "archive_path": "/tmp/test.zip"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown operation"));
    }

    #[test]
    fn test_archive_type_display() {
        assert_eq!(format!("{}", ArchiveType::TarGz), "tar.gz");
        assert_eq!(format!("{}", ArchiveType::Zip), "zip");
        assert_eq!(format!("{}", ArchiveType::SevenZip), "7z");
        assert_eq!(format!("{}", ArchiveType::Tar), "tar");
        assert_eq!(format!("{}", ArchiveType::TarBz2), "tar.bz2");
        assert_eq!(format!("{}", ArchiveType::TarXz), "tar.xz");
    }

    #[test]
    fn test_truncate_output() {
        assert_eq!(truncate_output("hello", 10), "hello");
        let long = "a".repeat(3000);
        let truncated = truncate_output(&long, 100);
        assert!(truncated.contains("[output truncated]"));
        assert!(truncated.len() < long.len());
    }

    #[test]
    fn test_walkdir_count_nonexistent() {
        // Use a path that cannot possibly exist (random UUID suffix ensures it)
        let nonexistent = std::env::temp_dir()
            .join("phantom_mesh_nonexistent_dir_29a1f74b8c3e")
            .to_string_lossy()
            .to_string();
        // Ensure it really doesn't exist
        let _ = std::fs::remove_dir_all(&nonexistent);
        assert_eq!(walkdir_count(&nonexistent), 0);
    }
}
