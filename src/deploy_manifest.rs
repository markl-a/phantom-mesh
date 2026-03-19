//! Deployment manifest generator for reproducible deployments.
//!
//! Captures a snapshot of the build and runtime environment — git commit,
//! cargo version, build timestamp, loaded hands, registered tools, active
//! providers, cluster nodes, config hash, and Rust version — and provides
//! serialization to JSON plus a structured diff between two manifests.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::process::Command;

// ---------------------------------------------------------------------------
// Core struct
// ---------------------------------------------------------------------------

/// A snapshot of the deployment environment at a given point in time.
#[derive(Debug, Clone, Serialize)]
pub struct DeployManifest {
    /// Git HEAD commit hash (None if not inside a git repository).
    pub git_commit: Option<String>,
    /// Crate version from Cargo.toml (compile-time).
    pub cargo_version: String,
    /// Timestamp at which the manifest was generated.
    pub build_timestamp: String,
    /// Names of all loaded hand workflows.
    pub loaded_hands: Vec<String>,
    /// Names of all registered tools.
    pub registered_tools: Vec<String>,
    /// Names of all active LLM providers.
    pub active_providers: Vec<String>,
    /// Addresses / identifiers of cluster nodes.
    pub cluster_nodes: Vec<String>,
    /// Hash of the `agents.toml` config file content.
    pub config_hash: String,
    /// Rust compiler version used to build the binary.
    pub rust_version: String,
}

// ---------------------------------------------------------------------------
// Generation helpers
// ---------------------------------------------------------------------------

/// Attempt to retrieve the current git HEAD commit hash.
///
/// Returns `None` when the working directory is not inside a git repo or the
/// `git` binary is unavailable.
fn git_head_commit() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;

    if output.status.success() {
        let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if hash.is_empty() {
            None
        } else {
            Some(hash)
        }
    } else {
        None
    }
}

/// Compute a SHA-256 hex digest of the given bytes.
fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    hex::encode(result)
}

