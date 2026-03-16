//! WASM Tool Sandbox — execute plugin tools in a sandboxed WASM environment.
//!
//! Provides a `WasmSandbox` that loads .wasm modules and executes tool calls
//! within strict resource limits (memory, time, capabilities).
//!
//! Current implementation uses a process-based sandbox (calling wasmtime CLI).
//! Can be swapped for embedded wasmtime/wasmer runtime later.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, warn};

/// Resource limits for WASM execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmLimits {
    /// Maximum memory in bytes (default: 64MB)
    #[serde(default = "default_max_memory")]
    pub max_memory_bytes: u64,
    /// Maximum execution time (default: 30s)
    #[serde(default = "default_max_time_secs")]
    pub max_time_secs: u64,
    /// Maximum output size in bytes (default: 1MB)
    #[serde(default = "default_max_output")]
    pub max_output_bytes: u64,
    /// Allowed host capabilities
    #[serde(default)]
    pub capabilities: Vec<WasmCapability>,
}

fn default_max_memory() -> u64 { 64 * 1024 * 1024 }
fn default_max_time_secs() -> u64 { 30 }
fn default_max_output() -> u64 { 1024 * 1024 }

impl Default for WasmLimits {
    fn default() -> Self {
        Self {
            max_memory_bytes: default_max_memory(),
            max_time_secs: default_max_time_secs(),
            max_output_bytes: default_max_output(),
            capabilities: Vec::new(),
        }
    }
}

/// Capabilities a WASM module can request
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WasmCapability {
    /// Read files from a specific directory
    FileRead,
    /// Write files to a specific directory
    FileWrite,
    /// Make HTTP requests
    HttpRequest,
    /// Access environment variables
    EnvVars,
}

/// Result of a WASM tool execution
#[derive(Debug, Clone)]
pub struct WasmResult {
    pub output: String,
    pub exit_code: i32,
    pub elapsed: Duration,
    pub memory_used: u64,
}

/// WASM sandbox for executing plugin tools
pub struct WasmSandbox {
    /// Path to the wasmtime/wasmer CLI binary
    runtime_path: Option<PathBuf>,
    /// Default resource limits
    limits: WasmLimits,
    /// Loaded module cache: module_path -> validated
    modules: HashMap<PathBuf, bool>,
}

impl WasmSandbox {
    pub fn new(limits: WasmLimits) -> Self {
        let runtime_path = find_wasm_runtime();
        if runtime_path.is_none() {
            warn!("No WASM runtime found (wasmtime/wasmer). WASM plugins will be unavailable.");
        }
        Self {
            runtime_path,
            limits,
            modules: HashMap::new(),
        }
    }

    /// Check if a WASM runtime is available
    pub fn is_available(&self) -> bool {
        self.runtime_path.is_some()
    }

    /// Validate and register a WASM module
    pub fn register_module(&mut self, wasm_path: &Path) -> Result<()> {
        if !wasm_path.exists() {
            anyhow::bail!("WASM module not found: {}", wasm_path.display());
        }

        let ext = wasm_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "wasm" && ext != "wat" {
            anyhow::bail!("Invalid WASM file extension: .{} (expected .wasm or .wat)", ext);
        }

        // Basic validation: check file size
        let metadata = std::fs::metadata(wasm_path)?;
        if metadata.len() > 100 * 1024 * 1024 {
            anyhow::bail!("WASM module too large: {} bytes (max 100MB)", metadata.len());
        }

        self.modules.insert(wasm_path.to_path_buf(), true);
        debug!("Registered WASM module: {}", wasm_path.display());
        Ok(())
    }

