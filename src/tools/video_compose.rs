//! Video composition tool — compose videos from images, audio, and other videos using ffmpeg.
//! Actions: slideshow (images->video), overlay_audio (video+audio->video),
//! concat (videos->video), add_subtitles (video+srt->video).

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{debug, warn};

use super::{Tool, ToolResult};

pub struct VideoComposeTool;

impl VideoComposeTool {
    pub fn new() -> Self {
        Self
    }

    /// Build the output file path (in workspace).
    fn build_output_path(output_path: Option<&str>) -> std::path::PathBuf {
        if let Some(p) = output_path {
            std::path::PathBuf::from(p)
        } else {
            let workspace = dirs::home_dir()
                .unwrap_or_default()
                .join(".clawtex")
                .join("workspace");
            let _ = std::fs::create_dir_all(&workspace);
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            workspace.join(format!("video_{}.mp4", timestamp))
        }
    }

    /// Parse resolution string like "1920x1080" into (width, height).
    fn parse_resolution(res: &str) -> Option<(u32, u32)> {
        let parts: Vec<&str> = res.split('x').collect();
        if parts.len() != 2 {
            return None;
        }
        let w = parts[0].parse::<u32>().ok()?;
        let h = parts[1].parse::<u32>().ok()?;
        if w == 0 || h == 0 || w > 7680 || h > 4320 {
            return None;
        }
        Some((w, h))
    }

