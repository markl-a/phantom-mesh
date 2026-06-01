//! Phase E V6 — Time-To-First-Response (TTFR, 首次回應耗時) budget gate.
//!
//! Per `docs/superpowers/PHASE-E-INTEGRATION-TEST-PLAN.md` §3.6 + SPEC-28 §1 /
//! §12, the v0.6.0 GA north-star perf budget is **p95 ≤ 30 s** from the moment
//! a capture event hits the pipeline until the first coach reply chunk
//! materialises on screen.
//!
//! These tests are **absolute-threshold pass/fail** — they do NOT track
//! historical regression deltas (that lives in `.perf-baseline/history.jsonl`
//! per V6 plan §3.6 ). The single assertion per test is `elapsed <= BUDGET`.
//!
//! Because `coach_wire::run_daily_review`, `onboarding_wire::compute_ttfr`,
//! and `providers_wire::complete` all still bottom out in Stage 3 / Stage 4
//! `unimplemented!()` markers, the pipeline below is a deterministic in-memory
//! **simulator**: each stage (capture validation → aggregate markdown brief →
//! mock provider first-chunk delay) runs the real wire functions where Stage 3
//! is live, and substitutes a `std::thread::sleep` of conservative magnitude
//! for the remote-LLM round-trip. The mocked provider chunk arrival is
//! deliberately deterministic + offline so the suite never flakes on network
//! or quota.
//!
//! Stage 4 follow-up: once `providers_wire::complete` ships its real reqwest
//! HTTP adapter, the cold-start / warm-cache / slow-provider tests below can
//! migrate to the real call with `#[ignore = "integration / env-dependent"]`
//! flips per `core/tests/v4_chaos.rs` precedent.

use std::time::{Duration, Instant};

use phantom_mesh::capture_food_wire::{FoodCaptureRequest, FOOD_LOG_KIND};
use phantom_mesh::coach_wire::aggregate;
use phantom_mesh::event_storage_wire::{AnalysisResult, EventKind, EventMeta};

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Conservative TTFR budget per SPEC-28 §1 / §12 G1.
///
/// SPEC pins p95 ≤ 30 s — for a single-run gate we keep the same 30 s ceiling
/// since we are simulating the worst-case path (cold cluster + cold provider +
/// no warm cache). Faster paths (warm cache test) tighten the budget locally.
const TTFR_BUDGET: Duration = Duration::from_secs(30);

/// Build a deterministic `FoodCaptureRequest` fixture. OSS-safe: uses
/// `user42@example.com`-equivalent placeholder text only (no real PII, no
/// hostnames, no private IPs).
fn fixture_food_capture(note: &str, ts_ms: u64) -> FoodCaptureRequest {
    FoodCaptureRequest {
        text: Some(note.to_string()),
        image_path: None,
        kind: FOOD_LOG_KIND.to_string(),
        tag: vec!["fat_loss".to_string()],
        timestamp_ms: ts_ms,
    }
}

/// Project a `FoodCaptureRequest` into the `(EventMeta, AnalysisResult)` pair
/// the coach aggregator consumes. This mirrors what the live pipeline would do
/// after the LLM analysis pass completes (here we substitute a deterministic
/// `AnalysisResult` so the test stays offline + deterministic).
fn project_to_event_pair(
    req: &FoodCaptureRequest,
    event_id: &str,
    iso_ts: &str,
) -> (EventMeta, AnalysisResult) {
    let meta = EventMeta {
        event_id: event_id.to_string(),
        timestamp: iso_ts.to_string(),
        kind: EventKind::Food,
        tags: req.tag.clone(),
    };
    let analysis = AnalysisResult {
        summary: req.text.clone().unwrap_or_else(|| "(no note)".to_string()),
        confidence: 0.85,
        goal_impact: "within target band".to_string(),
        suggestion: "stay the course".to_string(),
        cost_usd: 0.0,
        latency_ms: 12,
        model_id: "mock:offline-deterministic".to_string(),
        raw_response: "{}".to_string(),
    };
    (meta, analysis)
}

/// Deterministic mocked-provider first-chunk delay. The orchestrator prompt
/// specifies the mock must be offline + deterministic; we use `thread::sleep`
/// with a small fixed magnitude so the wall-clock measurement still exercises
/// `Instant`-based timing without depending on network.
fn mock_provider_first_chunk(delay: Duration) {
    std::thread::sleep(delay);
}

