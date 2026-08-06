//! In-process daily coach scheduler — the tokio half of SPEC-23 §G1.
//!
//! ## Why this exists (the launchd dead-trigger)
//!
//! [`coach_scheduler`](super::coach_scheduler) generates + installs an OS
//! scheduler unit (launchd / systemd / schtasks) that fires `spectyn coach
//! review` daily. On macOS that launchd path is fragile: a launchd-spawned
//! `spectyn` (parent pid 1) is run through the code-signing monitor and a
//! freshly built / linker-signed binary is SIGKILLed under a launch-constraint
//! / provenance check (`dyld` `open_with_subsystem` hang → no output) before any
//! review is written. The job *looks* installed but every fire dies silently.
//!
//! The insight (user, 2026-06-05): `spectyn serve` is ALREADY a long-lived
//! daemon that runs fine. Embedding the daily trigger as a tokio timer INSIDE
//! that process means **no new process is spawned** → no dyld provenance /
//! launch-constraint check → no SIGKILL. It is also the natural home for the
//! partner's "always-on, proactive" facet: the serve loop is the single thing
//! that fires the daily reflection.
//!
//! ## What it does (and does NOT do)
//!
//!   - [`next_fire_duration`] is a PURE function over an injected `now`: given
//!     the current local time + a [`CoachSchedule`] (default 21:00) it returns
//!     the [`Duration`] to sleep until the next occurrence. No wall clock is
//!     read inside it, so it is fully unit-testable across every clock position
//!     (before / after the target, the exact minute, the midnight roll-over).
//!   - [`spawn_daily_coach_loop`] sleeps that long, runs the coach review
//!     ONCE per calendar day (date-scoped idempotency key, see below), then
//!     recomputes and sleeps again. Forever, inside the serve runtime.
//!
//! It deliberately does NOT register any OS scheduler, write any unit file, or
//! spawn any subprocess — that is [`coach_scheduler`](super::coach_scheduler)'s
//! job and remains an optional, operator-invoked add-on for users who want a
//! portable launchd/systemd/schtasks registration. The two are complementary:
//! serve's in-process loop is the reliable default; the OS unit is the portable
//! opt-in. (If BOTH run, the date-scoped idempotency key below means the second
//! one to reach 21:00 sees the day already done and skips — no double review.)
//!
//! ## Deduplication: one review per calendar day
//!
//! We reuse the existing [`crate::idempotency`] ledger with a DATE-scoped key
//! `daily-coach:YYYY-MM-DD` and a 24h TTL. The first fire of the day records the
//! key and runs; any later attempt that day (serve restarted at 22:30, a brief
//! clock-skew double-wakeup, OR the optional OS scheduler firing too) sees the
//! key within TTL → [`Decision::Duplicate`] → skip. The date scope is the
//! intent ("once per calendar day, period"); a timestamp scope would let a
//! restart 2h later re-run. The ledger's process [`Mutex`] + atomic append make
//! the check→record race-safe across threads and a serve bounce.
//!
//! ## Opt-out
//!
//! `SPECTYN_COACH_DISABLE=1` skips spawning the loop entirely (used by tests and
//! single-run / manual-trigger setups). The caller (`spectyn serve`) checks this
//! and simply does not spawn — see `core/src/bin/spectyn.rs`.

use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Days, TimeZone};
use tokio::task::JoinHandle;

use super::coach_scheduler::CoachSchedule;

/// idempotency `kind` tag recorded alongside the date key (diagnostic metadata).
const COACH_KIND: &str = "daily-coach";

/// Dedup TTL: 24h, so the date key stays "seen" from its 21:00 fire through ~21:00
/// the next day. A serve restart anywhere in that window finds the key and skips.
const COACH_TTL_SECS: u64 = 24 * 60 * 60;

/// Env var that opts a serve process out of the embedded daily loop entirely.
pub const DISABLE_ENV: &str = "SPECTYN_COACH_DISABLE";

/// Date-scoped idempotency key for a given local calendar date (`YYYY-MM-DD`).
/// Public so a test (and the caller) can assert the exact "once per day" scope.
pub fn dedup_key(date: &str) -> String {
    format!("{COACH_KIND}:{date}")
}

