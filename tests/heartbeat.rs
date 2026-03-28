//! Integration tests for the heartbeat module.

use phantom_mesh::networking::heartbeat::{
    next_interval, process_heartbeat, HeartbeatConfig, HeartbeatPayload,
};
use phantom_mesh::networking::peer::{PeerRegistry, PeerStatus};

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
    assert_eq!(next_interval(&cfg, 3), 120); // 15 * 8 = 120, capped at max_backoff_secs
}

#[test]
fn test_next_interval_max_backoff() {
    let cfg = HeartbeatConfig::default();
    // Even with 10 consecutive failures, interval is capped at 120.
    assert_eq!(next_interval(&cfg, 10), 120);
}

#[test]
fn test_process_heartbeat_registers_peer() {
    let reg = PeerRegistry::new();
    let payload = make_payload("node-1", "Alpha");
    let resp = process_heartbeat(&reg, &payload, "coord-0");

    // Peer should now exist in the registry.
    let peer = reg.get("node-1").expect("peer should be registered");
    assert_eq!(peer.name, "Alpha");
    assert_eq!(peer.status, PeerStatus::Online);
    assert_eq!(resp.coordinator_id, "coord-0");
    assert_eq!(resp.status, "ok");
}

#[test]
fn test_process_heartbeat_updates_load() {
    let reg = PeerRegistry::new();
    let mut payload = make_payload("node-1", "Alpha");
    process_heartbeat(&reg, &payload, "coord-0");

    // Send a second heartbeat with updated cpu_load.
    payload.cpu_load = 0.95;
    process_heartbeat(&reg, &payload, "coord-0");

    let peer = reg.get("node-1").unwrap();
    assert!((peer.cpu_load - 0.95).abs() < f32::EPSILON);
    // Should still be exactly 1 peer (updated, not duplicated).
    assert_eq!(reg.all_peers().len(), 1);
}

#[test]
fn test_process_heartbeat_response_has_peers() {
    let reg = PeerRegistry::new();
    process_heartbeat(&reg, &make_payload("n1", "A"), "coord-0");
    process_heartbeat(&reg, &make_payload("n2", "B"), "coord-0");
    process_heartbeat(&reg, &make_payload("n3", "C"), "coord-0");

    // Re-heartbeat n1 and inspect the response.
    let resp = process_heartbeat(&reg, &make_payload("n1", "A"), "coord-0");
    assert_eq!(resp.known_peers.len(), 3);

    let ids: Vec<&str> = resp.known_peers.iter().map(|p| p.node_id.as_str()).collect();
    assert!(ids.contains(&"n1"));
    assert!(ids.contains(&"n2"));
    assert!(ids.contains(&"n3"));
}