    /// Extract input_files array from args as Vec<String>.
    fn extract_input_files(args: &Value) -> Vec<String> {
        args.get("input_files")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Build ffmpeg args for slideshow action (images -> video).
    fn build_slideshow_args(
        input_files: &[String],
        output: &std::path::Path,
        duration_per_image: f64,
        resolution: &str,
        fps: u32,
    ) -> Result<Vec<String>> {
        if input_files.is_empty() {
            anyhow::bail!("slideshow requires at least one input image");
        }

        let (w, h) = Self::parse_resolution(resolution)
            .ok_or_else(|| anyhow::anyhow!("Invalid resolution: {}", resolution))?;

        // Use concat demuxer approach: create a temporary file list
        // For command construction, we build a complex filter approach instead
        // Use -loop 1 -t <dur> for each image with concat filter
        let mut args = Vec::new();

        for file in input_files {
            args.push("-loop".to_string());
            args.push("1".to_string());
            args.push("-t".to_string());
            args.push(format!("{:.1}", duration_per_image));
            args.push("-i".to_string());
            args.push(file.clone());
        }

        // Build filter_complex for concat
        let n = input_files.len();
        let mut filter = String::new();
        for i in 0..n {
            filter.push_str(&format!(
                "[{idx}:v]scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={fps}[v{idx}];",
                idx = i, w = w, h = h, fps = fps
            ));
        }
        let stream_refs: Vec<String> = (0..n).map(|i| format!("[v{}]", i)).collect();
        filter.push_str(&format!(
            "{}concat=n={}:v=1:a=0[outv]",
            stream_refs.join(""),
            n
        ));

        args.push("-filter_complex".to_string());
        args.push(filter);
        args.push("-map".to_string());
        args.push("[outv]".to_string());
        args.push("-c:v".to_string());
        args.push("libx264".to_string());
        args.push("-pix_fmt".to_string());
        args.push("yuv420p".to_string());
        args.push("-y".to_string());
        args.push(output.to_string_lossy().to_string());

        Ok(args)
    }

    /// Build ffmpeg args for overlay_audio action (video + audio -> video).
    fn build_overlay_audio_args(
        input_files: &[String],
        audio_path: &str,
        output: &std::path::Path,
    ) -> Result<Vec<String>> {
        if input_files.is_empty() {
            anyhow::bail!("overlay_audio requires at least one input video in input_files");
        }

        let video_path = &input_files[0];
        let args = vec![
            "-i".to_string(),
            video_path.clone(),
            "-i".to_string(),
            audio_path.to_string(),
            "-c:v".to_string(),
            "copy".to_string(),
            "-c:a".to_string(),
            "aac".to_string(),
            "-map".to_string(),
            "0:v:0".to_string(),
            "-map".to_string(),
            "1:a:0".to_string(),
            "-shortest".to_string(),
            "-y".to_string(),
            output.to_string_lossy().to_string(),
        ];

        Ok(args)
    }

    /// Build ffmpeg args for concat action (videos -> video).
    fn build_concat_args(
        input_files: &[String],
        output: &std::path::Path,
    ) -> Result<Vec<String>> {
        if input_files.len() < 2 {
            anyhow::bail!("concat requires at least 2 input videos");
        }

        let mut args = Vec::new();
        for file in input_files {
            args.push("-i".to_string());
            args.push(file.clone());
        }

        let n = input_files.len();
        let mut filter = String::new();
        for i in 0..n {
            filter.push_str(&format!("[{}:v:0][{}:a:0]", i, i));
        }
        filter.push_str(&format!("concat=n={}:v=1:a=1[outv][outa]", n));

        args.push("-filter_complex".to_string());
        args.push(filter);
        args.push("-map".to_string());
        args.push("[outv]".to_string());
        args.push("-map".to_string());
        args.push("[outa]".to_string());
        args.push("-c:v".to_string());
        args.push("libx264".to_string());
        args.push("-c:a".to_string());
        args.push("aac".to_string());
        args.push("-y".to_string());
        args.push(output.to_string_lossy().to_string());

        Ok(args)
    }

    /// Build ffmpeg args for add_subtitles action (video + srt -> video).
    fn build_subtitles_args(
        input_files: &[String],
        output: &std::path::Path,
    ) -> Result<Vec<String>> {
        if input_files.len() < 2 {
            anyhow::bail!("add_subtitles requires input_files with [video_path, subtitle_path]");
        }

        let video_path = &input_files[0];
        let subtitle_path = &input_files[1];

        // Escape backslashes and colons for ffmpeg subtitles filter on Windows
        let escaped_sub = subtitle_path
            .replace('\\', "/")
            .replace(':', "\\:");

        let args = vec![
            "-i".to_string(),
            video_path.clone(),
            "-vf".to_string(),
            format!("subtitles='{}'", escaped_sub),
            "-c:a".to_string(),
            "copy".to_string(),
            "-y".to_string(),
            output.to_string_lossy().to_string(),
        ];

        Ok(args)
    }

    /// Execute ffmpeg subprocess with the given arguments.
    async fn run_ffmpeg(&self, args: Vec<String>, output: &std::path::Path) -> Result<ToolResult> {
        debug!("Running ffmpeg with {} args, output={}", args.len(), output.display());

        if let Some(parent) = output.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let result = tokio::process::Command::new("ffmpeg")
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let child = match result {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Failed to spawn ffmpeg: {}. Install ffmpeg and ensure it is in PATH.", e),
                });
            }
        };

        let output_result = tokio::time::timeout(
            std::time::Duration::from_secs(600),
            child.wait_with_output(),
        ).await
            .map_err(|_| anyhow::anyhow!("ffmpeg timed out after 600s"))?
            .map_err(|e| anyhow::anyhow!("ffmpeg process error: {}", e))?;

        let stderr = String::from_utf8_lossy(&output_result.stderr).to_string();

        if !output_result.status.success() {
            warn!("ffmpeg failed: {}", truncate(&stderr, 1000));
            return Ok(ToolResult {
                success: false,
                output: format!("ffmpeg failed (exit {}): {}", output_result.status, truncate(&stderr, 1000)),
            });
        }

        // Verify output file exists
        if output.exists() {
            let metadata = std::fs::metadata(output)?;
            Ok(ToolResult {
                success: true,
                output: format!(
                    "Video composed successfully!\nPath: {}\nSize: {} bytes",
                    output.display(), metadata.len()
                ),
            })
        } else {
            Ok(ToolResult {
                success: false,
                output: format!("ffmpeg completed but output file not found at {}", output.display()),
            })
        }
    }
}

#[async_trait]
impl Tool for VideoComposeTool {
    fn name(&self) -> &str {
        "video_compose"
    }

