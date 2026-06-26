//! bench_curator_v2_consensus
//!
//! A7 / T95 — pure-function micro-benchmarks for the V2 ensemble curator's
//! aggregation path. Only the three pure functions exported by
//! `core::skillbank::curator_ensemble` are exercised here:
//!
//!   * `median_score(&[u8]) -> f32`
//!   * `population_stddev(&[u8]) -> f32`
//!   * `aggregate(Vec<JudgeVerdict>, attempted: u8, judged_at_ms) -> EnsembleVerdict`
//!
//! ── Why this bench exists ────────────────────────────────────────────────
//! T28 documents the ensemble curator as the consensus engine for evolve
//! sessions. Median + stddev + agreement-class derivation runs on the
//! happy-path of every ensemble round. We want a tight perf budget on the
//! aggregation step so a future refactor (sort algorithm change, allocation
//! pattern, agreement-rule tweak) is caught by a regression bench rather
//! than a production slowdown.
//!
//! ── What this bench is NOT ───────────────────────────────────────────────
//! Strictly no HTTP. Never constructs an `AnthropicJudge`, `OpenAICompatJudge`,
//! `EnsembleCurator`, `reqwest::Client`, `wiremock::MockServer`, or any
//! `JudgeProvider` impl. Synthetic `JudgeVerdict` values built in-process
//! are fed directly into `aggregate`. The 50 fixture transcripts are loaded
//! from `core/tests/fixtures/curator_v2_transcripts.json` at bench startup
//! (one file read, outside the timed loop) and re-used across every iter.
//!
//! ── Agreement-rate metric ────────────────────────────────────────────────
//! The "agreement rate" benched here is the share of fixture transcripts for
//! which ≥2/3 of the judge scores fall within 1 point of each other. This is
//! a stricter pre-filter than the AgreementClass derived by `aggregate`
//! itself (which uses stddev > 2.0σ as the disagreement trigger). Both are
//! useful: the former tells us how often a simple "majority within 1pt"
//! rule would have produced consensus, the latter is what the ensemble
//! actually persists. The bench computes the metric on the full 50-fixture
//! set per iteration so the timing reflects realistic batch-aggregation
//! work (e.g. nightly replay over a journal of sessions).
//!
//! Run:
//! ```
//! cargo bench --bench curator_v2_consensus \
//!   --features experimental-curator \
//!   -- --noise-threshold 0.05
//! ```

#[path = "common/mod.rs"]
mod common;

#[cfg(not(feature = "experimental-curator"))]
fn main() {
    common::print_disabled_and_exit("experimental-curator");
}

#[cfg(feature = "experimental-curator")]
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
#[cfg(feature = "experimental-curator")]
use phantom_mesh::evolve_checkpoint::JudgeVerdict;
#[cfg(feature = "experimental-curator")]
use phantom_mesh::skillbank::{aggregate, median_score, population_stddev};
#[cfg(feature = "experimental-curator")]
use serde::Deserialize;

// ─── Fixture loader (off the hot path) ───────────────────────────────────

#[cfg(feature = "experimental-curator")]
#[derive(Debug, Clone, Deserialize)]
struct FixtureTranscript {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    class_hint: String,
    judges: Vec<JudgeVerdict>,
}

#[cfg(feature = "experimental-curator")]
fn load_fixtures() -> Vec<FixtureTranscript> {
    // Path is relative to the crate root (CARGO_MANIFEST_DIR is set by cargo
    // when invoking benches, so this resolves the same way the test harness
    // resolves `tests/fixtures/...`).
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("curator_v2_transcripts.json");
    let bytes =
        std::fs::read(&p).unwrap_or_else(|e| panic!("missing fixture {}: {}", p.display(), e));
    let v: Vec<FixtureTranscript> = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("malformed fixture {}: {}", p.display(), e));
    assert!(
        v.len() >= 50,
        "fixture must hold at least 50 transcripts, found {}",
        v.len()
    );
    v
}

// ─── Synthetic helpers (also off the hot path) ───────────────────────────

