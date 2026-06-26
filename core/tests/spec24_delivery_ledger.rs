//! SPEC-24 §7.4 + §8 — the coach delivery ledger comes alive.
//!
//! `coach_delivery_wire::deliver()` was complete but had **zero production
//! callers** and, worse, its `DeliveryReceipt`s evaporated: `deliver` step 4
//! defers ledger persistence to the caller, and nothing ever wrote a row. So
//! `dedup_check`'s `WHERE status = 'sent'` predicate could never match — the
//! 24-hour dedup window was dead code and every fan-out would re-send forever.
//!
//! These tests drive the new WRITE half (`persist_receipts`) + the single
//! caller-facing entry point (`deliver_and_persist`) and prove the ledger now
//! actually deduplicates:
//!   1. a persisted `Sent` row makes `dedup_check` suppress the re-send, while a
//!      `Failed` row leaves the channel retryable, and distinct reviews stay
//!      independent;
//!   2. a `Sent` row older than the 24-hour window does NOT suppress;
//!   3. end-to-end: `deliver_and_persist` against a REAL `.md.age` artifact
//!      sends the Markdown channel once (`Sent`) then `Suppressed` on the
//!      second same-day fan-out — the live round-trip a scheduler would use.
//!
//! Isolation: its own integration-test binary (separate process), so the
//! per-process `EventKey` cache + `PHANTOM_MESH_COACH_LEDGER_DIR` override are
//! owned solely by these tests. The env-mutating tests serialize behind one
//! lock (cargo runs a binary's tests on parallel threads).

use std::sync::Mutex;

use base64::Engine;
use phantom_mesh::coach_delivery_wire::{
    dedup_check, deliver_and_persist, persist_receipts, DeliveryChannel, DeliveryReceipt,
    DeliveryStatus,
};
use phantom_mesh::encryption_wire::{
    derive_recipient_from_identity, encrypt_event, event_key_to_age_identity,
    install_event_key_from_seed,
};

/// Serialise tests that mutate the process-global `PHANTOM_MESH_COACH_LEDGER_DIR`
/// (and the EventKey cache) so a parallel test can't clobber another's ledger dir.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Point the coach delivery ledger at a fresh tempdir for the closure's lifetime,
/// restoring any prior override afterwards. Holds `ENV_LOCK` so the global env
/// var mutation is serialised across tests in this binary.
fn with_temp_ledger<T>(f: impl FnOnce(&std::path::Path) -> T) -> T {
    let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let saved = std::env::var("PHANTOM_MESH_COACH_LEDGER_DIR").ok();
    std::env::set_var("PHANTOM_MESH_COACH_LEDGER_DIR", dir.path());
    let out = f(dir.path());
    match saved {
        Some(v) => std::env::set_var("PHANTOM_MESH_COACH_LEDGER_DIR", v),
        None => std::env::remove_var("PHANTOM_MESH_COACH_LEDGER_DIR"),
    }
    out
}

#[test]
fn persist_receipts_brings_dedup_ledger_to_life() {
    with_temp_ledger(|_dir| {
        let review = "rev-aaa";
        // Empty ledger → nothing to suppress.
        assert!(
            !dedup_check(review, DeliveryChannel::Telegram).unwrap(),
            "empty ledger must not suppress"
        );

        // Persist a Sent receipt (timestamped now → inside the 24h window).
        let n = persist_receipts(&[DeliveryReceipt {
            review_id: review.into(),
            channel: DeliveryChannel::Telegram,
            attempted_at_ms: now_ms(),
            status: DeliveryStatus::Sent,
            error_message: None,
        }])
        .unwrap();
        assert_eq!(n, 1, "one row written");

        // The reader now sees the Sent row → suppress the re-send.
        assert!(
            dedup_check(review, DeliveryChannel::Telegram).unwrap(),
            "a persisted Sent row within 24h → dedup_check suppresses"
        );

        // A Failed receipt on a DIFFERENT channel must NOT suppress (retry stays open).
        persist_receipts(&[DeliveryReceipt {
            review_id: review.into(),
            channel: DeliveryChannel::Email,
            attempted_at_ms: now_ms(),
            status: DeliveryStatus::Failed,
            error_message: Some("smtp 535 auth".into()),
        }])
        .unwrap();
        assert!(
            !dedup_check(review, DeliveryChannel::Email).unwrap(),
            "a Failed receipt must leave the channel retryable"
        );

        // A different review is independent of this one's ledger rows.
        assert!(
            !dedup_check("rev-other", DeliveryChannel::Telegram).unwrap(),
            "dedup is scoped per review_id"
        );
    });
}

