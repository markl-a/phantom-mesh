//! Dynamic plugin system for loading external tools at runtime.
//!
//! Supports four plugin types:
//! - `shell_script` — executes a shell script with JSON on stdin
//! - `python` — executes a Python script with JSON on stdin
//! - `wasm` — reserved for future WebAssembly-based plugins
//! - `mcp` — reserved for MCP protocol-based plugins
//!
//! Plugin directory layout:
//! ```text
//! ~/.phantom-mesh/plugins/<name>/
//!     plugin.toml       # manifest
//!     run.sh / run.py   # entry point
//! ```

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

// ── Manifest ─────────────────────────────────────────────────────────────────

/// The type of plugin entry point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginToolType {
    ShellScript,
    Python,
    Wasm,
    Mcp,
}

impl std::fmt::Display for PluginToolType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginToolType::ShellScript => write!(f, "shell_script"),
            PluginToolType::Python => write!(f, "python"),
            PluginToolType::Wasm => write!(f, "wasm"),
            PluginToolType::Mcp => write!(f, "mcp"),
        }
    }
}

/// Plugin manifest deserialized from `plugin.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Unique plugin name (alphanumeric + hyphens).
    pub name: String,
    /// Semantic version string.
    pub version: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Plugin author.
    #[serde(default)]
    pub author: String,
    /// Plugin type determines how the entry point is executed.
    pub tool_type: PluginToolType,
    /// Relative path to the entry point script/binary within the plugin directory.
    pub entry_point: String,
    /// JSON Schema describing the plugin's accepted parameters.
    #[serde(default = "default_schema")]
    pub schema: serde_json::Value,
    /// Permissions the plugin requires (e.g., "network", "filesystem").
    #[serde(default)]
    pub permissions: Vec<String>,
}

fn default_schema() -> serde_json::Value {
    serde_json::json!({ "type": "object" })
}

// ── LoadedPlugin ─────────────────────────────────────────────────────────────

/// Runtime status of a loaded plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadedPluginStatus {
    Loaded,
    Active,
    Error(String),
}

impl std::fmt::Display for LoadedPluginStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadedPluginStatus::Loaded => write!(f, "loaded"),
            LoadedPluginStatus::Active => write!(f, "active"),
            LoadedPluginStatus::Error(e) => write!(f, "error: {}", e),
        }
    }
}

/// A plugin that has been loaded into the registry.
#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub status: LoadedPluginStatus,
    /// Absolute path to the plugin directory.
    pub plugin_dir: PathBuf,
    /// When the plugin was loaded (not serializable, so we store millis since load).
    pub load_time_unix_ms: u64,
}

impl LoadedPlugin {
    /// Resolve the absolute path to the entry point script.
    pub fn entry_point_path(&self) -> PathBuf {
        self.plugin_dir.join(&self.manifest.entry_point)
    }
}

/// Summary info returned by `list_plugins()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginLoaderInfo {
    pub name: String,
    pub version: String,
    pub status: String,
    pub tool_type: String,
    pub description: String,
}

// ── Schema validation ────────────────────────────────────────────────────────

/// Validate parameters against a JSON Schema (lightweight validation).
/// Checks: required fields, type of top-level value, property names.
pub fn validate_params(schema: &serde_json::Value, params: &serde_json::Value) -> Result<()> {
    // If schema says type=object, params must be an object
    if let Some(schema_type) = schema.get("type").and_then(|v| v.as_str()) {
        match schema_type {
            "object" => {
                if !params.is_object() {
                    bail!("Parameters must be a JSON object");
                }
            }
            "array" => {
                if !params.is_array() {
                    bail!("Parameters must be a JSON array");
                }
            }
            "string" => {
                if !params.is_string() {
                    bail!("Parameters must be a string");
                }
            }
            "number" | "integer" => {
                if !params.is_number() {
                    bail!("Parameters must be a number");
                }
            }
            _ => {}
        }
    }

    // Check required fields
    if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
        if let Some(obj) = params.as_object() {
            for req in required {
                if let Some(field_name) = req.as_str() {
                    if !obj.contains_key(field_name) {
                        bail!("Missing required parameter: {}", field_name);
                    }
                }
            }
        }
    }

    // Check that properties exist in schema (if properties is defined)
    if let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) {
        let additional = schema
            .get("additionalProperties")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if !additional {
            if let Some(obj) = params.as_object() {
                for key in obj.keys() {
                    if !properties.contains_key(key) {
                        bail!("Unknown parameter: {}", key);
                    }
                }
            }
        }
    }

    Ok(())
}