#[cfg(feature = "experimental-curator")]
fn mk_verdict(score: u8, idx: usize) -> JudgeVerdict {
    JudgeVerdict {
        score,
        rubric_version: "h1-v1".to_string(),
        model: format!("synthetic-{}", idx),
        rationale: String::from("synthetic rationale"),
        judged_at_ms: 1_700_000_000_000 + idx as i64,
    }
}

/// Deterministic synthetic verdict list of length `n` with a tunable spread.
/// `spread=0` ⇒ all judges return `base` (unanimous).
/// `spread=2` ⇒ scores alternate base-1 / base / base+1 (consensus-bucket).
/// `spread=8` ⇒ scores alternate low / high (disagreement-bucket).
#[cfg(feature = "experimental-curator")]
fn synthetic_verdicts(n: usize, base: u8, spread: i8) -> Vec<JudgeVerdict> {
    (0..n)
        .map(|i| {
            // Cyclic delta over 3 phases so total population is balanced.
            let phase = (i % 3) as i8 - 1; // -1, 0, +1
            let raw = base as i16 + (phase as i16) * (spread as i16);
            let clamped = raw.clamp(0, 10) as u8;
            mk_verdict(clamped, i)
        })
        .collect()
}

#[cfg(feature = "experimental-curator")]
fn synthetic_scores(n: usize, base: u8, spread: i8) -> Vec<u8> {
    synthetic_verdicts(n, base, spread)
        .into_iter()
        .map(|v| v.score)
        .collect()
}

// ─── Agreement-rate (pure, computed on score slices) ─────────────────────

/// Returns true iff ≥2/3 of `scores` lie within a 1-point window. Implemented
/// via the standard sliding-window-on-sorted-scores trick: any window of size
/// `threshold` whose max-min ≤ 1 indicates a clustered majority. Sorted scan
/// is O(n log n) for the sort + O(n) for the sweep — dominated by the sort
/// at n ≤ 10 (so essentially constant for our judge counts).
#[cfg(feature = "experimental-curator")]
fn within_one_point_majority(scores: &[u8]) -> bool {
    if scores.is_empty() {
        return false;
    }
    let threshold = (scores.len() * 2 + 2) / 3; // ceil(2n/3)
    let mut sorted: Vec<u8> = scores.to_vec();
    sorted.sort_unstable();
    if threshold > sorted.len() {
        return false;
    }
    sorted
        .windows(threshold)
        .any(|w| w[w.len() - 1] - w[0] <= 1)
}

/// Compute agreement rate (0.0..1.0) across the full fixture set. This is
/// the per-iteration workload for the agreement-rate bench: 50 sliding-window
/// passes, one per transcript.
#[cfg(feature = "experimental-curator")]
fn agreement_rate(fixtures: &[FixtureTranscript]) -> f32 {
    let n = fixtures.len();
    if n == 0 {
        return 0.0;
    }
    let scored: usize = fixtures
        .iter()
        .filter(|t| {
            let scores: Vec<u8> = t.judges.iter().map(|v| v.score).collect();
            within_one_point_majority(&scores)
        })
        .count();
    scored as f32 / n as f32
}

// ─── Setup-time sanity checks ────────────────────────────────────────────

/// Catch fixture drift loudly at bench startup (NOT in the timed loop). If
/// the fixture file is replaced and these no longer hold, the user sees a
/// crisp panic rather than silently regressed numbers.
#[cfg(feature = "experimental-curator")]
fn assert_fixture_invariants(fixtures: &[FixtureTranscript]) {
    assert_eq!(
        fixtures.len(),
        50,
        "fixture set must hold exactly 50 transcripts"
    );
    for t in fixtures {
        assert!(
            (3..=10).contains(&t.judges.len()),
            "transcript {} has {} judges (expected 3..=10)",
            t.name,
            t.judges.len()
        );
        for v in &t.judges {
            assert!(v.score <= 10, "score {} out of [0..=10]", v.score);
            assert_eq!(v.rubric_version, "h1-v1");
        }
    }
    // Run aggregate over the first fixture to confirm the trait + types line
    // up before we enter Criterion (a typo here would fail every iter).
    let sample = &fixtures[0];
    let v = aggregate(sample.judges.clone(), sample.judges.len() as u8, 1);
    assert_eq!(v.judges_attempted as usize, sample.judges.len());
}

