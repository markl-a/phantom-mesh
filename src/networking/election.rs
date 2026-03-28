//! Bully-variant coordinator election for the Phantom Mesh peer-to-peer cluster.
//!
//! Election priority: **Desktop > Mobile**, then **longer uptime wins**, then
//! **alphabetically first `node_id`** as tiebreaker.
//!
//! The coordinator role is lightweight — it maintains the registry and route
//! table but does *not* relay data traffic.

use std::cmp::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use super::peer::{DeviceType, PeerInfo, PeerRegistry, PeerStatus};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Election configuration.
#[derive(Debug, Clone)]
pub struct ElectionConfig {
    /// How long to wait for an ALIVE response during election (seconds).
    pub timeout_secs: u64,
    /// Coordinator heartbeat miss threshold (seconds). If the coordinator's
    /// `last_seen` is older than this, a new election is triggered.
    pub heartbeat_miss_secs: u64,
}

impl Default for ElectionConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 5,
            heartbeat_miss_secs: 30,
        }
    }
}

// ---------------------------------------------------------------------------
// Priority
// ---------------------------------------------------------------------------

/// Election priority for a node. Higher = more likely to win.
///
/// Ordering: `device_rank` (desc) → `uptime_secs` (desc) → `node_id` (asc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElectionPriority {
    /// 1 for Desktop, 0 for Mobile.
    pub device_rank: u8,
    /// Longer uptime = higher priority.
    pub uptime_secs: u64,
    /// Alphabetically first wins as tiebreaker.
    pub node_id: String,
}

impl ElectionPriority {
    pub fn new(device_type: DeviceType, uptime_secs: u64, node_id: &str) -> Self {
        let device_rank = match device_type {
            DeviceType::Desktop => 1,
            DeviceType::Mobile => 0,
        };
        Self {
            device_rank,
            uptime_secs,
            node_id: node_id.to_string(),
        }
    }
}

/// We implement `Ord` so that the *best* candidate sorts **last** with
/// `Vec::sort`, or equivalently is the `max()`.
///
/// Comparison order:
/// 1. `device_rank` ascending (higher rank = greater)
/// 2. `uptime_secs` ascending (longer uptime = greater)
/// 3. `node_id` **reverse** alphabetical (alphabetically first = greater, so
///    `"aaa"` beats `"zzz"`)
impl Ord for ElectionPriority {
    fn cmp(&self, other: &Self) -> Ordering {
        self.device_rank
            .cmp(&other.device_rank)
            .then(self.uptime_secs.cmp(&other.uptime_secs))
            .then(other.node_id.cmp(&self.node_id)) // reversed: smaller id wins
    }
}

impl PartialOrd for ElectionPriority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// ---------------------------------------------------------------------------
// Election logic
// ---------------------------------------------------------------------------

/// Determine the winner among a set of candidates.
///
/// Only **online** candidates are considered. Returns `None` if `candidates`
/// is empty or none are online.
pub fn elect_coordinator(candidates: &[PeerInfo]) -> Option<String> {
    candidates
        .iter()
        .filter(|p| p.status == PeerStatus::Online)
        .max_by_key(|p| {
            ElectionPriority::new(p.device_type, p.uptime_secs, &p.node_id)
        })
        .map(|p| p.node_id.clone())
}

/// Check whether a new election should be triggered.
///
/// Returns `true` if:
/// 1. No coordinator is currently set, **or**
/// 2. The coordinator's `last_seen` timestamp is older than
///    `config.heartbeat_miss_secs`.
pub fn should_trigger_election(
    registry: &PeerRegistry,
    _our_node_id: &str,
    coordinator_last_seen: u64,
    config: &ElectionConfig,
) -> bool {
    // Case 1: no coordinator
    let coordinator = match registry.coordinator() {
        Some(id) => id,
        None => return true,
    };

    // Case 1b: coordinator exists in registry but is offline
    if let Some(info) = registry.get(&coordinator) {
        if info.status == PeerStatus::Offline {
            return true;
        }
    }

    // Case 2: coordinator heartbeat is stale
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_secs();

    now.saturating_sub(coordinator_last_seen) > config.heartbeat_miss_secs
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::networking::peer::{DeviceType, PeerInfo, PeerStatus};

    fn make_candidate(
        id: &str,
        device_type: DeviceType,
        uptime_secs: u64,
        status: PeerStatus,
    ) -> PeerInfo {
        PeerInfo {
            node_id: id.to_string(),
            name: format!("node-{id}"),
            addr: "127.0.0.1:7878".to_string(),
            device_type,
            capabilities: vec![],
            cpu_load: 0.0,
            memory_pct: 0.0,
            active_tasks: 0,
            uptime_secs,
            last_seen: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            status,
        }
    }

    #[test]
    fn desktop_wins_over_mobile() {
        let candidates = vec![
            make_candidate("mob1", DeviceType::Mobile, 9999, PeerStatus::Online),
            make_candidate("desk1", DeviceType::Desktop, 100, PeerStatus::Online),
        ];
        assert_eq!(elect_coordinator(&candidates).unwrap(), "desk1");
    }

    #[test]
    fn longer_uptime_wins() {
        let candidates = vec![
            make_candidate("a", DeviceType::Desktop, 1000, PeerStatus::Online),
            make_candidate("b", DeviceType::Desktop, 5000, PeerStatus::Online),
        ];
        assert_eq!(elect_coordinator(&candidates).unwrap(), "b");
    }

    #[test]
    fn node_id_tiebreaker() {
        let candidates = vec![
            make_candidate("zzz", DeviceType::Desktop, 1000, PeerStatus::Online),
            make_candidate("aaa", DeviceType::Desktop, 1000, PeerStatus::Online),
        ];
        // Alphabetically first ("aaa") wins
        assert_eq!(elect_coordinator(&candidates).unwrap(), "aaa");
    }

    #[test]
    fn empty_candidates() {
        assert!(elect_coordinator(&[]).is_none());
    }

    #[test]
    fn offline_candidates_ignored() {
        let candidates = vec![
            make_candidate("a", DeviceType::Desktop, 9999, PeerStatus::Offline),
        ];
        assert!(elect_coordinator(&candidates).is_none());
    }

    #[test]
    fn priority_ordering() {
        let p1 = ElectionPriority::new(DeviceType::Desktop, 5000, "aaa");
        let p2 = ElectionPriority::new(DeviceType::Mobile, 9999, "aaa");
        assert!(p1 > p2, "Desktop should beat Mobile regardless of uptime");

        let p3 = ElectionPriority::new(DeviceType::Desktop, 1000, "aaa");
        let p4 = ElectionPriority::new(DeviceType::Desktop, 5000, "aaa");
        assert!(p4 > p3, "Longer uptime should win");

        let p5 = ElectionPriority::new(DeviceType::Desktop, 1000, "aaa");
        let p6 = ElectionPriority::new(DeviceType::Desktop, 1000, "zzz");
        assert!(p5 > p6, "Alphabetically first node_id should win");
    }
}
