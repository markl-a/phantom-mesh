//! Integration tests for the P2P cluster layer (peer registry + election).

use std::time::{SystemTime, UNIX_EPOCH};

// Use the crate's public re-exports from networking/mod.rs.
use phantom_mesh::networking::peer::{DeviceType, PeerInfo, PeerRegistry, PeerStatus};
use phantom_mesh::networking::election::{
    elect_coordinator, should_trigger_election, ElectionConfig,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn make_peer(
    id: &str,
    device_type: DeviceType,
    status: PeerStatus,
    last_seen: u64,
    capabilities: Vec<&str>,
    active_tasks: usize,
    cpu_load: f32,
    uptime_secs: u64,
) -> PeerInfo {
    PeerInfo {
        node_id: id.to_string(),
        name: format!("node-{id}"),
        addr: "127.0.0.1:7878".to_string(),
        device_type,
        capabilities: capabilities.into_iter().map(String::from).collect(),
        cpu_load,
        memory_pct: 40.0,
        active_tasks,
        uptime_secs,
        last_seen,
        status,
    }
}

fn simple_peer(id: &str, status: PeerStatus, last_seen: u64) -> PeerInfo {
    make_peer(id, DeviceType::Desktop, status, last_seen, vec!["llm"], 0, 0.5, 3600)
}

// ===========================================================================
// PeerRegistry tests
// ===========================================================================

#[test]
fn test_peer_registry_upsert_and_get() {
    let reg = PeerRegistry::new();
    let now = now_secs();
    let peer = simple_peer("aabbccdd", PeerStatus::Online, now);
    reg.upsert(peer);

    let got = reg.get("aabbccdd").expect("peer should exist");
    assert_eq!(got.node_id, "aabbccdd");
    assert_eq!(got.name, "node-aabbccdd");
    assert_eq!(got.status, PeerStatus::Online);
    assert_eq!(got.last_seen, now);
}

#[test]
fn test_peer_registry_mark_stale() {
    // Use a short stale timeout so we can test easily.
    let reg = PeerRegistry::with_stale_timeout(5);
    let old_ts = now_secs().saturating_sub(60); // 60 seconds ago
    reg.upsert(simple_peer("stale1", PeerStatus::Online, old_ts));

    let went_offline = reg.mark_stale();
    assert!(went_offline.contains(&"stale1".to_string()));

    let peer = reg.get("stale1").unwrap();
    assert_eq!(peer.status, PeerStatus::Offline);
}

#[test]
fn test_peer_registry_online_peers() {
    let reg = PeerRegistry::new();
    let now = now_secs();

    reg.upsert(simple_peer("a", PeerStatus::Online, now));
    reg.upsert(simple_peer("b", PeerStatus::Online, now));
    reg.upsert(simple_peer("c", PeerStatus::Offline, now));

    let online = reg.online_peers();
    assert_eq!(online.len(), 2);

    let ids: Vec<String> = online.iter().map(|p| p.node_id.clone()).collect();
    assert!(ids.contains(&"a".to_string()));
    assert!(ids.contains(&"b".to_string()));
}

#[test]
fn test_peer_registry_coordinator() {
    let reg = PeerRegistry::new();
    assert!(reg.coordinator().is_none());

    reg.set_coordinator("leader-node");
    assert_eq!(reg.coordinator().unwrap(), "leader-node");
}

#[test]
fn test_peer_registry_capability_search() {
    let reg = PeerRegistry::new();
    let now = now_secs();

    reg.upsert(make_peer("n1", DeviceType::Desktop, PeerStatus::Online, now, vec!["llm", "code_exec"], 0, 0.1, 100));
    reg.upsert(make_peer("n2", DeviceType::Desktop, PeerStatus::Online, now, vec!["llm"], 0, 0.2, 100));
    reg.upsert(make_peer("n3", DeviceType::Mobile, PeerStatus::Online, now, vec!["code_exec"], 0, 0.3, 100));
    // Offline peer with llm — should NOT be returned.
    reg.upsert(make_peer("n4", DeviceType::Desktop, PeerStatus::Offline, now, vec!["llm"], 0, 0.1, 100));

    let llm_peers = reg.peers_with_capability("llm");
    assert_eq!(llm_peers.len(), 2);
    let ids: Vec<String> = llm_peers.iter().map(|p| p.node_id.clone()).collect();
    assert!(ids.contains(&"n1".to_string()));
    assert!(ids.contains(&"n2".to_string()));

    let exec_peers = reg.peers_with_capability("code_exec");
    assert_eq!(exec_peers.len(), 2);
}

#[test]
fn test_peer_registry_least_loaded() {
    let reg = PeerRegistry::new();
    let now = now_secs();

    // n1: 3 tasks, low cpu
    reg.upsert(make_peer("n1", DeviceType::Desktop, PeerStatus::Online, now, vec!["llm"], 3, 0.1, 100));
    // n2: 1 task, high cpu
    reg.upsert(make_peer("n2", DeviceType::Desktop, PeerStatus::Online, now, vec!["llm"], 1, 0.9, 100));
    // n3: 1 task, low cpu — should win (fewest tasks, then lowest cpu)
    reg.upsert(make_peer("n3", DeviceType::Desktop, PeerStatus::Online, now, vec!["llm"], 1, 0.1, 100));

    let best = reg.least_loaded_with_capability("llm").unwrap();
    assert_eq!(best.node_id, "n3");
}

// ===========================================================================
// Election tests
// ===========================================================================

fn election_candidate(id: &str, dt: DeviceType, uptime: u64) -> PeerInfo {
    make_peer(id, dt, PeerStatus::Online, now_secs(), vec![], 0, 0.0, uptime)
}

#[test]
fn test_election_desktop_wins_over_mobile() {
    let candidates = vec![
        election_candidate("mob1", DeviceType::Mobile, 99999),
        election_candidate("desk1", DeviceType::Desktop, 100),
    ];
    assert_eq!(elect_coordinator(&candidates).unwrap(), "desk1");
}

#[test]
fn test_election_longer_uptime_wins() {
    let candidates = vec![
        election_candidate("a", DeviceType::Desktop, 1000),
        election_candidate("b", DeviceType::Desktop, 5000),
    ];
    assert_eq!(elect_coordinator(&candidates).unwrap(), "b");
}

#[test]
fn test_election_node_id_tiebreaker() {
    let candidates = vec![
        election_candidate("zzz", DeviceType::Desktop, 1000),
        election_candidate("aaa", DeviceType::Desktop, 1000),
    ];
    // Alphabetically first ("aaa") wins.
    assert_eq!(elect_coordinator(&candidates).unwrap(), "aaa");
}

#[test]
fn test_should_trigger_election_no_coordinator() {
    let reg = PeerRegistry::new();
    // No coordinator set → should trigger.
    assert!(should_trigger_election(&reg, "me", now_secs(), &ElectionConfig::default()));
}

#[test]
fn test_should_trigger_election_stale_coordinator() {
    let reg = PeerRegistry::new();
    let now = now_secs();

    // Coordinator exists and is online, but its last_seen is 60 s ago.
    reg.upsert(simple_peer("coord", PeerStatus::Online, now));
    reg.set_coordinator("coord");

    let stale_last_seen = now.saturating_sub(60);
    let config = ElectionConfig::default(); // heartbeat_miss_secs = 30
    assert!(should_trigger_election(&reg, "me", stale_last_seen, &config));
}

#[test]
fn test_should_trigger_election_healthy() {
    let reg = PeerRegistry::new();
    let now = now_secs();

    reg.upsert(simple_peer("coord", PeerStatus::Online, now));
    reg.set_coordinator("coord");

    // Coordinator seen just now → no election needed.
    let config = ElectionConfig::default();
    assert!(!should_trigger_election(&reg, "me", now, &config));
}