/// `true` when `SPECTYN_COACH_DISABLE` is set to a truthy value (`1`/`true`/`yes`,
/// case-insensitive). Anything else (unset, empty, `0`) means the loop runs.
pub fn is_disabled() -> bool {
    match std::env::var(DISABLE_ENV) {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes"
        }
        Err(_) => false,
    }
}

/// PURE: how long to sleep from `now` until the next local occurrence of
/// `schedule` (default 21:00). Clock is INJECTED (`now`), never read here, so
/// every edge is unit-testable.
///
/// Rules:
///   - If today's target time is still in the future (`now` < target), sleep
///     until today's target.
///   - If `now` is exactly at or past today's target (e.g. 21:00:00 or 22:30),
///     sleep until TOMORROW's target — never returns a zero/negative duration,
///     so the loop can't busy-spin firing the same instant repeatedly.
///   - Crossing midnight / month / year is handled by `chrono`'s date math
///     ([`Days::new(1)`]).
///
/// Generic over the timezone so a test can construct a deterministic
/// `chrono::Local`/`Utc`/`FixedOffset` `now`; production passes `Local::now()`.
/// Defensive: if a DST gap/fold makes a wall time invalid/ambiguous the helper
/// nudges by a minute rather than panicking, so the loop always gets a finite
/// sleep.
pub fn next_fire_duration<Tz: TimeZone>(now: DateTime<Tz>, schedule: CoachSchedule) -> Duration {
    let target_today = at_time_on(&now, now.date_naive(), schedule);
    let fire_at = if target_today > now {
        target_today
    } else {
        // At or past today's target → next is tomorrow's target.
        let tomorrow = now
            .date_naive()
            .checked_add_days(Days::new(1))
            .unwrap_or_else(|| now.date_naive()); // saturate at chrono's max date
        at_time_on(&now, tomorrow, schedule)
    };
    // `fire_at` is strictly in the future by construction; clamp the (otherwise
    // impossible) negative case to zero so `to_std` can't fail.
    let delta = fire_at.signed_duration_since(now);
    delta.to_std().unwrap_or(Duration::ZERO)
}

/// Clock-injectable daily-transition core (P0-6, mirroring P0-9's `*_with_clock`
/// template): resolve "today" (LOCAL `YYYY-MM-DD`) and the sleep-until-next-fire
/// from the SAME injected clock, so the calendar-day decision (which drives
/// [`dedup_key`]) is deterministic under `MockClock`. Production passes
/// `&crate::clock::SystemClock`, whose `now_utc().with_timezone(&Local)` is
/// byte-identical to the previous inline `chrono::Local::now()` reads (both
/// resolve the same wall instant in the same local zone), so this is a pure
/// refactor with no behaviour change.
///
/// Returns `(today, sleep_for)`: the local calendar-date string the daily run is
/// keyed to, and the [`Duration`] until the next `schedule` fire.
pub(crate) fn today_and_sleep_with_clock(
    clock: &dyn crate::clock::Clock,
    schedule: CoachSchedule,
) -> (String, Duration) {
    let now_local = clock.now_utc().with_timezone(&chrono::Local);
    let today = now_local.format("%Y-%m-%d").to_string();
    let sleep_for = next_fire_duration(now_local, schedule);
    (today, sleep_for)
}

