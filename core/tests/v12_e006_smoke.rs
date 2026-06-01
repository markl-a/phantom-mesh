// Phase E V12 — E006 30-second hello smoke test.
//
// Programmatic equivalent of `scripts/demo-30sec-life-hello.sh`: build 3 mock
// events in-memory (no LLM call), feed them through the pure
// `coach_wire::aggregate` markdown formatter, and assert the §7.1 markdown
// brief contract holds — header, count line, per-tag sections.
//
// We deliberately use the wire-level `aggregate(&[(EventMeta, AnalysisResult)])`
// surface (pure, no I/O, no provider call) rather than `run_daily_review`
// (which still hits Stage 4 panicking markers on the providers + memory
// helpers). That keeps this smoke test deterministic + offline + zero-cost.
//
// Reference: docs/superpowers/PHASE-E-INTEGRATION-TEST-PLAN.md §V12.

use phantom_mesh::coach_wire::aggregate;
use phantom_mesh::event_storage_wire::{AnalysisResult, EventKind, EventMeta};

/// Build a deterministic mock `AnalysisResult` — matches the real wire shape
/// in `event_storage_wire.rs` (confidence: f32, goal_impact/suggestion: String,
/// cost_usd: f64, raw_response: String). NOT `Option<...>` wrappers — the
/// wire type uses concrete defaults to keep JSON round-trip stable.
fn mock_analysis(summary: &str) -> AnalysisResult {
    AnalysisResult {
        summary: summary.to_string(),
        confidence: 0.8,
        goal_impact: "positive".to_string(),
        suggestion: "keep going".to_string(),
        cost_usd: 0.001,
        latency_ms: 100,
        model_id: "test-mock".to_string(),
        raw_response: "{}".to_string(),
    }
}

/// Build a deterministic mock `EventMeta` for `(kind, primary_tag)`. UUIDs +
/// timestamps are stable strings so the markdown output is byte-identical
/// across runs (no clock dependency).
fn mock_meta(event_id: &str, ts_iso: &str, kind: EventKind, tag: &str) -> EventMeta {
    EventMeta {
        event_id: event_id.to_string(),
        timestamp: ts_iso.to_string(),
        kind,
        tags: vec![tag.to_string()],
    }
}

#[test]
fn v12_e006_capture_then_aggregate() {
    // E006 demo flow: 3 captures (food + focus + habit) on a single day, then
    // run the pure-function aggregator that the daily-review coach pipeline
    // consumes before the LLM call. No tempdir + write_event round-trip
    // because `read_event` still routes through the Stage 4 keystore bridge
    // (`encryption_key_available_pseudo` panics) — the wire-level aggregate()
    // is the deterministic boundary the E006 smoke targets.
    let events: Vec<(EventMeta, AnalysisResult)> = vec![
        (
            mock_meta("e1", "2026-05-25T08:00:00Z", EventKind::Food, "fat_loss"),
            mock_analysis("Caesar salad good protein"),
        ),
        (
            mock_meta("e2", "2026-05-25T10:30:00Z", EventKind::Focus, "focus"),
            mock_analysis("90 min deep work block"),
        ),
        (
            mock_meta("e3", "2026-05-25T22:00:00Z", EventKind::Habit, "habit"),
            mock_analysis("evening walk 20 min"),
        ),
    ];

    let md = aggregate(&events);

    // §7.1 markdown contract: header + count line are mandatory.
    assert!(
        md.contains("# Daily review"),
        "missing daily-review header: {md}"
    );
    assert!(
        md.contains("**Events captured:** 3"),
        "missing 3-event count line: {md}"
    );

    // Per-tag sections (BTreeMap ordering → alphabetical). All three tags
    // surface; the food event's tag is `fat_loss` (matches SPEC-23 §7.1 +
    // existing coach_wire `aggregate_one_event_pins_section_shape` KAT).
    assert!(md.contains("## fat_loss (1)"), "missing fat_loss section: {md}");
    assert!(md.contains("## focus (1)"), "missing focus section: {md}");
    assert!(md.contains("## habit (1)"), "missing habit section: {md}");

    // Date stamp derived from first event's `timestamp[..10]`.
    assert!(
        md.contains("# Daily review — 2026-05-25"),
        "wrong date stamp: {md}"
    );

    // Bullet shape per `format_section`: `- **<kind>** (<ts>): <summary>`.
    assert!(
        md.contains("- **food** (2026-05-25T08:00:00Z): Caesar salad good protein"),
        "food bullet shape drift: {md}"
    );
}

#[test]
fn v12_e006_zero_events_graceful() {
    // Empty event list must NOT panic and must return the canonical
    // `(no events for this date)` stub. This is the same shape the coach
    // UI renders on a fresh install or backfill date with zero captures.
    let md = aggregate(&[]);
    assert!(md.contains("# Daily review"), "header missing on empty: {md}");
    assert!(md.contains("**Events captured:** 0"), "zero-count line missing: {md}");
    assert!(
        md.contains("(no events for this date)"),
        "empty stub missing: {md}"
    );
}

#[test]
fn v12_e006_event_kind_grouping() {
    // 5 events, mixed kinds (Food×2, Focus×1, Habit×2). Each tag bucket
    // must surface once with the correct `(count)` suffix — this guards
    // the SPEC-23 §7.1 grouping invariant that powers the LLM brief.
    let events: Vec<(EventMeta, AnalysisResult)> = vec![
        (
            mock_meta("k1", "2026-05-25T07:00:00Z", EventKind::Food, "fat_loss"),
            mock_analysis("breakfast: oatmeal + berries"),
        ),
        (
            mock_meta("k2", "2026-05-25T12:30:00Z", EventKind::Food, "fat_loss"),
            mock_analysis("lunch: grilled chicken bowl"),
        ),
        (
            mock_meta("k3", "2026-05-25T14:00:00Z", EventKind::Focus, "focus"),
            mock_analysis("2h coding sprint"),
        ),
        (
            mock_meta("k4", "2026-05-25T18:00:00Z", EventKind::Habit, "habit"),
            mock_analysis("stretch 10 min"),
        ),
        (
            mock_meta("k5", "2026-05-25T22:30:00Z", EventKind::Habit, "habit"),
            mock_analysis("journaled 3 lines"),
        ),
    ];

    let md = aggregate(&events);

    assert!(
        md.contains("**Events captured:** 5"),
        "missing 5-event count: {md}"
    );
    // Counts per tag bucket — 2 / 1 / 2.
    assert!(
        md.contains("## fat_loss (2)"),
        "fat_loss should have 2 events: {md}"
    );
    assert!(
        md.contains("## focus (1)"),
        "focus should have 1 event: {md}"
    );
    assert!(
        md.contains("## habit (2)"),
        "habit should have 2 events: {md}"
    );

    // Each tag section must appear exactly once (no duplicate headers).
    assert_eq!(
        md.matches("## fat_loss").count(),
        1,
        "fat_loss section duplicated: {md}"
    );
    assert_eq!(
        md.matches("## focus").count(),
        1,
        "focus section duplicated: {md}"
    );
    assert_eq!(
        md.matches("## habit").count(),
        1,
        "habit section duplicated: {md}"
    );
}
