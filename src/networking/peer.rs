//! Peer-to-peer mesh node registry for Phantom Mesh.
//!
//! Every node in the cluster is a [`PeerNode`](PeerInfo). One node is elected
//! *Coordinator* (lightweight — maintains registry + route table, doesn't relay
//! data). The [`PeerRegistry`] tracks all known peers in-memory, rebuilt from
//! heartbeats.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Device type — affects election priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceType {
    Desktop,
    Mobile,
}

/// Status of a peer node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeerStatus {
    Online,
    Offline,
    Unknown,
}

/// Information about a peer node in the mesh.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    /// Ed25519 public key hex prefix (16 chars).
    pub node_id: String,
    /// Human-friendly name.
    pub name: String,
    /// Address, e.g. `"100.x.x.x:7878"`.
    pub addr: String,
    /// Device type (Desktop or Mobile).
    pub device_type: DeviceType,
    /// Capabilities this node advertises (e.g. `["llm", "code_exec"]`).
    pub capabilities: Vec<String>,
    /// Current CPU load (0.0–1.0).
    pub cpu_load: f32,
    /// Current memory usage percentage (0.0–100.0).
    pub memory_pct: f32,
    /// Number of tasks currently being executed.
    pub active_tasks: usize,
    /// Seconds since this node started.
    pub uptime_secs: u64,
    /// Unix timestamp of the last heartbeat received.
    pub last_seen: u64,
    /// Current status.
    pub status: PeerStatus,
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// In-memory registry of all known peers, rebuilt from heartbeats.
#[derive(Debug)]
pub struct PeerRegistry {
    /// `node_id` → `PeerInfo`.
    peers: RwLock<HashMap<String, PeerInfo>>,
    /// Current coordinator node_id.
    coordinator_id: RwLock<Option<String>>,
    /// A peer is considered stale if `now - last_seen > stale_timeout_secs`.
    stale_timeout_secs: u64,
}

impl PeerRegistry {
    /// Create a new, empty registry with a default stale timeout of 30 s.
    pub fn new() -> Self {
        Self {
            peers: RwLock::new(HashMap::new()),
            coordinator_id: RwLock::new(None),
            stale_timeout_secs: 30,
        }
    }

    /// Create a registry with a custom stale timeout.
    pub fn with_stale_timeout(secs: u64) -> Self {
        Self {
            peers: RwLock::new(HashMap::new()),
            coordinator_id: RwLock::new(None),
            stale_timeout_secs: secs,
        }
    }

    // -- mutators -----------------------------------------------------------

    /// Register a new peer or update an existing one.
    pub fn upsert(&self, info: PeerInfo) {
        let mut peers = self.peers.write().expect("PeerRegistry lock poisoned");
        peers.insert(info.node_id.clone(), info);
    }

    /// Remove a peer entirely.
    pub fn remove(&self, node_id: &str) {
        let mut peers = self.peers.write().expect("PeerRegistry lock poisoned");
        peers.remove(node_id);
    }

    /// Set the current coordinator's `node_id`.
    pub fn set_coordinator(&self, node_id: &str) {
        let mut coord = self.coordinator_id.write().expect("PeerRegistry lock poisoned");
        *coord = Some(node_id.to_string());
    }

    /// Mark peers whose `last_seen` is older than `stale_timeout_secs` as
    /// [`PeerStatus::Offline`]. Returns the list of node_ids that transitioned
    /// to offline.
    pub fn mark_stale(&self) -> Vec<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX epoch")
            .as_secs();

        let mut peers = self.peers.write().expect("PeerRegistry lock poisoned");
        let mut stale_ids = Vec::new();

        for (id, peer) in peers.iter_mut() {
            if peer.status != PeerStatus::Offline
                && now.saturating_sub(peer.last_seen) > self.stale_timeout_secs
            {
                peer.status = PeerStatus::Offline;
                stale_ids.push(id.clone());
            }
        }

