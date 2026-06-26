//! Mockable system clock (P0-9).
//!
//! A single, tiny time-source abstraction so streak / daily-transition /
//! date-bucketing logic — and latency telemetry — are deterministic under test.
//!
//! Design: `now_ms()` (unix epoch milliseconds) is the ONLY required method, the
//! single source of truth. Every other reading (`now_unix_secs`, `now_utc`) is a
//! default method derived from `now_ms()`, so an impl can never drift between the
//! seconds view and the millis view. `SystemClock` reproduces today's production
//! behavior exactly: it reads `SystemTime::now()` the same way the legacy
//! `focus_session::now_ms` did, and `now_utc()` returns `chrono::Utc::now()`-
//! equivalent instants. `MockClock` lets a test pin or advance "now" with no
//! sleeps and no wall-clock dependence.

use std::time::{SystemTime, UNIX_EPOCH};

/// A source of "now". Production uses [`SystemClock`]; tests inject [`MockClock`].
///
/// `now_ms` is the single required reading (unix epoch milliseconds). The
/// seconds and `chrono` views are derived defaults so they can never disagree.
pub trait Clock: Send + Sync {
    /// Current unix-epoch time in milliseconds.
    fn now_ms(&self) -> u64;

    /// Current unix-epoch time in whole seconds (floor of `now_ms / 1000`).
    /// Matches the `now_secs` shape used by `nudge_ledger` / partner windows.
    fn now_unix_secs(&self) -> u64 {
        self.now_ms() / 1000
    }

    /// Current instant as a `chrono` UTC datetime, derived from `now_ms` so it
    /// stays consistent with the millisecond reading. Falls back to the unix
    /// epoch on the (unreachable for `SystemClock`) out-of-range case.
    fn now_utc(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::<chrono::Utc>::from_timestamp_millis(self.now_ms() as i64)
            .unwrap_or_else(|| chrono::DateTime::<chrono::Utc>::from_timestamp_millis(0).unwrap())
    }
}

/// Production clock: reads the real wall clock. Reproduces the legacy
/// `focus_session::now_ms` behavior byte-for-byte (`SystemTime::now() -
/// UNIX_EPOCH`, saturating to 0 on a pre-epoch clock).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

use std::sync::atomic::{AtomicU64, Ordering};

/// Test clock: returns a settable, advanceable "now" with no wall-clock read.
///
/// Uses an `AtomicU64` so `advance_ms` / `set_ms` work through a shared `&self`
/// (the injection sites take `&dyn Clock`) — no `&mut` or lock needed. Cheap to
/// clone-by-reference; tests typically construct one and pass `&clock`.
#[derive(Debug)]
pub struct MockClock {
    now_ms: AtomicU64,
}

impl MockClock {
    /// A clock pinned at `now_ms` (unix epoch milliseconds).
    pub fn new(now_ms: u64) -> Self {
        Self { now_ms: AtomicU64::new(now_ms) }
    }

    /// A clock pinned at 00:00:00 UTC of the given calendar date. Panics on an
    /// invalid date (test-only helper, so a bad literal fails loudly).
    pub fn at_utc_date(year: i32, month: u32, day: u32) -> Self {
        let date = chrono::NaiveDate::from_ymd_opt(year, month, day)
            .expect("at_utc_date: invalid Y/M/D literal");
        let dt = date
            .and_hms_opt(0, 0, 0)
            .expect("midnight is always valid")
            .and_utc();
        Self::new(dt.timestamp_millis() as u64)
    }

    /// Jump to an absolute instant (unix epoch milliseconds).
    pub fn set_ms(&self, now_ms: u64) {
        self.now_ms.store(now_ms, Ordering::SeqCst);
    }

    /// Move "now" forward by `delta_ms` milliseconds.
    pub fn advance_ms(&self, delta_ms: u64) {
        self.now_ms.fetch_add(delta_ms, Ordering::SeqCst);
    }
}

