// computer_use tool — operate GUI applications via Claude's Computer Use API
// Runs an agentic loop: screenshot → Claude → action → execute → screenshot → repeat
// Uses a SandboxController backend (Docker or Windows Sandbox)

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

use super::{Tool, ToolResult};
use crate::sandbox::{SandboxController, HumanLikeConfig, screenshot_scale_factor};
use crate::sandbox::docker::{DockerSandbox, DockerSandboxConfig};

/// Configuration for computer_use tool (from [computer_use] in agents.toml)
#[derive(Debug, Clone, Deserialize)]
pub struct ComputerUseConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_sandbox")]
    pub sandbox: String,
    #[serde(default)]
    pub anthropic_api_key: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_width")]
    pub display_width: u32,
    #[serde(default = "default_height")]
    pub display_height: u32,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub human_like: Option<HumanLikeConfig>,
    #[serde(default)]
    pub docker: Option<DockerSandboxConfig>,
    /// Provider: "claude" (API), "local" (Ollama vision model), "auto" (local with Claude fallback)
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Ollama endpoint URL for local vision model
    #[serde(default = "default_local_url")]
    pub local_url: String,
    /// Vision model name in Ollama (e.g., "qwen3.5:27b", "qwen3-vl:32b")
    #[serde(default = "default_local_model")]
    pub local_model: String,
}

fn default_true() -> bool { true }
fn default_sandbox() -> String { "docker".to_string() }
fn default_model() -> String { "claude-sonnet-4-6".to_string() }
fn default_width() -> u32 { 1024 }
fn default_height() -> u32 { 768 }
fn default_max_iterations() -> u32 { 50 }
fn default_timeout() -> u64 { 300 }
fn default_provider() -> String { "claude".to_string() }
fn default_local_url() -> String { "http://localhost:11434".to_string() }
fn default_local_model() -> String { "qwen3.5:27b".to_string() }

impl Default for ComputerUseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sandbox: default_sandbox(),
            anthropic_api_key: String::new(),
            model: default_model(),
            display_width: default_width(),
            display_height: default_height(),
            max_iterations: default_max_iterations(),
            timeout_secs: default_timeout(),
            human_like: None,
            docker: None,
            provider: default_provider(),
            local_url: default_local_url(),
            local_model: default_local_model(),
        }
    }
}

/// System prompt for local vision models (Ollama + UI-TARS / Qwen2.5-VL / etc.)
const LOCAL_VISION_SYSTEM_PROMPT: &str = r#"You are a GUI automation agent controlling a desktop computer. You will see screenshots and must respond with exactly ONE JSON action to perform.

Available actions:
{"action": "left_click", "coordinate": [x, y]}
{"action": "right_click", "coordinate": [x, y]}
{"action": "double_click", "coordinate": [x, y]}
{"action": "type", "text": "text to type"}
{"action": "key", "text": "ctrl+s"}
{"action": "scroll", "coordinate": [x, y], "scroll_direction": "down", "scroll_amount": 3}
{"action": "mouse_move", "coordinate": [x, y]}
{"action": "drag", "start": [x1, y1], "end": [x2, y2]}
{"action": "screenshot"}
{"action": "wait"}
{"action": "done", "summary": "what was accomplished"}

Rules:
- Coordinates are pixel positions in the screenshot image
- Output ONLY the JSON action, no other text
- Perform one action at a time
- Use "done" when the task is complete
- Use "screenshot" if you need to see the screen without acting"#;

pub struct ComputerUseTool {
    config: ComputerUseConfig,
    sandbox: Box<dyn SandboxController>,
    http_client: reqwest::Client,
    api_key: String,
    scale_factor: f64,
}

