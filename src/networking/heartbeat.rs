//! Heartbeat protocol for Phantom Mesh cluster.
//!
//! Each node sends a [`HeartbeatPayload`] to the Coordinator every 15 seconds
//! (configurable via [`HeartbeatConfig`]). The Coordinator processes each
//! heartbeat via [`process_heartbeat`], updating its [`super::peer::PeerRegistry`]
//! and returning a [`HeartbeatResponse`] with the current peer list.
//!
//! On failure the sender backs off exponentially (see [`next_interval`]).

use serde::{Deserialize, Serialize};

use super::peer::{DeviceType, PeerInfo, PeerRegistry, PeerStatus};

// ---------------------------------------------------------------------------
// Payload & compact summaries
// ---------------------------------------------------------------------------

/// Heartbeat payload sent from each node to the Coordinator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatPayload {
    pub node_id: String,
    pub name: String,
    pub addr: String,
    /// `"desktop"` or `"mobile"`.
    pub device_type: String,
    pub capabilities: Vec<String>,
    pub cpu_load: f32,
    pub memory_pct: f32,
    pub active_tasks: usize,
    pub uptime_secs: u64,
    // Sync data — lightweight summaries for cross-node index
    pub conversation_count: usize,
    pub task_count: usize,
    pub recent_conversations: Vec<ConversationSummaryCompact>,
    pub recent_tasks: Vec<TaskSummaryCompact>,
}

/// Compact conversation summary included in heartbeat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummaryCompact {
    pub chat_id: String,
    pub last_message_at: u64,
    pub message_count: usize,
}

/// Compact task summary included in heartbeat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSummaryCompact {
    pub task_id: String,
    pub title: String,
    pub status: String,
    pub updated_at: u64,
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

/// Heartbeat response from the Coordinator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatResponse {
    pub coordinator_id: String,
    pub known_peers: Vec<PeerSummary>,
    pub status: String, // "ok", "election_in_progress"
}

/// Minimal peer info shared via heartbeat response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerSummary {
    pub node_id: String,
    pub name: String,
    pub addr: String,
    pub status: String,
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Configuration for heartbeat behavior.
#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// Normal interval in seconds (default: 15).
    pub interval_secs: u64,
    /// Maximum backoff on consecutive failures (default: 120).
    pub max_backoff_secs: u64,
    /// Mark a peer offline after this many seconds without heartbeat (default: 30).
    pub stale_threshold_secs: u64,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            interval_secs: 15,
            max_backoff_secs: 120,
            stale_threshold_secs: 30,
        }
    }
}

// ---------------------------------------------------------------------------
// Backoff
// ---------------------------------------------------------------------------

/// Calculate next heartbeat interval with exponential backoff.
///
/// - 0 failures → `config.interval_secs`
/// - *n* failures → `interval * 2^n`, capped at `config.max_backoff_secs`
pub fn next_interval(config: &HeartbeatConfig, consecutive_failures: u32) -> u64 {
    if consecutive_failures == 0 {
        return config.interval_secs;
    }
    // Cap the exponent to avoid overflow (2^6 = 64, 15*64 = 960 > 120 already).
    let backoff = config
        .interval_secs
        .saturating_mul(2u64.saturating_pow(consecutive_failures.min(6)));
    backoff.min(config.max_backoff_secs)
}

// ---------------------------------------------------------------------------
// Processing
// ---------------------------------------------------------------------------

/// Parse a device-type string into the enum, defaulting to Desktop.
fn parse_device_type(s: &str) -> DeviceType {
    match s {
        "mobile" => DeviceType::Mobile,
        _ => DeviceType::Desktop,
    }
}

/// Format a [`PeerStatus`] as a lowercase string.
fn status_str(s: &PeerStatus) -> String {
    match s {
        PeerStatus::Online => "online".to_string(),
        PeerStatus::Offline => "offline".to_string(),
        PeerStatus::Unknown => "unknown".to_string(),
    }
}

