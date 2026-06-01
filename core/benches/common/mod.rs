//! Shared bench helpers — kept small on purpose. The individual benches
//! own their domain-specific setup.

use std::time::Duration;

/// Standard Criterion configuration used by every bench in this directory.
/// 1-second warm-up + 3-second sample window keeps a full bench-suite run
/// under 90 seconds on a laptop while still producing stable medians.
pub fn standard_criterion() -> criterion::Criterion {
    criterion::Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .sample_size(50)
}

/// Print a "feature disabled" notice and exit 0 — used by each bench's
/// `main()` when its `required-features` flag is OFF. Keeps `cargo bench`
/// (no flags) compiling without forcing a real benchmark run.
#[allow(dead_code)]
pub fn print_disabled_and_exit(feature: &str) -> ! {
    eprintln!(
        "[T16 bench skipped] feature `{feature}` is not enabled — \
         run `cargo bench --features {feature}` to execute this bench."
    );
    std::process::exit(0);
}