/// Build the `DateTime` at `schedule`'s hour:minute on local calendar `date`,
/// in `reference`'s timezone. DST-safe: an invalid (spring-forward gap) or
/// ambiguous (fall-back fold) wall time falls back to the earliest valid instant
/// at/after that wall clock by probing the same date at minute granularity, and
/// ultimately to the reference instant + a day so the loop never stalls.
fn at_time_on<Tz: TimeZone>(
    reference: &DateTime<Tz>,
    date: chrono::NaiveDate,
    schedule: CoachSchedule,
) -> DateTime<Tz> {
    let tz = reference.timezone();
    let naive = match date.and_hms_opt(schedule.hour as u32, schedule.minute as u32, 0) {
        Some(n) => n,
        // hour/minute are range-checked by CoachSchedule::new, but stay total.
        None => return reference.clone() + chrono::Duration::days(1),
    };
    match tz.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) => dt,
        // Fall-back fold: two valid instants; the earlier one is the first time
        // the wall clock reads 21:00, which is what a daily reminder means.
        chrono::LocalResult::Ambiguous(earliest, _latest) => earliest,
        // Spring-forward gap: that exact wall time doesn't exist on this date.
        // Probe forward minute-by-minute (bounded to the rest of the day) for
        // the first VALID local instant — the moment the clock jumps past the
        // gap — so we still fire ~once that day. If nothing in the day is valid
        // (impossible in practice), saturate to reference + 1 day so the loop
        // always gets a finite, future sleep.
        chrono::LocalResult::None => {
            let mut probe = naive;
            for _ in 0..(24 * 60) {
                probe += chrono::Duration::minutes(1);
                if let chrono::LocalResult::Single(dt) = tz.from_local_datetime(&probe) {
                    return dt;
                }
            }
            reference.clone() + chrono::Duration::days(1)
        }
    }
}

/// Spawn the forever-loop daily coach scheduler as a tokio task on the serve
/// runtime. Returns the [`JoinHandle`] (serve drops it; the task lives as long
/// as the process). `home` is the brain root (`~`); `runtime`/`agent` drive the
/// proactive partner reflection half (same deps the `spectyn coach review` CLI
/// builds); `schedule` is the daily trigger time (default 21:00).
///
/// On each iteration:
///   1. compute today's local date + sleep until the next `schedule` instant,
///   2. resolve "today" fresh AFTER waking (so a long sleep across midnight
///      records the correct calendar day),
///   3. consult the idempotency ledger with `daily-coach:<today>` — skip if
///      already fired today (restart / OS-scheduler overlap / clock skew),
///   4. otherwise run `daily_review::run_coach_review(home, today, save=true,
///      partner_deps)` and let the ledger record stand as the dedup marker.
///
/// Errors from the review are logged (never panic the loop) — a bad night must
/// not take down the serve daemon or stop tomorrow's review.
///
/// `notifier` (optional) is the delivery channel for ability ③ *arrival*: when
/// present, each successful review also dispatches a P0 desktop/Telegram
/// notification so the proactive check-in reaches the user instead of only
/// landing as an encrypted file on disk. `None` keeps the save-only behaviour.
pub fn spawn_daily_coach_loop(
    home: PathBuf,
    runtime: crate::agent::AgentRuntime,
    agent: String,
    schedule: CoachSchedule,
    notifier: Option<crate::NotificationDispatcher>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        tracing::info!(
            hour = schedule.hour,
            minute = schedule.minute,
            notify = notifier.is_some(),
            "coach scheduler: embedded daily loop armed (in-process, no OS scheduler)"
        );
        loop {
            // Resolve the sleep-until-next-fire from the injected clock. Production
            // passes &SystemClock → byte-identical to the previous Local::now().
            let (_pre_today, sleep_for) =
                today_and_sleep_with_clock(&crate::clock::SystemClock, schedule);
            tracing::debug!(
                secs = sleep_for.as_secs(),
                "coach scheduler: sleeping until next {:02}:{:02}",
                schedule.hour,
                schedule.minute
            );
            tokio::time::sleep(sleep_for).await;

            // Resolve the date AFTER waking — the sleep may have crossed midnight.
            let (today, _) = today_and_sleep_with_clock(&crate::clock::SystemClock, schedule);
            run_once_for_date(&home, &runtime, &agent, &today, notifier.as_ref()).await;
        }
    })
}