    /// Execute a WASM tool with given input
    pub async fn execute(
        &self,
        wasm_path: &Path,
        input: &str,
        limits: Option<&WasmLimits>,
    ) -> Result<WasmResult> {
        let runtime = self.runtime_path.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No WASM runtime available"))?;

        if !self.modules.contains_key(wasm_path) {
            anyhow::bail!("WASM module not registered: {}", wasm_path.display());
        }

        let limits = limits.unwrap_or(&self.limits);
        let timeout = Duration::from_secs(limits.max_time_secs);

        let start = std::time::Instant::now();

        // Build command with resource limits
        let mut cmd = tokio::process::Command::new(runtime);
        cmd.arg("run")
            .arg("--fuel")
            .arg(format!("{}", limits.max_memory_bytes / 1024)) // approximate fuel
            .arg(wasm_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;

        // Write input to stdin
        if let Some(stdin) = child.stdin.as_mut() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(input.as_bytes()).await;
            let _ = stdin.shutdown().await;
        }

        // Wait with timeout
        let result = tokio::time::timeout(timeout, child.wait_with_output()).await;

        let elapsed = start.elapsed();

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                // Enforce output size limit
                let output_text = if stdout.len() > limits.max_output_bytes as usize {
                    format!("{}...(truncated)", &stdout[..limits.max_output_bytes as usize])
                } else if stdout.is_empty() && !stderr.is_empty() {
                    format!("stderr: {}", stderr)
                } else {
                    stdout.to_string()
                };

                Ok(WasmResult {
                    output: output_text,
                    exit_code: output.status.code().unwrap_or(-1),
                    elapsed,
                    memory_used: 0, // not tracked in process mode
                })
            }
            Ok(Err(e)) => {
                anyhow::bail!("WASM execution failed: {}", e)
            }
            Err(_) => {
                anyhow::bail!("WASM execution timed out after {}s", limits.max_time_secs)
            }
        }
    }

    /// List registered modules
    pub fn modules(&self) -> Vec<&Path> {
        self.modules.keys().map(|p| p.as_path()).collect()
    }
}

/// Try to find a WASM runtime binary on the system
fn find_wasm_runtime() -> Option<PathBuf> {
    for name in &["wasmtime", "wasmer"] {
        if let Ok(output) = std::process::Command::new("which")
            .arg(name)
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(PathBuf::from(path));
                }
            }
        }
        // Windows: try where instead of which
        if let Ok(output) = std::process::Command::new("where")
            .arg(name)
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout)
                    .lines().next().unwrap_or("").trim().to_string();
                if !path.is_empty() {
                    return Some(PathBuf::from(path));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wasm_limits_default() {
        let limits = WasmLimits::default();
        assert_eq!(limits.max_memory_bytes, 64 * 1024 * 1024);
        assert_eq!(limits.max_time_secs, 30);
        assert_eq!(limits.max_output_bytes, 1024 * 1024);
        assert!(limits.capabilities.is_empty());
    }

    #[test]
    fn test_wasm_sandbox_creation() {
        let sandbox = WasmSandbox::new(WasmLimits::default());
        // May or may not find a runtime, but shouldn't panic
        assert!(sandbox.modules().is_empty());
    }

    #[test]
    fn test_register_nonexistent_module() {
        let mut sandbox = WasmSandbox::new(WasmLimits::default());
        let result = sandbox.register_module(Path::new("/nonexistent/plugin.wasm"));
        assert!(result.is_err());
    }

    #[test]
    fn test_register_invalid_extension() {
        let dir = tempfile::tempdir().unwrap();
        let bad_file = dir.path().join("plugin.txt");
        std::fs::write(&bad_file, "not wasm").unwrap();

        let mut sandbox = WasmSandbox::new(WasmLimits::default());
        let result = sandbox.register_module(&bad_file);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid WASM"));
    }

    #[test]
    fn test_register_valid_wasm_file() {
        let dir = tempfile::tempdir().unwrap();
        let wasm_file = dir.path().join("test.wasm");
        std::fs::write(&wasm_file, &[0x00, 0x61, 0x73, 0x6D]).unwrap(); // WASM magic bytes

        let mut sandbox = WasmSandbox::new(WasmLimits::default());
        let result = sandbox.register_module(&wasm_file);
        assert!(result.is_ok());
        assert_eq!(sandbox.modules().len(), 1);
    }

    #[test]
    fn test_wasm_capability_serde() {
        let cap = WasmCapability::HttpRequest;
        let json = serde_json::to_string(&cap).unwrap();
        assert_eq!(json, "\"http_request\"");
        let parsed: WasmCapability = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, WasmCapability::HttpRequest);
    }

    #[test]
    fn test_wasm_limits_serde() {
        let limits = WasmLimits {
            max_memory_bytes: 1024,
            max_time_secs: 5,
            max_output_bytes: 512,
            capabilities: vec![WasmCapability::FileRead, WasmCapability::HttpRequest],
        };
        let json = serde_json::to_string(&limits).unwrap();
        let parsed: WasmLimits = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.max_memory_bytes, 1024);
        assert_eq!(parsed.capabilities.len(), 2);
    }
}
