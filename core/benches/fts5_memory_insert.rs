//! bench_fts5_memory_insert_throughput
//!
//! Goal: establish an inserts/sec baseline for the H3 (PR #30) FTS5 memory
//! backend. Each iteration inserts ONE row into a fresh on-disk SQLite DB.
//! Criterion reports the per-iter timing; the docs/perf report derives
//! inserts/sec from the median. The claim under test is "1000 inserts/sec";
//! a tempfile DB on a modern SSD should clear that comfortably.
//!
//! Why on-disk and not :memory:? — On-disk is the realistic production
//! path. In-memory would inflate the number and hide WAL / fsync cost.

#[path = "common/mod.rs"]
mod common;

// ── Feature OFF: empty stub so `cargo check --benches` (no features) is clean.
// The `required-features` declaration in Cargo.toml means
// `cargo bench --bench fts5_memory_insert` (no features) already skips the run
// — this main is purely a compile-gate.
#[cfg(not(feature = "experimental-memory"))]
fn main() {
    common::print_disabled_and_exit("experimental-memory");
}

// ── Feature ON: the real Criterion bench.
#[cfg(feature = "experimental-memory")]
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
#[cfg(feature = "experimental-memory")]
use spectyn_mesh::skillbank::{SkillMemory, NewMemory};
#[cfg(feature = "experimental-memory")]
use tempfile::TempDir;
#[cfg(feature = "experimental-memory")]
use tokio::runtime::Runtime;

/// Build a fresh empty DB at a tempdir path. The tempdir is returned so
/// it lives at least as long as the SkillMemory handle.
#[cfg(feature = "experimental-memory")]
fn fresh_memory(_rt: &Runtime) -> (TempDir, SkillMemory) {
    let td = TempDir::new().expect("tempdir");
    let path = td.path().join("skill.db");
    let mem = SkillMemory::open_at(path).expect("open_at");
    (td, mem)
}

#[cfg(feature = "experimental-memory")]
fn bench_insert(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");

    let mut g = c.benchmark_group("fts5_memory_insert");
    // One row per iteration → Throughput::Elements(1) lets Criterion
    // report elements/sec automatically in the HTML report.
    g.throughput(Throughput::Elements(1));

    g.bench_function("single_row", |b| {
        // Per-iter setup: a fresh DB so we benchmark steady-state insert,
        // not "insert into table that's grown to N rows". Setup time is
        // excluded from the timed body by `iter_batched`.
        b.iter_batched(
            || fresh_memory(&rt),
            |(td, mem)| {
                rt.block_on(async {
                    mem.insert(NewMemory {
                        kind: "fact",
                        source: "bench",
                        text: "the quick brown fox jumps over the lazy dog",
                        tags: "english pangram",
                    })
                    .await
                    .expect("insert")
                });
                drop(td); // explicit, so cleanup runs inside the batch
            },
            criterion::BatchSize::SmallInput,
        );
    });
    g.finish();
}

#[cfg(feature = "experimental-memory")]
criterion_group! {
    name = fts5_insert;
    config = common::standard_criterion();
    targets = bench_insert
}

#[cfg(feature = "experimental-memory")]
criterion_main!(fts5_insert);