/// Process an incoming heartbeat on the Coordinator side.
///
/// 1. Converts the [`HeartbeatPayload`] to a [`PeerInfo`] and upserts it into
///    the [`PeerRegistry`].
/// 2. Builds a [`HeartbeatResponse`] containing the current coordinator ID and
///    the full list of known peers.
pub fn process_heartbeat(
    registry: &PeerRegistry,
    payload: &HeartbeatPayload,
    coordinator_id: &str,
) -> HeartbeatResponse {
    // 1. Upsert the peer
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let peer = PeerInfo {
        node_id: payload.node_id.clone(),
        name: payload.name.clone(),
        addr: payload.addr.clone(),
        device_type: parse_device_type(&payload.device_type),
        capabilities: payload.capabilities.clone(),
        cpu_load: payload.cpu_load,
        memory_pct: payload.memory_pct,
        active_tasks: payload.active_tasks,
        uptime_secs: payload.uptime_secs,
        status: PeerStatus::Online,
        last_seen: now_secs,
    };
    registry.upsert(peer);

    // 2. Build response
    let known_peers = registry
        .all_peers()
        .into_iter()
        .map(|p| PeerSummary {
            node_id: p.node_id,
            name: p.name,
            addr: p.addr,
            status: status_str(&p.status),
        })
        .collect();

    HeartbeatResponse {
        coordinator_id: coordinator_id.to_string(),
        known_peers,
        status: "ok".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a minimal HeartbeatPayload for testing.
    fn make_payload(node_id: &str, name: &str) -> HeartbeatPayload {
        HeartbeatPayload {
            node_id: node_id.into(),
            name: name.into(),
            addr: format!("192.168.1.{}:7878", node_id.len()),
            device_type: "desktop".into(),
            capabilities: vec!["shell".into()],
            cpu_load: 0.25,
            memory_pct: 45.0,
            active_tasks: 1,
            uptime_secs: 600,
            conversation_count: 3,
            task_count: 2,
            recent_conversations: vec![],
            recent_tasks: vec![],
        }
    }

    #[test]
    fn test_next_interval_no_failures() {
        let cfg = HeartbeatConfig::default();
        assert_eq!(next_interval(&cfg, 0), 15);
    }

    #[test]
    fn test_next_interval_backoff() {
        let cfg = HeartbeatConfig::default();
        assert_eq!(next_interval(&cfg, 1), 30);  // 15 * 2
        assert_eq!(next_interval(&cfg, 2), 60);  // 15 * 4
        assert_eq!(next_interval(&cfg, 3), 120); // 15 * 8 = 120 (capped)
    }

    #[test]
    fn test_next_interval_max_backoff() {
        let cfg = HeartbeatConfig::default();
        // Even with many failures, should never exceed max_backoff_secs.
        assert_eq!(next_interval(&cfg, 10), 120);
    }

    #[test]
    fn test_process_heartbeat_registers_peer() {
        let reg = PeerRegistry::new();
        let payload = make_payload("node-1", "Alpha");
        let resp = process_heartbeat(&reg, &payload, "coord-0");

        let peer = reg.get("node-1").unwrap();
        assert_eq!(peer.name, "Alpha");
        assert_eq!(peer.status, PeerStatus::Online);
        assert_eq!(resp.coordinator_id, "coord-0");
    }

    #[test]
    fn test_process_heartbeat_updates_load() {
        let reg = PeerRegistry::new();
        let mut payload = make_payload("node-1", "Alpha");
        process_heartbeat(&reg, &payload, "coord-0");

        // Send a second heartbeat with different cpu_load
        payload.cpu_load = 0.95;
        process_heartbeat(&reg, &payload, "coord-0");

        let peer = reg.get("node-1").unwrap();
        assert!((peer.cpu_load - 0.95).abs() < f32::EPSILON);
        // Still only 1 peer (updated, not duplicated)
        assert_eq!(reg.all_peers().len(), 1);
    }

    #[test]
    fn test_process_heartbeat_response_has_peers() {
        let reg = PeerRegistry::new();
        process_heartbeat(&reg, &make_payload("n1", "A"), "coord-0");
        process_heartbeat(&reg, &make_payload("n2", "B"), "coord-0");
        process_heartbeat(&reg, &make_payload("n3", "C"), "coord-0");

        let resp = process_heartbeat(&reg, &make_payload("n1", "A"), "coord-0");
        assert_eq!(resp.known_peers.len(), 3);

        let ids: Vec<&str> = resp.known_peers.iter().map(|p| p.node_id.as_str()).collect();
        assert!(ids.contains(&"n1"));
        assert!(ids.contains(&"n2"));
        assert!(ids.contains(&"n3"));
    }

    #[test]
    fn heartbeat_payload_serialization_roundtrip() {
        let payload = make_payload("n1", "TestNode");
        let json = serde_json::to_string(&payload).unwrap();
        let deser: HeartbeatPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.node_id, "n1");
        assert_eq!(deser.name, "TestNode");
    }

    #[test]
    fn heartbeat_response_serialization_roundtrip() {
        let resp = HeartbeatResponse {
            coordinator_id: "c1".into(),
            known_peers: vec![PeerSummary {
                node_id: "n1".into(),
                name: "A".into(),
                addr: "1.2.3.4:7878".into(),
                status: "online".into(),
            }],
            status: "ok".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deser: HeartbeatResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.coordinator_id, "c1");
        assert_eq!(deser.known_peers.len(), 1);
    }

    #[test]
    fn parse_device_type_variants() {
        assert_eq!(parse_device_type("desktop"), DeviceType::Desktop);
        assert_eq!(parse_device_type("mobile"), DeviceType::Mobile);
        assert_eq!(parse_device_type("unknown"), DeviceType::Desktop); // fallback
    }
}