        stale_ids
    }

    // -- queries ------------------------------------------------------------

    /// Get a clone of the [`PeerInfo`] for `node_id`, if present.
    pub fn get(&self, node_id: &str) -> Option<PeerInfo> {
        let peers = self.peers.read().expect("PeerRegistry lock poisoned");
        peers.get(node_id).cloned()
    }

    /// Get all peers with status [`PeerStatus::Online`].
    pub fn online_peers(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().expect("PeerRegistry lock poisoned");
        peers
            .values()
            .filter(|p| p.status == PeerStatus::Online)
            .cloned()
            .collect()
    }

    /// Get all peers regardless of status.
    pub fn all_peers(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().expect("PeerRegistry lock poisoned");
        peers.values().cloned().collect()
    }

    /// Get the current coordinator's `node_id`.
    pub fn coordinator(&self) -> Option<String> {
        let coord = self.coordinator_id.read().expect("PeerRegistry lock poisoned");
        coord.clone()
    }

    /// Number of currently online peers.
    pub fn online_count(&self) -> usize {
        let peers = self.peers.read().expect("PeerRegistry lock poisoned");
        peers
            .values()
            .filter(|p| p.status == PeerStatus::Online)
            .count()
    }

    /// Find all online peers that advertise `capability`.
    pub fn peers_with_capability(&self, capability: &str) -> Vec<PeerInfo> {
        let peers = self.peers.read().expect("PeerRegistry lock poisoned");
        peers
            .values()
            .filter(|p| {
                p.status == PeerStatus::Online
                    && p.capabilities.iter().any(|c| c == capability)
            })
            .cloned()
            .collect()
    }

    /// Find the least-loaded **online** peer that has `capability`.
    ///
    /// "Load" is measured as `active_tasks` (primary), then `cpu_load`
    /// (secondary).
    pub fn least_loaded_with_capability(&self, capability: &str) -> Option<PeerInfo> {
        let peers = self.peers.read().expect("PeerRegistry lock poisoned");
        peers
            .values()
            .filter(|p| {
                p.status == PeerStatus::Online
                    && p.capabilities.iter().any(|c| c == capability)
            })
            .min_by(|a, b| {
                a.active_tasks
                    .cmp(&b.active_tasks)
                    .then(a.cpu_load.partial_cmp(&b.cpu_load).unwrap_or(std::cmp::Ordering::Equal))
            })
            .cloned()
    }
}

impl Default for PeerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_peer(id: &str, status: PeerStatus, last_seen: u64) -> PeerInfo {
        PeerInfo {
            node_id: id.to_string(),
            name: format!("node-{id}"),
            addr: "127.0.0.1:7878".to_string(),
            device_type: DeviceType::Desktop,
            capabilities: vec!["llm".to_string()],
            cpu_load: 0.5,
            memory_pct: 40.0,
            active_tasks: 0,
            uptime_secs: 3600,
            last_seen,
            status,
        }
    }

    #[test]
    fn upsert_and_get() {
        let reg = PeerRegistry::new();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let peer = make_peer("aabb", PeerStatus::Online, now);
        reg.upsert(peer.clone());

        let got = reg.get("aabb").unwrap();
        assert_eq!(got.node_id, "aabb");
        assert_eq!(got.name, "node-aabb");
    }

    #[test]
    fn online_count() {
        let reg = PeerRegistry::new();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        reg.upsert(make_peer("a", PeerStatus::Online, now));
        reg.upsert(make_peer("b", PeerStatus::Online, now));
        reg.upsert(make_peer("c", PeerStatus::Offline, now));

        assert_eq!(reg.online_count(), 2);
        assert_eq!(reg.online_peers().len(), 2);
        assert_eq!(reg.all_peers().len(), 3);
    }

    #[test]
    fn remove_peer() {
        let reg = PeerRegistry::new();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        reg.upsert(make_peer("x", PeerStatus::Online, now));
        assert!(reg.get("x").is_some());
        reg.remove("x");
        assert!(reg.get("x").is_none());
    }
}
