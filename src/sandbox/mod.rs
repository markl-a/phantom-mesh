// Sandbox controllers for Computer Use — virtual desktop environments
// Provides trait abstraction over Docker, Windows Sandbox, and MCP backends

pub mod docker;
pub mod wasm;

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;

/// Abstract interface for controlling a sandboxed desktop environment
#[async_trait]
pub trait SandboxController: Send + Sync {
    /// Capture a screenshot of the virtual display (returns PNG bytes)
    async fn screenshot(&self) -> Result<Vec<u8>>;

    /// Perform a mouse click at coordinates
    /// action: "left_click", "right_click", "double_click", "middle_click", "triple_click"
    async fn click(&self, action: &str, x: u32, y: u32) -> Result<()>;

    /// Type text string
    async fn type_text(&self, text: &str) -> Result<()>;

    /// Press a key or key combination (e.g., "ctrl+s", "Return", "space")
    async fn key_press(&self, key: &str) -> Result<()>;

    /// Scroll at optional coordinates in a direction
    async fn scroll(&self, x: u32, y: u32, direction: &str, amount: u32) -> Result<()>;

    /// Move mouse to coordinates
    async fn mouse_move(&self, x: u32, y: u32) -> Result<()>;

    /// Click and drag from current position to target
    async fn drag(&self, start_x: u32, start_y: u32, end_x: u32, end_y: u32) -> Result<()>;

    /// Execute a shell command inside the sandbox
    async fn execute_command(&self, cmd: &str) -> Result<String>;

    /// Get the display resolution
    fn display_size(&self) -> (u32, u32);

    /// Ensure the sandbox environment is running (start if needed)
    /// Default: no-op (override in backends that manage container lifecycle)
    async fn ensure_running(&self) -> Result<()> {
        Ok(())
    }
}

/// Configuration for human-like behavior (anti-detection)
#[derive(Debug, Clone, Deserialize)]
pub struct HumanLikeConfig {
    /// Enable human-like delays and mouse movement
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Min delay between actions in milliseconds
    #[serde(default = "default_min_delay")]
    pub min_delay_ms: u64,
    /// Max delay between actions in milliseconds
    #[serde(default = "default_max_delay")]
    pub max_delay_ms: u64,
    /// Min typing delay per character in milliseconds
    #[serde(default = "default_min_typing")]
    pub min_typing_delay_ms: u64,
    /// Max typing delay per character in milliseconds
    #[serde(default = "default_max_typing")]
    pub max_typing_delay_ms: u64,
    /// Random pixel offset for clicks (±N pixels)
    #[serde(default = "default_click_offset")]
    pub click_offset_px: u32,
}

fn default_true() -> bool { true }
fn default_min_delay() -> u64 { 100 }
fn default_max_delay() -> u64 { 500 }
fn default_min_typing() -> u64 { 30 }
fn default_max_typing() -> u64 { 80 }
fn default_click_offset() -> u32 { 3 }

impl Default for HumanLikeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_delay_ms: default_min_delay(),
            max_delay_ms: default_max_delay(),
            min_typing_delay_ms: default_min_typing(),
            max_typing_delay_ms: default_max_typing(),
            click_offset_px: default_click_offset(),
        }
    }
}

/// Generate a random delay between min and max milliseconds
pub fn random_delay(min_ms: u64, max_ms: u64) -> std::time::Duration {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    let range = max_ms.saturating_sub(min_ms).max(1);
    let delay = min_ms + (seed % range);
    std::time::Duration::from_millis(delay)
}

/// Generate a random offset within ±max pixels
pub fn random_offset(max_px: u32) -> i32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    if max_px == 0 { return 0; }
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let range = (max_px * 2 + 1) as i32;
    (seed as i32 % range) - max_px as i32
}

/// Calculate screenshot scale factor per Anthropic constraints
/// Max 1568px longest edge, ~1.15 megapixels
pub fn screenshot_scale_factor(width: u32, height: u32) -> f64 {
    let long_edge = width.max(height) as f64;
    let total_pixels = (width as f64) * (height as f64);

    let long_edge_scale = 1568.0 / long_edge;
    let total_pixels_scale = (1_150_000.0 / total_pixels).sqrt();

    long_edge_scale.min(total_pixels_scale).min(1.0)
}
