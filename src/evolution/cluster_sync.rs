//! M5.5 Cluster Capability Sync — synchronizes capabilities across cluster nodes.
//!
//! When the hub installs a new skill or plugin, CapabilitySyncManager broadcasts
//! the change to all workers. Workers can request a diff to discover what they
//! are missing, and the hub responds with the list of packages to install.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tracing::info;

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Messages for cluster-wide capability synchronization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CapabilitySyncMessage {
    /// A node announces its capabilities.
    Announce {
        node_id: String,
        capabilities: Vec<String>,
        installed_packages: Vec<String>,
    },
    /// Hub broadcasts a new skill/plugin install.
    SkillSync {
        package_id: String,
        version: String,
        capabilities: Vec<String>,
        checksum_sha256: String,
    },
    /// Hub broadcasts a config change.
    ConfigSync {
        key: String,
        value: String,
    },
    /// Request package diff from hub.
    RequestSync {
        node_id: String,
        current_packages: Vec<String>,
    },
    /// Response with missing capabilities.
    SyncResponse {
        missing_packages: Vec<String>,
    },
}

// ---------------------------------------------------------------------------
// Node manifest
// ---------------------------------------------------------------------------

/// Per-node capability manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeManifest {
    pub node_id: String,
    pub capabilities: HashSet<String>,
    pub installed_packages: HashSet<String>,
    pub last_sync_at: u64,
}

// ---------------------------------------------------------------------------
// CapabilitySyncManager
// ---------------------------------------------------------------------------

/// Manages cluster-wide capability synchronization.
pub struct CapabilitySyncManager {
    /// Known node manifests (node_id -> manifest)
    nodes: HashMap<String, NodeManifest>,
    /// Hub's own manifest
    hub_manifest: NodeManifest,
}

impl CapabilitySyncManager {
    pub fn new(hub_id: &str) -> Self {
        Self {
            nodes: HashMap::new(),
            hub_manifest: NodeManifest {
                node_id: hub_id.to_string(),
                capabilities: HashSet::new(),
                installed_packages: HashSet::new(),
                last_sync_at: 0,
            },
        }
    }

    /// Register a hub capability.
    pub fn add_hub_capability(&mut self, capability: &str) {
        self.hub_manifest
            .capabilities
            .insert(capability.to_string());
    }

    /// Register a hub package.
    pub fn add_hub_package(&mut self, package_id: &str) {
        self.hub_manifest
            .installed_packages
            .insert(package_id.to_string());
    }

