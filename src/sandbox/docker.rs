// Docker sandbox backend — uses `docker exec` + xdotool/scrot for GUI control
// Container: Ubuntu 22.04 + Xvfb + Xfce + xdotool + scrot + noVNC

use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use tokio::process::Command;
use tracing::{debug, warn};

/// Maximum execution time for any single docker exec command (seconds).
const DOCKER_EXEC_TIMEOUT_SECS: u64 = 60;

use super::{HumanLikeConfig, SandboxController, random_delay, random_offset};

/// Validate that a string is safe for use as an xdotool key name.
/// Allows alphanumeric, plus, underscore, hyphen only (e.g. "ctrl+s", "Return", "super+Left").
fn is_safe_key_name(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 64
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '_' || c == '-')
}

/// Docker sandbox configuration (from [computer_use.docker] in agents.toml)
#[derive(Debug, Clone, Deserialize)]
pub struct DockerSandboxConfig {
    #[serde(default = "default_image")]
    pub image: String,
    #[serde(default = "default_container_name")]
    pub container_name: String,
    #[serde(default = "default_vnc_port")]
    pub vnc_port: u16,
    #[serde(default = "default_novnc_port")]
    pub novnc_port: u16,
}

fn default_image() -> String { "phantom-mesh-sandbox:latest".to_string() }
fn default_container_name() -> String { "phantom-mesh-sandbox".to_string() }
fn default_vnc_port() -> u16 { 5900 }
fn default_novnc_port() -> u16 { 6080 }

impl Default for DockerSandboxConfig {
    fn default() -> Self {
        Self {
            image: default_image(),
            container_name: default_container_name(),
            vnc_port: default_vnc_port(),
            novnc_port: default_novnc_port(),
        }
    }
}

pub struct DockerSandbox {
    config: DockerSandboxConfig,
    human_like: HumanLikeConfig,
    display_width: u32,
    display_height: u32,
}

impl DockerSandbox {
    pub fn new(config: DockerSandboxConfig, human_like: HumanLikeConfig, width: u32, height: u32) -> Self {
        Self {
            config,
            human_like,
            display_width: width,
            display_height: height,
        }
    }