impl ComputerUseTool {
    pub fn new(config: ComputerUseConfig) -> Self {
        // Resolve API key: config > env var
        let api_key = if !config.anthropic_api_key.is_empty() {
            config.anthropic_api_key.clone()
        } else {
            std::env::var("ANTHROPIC_API_KEY").unwrap_or_default()
        };

        if api_key.is_empty() && config.provider != "local" {
            warn!("computer_use: No ANTHROPIC_API_KEY configured — Claude provider will fail at runtime");
        }

        let human_like = config.human_like.clone().unwrap_or_default();
        let docker_config = config.docker.clone().unwrap_or_default();

        let sandbox: Box<dyn SandboxController> = match config.sandbox.as_str() {
            "docker" => Box::new(DockerSandbox::new(
                docker_config,
                human_like,
                config.display_width,
                config.display_height,
            )),
            other => {
                warn!("computer_use: Unknown sandbox '{}', defaulting to docker", other);
                Box::new(DockerSandbox::new(
                    docker_config,
                    human_like,
                    config.display_width,
                    config.display_height,
                ))
            }
        };

        let scale_factor = screenshot_scale_factor(config.display_width, config.display_height);
        match config.provider.as_str() {
            "local" => info!(
                "computer_use: provider=local, model={}, url={}, sandbox={}, display={}x{}",
                config.local_model, config.local_url, config.sandbox, config.display_width, config.display_height
            ),
            "auto" => info!(
                "computer_use: provider=auto, local_model={}, claude_model={}, sandbox={}, display={}x{}",
                config.local_model, config.model, config.sandbox, config.display_width, config.display_height
            ),
            _ => info!(
                "computer_use: provider=claude, model={}, sandbox={}, display={}x{}, scale={:.3}",
                config.model, config.sandbox, config.display_width, config.display_height, scale_factor
            ),
        }

        let http_timeout = match config.provider.as_str() {
            "local" | "auto" => 180, // vision models can be slow on first call
            _ => 120,
        };
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(http_timeout))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            config,
            sandbox,
            http_client,
            api_key,
            scale_factor,
        }
    }

    /// Call Claude API with computer_20251124 tool
    async fn call_claude(&self, messages: &[Value]) -> Result<Value> {
        let (width, height) = self.sandbox.display_size();
        let scaled_w = (width as f64 * self.scale_factor) as u32;
        let scaled_h = (height as f64 * self.scale_factor) as u32;

        let body = json!({
            "model": self.config.model,
            "max_tokens": 4096,
            "tools": [{
                "type": "computer_20251124",
                "name": "computer",
                "display_width_px": scaled_w,
                "display_height_px": scaled_h
            }],
            "messages": messages
        });

        let resp = self.http_client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "computer-use-2025-11-24")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Failed to call Claude API")?;

        let status = resp.status();
        let response_text = resp.text().await.context("Failed to read Claude response")?;

        if !status.is_success() {
            anyhow::bail!("Claude API returned {}: {}", status, &response_text[..response_text.len().min(500)]);
        }

        serde_json::from_str(&response_text).context("Failed to parse Claude response JSON")
    }

    /// Execute a single action from Claude's response
    async fn execute_action(&self, action: &Value) -> Result<()> {
        let action_type = action.get("action").and_then(|v| v.as_str()).unwrap_or("");

        match action_type {
            "screenshot" => {
                // No action needed — screenshot is taken after every action
                debug!("action: screenshot (no-op, will capture after)");
            }
            "left_click" | "right_click" | "middle_click" | "double_click" | "triple_click" => {
                let (x, y) = self.extract_coordinate(action)?;
                let (sx, sy) = self.scale_up(x, y);
                self.sandbox.click(action_type, sx, sy).await?;
            }
            "type" => {
                let text = action.get("text").and_then(|v| v.as_str()).unwrap_or("");
                self.sandbox.type_text(text).await?;
            }
            "key" => {
                let key = action.get("text").and_then(|v| v.as_str()).unwrap_or("");
                self.sandbox.key_press(key).await?;
            }
            "scroll" => {
                let (x, y) = self.extract_coordinate(action)?;
                let (sx, sy) = self.scale_up(x, y);
                let direction = action.get("scroll_direction").and_then(|v| v.as_str()).unwrap_or("down");
                let amount = action.get("scroll_amount").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
                self.sandbox.scroll(sx, sy, direction, amount).await?;
            }
            "mouse_move" => {
                let (x, y) = self.extract_coordinate(action)?;
                let (sx, sy) = self.scale_up(x, y);
                self.sandbox.mouse_move(sx, sy).await?;
            }
            "left_click_drag" => {
                let (x, y) = self.extract_coordinate(action)?;
                let (sx, sy) = self.scale_up(x, y);
                // start_coordinate for drag start
                if let Some(start) = action.get("start_coordinate") {
                    let start_x = start.get(0).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    let start_y = start.get(1).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    let (ssx, ssy) = self.scale_up(start_x, start_y);
                    self.sandbox.drag(ssx, ssy, sx, sy).await?;
                } else {
                    // coordinate is the end point, drag from current position
                    self.sandbox.drag(sx, sy, sx, sy).await?;
                }
            }
            "wait" => {
                // Claude requests a pause
                let duration = action.get("duration").and_then(|v| v.as_f64()).unwrap_or(1.0);
                let ms = (duration * 1000.0).min(10000.0) as u64; // cap at 10s
                tokio::time::sleep(Duration::from_millis(ms)).await;
                debug!("action: wait {}ms", ms);
            }
            other => {
                warn!("computer_use: Unknown action '{}', skipping", other);
            }
        }
        Ok(())
    }

    /// Extract coordinate [x, y] from action
    fn extract_coordinate(&self, action: &Value) -> Result<(u32, u32)> {
        let coord = action.get("coordinate")
            .ok_or_else(|| anyhow::anyhow!("Action missing 'coordinate' field"))?;
        let x = coord.get(0).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let y = coord.get(1).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        Ok((x, y))
    }

    /// Scale Claude's coordinates back to actual display resolution
    fn scale_up(&self, x: u32, y: u32) -> (u32, u32) {
        if (self.scale_factor - 1.0).abs() < 0.001 {
            return (x, y);
        }
        let real_x = (x as f64 / self.scale_factor).round() as u32;
        let real_y = (y as f64 / self.scale_factor).round() as u32;
        let (w, h) = self.sandbox.display_size();
        (real_x.min(w.saturating_sub(1)), real_y.min(h.saturating_sub(1)))
    }

    /// Encode screenshot as base64 PNG.
    /// For local provider, resize to half resolution for faster inference.
    fn encode_screenshot(&self, png_bytes: &[u8]) -> Result<String> {
        use base64::Engine;
        Ok(base64::engine::general_purpose::STANDARD.encode(png_bytes))
    }

    /// Capture screenshot resized to 512x384 for local vision model.
    /// Uses scrot + imagemagick inside the container, reads binary via docker exec.
    /// Falls back to full-size screenshot if resize fails.
    async fn capture_for_local(&self) -> Result<String> {
        let container = self.sandbox_container_name();

        // Step 1: capture + resize inside the container
        let resize_result = self.sandbox.execute_command(
            "DISPLAY=:1 scrot -o /tmp/_cu_shot.png && convert /tmp/_cu_shot.png -resize 512x384 -quality 85 /tmp/_cu_small.png && echo OK"
        ).await;

        if resize_result.is_err() {
            debug!("computer_use [local]: resize failed, falling back to full screenshot");
            let full = self.sandbox.screenshot().await?;
            return self.encode_screenshot(&full);
        }

        // Step 2: read the resized PNG as raw bytes via docker exec
        let output = tokio::process::Command::new("docker")
            .args(["exec", &container, "bash", "-c", "cat /tmp/_cu_small.png"])
            .output()
            .await
            .context("Failed to read resized screenshot")?;

        if !output.status.success() || output.stdout.is_empty() {
            debug!("computer_use [local]: binary read failed, falling back to full screenshot");
            let full = self.sandbox.screenshot().await?;
            return self.encode_screenshot(&full);
        }

        use base64::Engine;
        Ok(base64::engine::general_purpose::STANDARD.encode(&output.stdout))
    }

    fn sandbox_container_name(&self) -> String {
        self.config.docker.as_ref()
            .map(|d| d.container_name.clone())
            .unwrap_or_else(|| "phantom-mesh-sandbox".to_string())
    }

    /// Call local vision model via Ollama /api/chat endpoint
    async fn call_ollama(&self, messages: &[Value]) -> Result<String> {
        let url = format!("{}/api/chat", self.config.local_url.trim_end_matches('/'));

        let body = json!({
            "model": self.config.local_model,
            "messages": messages,
            "stream": false,
            "options": {
                "num_predict": 256,    // actions are short JSON — keep tight for speed
                "temperature": 0.1     // near-deterministic for reliable JSON
            }
        });

        let resp = self.http_client
            .post(&url)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Failed to call Ollama API")?;

        let status = resp.status();
        let response_text = resp.text().await.context("Failed to read Ollama response")?;

        if !status.is_success() {
            anyhow::bail!("Ollama API returned {}: {}", status, &response_text[..response_text.len().min(500)]);
        }

        let response: Value = serde_json::from_str(&response_text)
            .context("Failed to parse Ollama response JSON")?;

        // Ollama /api/chat returns: {"message": {"role": "assistant", "content": "..."}, ...}
        let content = response
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        Ok(content)
    }

    /// Parse a JSON action from local model response text
    /// Handles cases where model wraps JSON in markdown code blocks or adds extra text
    fn parse_local_action(&self, response: &str) -> Result<Value> {
        let trimmed = response.trim();

        // Try parsing directly first
        if let Ok(action) = serde_json::from_str::<Value>(trimmed) {
            if action.get("action").is_some() {
                return Ok(action);
            }
        }

        // Try extracting JSON from markdown code block: ```json ... ```
        if let Some(start) = trimmed.find('{') {
            if let Some(end) = trimmed.rfind('}') {
                let json_str = &trimmed[start..=end];
                if let Ok(action) = serde_json::from_str::<Value>(json_str) {
                    if action.get("action").is_some() {
                        return Ok(action);
                    }
                }
            }
        }

        let preview: String = trimmed.chars().take(200).collect();
        anyhow::bail!(
            "Could not parse action JSON from model response: {}",
            preview
        )
    }

    /// Run agentic loop using local vision model (Ollama)
    ///
    /// Performance optimizations:
    /// - Screenshots resized to 512x384 (4x fewer image tokens)
    /// - Sliding window: only keep last 2 screenshot rounds (prevents context bloat)
    /// - Short num_predict since actions are tiny JSON
    async fn run_local_loop(&self, task: &str) -> Result<String> {
        let start = Instant::now();
        let timeout = Duration::from_secs(self.config.timeout_secs);

        // Ensure sandbox is running
        self.sandbox.ensure_running().await?;

        // Capture resized screenshot for faster inference
        let b64 = self.capture_for_local().await
            .unwrap_or_else(|_| {
                warn!("computer_use [local]: resize capture failed, using full screenshot");
                String::new()
            });

        // If resize failed, fallback to full screenshot
        let b64 = if b64.is_empty() {
            let full = self.sandbox.screenshot().await
                .context("Failed to capture initial screenshot")?;
            self.encode_screenshot(&full)?
        } else {
            b64
        };

        // Track action history as compact text (no images) for context
        let mut action_log: Vec<String> = Vec::new();

        for iteration in 0..self.config.max_iterations {
            // Check timeout
            if start.elapsed() > timeout {
                warn!("computer_use [local]: Timeout after {}s ({} iterations)", self.config.timeout_secs, iteration);
                return Ok(format!("[Timeout after {}s, {} iterations]", self.config.timeout_secs, iteration));
            }

            debug!("computer_use [local]: iteration {}/{}", iteration + 1, self.config.max_iterations);

            // Build minimal messages each round: system + action_log + current screenshot
            // This prevents context from growing with accumulated images
            let mut messages: Vec<Value> = vec![
                json!({
                    "role": "system",
                    "content": LOCAL_VISION_SYSTEM_PROMPT
                })
            ];

            // Include compact action history (text only, no images)
            let history_text = if action_log.is_empty() {
                format!("Task: {}", task)
            } else {
                format!("Task: {}\n\nActions completed so far:\n{}\n\nContinue with the next action.",
                    task, action_log.join("\n"))
            };

            // Current screenshot — only ONE image per request
            let current_b64 = if iteration == 0 {
                b64.clone()
            } else {
                self.capture_for_local().await.unwrap_or_else(|e| {
                    warn!("computer_use [local]: screenshot failed: {}", e);
                    String::new()
                })
            };

            if current_b64.is_empty() {
                messages.push(json!({
                    "role": "user",
                    "content": format!("{}\n\n(Screenshot capture failed, please try an action based on previous context)", history_text)
                }));
            } else {
                messages.push(json!({
                    "role": "user",
                    "content": history_text,
                    "images": [current_b64]
                }));
            }

            // Call local vision model
            let response_text = self.call_ollama(&messages).await?;

            let resp_preview: String = response_text.chars().take(200).collect();
            debug!("computer_use [local]: model response: {}", resp_preview);

            // Parse action from response
            let action = match self.parse_local_action(&response_text) {
                Ok(a) => a,
                Err(e) => {
                    warn!("computer_use [local]: Failed to parse action (iter {}): {}", iteration + 1, e);
                    action_log.push(format!("- [{}] (parse error, retrying)", iteration + 1));
                    continue;
                }
            };

            let action_type = action.get("action").and_then(|v| v.as_str()).unwrap_or("");

            info!("computer_use [local]: [{}] action={} {}",
                iteration + 1, action_type,
                if action_type != "screenshot" && action_type != "done" {
                    serde_json::to_string(&action).unwrap_or_default()
                } else {
                    String::new()
                }
            );

            // Log action compactly
            action_log.push(format!("- [{}] {}", iteration + 1,
                serde_json::to_string(&action).unwrap_or_default()));

            // Check for completion
            if action_type == "done" || action_type == "finished" {
                let summary = action.get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Task completed.")
                    .to_string();
                info!("computer_use [local]: Task complete after {} iterations ({:.1}s)",
                    iteration + 1, start.elapsed().as_secs_f64());
                return Ok(summary);
            }

            // Execute the action
            if let Err(e) = self.execute_action(&action).await {
                error!("computer_use [local]: Action '{}' failed: {}", action_type, e);
                let last = action_log.last_mut().unwrap();
                *last = format!("{} FAILED: {}", last, e);
                continue;
            }
        }

        warn!("computer_use [local]: Max iterations ({}) reached", self.config.max_iterations);
        Ok(format!("[Reached max {} iterations]", self.config.max_iterations))
    }

    /// Check if Ollama is reachable (for "auto" provider)
    async fn is_ollama_available(&self) -> bool {
        let url = format!("{}/api/tags", self.config.local_url.trim_end_matches('/'));
        match self.http_client.get(&url).timeout(Duration::from_secs(3)).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    /// Run the full agentic loop (Claude API provider)
    async fn run_agent_loop(&self, task: &str) -> Result<String> {
        let start = Instant::now();
        let timeout = Duration::from_secs(self.config.timeout_secs);

        // Ensure sandbox is running
        self.sandbox.ensure_running().await?;

        let mut messages: Vec<Value> = vec![
            json!({
                "role": "user",
                "content": task
            })
        ];

        let mut final_text = String::new();

        for iteration in 0..self.config.max_iterations {
            // Check timeout
            if start.elapsed() > timeout {
                warn!("computer_use: Timeout after {}s ({} iterations)", self.config.timeout_secs, iteration);
                return Ok(format!(
                    "{}\n\n[Timeout after {}s, {} iterations]",
                    final_text, self.config.timeout_secs, iteration
                ));
            }

            debug!("computer_use: iteration {}/{}", iteration + 1, self.config.max_iterations);

            // Call Claude API
            let response = self.call_claude(&messages).await?;

            // Extract content blocks from response
            let content = response.get("content")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let stop_reason = response.get("stop_reason")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // Add assistant response to conversation
            messages.push(json!({
                "role": "assistant",
                "content": content
            }));

            // Process each content block
            let mut tool_results: Vec<Value> = Vec::new();

            for block in &content {
                let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");

                match block_type {
                    "text" => {
                        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                            final_text = text.to_string();
                            debug!("computer_use: text response: {}...",
                                &text[..text.len().min(100)]);
                        }
                    }
                    "tool_use" => {
                        let tool_use_id = block.get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let input = block.get("input").cloned().unwrap_or(json!({}));
                        let action_type = input.get("action")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");

                        info!("computer_use: [{}] action={} {}",
                            iteration + 1, action_type,
                            if action_type != "screenshot" {
                                format!("{}", serde_json::to_string(&input).unwrap_or_default())
                            } else {
                                String::new()
                            }
                        );

                        // Execute the action
                        if let Err(e) = self.execute_action(&input).await {
                            error!("computer_use: Action '{}' failed: {}", action_type, e);
                            tool_results.push(json!({
                                "type": "tool_result",
                                "tool_use_id": tool_use_id,
                                "content": format!("Error: {}", e),
                                "is_error": true
                            }));
                            continue;
                        }

                        // Take screenshot after action and send back
                        match self.sandbox.screenshot().await {
                            Ok(png_bytes) => {
                                let b64 = self.encode_screenshot(&png_bytes)?;
                                tool_results.push(json!({
                                    "type": "tool_result",
                                    "tool_use_id": tool_use_id,
                                    "content": [{
                                        "type": "image",
                                        "source": {
                                            "type": "base64",
                                            "media_type": "image/png",
                                            "data": b64
                                        }
                                    }]
                                }));
                            }
                            Err(e) => {
                                error!("computer_use: Screenshot failed: {}", e);
                                tool_results.push(json!({
                                    "type": "tool_result",
                                    "tool_use_id": tool_use_id,
                                    "content": format!("Screenshot failed: {}", e),
                                    "is_error": true
                                }));
                            }
                        }
                    }
                    _ => {
                        // thinking blocks, etc. — just log
                        debug!("computer_use: block type '{}' (ignored)", block_type);
                    }
                }
            }

            // If no tool_use blocks, Claude is done
            if tool_results.is_empty() || stop_reason == "end_turn" {
                info!("computer_use: Task complete after {} iterations ({:.1}s)",
                    iteration + 1, start.elapsed().as_secs_f64());
                return Ok(final_text);
            }

            // Send tool results back to Claude
            messages.push(json!({
                "role": "user",
                "content": tool_results
            }));
        }

        warn!("computer_use: Max iterations ({}) reached", self.config.max_iterations);
        Ok(format!(
            "{}\n\n[Reached max {} iterations]",
            final_text, self.config.max_iterations
        ))
    }
}

