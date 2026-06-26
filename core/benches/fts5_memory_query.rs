//! bench_fts5_memory_query_p99
//!
//! Goal: measure single-term BM25 search latency on a memory store that has
//! been pre-populated with 10,000 rows. Criterion's HTML report includes a
//! p99 estimate; the docs/perf report captures the median + p99 numbers.
//!
//! The claim under test is "BM25 search is fast" (H3, PR #30). What
//! "fast" means is exactly what this bench produces — we are establishing
//! the baseline, not validating against a pre-existing target.

#[path = "common/mod.rs"]
mod common;

#[cfg(not(feature = "experimental-memory"))]
fn main() {
    common::print_disabled_and_exit("experimental-memory");
}

#[cfg(feature = "experimental-memory")]
use criterion::{criterion_group, criterion_main, Criterion};
#[cfg(feature = "experimental-memory")]
use phantom_mesh::skillbank::{SkillMemory, NewMemory};
#[cfg(feature = "experimental-memory")]
use std::sync::OnceLock;
#[cfg(feature = "experimental-memory")]
use tempfile::TempDir;
#[cfg(feature = "experimental-memory")]
use tokio::runtime::Runtime;

#[cfg(feature = "experimental-memory")]
const SEED_ROWS: usize = 10_000;

/// Reusable lorem-ipsum-ish corpus. 100 distinct word stems shuffled
/// per-row so BM25 has something to score. One of the words ("phantom")
/// only appears in 10% of rows so we have a realistic search target.
#[cfg(feature = "experimental-memory")]
fn corpus_row(i: usize) -> String {
    const WORDS: &[&str] = &[
        "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
        "lambda", "mu", "nu", "xi", "omicron", "pi", "rho", "sigma", "tau", "upsilon", "phi",
        "chi", "psi", "omega", "quick", "brown", "fox", "lazy", "dog", "rust", "memory", "safe",
        "fts5", "search", "index", "tokenize", "unicode", "bm25", "rank",
    ];
    let phantom_marker = if i % 10 == 0 { " phantom" } else { "" };
    let mut s = String::with_capacity(160);
    for w in WORDS.iter().take(20 + (i % 8)) {
        s.push_str(w);
        s.push(' ');
    }
    s.push_str(phantom_marker);
    s
}

/// Build (or fetch the cached) 10K-row FTS5 store. We keep one DB across
/// all iterations so we benchmark the query side, not the setup cost.
/// The TempDir is leaked into a static OnceLock so it outlives the
/// SkillMemory handle.
#[cfg(feature = "experimental-memory")]
fn seeded_memory(rt: &Runtime) -> &'static SkillMemory {
    static CELL: OnceLock<SkillMemory> = OnceLock::new();
    static TD: OnceLock<TempDir> = OnceLock::new();
    CELL.get_or_init(|| {
        let td = TempDir::new().expect("tempdir");
        let path = td.path().join("skill.db");
        let mem = SkillMemory::open_at(path).expect("open_at");
        rt.block_on(async {
            for i in 0..SEED_ROWS {
                let text = corpus_row(i);
                mem.insert(NewMemory {
                    kind: "fact",
                    source: "seed",
                    text: &text,
                    tags: "bench seed",
                })
                .await
                .expect("seed insert");
            }
        });
        // Park the tempdir in a static so it doesn't drop before the bench ends.
        let _ = TD.set(td);
        mem
    })
}

#[cfg(feature = "experimental-memory")]
fn bench_query(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let mem = seeded_memory(&rt);

    // Sanity check: the query *must* return matches, otherwise we're
    // benchmarking the empty-result path which is not what we claim.
    let warm = rt.block_on(async { mem.search("phantom", 10).await.expect("warm search") });
    assert!(
        !warm.is_empty(),
        "10K-row seed should produce matches for 'phantom'; got {} rows",
        warm.len()
    );

    let mut g = c.benchmark_group("fts5_memory_query");
    // Single bench: a one-term BM25 query that hits ~10% of rows.
    g.bench_function("single_term_bm25_10k_rows", |b| {
        b.to_async(&rt)
            .iter(|| async { mem.search("phantom", 10).await.expect("search") });
    });
    g.finish();
}

#[cfg(feature = "experimental-memory")]
criterion_group! {
    name = fts5_query;
    config = common::standard_criterion();
    targets = bench_query
}

#[cfg(feature = "experimental-memory")]
criterion_main!(fts5_query);