// ─── Bench: median_score isolated, 3 / 5 / 10 scores ─────────────────────

#[cfg(feature = "experimental-curator")]
fn bench_median_score(c: &mut Criterion) {
    let mut g = c.benchmark_group("median_score");
    for &n in &[3usize, 5, 10] {
        let scores = synthetic_scores(n, 5, 2);
        g.bench_with_input(BenchmarkId::from_parameter(n), &scores, |b, s| {
            b.iter(|| {
                let m = median_score(black_box(s));
                black_box(m)
            });
        });
    }
    g.finish();
}

// ─── Bench: population_stddev isolated, 3 / 5 / 10 scores ────────────────

#[cfg(feature = "experimental-curator")]
fn bench_population_stddev(c: &mut Criterion) {
    let mut g = c.benchmark_group("population_stddev");
    for &n in &[3usize, 5, 10] {
        let scores = synthetic_scores(n, 5, 2);
        g.bench_with_input(BenchmarkId::from_parameter(n), &scores, |b, s| {
            b.iter(|| {
                let v = population_stddev(black_box(s));
                black_box(v)
            });
        });
    }
    g.finish();
}

// ─── Bench: aggregate end-to-end, 3 / 5 / 10 judges ──────────────────────

#[cfg(feature = "experimental-curator")]
fn bench_aggregate(c: &mut Criterion) {
    let mut g = c.benchmark_group("aggregate");
    for &n in &[3usize, 5, 10] {
        // Mid-range scores with a small spread — drives the Consensus branch
        // (the most common production case). The bench measures only `aggregate`
        // itself; the verdict Vec is cloned per iter so allocation cost is
        // included on every call (same as production hits it).
        let verdicts = synthetic_verdicts(n, 5, 2);
        g.bench_with_input(BenchmarkId::from_parameter(n), &verdicts, |b, vs| {
            b.iter(|| {
                let out = aggregate(black_box(vs.clone()), black_box(n as u8), black_box(1));
                black_box(out)
            });
        });
    }
    g.finish();
}

// ─── Bench: agreement-rate over the full 50-fixture set ──────────────────

#[cfg(feature = "experimental-curator")]
fn bench_agreement_rate(c: &mut Criterion) {
    let fixtures = load_fixtures();
    assert_fixture_invariants(&fixtures);

    // Print the agreement rate at setup so the human running this bench can
    // capture it for the report (matches the task's "agreement rate" line).
    let rate = agreement_rate(&fixtures);
    eprintln!(
        "[bench_curator_v2_consensus] agreement-rate over 50 fixtures = {:.4} ({} / {})",
        rate,
        fixtures
            .iter()
            .filter(|t| within_one_point_majority(
                &t.judges.iter().map(|v| v.score).collect::<Vec<_>>()
            ))
            .count(),
        fixtures.len()
    );

    let mut g = c.benchmark_group("agreement_rate");
    g.bench_function("over_50_fixtures", |b| {
        b.iter(|| {
            let r = agreement_rate(black_box(&fixtures));
            black_box(r)
        });
    });
    g.finish();
}

// ─── Bench: aggregate driven by every fixture transcript (batch) ─────────

#[cfg(feature = "experimental-curator")]
fn bench_aggregate_over_fixtures(c: &mut Criterion) {
    let fixtures = load_fixtures();
    assert_fixture_invariants(&fixtures);

    let mut g = c.benchmark_group("aggregate_over_fixtures");
    g.bench_function("all_50", |b| {
        b.iter(|| {
            for t in &fixtures {
                let v = aggregate(
                    black_box(t.judges.clone()),
                    black_box(t.judges.len() as u8),
                    black_box(1),
                );
                black_box(v);
            }
        });
    });
    g.finish();
}

#[cfg(feature = "experimental-curator")]
criterion_group! {
    name = consensus;
    config = common::standard_criterion();
    targets =
        bench_median_score,
        bench_population_stddev,
        bench_aggregate,
        bench_agreement_rate,
        bench_aggregate_over_fixtures
}

#[cfg(feature = "experimental-curator")]
criterion_main!(consensus);
