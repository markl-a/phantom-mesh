//! Phase E V6 — `coach_wire::aggregate` throughput budget gate.
//!
//! Per `docs/superpowers/PHASE-E-INTEGRATION-TEST-PLAN.md` §3.6 + SPEC-23 §13
//! perf budget, the pure-function coach aggregator MUST process the daily
//! event batch (N=100 events nominal — a heavy capture day) in **≤ 200 ms**
//! wall clock on a developer-class machine.
//!
//! These tests are **absolute-threshold pass/fail** — v0.6.0 GA mandate is
//! "did we meet the SPEC ceiling?", not "did we regress against last week".
//! Historical regression deltas live separately in `.perf-baseline/history.
//! jsonl` (V6 plan §3.6).
//!
//! No I/O, no async — `aggregate` is a pure deterministic markdown formatter
//! (Stage 3 real impl, see `core/src/coach_wire.rs` §7.1 docs). Timing uses
//! `std::time::Instant` only.
//!
//! Companion test: `core/tests/v7_perf_budgets.rs::v7_coach_aggregate_50_events_under_5ms_avg`
//! exercises the per-iter average; this file pins the **single-shot wall
//! clock** at the SPEC-listed batch sizes the GA gate cares about.

use std::time::{Duration, Instant};

use spectyn_mesh::coach_wire::aggregate;
use spectyn_mesh::event_storage_wire::{AnalysisResult, EventKind, EventMeta};

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Build N deterministic `(EventMeta, AnalysisResult)` pairs. Field shapes
/// mirror `core/tests/v7_perf_budgets.rs::v7_coach_aggregate_50_events_*` so
/// the two suites stay structurally aligned.
///
/// OSS-safe: ids / tags / model strings use generic placeholders
/// (`peer-alpha`-class) — no operator hostname, no Tailscale IPs, no email.
fn build_events(n: usize) -> Vec<(EventMeta, AnalysisResult)> {
    (0..n)
        .map(|i| {
            let meta = EventMeta {
                event_id: format!("evt-perf-{i:04}"),
                timestamp: "2026-05-26T12:00:00Z".to_string(),
                kind: match i % 3 {
                    0 => EventKind::Food,
                    1 => EventKind::Focus,
                    _ => EventKind::Habit,
                },
                tags: vec![
                    if i % 2 == 0 { "fat_loss" } else { "work" }.to_string(),
                ],
            };
            let analysis = AnalysisResult {
                summary: format!("event {i} brief summary line for the aggregator"),
                confidence: 0.8,
                goal_impact: "+0kcal vs target".to_string(),
                suggestion: "stay the course".to_string(),
                cost_usd: 0.0,
                latency_ms: 12,
                model_id: "mock:offline-deterministic".to_string(),
                raw_response: "{}".to_string(),
            };
            (meta, analysis)
        })
        .collect()
}

// ─── 1/4 — Small set: 10 events ────────────────────────────────────────────

/// **Budget**: aggregate of 10 events ≤ 50 ms wall clock.
///
/// Smallest realistic batch (a light capture day: 3 meals + 4 focus blocks +
/// 3 habit checkins). Conservative 50 ms ceiling — measured runtime should be
/// sub-millisecond. Failure here means a deep algorithm regression (e.g.
/// quadratic grouping replaced the BTreeMap path).
#[test]
fn v6_aggregator_small_set_10_events_under_50ms() {
    const BUDGET: Duration = Duration::from_millis(50);
    let events = build_events(10);

    let start = Instant::now();
    let brief = aggregate(&events);
    let elapsed = start.elapsed();

    assert!(brief.contains("# Daily review"), "header line missing");
    assert!(brief.contains("**Events captured:** 10"), "count line wrong: {brief}");
    assert!(
        elapsed <= BUDGET,
        "V6.AGG.1 aggregate(10) took {:?}, budget {:?}",
        elapsed,
        BUDGET
    );
    println!("V6.AGG.1 aggregate(10) = {:?} (budget {:?})", elapsed, BUDGET);
}

// ─── 2/4 — Medium set: 100 events (SPEC-listed nominal) ────────────────────

/// **Budget**: aggregate of 100 events ≤ 200 ms wall clock (SPEC-23 §13 line).
///
/// Nominal heavy-day batch per the orchestrator prompt: 100 events @ ~1 KB
/// each. This is the canonical V6 gate value pulled straight from the
/// `PHASE-E-INTEGRATION-TEST-PLAN.md` §3.6 table. Failure ⇒ ship-block.
#[test]
fn v6_aggregator_medium_set_100_events_under_200ms() {
    const BUDGET: Duration = Duration::from_millis(200);
    let events = build_events(100);

    let start = Instant::now();
    let brief = aggregate(&events);
    let elapsed = start.elapsed();

    assert!(brief.contains("**Events captured:** 100"), "count: {}", brief.lines().nth(2).unwrap_or(""));
    // Both tag buckets must surface as `## <tag> (N)` sections.
    assert!(brief.contains("## fat_loss"), "fat_loss section missing");
    assert!(brief.contains("## work"), "work section missing");
    assert!(
        elapsed <= BUDGET,
        "V6.AGG.2 aggregate(100) took {:?}, budget {:?}",
        elapsed,
        BUDGET
    );
    println!("V6.AGG.2 aggregate(100) = {:?} (budget {:?})", elapsed, BUDGET);
}

// ─── 3/4 — Large set: 500 events (5× the nominal) ──────────────────────────

/// **Budget**: aggregate of 500 events ≤ 1 s wall clock.
///
/// Stress sample at 5× nominal — accounts for power users who capture
/// continuously through a long day plus week-backfill scenarios. 1 s ceiling
/// is conservative (linear scale from the 200 ms / 100 evt nominal would
/// predict ~1 s @ 500 evt). Failure here means the aggregator grew
/// super-linear, which would degrade the worst-case UX badly.
#[test]
fn v6_aggregator_large_set_500_events_under_1s() {
    const BUDGET: Duration = Duration::from_millis(1_000);
    let events = build_events(500);

    let start = Instant::now();
    let brief = aggregate(&events);
    let elapsed = start.elapsed();

    assert!(brief.contains("**Events captured:** 500"), "count line wrong");
    assert!(
        elapsed <= BUDGET,
        "V6.AGG.3 aggregate(500) took {:?}, budget {:?}",
        elapsed,
        BUDGET
    );
    println!("V6.AGG.3 aggregate(500) = {:?} (budget {:?})", elapsed, BUDGET);
}

// ─── 4/4 — Empty input: degenerate path stays cheap ────────────────────────

/// **Budget**: aggregate of 0 events ≤ 10 ms wall clock.
///
/// Degenerate case — a brand-new install or backfill date with no captures.
/// The aggregator's empty-input branch must stay near-zero cost (returning
/// the canonical "no events for this date" placeholder per
/// `coach_wire::aggregate_empty_returns_placeholder_markdown` KAT). 10 ms
/// is the smallest measurable budget that survives CI scheduler jitter.
#[test]
fn v6_aggregator_empty_input_under_10ms() {
    const BUDGET: Duration = Duration::from_millis(10);

    let start = Instant::now();
    let brief = aggregate(&[]);
    let elapsed = start.elapsed();

    assert!(brief.contains("(no events for this date)"), "empty marker missing");
    assert!(
        elapsed <= BUDGET,
        "V6.AGG.4 aggregate(empty) took {:?}, budget {:?}",
        elapsed,
        BUDGET
    );
    println!("V6.AGG.4 aggregate(empty) = {:?} (budget {:?})", elapsed, BUDGET);
}
