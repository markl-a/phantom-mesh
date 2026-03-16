//! Plugin system — runtime extensibility without recompilation.
//! Inspired by ZeroClaw's Plugin trait + PluginManifest discovery.
//!
//! Plugins are discovered from:
//! - `~/.clawtex/plugins/<name>/plugin.toml` (user-installed)
//! - Built-in directory (bundled)
//!
//! Plugin TOML format:
//! ```toml
//! name = "my-plugin"
//! version = "0.1.0"
//! description = "A plugin that does X"
//! capabilities = ["tools", "hooks"]
//! entry = "plugin.wasm"  # or "plugin.py" for script plugins
//! ```

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Plugin capabilities
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginCapability {
    /// Can register new tools
    Tools,
    /// Can register hooks (llm, tool, message)
    Hooks,
    /// Can provide new LLM providers
    Providers,
    /// Can modify tool results before they're sent back
    ModifyToolResults,
}

/// Plugin manifest (from plugin.toml)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub capabilities: Vec<PluginCapability>,
    /// Entry point (e.g., "plugin.wasm", "plugin.py")
    #[serde(default)]
    pub entry: String,
    /// Whether this plugin is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Plugin status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginStatus {
    /// Discovered but not loaded
    Discovered,
    /// Successfully loaded and registered
    Loaded,
    /// Failed to load
    Failed(String),
    /// Disabled by user
    Disabled,
}

/// A discovered plugin with its manifest and status
#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub manifest: PluginManifest,
    pub path: PathBuf,
    pub status: PluginStatus,
}

/// Plugin registry — discovers, loads, and manages plugins
pub struct PluginRegistry {
    plugins: HashMap<String, PluginInfo>,
    plugin_dirs: Vec<PathBuf>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        let mut dirs = Vec::new();

