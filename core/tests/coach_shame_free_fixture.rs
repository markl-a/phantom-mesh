//! P0-6 — SPEC-23 G3 `T-coach-shame-free-fixture` + the prompt-lint pattern-set
//! lock + the below-threshold "gentle, never punitive" delivery-gate invariant.
//!
//! The shame-free gate (`coach_prompts::lint::check`) is fail-closed: one match
//! rejects the WHOLE review. This battery is the cross-crate SSOT a reviewer
//! checks — ≥100 shaming lines (zh+en) that MUST all be rejected, ≥100 clean
//! supportive lines that MUST all pass, the locked shame+medical vocabulary that
//! must stay rejected, and the executable form of BIG-GOAL Operational Principle
//! #1: a fully-behind day is reported NEUTRALLY (signed deviation), never turned
//! into a reprimand.
//!
//! Fixture maintenance rule (constraint 3): a clean-line FALSE-REJECT is fixed in
//! the FIXTURE (never by weakening the lint); only a real shaming LEAK extends
//! the lint pattern set (high-precision, with the clean battery re-proven).

use phantom_mesh::life_node::coach_prompts::lint::check;

/// Read a fixture file into trimmed, non-empty lines.
fn lines(raw: &str) -> Vec<&str> {
    raw.lines().map(str::trim).filter(|l| !l.is_empty()).collect()
}

#[test]
fn every_shaming_line_is_rejected() {
    let raw = include_str!("fixtures/shame_free/shaming.txt");
    let lines = lines(raw);
    assert!(
        lines.len() >= 100,
        "fixture must hold >=100 shaming lines, got {}",
        lines.len()
    );
    let leaked: Vec<&str> = lines.iter().copied().filter(|l| check(l).is_ok()).collect();
    assert!(
        leaked.is_empty(),
        "shame-free gate LEAKED these lines (must be 0): {:#?}",
        leaked
    );
}

#[test]
fn every_clean_line_passes() {
    let raw = include_str!("fixtures/shame_free/clean.txt");
    let lines = lines(raw);
    assert!(
        lines.len() >= 100,
        "fixture must hold >=100 clean lines, got {}",
        lines.len()
    );
    let false_rejects: Vec<(&str, String)> = lines
        .iter()
        .filter_map(|l| check(l).err().map(|e| (*l, e)))
        .collect();
    assert!(
        false_rejects.is_empty(),
        "gate FALSE-REJECTED clean lines (must be 0): {:#?}",
        false_rejects
    );
}

/// Pattern-set LOCK: the locked shame+medical vocabulary must stay rejected, so a
/// future refactor that drops a pattern fails here loudly (the cross-crate SSOT
/// for the set). Every literal below is an actual entry of `lint`'s SHAME/MEDICAL
/// pattern tables (or, for `diagnosis`, a superstring of the `diagnos` entry).
#[test]
fn locked_pattern_set_stays_rejected() {
    for p in [
        // SHAME (zh)
        "你又",
        "你終於",
        "你居然",
        "你怎麼又",
        "還不去",
        "還不快",
        // SHAME (en)
        "you failed again",
        "you never learn",
        "shame on you",
        "you should be ashamed",
        "what's wrong with you",
        // MEDICAL (zh+en)
        "處方",
        "診斷",
        "prescription",
        "prescribe",
        "diagnosis",
        "dosage",
    ] {
        assert!(check(p).is_err(), "locked pattern no longer rejected: {p:?}");
    }
}

// ─── Task 4: below-threshold delivery GATE — gentle, never punitive ───────────

use phantom_mesh::life_node::daily_review::{deviation_section, golden_review};
use phantom_mesh::life_node::goals::Goal;

/// A worst-case day for two goals: nothing logged, so both deviations are
/// maximally negative (focus -180, reading -30).
fn fully_behind_goals() -> Vec<Goal> {
    vec![
        Goal {
            tag: "focus".into(),
            target: 180.0,
            unit: "minutes".into(),
            window: "daily".into(),
        },
        Goal {
            tag: "reading".into(),
            target: 30.0,
            unit: "minutes".into(),
            window: "daily".into(),
        },
    ]
}

#[test]
fn below_threshold_day_review_is_gentle_not_punitive() {
    let pairs = vec![]; // no logged minutes for either goal → maximally "behind"
    let goals = fully_behind_goals();
    let review = golden_review("2026-05-22", &pairs, &goals);

    // (a) the gate stays GREEN on a bad day — no shame/medical leakage in the
    // rendered review.
    assert!(
        check(&review).is_ok(),
        "a fully-behind day must still pass the shame-free gate:\n{review}"
    );

    // (b) it states the gap NEUTRALLY (signed deviation), with none of the
    // punitive vocabulary.
    assert!(
        review.contains("deviation -180"),
        "neutral signed gap present:\n{review}"
    );
    for bad in ["你又", "failed again", "shame", "ashamed", "what's wrong"] {
        assert!(
            !review.to_lowercase().contains(&bad.to_lowercase()),
            "review must not contain punitive token {bad:?}:\n{review}"
        );
    }
}

#[test]
fn behind_goal_nudge_body_is_shame_free() {
    // The DELIVERY leg, not just the saved file: the behind-goal deviation
    // section (which renders the same "落後目標 / behind by N" intent the desktop
    // nudge body carries) must also pass the shame-free gate. `nudge_body` itself
    // is private; `deviation_section` is the public surface with identical
    // "short by N" phrasing, so we assert through it (no visibility change).
    let goals = vec![Goal {
        tag: "focus".into(),
        target: 180.0,
        unit: "minutes".into(),
        window: "daily".into(),
    }];
    let section = deviation_section(&goals, &[]);
    assert!(
        section.contains("deviation -180"),
        "the behind-goal section reports the signed gap:\n{section}"
    );
    assert!(
        check(&section).is_ok(),
        "the behind-goal deviation/nudge phrasing must be shame-free:\n{section}"
    );
}