    /// Process an incoming sync message.
    pub fn process_message(
        &mut self,
        msg: CapabilitySyncMessage,
    ) -> Option<CapabilitySyncMessage> {
        match msg {
            CapabilitySyncMessage::Announce {
                node_id,
                capabilities,
                installed_packages,
            } => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                self.nodes.insert(
                    node_id.clone(),
                    NodeManifest {
                        node_id,
                        capabilities: capabilities.into_iter().collect(),
                        installed_packages: installed_packages.into_iter().collect(),
                        last_sync_at: now,
                    },
                );
                None
            }
            CapabilitySyncMessage::RequestSync {
                node_id,
                current_packages,
            } => {
                let current: HashSet<String> = current_packages.into_iter().collect();
                let missing: Vec<String> = self
                    .hub_manifest
                    .installed_packages
                    .difference(&current)
                    .cloned()
                    .collect();
                info!(
                    "CapabilitySyncManager: {} missing {} packages",
                    node_id,
                    missing.len()
                );
                Some(CapabilitySyncMessage::SyncResponse {
                    missing_packages: missing,
                })
            }
            _ => None,
        }
    }

    /// Calculate what a specific worker is missing compared to hub.
    pub fn diff_for_node(&self, node_id: &str) -> Vec<String> {
        if let Some(node) = self.nodes.get(node_id) {
            self.hub_manifest
                .installed_packages
                .difference(&node.installed_packages)
                .cloned()
                .collect()
        } else {
            self.hub_manifest
                .installed_packages
                .iter()
                .cloned()
                .collect()
        }
    }

    /// Generate broadcast message for a new install.
    pub fn broadcast_install(
        &self,
        package_id: &str,
        version: &str,
        capabilities: Vec<String>,
        checksum: &str,
    ) -> CapabilitySyncMessage {
        CapabilitySyncMessage::SkillSync {
            package_id: package_id.to_string(),
            version: version.to_string(),
            capabilities,
            checksum_sha256: checksum.to_string(),
        }
    }

    /// Get all known nodes.
    pub fn known_nodes(&self) -> Vec<&NodeManifest> {
        self.nodes.values().collect()
    }

    /// Get hub manifest.
    pub fn hub_manifest(&self) -> &NodeManifest {
        &self.hub_manifest
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manager() -> CapabilitySyncManager {
        let mut mgr = CapabilitySyncManager::new("hub-1");
        mgr.add_hub_capability("ocr");
        mgr.add_hub_capability("tts");
        mgr.add_hub_package("pkg-ocr");
        mgr.add_hub_package("pkg-tts");
        mgr.add_hub_package("pkg-translate");
        mgr
    }

    #[test]
    fn test_announce_registers_node() {
        let mut mgr = make_manager();

        let msg = CapabilitySyncMessage::Announce {
            node_id: "worker-1".to_string(),
            capabilities: vec!["ocr".to_string()],
            installed_packages: vec!["pkg-ocr".to_string()],
        };

        let response = mgr.process_message(msg);
        assert!(response.is_none()); // Announce doesn't produce a response

        let nodes = mgr.known_nodes();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].node_id, "worker-1");
        assert!(nodes[0].capabilities.contains("ocr"));
        assert!(nodes[0].installed_packages.contains("pkg-ocr"));
        assert!(nodes[0].last_sync_at > 0);
    }

    #[test]
    fn test_request_sync_returns_diff() {
        let mut mgr = make_manager();

        let msg = CapabilitySyncMessage::RequestSync {
            node_id: "worker-1".to_string(),
            current_packages: vec!["pkg-ocr".to_string()],
        };

        let response = mgr.process_message(msg);
        assert!(response.is_some());

        if let Some(CapabilitySyncMessage::SyncResponse { missing_packages }) = response {
            // Hub has pkg-ocr, pkg-tts, pkg-translate. Worker has pkg-ocr.
            // Missing: pkg-tts, pkg-translate
            assert_eq!(missing_packages.len(), 2);
            let missing_set: HashSet<String> = missing_packages.into_iter().collect();
            assert!(missing_set.contains("pkg-tts"));
            assert!(missing_set.contains("pkg-translate"));
        } else {
            panic!("Expected SyncResponse");
        }
    }

    #[test]
    fn test_diff_for_node() {
        let mut mgr = make_manager();

        // Unknown node — gets all hub packages
        let diff_unknown = mgr.diff_for_node("unknown-worker");
        assert_eq!(diff_unknown.len(), 3); // all hub packages

        // Register a worker with some packages
        let msg = CapabilitySyncMessage::Announce {
            node_id: "worker-2".to_string(),
            capabilities: vec!["ocr".to_string(), "tts".to_string()],
            installed_packages: vec!["pkg-ocr".to_string(), "pkg-tts".to_string()],
        };
        mgr.process_message(msg);

        // Known node — only missing packages
        let diff_known = mgr.diff_for_node("worker-2");
        assert_eq!(diff_known.len(), 1);
        assert!(diff_known.contains(&"pkg-translate".to_string()));

        // Worker with all packages — no diff
        let msg2 = CapabilitySyncMessage::Announce {
            node_id: "worker-3".to_string(),
            capabilities: vec![],
            installed_packages: vec![
                "pkg-ocr".to_string(),
                "pkg-tts".to_string(),
                "pkg-translate".to_string(),
            ],
        };
        mgr.process_message(msg2);
        let diff_full = mgr.diff_for_node("worker-3");
        assert!(diff_full.is_empty());
    }

    #[test]
    fn test_broadcast_install() {
        let mgr = make_manager();
        let msg = mgr.broadcast_install(
            "pkg-vision",
            "1.2.0",
            vec!["vision".to_string(), "image-detect".to_string()],
            "sha256abc",
        );

        match msg {
            CapabilitySyncMessage::SkillSync {
                package_id,
                version,
                capabilities,
                checksum_sha256,
            } => {
                assert_eq!(package_id, "pkg-vision");
                assert_eq!(version, "1.2.0");
                assert_eq!(capabilities.len(), 2);
                assert!(capabilities.contains(&"vision".to_string()));
                assert!(capabilities.contains(&"image-detect".to_string()));
                assert_eq!(checksum_sha256, "sha256abc");
            }
            _ => panic!("Expected SkillSync message"),
        }
    }

    #[test]
    fn test_multiple_nodes() {
        let mut mgr = make_manager();

        // Register worker-1
        mgr.process_message(CapabilitySyncMessage::Announce {
            node_id: "worker-1".to_string(),
            capabilities: vec!["ocr".to_string()],
            installed_packages: vec!["pkg-ocr".to_string()],
        });

        // Register worker-2
        mgr.process_message(CapabilitySyncMessage::Announce {
            node_id: "worker-2".to_string(),
            capabilities: vec!["tts".to_string()],
            installed_packages: vec!["pkg-tts".to_string()],
        });

        // Register worker-3
        mgr.process_message(CapabilitySyncMessage::Announce {
            node_id: "worker-3".to_string(),
            capabilities: vec!["ocr".to_string(), "tts".to_string()],
            installed_packages: vec!["pkg-ocr".to_string(), "pkg-tts".to_string()],
        });

        assert_eq!(mgr.known_nodes().len(), 3);

        // Each worker has different diff
        let diff1 = mgr.diff_for_node("worker-1");
        assert_eq!(diff1.len(), 2); // missing pkg-tts, pkg-translate

        let diff2 = mgr.diff_for_node("worker-2");
        assert_eq!(diff2.len(), 2); // missing pkg-ocr, pkg-translate

        let diff3 = mgr.diff_for_node("worker-3");
        assert_eq!(diff3.len(), 1); // missing pkg-translate

        // Update worker-1 with a re-announce (should overwrite)
        mgr.process_message(CapabilitySyncMessage::Announce {
            node_id: "worker-1".to_string(),
            capabilities: vec!["ocr".to_string(), "tts".to_string()],
            installed_packages: vec![
                "pkg-ocr".to_string(),
                "pkg-tts".to_string(),
                "pkg-translate".to_string(),
            ],
        });

        // Still 3 nodes (overwrite, not duplicate)
        assert_eq!(mgr.known_nodes().len(), 3);
        let diff1_updated = mgr.diff_for_node("worker-1");
        assert!(diff1_updated.is_empty()); // now has everything
    }

    #[test]
    fn test_add_hub_capabilities() {
        let mut mgr = CapabilitySyncManager::new("hub-test");

        // Initially empty
        assert!(mgr.hub_manifest().capabilities.is_empty());
        assert!(mgr.hub_manifest().installed_packages.is_empty());

        // Add capabilities
        mgr.add_hub_capability("ocr");
        mgr.add_hub_capability("tts");
        mgr.add_hub_capability("ocr"); // duplicate — should be idempotent

        assert_eq!(mgr.hub_manifest().capabilities.len(), 2);
        assert!(mgr.hub_manifest().capabilities.contains("ocr"));
        assert!(mgr.hub_manifest().capabilities.contains("tts"));

        // Add packages
        mgr.add_hub_package("pkg-a");
        mgr.add_hub_package("pkg-b");
        mgr.add_hub_package("pkg-a"); // duplicate

        assert_eq!(mgr.hub_manifest().installed_packages.len(), 2);
        assert!(mgr.hub_manifest().installed_packages.contains("pkg-a"));
        assert!(mgr.hub_manifest().installed_packages.contains("pkg-b"));

        // Hub ID is correct
        assert_eq!(mgr.hub_manifest().node_id, "hub-test");
    }

    #[test]
    fn test_config_sync_and_skill_sync_ignored() {
        let mut mgr = make_manager();

        // ConfigSync should return None (no special handling yet)
        let response = mgr.process_message(CapabilitySyncMessage::ConfigSync {
            key: "max_workers".to_string(),
            value: "8".to_string(),
        });
        assert!(response.is_none());

        // SkillSync should return None (broadcast, no response needed)
        let response = mgr.process_message(CapabilitySyncMessage::SkillSync {
            package_id: "pkg-new".to_string(),
            version: "1.0.0".to_string(),
            capabilities: vec!["new-cap".to_string()],
            checksum_sha256: "abc".to_string(),
        });
        assert!(response.is_none());
    }
}
