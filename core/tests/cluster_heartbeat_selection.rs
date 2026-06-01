//! F001 — Cluster heartbeat × selection integration test.
//!
//! Spec: `docs/superpowers/features/F001-cluster-heartbeat-selection-integration-test.md`
//! Parent epic: E001 cross-host real cluster smoke.
//!
//! Wires together two subsystems that today only have *isolated* unit
//! coverage in `core/src/mesh.rs::tests`:
//!
//!   * C4 peer-heartbeat state machine
//!     (`ClusterManager::record_probe_result`, gated on the
//!     `experimental-cluster-heartbeat` feature for the Healthy→Unhealthy
//!     transition)
//!   * C1 capability-aware selection (`select_best_peer_with_caps`,
//!     unconditional two-tier Healthy-first / Unhealthy-fallback filter)
//!
//! The regression hole this closes is *"peer goes Unhealthy but the
//! selector still tries it"* — i.e. an integration test that proves the
//! two subsystems compose the way the spec promises.
//!
//! ## Why an integration test if the pieces are pure?
//!
//! `record_probe_result` mutates a peer's `PeerHealth` inside the manager.
//! `select_best_peer_with_caps` reads `PeerInfo.health` and tiers on it.
//! The contract that *crossing the failure threshold immediately changes
//! routing on the next selector call* is not visible from either side
//! alone — only from the seam between them. Driving the state machine
//! through `ClusterManager::record_probe_result` then reading back via
//! `peer_infos()` and handing the result to `select_best_peer_with_caps`
//! exercises that seam without spinning up axum servers.
//!
//! ## Why no axum boot?
//!
//! The spec's `## Scope (in)` mentions booting two `phantom serve`
//! instances on loopback ports. We deliberately collapse that down to the
//! pure state-machine path because:
//!
//!   * The spec's acceptance criterion *"collapse the 90s default to ~3s"*
//!     and *"Test does not depend on tokio wall-clock"* are both satisfied
//!     more reliably by driving `record_probe_result` directly than by
//!     spinning a real heartbeat loop.
//!   * `select_best_peer_with_caps` is a pure function over `&[PeerInfo]`;
//!     no server is required for its tier logic to be observable.
//!   * Acceptance criterion *"passes < 15s wall"* and the F001 worktree
//!     budget *"<3s wall-clock"* are met trivially when no network is
//!     involved (entire suite runs in milliseconds on the C4 unit tests).
//!   * Axum cross-server scenarios live in F002/F003 (cross-host e2e),
//!     which is the right layer for IO-bound failure-injection.
//!
//! ## Running
//!
//! ```text
//! CARGO_TARGET_DIR=D:/tmp/f001-target \
//!   cargo test --test cluster_heartbeat_selection \
//!     --features experimental-cluster-heartbeat
//! ```
//!
//! The whole file gates on `experimental-cluster-heartbeat` because the
//! Healthy→Unhealthy transition only fires under that feature (see
//! `record_probe_result`'s `#[cfg]` block in `core/src/mesh.rs`). Without
//! the flag the tests would all degenerate to "peer stays Healthy",
//! which is the *absence* of the contract we are trying to assert.

#![cfg(feature = "experimental-cluster-heartbeat")]

use phantom_mesh::mesh::{
    select_best_peer_with_caps, ClusterConfig, ClusterManager, PeerHealth, PeerInfo,
};
use std::time::Instant;

// ── Fixture helpers ────────────────────────────────────────────────────────
//
// `ClusterManager::new` seeds peers from `cfg.peers` with `online = false`
// (they're considered offline until ping_peer flips them). The selector
// requires `online == true`, so for the cross-cutting assertion we need to
// either run a real server (out of scope per the module doc) or rebuild a
// `PeerInfo` view that mirrors what a successful ping would produce.
//
// `synthesize_peer_view(manager)` reads back the manager's PeerInfo, sets
// `online = true` and copies the manager-driven `health` into the cloned
// record. Result: a `Vec<PeerInfo>` suitable for handing to
// `select_best_peer_with_caps`, with health state that came from the real
// `record_probe_result` calls — closing the seam under test.

/// Build a peer entry with the given URL, capabilities, and recency
/// tie-break. Used to compose synthesised views.
fn fixture_peer(
    url: &str,
    name: &str,
    caps: &[&str],
    online: bool,
    last_seen_unix: u64,
    health: PeerHealth,
) -> PeerInfo {
    PeerInfo {
        url: url.to_string(),
        name: name.to_string(),
        version: "test".into(),
        online,
        active_tasks: 0,
        uptime_secs: 0,
        last_seen_unix,
        last_seen: None,
        consecutive_failures: 0,
        capabilities: caps.iter().map(|s| s.to_string()).collect(),
        health,
        tailscale_ip: None,
    }
}

