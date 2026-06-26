//! bench_skill_tool_catalog_lookup
//!
//! Goal: prove the H5 catalog claim of "O(1) tool lookup". The current
//! implementation does a linear scan over `catalog()`'s Vec of 10 tools.
//! Strictly, 10-element linear scan is O(1) only by virtue of `n` being
//! a fixed constant — we want to demonstrate it lands in the
//! sub-microsecond bucket so callers can treat lookup as free.

#[path = "common/mod.rs"]
mod common;

#[cfg(not(feature = "experimental-tools"))]
fn main() {
    common::print_disabled_and_exit("experimental-tools");
}

#[cfg(feature = "experimental-tools")]
use criterion::{black_box, criterion_group, criterion_main, Criterion};
#[cfg(feature = "experimental-tools")]
use phantom_mesh::skillbank::tools::{catalog, SkillTool};

/// Linear-scan lookup — the only API the catalog currently exposes.
/// Returns the tool's index in the Vec, or None.
#[cfg(feature = "experimental-tools")]
fn find_tool(cat: &[Box<dyn SkillTool>], name: &str) -> Option<usize> {
    cat.iter().position(|t| t.name() == name)
}

#[cfg(feature = "experimental-tools")]
fn bench_lookup(c: &mut Criterion) {
    let cat = catalog();
    assert_eq!(cat.len(), 10, "T16 bench assumes a 10-tool catalog");

    let mut g = c.benchmark_group("skill_tool_lookup");

    // Best case: first tool in the Vec.
    g.bench_function("first", |b| {
        b.iter(|| {
            let r = find_tool(black_box(&cat), black_box("skill_calculator"));
            black_box(r)
        });
    });

    // Worst case: last tool in the Vec.
    g.bench_function("last", |b| {
        b.iter(|| {
            let r = find_tool(black_box(&cat), black_box("skill_uuid_gen"));
            black_box(r)
        });
    });

    // Miss case: tool that doesn't exist (full scan, no match).
    g.bench_function("miss", |b| {
        b.iter(|| {
            let r = find_tool(black_box(&cat), black_box("does_not_exist"));
            black_box(r)
        });
    });

    g.finish();
}

#[cfg(feature = "experimental-tools")]
criterion_group! {
    name = tool_lookup;
    config = common::standard_criterion();
    targets = bench_lookup
}

#[cfg(feature = "experimental-tools")]
criterion_main!(tool_lookup);
