// SPEC-10 §6.4 Tailscale-aware peer routing — integration tests.
//
// Covers task-2026052701 acceptance row: `PeerInfo` carries an optional
// `tailscale_ip` that, when populated from `tailscale status --json`,
// causes the dispatch URL to prefer the stable Tailscale address. The
// fixture mirrors the shape produced by `tailscale status --json` on a
// real tailnet (Self + Peer.* + offline + IPv6 + missing HostName).

use spectyn_mesh::mesh::{
    parse_tailscale_status_json, peer_dispatch_base_url, MeshError, PeerHealth, PeerInfo,
    TailscaleStatus,
};

const FIXTURE: &str = include_str!("fixtures/tailscale-status.json");

fn make_peer(name: &str, url: &str, tailscale_ip: Option<&str>) -> PeerInfo {
    PeerInfo {
        url: url.to_string(),
        name: name.to_string(),
        version: "0.6.0".into(),
        online: true,
        active_tasks: 0,
        uptime_secs: 0,
        last_seen_unix: 1_700_000_000,
        last_seen: None,
        consecutive_failures: 0,
        capabilities: vec![],
        health: PeerHealth::default(),
        tailscale_ip: tailscale_ip.map(|s| s.to_string()),
    }
}

#[test]
fn test_peer_info_prefers_tailscale_ip() {
    // 1. Parse the fixture exactly as if it came from `tailscale status --json`.
    let status: TailscaleStatus =
        parse_tailscale_status_json(FIXTURE.as_bytes()).expect("fixture parses");

    // 2. Online peers with an IPv4 + HostName appear in the lookup map.
    let node_a_ip = status.lookup("node-a").expect("node-a in map");
    assert_eq!(node_a_ip, "100.64.0.10", "node-a IPv4");
    let mac_ip = status.lookup("mac-mini").expect("mac-mini in map");
    assert_eq!(mac_ip, "100.64.0.20", "mac-mini IPv4");

    // 3. Offline peers and entries lacking HostName are filtered out.
    assert!(
        status.lookup("node-b").is_none(),
        "offline node-b must be filtered",
    );
    assert!(
        status.peers.values().all(|ip| !ip.contains(':')),
        "IPv6 entries must be dropped (spectyn dials IPv4 only)",
    );
    // `Self` must not bleed into the peer list — we only route to others.
    assert!(
        status.lookup("host-self").is_none(),
        "Self entry must be excluded from peers",
    );

    // 4. PeerInfo populated from the lookup feeds dispatch URL selection:
    //    with tailscale_ip = Some(...), the URL prefers the ts address.
    let peer_with_ts = make_peer(
        "node-a",
        "http://192.168.1.10:7878", // LAN-only address (fallback)
        Some(node_a_ip),
    );
    let url = peer_dispatch_base_url(&peer_with_ts);
    assert_eq!(
        url, "http://100.64.0.10:7878",
        "tailscale_ip Some ⇒ prefer ts base URL",
    );

    // 5. PeerInfo without a known ts ip falls back to peer.url (pre-Tailscale
    //    behaviour — single-network deploys see zero change).
    let peer_without_ts = make_peer("lan-only", "http://192.168.1.20:7878/", None);
    assert_eq!(
        peer_dispatch_base_url(&peer_without_ts),
        "http://192.168.1.20:7878",
        "tailscale_ip None ⇒ peer.url (trailing slash trimmed)",
    );
}

#[test]
fn parse_handles_empty_tailnet() {
    // Single-node tailnet — Self present, no Peer object.
    let body = br#"{"Self":{"HostName":"alone","Online":true,"TailscaleIPs":["100.64.0.1"]}}"#;
    let status = parse_tailscale_status_json(body).expect("parse");
    assert!(status.peers.is_empty());
}

#[test]
fn parse_rejects_garbage_bytes() {
    let err = parse_tailscale_status_json(b"not-json").expect_err("must reject");
    assert!(matches!(err, MeshError::JsonParse(_)), "got {err:?}");
}

#[test]
fn peer_info_serde_round_trip_preserves_tailscale_ip() {
    // Backwards-compat: existing peers.json files predate the field.
    // Round-trip must succeed AND new field must default to None on
    // missing input. Plus serialise an explicit Some value back.
    let body = serde_json::json!({
        "url": "http://a:7878",
        "name": "A",
        "version": "0.6.0",
        "online": true,
        "active_tasks": 0,
        "uptime_secs": 0,
        "last_seen_unix": 0,
        "consecutive_failures": 0,
    });
    let p: PeerInfo = serde_json::from_value(body).expect("legacy peers.json parses");
    assert!(
        p.tailscale_ip.is_none(),
        "missing tailscale_ip ⇒ default None",
    );

    let mut p2 = make_peer("node-a", "http://lan:7878", Some("100.64.0.10"));
    p2.last_seen_unix = 42;
    let json = serde_json::to_string(&p2).expect("serialise");
    assert!(
        json.contains("\"tailscale_ip\":\"100.64.0.10\""),
        "serialised form carries ts ip: {json}",
    );
    let back: PeerInfo = serde_json::from_str(&json).expect("re-parse");
    assert_eq!(back.tailscale_ip.as_deref(), Some("100.64.0.10"));
}