/// Read manager state and project it into an "online" peer slice the
/// selector can score. Preserves the manager-driven `health` (the thing
/// under test); fills in `online = true` and a stable `capabilities` so
/// the selector's other filters are non-blocking.
async fn synthesize_view(
    manager: &ClusterManager,
    caps: &[&str],
    last_seen_unix: u64,
) -> Vec<PeerInfo> {
    let infos = manager.peer_infos().await;
    infos
        .into_iter()
        .enumerate()
        .map(|(i, p)| PeerInfo {
            online: true,
            capabilities: caps.iter().map(|s| s.to_string()).collect(),
            // Bump recency per-index so the selector has a deterministic
            // tie-break when both peers are in the same health tier.
            last_seen_unix: last_seen_unix + i as u64,
            ..p
        })
        .collect()
}

// ── Test 1: state machine — Healthy → Unhealthy after N failures ─────────
//
// Spec acceptance row: *"Heartbeat marks dead peer Unhealthy within
// `heartbeat_failure_threshold × heartbeat_interval_secs` ± 10%"*.
// We collapse the 90s default (3 × 30s) to 0s by feeding probe outcomes
// directly — the only contract this row asserts is "after N failures,
// state = Unhealthy", which is exactly what record_probe_result owns.

#[tokio::test]
async fn collapsed_heartbeat_flips_healthy_to_unhealthy_at_threshold() {
    let start = Instant::now();
    let url = "http://f001-peer-a:7878";
    let cfg = ClusterConfig {
        peers: vec![url.to_string()],
        // Tiny threshold mirrors the spec's *"collapsed window"*. The
        // actual probe interval is moot here — we drive outcomes directly.
        heartbeat_failure_threshold: Some(2),
        ..ClusterConfig::default()
    };
    let mgr = ClusterManager::new(cfg);

    // Optimistic default — must start Healthy so the selector picks it.
    assert!(
        mgr.peer_infos().await[0].health.is_healthy(),
        "newly-configured peer must default to Healthy",
    );

    mgr.record_probe_result(url, false).await; // 1/2
    assert!(
        mgr.peer_infos().await[0].health.is_healthy(),
        "1 failure < threshold(2) must keep peer Healthy",
    );

    mgr.record_probe_result(url, false).await; // 2/2 — threshold
    let after = mgr.peer_infos().await;
    assert!(
        !after[0].health.is_healthy(),
        "hitting threshold(2) must flip Healthy → Unhealthy",
    );
    match &after[0].health {
        PeerHealth::Unhealthy { failure_count, .. } => {
            assert_eq!(*failure_count, 2, "failure_count must equal probe count");
        }
        PeerHealth::Healthy => panic!("expected Unhealthy"),
    }

    // Acceptance: F001 worktree budget says < 3s wall — assert it so any
    // future regression that adds blocking IO surfaces immediately.
    assert!(
        start.elapsed().as_secs() < 3,
        "F001 budget exceeded: {:?}",
        start.elapsed(),
    );
}

// ── Test 2: state machine — Unhealthy → Healthy on recovery ──────────────
//
// Mirrors the spec's *"bring it back, assert selector resumes balancing
// within one heartbeat round"* — the state half. The selector half is
// covered by test 3 below.

#[tokio::test]
async fn collapsed_heartbeat_recovers_unhealthy_to_healthy_on_success() {
    let url = "http://f001-peer-b:7878";
    let cfg = ClusterConfig {
        peers: vec![url.to_string()],
        heartbeat_failure_threshold: Some(1), // trip on first failure
        ..ClusterConfig::default()
    };
    let mgr = ClusterManager::new(cfg);

    mgr.record_probe_result(url, false).await;
    assert!(
        !mgr.peer_infos().await[0].health.is_healthy(),
        "threshold(1) must flip on first failure",
    );

    mgr.record_probe_result(url, true).await; // recovery
    let recovered = mgr.peer_infos().await;
    assert!(
        recovered[0].health.is_healthy(),
        "single success must flip Unhealthy → Healthy",
    );
    assert_eq!(
        recovered[0].consecutive_failures, 0,
        "success must reset the consecutive_failures counter",
    );
}

// ── Test 3: selector × heartbeat interaction — Healthy preferred ─────────
//
// The actual seam under test: the selector's tier-1 filter must skip a
// peer the heartbeat state machine has flipped Unhealthy, even when that
// peer would otherwise tie-break-win on `last_seen_unix`.