        // Default plugin directory
        if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
            dirs.push(PathBuf::from(home).join(".clawtex").join("plugins"));
        }

        Self {
            plugins: HashMap::new(),
            plugin_dirs: dirs,
        }
    }

    /// Create with custom plugin directories
    pub fn with_dirs(dirs: Vec<PathBuf>) -> Self {
        Self {
            plugins: HashMap::new(),
            plugin_dirs: dirs,
        }
    }

    /// Discover all plugins from configured directories
    pub fn discover(&mut self) -> Vec<String> {
        let mut discovered = Vec::new();

        for dir in &self.plugin_dirs.clone() {
            if !dir.exists() {
                continue;
            }

            match std::fs::read_dir(dir) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            let manifest_path = path.join("plugin.toml");
                            if manifest_path.exists() {
                                match self.load_manifest(&manifest_path) {
                                    Ok(manifest) => {
                                        let name = manifest.name.clone();
                                        let status = if manifest.enabled {
                                            PluginStatus::Discovered
                                        } else {
                                            PluginStatus::Disabled
                                        };
                                        info!("Discovered plugin: {} v{}", name, manifest.version);
                                        self.plugins.insert(name.clone(), PluginInfo {
                                            manifest,
                                            path,
                                            status,
                                        });
                                        discovered.push(name);
                                    }
                                    Err(e) => {
                                        warn!("Failed to load plugin manifest at {:?}: {}", manifest_path, e);
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    debug!("Could not read plugin directory {:?}: {}", dir, e);
                }
            }
        }

        discovered
    }

    /// Load a plugin manifest from TOML file
    fn load_manifest(&self, path: &Path) -> Result<PluginManifest> {
        let content = std::fs::read_to_string(path)?;
        let manifest: PluginManifest = toml::from_str(&content)?;
        Ok(manifest)
    }

    /// Get a plugin by name
    pub fn get(&self, name: &str) -> Option<&PluginInfo> {
        self.plugins.get(name)
    }

    /// Enable/disable a plugin
    pub fn set_enabled(&mut self, name: &str, enabled: bool) {
        if let Some(plugin) = self.plugins.get_mut(name) {
            plugin.status = if enabled {
                PluginStatus::Discovered
            } else {
                PluginStatus::Disabled
            };
            plugin.manifest.enabled = enabled;
        }
    }

    /// List all plugins with their status
    pub fn list(&self) -> Vec<&PluginInfo> {
        let mut plugins: Vec<_> = self.plugins.values().collect();
        plugins.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
        plugins
    }

    /// Get plugins that have a specific capability
    pub fn with_capability(&self, cap: &PluginCapability) -> Vec<&PluginInfo> {
        self.plugins.values()
            .filter(|p| p.status != PluginStatus::Disabled && p.manifest.capabilities.contains(cap))
            .collect()
    }

    /// Register a plugin directly (for built-in plugins)
    pub fn register_builtin(&mut self, manifest: PluginManifest, path: PathBuf) {
        let name = manifest.name.clone();
        self.plugins.insert(name, PluginInfo {
            manifest,
            path,
            status: PluginStatus::Loaded,
        });
    }

    /// Total number of plugins
    pub fn count(&self) -> usize {
        self.plugins.len()
    }

    /// Number of enabled plugins
    pub fn enabled_count(&self) -> usize {
        self.plugins.values()
            .filter(|p| p.status != PluginStatus::Disabled)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest(name: &str) -> PluginManifest {
        PluginManifest {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            description: "Test plugin".to_string(),
            author: "test".to_string(),
            capabilities: vec![PluginCapability::Tools],
            entry: "plugin.wasm".to_string(),
            enabled: true,
        }
    }

    #[test]
    fn test_plugin_registry_empty() {
        let registry = PluginRegistry::with_dirs(vec![]);
        assert_eq!(registry.count(), 0);
        assert!(registry.list().is_empty());
    }

    #[test]
    fn test_register_builtin() {
        let mut registry = PluginRegistry::with_dirs(vec![]);
        registry.register_builtin(sample_manifest("test-plugin"), PathBuf::from("/test"));
        assert_eq!(registry.count(), 1);
        let plugin = registry.get("test-plugin").unwrap();
        assert_eq!(plugin.status, PluginStatus::Loaded);
    }

    #[test]
    fn test_enable_disable() {
        let mut registry = PluginRegistry::with_dirs(vec![]);
        registry.register_builtin(sample_manifest("test"), PathBuf::from("/test"));

        registry.set_enabled("test", false);
        assert_eq!(registry.get("test").unwrap().status, PluginStatus::Disabled);
        assert_eq!(registry.enabled_count(), 0);

        registry.set_enabled("test", true);
        assert_ne!(registry.get("test").unwrap().status, PluginStatus::Disabled);
        assert_eq!(registry.enabled_count(), 1);
    }

    #[test]
    fn test_with_capability() {
        let mut registry = PluginRegistry::with_dirs(vec![]);
        let mut m1 = sample_manifest("plugin-a");
        m1.capabilities = vec![PluginCapability::Tools, PluginCapability::Hooks];
        let mut m2 = sample_manifest("plugin-b");
        m2.capabilities = vec![PluginCapability::Hooks];

        registry.register_builtin(m1, PathBuf::from("/a"));
        registry.register_builtin(m2, PathBuf::from("/b"));

        let tools_plugins = registry.with_capability(&PluginCapability::Tools);
        assert_eq!(tools_plugins.len(), 1);
        assert_eq!(tools_plugins[0].manifest.name, "plugin-a");

        let hooks_plugins = registry.with_capability(&PluginCapability::Hooks);
        assert_eq!(hooks_plugins.len(), 2);
    }

    #[test]
    fn test_discover_nonexistent_dir() {
        let mut registry = PluginRegistry::with_dirs(vec![PathBuf::from("/nonexistent/plugins")]);
        let discovered = registry.discover();
        assert!(discovered.is_empty());
    }

    #[test]
    fn test_discover_real_dir() {
        let dir = std::env::temp_dir().join("clawtex_test_plugins");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);

        // Create a plugin directory with manifest
        let plugin_dir = dir.join("my-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            r#"
name = "my-plugin"
version = "1.0.0"
description = "Test"
capabilities = ["tools"]
"#,
        ).unwrap();

        let mut registry = PluginRegistry::with_dirs(vec![dir.clone()]);
        let discovered = registry.discover();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0], "my-plugin");

        let plugin = registry.get("my-plugin").unwrap();
        assert_eq!(plugin.manifest.version, "1.0.0");
        assert_eq!(plugin.status, PluginStatus::Discovered);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_manifest_serde() {
        let toml_str = r#"
name = "test-plugin"
version = "0.2.0"
description = "A test"
capabilities = ["tools", "hooks"]
entry = "main.py"
enabled = false
"#;
        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.name, "test-plugin");
        assert_eq!(manifest.capabilities.len(), 2);
        assert!(!manifest.enabled);
    }

    #[test]
    fn test_plugin_capability_serde() {
        let cap = PluginCapability::ModifyToolResults;
        let json = serde_json::to_string(&cap).unwrap();
        assert_eq!(json, "\"modifytoolresults\"");
    }
}