impl Clock for MockClock {
    fn now_ms(&self) -> u64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

/// A latency-measurement span anchored to a [`Clock`]. Capture the start instant
/// from a clock, later read `elapsed_ms` against the SAME (or any) clock. Built
/// on the injectable `Clock` so latency telemetry is deterministic under test —
/// production passes `&SystemClock`, tests pass a `&MockClock` they can advance.
#[derive(Debug, Clone, Copy)]
pub struct LatencyTimer {
    start_ms: u64,
}

impl LatencyTimer {
    /// Capture the start instant from `clock`.
    pub fn start(clock: &dyn Clock) -> Self {
        Self { start_ms: clock.now_ms() }
    }

    /// Milliseconds elapsed between `start` and `clock`'s current reading.
    /// Saturates at 0 if the clock moved backwards (never underflows / panics).
    pub fn elapsed_ms(&self, clock: &dyn Clock) -> u64 {
        clock.now_ms().saturating_sub(self.start_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_now_ms_is_plausible_and_monotonic_view() {
        let c = SystemClock;
        let a = c.now_ms();
        // Sometime after 2020-01-01 (1_577_836_800_000 ms) — proves it's real epoch ms.
        assert!(a > 1_577_836_800_000, "now_ms must be a real epoch-ms reading, got {a}");
        // Derived seconds view is exactly floor(ms/1000).
        assert_eq!(c.now_unix_secs(), a / 1000);
        // now_utc agrees with now_ms to the second (both derived from the same call
        // is not guaranteed across two reads, so compare coarsely).
        let utc = c.now_utc();
        let secs = utc.timestamp() as u64;
        assert!(secs.abs_diff(a / 1000) <= 1, "now_utc must track now_ms");
    }

    #[test]
    fn mock_clock_is_settable_and_advanceable() {
        let c = MockClock::new(1_000_000); // pinned at 1,000,000 ms
        assert_eq!(c.now_ms(), 1_000_000);
        assert_eq!(c.now_unix_secs(), 1_000); // 1_000_000 / 1000
        // now_utc derives from the pinned ms.
        assert_eq!(c.now_utc().timestamp_millis(), 1_000_000);

        // advance_ms moves "now" forward without any sleep.
        c.advance_ms(500);
        assert_eq!(c.now_ms(), 1_000_500);

        // set_ms jumps to an absolute instant.
        c.set_ms(2_000_000);
        assert_eq!(c.now_ms(), 2_000_000);
        assert_eq!(c.now_unix_secs(), 2_000);

        // Constructor-by-day helper: midnight UTC of a fixed date.
        let d = MockClock::at_utc_date(2026, 6, 17);
        assert_eq!(
            d.now_utc().date_naive(),
            chrono::NaiveDate::from_ymd_opt(2026, 6, 17).unwrap()
        );
    }

    #[test]
    fn mock_clock_is_shareable_as_dyn_clock() {
        // The injection sites take `&dyn Clock`; confirm MockClock coerces and
        // that interior mutability works through a shared reference.
        let c = MockClock::new(10);
        let dyn_ref: &dyn Clock = &c;
        assert_eq!(dyn_ref.now_ms(), 10);
        c.advance_ms(5);
        assert_eq!(dyn_ref.now_ms(), 15, "advance visible through &dyn Clock");
    }

    #[test]
    fn latency_timer_measures_elapsed_via_injected_clock() {
        // Deterministic latency telemetry: start the timer at a pinned instant,
        // advance the mock clock, and read an EXACT elapsed_ms — no sleeps, no
        // wall-clock flakiness.
        let c = MockClock::new(5_000);
        let t = LatencyTimer::start(&c);
        assert_eq!(t.elapsed_ms(&c), 0, "no time advanced yet");
        c.advance_ms(137);
        assert_eq!(t.elapsed_ms(&c), 137, "elapsed tracks the injected clock exactly");
        c.advance_ms(63);
        assert_eq!(t.elapsed_ms(&c), 200);
    }

    #[test]
    fn latency_timer_saturates_on_backwards_clock() {
        // A clock that jumps backwards (set to before start) must not underflow;
        // elapsed saturates at 0.
        let c = MockClock::new(5_000);
        let t = LatencyTimer::start(&c);
        c.set_ms(4_000); // backwards
        assert_eq!(t.elapsed_ms(&c), 0, "backwards clock saturates to 0, never panics");
    }
}
