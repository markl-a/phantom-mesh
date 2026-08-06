//! P0-6 — deterministic, shame-free daily loop: golden-fixture review,
//! capture→store→recall round-trip, and clock-pinned daily transition.
//!
//! These are cross-crate integration tests (import via `spectyn_mesh::`). The
//! golden review is asserted byte-for-byte against a committed `.golden.md`
//! artifact built from a fixed `events.json`, with a `SPECTYN_UPDATE_GOLDEN=1`
//! escape hatch to regenerate it intentionally. The review surface
//! (`golden_review`) reads no clock and no filesystem, so the bytes are stable
//! across platforms and timezones (SPEC-23 §G5 parity) — proven in Task 5 by
//! running this test under `TZ=UTC` / `Asia/Taipei` / `America/Los_Angeles`.

use spectyn_mesh::event_storage_wire::EventMeta;
use spectyn_mesh::life_node::daily_review::golden_review;
use spectyn_mesh::life_node::goals::Goal;
use spectyn_mesh::life_node::multimodal::AnalysisResult;

/// One `{meta, analysis}` row of the fixture. `EventMeta` is `camelCase` on the
/// wire (`eventId`), `AnalysisResult` is plain `snake_case` — the fixture JSON
/// matches both exactly.
#[derive(serde::Deserialize)]
struct Pair {
    meta: EventMeta,
    analysis: AnalysisResult,
}

/// Deserialize the committed fixture into the `(EventMeta, AnalysisResult)`
/// pairs `golden_review` consumes. Shared by the byte-stable + round-trip tests.
fn fixture_pairs() -> Vec<(EventMeta, AnalysisResult)> {
    let raw = include_str!("fixtures/daily_review/events.json");
    serde_json::from_str::<Vec<Pair>>(raw)
        .expect("events.json must deserialize into Vec<Pair>")
        .into_iter()
        .map(|p| (p.meta, p.analysis))
        .collect()
}

/// The fixed goal set the golden review is pinned against: a focus target of
/// 180 minutes/day. The fixture logs a single 50-minute focus session, so the
/// rendered deviation is `-130` — a deterministic, signed gap.
fn fixture_goals() -> Vec<Goal> {
    vec![Goal {
        tag: "focus".into(),
        target: 180.0,
        unit: "minutes".into(),
        window: "daily".into(),
    }]
}

#[test]
fn golden_review_is_byte_stable() {
    let pairs = fixture_pairs();
    let goals = fixture_goals();
    let out = golden_review("2026-05-22", &pairs, &goals);

    // (a) self-consistency: pure ⇒ identical on a second call (catches any
    // accidental iteration-order drift — `aggregate` uses a BTreeMap, so this
    // should always hold; the assertion documents + enforces it).
    assert_eq!(
        out,
        golden_review("2026-05-22", &pairs, &goals),
        "golden_review must be deterministic"
    );

    // (b) byte-exact vs the committed artifact, with an explicit update escape
    // hatch. CI never sets SPECTYN_UPDATE_GOLDEN; a developer regenerates the
    // golden intentionally and reviews the diff.
    let golden_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/daily_review/2026-05-22.golden.md"
    );
    if std::env::var("SPECTYN_UPDATE_GOLDEN").as_deref() == Ok("1") {
        std::fs::write(golden_path, &out).expect("write golden artifact");
    } else {
        let expected = std::fs::read_to_string(golden_path)
            .expect("golden artifact must exist — generate it with SPECTYN_UPDATE_GOLDEN=1");
        assert_eq!(
            out, expected,
            "review drifted from golden; rerun with SPECTYN_UPDATE_GOLDEN=1 and review the diff"
        );
    }
}

#[test]
fn capture_then_recall_hits_the_event() {
    let tmp = tempfile::tempdir().unwrap();
    let spectyn = tmp.path().join(".spectyn-mesh");
    std::fs::create_dir_all(&spectyn).unwrap();
    spectyn_mesh::life_node::note_capture::capture_note(
        &spectyn,
        "rebased the daily loop onto the mockable clock",
        &["dev".into()],
    )
    .unwrap();

    let hits = spectyn_mesh::life_node::recall::search_events(
        &spectyn.join("events"),
        None,
        &spectyn_mesh::life_node::recall::RecallFilter::text("clock"),
        15,
    )
    .unwrap();
    assert_eq!(hits.len(), 1, "the captured note is recallable by a content term");
    assert!(hits[0].summary.contains("mockable clock"));
    assert_eq!(hits[0].kind, "text"); // "note" projects to EventKind::Text
}

#[test]
fn capture_round_trips_through_daily_review_loader() {
    let tmp = tempfile::tempdir().unwrap();
    let spectyn = tmp.path().join(".spectyn-mesh");
    std::fs::create_dir_all(&spectyn).unwrap();
    let out = spectyn_mesh::life_node::note_capture::capture_note(
        &spectyn,
        "shipped the golden fixture",
        &["note".into()],
    )
    .unwrap();

    // Resolve "today" the way load_events_for_date does (LOCAL date of the
    // just-written event's UTC timestamp) so this is tz-correct, not
    // wall-clock-flaky.
    let meta = spectyn_mesh::life_node::storage::EventStore::new(&spectyn.join("events"))
        .read_meta(&out.event_id)
        .unwrap();
    let today = spectyn_mesh::event_storage_wire::ts_local_date(&meta.timestamp);

    let pairs = spectyn_mesh::life_node::daily_review::load_events_for_date(
        &spectyn.join("events"),
        &today,
        None,
    )
    .unwrap();
    assert_eq!(pairs.len(), 1, "the captured note loads back for its own local date");
    assert_eq!(pairs[0].1.summary, "shipped the golden fixture");
}

/// Task 2 cross-crate contract: distinct calendar days resolve to distinct
/// dedup keys, so the daily-transition (one review per local day) is honoured.
/// The clock-injectable date resolution itself (`today_and_sleep_with_clock`)
/// is asserted in `coach_scheduler_daemon`'s own in-module tests, where the
/// private surface is reachable; here we drive a `MockClock` across a midnight
/// roll and assert the OBSERVABLE contract — the two days key differently.
#[test]
fn daily_transition_resolves_distinct_dedup_keys_across_midnight() {
    use spectyn_mesh::clock::{Clock, MockClock};
    use spectyn_mesh::life_node::coach_scheduler_daemon::dedup_key;

    let clock = MockClock::at_utc_date(2026, 5, 22);
    clock.advance_ms(13 * 3600 * 1000); // 13:00 UTC on 2026-05-22
    let day1 = clock.now_utc().format("%Y-%m-%d").to_string();
    assert_eq!(day1, "2026-05-22");

    clock.advance_ms(24 * 3600 * 1000); // roll a full day forward
    let day2 = clock.now_utc().format("%Y-%m-%d").to_string();
    assert_eq!(day2, "2026-05-23");

    // The daily-transition invariant: distinct days ⇒ distinct dedup keys ⇒
    // distinct daily runs (no review collapses two calendar days into one).
    assert_ne!(
        dedup_key(&day1),
        dedup_key(&day2),
        "a calendar-day transition must key to a different daily run"
    );
}