/// Compute a hash of the given string using `DefaultHasher` and return a hex
/// representation. This is used as a lightweight fallback when SHA-256 is not
/// required.
fn default_hash_hex(data: &str) -> String {
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Read `~/.clawtex/agents.toml` and return its content, or an empty string
/// if the file is missing or unreadable.
fn read_agents_toml() -> String {
    let path = dirs::home_dir()
        .map(|h| h.join(".clawtex").join("agents.toml"));

    match path {
        Some(p) => std::fs::read_to_string(p).unwrap_or_default(),
        None => String::new(),
    }
}

/// Hash the `agents.toml` content with SHA-256 (preferred) and annotate the
/// result. If the file is empty / missing, falls back to `DefaultHasher` over
/// an empty string.
fn config_hash() -> String {
    let content = read_agents_toml();
    if content.is_empty() {
        // Fallback — produce a deterministic hash for "no config"
        default_hash_hex("")
    } else {
        sha256_hex(content.as_bytes())
    }
}

/// Return the Rust compiler version string, e.g. `"rustc 1.78.0 (…)"`.
fn rust_version() -> String {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

impl DeployManifest {
    /// Collect all available information and build a `DeployManifest`.
    ///
    /// Fields that depend on external processes (`git`, `rustc`) are
    /// best-effort — failures are silently swallowed and safe defaults are
    /// used instead.
    pub fn generate() -> Self {
        let now = chrono::Utc::now().to_rfc3339();

        Self {
            git_commit: git_head_commit(),
            cargo_version: env!("CARGO_PKG_VERSION").to_string(),
            build_timestamp: now,
            loaded_hands: Vec::new(),
            registered_tools: Vec::new(),
            active_providers: Vec::new(),
            cluster_nodes: Vec::new(),
            config_hash: config_hash(),
            rust_version: rust_version(),
        }
    }

    /// Convenience builder — populate the `loaded_hands` field.
    pub fn with_hands(mut self, hands: Vec<String>) -> Self {
        self.loaded_hands = hands;
        self
    }

    /// Convenience builder — populate the `registered_tools` field.
    pub fn with_tools(mut self, tools: Vec<String>) -> Self {
        self.registered_tools = tools;
        self
    }

    /// Convenience builder — populate the `active_providers` field.
    pub fn with_providers(mut self, providers: Vec<String>) -> Self {
        self.active_providers = providers;
        self
    }

    /// Convenience builder — populate the `cluster_nodes` field.
    pub fn with_nodes(mut self, nodes: Vec<String>) -> Self {
        self.cluster_nodes = nodes;
        self
    }

    /// Serialize the manifest as pretty-printed JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("DeployManifest serialization cannot fail")
    }

    /// Compare two manifests and return a list of human-readable difference
    /// descriptions. An empty list means the manifests are identical.
    pub fn diff(a: &DeployManifest, b: &DeployManifest) -> Vec<String> {
        let mut diffs = Vec::new();

        if a.git_commit != b.git_commit {
            diffs.push(format!(
                "git_commit: {:?} -> {:?}",
                a.git_commit, b.git_commit
            ));
        }
        if a.cargo_version != b.cargo_version {
            diffs.push(format!(
                "cargo_version: {} -> {}",
                a.cargo_version, b.cargo_version
            ));
        }
        if a.build_timestamp != b.build_timestamp {
            diffs.push(format!(
                "build_timestamp: {} -> {}",
                a.build_timestamp, b.build_timestamp
            ));
        }
        if a.config_hash != b.config_hash {
            diffs.push(format!(
                "config_hash: {} -> {}",
                a.config_hash, b.config_hash
            ));
        }
        if a.rust_version != b.rust_version {
            diffs.push(format!(
                "rust_version: {} -> {}",
                a.rust_version, b.rust_version
            ));
        }

        // Compare sorted vec fields for order-independent diffs
        Self::diff_vec("loaded_hands", &a.loaded_hands, &b.loaded_hands, &mut diffs);
        Self::diff_vec("registered_tools", &a.registered_tools, &b.registered_tools, &mut diffs);
        Self::diff_vec("active_providers", &a.active_providers, &b.active_providers, &mut diffs);
        Self::diff_vec("cluster_nodes", &a.cluster_nodes, &b.cluster_nodes, &mut diffs);

        diffs
    }

    /// Helper: diff two string vectors, reporting added and removed items.
    fn diff_vec(
        label: &str,
        old: &[String],
        new: &[String],
        diffs: &mut Vec<String>,
    ) {
        let old_set: std::collections::HashSet<&str> =
            old.iter().map(|s| s.as_str()).collect();
        let new_set: std::collections::HashSet<&str> =
            new.iter().map(|s| s.as_str()).collect();

        let mut added: Vec<&str> = new_set.difference(&old_set).copied().collect();
        let mut removed: Vec<&str> = old_set.difference(&new_set).copied().collect();
        added.sort();
        removed.sort();

        if !added.is_empty() {
            diffs.push(format!("{} added: {}", label, added.join(", ")));
        }
        if !removed.is_empty() {
            diffs.push(format!("{} removed: {}", label, removed.join(", ")));
        }
    }
}

// ---------------------------------------------------------------------------
// Standalone helper (re-exported for other modules)
// ---------------------------------------------------------------------------

/// Compute the SHA-256 hex digest of a byte slice. Useful for downstream
/// consumers that want to hash arbitrary config payloads.
pub fn sha256_of(data: &[u8]) -> String {
    sha256_hex(data)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- 1. generate() returns sensible defaults --------------------------

    #[test]
    fn test_generate_has_cargo_version() {
        let m = DeployManifest::generate();
        assert_eq!(m.cargo_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn test_generate_has_build_timestamp() {
        let m = DeployManifest::generate();
        // RFC-3339 timestamps always contain 'T' and '+' or 'Z'
        assert!(
            m.build_timestamp.contains('T'),
            "build_timestamp should be RFC-3339: {}",
            m.build_timestamp
        );
    }

    #[test]
    fn test_generate_has_rust_version() {
        let m = DeployManifest::generate();
        // In a cargo test environment rustc is always available
        assert!(
            m.rust_version.starts_with("rustc"),
            "Expected rust_version to start with 'rustc', got: {}",
            m.rust_version
        );
    }

    #[test]
    fn test_generate_config_hash_non_empty() {
        let m = DeployManifest::generate();
        assert!(
            !m.config_hash.is_empty(),
            "config_hash must not be empty"
        );
    }

    // -- 2. Builder helpers -----------------------------------------------

    #[test]
    fn test_builder_with_fields() {
        let m = DeployManifest::generate()
            .with_hands(vec!["seo_content".into(), "outreach".into()])
            .with_tools(vec!["web_search".into(), "file_write".into()])
            .with_providers(vec!["gemini".into(), "groq".into()])
            .with_nodes(vec!["z13:7878".into(), "m1mac:7879".into()]);

        assert_eq!(m.loaded_hands, vec!["seo_content", "outreach"]);
        assert_eq!(m.registered_tools, vec!["web_search", "file_write"]);
        assert_eq!(m.active_providers, vec!["gemini", "groq"]);
        assert_eq!(m.cluster_nodes, vec!["z13:7878", "m1mac:7879"]);
    }

    // -- 3. to_json() output is valid JSON --------------------------------

    #[test]
    fn test_to_json_valid() {
        let m = DeployManifest::generate()
            .with_hands(vec!["content".into()])
            .with_tools(vec!["shell".into()]);

        let json = m.to_json();
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("to_json must produce valid JSON");

        assert!(parsed.is_object());
        assert_eq!(
            parsed["cargo_version"].as_str().unwrap(),
            env!("CARGO_PKG_VERSION")
        );
        assert!(parsed["loaded_hands"].is_array());
        assert_eq!(parsed["loaded_hands"][0].as_str().unwrap(), "content");
    }

    #[test]
    fn test_to_json_contains_all_fields() {
        let m = DeployManifest::generate();
        let json = m.to_json();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();

        for field in &[
            "git_commit",
            "cargo_version",
            "build_timestamp",
            "loaded_hands",
            "registered_tools",
            "active_providers",
            "cluster_nodes",
            "config_hash",
            "rust_version",
        ] {
            assert!(
                v.get(field).is_some(),
                "JSON must contain field '{}'",
                field
            );
        }
    }

    // -- 4. diff() --------------------------------------------------------

    #[test]
    fn test_diff_identical_manifests() {
        let m = DeployManifest {
            git_commit: Some("abc123".into()),
            cargo_version: "0.1.0".into(),
            build_timestamp: "2026-03-19T12:00:00+00:00".into(),
            loaded_hands: vec!["seo".into()],
            registered_tools: vec!["shell".into()],
            active_providers: vec!["gemini".into()],
            cluster_nodes: vec!["z13".into()],
            config_hash: "deadbeef".into(),
            rust_version: "rustc 1.80.0".into(),
        };

        let diffs = DeployManifest::diff(&m, &m);
        assert!(diffs.is_empty(), "Identical manifests should produce no diffs");
    }

    #[test]
    fn test_diff_detects_scalar_changes() {
        let a = DeployManifest {
            git_commit: Some("aaa".into()),
            cargo_version: "0.1.0".into(),
            build_timestamp: "t1".into(),
            loaded_hands: vec![],
            registered_tools: vec![],
            active_providers: vec![],
            cluster_nodes: vec![],
            config_hash: "h1".into(),
            rust_version: "rustc 1.79.0".into(),
        };
        let b = DeployManifest {
            git_commit: Some("bbb".into()),
            cargo_version: "0.2.0".into(),
            build_timestamp: "t2".into(),
            loaded_hands: vec![],
            registered_tools: vec![],
            active_providers: vec![],
            cluster_nodes: vec![],
            config_hash: "h2".into(),
            rust_version: "rustc 1.80.0".into(),
        };

        let diffs = DeployManifest::diff(&a, &b);
        assert!(diffs.iter().any(|d| d.contains("git_commit")));
        assert!(diffs.iter().any(|d| d.contains("cargo_version")));
        assert!(diffs.iter().any(|d| d.contains("build_timestamp")));
        assert!(diffs.iter().any(|d| d.contains("config_hash")));
        assert!(diffs.iter().any(|d| d.contains("rust_version")));
    }

    #[test]
    fn test_diff_detects_vec_additions_and_removals() {
        let a = DeployManifest {
            git_commit: None,
            cargo_version: "0.1.0".into(),
            build_timestamp: "t".into(),
            loaded_hands: vec!["seo".into(), "outreach".into()],
            registered_tools: vec!["shell".into()],
            active_providers: vec!["gemini".into()],
            cluster_nodes: vec!["z13".into()],
            config_hash: "h".into(),
            rust_version: "rustc 1.80.0".into(),
        };
        let b = DeployManifest {
            git_commit: None,
            cargo_version: "0.1.0".into(),
            build_timestamp: "t".into(),
            loaded_hands: vec!["seo".into(), "content".into()],
            registered_tools: vec!["shell".into(), "web_search".into()],
            active_providers: vec![],
            cluster_nodes: vec!["z13".into(), "m1mac".into()],
            config_hash: "h".into(),
            rust_version: "rustc 1.80.0".into(),
        };

        let diffs = DeployManifest::diff(&a, &b);

        // loaded_hands: outreach removed, content added
        assert!(diffs.iter().any(|d| d.contains("loaded_hands added") && d.contains("content")));
        assert!(diffs.iter().any(|d| d.contains("loaded_hands removed") && d.contains("outreach")));

        // registered_tools: web_search added
        assert!(diffs.iter().any(|d| d.contains("registered_tools added") && d.contains("web_search")));

        // active_providers: gemini removed
        assert!(diffs.iter().any(|d| d.contains("active_providers removed") && d.contains("gemini")));

        // cluster_nodes: m1mac added
        assert!(diffs.iter().any(|d| d.contains("cluster_nodes added") && d.contains("m1mac")));
    }

    #[test]
    fn test_diff_git_commit_none_vs_some() {
        let a = DeployManifest {
            git_commit: None,
            cargo_version: "0.1.0".into(),
            build_timestamp: "t".into(),
            loaded_hands: vec![],
            registered_tools: vec![],
            active_providers: vec![],
            cluster_nodes: vec![],
            config_hash: "h".into(),
            rust_version: "r".into(),
        };
        let b = DeployManifest {
            git_commit: Some("abc".into()),
            ..a.clone()
        };

        let diffs = DeployManifest::diff(&a, &b);
        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].contains("git_commit"));
        assert!(diffs[0].contains("None"));
    }

    // -- 5. SHA-256 helper ------------------------------------------------

    #[test]
    fn test_sha256_of_known_value() {
        // SHA-256 of "" is a well-known constant
        let empty_hash = sha256_of(b"");
        assert_eq!(
            empty_hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_sha256_of_deterministic() {
        let h1 = sha256_of(b"hello clawtex");
        let h2 = sha256_of(b"hello clawtex");
        assert_eq!(h1, h2);
        assert_ne!(h1, sha256_of(b"different"));
    }

    // -- 6. default_hash_hex internal helper ------------------------------

    #[test]
    fn test_default_hash_hex_deterministic() {
        let h1 = default_hash_hex("test input");
        let h2 = default_hash_hex("test input");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16, "DefaultHasher produces a u64 -> 16 hex chars");
    }
}
