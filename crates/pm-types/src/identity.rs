use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::capability::Capability;

/// Type alias for node identifier strings.
pub type NodeId = String;

/// A node's cryptographic identity (Ed25519 public key).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeIdentity {
    pub public_key: [u8; 32],
    pub node_id: NodeId,
}

/// Information about a peer node in the cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub node_id: NodeId,
    pub name: String,
    pub addr: String,
    pub device_type: String,
    pub capabilities: Vec<Capability>,
    pub role: NodeRole,
    pub cpu_load: f32,
    pub memory_pct: f32,
    pub active_tasks: usize,
    pub uptime_secs: u64,
    pub last_seen: u64,
    pub status: PeerStatus,
    pub providers: Vec<String>,
    /// Dynamic extension fields for forward compatibility.
    #[serde(default)]
    pub extra_fields: HashMap<String, serde_json::Value>,
}

/// Role of a node in the cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum NodeRole {
    Coordinator,
    Worker,
    Candidate,
}

/// Status of a peer node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PeerStatus {
    Online,
    Offline,
    Suspect,
}

/// Control plane operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ControlPlane {
    Heartbeat,
    Election,
    Discovery,
    Pairing,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_identity_creation() {
        let id = NodeIdentity {
            public_key: [0u8; 32],
            node_id: "node-1".to_string(),
        };
        assert_eq!(id.node_id, "node-1");
    }

    #[test]
    fn test_peer_info_serde() {
        let peer = PeerInfo {
            node_id: "n1".into(),
            name: "desktop".into(),
            addr: "127.0.0.1:7878".into(),
            device_type: "desktop".into(),
            capabilities: vec![Capability::Shell],
            role: NodeRole::Worker,
            cpu_load: 0.5,
            memory_pct: 60.0,
            active_tasks: 2,
            uptime_secs: 3600,
            last_seen: 1000,
            status: PeerStatus::Online,
            providers: vec!["ollama".into()],
            extra_fields: HashMap::new(),
        };
        let json = serde_json::to_string(&peer).unwrap();
        let back: PeerInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back.node_id, "n1");
        assert_eq!(back.capabilities, vec![Capability::Shell]);
    }

    #[test]
    fn test_peer_info_extra_fields_preserved() {
        let json = r#"{"node_id":"n1","name":"test","addr":"127.0.0.1:7878","device_type":"desktop","capabilities":["Shell"],"role":"Worker","cpu_load":0.0,"memory_pct":0.0,"active_tasks":0,"uptime_secs":0,"last_seen":0,"status":"Online","providers":[],"extra_fields":{"custom":"value"}}"#;
        let peer: PeerInfo = serde_json::from_str(json).unwrap();
        assert_eq!(peer.extra_fields.get("custom").unwrap(), "value");
    }
}