// ─── 1/4 — Happy path: warm cluster, fast provider ─────────────────────────

/// **Budget**: TTFR ≤ 30 s (SPEC-28 §1 G1 absolute ceiling).
///
/// Happy path — small capture (1 food event), no image, warm-cluster
/// assumption (no cold-start mDNS browse + no demo-relay fallback). Simulated
/// provider responds within 100 ms. End-to-end wall clock MUST be well under
/// budget; this test fails loudly if any pure-CPU stage (aggregate, project,
/// fixture build) regresses to seconds-scale.
#[test]
fn v6_ttfr_happy_path_warm_provider_under_budget() {
    let start = Instant::now();

    // Stage 1 — capture: build the wire request (sub-millisecond pure-CPU).
    let req = fixture_food_capture("oatmeal + boiled eggs", 1_716_563_400_000);
    assert_eq!(req.kind, FOOD_LOG_KIND, "kind sentinel must stick");

    // Stage 2 — project to coach-input pair (pure-CPU; would be the LLM
    // analysis pass in the real pipeline — mocked here for offline determinism).
    let pair = project_to_event_pair(&req, "evt-warm-001", "2026-05-26T08:00:00Z");

    // Stage 3 — coach aggregate (real Stage 3 impl — pure markdown formatter).
    let brief = aggregate(std::slice::from_ref(&pair));
    assert!(brief.contains("# Daily review"), "brief must render header");

    // Stage 4 — mocked provider first-chunk arrival (100 ms warm provider).
    mock_provider_first_chunk(Duration::from_millis(100));

    let elapsed = start.elapsed();
    assert!(
        elapsed <= TTFR_BUDGET,
        "V6.TTFR.1 warm-provider TTFR {:?} exceeded budget {:?}",
        elapsed,
        TTFR_BUDGET
    );
    println!("V6.TTFR.1 warm-provider TTFR = {:?} (budget {:?})", elapsed, TTFR_BUDGET);
}

// ─── 2/4 — Cold start: simulated mDNS + identity warm-up ───────────────────

/// **Budget**: TTFR ≤ 30 s (SPEC-28 §1 G1).
///
/// Cold-start path — simulates the worst-case onboarding flow where the
/// device just installed and must (a) browse mDNS for peers, (b) HKDF-derive
/// the event key, (c) aggregate a single capture, (d) call provider. Each
/// stage gets a conservative simulated delay; total MUST still fit in 30 s.
#[test]
fn v6_ttfr_cold_start_under_budget() {
    let start = Instant::now();

    // Stage 0 — simulated mDNS browse / identity derive cost (cold cache).
    // Real Stage 4 mDNS browse on a quiet LAN measures ~1-3 s; we pick 500 ms
    // as a deterministic stand-in so the test stays fast in CI.
    mock_provider_first_chunk(Duration::from_millis(500));

    // Stage 1 — capture build.
    let req = fixture_food_capture("morning coffee", 1_716_563_700_000);

    // Stage 2 — project to coach pair.
    let pair = project_to_event_pair(&req, "evt-cold-001", "2026-05-26T08:05:00Z");

    // Stage 3 — coach aggregate (real impl).
    let brief = aggregate(std::slice::from_ref(&pair));
    assert!(brief.contains("**Events captured:** 1"), "brief count line");

    // Stage 4 — simulated cold provider first-chunk (300 ms — slower than warm
    // because TLS handshake + auth round-trip on first call).
    mock_provider_first_chunk(Duration::from_millis(300));

    let elapsed = start.elapsed();
    assert!(
        elapsed <= TTFR_BUDGET,
        "V6.TTFR.2 cold-start TTFR {:?} exceeded budget {:?}",
        elapsed,
        TTFR_BUDGET
    );
    println!("V6.TTFR.2 cold-start TTFR = {:?} (budget {:?})", elapsed, TTFR_BUDGET);
}

// ─── 3/4 — Warm cache: tighter local budget ────────────────────────────────