/// Build a short, glanceable notification body from the review markdown.
/// Prefers the "Tomorrow's one action" line (the actionable nudge); falls back
/// to an event-count summary. Kept short so an OS popup stays one-glance.
fn coach_notification_body(markdown: &str, event_count: usize) -> String {
    const HEADER: &str = "## Tomorrow's one action";
    if let Some(idx) = markdown.find(HEADER) {
        if let Some(action) = markdown[idx + HEADER.len()..]
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
        {
            if !action.starts_with("(skipped") {
                return truncate_chars(&format!("Tomorrow: {action}"), 180);
            }
        }
    }
    if event_count == 0 {
        "Your daily review is ready.".to_string()
    } else {
        format!("{event_count} moment(s) reviewed — your daily check-in is ready.")
    }
}

/// Char-boundary-safe truncation with an ellipsis (never splits a UTF-8 char).
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Run the coach review for `date` exactly once (idempotency-guarded). Factored
/// out of the loop so it stays small and the dedup decision is explicit. The
/// ledger check + record is the single source of truth for "did today fire".
async fn run_once_for_date(
    home: &std::path::Path,
    runtime: &crate::agent::AgentRuntime,
    agent: &str,
    date: &str,
    notifier: Option<&crate::NotificationDispatcher>,
) {
    // Thin production wrapper: delegates to the clock-injectable core with
    // `&SystemClock`, whose `now_unix_secs()` is byte-identical to the previous
    // inline `SystemTime::now()` read.
    run_once_for_date_with_clock(home, runtime, agent, date, notifier, &crate::clock::SystemClock)
        .await
}