    /// Run a command inside the Docker container via `docker exec`
    async fn docker_exec(&self, cmd: &str) -> Result<String> {
        let timeout_dur = Duration::from_secs(DOCKER_EXEC_TIMEOUT_SECS);
        let exec_future = Command::new("docker")
            .arg("exec")
            .arg(&self.config.container_name)
            .arg("bash")
            .arg("-c")
            .arg(cmd)
            .env("DISPLAY", ":1")
            .output();

        let output = match tokio::time::timeout(timeout_dur, exec_future).await {
            Ok(result) => result.context("Failed to run docker exec")?,
            Err(_) => {
                warn!(
                    "docker exec timed out after {}s, force-stopping container '{}'",
                    DOCKER_EXEC_TIMEOUT_SECS, self.config.container_name
                );
                self.force_stop().await;
                anyhow::bail!(
                    "Container execution timed out after {}s — command was terminated",
                    DOCKER_EXEC_TIMEOUT_SECS
                );
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("docker exec failed: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Run a command inside the container and return raw bytes (for screenshots)
    async fn docker_exec_bytes(&self, cmd: &str) -> Result<Vec<u8>> {
        let timeout_dur = Duration::from_secs(DOCKER_EXEC_TIMEOUT_SECS);
        let exec_future = Command::new("docker")
            .arg("exec")
            .arg(&self.config.container_name)
            .arg("bash")
            .arg("-c")
            .arg(cmd)
            .env("DISPLAY", ":1")
            .output();

        let output = match tokio::time::timeout(timeout_dur, exec_future).await {
            Ok(result) => result.context("Failed to run docker exec")?,
            Err(_) => {
                warn!(
                    "docker exec timed out after {}s, force-stopping container '{}'",
                    DOCKER_EXEC_TIMEOUT_SECS, self.config.container_name
                );
                self.force_stop().await;
                anyhow::bail!(
                    "Container execution timed out after {}s — command was terminated",
                    DOCKER_EXEC_TIMEOUT_SECS
                );
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("docker exec screenshot failed: {}", stderr);
        }

        Ok(output.stdout)
    }

    /// Apply human-like delay between actions
    async fn human_delay(&self) {
        if self.human_like.enabled {
            let delay = random_delay(self.human_like.min_delay_ms, self.human_like.max_delay_ms);
            tokio::time::sleep(delay).await;
        }
    }

    /// Apply random offset to click coordinates
    fn jitter_coord(&self, x: u32, y: u32) -> (u32, u32) {
        if !self.human_like.enabled || self.human_like.click_offset_px == 0 {
            return (x, y);
        }
        let dx = random_offset(self.human_like.click_offset_px);
        let dy = random_offset(self.human_like.click_offset_px);
        let new_x = (x as i32 + dx).max(0).min(self.display_width as i32 - 1) as u32;
        let new_y = (y as i32 + dy).max(0).min(self.display_height as i32 - 1) as u32;
        (new_x, new_y)
    }

    /// Check if the container is running
    pub async fn is_running(&self) -> bool {
        let output = Command::new("docker")
            .arg("inspect")
            .arg("-f")
            .arg("{{.State.Running}}")
            .arg(&self.config.container_name)
            .output()
            .await;
        match output {
            Ok(o) => String::from_utf8_lossy(&o.stdout).trim() == "true",
            Err(_) => false,
        }
    }

    /// Force-stop the container (used when execution times out)
    async fn force_stop(&self) {
        let _ = Command::new("docker")
            .arg("kill")
            .arg(&self.config.container_name)
            .output()
            .await;
        warn!("Force-killed container '{}'", self.config.container_name);
    }

    /// Start the sandbox container (creates if not exists) — called via trait
    async fn start_or_create(&self) -> Result<()> {
        if self.is_running().await {
            debug!("Docker sandbox '{}' already running", self.config.container_name);
            return Ok(());
        }

        // Try to start existing stopped container first
        let start_result = Command::new("docker")
            .arg("start")
            .arg(&self.config.container_name)
            .output()
            .await;

        if let Ok(output) = start_result {
            if output.status.success() {
                debug!("Started existing container '{}'", self.config.container_name);
                // Wait for display to be ready
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                return Ok(());
            }
        }

        // Create and start new container
        let output = Command::new("docker")
            .arg("run")
            .arg("-d")
            .arg("--rm") // Auto-remove container on stop to prevent resource leaks
            .arg("--name")
            .arg(&self.config.container_name)
            // Resource limits to prevent sandbox escape via resource exhaustion
            .arg("--memory=512m")
            .arg("--cpus=1.0")
            .arg("--pids-limit=256")
            // Execution time limit — container stops after 60s of inactivity
            .arg("--stop-timeout=60")
            // Security hardening: drop all capabilities, no privilege escalation
            .arg("--cap-drop=ALL")
            .arg("--security-opt=no-new-privileges")
            .arg("-p")
            .arg(format!("{}:5900", self.config.vnc_port))
            .arg("-p")
            .arg(format!("{}:6080", self.config.novnc_port))
            .arg("-e")
            .arg(format!("WIDTH={}", self.display_width))
            .arg("-e")
            .arg(format!("HEIGHT={}", self.display_height))
            .arg(&self.config.image)
            .output()
            .await
            .context("Failed to start Docker sandbox container")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to create sandbox container: {}", stderr);
        }

        // Wait for display to be ready
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        debug!("Docker sandbox '{}' started", self.config.container_name);
        Ok(())
    }
}

#[async_trait]
impl SandboxController for DockerSandbox {
    async fn screenshot(&self) -> Result<Vec<u8>> {
        // Use scrot to capture screenshot, output to stdout as PNG
        // -o = overwrite, -z = silent, /dev/stdout = output to stdout
        let png_bytes = self.docker_exec_bytes(
            "DISPLAY=:1 scrot -o /tmp/_screenshot.png && cat /tmp/_screenshot.png"
        ).await.context("Screenshot capture failed")?;

        if png_bytes.is_empty() {
            anyhow::bail!("Screenshot returned empty data");
        }

        debug!("Screenshot captured: {} bytes", png_bytes.len());
        Ok(png_bytes)
    }

    async fn click(&self, action: &str, x: u32, y: u32) -> Result<()> {
        let (jx, jy) = self.jitter_coord(x, y);
        let jx_s = jx.to_string();
        let jy_s = jy.to_string();

        // Move mouse first (human-like: don't teleport)
        if self.human_like.enabled {
            let output = Command::new("docker")
                .arg("exec")
                .arg("-e").arg("DISPLAY=:1")
                .arg(&self.config.container_name)
                .arg("xdotool").arg("mousemove").arg("--sync").arg(&jx_s).arg(&jy_s)
                .output()
                .await
                .context("Failed to run docker exec for mouse move")?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("docker exec mousemove failed: {}", stderr);
            }
            self.human_delay().await;
        }

        // Build xdotool click command with direct arg passing (no shell interpolation)
        let mut cmd = Command::new("docker");
        cmd.arg("exec")
            .arg("-e").arg("DISPLAY=:1")
            .arg(&self.config.container_name)
            .arg("xdotool")
            .arg("mousemove").arg("--sync").arg(&jx_s).arg(&jy_s)
            .arg("click");

        match action {
            "double_click" => {
                cmd.arg("--repeat").arg("2").arg("--delay").arg("50").arg("1");
            }
            "triple_click" => {
                cmd.arg("--repeat").arg("3").arg("--delay").arg("50").arg("1");
            }
            _ => {
                let button = match action {
                    "right_click" => "3",
                    "middle_click" => "2",
                    _ => "1", // left_click and default
                };
                cmd.arg(button);
            }
        };

        let output = cmd.output().await.context("Failed to run docker exec for click")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("docker exec click failed: {}", stderr);
        }

        self.human_delay().await;
        debug!("click: {} at ({}, {}) [jittered to ({}, {})]", action, x, y, jx, jy);
        Ok(())
    }

    async fn type_text(&self, text: &str) -> Result<()> {
        // Use direct argument passing to avoid shell injection.
        // Each arg is passed separately so xdotool receives the text verbatim.
        let mut cmd = Command::new("docker");
        cmd.arg("exec")
            .arg("-e").arg("DISPLAY=:1")
            .arg(&self.config.container_name)
            .arg("xdotool").arg("type");

        if self.human_like.enabled {
            let delay_ms = (self.human_like.min_typing_delay_ms + self.human_like.max_typing_delay_ms) / 2;
            cmd.arg("--delay").arg(delay_ms.to_string());
        }

        cmd.arg("--").arg(text);

        let output = cmd.output().await.context("Failed to run docker exec for type_text")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("docker exec type_text failed: {}", stderr);
        }

        self.human_delay().await;
        debug!("type_text: {} chars", text.len());
        Ok(())
    }

    async fn key_press(&self, key: &str) -> Result<()> {
        // Convert common key names to xdotool format
        // e.g., "ctrl+s" → "ctrl+s", "Return" → "Return"
        let xdotool_key = key
            .replace("cmd+", "super+")
            .replace("command+", "super+");

        // Validate key name to prevent command injection via crafted key strings
        if !is_safe_key_name(&xdotool_key) {
            anyhow::bail!(
                "Invalid key name '{}': must contain only alphanumeric, +, _, - characters",
                key
            );
        }

        // Use direct argument passing instead of shell interpolation
        let output = Command::new("docker")
            .arg("exec")
            .arg("-e").arg("DISPLAY=:1")
            .arg(&self.config.container_name)
            .arg("xdotool").arg("key").arg("--").arg(&xdotool_key)
            .output()
            .await
            .context("Failed to run docker exec for key_press")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("docker exec key_press failed: {}", stderr);
        }

        self.human_delay().await;
        debug!("key_press: {}", key);
        Ok(())
    }

