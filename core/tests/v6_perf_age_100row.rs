//! Phase E V6 — `encryption_wire` age v1 batch throughput budget gate.
//!
//! Per `docs/superpowers/PHASE-E-INTEGRATION-TEST-PLAN.md` §3.6 + SPEC-13 §13
//! perf budget, the at-rest encryption layer MUST sustain **100 rows × ~1 KB
//! plaintext encrypt + decrypt round-trip in ≤ 2 s wall clock** on a
//! developer-class machine. This is the "user opens history pane and we
//! decrypt the last 100 events in one sweep" UX gate.
//!
//! These tests are **absolute-threshold pass/fail** — v0.6.0 GA mandate is
//! "SPEC ceiling met?", not "regression vs last week". Historical baseline
//! deltas live in `.perf-baseline/history.jsonl` (V6 plan §3.6).
//!
//! Pipeline per test:
//!   1. Derive an `EventKey` from a fixed 32-byte seed (HKDF-SHA256 — pure CPU).
//!   2. Convert to age x25519 identity → recipient (bech32 `age1...`).
//!   3. Loop N times: `encrypt_event(plaintext, &recipient)` →
//!      `decrypt_event(&envelope, &identity)` round-trip.
//!   4. Assert wall-clock elapsed ≤ budget.
//!
//! No I/O (no `~/.phantom-mesh/blobs/*` writes), no network — everything runs
//! in-memory through the Stage 3 real impl in `core/src/encryption_wire.rs`.
//! `std::time::Instant` for measurement, no extra crates.

use std::time::{Duration, Instant};

use phantom_mesh::encryption_wire::{
    decrypt_event, derive_event_key_from_identity, derive_recipient_from_identity, encrypt_event,
    event_key_to_age_identity,
};

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Build the deterministic crypto material once per test. OSS-safe: uses a
/// fixed byte pattern seed (no real identity material, no operator keys).
fn build_crypto_material() -> (
    phantom_mesh::encryption_wire::X25519Identity,
    phantom_mesh::encryption_wire::X25519Recipient,
) {
    // Fixed seed pattern — purely a test fixture.
    let seed = [0xA5u8; 32];
    let key = derive_event_key_from_identity(&seed).expect("HKDF derive must succeed");
    let identity = event_key_to_age_identity(&key).expect("age identity build must succeed");
    let recipient = derive_recipient_from_identity(&identity);
    assert!(
        recipient.0.starts_with("age1"),
        "recipient bech32 must start with `age1`"
    );
    (identity, recipient)
}

/// Deterministic ~1 KB plaintext row. OSS-safe: ASCII letters + index counter
/// only — no PII, no hostnames, no emails.
fn build_row(index: usize) -> Vec<u8> {
    // 1 KB target — 1024 chars of repeating padded content.
    let mut s = format!("event-{index:06}-payload:");
    let base = "abcdefghijklmnopqrstuvwxyz0123456789";
    while s.len() < 1024 {
        s.push_str(base);
    }
    s.truncate(1024);
    s.into_bytes()
}

// ─── 1/3 — 100 rows × ~1 KB (SPEC-listed nominal) ──────────────────────────

/// **Budget**: 100 rows × ~1 KB encrypt+decrypt round-trip ≤ 2 s wall clock.
///
/// Canonical V6 gate per `PHASE-E-INTEGRATION-TEST-PLAN.md` §3.6 table line
/// `v6_perf_age_100row.rs`. Failure ⇒ ship-block: the history-pane UX
/// (decrypt last 100 events on open) would regress past the perceptible-lag
/// threshold.
#[test]
fn v6_age_100_rows_round_trip_under_2s() {
    const BUDGET: Duration = Duration::from_secs(2);
    const N: usize = 100;
    let (identity, recipient) = build_crypto_material();
    let rows: Vec<Vec<u8>> = (0..N).map(build_row).collect();

    let start = Instant::now();
    for row in &rows {
        let env = encrypt_event(row, &recipient).expect("encrypt must succeed");
        let recovered = decrypt_event(&env, &identity).expect("decrypt must succeed");
        // Sanity assertion — bail loud if AEAD silently dropped bytes.
        debug_assert_eq!(recovered.len(), row.len(), "row size mismatch");
    }
    let elapsed = start.elapsed();

    assert!(
        elapsed <= BUDGET,
        "V6.AGE.1 age round-trip(100 × ~1 KB) took {:?}, budget {:?}",
        elapsed,
        BUDGET
    );
    println!("V6.AGE.1 age round-trip(100 × ~1 KB) = {:?} (budget {:?})", elapsed, BUDGET);
}