/// Clock-injectable form of [`run_once_for_date`] (P0-6): the partner window /
/// notification timestamp `now_unix` comes from the injected clock instead of a
/// raw `SystemTime::now()`, so a test can pin it via `MockClock`. The dedup
/// check + review dispatch body is otherwise identical. Production passes
/// `&crate::clock::SystemClock`.
async fn run_once_for_date_with_clock(
    home: &std::path::Path,
    runtime: &crate::agent::AgentRuntime,
    agent: &str,
    date: &str,
    notifier: Option<&crate::NotificationDispatcher>,
    clock: &dyn crate::clock::Clock,
) {
    let key = dedup_key(date);
    // Atomically check-and-record: First → we own today's run; Duplicate → some
    // other path (restart, OS scheduler, skew) already fired, so we must skip.
    let decision = crate::idempotency::check_and_record(&key, COACH_KIND, COACH_TTL_SECS);
    if decision.is_duplicate() {
        tracing::info!(date, "coach scheduler: daily review already fired today — skipping");
        return;
    }

    let now_unix = clock.now_unix_secs();
    let tools = runtime.config().tools.clone();
    let partner = crate::life_node::daily_review::PartnerReflectionDeps {
        runtime,
        agent,
        now_unix,
        tools: Some(&tools),
    };
    match crate::life_node::daily_review::run_coach_review(home, date, true, Some(partner)).await {
        Ok(r) => {
            let where_to = r
                .saved_to
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(not saved)".to_string());
            tracing::info!(
                date,
                events = r.event_count,
                encrypted = r.saved_encrypted,
                saved_to = %where_to,
                "coach scheduler: daily review fired in-process"
            );
            // ③ ARRIVAL: a saved (often age-encrypted) file reaches no one. Push a
            // real notification through the same channel stack the task-notifier
            // uses, so the proactive review actually lands in front of the user.
            // Best-effort: a missing/empty dispatcher just no-ops (the file still
            // saved), never failing the loop.
            if let Some(disp) = notifier {
                let n = crate::notifications::Notification {
                    id: uuid::Uuid::new_v4(),
                    dedup_key: format!("daily-coach-notify:{date}"),
                    task_id: None,
                    workspace_id: agent.to_string(),
                    priority: crate::notifications::NotificationPriority::P0,
                    title: format!("spectyn · daily review ({date})"),
                    body: coach_notification_body(&r.markdown, r.event_count),
                    actions: vec![],
                    timestamp: (now_unix as i64).saturating_mul(1000),
                };
                disp.notify(n).await;
                tracing::info!(date, "coach scheduler: dispatched daily-review notification");
            }
        }
        Err(e) => {
            // Do NOT panic the serve loop on a bad review — log and carry on so
            // tomorrow's run still happens. (The ledger key already recorded
            // today; a transient failure forfeits today's review rather than
            // hot-looping a broken provider — matches the once-per-day intent.)
            tracing::error!(date, error = %e, "coach scheduler: daily review failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    /// `SPECTYN_IDEMPOTENCY_STORE` / `SPECTYN_COACH_DISABLE` are process-global,
    /// but `cargo test` runs tests in parallel — one test's `remove_var` can
    /// clobber another's `set_var` mid-flight. Serialize the env-mutating tests
    /// behind one lock (same pattern as `partner::tests::ENV_LOCK`).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Build a deterministic local `DateTime` for tests (a fixed wall clock).
    fn local(y: i32, m: u32, d: u32, hh: u32, mm: u32, ss: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(y, m, d, hh, mm, ss)
            .single()
            .expect("test datetime is unambiguous")
    }

    #[test]
    fn next_fire_before_target_is_same_day() {
        // 08:00, target 21:00 → 13h to go, still today.
        let now = local(2026, 6, 6, 8, 0, 0);
        let d = next_fire_duration(now, CoachSchedule::new(21, 0).unwrap());
        assert_eq!(d, Duration::from_secs(13 * 3600), "13h until today's 21:00");
    }

    #[test]
    fn next_fire_after_target_rolls_to_tomorrow() {
        // 22:30, target 21:00 → already past today; next is tomorrow 21:00 =
        // 22h30m away (1h30m to midnight + 21h).
        let now = local(2026, 6, 6, 22, 30, 0);
        let d = next_fire_duration(now, CoachSchedule::new(21, 0).unwrap());
        let expected = (90 + 21 * 60) * 60; // minutes → secs
        assert_eq!(d.as_secs(), expected as u64, "tomorrow's 21:00, ~22h30m");
    }

    #[test]
    fn next_fire_exactly_at_target_waits_full_day_not_zero() {
        // Exactly 21:00:00 → must NOT return 0 (which would busy-spin), it waits
        // a full 24h for tomorrow's 21:00.
        let now = local(2026, 6, 6, 21, 0, 0);
        let d = next_fire_duration(now, CoachSchedule::new(21, 0).unwrap());
        assert_eq!(d.as_secs(), 24 * 3600, "exactly at target → next day, never 0");
        assert!(d.as_secs() > 0, "never fires the same instant twice");
    }

    #[test]
    fn next_fire_one_second_before_target_is_one_second() {
        let now = local(2026, 6, 6, 20, 59, 59);
        let d = next_fire_duration(now, CoachSchedule::new(21, 0).unwrap());
        assert_eq!(d, Duration::from_secs(1), "1s until 21:00");
    }

    #[test]
    fn next_fire_crosses_month_boundary() {
        // 23:00 on the last day of June, target 21:00 → past → July 1 21:00.
        let now = local(2026, 6, 30, 23, 0, 0);
        let d = next_fire_duration(now, CoachSchedule::new(21, 0).unwrap());
        // 1h to midnight (July 1 00:00) + 21h to July 1 21:00 = 22h.
        assert_eq!(d.as_secs(), 22 * 3600, "rolls into next month");
    }

    #[test]
    fn next_fire_honors_custom_time() {
        // Custom 06:30 trigger at 05:00 → 1h30m.
        let now = local(2026, 6, 6, 5, 0, 0);
        let d = next_fire_duration(now, CoachSchedule::new(6, 30).unwrap());
        assert_eq!(d.as_secs(), 90 * 60, "1h30m until 06:30");
    }

    #[test]
    fn dedup_key_is_date_scoped() {
        assert_eq!(dedup_key("2026-06-06"), "daily-coach:2026-06-06");
        assert_ne!(
            dedup_key("2026-06-06"),
            dedup_key("2026-06-07"),
            "different calendar days → different keys"
        );
    }

    #[test]
    fn today_and_sleep_resolves_calendar_day_under_mock_clock() {
        use crate::clock::MockClock;
        let sched = CoachSchedule::new(21, 0).unwrap();

        // Pin the mock clock at MIDDAY UTC (12:00) so the LOCAL calendar date the
        // helper resolves equals the UTC date in every CI timezone (offsets in
        // [-11, +11] can't roll a midday-UTC instant across a date boundary) —
        // the daily-transition determinism is asserted without a tz flake.
        let clock = MockClock::at_utc_date(2026, 5, 22);
        clock.advance_ms(12 * 3600 * 1000); // 12:00 UTC on 2026-05-22
        let (today, sleep_for) = today_and_sleep_with_clock(&clock, sched);
        assert_eq!(today, "2026-05-22", "resolves the pinned calendar day");
        // Sleep-until-next-fire is finite + future (never a busy-spinning zero).
        assert!(sleep_for.as_secs() > 0, "sleep is a finite future duration");

        // Roll a full 24h forward → the daily transition resolves the NEXT day.
        clock.advance_ms(24 * 3600 * 1000); // 12:00 UTC on 2026-05-23
        let (tomorrow, _) = today_and_sleep_with_clock(&clock, sched);
        assert_eq!(tomorrow, "2026-05-23", "the midnight roll resolves the next day");

        // The observable daily-transition contract: distinct days ⇒ distinct
        // dedup keys ⇒ distinct daily runs (no two calendar days collapse).
        assert_ne!(dedup_key(&today), dedup_key(&tomorrow));
    }

    #[test]
    fn daily_dedup_skips_second_run_same_day() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // Drive the SAME idempotency ledger the loop uses (via the env override)
        // and assert: first sighting of the day's key proceeds, a second
        // sighting within TTL is a duplicate (so the loop would skip the rerun).
        let dir = tempfile::tempdir().unwrap();
        let ledger = dir.path().join("idempotency.jsonl");
        // Scope the env override to this test; restore on drop is implicit since
        // each test process is isolated, but be explicit to avoid cross-talk.
        std::env::set_var("SPECTYN_IDEMPOTENCY_STORE", &ledger);

        let key = dedup_key("2026-06-06");
        let first = crate::idempotency::check_and_record(&key, COACH_KIND, COACH_TTL_SECS);
        assert!(first.is_first(), "first run of the day proceeds: {first:?}");

        let second = crate::idempotency::check_and_record(&key, COACH_KIND, COACH_TTL_SECS);
        assert!(
            second.is_duplicate(),
            "a second attempt the same day is a duplicate → loop skips: {second:?}"
        );

        // A DIFFERENT day's key is independent (tomorrow still fires).
        let tomorrow = crate::idempotency::check_and_record(
            &dedup_key("2026-06-07"),
            COACH_KIND,
            COACH_TTL_SECS,
        );
        assert!(tomorrow.is_first(), "next calendar day fires again: {tomorrow:?}");

        std::env::remove_var("SPECTYN_IDEMPOTENCY_STORE");
    }

    #[test]
    fn disabled_env_parsing() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var(DISABLE_ENV);
        assert!(!is_disabled(), "unset → enabled");
        std::env::set_var(DISABLE_ENV, "1");
        assert!(is_disabled(), "1 → disabled");
        std::env::set_var(DISABLE_ENV, "TRUE");
        assert!(is_disabled(), "TRUE (case-insensitive) → disabled");
        std::env::set_var(DISABLE_ENV, "0");
        assert!(!is_disabled(), "0 → enabled");
        std::env::set_var(DISABLE_ENV, "");
        assert!(!is_disabled(), "empty → enabled");
        std::env::remove_var(DISABLE_ENV);
    }

    #[test]
    fn notification_body_prefers_tomorrow_action() {
        let md = "# Daily review\n\nsome events\n\n## Tomorrow's one action\n\nGo for a 20-minute walk before noon.\n\n## Daily alignment\n\n...";
        assert_eq!(
            coach_notification_body(md, 4),
            "Tomorrow: Go for a 20-minute walk before noon."
        );
    }

    #[test]
    fn notification_body_falls_back_when_action_skipped() {
        // A "(skipped: ...)" action is not a real nudge → fall back to a summary.
        let md = "## Tomorrow's one action\n\n(skipped: no GEMINI_API_KEY)\n";
        assert_eq!(
            coach_notification_body(md, 2),
            "2 moment(s) reviewed — your daily check-in is ready."
        );
    }

    #[test]
    fn notification_body_zero_events_has_friendly_default() {
        assert_eq!(
            coach_notification_body("no header here", 0),
            "Your daily review is ready."
        );
    }

    #[test]
    fn truncate_chars_is_utf8_safe_and_bounded() {
        // ASCII over the limit → ellipsis, length == max.
        let long = "a".repeat(200);
        let out = truncate_chars(&long, 180);
        assert_eq!(out.chars().count(), 180);
        assert!(out.ends_with('…'));
        // Under the limit → unchanged.
        assert_eq!(truncate_chars("short", 180), "short");
        // Multi-byte chars must not be split mid-codepoint.
        let emoji = "🚶".repeat(100);
        let out = truncate_chars(&emoji, 10);
        assert_eq!(out.chars().count(), 10);
        assert!(out.ends_with('…'));
    }

    /// End-to-end wiring proof for ability ③ *arrival*: a successful fire must
    /// actually dispatch a notification through the channel stack — not just save
    /// a file. Drives the REAL `run_once_for_date` with a recording channel.
    #[tokio::test]
    async fn run_once_dispatches_notification_after_fire() {
        use std::sync::Arc;
        use tokio::sync::Mutex as AsyncMutex;

        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();

        // Hermetic + fast: fresh ledger (this fire is the day's first), and no
        // provider keys / Ollama so the review's best-effort LLM passes fail fast
        // into "(skipped: …)" footers. run_coach_review still returns Ok and
        // saves — the precondition for the dispatch we're asserting.
        let provider_keys = ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "GEMINI_API_KEY", "GROQ_API_KEY"];
        let saved: Vec<(&str, Option<String>)> =
            provider_keys.iter().map(|k| (*k, std::env::var(k).ok())).collect();
        for k in provider_keys {
            std::env::remove_var(k);
        }
        std::env::set_var("OLLAMA_DISABLE", "1");
        std::env::set_var("SPECTYN_IDEMPOTENCY_STORE", home.join("idem.jsonl"));

        // A channel that records every notification it is handed.
        struct RecCh(Arc<AsyncMutex<Vec<crate::notifications::Notification>>>);
        #[async_trait::async_trait]
        impl crate::notifications::channels::NotificationChannel for RecCh {
            fn name(&self) -> &str {
                "rec"
            }
            async fn send(&self, n: &crate::notifications::Notification) -> anyhow::Result<()> {
                self.0.lock().await.push(n.clone());
                Ok(())
            }
            async fn send_batch(
                &self,
                ns: &[crate::notifications::Notification],
            ) -> anyhow::Result<()> {
                self.0.lock().await.extend_from_slice(ns);
                Ok(())
            }
        }
        let captured = Arc::new(AsyncMutex::new(Vec::new()));
        let disp = crate::NotificationDispatcher::new();
        disp.add_channel(Arc::new(RecCh(captured.clone()))).await;

        let runtime = crate::agent::AgentRuntime::new(crate::config::AgentsConfig::default());
        run_once_for_date(home, &runtime, "master", "2026-06-06", Some(&disp)).await;

        // P0 sends fan out via tokio::spawn — give them a beat to land.
        tokio::time::sleep(Duration::from_millis(150)).await;

        {
            let got = captured.lock().await;
            assert_eq!(got.len(), 1, "exactly one notification dispatched after a successful fire");
            assert!(got[0].title.contains("daily review"), "title was: {}", got[0].title);
            assert_eq!(got[0].priority, crate::notifications::NotificationPriority::P0);
        }
        assert!(
            home.join(".spectyn-mesh/reviews/2026-06-06.md").exists(),
            "the review file was also saved alongside the notification"
        );

        std::env::remove_var("OLLAMA_DISABLE");
        std::env::remove_var("SPECTYN_IDEMPOTENCY_STORE");
        for (k, v) in saved {
            if let Some(v) = v {
                std::env::set_var(k, v);
            }
        }
    }
}