    async fn scroll(&self, x: u32, y: u32, direction: &str, amount: u32) -> Result<()> {
        let x_s = x.to_string();
        let y_s = y.to_string();

        // Move to position first (direct arg passing, no shell interpolation)
        let output = Command::new("docker")
            .arg("exec")
            .arg("-e").arg("DISPLAY=:1")
            .arg(&self.config.container_name)
            .arg("xdotool").arg("mousemove").arg("--sync").arg(&x_s).arg(&y_s)
            .output()
            .await
            .context("Failed to run docker exec for scroll mousemove")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("docker exec scroll mousemove failed: {}", stderr);
        }

        let button = match direction {
            "up" => "4",
            "down" => "5",
            "left" => "6",
            "right" => "7",
            _ => "5", // default: scroll down
        };

        // Scroll in increments for smoother behavior
        for _ in 0..amount {
            let output = Command::new("docker")
                .arg("exec")
                .arg("-e").arg("DISPLAY=:1")
                .arg(&self.config.container_name)
                .arg("xdotool").arg("click").arg(button)
                .output()
                .await
                .context("Failed to run docker exec for scroll click")?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("docker exec scroll click failed: {}", stderr);
            }
            if self.human_like.enabled {
                let delay = random_delay(30, 80);
                tokio::time::sleep(delay).await;
            }
        }

        self.human_delay().await;
        debug!("scroll: {} x{} at ({}, {})", direction, amount, x, y);
        Ok(())
    }

    async fn mouse_move(&self, x: u32, y: u32) -> Result<()> {
        let output = Command::new("docker")
            .arg("exec")
            .arg("-e").arg("DISPLAY=:1")
            .arg(&self.config.container_name)
            .arg("xdotool").arg("mousemove").arg("--sync")
            .arg(x.to_string()).arg(y.to_string())
            .output()
            .await
            .context("Failed to run docker exec for mouse_move")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("docker exec mouse_move failed: {}", stderr);
        }
        debug!("mouse_move: ({}, {})", x, y);
        Ok(())
    }

    async fn drag(&self, start_x: u32, start_y: u32, end_x: u32, end_y: u32) -> Result<()> {
        let output = Command::new("docker")
            .arg("exec")
            .arg("-e").arg("DISPLAY=:1")
            .arg(&self.config.container_name)
            .arg("xdotool")
            .arg("mousemove").arg("--sync")
            .arg(start_x.to_string()).arg(start_y.to_string())
            .arg("mousedown").arg("1")
            .arg("mousemove").arg("--sync")
            .arg(end_x.to_string()).arg(end_y.to_string())
            .arg("mouseup").arg("1")
            .output()
            .await
            .context("Failed to run docker exec for drag")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("docker exec drag failed: {}", stderr);
        }
        self.human_delay().await;
        debug!("drag: ({},{}) -> ({},{})", start_x, start_y, end_x, end_y);
        Ok(())
    }

    async fn execute_command(&self, cmd: &str) -> Result<String> {
        // SECURITY: cmd runs inside the Docker container (sandboxed), not on the host.
        // The container has --cap-drop=ALL, --security-opt=no-new-privileges,
        // memory/CPU/PID limits, and runs as non-root user 'sandbox'.
        // The docker_exec method uses Command::arg() (no shell on the host side),
        // so the container_name cannot be used for host-side injection.
        self.docker_exec(cmd).await
    }

    fn display_size(&self) -> (u32, u32) {
        (self.display_width, self.display_height)
    }

    async fn ensure_running(&self) -> Result<()> {
        self.start_or_create().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = DockerSandboxConfig::default();
        assert_eq!(config.image, "phantom-mesh-sandbox:latest");
        assert_eq!(config.container_name, "phantom-mesh-sandbox");
        assert_eq!(config.vnc_port, 5900);
        assert_eq!(config.novnc_port, 6080);
    }

    #[test]
    fn test_jitter_coord_within_bounds() {
        let sandbox = DockerSandbox::new(
            DockerSandboxConfig::default(),
            HumanLikeConfig { enabled: true, click_offset_px: 3, ..Default::default() },
            1024,
            768,
        );
        // Run multiple times to test randomness stays in bounds
        for _ in 0..100 {
            let (jx, jy) = sandbox.jitter_coord(500, 400);
            assert!(jx < 1024);
            assert!(jy < 768);
        }
    }

    #[test]
    fn test_jitter_coord_disabled() {
        let sandbox = DockerSandbox::new(
            DockerSandboxConfig::default(),
            HumanLikeConfig { enabled: false, ..Default::default() },
            1024,
            768,
        );
        let (jx, jy) = sandbox.jitter_coord(500, 400);
        assert_eq!(jx, 500);
        assert_eq!(jy, 400);
    }

    #[test]
    fn test_is_safe_key_name_valid() {
        assert!(is_safe_key_name("Return"));
        assert!(is_safe_key_name("ctrl+s"));
        assert!(is_safe_key_name("super+Left"));
        assert!(is_safe_key_name("ctrl+shift+t"));
        assert!(is_safe_key_name("F12"));
        assert!(is_safe_key_name("space"));
        assert!(is_safe_key_name("ctrl+alt+Delete"));
    }

    #[test]
    fn test_is_safe_key_name_rejects_injection() {
        // Shell metacharacters that could be used for injection
        assert!(!is_safe_key_name(""));
        assert!(!is_safe_key_name("; rm -rf /"));
        assert!(!is_safe_key_name("key$(whoami)"));
        assert!(!is_safe_key_name("key`id`"));
        assert!(!is_safe_key_name("a\nb"));
        assert!(!is_safe_key_name("key | cat /etc/passwd"));
        assert!(!is_safe_key_name("key & curl evil.com"));
        // Too long
        assert!(!is_safe_key_name(&"a".repeat(65)));
    }

    #[test]
    fn test_jitter_coord_edge_cases() {
        let sandbox = DockerSandbox::new(
            DockerSandboxConfig::default(),
            HumanLikeConfig { enabled: true, click_offset_px: 5, ..Default::default() },
            1024,
            768,
        );
        // Near edges — should clamp
        for _ in 0..100 {
            let (jx, _) = sandbox.jitter_coord(0, 0);
            assert!(jx < 1024);
            let (jx, _) = sandbox.jitter_coord(1023, 767);
            assert!(jx < 1024);
        }
    }
}
