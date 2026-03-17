//! Screenshot tool — capture screen using platform-specific commands.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolResult};

pub struct ScreenshotTool;

impl ScreenshotTool {
    pub fn new() -> Self {
        Self
    }
}

/// Build the platform-specific screenshot command.
fn build_capture_command(output_path: &str, region: Option<&str>) -> (String, Vec<String>) {
    #[cfg(target_os = "windows")]
    {
        // Use PowerShell to capture screen via .NET
        let script = if let Some(r) = region {
            // Parse region: "x,y,width,height"
            format!(
                r#"Add-Type -AssemblyName System.Drawing; Add-Type -AssemblyName System.Windows.Forms; $parts = '{}' -split ','; $x=[int]$parts[0]; $y=[int]$parts[1]; $w=[int]$parts[2]; $h=[int]$parts[3]; $bmp = New-Object System.Drawing.Bitmap($w, $h); $g = [System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($x, $y, 0, 0, (New-Object System.Drawing.Size($w, $h))); $bmp.Save('{}'); $g.Dispose(); $bmp.Dispose(); Write-Output 'OK'"#,
                r, output_path
            )
        } else {
            format!(
                r#"Add-Type -AssemblyName System.Drawing; Add-Type -AssemblyName System.Windows.Forms; $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds; $bmp = New-Object System.Drawing.Bitmap($bounds.Width, $bounds.Height); $g = [System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size); $bmp.Save('{}'); $g.Dispose(); $bmp.Dispose(); Write-Output 'OK'"#,
                output_path
            )
        };
        ("powershell".to_string(), vec!["-NoProfile".to_string(), "-Command".to_string(), script])
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(r) = region {
            // screencapture -R x,y,w,h output.png
            ("screencapture".to_string(), vec!["-R".to_string(), r.to_string(), output_path.to_string()])
        } else {
            ("screencapture".to_string(), vec![output_path.to_string()])
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(r) = region {
            // scrot --select --autoselect=x,y,W,H
            ("scrot".to_string(), vec!["-a".to_string(), r.to_string(), output_path.to_string()])
        } else {
            ("scrot".to_string(), vec![output_path.to_string()])
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        ("echo".to_string(), vec!["unsupported platform".to_string()])
    }
}

/// Validate region format: "x,y,width,height" — all non-negative integers.
fn validate_region(region: &str) -> bool {
    let parts: Vec<&str> = region.split(',').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|p| p.trim().parse::<u32>().is_ok())
}

/// Default output path for screenshots.
fn default_output_path() -> String {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{}/.clawtex/workspace/screenshot_{}.png", home, ts)
}

#[async_trait]
impl Tool for ScreenshotTool {
    fn name(&self) -> &str {
        "screenshot"
    }

    fn description(&self) -> &str {
        "Capture a screenshot of the screen or a specific region. Uses platform-specific commands (Windows: PowerShell, macOS: screencapture, Linux: scrot)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "Operation to perform: capture (default)"
                },
                "output_path": {
                    "type": "string",
                    "description": "Path to save the screenshot (default: workspace/screenshot_<timestamp>.png)"
                },
                "region": {
                    "type": "string",
                    "description": "Screen region to capture: 'x,y,width,height' (optional, captures full screen if omitted)"
                }
            },
            "required": []
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let operation = args["operation"].as_str().unwrap_or("capture").trim();
        if operation != "capture" {
            return Ok(ToolResult {
                success: false,
                output: format!(
                    "Unknown operation: '{}'. Only 'capture' is supported.",
                    operation
                ),
            });
        }

        let output_path = args["output_path"]
            .as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(default_output_path);

        let region = args["region"]
            .as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if let Some(ref r) = region {
            if !validate_region(r) {
                return Ok(ToolResult {
                    success: false,
                    output: format!(
                        "Invalid region format: '{}'. Expected 'x,y,width,height' with non-negative integers.",
                        r
                    ),
                });
            }
        }

        // Ensure parent directory exists
        if let Some(parent) = std::path::Path::new(&output_path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let (cmd, cmd_args) = build_capture_command(&output_path, region.as_deref());

        let output = tokio::process::Command::new(&cmd)
            .args(&cmd_args)
            .output()
            .await;

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();

                if out.status.success() {
                    // Check if file was actually created
                    let file_exists = std::path::Path::new(&output_path).exists();
                    let file_size = if file_exists {
                        std::fs::metadata(&output_path)
                            .map(|m| m.len())
                            .unwrap_or(0)
                    } else {
                        0
                    };

                    Ok(ToolResult {
                        success: true,
                        output: json!({
                            "message": "Screenshot captured successfully",
                            "path": output_path,
                            "file_exists": file_exists,
                            "file_size_bytes": file_size,
                            "region": region,
                            "stdout": stdout.trim()
                        })
                        .to_string(),
                    })
                } else {
                    Ok(ToolResult {
                        success: false,
                        output: format!(
                            "Screenshot command failed (exit code: {:?}). stderr: {}. stdout: {}",
                            out.status.code(),
                            stderr.trim(),
                            stdout.trim()
                        ),
                    })
                }
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: format!(
                    "Failed to execute screenshot command '{}': {}",
                    cmd, e
                ),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        assert_eq!(ScreenshotTool::new().name(), "screenshot");
    }

    #[test]
    fn test_description() {
        let tool = ScreenshotTool::new();
        assert!(tool.description().contains("screenshot"));
    }

    #[test]
    fn test_schema() {
        let tool = ScreenshotTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["operation"].is_object());
        assert!(schema["properties"]["output_path"].is_object());
        assert!(schema["properties"]["region"].is_object());
    }

    #[test]
    fn test_validate_region_valid() {
        assert!(validate_region("0,0,1920,1080"));
        assert!(validate_region("100,200,800,600"));
        assert!(validate_region("0,0,1,1"));
    }

    #[test]
    fn test_validate_region_invalid() {
        assert!(!validate_region("0,0,1920")); // only 3 parts
        assert!(!validate_region("a,b,c,d")); // not numbers
        assert!(!validate_region("")); // empty
        assert!(!validate_region("0,0,1920,1080,extra")); // too many parts
        assert!(!validate_region("-1,0,100,100")); // negative (u32 parse fails)
    }

    #[test]
    fn test_build_capture_command_full_screen() {
        let (cmd, args) = build_capture_command("test.png", None);
        #[cfg(target_os = "windows")]
        {
            assert_eq!(cmd, "powershell");
            assert!(args.iter().any(|a| a.contains("test.png")));
        }
        #[cfg(target_os = "macos")]
        {
            assert_eq!(cmd, "screencapture");
            assert!(args.contains(&"test.png".to_string()));
        }
        #[cfg(target_os = "linux")]
        {
            assert_eq!(cmd, "scrot");
            assert!(args.contains(&"test.png".to_string()));
        }
    }

    #[test]
    fn test_build_capture_command_with_region() {
        let (cmd, args) = build_capture_command("test.png", Some("0,0,800,600"));
        #[cfg(target_os = "windows")]
        {
            assert_eq!(cmd, "powershell");
            assert!(args.iter().any(|a| a.contains("0,0,800,600")));
        }
        #[cfg(target_os = "macos")]
        {
            assert_eq!(cmd, "screencapture");
            assert!(args.contains(&"-R".to_string()));
        }
        #[cfg(target_os = "linux")]
        {
            assert_eq!(cmd, "scrot");
            assert!(args.contains(&"-a".to_string()));
        }
    }

    #[test]
    fn test_default_output_path() {
        let path = default_output_path();
        assert!(path.contains("screenshot_"));
        assert!(path.ends_with(".png"));
        assert!(path.contains(".clawtex/workspace"));
    }

    #[tokio::test]
    async fn test_unknown_operation() {
        let tool = ScreenshotTool::new();
        let result = tool
            .execute(json!({"operation": "invalid"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown operation"));
    }

    #[tokio::test]
    async fn test_invalid_region() {
        let tool = ScreenshotTool::new();
        let result = tool
            .execute(json!({
                "operation": "capture",
                "region": "bad"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Invalid region"));
    }

    #[tokio::test]
    async fn test_capture_returns_result() {
        // This test runs an actual capture command, but it may fail in CI (no display)
        // We just ensure it doesn't panic and returns a ToolResult
        let tool = ScreenshotTool::new();
        let tmp = std::env::temp_dir()
            .join("clawtex_screenshot_test.png")
            .to_string_lossy()
            .to_string();
        let result = tool
            .execute(json!({
                "operation": "capture",
                "output_path": tmp
            }))
            .await
            .unwrap();
        // In headless/CI, this may fail, but should still return a ToolResult
        assert!(result.success || result.output.contains("failed") || result.output.contains("Failed"));
        let _ = std::fs::remove_file(&tmp);
    }
}