#[test]
fn sent_outside_24h_window_does_not_suppress() {
    with_temp_ledger(|_dir| {
        let review = "rev-old";
        // A Sent attempt 25h ago is outside the dedup window.
        let stale = now_ms().saturating_sub(25 * 60 * 60 * 1000);
        persist_receipts(&[DeliveryReceipt {
            review_id: review.into(),
            channel: DeliveryChannel::Telegram,
            attempted_at_ms: stale,
            status: DeliveryStatus::Sent,
            error_message: None,
        }])
        .unwrap();
        assert!(
            !dedup_check(review, DeliveryChannel::Telegram).unwrap(),
            "a Sent row older than 24h is outside the window → re-send allowed"
        );
    });
}

#[test]
fn persist_skips_reserved_push_receipt_without_aborting_batch() {
    with_temp_ledger(|_dir| {
        let review = "rev-mixed";
        let now = now_ms();
        // A batch containing a reserved Push receipt (deliver() emits a Failed
        // one) BETWEEN two real Sent receipts. Push must be skipped, not abort
        // the batch — both real rows must still land.
        let written = persist_receipts(&[
            DeliveryReceipt {
                review_id: review.into(),
                channel: DeliveryChannel::Markdown,
                attempted_at_ms: now,
                status: DeliveryStatus::Sent,
                error_message: None,
            },
            DeliveryReceipt {
                review_id: review.into(),
                channel: DeliveryChannel::Push,
                attempted_at_ms: now,
                status: DeliveryStatus::Failed,
                error_message: Some("delivery.config_missing: push".into()),
            },
            DeliveryReceipt {
                review_id: review.into(),
                channel: DeliveryChannel::Telegram,
                attempted_at_ms: now,
                status: DeliveryStatus::Sent,
                error_message: None,
            },
        ])
        .expect("a reserved Push receipt must not error the whole batch");
        assert_eq!(written, 2, "Push skipped; both real Sent rows written");
        assert!(
            dedup_check(review, DeliveryChannel::Markdown).unwrap(),
            "Markdown row before the Push survived"
        );
        assert!(
            dedup_check(review, DeliveryChannel::Telegram).unwrap(),
            "Telegram row after the Push survived"
        );
    });
}

#[test]
fn deliver_and_persist_markdown_round_trips_then_suppresses() {
    with_temp_ledger(|dir| {
        // Build a REAL .md.age the Markdown channel can read + decrypt + confirm
        // is age-ciphertext. encrypt_event addresses the per-process EventKey's
        // recipient; deliver()'s decrypt_raw_age_blob reads it back via the same
        // cached key. (ciphertext_b64 is base64 of the raw age blob; the on-disk
        // artifact must be the RAW bytes, so we base64-decode before writing.)
        let key = install_event_key_from_seed(&[9u8; 32]).expect("install EventKey");
        let ident = event_key_to_age_identity(&key).expect("age identity");
        let recipient = derive_recipient_from_identity(&ident);
        let envelope = encrypt_event(b"# Daily review\n\nbody text", &recipient).expect("encrypt");
        let raw_blob = base64::engine::general_purpose::STANDARD
            .decode(envelope.ciphertext_b64.as_bytes())
            .expect("decode raw age blob");
        let md_path = dir.join("2026-06-25.md.age");
        std::fs::write(&md_path, &raw_blob).unwrap();

        let review = "rev-e2e";

        // First fan-out: Markdown confirms the canonical ciphertext artifact →
        // Sent, and deliver_and_persist records the receipt in the ledger.
        let r1 = deliver_and_persist(review, &md_path, &[DeliveryChannel::Markdown])
            .expect("first delivery");
        assert_eq!(r1.len(), 1, "one receipt for one channel");
        assert_eq!(
            r1[0].status,
            DeliveryStatus::Sent,
            "first Markdown delivery should send: {:?}",
            r1[0]
        );
        assert!(
            dedup_check(review, DeliveryChannel::Markdown).unwrap(),
            "after persist the ledger knows this review×channel was sent"
        );

        // Second same-day fan-out: the now-live ledger suppresses the re-send
        // (the whole point — before persist_receipts existed this re-sent forever).
        let r2 = deliver_and_persist(review, &md_path, &[DeliveryChannel::Markdown])
            .expect("second delivery");
        assert_eq!(r2.len(), 1);
        assert_eq!(
            r2[0].status,
            DeliveryStatus::Suppressed,
            "second same-day Markdown delivery must be Suppressed by the ledger: {:?}",
            r2[0]
        );
    });
}