// ─── 2/3 — 500 rows × ~1 KB (5× nominal stress) ────────────────────────────

/// **Budget**: 500 rows × ~1 KB encrypt+decrypt round-trip ≤ 10 s wall clock.
///
/// Stress sample at 5× nominal — covers full-week history backfill (~7 days
/// × ~70 events = 490 ish). Conservative 10 s ceiling derived from a linear
/// extrapolation of the 2 s / 100 rows nominal (5×). Failure here means age
/// throughput degraded super-linearly — would surface as multi-second hangs
/// when scrolling through history.
#[test]
fn v6_age_500_rows_round_trip_under_10s() {
    const BUDGET: Duration = Duration::from_secs(10);
    const N: usize = 500;
    let (identity, recipient) = build_crypto_material();
    let rows: Vec<Vec<u8>> = (0..N).map(build_row).collect();

    let start = Instant::now();
    for row in &rows {
        let env = encrypt_event(row, &recipient).expect("encrypt must succeed");
        let recovered = decrypt_event(&env, &identity).expect("decrypt must succeed");
        debug_assert_eq!(recovered.len(), row.len(), "row size mismatch");
    }
    let elapsed = start.elapsed();

    assert!(
        elapsed <= BUDGET,
        "V6.AGE.2 age round-trip(500 × ~1 KB) took {:?}, budget {:?}",
        elapsed,
        BUDGET
    );
    println!("V6.AGE.2 age round-trip(500 × ~1 KB) = {:?} (budget {:?})", elapsed, BUDGET);
}

// ─── 3/3 — Single large row: 1 MB ──────────────────────────────────────────

/// **Budget**: single 1 MB row encrypt+decrypt round-trip ≤ 3 s wall clock.
///
/// Edge case — a single fat capture (e.g. an audio transcript or pasted log
/// snippet near the SPEC-20 §8.1 read-side ceiling). The age v1 streaming
/// AEAD should handle 1 MB in 1-2 s on a modern laptop (measured ~1.3 s on
/// M-class Mac); budget set at 3 s for ~2× headroom against CI scheduler
/// noise + Linux runner variance. Failure here would suggest the age crate
/// underwent a behavioural regression (e.g. degraded ChaCha20-Poly1305 chunk
/// size).
#[test]
fn v6_age_single_1mb_row_under_3s() {
    const BUDGET: Duration = Duration::from_secs(3);
    let (identity, recipient) = build_crypto_material();

    // Build a 1 MB plaintext deterministically (no random — keeps the test
    // reproducible across runs and OSS-safe).
    let plaintext: Vec<u8> = (0..(1024 * 1024)).map(|i| (i % 251) as u8).collect();
    assert_eq!(plaintext.len(), 1024 * 1024, "plaintext fixture must be exactly 1 MB");

    let start = Instant::now();
    let env = encrypt_event(&plaintext, &recipient).expect("encrypt 1 MB must succeed");
    let recovered = decrypt_event(&env, &identity).expect("decrypt 1 MB must succeed");
    let elapsed = start.elapsed();

    assert_eq!(recovered.len(), plaintext.len(), "1 MB round-trip size mismatch");
    assert_eq!(recovered, plaintext, "1 MB round-trip content mismatch");
    assert!(
        elapsed <= BUDGET,
        "V6.AGE.3 age round-trip(1 × 1 MB) took {:?}, budget {:?}",
        elapsed,
        BUDGET
    );
    println!("V6.AGE.3 age round-trip(1 × 1 MB) = {:?} (budget {:?})", elapsed, BUDGET);
}