    fn description(&self) -> &str {
        "Compose video from images, audio, and other videos using ffmpeg. Actions: 'slideshow' (images to video), 'overlay_audio' (add audio to video), 'concat' (join videos), 'add_subtitles' (burn subtitles into video)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Action to perform: 'slideshow', 'overlay_audio', 'concat', 'add_subtitles'",
                    "enum": ["slideshow", "overlay_audio", "concat", "add_subtitles"]
                },
                "input_files": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Array of input file paths (images for slideshow, videos for concat, [video, subtitle] for add_subtitles)"
                },
                "audio_path": {
                    "type": "string",
                    "description": "Path to audio file (used with overlay_audio action)"
                },
                "output_path": {
                    "type": "string",
                    "description": "Output video file path. Defaults to ~/.clawtex/workspace/video_{timestamp}.mp4"
                },
                "duration_per_image": {
                    "type": "number",
                    "description": "Duration in seconds for each image in slideshow (default: 5.0)",
                    "default": 5.0
                },
                "resolution": {
                    "type": "string",
                    "description": "Video resolution as WIDTHxHEIGHT (default: '1920x1080')",
                    "default": "1920x1080"
                },
                "fps": {
                    "type": "integer",
                    "description": "Frames per second (default: 30)",
                    "default": 30
                }
            },
            "required": ["action", "input_files"]
        })
    }

    fn preflight(&self, args: &Value) -> Result<()> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        if action.is_empty() {
            anyhow::bail!("Preflight: 'action' is required");
        }

        match action {
            "slideshow" | "overlay_audio" | "concat" | "add_subtitles" => {}
            other => {
                anyhow::bail!(
                    "Preflight: unknown action '{}'. Use 'slideshow', 'overlay_audio', 'concat', or 'add_subtitles'",
                    other
                );
            }
        }

        let input_files = Self::extract_input_files(args);
        if input_files.is_empty() {
            anyhow::bail!("Preflight: 'input_files' must be a non-empty array of file paths");
        }

        if action == "overlay_audio" {
            let audio = args.get("audio_path").and_then(|v| v.as_str()).unwrap_or("");
            if audio.is_empty() {
                anyhow::bail!("Preflight: 'audio_path' is required for overlay_audio action");
            }
        }

        if action == "concat" && input_files.len() < 2 {
            anyhow::bail!("Preflight: concat requires at least 2 input files");
        }

        if action == "add_subtitles" && input_files.len() < 2 {
            anyhow::bail!("Preflight: add_subtitles requires input_files with [video_path, subtitle_path]");
        }

        if let Some(res) = args.get("resolution").and_then(|v| v.as_str()) {
            if Self::parse_resolution(res).is_none() {
                anyhow::bail!("Preflight: invalid resolution '{}'. Use format WIDTHxHEIGHT (e.g., '1920x1080')", res);
            }
        }

        // Check if ffmpeg is available
        let check = std::process::Command::new("ffmpeg")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match check {
            Ok(status) if status.success() => Ok(()),
            _ => anyhow::bail!("Preflight: ffmpeg is not installed or not in PATH. Install from https://ffmpeg.org"),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let action = args.get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if action.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Error: 'action' is required".to_string(),
            });
        }

        let input_files = Self::extract_input_files(&args);
        if input_files.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Error: 'input_files' must be a non-empty array of file paths".to_string(),
            });
        }

        let output_path_str = args.get("output_path").and_then(|v| v.as_str());
        let output = Self::build_output_path(output_path_str);

        let duration_per_image = args.get("duration_per_image")
            .and_then(|v| v.as_f64())
            .unwrap_or(5.0);

        let resolution = args.get("resolution")
            .and_then(|v| v.as_str())
            .unwrap_or("1920x1080");

        let fps = args.get("fps")
            .and_then(|v| v.as_u64())
            .unwrap_or(30) as u32;

        match action {
            "slideshow" => {
                match Self::build_slideshow_args(&input_files, &output, duration_per_image, resolution, fps) {
                    Ok(ffmpeg_args) => self.run_ffmpeg(ffmpeg_args, &output).await,
                    Err(e) => Ok(ToolResult {
                        success: false,
                        output: format!("Error building slideshow command: {}", e),
                    }),
                }
            }
            "overlay_audio" => {
                let audio_path = args.get("audio_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if audio_path.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: "Error: 'audio_path' is required for overlay_audio action".to_string(),
                    });
                }
                match Self::build_overlay_audio_args(&input_files, audio_path, &output) {
                    Ok(ffmpeg_args) => self.run_ffmpeg(ffmpeg_args, &output).await,
                    Err(e) => Ok(ToolResult {
                        success: false,
                        output: format!("Error building overlay_audio command: {}", e),
                    }),
                }
            }
            "concat" => {
                match Self::build_concat_args(&input_files, &output) {
                    Ok(ffmpeg_args) => self.run_ffmpeg(ffmpeg_args, &output).await,
                    Err(e) => Ok(ToolResult {
                        success: false,
                        output: format!("Error building concat command: {}", e),
                    }),
                }
            }
            "add_subtitles" => {
                match Self::build_subtitles_args(&input_files, &output) {
                    Ok(ffmpeg_args) => self.run_ffmpeg(ffmpeg_args, &output).await,
                    Err(e) => Ok(ToolResult {
                        success: false,
                        output: format!("Error building add_subtitles command: {}", e),
                    }),
                }
            }
            other => Ok(ToolResult {
                success: false,
                output: format!("Unknown action '{}'. Use 'slideshow', 'overlay_audio', 'concat', or 'add_subtitles'.", other),
            }),
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        let tool = VideoComposeTool::new();
        assert_eq!(tool.name(), "video_compose");
    }

    #[test]
    fn test_description() {
        let tool = VideoComposeTool::new();
        let desc = tool.description();
        assert!(desc.contains("ffmpeg"), "Description should mention ffmpeg: {}", desc);
        assert!(desc.contains("slideshow"), "Description should mention slideshow: {}", desc);
    }

    #[test]
    fn test_schema() {
        let tool = VideoComposeTool::new();
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"][0], "action");
        assert_eq!(schema["required"][1], "input_files");
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["input_files"].is_object());
        assert!(schema["properties"]["audio_path"].is_object());
        assert!(schema["properties"]["output_path"].is_object());
        assert!(schema["properties"]["duration_per_image"].is_object());
        assert!(schema["properties"]["resolution"].is_object());
        assert!(schema["properties"]["fps"].is_object());
    }

    #[test]
    fn test_parse_resolution_valid() {
        assert_eq!(VideoComposeTool::parse_resolution("1920x1080"), Some((1920, 1080)));
        assert_eq!(VideoComposeTool::parse_resolution("1280x720"), Some((1280, 720)));
        assert_eq!(VideoComposeTool::parse_resolution("3840x2160"), Some((3840, 2160)));
        assert_eq!(VideoComposeTool::parse_resolution("640x480"), Some((640, 480)));
    }

    #[test]
    fn test_parse_resolution_invalid() {
        assert_eq!(VideoComposeTool::parse_resolution("invalid"), None);
        assert_eq!(VideoComposeTool::parse_resolution("1920"), None);
        assert_eq!(VideoComposeTool::parse_resolution("0x0"), None);
        assert_eq!(VideoComposeTool::parse_resolution("99999x99999"), None);
        assert_eq!(VideoComposeTool::parse_resolution("abcxdef"), None);
        assert_eq!(VideoComposeTool::parse_resolution(""), None);
    }

    #[test]
    fn test_output_path_default() {
        let path = VideoComposeTool::build_output_path(None);
        let path_str = path.to_string_lossy().to_string();
        assert!(path_str.contains(".clawtex"), "Path should contain .clawtex: {}", path_str);
        assert!(path_str.contains("workspace"), "Path should contain workspace: {}", path_str);
        assert!(path_str.contains("video_"), "Path should contain video_ prefix: {}", path_str);
        assert!(path_str.ends_with(".mp4"), "Path should end with .mp4: {}", path_str);
    }

    #[test]
    fn test_output_path_custom() {
        let custom = VideoComposeTool::build_output_path(Some("/tmp/my_video.mp4"));
        assert_eq!(custom.to_string_lossy(), "/tmp/my_video.mp4");
    }

    #[test]
    fn test_extract_input_files() {
        let args = json!({"input_files": ["a.png", "b.png", "c.png"]});
        let files = VideoComposeTool::extract_input_files(&args);
        assert_eq!(files, vec!["a.png", "b.png", "c.png"]);
    }

    #[test]
    fn test_extract_input_files_empty() {
        let args = json!({});
        let files = VideoComposeTool::extract_input_files(&args);
        assert!(files.is_empty());

        let args2 = json!({"input_files": []});
        let files2 = VideoComposeTool::extract_input_files(&args2);
        assert!(files2.is_empty());
    }

    #[test]
    fn test_build_slideshow_args() {
        let files = vec!["img1.png".to_string(), "img2.png".to_string()];
        let output = std::path::PathBuf::from("/tmp/out.mp4");
        let args = VideoComposeTool::build_slideshow_args(&files, &output, 5.0, "1920x1080", 30).unwrap();

        // Should contain -loop, -t, -i for each image
        assert!(args.contains(&"-loop".to_string()));
        assert!(args.contains(&"-i".to_string()));
        assert!(args.contains(&"img1.png".to_string()));
        assert!(args.contains(&"img2.png".to_string()));
        assert!(args.contains(&"-filter_complex".to_string()));
        assert!(args.contains(&"-c:v".to_string()));
        assert!(args.contains(&"libx264".to_string()));
        assert!(args.contains(&"-y".to_string()));
        assert!(args.contains(&"/tmp/out.mp4".to_string()));
    }

    #[test]
    fn test_build_slideshow_args_empty_files() {
        let files: Vec<String> = vec![];
        let output = std::path::PathBuf::from("/tmp/out.mp4");
        let result = VideoComposeTool::build_slideshow_args(&files, &output, 5.0, "1920x1080", 30);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("at least one input image"));
    }

    #[test]
    fn test_build_slideshow_args_invalid_resolution() {
        let files = vec!["img1.png".to_string()];
        let output = std::path::PathBuf::from("/tmp/out.mp4");
        let result = VideoComposeTool::build_slideshow_args(&files, &output, 5.0, "bad", 30);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid resolution"));
    }

    #[test]
    fn test_build_overlay_audio_args() {
        let files = vec!["video.mp4".to_string()];
        let output = std::path::PathBuf::from("/tmp/out.mp4");
        let args = VideoComposeTool::build_overlay_audio_args(&files, "audio.mp3", &output).unwrap();

        assert!(args.contains(&"video.mp4".to_string()));
        assert!(args.contains(&"audio.mp3".to_string()));
        assert!(args.contains(&"-shortest".to_string()));
        assert!(args.contains(&"-c:v".to_string()));
        assert!(args.contains(&"copy".to_string()));
        assert!(args.contains(&"-c:a".to_string()));
        assert!(args.contains(&"aac".to_string()));
    }

    #[test]
    fn test_build_overlay_audio_args_no_video() {
        let files: Vec<String> = vec![];
        let output = std::path::PathBuf::from("/tmp/out.mp4");
        let result = VideoComposeTool::build_overlay_audio_args(&files, "audio.mp3", &output);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("at least one input video"));
    }

    #[test]
    fn test_build_concat_args() {
        let files = vec!["v1.mp4".to_string(), "v2.mp4".to_string(), "v3.mp4".to_string()];
        let output = std::path::PathBuf::from("/tmp/out.mp4");
        let args = VideoComposeTool::build_concat_args(&files, &output).unwrap();

        assert!(args.contains(&"v1.mp4".to_string()));
        assert!(args.contains(&"v2.mp4".to_string()));
        assert!(args.contains(&"v3.mp4".to_string()));
        assert!(args.contains(&"-filter_complex".to_string()));

        // Check that the filter contains concat=n=3
        let filter_idx = args.iter().position(|a| a == "-filter_complex").unwrap();
        let filter = &args[filter_idx + 1];
        assert!(filter.contains("concat=n=3:v=1:a=1"), "Filter should contain concat=n=3: {}", filter);
    }

    #[test]
    fn test_build_concat_args_too_few() {
        let files = vec!["v1.mp4".to_string()];
        let output = std::path::PathBuf::from("/tmp/out.mp4");
        let result = VideoComposeTool::build_concat_args(&files, &output);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("at least 2"));
    }

    #[test]
    fn test_build_subtitles_args() {
        let files = vec!["video.mp4".to_string(), "subs.srt".to_string()];
        let output = std::path::PathBuf::from("/tmp/out.mp4");
        let args = VideoComposeTool::build_subtitles_args(&files, &output).unwrap();

        assert!(args.contains(&"video.mp4".to_string()));
        assert!(args.contains(&"-c:a".to_string()));
        assert!(args.contains(&"copy".to_string()));
        // Check that subtitles filter is present
        let has_vf = args.iter().any(|a| a.contains("subtitles="));
        assert!(has_vf, "Should contain subtitles filter");
    }

    #[test]
    fn test_build_subtitles_args_too_few() {
        let files = vec!["video.mp4".to_string()];
        let output = std::path::PathBuf::from("/tmp/out.mp4");
        let result = VideoComposeTool::build_subtitles_args(&files, &output);
        assert!(result.is_err());
    }

    #[test]
    fn test_preflight_missing_action() {
        let tool = VideoComposeTool::new();
        let args = json!({"input_files": ["a.png"]});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("action"));
    }

    #[test]
    fn test_preflight_unknown_action() {
        let tool = VideoComposeTool::new();
        let args = json!({"action": "explode", "input_files": ["a.png"]});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unknown action"), "Error should mention unknown action: {}", err);
        assert!(err.contains("explode"), "Error should include the bad action: {}", err);
    }

    #[test]
    fn test_preflight_empty_input_files() {
        let tool = VideoComposeTool::new();
        let args = json!({"action": "slideshow", "input_files": []});
        let result = tool.preflight(&args);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("non-empty"));
    }

    #[test]
    fn test_preflight_overlay_audio_no_audio_path() {
        let tool = VideoComposeTool::new();
        let args = json!({"action": "overlay_audio", "input_files": ["video.mp4"]});
        let result = tool.preflight(&args);
        // Might fail on ffmpeg check first, but if ffmpeg is present, should fail on audio_path
        if let Err(e) = result {
            let msg = e.to_string();
            assert!(
                msg.contains("audio_path") || msg.contains("ffmpeg"),
                "Error should mention audio_path or ffmpeg: {}", msg
            );
        }
    }

    #[test]
    fn test_preflight_concat_too_few_files() {
        let tool = VideoComposeTool::new();
        let args = json!({"action": "concat", "input_files": ["v1.mp4"]});
        let result = tool.preflight(&args);
        if let Err(e) = result {
            let msg = e.to_string();
            assert!(
                msg.contains("at least 2") || msg.contains("ffmpeg"),
                "Error should mention at least 2 or ffmpeg: {}", msg
            );
        }
    }

    #[test]
    fn test_preflight_invalid_resolution() {
        let tool = VideoComposeTool::new();
        let args = json!({"action": "slideshow", "input_files": ["a.png"], "resolution": "nope"});
        let result = tool.preflight(&args);
        if let Err(e) = result {
            let msg = e.to_string();
            assert!(
                msg.contains("resolution") || msg.contains("ffmpeg"),
                "Error should mention resolution or ffmpeg: {}", msg
            );
        }
    }

    #[tokio::test]
    async fn test_execute_empty_action() {
        let tool = VideoComposeTool::new();
        let result = tool.execute(json!({"action": "", "input_files": ["a.png"]})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("required"));
    }

    #[tokio::test]
    async fn test_execute_empty_input_files() {
        let tool = VideoComposeTool::new();
        let result = tool.execute(json!({"action": "slideshow", "input_files": []})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("non-empty"));
    }

    #[tokio::test]
    async fn test_execute_unknown_action() {
        let tool = VideoComposeTool::new();
        let result = tool.execute(json!({"action": "dance", "input_files": ["a.png"]})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_execute_overlay_audio_missing_audio() {
        let tool = VideoComposeTool::new();
        let result = tool.execute(json!({
            "action": "overlay_audio",
            "input_files": ["video.mp4"]
        })).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("audio_path"));
    }

    #[test]
    fn test_truncate_fn() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello...");
        assert_eq!(truncate("", 5), "");
    }

    #[test]
    fn test_slideshow_filter_complex_content() {
        let files = vec!["a.png".to_string(), "b.png".to_string()];
        let output = std::path::PathBuf::from("/tmp/out.mp4");
        let args = VideoComposeTool::build_slideshow_args(&files, &output, 3.0, "1280x720", 24).unwrap();

        let filter_idx = args.iter().position(|a| a == "-filter_complex").unwrap();
        let filter = &args[filter_idx + 1];

        // Check scale dimensions
        assert!(filter.contains("scale=1280:720"), "Filter should scale to 1280x720: {}", filter);
        // Check fps
        assert!(filter.contains("fps=24"), "Filter should set fps=24: {}", filter);
        // Check concat n=2
        assert!(filter.contains("concat=n=2:v=1:a=0"), "Filter should concat n=2: {}", filter);
    }

    #[test]
    fn test_slideshow_duration_in_args() {
        let files = vec!["a.png".to_string()];
        let output = std::path::PathBuf::from("/tmp/out.mp4");
        let args = VideoComposeTool::build_slideshow_args(&files, &output, 7.5, "1920x1080", 30).unwrap();

        // Check that -t 7.5 is in the args
        let t_idx = args.iter().position(|a| a == "-t").unwrap();
        assert_eq!(args[t_idx + 1], "7.5");
    }
}
