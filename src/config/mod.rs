//! Configuration management — separates cluster-wide config from per-node config.

use serde::{Serialize, Deserialize};
use std::collections::HashMap;

/// Cluster-wide configuration (distributed by Coordinator).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClusterConfig {
    /// Known node list (node_id -> last known address).
    #[serde(default)]
    pub nodes: HashMap<String, String>,
    /// Shared cluster secret (for RPC auth).
    #[serde(default)]
    pub cluster_secret: Option<String>,
    /// Global budget limit (USD per day, across all nodes).
    #[serde(default)]
    pub daily_budget_usd: Option<f64>,
    /// Config version (incremented on each change).
    #[serde(default)]
    pub version: u64,
}

/// Per-node configuration (local to each node, not shared).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Human-friendly node name.
    pub node_name: Option<String>,
    /// Port to listen on.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Device type override (auto-detected if not set).
    pub device_type: Option<String>,
    /// Tool permissions (tool_name -> enabled).
    #[serde(default)]
    pub tool_permissions: HashMap<String, bool>,
}

fn default_port() -> u16 { 7878 }

impl ClusterConfig {
    /// Merge updates from Coordinator (only if version is newer).
    pub fn merge(&mut self, other: &ClusterConfig) -> bool {
        if other.version > self.version {
            *self = other.clone();
            true
        } else {
            false
        }
    }
}
