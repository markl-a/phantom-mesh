//! bench_cluster_forwarding
//!
//! F004 / E001 — Cross-host forwarding latency budget instrumentation.
//!
//! Goal
//! ----
//! E001's perf acceptance bar is "Cross-host forwarding latency p99 < 500ms
//! on a LAN". That 500ms budget is dominated by network + HMAC + JSON
//! round-trip; the *in-process* selection cost on the dispatcher side has
//! to be a vanishing fraction of it (~microseconds, not milliseconds) for
//! the budget to be reachable.
//!
//! This bench measures only the in-process slice:
//!   * `mesh::select_best_peer_with_caps` over peer inventories of size
//!     N = 10, 100, 1000.
//!
//! The LAN end-to-end p50/p95/p99 measurement lives in the sibling bash
//! scenario `scripts/phantom-test/scenarios/cross_host_perf.sh`, which
//! drives two real `phantom serve` instances and asserts the 500ms p99
//! budget on real hardware. The two together cover both halves of the
//! perf-gate test matrix row from F004's spec.
//!
//! Conventions
//! -----------
//! * No feature flags — runs under default `cargo bench`.
//! * Pure-fn / in-process only — no tokio runtime, no sockets, no HMAC.
//! * Uses the shared `common::standard_criterion` config so suite runtime
//!   stays under ~90s even with the new bench added.
//! * Prints a budget-comparison footer so an operator scanning bench
//!   output can sanity-check "selection is nowhere near the 500ms LAN
//!   budget" at a glance.

#[path = "common/mod.rs"]
mod common;

use std::time::Instant;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use phantom_mesh::mesh::{select_best_peer_with_caps, PeerHealth, PeerInfo};

/// Build a `PeerInfo` inventory of `n` online, healthy peers. Capabilities
/// are sharded across three tags so that `required_caps = ["shell.write"]`
/// matches roughly 1/3 of the inventory — realistic for a heterogeneous
/// mesh and forces the selector to walk the full list (worst-ish case).
fn build_peers(n: usize) -> Vec<PeerInfo> {
    const CAPS: &[&[&str]] = &[
        &["shell.write", "network"],
        &["memory", "vision"],
        &["gpu", "shell.write"],
    ];
    (0..n)
        .map(|i| {
            let caps = CAPS[i % CAPS.len()];
            PeerInfo {
                url: format!("http://10.0.0.{}:7878", (i % 250) + 1),
                name: format!("peer-{i:04}"),
                version: "bench".into(),
                online: true,
                active_tasks: (i % 5) as u32,
                uptime_secs: 60 * i as u64,
                // Stagger last_seen so the tie-break path
                // (cmp by last_seen_unix) still does meaningful work.
                last_seen_unix: 1_700_000_000 + i as u64,
                last_seen: None,
                consecutive_failures: 0,
                capabilities: caps.iter().map(|s| (*s).to_string()).collect(),
                health: PeerHealth::default(), // Healthy
                tailscale_ip: None,
            }
        })
        .collect()
}

fn bench_select(c: &mut Criterion) {
    let mut group = c.benchmark_group("select_best_peer_with_caps");
    let required = vec!["shell.write".to_string()];

    for &n in &[10usize, 100, 1000] {
        let peers = build_peers(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &peers, |b, peers| {
            b.iter(|| {
                let picked = select_best_peer_with_caps(black_box(&required), black_box(peers));
                black_box(picked);
            });
        });
    }

    group.finish();

    // ── Budget-comparison footer ───────────────────────────────────────
    // Run one untimed sample per N so we can print a concrete µs number
    // alongside the 500ms LAN budget for operator-eyeball comparison.
    // This is intentionally informational; Criterion's HTML report has
    // the rigorous numbers.
    eprintln!();
    eprintln!("── F004 budget context ─────────────────────────────────────");
    eprintln!("  E001 acceptance bar: cross-host forwarding p99 < 500 ms (LAN)");
    eprintln!("  This bench measures only the in-process selection slice.");
    eprintln!("  Network + HMAC + JSON round-trip dominates the LAN budget;");
    eprintln!("  selection should be ~µs (≪ 1 ms), not ms.");
    eprintln!();
    eprintln!("  one-shot wall-clock samples (informational, not statistical):");
    for &n in &[10usize, 100, 1000] {
        let peers = build_peers(n);
        // Warm.
        let _ = select_best_peer_with_caps(&required, &peers);
        let iters = 10_000u64;
        let t0 = Instant::now();
        for _ in 0..iters {
            let p = select_best_peer_with_caps(black_box(&required), black_box(&peers));
            black_box(p);
        }
        let avg_ns = (t0.elapsed().as_nanos() as f64) / (iters as f64);
        let avg_us = avg_ns / 1000.0;
        let pct_of_budget = (avg_ns / 1_000_000.0) / 500.0 * 100.0;
        eprintln!(
            "    N = {n:>4}  avg = {avg_us:>8.3} µs  ({pct_of_budget:>7.4}% of 500 ms LAN budget)"
        );
    }
    eprintln!("────────────────────────────────────────────────────────────");
}

criterion_group! {
    name = benches;
    config = common::standard_criterion();
    targets = bench_select
}
criterion_main!(benches);
