//! bench_hermes_tool_dispatch
//!
//! Goal: measure the *end-to-end* dispatch cost of "find tool by name,
//! then call it with a small input". Chosen tool is `hermes_calculator`
//! because:
//!   * it's pure-CPU (no FS, no network)
//!   * its input is a short ASCII string ("2 + 3 * 4")
//!   * it returns a JSON value, exercising the same return path the
//!     real LLM-driven dispatcher uses
//!
//! Contrast with `bench_hermes_tool_catalog_lookup`: that one only times
//! the name match; this one times match + call.

#[path = "common/mod.rs"]
mod common;

#[cfg(not(feature = "experimental-hermes-tools"))]
fn main() {
    common::print_disabled_and_exit("experimental-hermes-tools");
}

#[cfg(feature = "experimental-hermes-tools")]
use criterion::{black_box, criterion_group, criterion_main, Criterion};
#[cfg(feature = "experimental-hermes-tools")]
use phantom_mesh::hermes::tools::{catalog, HermesTool};
#[cfg(feature = "experimental-hermes-tools")]
use serde_json::{json, Value};
#[cfg(feature = "experimental-hermes-tools")]
use tokio::runtime::Runtime;

#[cfg(feature = "experimental-hermes-tools")]
fn find_tool<'a>(
    cat: &'a [Box<dyn HermesTool>],
    name: &str,
) -> Option<&'a (dyn HermesTool + 'static)> {
    cat.iter().find(|t| t.name() == name).map(|b| b.as_ref())
}

#[cfg(feature = "experimental-hermes-tools")]
fn bench_dispatch(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let cat = catalog();
    let args: Value = json!({"expression": "2 + 3 * 4"});

    // Sanity check: calling the calculator must return 14 to confirm we
    // are actually dispatching to a working tool and not no-op'ing.
    let warm = rt.block_on(async {
        let t = find_tool(&cat, "hermes_calculator").expect("calculator present");
        t.call(&args).await.expect("warm call")
    });
    assert_eq!(
        warm["result"], 14.0,
        "calculator must return 14 for '2 + 3 * 4'"
    );

    let mut g = c.benchmark_group("hermes_tool_dispatch");

    g.bench_function("calculator_simple_expr", |b| {
        b.to_async(&rt).iter(|| async {
            let t = find_tool(black_box(&cat), black_box("hermes_calculator"))
                .expect("calculator present");
            t.call(black_box(&args)).await.expect("call")
        });
    });

    g.finish();
}

#[cfg(feature = "experimental-hermes-tools")]
criterion_group! {
    name = tool_dispatch;
    config = common::standard_criterion();
    targets = bench_dispatch
}

#[cfg(feature = "experimental-hermes-tools")]
criterion_main!(tool_dispatch);