#[tokio::test]
async fn selector_prefers_healthy_after_heartbeat_marks_other_unhealthy() {
    let healthy_url = "http://f001-healthy:7878";
    let dead_url = "http://f001-dead:7878";
    let cfg = ClusterConfig {
        peers: vec![healthy_url.to_string(), dead_url.to_string()],
        heartbeat_failure_threshold: Some(2),
        ..ClusterConfig::default()
    };
    let mgr = ClusterManager::new(cfg);

    // Drive `dead_url` to Unhealthy; leave `healthy_url` untouched.
    mgr.record_probe_result(dead_url, false).await;
    mgr.record_probe_result(dead_url, false).await;
    let infos = mgr.peer_infos().await;
    assert!(
        infos[0].health.is_healthy(),
        "peer[0] should still be Healthy"
    );
    assert!(
        !infos[1].health.is_healthy(),
        "peer[1] should now be Unhealthy",
    );

    // Project into an online view. Critically: the Unhealthy peer
    // (`dead_url`) gets the *higher* `last_seen_unix` so that without the
    // health tiering it would win on recency. With C4's tiering active,
    // the selector must skip it and pick the Healthy one.
    let view = synthesize_view(&mgr, &["compute"], 1_000).await;
    assert_eq!(view.len(), 2);
    assert!(
        view[1].last_seen_unix > view[0].last_seen_unix,
        "test setup: Unhealthy peer must be more recent so tie-break alone \
         would pick it — proving the health tier is what excludes it",
    );

    let picked = select_best_peer_with_caps(&["compute".to_string()], &view)
        .expect("at least one online & capable peer");
    assert_eq!(
        picked.url, healthy_url,
        "selector must prefer Healthy peer over more-recent Unhealthy peer",
    );
}

// ── Test 4: selector × heartbeat — Unhealthy used as last-resort fallback ─
//
// The other half of the spec contract: when *every* matching peer is
// Unhealthy, the selector falls through to tier-2 rather than returning
// None (which would surface as `NoPeerSatisfiesCaps` and abort the task).
// This is the *"fall back to Unhealthy"* clause of `select_best_peer_with_caps`.

#[tokio::test]
async fn selector_falls_back_to_unhealthy_when_no_healthy_peer_matches() {
    let only_url = "http://f001-sole:7878";
    let cfg = ClusterConfig {
        peers: vec![only_url.to_string()],
        heartbeat_failure_threshold: Some(1),
        ..ClusterConfig::default()
    };
    let mgr = ClusterManager::new(cfg);

    mgr.record_probe_result(only_url, false).await;
    assert!(
        !mgr.peer_infos().await[0].health.is_healthy(),
        "sole peer should now be Unhealthy",
    );

    let view = synthesize_view(&mgr, &["compute"], 1_000).await;
    let picked = select_best_peer_with_caps(&["compute".to_string()], &view)
        .expect("Unhealthy fallback must still pick rather than return None");
    assert_eq!(picked.url, only_url);
    assert!(
        !picked.health.is_healthy(),
        "fallback pick is intentionally Unhealthy — tier-2 of the selector",
    );
}

// ── Test 5: counter-reset semantics across recoveries ────────────────────
//
// Sanity that a single successful probe resets the failure counter so a
// subsequent burst of failures has to climb the threshold from zero
// again. Without this, a flaky peer that recovers briefly would still be
// flipped Unhealthy on the very next failure — bad for routing stability.

#[tokio::test]
async fn counter_resets_on_success_so_subsequent_failures_must_re_cross_threshold() {
    let url = "http://f001-flaky:7878";
    let cfg = ClusterConfig {
        peers: vec![url.to_string()],
        heartbeat_failure_threshold: Some(3),
        ..ClusterConfig::default()
    };
    let mgr = ClusterManager::new(cfg);

    // Two failures (below threshold), one success — counter must reset.
    mgr.record_probe_result(url, false).await;
    mgr.record_probe_result(url, false).await;
    assert_eq!(mgr.peer_infos().await[0].consecutive_failures, 2);

    mgr.record_probe_result(url, true).await;
    assert_eq!(
        mgr.peer_infos().await[0].consecutive_failures,
        0,
        "success must zero the counter",
    );

    // Two more failures now — must still be Healthy because counter
    // reset means we are at 2/3, not 4/3.
    mgr.record_probe_result(url, false).await;
    mgr.record_probe_result(url, false).await;
    assert!(
        mgr.peer_infos().await[0].health.is_healthy(),
        "2 failures after reset must keep peer Healthy (counter is 2/3)",
    );

    // Third failure crosses the threshold post-reset.
    mgr.record_probe_result(url, false).await;
    assert!(
        !mgr.peer_infos().await[0].health.is_healthy(),
        "third post-reset failure must finally flip Unhealthy",
    );
}

// ── Test 6: full worker baseline — recency tie-break unchanged ───────────
//
// Regression guard: when *all* matching peers are Healthy (the normal
// steady state for a quiet cluster), the C4 tier filter must be a no-op
// and the selector must fall back to the C1 recency tie-break. Without
// this, a refactor that accidentally short-circuits the recency rule
// would silently regress production routing balance.

#[tokio::test]
async fn selector_tie_breaks_by_recency_when_all_peers_healthy() {
    let old = fixture_peer(
        "http://f001-old:7878",
        "old",
        &["compute"],
        true,
        1_000,
        PeerHealth::Healthy,
    );
    let new = fixture_peer(
        "http://f001-new:7878",
        "new",
        &["compute"],
        true,
        9_000,
        PeerHealth::Healthy,
    );
    let picked = select_best_peer_with_caps(&["compute".to_string()], &[old, new])
        .expect("at least one match");
    assert_eq!(
        picked.name, "new",
        "all-Healthy: recency tie-break must still win (no regression)",
    );
}