/// **Budget**: TTFR ≤ 2 s for the warm-cache path (local tighter ceiling).
///
/// When the user fires a second capture within the same session (cluster
/// already joined, provider connection alive, event key already cached), the
/// pipeline should respond near-instantly. We pin a tighter 2 s ceiling here
/// because exceeding that signals a regression in the warm path even though
/// the global SPEC budget is 30 s. The ceiling is still 20× the expected
/// measurement (~100 ms) so CI noise cannot flap this test.
#[test]
fn v6_ttfr_warm_cache_tight_budget() {
    const WARM_BUDGET: Duration = Duration::from_secs(2);
    let start = Instant::now();

    // No cold-start cost — caches are already populated.
    let req = fixture_food_capture("snack: apple", 1_716_564_000_000);
    let pair = project_to_event_pair(&req, "evt-warm-002", "2026-05-26T08:10:00Z");
    let brief = aggregate(std::slice::from_ref(&pair));
    assert!(brief.contains("## fat_loss (1)"), "brief must group by tag");

    // Mocked first-chunk arrival — 50 ms (warm path, TLS keepalive).
    mock_provider_first_chunk(Duration::from_millis(50));

    let elapsed = start.elapsed();
    assert!(
        elapsed <= WARM_BUDGET,
        "V6.TTFR.3 warm-cache TTFR {:?} exceeded tight budget {:?}",
        elapsed,
        WARM_BUDGET
    );
    println!("V6.TTFR.3 warm-cache TTFR = {:?} (tight budget {:?})", elapsed, WARM_BUDGET);
}

// ─── 4/4 — Slow provider edge: still under global budget ───────────────────

/// **Budget**: TTFR ≤ 30 s (SPEC-28 §1 G1 ceiling — the p95 line).
///
/// Edge case — provider is on the slow side of the latency distribution
/// (simulated 2 s first-chunk delay, e.g. cold cloud region or model warming
/// up). Even at this pessimistic provider latency the end-to-end TTFR MUST
/// still fit the 30 s p95 budget. Failure here means the non-provider stages
/// (capture / aggregate / projection) are themselves slow enough to consume
/// most of the budget — a serious architecture regression.
#[test]
fn v6_ttfr_slow_provider_still_under_budget() {
    let start = Instant::now();

    let req = fixture_food_capture("late lunch", 1_716_580_000_000);
    let pair = project_to_event_pair(&req, "evt-slow-001", "2026-05-26T12:30:00Z");
    let brief = aggregate(std::slice::from_ref(&pair));
    assert!(!brief.is_empty(), "brief must not be empty for non-empty input");

    // Simulated slow provider first chunk — 2 s. Still leaves 28 s headroom
    // versus the 30 s SPEC budget for the pure-CPU stages above.
    mock_provider_first_chunk(Duration::from_secs(2));

    let elapsed = start.elapsed();
    assert!(
        elapsed <= TTFR_BUDGET,
        "V6.TTFR.4 slow-provider TTFR {:?} exceeded budget {:?}",
        elapsed,
        TTFR_BUDGET
    );
    println!("V6.TTFR.4 slow-provider TTFR = {:?} (budget {:?})", elapsed, TTFR_BUDGET);
}

// ─── 5/5 — Integration / env-dependent — real providers_wire path ──────────

/// **Budget**: TTFR ≤ 30 s (SPEC-28 §1 G1) against the real
/// `providers_wire::complete` HTTP adapter.
///
/// Pending Stage 4 — `providers_wire::complete` still bottoms out in
/// per-provider `complete_*_pseudo` helpers (only `Groq` / `Anthropic` /
/// `Gemini` shipped real impls so far per the file's Stage 3 banner). When the
/// full chain is live this test exercises the real reqwest round-trip; until
/// then it is `#[ignore]`d per the v4/v5/v7 pattern + the orchestrator hard
/// rule on env-dependent integration tests.
#[test]
#[ignore = "integration / env-dependent — run via --ignored (requires real provider key)"]
fn v6_ttfr_real_provider_round_trip_under_budget() {
    // Placeholder for when the real path is wired. The test body intentionally
    // mirrors the happy-path shape so flipping `#[ignore]` off only needs the
    // provider call substituted in for `mock_provider_first_chunk`.
    let start = Instant::now();
    let req = fixture_food_capture("smoke test", 1_716_563_400_000);
    let pair = project_to_event_pair(&req, "evt-real-001", "2026-05-26T08:00:00Z");
    let _ = aggregate(std::slice::from_ref(&pair));

    // TODO Stage 4: swap this for `providers_wire::complete(...)` and assert
    // the first response chunk arrived within `TTFR_BUDGET`.
    mock_provider_first_chunk(Duration::from_millis(100));

    let elapsed = start.elapsed();
    assert!(
        elapsed <= TTFR_BUDGET,
        "V6.TTFR.5 real-provider TTFR {:?} exceeded budget {:?}",
        elapsed,
        TTFR_BUDGET
    );
}