#[async_trait]
impl Tool for ComputerUseTool {
    fn name(&self) -> &str { "computer_use" }

    fn description(&self) -> &str {
        "Operate GUI applications through a virtual desktop. Claude controls the screen via screenshot-analyze-click loops. Use for tasks that require visual interaction with desktop apps (e.g., Antigravity IDE, browsers, design tools)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Description of the GUI task to perform (e.g., 'Open Firefox and search for cats')"
                },
                "setup_commands": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional shell commands to run in the sandbox before starting (e.g., launch an app)"
                }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        // Determine effective provider
        let provider = match self.config.provider.as_str() {
            "auto" => {
                if self.is_ollama_available().await {
                    info!("computer_use: auto provider → using local (Ollama reachable)");
                    "local"
                } else if !self.api_key.is_empty() {
                    info!("computer_use: auto provider → using claude (Ollama unreachable)");
                    "claude"
                } else {
                    return Ok(ToolResult {
                        success: false,
                        output: "Error: auto provider failed — Ollama unreachable and no ANTHROPIC_API_KEY configured.".to_string(),
                    });
                }
            }
            "local" => "local",
            _ => "claude",
        };

        // Validate API key for Claude provider
        if provider == "claude" && self.api_key.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Error: ANTHROPIC_API_KEY not configured. Set it in [computer_use] config or as an environment variable.".to_string(),
            });
        }

        // Extract task
        let task = args.get("task").and_then(|v| v.as_str()).unwrap_or("");
        if task.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Error: 'task' is required and must not be empty".to_string(),
            });
        }

        info!("computer_use: Starting task (provider={}): {}...", provider, &task[..task.len().min(100)]);

        // Run optional setup commands
        if let Some(setup) = args.get("setup_commands").and_then(|v| v.as_array()) {
            for cmd in setup {
                if let Some(cmd_str) = cmd.as_str() {
                    debug!("computer_use: setup command: {}", cmd_str);
                    match self.sandbox.execute_command(cmd_str).await {
                        Ok(out) => debug!("computer_use: setup output: {}", out.trim()),
                        Err(e) => warn!("computer_use: setup command failed: {}", e),
                    }
                }
            }
        }

        // Run the agentic loop with the appropriate provider
        let loop_result = match provider {
            "local" => self.run_local_loop(task).await,
            _ => self.run_agent_loop(task).await,
        };

        match loop_result {
            Ok(result) => {
                let output = if result.is_empty() {
                    "Task completed (no text response).".to_string()
                } else {
                    result
                };
                Ok(ToolResult {
                    success: true,
                    output,
                })
            }
            Err(e) => {
                error!("computer_use: Task failed: {}", e);
                Ok(ToolResult {
                    success: false,
                    output: format!("Computer use failed: {}", e),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ComputerUseConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.sandbox, "docker");
        assert_eq!(config.model, "claude-sonnet-4-6");
        assert_eq!(config.display_width, 1024);
        assert_eq!(config.display_height, 768);
        assert_eq!(config.max_iterations, 50);
        assert_eq!(config.timeout_secs, 300);
    }

    #[test]
    fn test_scale_factor_1024x768() {
        let scale = screenshot_scale_factor(1024, 768);
        // 1024x768 = 786432 pixels, longest edge 1024
        // long_edge_scale = 1568/1024 = 1.53
        // pixel_scale = sqrt(1150000/786432) = 1.21
        // min(1.0, 1.53, 1.21) = 1.0
        assert!((scale - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_scale_factor_1920x1080() {
        let scale = screenshot_scale_factor(1920, 1080);
        // 1920x1080 = 2073600 pixels, longest edge 1920
        // long_edge_scale = 1568/1920 = 0.817
        // pixel_scale = sqrt(1150000/2073600) = 0.745
        // min(1.0, 0.817, 0.745) = 0.745
        assert!(scale < 0.8);
        assert!(scale > 0.7);
    }

    #[test]
    fn test_scale_factor_2560x1440() {
        let scale = screenshot_scale_factor(2560, 1440);
        // Should be less than 0.7
        assert!(scale < 0.7);
    }

    #[test]
    fn test_default_config_provider() {
        let config = ComputerUseConfig::default();
        assert_eq!(config.provider, "claude");
        assert_eq!(config.local_url, "http://localhost:11434");
        assert_eq!(config.local_model, "qwen3.5:27b");
    }

    #[test]
    fn test_parse_local_action_direct_json() {
        let config = ComputerUseConfig { provider: "local".to_string(), ..Default::default() };
        let tool = ComputerUseTool::new(config);
        let action = tool.parse_local_action(r#"{"action": "left_click", "coordinate": [100, 200]}"#).unwrap();
        assert_eq!(action.get("action").unwrap().as_str().unwrap(), "left_click");
        let coord = action.get("coordinate").unwrap().as_array().unwrap();
        assert_eq!(coord[0].as_u64().unwrap(), 100);
        assert_eq!(coord[1].as_u64().unwrap(), 200);
    }

    #[test]
    fn test_parse_local_action_with_markdown() {
        let config = ComputerUseConfig { provider: "local".to_string(), ..Default::default() };
        let tool = ComputerUseTool::new(config);
        let response = "Here's my action:\n```json\n{\"action\": \"type\", \"text\": \"hello world\"}\n```";
        let action = tool.parse_local_action(response).unwrap();
        assert_eq!(action.get("action").unwrap().as_str().unwrap(), "type");
        assert_eq!(action.get("text").unwrap().as_str().unwrap(), "hello world");
    }

    #[test]
    fn test_parse_local_action_done() {
        let config = ComputerUseConfig { provider: "local".to_string(), ..Default::default() };
        let tool = ComputerUseTool::new(config);
        let action = tool.parse_local_action(r#"{"action": "done", "summary": "Task completed successfully"}"#).unwrap();
        assert_eq!(action.get("action").unwrap().as_str().unwrap(), "done");
        assert_eq!(action.get("summary").unwrap().as_str().unwrap(), "Task completed successfully");
    }

    #[test]
    fn test_parse_local_action_invalid() {
        let config = ComputerUseConfig { provider: "local".to_string(), ..Default::default() };
        let tool = ComputerUseTool::new(config);
        assert!(tool.parse_local_action("not valid json at all").is_err());
        assert!(tool.parse_local_action("{}").is_err()); // missing "action" field
    }
}