// ── Plugin execution ─────────────────────────────────────────────────────────

/// Execute a shell-script plugin by piping JSON params to stdin.
pub fn execute_shell_plugin(manifest: &PluginManifest, plugin_dir: &Path, params: &serde_json::Value) -> Result<String> {
    validate_params(&manifest.schema, params)?;

    let entry = plugin_dir.join(&manifest.entry_point);
    if !entry.exists() {
        bail!("Shell plugin entry point not found: {}", entry.display());
    }

    let json_input = serde_json::to_string(params)?;

    let shell = find_shell();
    // Convert to forward-slash path for bash on Windows
    let entry_str = entry.to_string_lossy().replace('\\', "/");
    let output = std::process::Command::new(&shell)
        .arg(&entry_str)
        .current_dir(plugin_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(ref mut stdin) = child.stdin {
                stdin.write_all(json_input.as_bytes())?;
            }
            child.wait_with_output()
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "Shell plugin '{}' exited with code {}: {}",
            manifest.name,
            output.status.code().unwrap_or(-1),
            stderr.trim()
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Execute a Python plugin by piping JSON params to stdin.
pub fn execute_python_plugin(manifest: &PluginManifest, plugin_dir: &Path, params: &serde_json::Value) -> Result<String> {
    validate_params(&manifest.schema, params)?;

    let entry = plugin_dir.join(&manifest.entry_point);
    if !entry.exists() {
        bail!("Python plugin entry point not found: {}", entry.display());
    }

    let json_input = serde_json::to_string(params)?;

    // Try python3 first, fall back to python
    let python = find_python();

    let output = std::process::Command::new(&python)
        .arg(entry.to_string_lossy().as_ref())
        .current_dir(plugin_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(ref mut stdin) = child.stdin {
                stdin.write_all(json_input.as_bytes())?;
            }
            child.wait_with_output()
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "Python plugin '{}' exited with code {}: {}",
            manifest.name,
            output.status.code().unwrap_or(-1),
            stderr.trim()
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Find the best available shell for running scripts.
/// On Windows, prefer Git Bash over WSL bash.
fn find_shell() -> String {
    if cfg!(windows) {
        // Try Git Bash first (avoids WSL bash which mangles Windows paths)
        let git_bash = r"C:\Program Files\Git\usr\bin\bash.exe";
        if Path::new(git_bash).exists() {
            return git_bash.to_string();
        }
        // Fallback to bash on PATH (may be WSL)
        "bash".to_string()
    } else {
        "sh".to_string()
    }
}

/// Find the best available Python interpreter.
fn find_python() -> String {
    // On macOS/Linux prefer python3
    if !cfg!(windows) {
        if let Ok(output) = std::process::Command::new("python3").arg("--version").output() {
            if output.status.success() {
                return "python3".to_string();
            }
        }
    }
    "python".to_string()
}

// ── PluginLoaderRegistry ─────────────────────────────────────────────────────

/// Registry that discovers, loads, manages, and executes external tool plugins.
pub struct PluginLoaderRegistry {
    plugins: HashMap<String, LoadedPlugin>,
}

impl PluginLoaderRegistry {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    /// Scan a directory for subdirectories containing `plugin.toml` files.
    /// Returns a list of parsed manifests.
    pub fn scan_directory(path: &Path) -> Vec<PluginManifest> {
        let mut manifests = Vec::new();

        if !path.exists() || !path.is_dir() {
            debug!("Plugin directory does not exist: {}", path.display());
            return manifests;
        }

        let entries = match std::fs::read_dir(path) {
            Ok(e) => e,
            Err(e) => {
                warn!("Cannot read plugin directory {}: {}", path.display(), e);
                return manifests;
            }
        };

        for entry in entries.flatten() {
            let sub = entry.path();
            if !sub.is_dir() {
                continue;
            }
            let manifest_path = sub.join("plugin.toml");
            if !manifest_path.exists() {
                continue;
            }

            match std::fs::read_to_string(&manifest_path) {
                Ok(content) => match toml::from_str::<PluginManifest>(&content) {
                    Ok(m) => {
                        info!(
                            "Scanned plugin '{}' v{} ({})",
                            m.name, m.version, m.tool_type
                        );
                        manifests.push(m);
                    }
                    Err(e) => {
                        warn!(
                            "Invalid plugin.toml at {}: {}",
                            manifest_path.display(),
                            e
                        );
                    }
                },
                Err(e) => {
                    warn!(
                        "Cannot read {}: {}",
                        manifest_path.display(),
                        e
                    );
                }
            }
        }

        manifests
    }

    /// Load a plugin from its manifest. The plugin_dir is the directory
    /// containing the plugin.toml file.
    pub fn load_plugin(&mut self, manifest: &PluginManifest, plugin_dir: &Path) -> Result<()> {
        if self.plugins.contains_key(&manifest.name) {
            bail!("Plugin '{}' is already loaded", manifest.name);
        }

        // Validate entry point exists for executable types
        match manifest.tool_type {
            PluginToolType::ShellScript | PluginToolType::Python => {
                let entry = plugin_dir.join(&manifest.entry_point);
                if !entry.exists() {
                    let loaded = LoadedPlugin {
                        manifest: manifest.clone(),
                        status: LoadedPluginStatus::Error(format!(
                            "Entry point not found: {}",
                            entry.display()
                        )),
                        plugin_dir: plugin_dir.to_path_buf(),
                        load_time_unix_ms: current_time_ms(),
                    };
                    self.plugins.insert(manifest.name.clone(), loaded);
                    bail!(
                        "Entry point not found: {}",
                        entry.display()
                    );
                }
            }
            PluginToolType::Wasm | PluginToolType::Mcp => {
                // Wasm/MCP validation deferred to execution time
            }
        }

        let loaded = LoadedPlugin {
            manifest: manifest.clone(),
            status: LoadedPluginStatus::Loaded,
            plugin_dir: plugin_dir.to_path_buf(),
            load_time_unix_ms: current_time_ms(),
        };
        info!("Loaded plugin '{}' v{}", manifest.name, manifest.version);
        self.plugins.insert(manifest.name.clone(), loaded);
        Ok(())
    }

    /// List all loaded plugins.
    pub fn list_plugins(&self) -> Vec<PluginLoaderInfo> {
        let mut infos: Vec<PluginLoaderInfo> = self
            .plugins
            .values()
            .map(|p| PluginLoaderInfo {
                name: p.manifest.name.clone(),
                version: p.manifest.version.clone(),
                status: p.status.to_string(),
                tool_type: p.manifest.tool_type.to_string(),
                description: p.manifest.description.clone(),
            })
            .collect();
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        infos
    }

    /// Unload (remove) a plugin by name.
    pub fn unload_plugin(&mut self, name: &str) -> bool {
        if self.plugins.remove(name).is_some() {
            info!("Unloaded plugin '{}'", name);
            true
        } else {
            debug!("Plugin '{}' not found for unload", name);
            false
        }
    }

    /// Get a reference to a loaded plugin by name.
    pub fn get_plugin(&self, name: &str) -> Option<&LoadedPlugin> {
        self.plugins.get(name)
    }

    /// Get a mutable reference to a loaded plugin by name.
    pub fn get_plugin_mut(&mut self, name: &str) -> Option<&mut LoadedPlugin> {
        self.plugins.get_mut(name)
    }

    /// Total number of loaded plugins.
    pub fn count(&self) -> usize {
        self.plugins.len()
    }

    /// Execute a loaded plugin with the given parameters.
    /// Routes to the appropriate executor based on `tool_type`.
    pub fn execute(&mut self, name: &str, params: &serde_json::Value) -> Result<String> {
        let plugin = self.plugins.get(name)
            .ok_or_else(|| anyhow!("Plugin '{}' not found", name))?;

        // Check status
        if let LoadedPluginStatus::Error(ref e) = plugin.status {
            bail!("Plugin '{}' is in error state: {}", name, e);
        }

        let manifest = plugin.manifest.clone();
        let plugin_dir = plugin.plugin_dir.clone();

        // Mark active
        if let Some(p) = self.plugins.get_mut(name) {
            p.status = LoadedPluginStatus::Active;
        }

        let result = match manifest.tool_type {
            PluginToolType::ShellScript => {
                execute_shell_plugin(&manifest, &plugin_dir, params)
            }
            PluginToolType::Python => {
                execute_python_plugin(&manifest, &plugin_dir, params)
            }
            PluginToolType::Wasm => {
                Err(anyhow!("WASM plugin execution is not yet implemented"))
            }
            PluginToolType::Mcp => {
                Err(anyhow!("MCP plugin execution is not yet implemented"))
            }
        };

        // Update status based on result
        if let Some(p) = self.plugins.get_mut(name) {
            match &result {
                Ok(_) => p.status = LoadedPluginStatus::Loaded,
                Err(e) => p.status = LoadedPluginStatus::Error(e.to_string()),
            }
        }

        result
    }

    /// Scan a directory and auto-load all valid plugins found.
    pub fn scan_and_load(&mut self, path: &Path) -> Vec<String> {
        let manifests = Self::scan_directory(path);
        let mut loaded_names = Vec::new();

        for manifest in &manifests {
            let plugin_dir = path.join(&manifest.name);
            match self.load_plugin(manifest, &plugin_dir) {
                Ok(()) => loaded_names.push(manifest.name.clone()),
                Err(e) => {
                    warn!("Failed to load plugin '{}': {}", manifest.name, e);
                }
            }
        }

        loaded_names
    }

    /// Check if a plugin has a specific permission.
    pub fn has_permission(&self, name: &str, permission: &str) -> bool {
        self.plugins
            .get(name)
            .map(|p| p.manifest.permissions.contains(&permission.to_string()))
            .unwrap_or(false)
    }

    /// Get names of all plugins with a given permission.
    pub fn plugins_with_permission(&self, permission: &str) -> Vec<String> {
        self.plugins
            .values()
            .filter(|p| p.manifest.permissions.contains(&permission.to_string()))
            .map(|p| p.manifest.name.clone())
            .collect()
    }
}

/// Get current time in milliseconds since UNIX epoch.
fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ── Helpers ──────────────────────────────────────────────────────────

    fn make_manifest(name: &str, tool_type: PluginToolType) -> PluginManifest {
        PluginManifest {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            description: format!("Test plugin {}", name),
            author: "test-author".to_string(),
            tool_type: tool_type.clone(),
            entry_point: match tool_type {
                PluginToolType::ShellScript => "run.sh".to_string(),
                PluginToolType::Python => "run.py".to_string(),
                _ => "plugin.wasm".to_string(),
            },
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "input": { "type": "string" }
                },
                "required": ["input"]
            }),
            permissions: vec!["network".to_string()],
        }
    }

    fn write_plugin_toml(dir: &Path, manifest: &PluginManifest) {
        let toml_str = toml::to_string_pretty(manifest).unwrap();
        fs::write(dir.join("plugin.toml"), toml_str).unwrap();
    }

    fn create_temp_plugin_dir(base: &Path, name: &str, tool_type: PluginToolType) -> (PathBuf, PluginManifest) {
        let plugin_dir = base.join(name);
        fs::create_dir_all(&plugin_dir).unwrap();
        let manifest = make_manifest(name, tool_type);
        write_plugin_toml(&plugin_dir, &manifest);
        (plugin_dir, manifest)
    }

    // ── PluginManifest tests ────────────────────────────────────────────

    #[test]
    fn test_manifest_deserialize_from_toml() {
        let toml_str = r#"
name = "hello-world"
version = "0.2.0"
description = "A simple greeting plugin"
author = "alice"
tool_type = "shell_script"
entry_point = "greet.sh"
permissions = ["filesystem"]

[schema]
type = "object"
"#;
        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.name, "hello-world");
        assert_eq!(manifest.version, "0.2.0");
        assert_eq!(manifest.tool_type, PluginToolType::ShellScript);
        assert_eq!(manifest.entry_point, "greet.sh");
        assert_eq!(manifest.permissions, vec!["filesystem"]);
        assert_eq!(manifest.author, "alice");
    }

    #[test]
    fn test_manifest_deserialize_python_type() {
        let toml_str = r#"
name = "py-analyzer"
version = "1.0.0"
tool_type = "python"
entry_point = "analyze.py"
"#;
        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.tool_type, PluginToolType::Python);
        assert_eq!(manifest.entry_point, "analyze.py");
        assert!(manifest.permissions.is_empty());
        assert!(manifest.description.is_empty());
    }

    #[test]
    fn test_manifest_deserialize_wasm_type() {
        let toml_str = r#"
name = "wasm-tool"
version = "0.1.0"
tool_type = "wasm"
entry_point = "tool.wasm"
permissions = ["network", "filesystem"]
"#;
        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.tool_type, PluginToolType::Wasm);
        assert_eq!(manifest.permissions.len(), 2);
    }

    #[test]
    fn test_manifest_deserialize_mcp_type() {
        let toml_str = r#"
name = "mcp-bridge"
version = "2.0.0"
tool_type = "mcp"
entry_point = "npx mcp-server"
"#;
        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.tool_type, PluginToolType::Mcp);
    }

    #[test]
    fn test_manifest_default_schema() {
        let toml_str = r#"
name = "minimal"
version = "0.1.0"
tool_type = "python"
entry_point = "run.py"
"#;
        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.schema, serde_json::json!({ "type": "object" }));
    }

    #[test]
    fn test_manifest_roundtrip_serde() {
        let manifest = make_manifest("roundtrip-test", PluginToolType::ShellScript);
        let json = serde_json::to_string(&manifest).unwrap();
        let deserialized: PluginManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, manifest.name);
        assert_eq!(deserialized.version, manifest.version);
        assert_eq!(deserialized.tool_type, manifest.tool_type);
    }

    // ── PluginToolType tests ────────────────────────────────────────────

    #[test]
    fn test_tool_type_display() {
        assert_eq!(PluginToolType::ShellScript.to_string(), "shell_script");
        assert_eq!(PluginToolType::Python.to_string(), "python");
        assert_eq!(PluginToolType::Wasm.to_string(), "wasm");
        assert_eq!(PluginToolType::Mcp.to_string(), "mcp");
    }

    // ── LoadedPluginStatus tests ────────────────────────────────────────

    #[test]
    fn test_loaded_plugin_status_display() {
        assert_eq!(LoadedPluginStatus::Loaded.to_string(), "loaded");
        assert_eq!(LoadedPluginStatus::Active.to_string(), "active");
        assert_eq!(
            LoadedPluginStatus::Error("bad".to_string()).to_string(),
            "error: bad"
        );
    }

    // ── validate_params tests ───────────────────────────────────────────

    #[test]
    fn test_validate_params_valid_object() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "name": { "type": "string" } },
            "required": ["name"]
        });
        let params = serde_json::json!({ "name": "test" });
        assert!(validate_params(&schema, &params).is_ok());
    }

    #[test]
    fn test_validate_params_missing_required() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["name", "age"]
        });
        let params = serde_json::json!({ "name": "test" });
        let err = validate_params(&schema, &params).unwrap_err();
        assert!(err.to_string().contains("Missing required parameter: age"));
    }

    #[test]
    fn test_validate_params_wrong_type() {
        let schema = serde_json::json!({ "type": "object" });
        let params = serde_json::json!("a string");
        let err = validate_params(&schema, &params).unwrap_err();
        assert!(err.to_string().contains("must be a JSON object"));
    }

    #[test]
    fn test_validate_params_unknown_property() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "name": { "type": "string" } },
            "additionalProperties": false
        });
        let params = serde_json::json!({ "name": "test", "extra": 42 });
        let err = validate_params(&schema, &params).unwrap_err();
        assert!(err.to_string().contains("Unknown parameter: extra"));
    }

    #[test]
    fn test_validate_params_array_type() {
        let schema = serde_json::json!({ "type": "array" });
        let params = serde_json::json!([1, 2, 3]);
        assert!(validate_params(&schema, &params).is_ok());

        let bad = serde_json::json!({ "a": 1 });
        assert!(validate_params(&schema, &bad).is_err());
    }

    #[test]
    fn test_validate_params_number_type() {
        let schema = serde_json::json!({ "type": "number" });
        assert!(validate_params(&schema, &serde_json::json!(42)).is_ok());
        assert!(validate_params(&schema, &serde_json::json!("text")).is_err());
    }

    #[test]
    fn test_validate_params_string_type() {
        let schema = serde_json::json!({ "type": "string" });
        assert!(validate_params(&schema, &serde_json::json!("hello")).is_ok());
        assert!(validate_params(&schema, &serde_json::json!(42)).is_err());
    }

    #[test]
    fn test_validate_params_empty_schema() {
        let schema = serde_json::json!({});
        let params = serde_json::json!({ "anything": "goes" });
        assert!(validate_params(&schema, &params).is_ok());
    }

    // ── PluginLoaderRegistry tests ──────────────────────────────────────

    #[test]
    fn test_registry_new_is_empty() {
        let registry = PluginLoaderRegistry::new();
        assert_eq!(registry.count(), 0);
        assert!(registry.list_plugins().is_empty());
    }

    #[test]
    fn test_load_plugin_wasm_no_entry_check() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("wasm-plugin");
        fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = make_manifest("wasm-plugin", PluginToolType::Wasm);
        // No actual .wasm file -- should still load (validation deferred)

        let mut registry = PluginLoaderRegistry::new();
        let result = registry.load_plugin(&manifest, &plugin_dir);
        assert!(result.is_ok());
        assert_eq!(registry.count(), 1);
    }

    #[test]
    fn test_load_plugin_shell_missing_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("missing-entry");
        fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = make_manifest("missing-entry", PluginToolType::ShellScript);
        // No run.sh created

        let mut registry = PluginLoaderRegistry::new();
        let result = registry.load_plugin(&manifest, &plugin_dir);
        assert!(result.is_err());
        // Plugin is registered in error state
        assert_eq!(registry.count(), 1);
        let p = registry.get_plugin("missing-entry").unwrap();
        assert!(matches!(p.status, LoadedPluginStatus::Error(_)));
    }

    #[test]
    fn test_load_plugin_success() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("good-shell");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("run.sh"), "#!/bin/sh\necho hello").unwrap();

        let manifest = make_manifest("good-shell", PluginToolType::ShellScript);

        let mut registry = PluginLoaderRegistry::new();
        assert!(registry.load_plugin(&manifest, &plugin_dir).is_ok());
        assert_eq!(registry.count(), 1);

        let plugin = registry.get_plugin("good-shell").unwrap();
        assert_eq!(plugin.status, LoadedPluginStatus::Loaded);
        assert_eq!(plugin.manifest.version, "1.0.0");
    }

    #[test]
    fn test_load_plugin_duplicate_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("dup-test");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("run.sh"), "#!/bin/sh\necho ok").unwrap();

        let manifest = make_manifest("dup-test", PluginToolType::ShellScript);

        let mut registry = PluginLoaderRegistry::new();
        assert!(registry.load_plugin(&manifest, &plugin_dir).is_ok());
        let result = registry.load_plugin(&manifest, &plugin_dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already loaded"));
    }

    #[test]
    fn test_unload_plugin() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("unload-test");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("run.sh"), "echo bye").unwrap();

        let manifest = make_manifest("unload-test", PluginToolType::ShellScript);

        let mut registry = PluginLoaderRegistry::new();
        registry.load_plugin(&manifest, &plugin_dir).unwrap();
        assert_eq!(registry.count(), 1);

        assert!(registry.unload_plugin("unload-test"));
        assert_eq!(registry.count(), 0);
        assert!(registry.get_plugin("unload-test").is_none());
    }

    #[test]
    fn test_unload_nonexistent() {
        let mut registry = PluginLoaderRegistry::new();
        assert!(!registry.unload_plugin("ghost"));
    }

    #[test]
    fn test_list_plugins_sorted() {
        let tmp = tempfile::tempdir().unwrap();

        let mut registry = PluginLoaderRegistry::new();

        for name in &["zulu", "alpha", "mike"] {
            let dir = tmp.path().join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("run.sh"), "echo ok").unwrap();
            let m = make_manifest(name, PluginToolType::ShellScript);
            registry.load_plugin(&m, &dir).unwrap();
        }

        let list = registry.list_plugins();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].name, "alpha");
        assert_eq!(list[1].name, "mike");
        assert_eq!(list[2].name, "zulu");
    }

    #[test]
    fn test_scan_directory_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let manifests = PluginLoaderRegistry::scan_directory(tmp.path());
        assert!(manifests.is_empty());
    }

    #[test]
    fn test_scan_directory_nonexistent() {
        let manifests = PluginLoaderRegistry::scan_directory(Path::new("/nonexistent/path/xyz"));
        assert!(manifests.is_empty());
    }

    #[test]
    fn test_scan_directory_finds_plugins() {
        let tmp = tempfile::tempdir().unwrap();

        // Create two plugin dirs
        create_temp_plugin_dir(tmp.path(), "plugin-a", PluginToolType::Python);
        create_temp_plugin_dir(tmp.path(), "plugin-b", PluginToolType::ShellScript);

        // Create a non-plugin dir (no plugin.toml)
        fs::create_dir_all(tmp.path().join("not-a-plugin")).unwrap();

        let manifests = PluginLoaderRegistry::scan_directory(tmp.path());
        assert_eq!(manifests.len(), 2);
        let names: Vec<_> = manifests.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"plugin-a"));
        assert!(names.contains(&"plugin-b"));
    }

    #[test]
    fn test_scan_and_load() {
        let tmp = tempfile::tempdir().unwrap();

        // Create plugin with actual entry file
        let (dir, _) = create_temp_plugin_dir(tmp.path(), "auto-load", PluginToolType::ShellScript);
        fs::write(dir.join("run.sh"), "#!/bin/sh\necho auto").unwrap();

        let mut registry = PluginLoaderRegistry::new();
        let loaded = registry.scan_and_load(tmp.path());

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], "auto-load");
        assert_eq!(registry.count(), 1);
    }

    #[test]
    fn test_has_permission() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("perm-test");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("run.sh"), "echo ok").unwrap();

        let mut manifest = make_manifest("perm-test", PluginToolType::ShellScript);
        manifest.permissions = vec!["network".to_string(), "filesystem".to_string()];

        let mut registry = PluginLoaderRegistry::new();
        registry.load_plugin(&manifest, &dir).unwrap();

        assert!(registry.has_permission("perm-test", "network"));
        assert!(registry.has_permission("perm-test", "filesystem"));
        assert!(!registry.has_permission("perm-test", "camera"));
        assert!(!registry.has_permission("nonexistent", "network"));
    }

    #[test]
    fn test_plugins_with_permission() {
        let tmp = tempfile::tempdir().unwrap();
        let mut registry = PluginLoaderRegistry::new();

        // Plugin with network permission
        let dir1 = tmp.path().join("net-plugin");
        fs::create_dir_all(&dir1).unwrap();
        fs::write(dir1.join("run.sh"), "echo ok").unwrap();
        let mut m1 = make_manifest("net-plugin", PluginToolType::ShellScript);
        m1.permissions = vec!["network".to_string()];
        registry.load_plugin(&m1, &dir1).unwrap();

        // Plugin without network permission
        let dir2 = tmp.path().join("fs-plugin");
        fs::create_dir_all(&dir2).unwrap();
        fs::write(dir2.join("run.sh"), "echo ok").unwrap();
        let mut m2 = make_manifest("fs-plugin", PluginToolType::ShellScript);
        m2.permissions = vec!["filesystem".to_string()];
        registry.load_plugin(&m2, &dir2).unwrap();

        let net = registry.plugins_with_permission("network");
        assert_eq!(net.len(), 1);
        assert_eq!(net[0], "net-plugin");
    }

    #[test]
    fn test_execute_wasm_not_implemented() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("wasm-exec");
        fs::create_dir_all(&dir).unwrap();

        let manifest = make_manifest("wasm-exec", PluginToolType::Wasm);

        let mut registry = PluginLoaderRegistry::new();
        registry.load_plugin(&manifest, &dir).unwrap();

        let result = registry.execute("wasm-exec", &serde_json::json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not yet implemented"));
    }

    #[test]
    fn test_execute_mcp_not_implemented() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("mcp-exec");
        fs::create_dir_all(&dir).unwrap();

        let manifest = make_manifest("mcp-exec", PluginToolType::Mcp);

        let mut registry = PluginLoaderRegistry::new();
        registry.load_plugin(&manifest, &dir).unwrap();

        let result = registry.execute("mcp-exec", &serde_json::json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not yet implemented"));
    }

    #[test]
    fn test_execute_nonexistent_plugin() {
        let mut registry = PluginLoaderRegistry::new();
        let result = registry.execute("nope", &serde_json::json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_execute_plugin_in_error_state() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("err-plugin");
        fs::create_dir_all(&dir).unwrap();
        // No entry point -> error state

        let manifest = make_manifest("err-plugin", PluginToolType::ShellScript);
        let mut registry = PluginLoaderRegistry::new();
        let _ = registry.load_plugin(&manifest, &dir); // fails

        let result = registry.execute("err-plugin", &serde_json::json!({"input": "x"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("error state"));
    }

    #[test]
    fn test_execute_shell_plugin_success() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("echo-plugin");
        fs::create_dir_all(&dir).unwrap();

        // Shell script that echoes "ok"
        fs::write(dir.join("run.sh"), "#!/bin/sh\necho ok").unwrap();

        let mut manifest = make_manifest("echo-plugin", PluginToolType::ShellScript);
        // Relax schema so we don't need "input"
        manifest.schema = serde_json::json!({ "type": "object" });

        let mut registry = PluginLoaderRegistry::new();
        registry.load_plugin(&manifest, &dir).unwrap();

        let result = registry.execute("echo-plugin", &serde_json::json!({}));
        assert!(result.is_ok(), "Shell plugin failed: {:?}", result.err());
        assert!(result.unwrap().trim() == "ok");
    }

    #[test]
    fn test_execute_python_plugin_success() {
        // Only run if python is available
        let python = find_python();
        let check = std::process::Command::new(&python).arg("--version").output();
        if check.is_err() || !check.unwrap().status.success() {
            eprintln!("Skipping Python test: no python found");
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("py-plugin");
        fs::create_dir_all(&dir).unwrap();

        // Python script that reads JSON from stdin and prints a field
        fs::write(
            dir.join("run.py"),
            "import sys, json\ndata = json.load(sys.stdin)\nprint(f\"Hello {data['input']}\")\n",
        )
        .unwrap();

        let manifest = make_manifest("py-plugin", PluginToolType::Python);

        let mut registry = PluginLoaderRegistry::new();
        registry.load_plugin(&manifest, &dir).unwrap();

        let result = registry.execute("py-plugin", &serde_json::json!({"input": "World"}));
        assert!(result.is_ok());
        assert!(result.unwrap().trim() == "Hello World");
    }

    #[test]
    fn test_execute_validation_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("validate-fail");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("run.sh"), "#!/bin/sh\necho ok").unwrap();

        let manifest = make_manifest("validate-fail", PluginToolType::ShellScript);
        // schema requires "input" field

        let mut registry = PluginLoaderRegistry::new();
        registry.load_plugin(&manifest, &dir).unwrap();

        // Pass params without required "input" field
        let result = registry.execute("validate-fail", &serde_json::json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing required parameter"));
    }

    #[test]
    fn test_loaded_plugin_entry_point_path() {
        let plugin = LoadedPlugin {
            manifest: make_manifest("path-test", PluginToolType::ShellScript),
            status: LoadedPluginStatus::Loaded,
            plugin_dir: PathBuf::from("/home/user/.phantom-mesh/plugins/path-test"),
            load_time_unix_ms: 0,
        };
        let expected = PathBuf::from("/home/user/.phantom-mesh/plugins/path-test/run.sh");
        assert_eq!(plugin.entry_point_path(), expected);
    }

    #[test]
    fn test_get_plugin_mut() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("mut-test");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("run.sh"), "echo ok").unwrap();

        let manifest = make_manifest("mut-test", PluginToolType::ShellScript);
        let mut registry = PluginLoaderRegistry::new();
        registry.load_plugin(&manifest, &dir).unwrap();

        let plugin = registry.get_plugin_mut("mut-test").unwrap();
        plugin.status = LoadedPluginStatus::Active;

        assert_eq!(
            registry.get_plugin("mut-test").unwrap().status,
            LoadedPluginStatus::Active
        );
    }

    #[test]
    fn test_plugin_loader_info_fields() {
        let info = PluginLoaderInfo {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            status: "loaded".to_string(),
            tool_type: "python".to_string(),
            description: "A test plugin".to_string(),
        };

        let json = serde_json::to_string(&info).unwrap();
        let deserialized: PluginLoaderInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "test");
        assert_eq!(deserialized.tool_type, "python");
    }
}
