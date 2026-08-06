//! Hermetic E2E for `desktop-active-app-capture` (capability ① "sense" — the
//! CONTINUOUS frontmost-app sampler). NO GUI, NO real `$HOME`, NO real keychain:
//! every test drives the PURE sampler state machine and/or a TEMP EventStore
//! with a TEMP EventKey, so it is deterministic on any CI runner (incl.
//! non-macOS, since nothing here calls `read_frontmost`).
//!
//! Covers the three spec invariants:
//!   1. `on_sample` sequence → ≥2 distinct app-focus records with correct durations.
//!   2. emitted events are age-ENCRYPTED on disk (NOT plaintext containing the
//!      bundle id; mirrors the E004 guard) AND decrypt back via the store/recall.
//!   3. no-key no-op: with NO EventKey available, the writer logs + writes NOTHING.

use spectyn_mesh::capture_focus_wire::{
    focus_event_tags, write_focus_event, ActiveAppFocus, ActiveAppSampler,
};
use spectyn_mesh::event_storage_wire::EventKind;
use spectyn_mesh::life_node::storage::EventStore;

/// 1. on_sample sequence: feed (t0,A),(t1,A),(t2,B),(t3,C) and assert it emits
///    focus records for A (duration t2−t0) then B (t3−t2) — ≥2 DISTINCT
///    app-focus records with correct durations.
#[test]
fn on_sample_sequence_emits_two_distinct_records_with_durations() {
    let mut s = ActiveAppSampler::new();
    let (t0, t1, t2, t3) = (1_000u64, 1_010, 1_055, 1_090);

    let mut emitted: Vec<ActiveAppFocus> = Vec::new();
    if let Some(f) = s.on_sample(t0, Some("com.A".into())) {
        emitted.push(f);
    }
    if let Some(f) = s.on_sample(t1, Some("com.A".into())) {
        emitted.push(f); // same app → no emit
    }
    if let Some(f) = s.on_sample(t2, Some("com.B".into())) {
        emitted.push(f); // switch A→B → emit A
    }
    if let Some(f) = s.on_sample(t3, Some("com.C".into())) {
        emitted.push(f); // switch B→C → emit B
    }

    // Two completed intervals so far (A then B); C is still in flight.
    assert_eq!(emitted.len(), 2, "expected 2 completed intervals, got {emitted:?}");

    assert_eq!(emitted[0].bundle_id, "com.A");
    assert_eq!(emitted[0].focus_secs, t2 - t0, "A held focus from t0 until the switch at t2");

    assert_eq!(emitted[1].bundle_id, "com.B");
    assert_eq!(emitted[1].focus_secs, t3 - t2, "B held focus from t2 until the switch at t3");

    // Distinct apps.
    assert_ne!(emitted[0].bundle_id, emitted[1].bundle_id);
}

/// 2. encryption round-trip: write emitted focus events into a TEMP EventStore
///    with a TEMP EventKey → assert on-disk bytes are age-encrypted (NOT
///    plaintext containing the bundle id; mirrors the E004 guard) AND reading
///    back decrypts and finds them.
#[test]
fn emitted_events_are_age_encrypted_on_disk_and_decrypt_back() {
    let tmp = tempfile::tempdir().unwrap();
    let spectyn_dir = tmp.path().join(".spectyn-mesh");
    let events_dir = spectyn_dir.join("events");
    let identity_path = spectyn_dir.join("identity.key");
    std::fs::create_dir_all(&events_dir).unwrap();
    // 64-byte identity.key → derives a real age EventKey (same path production uses).
    std::fs::write(&identity_path, [0x5Au8; 64]).unwrap();

    let bundle_a = "com.apple.Safari";
    let bundle_b = "com.googlecode.iterm2";
    let focus_a = ActiveAppFocus { bundle_id: bundle_a.into(), focus_secs: 55 };
    let focus_b = ActiveAppFocus { bundle_id: bundle_b.into(), focus_secs: 35 };

    let id_a = write_focus_event(&events_dir, &identity_path, "test-node", &focus_a)
        .expect("write A must succeed")
        .expect("write A must produce an event id (key present)");
    let id_b = write_focus_event(&events_dir, &identity_path, "test-node", &focus_b)
        .expect("write B must succeed")
        .expect("write B must produce an event id (key present)");
    assert_ne!(id_a, id_b);

    // ── E004 invariant: each meta.json on disk MUST be age-encrypted, must NOT
    //    parse as JSON, and the raw bytes must NOT leak the bundle id in plaintext.
    for id in [&id_a, &id_b] {
        let meta_path = events_dir.join(id).join("meta.json");
        let raw = std::fs::read(&meta_path).expect("read meta.json");
        assert!(
            raw.starts_with(b"age-encryption.org/v1\n"),
            "meta.json must be age-encrypted; first 32 bytes = {:?}",
            &raw[..raw.len().min(32)]
        );
        assert!(
            serde_json::from_slice::<serde_json::Value>(&raw).is_err(),
            "encrypted meta.json must NOT parse as JSON"
        );
        // The bundle id rides in tags (encrypted). It must not appear in the
        // ciphertext as plaintext bytes.
        let raw_str = String::from_utf8_lossy(&raw);
        assert!(
            !raw_str.contains(bundle_a) && !raw_str.contains(bundle_b),
            "bundle id must NOT appear in plaintext in the encrypted file"
        );
    }

    // ── decrypt round-trip: open the store with the SAME key and read meta back.
    let store = EventStore::with_identity_file(&events_dir, &identity_path);
    let meta_a = store.read_meta(&id_a).expect("decrypt A");
    let meta_b = store.read_meta(&id_b).expect("decrypt B");

    // Both bridge to EventKind::Focus.
    assert_eq!(meta_a.kind, EventKind::Focus);
    assert_eq!(meta_b.kind, EventKind::Focus);

    // Bundle id is recall-searchable: it lives in the decrypted tags (the
    // no-LLM recall haystack), per the N4 lesson.
    assert!(
        meta_a.tags.contains(&bundle_a.to_string()),
        "A's bundle id must be a recall-searchable tag, got {:?}",
        meta_a.tags
    );
    assert!(
        meta_b.tags.contains(&bundle_b.to_string()),
        "B's bundle id must be a recall-searchable tag, got {:?}",
        meta_b.tags
    );
    // Duration also survives as a tag.
    assert!(meta_a.tags.contains(&"focus_secs=55".to_string()));
    assert!(meta_b.tags.contains(&"focus_secs=35".to_string()));

    // Sanity: the tag builder and the persisted tags agree.
    assert_eq!(meta_a.tags, focus_event_tags(&focus_a));
}

/// 2b. REAL recall path (FINDING 3 / spec acceptance "visible via spectyn recall
///     --json"): write a focus event through the PRODUCTION `write_focus_event`
///     path (temp EventStore + temp identity.key), then query it through the
///     EXACT same function `spectyn recall` calls — `recall::search_events` — and
///     assert the event is RETURNED. `search_events` SKIPS any event whose
///     `analysis.json` can't be read (`let Ok(analysis) = … else continue`); a
///     focus event with no analysis sibling is therefore INVISIBLE to recall even
///     though the store decrypts it (the gap the encryption test above could not
///     catch, because it read the store directly). This test fails WITHOUT the
///     analysis-sibling fix in `write_focus_event` and passes WITH it.
#[test]
fn focus_event_visible_via_real_recall_by_bundle_id_and_kind() {
    use spectyn_mesh::life_node::key_derivation::event_key_for_write;
    use spectyn_mesh::life_node::recall::{search_events, RecallFilter, RecallMode};

    let tmp = tempfile::tempdir().unwrap();
    let spectyn_dir = tmp.path().join(".spectyn-mesh");
    let events_dir = spectyn_dir.join("events");
    let identity_path = spectyn_dir.join("identity.key");
    std::fs::create_dir_all(&events_dir).unwrap();
    // Same 64-byte identity.key the production read path derives its EventKey from.
    std::fs::write(&identity_path, [0x5Au8; 64]).unwrap();

    let bundle = "com.googlecode.iterm2";
    let focus = ActiveAppFocus { bundle_id: bundle.into(), focus_secs: 42 };

    // PRODUCTION write path.
    let id = write_focus_event(&events_dir, &identity_path, "test-node", &focus)
        .expect("write must succeed")
        .expect("write must produce an event id (key present)");

    // The SAME key `spectyn recall` resolves for reads.
    let key = event_key_for_write(&identity_path)
        .expect("identity.key must load")
        .expect("a present 64-byte identity.key must yield a key");

    // (a) recall BY BUNDLE ID + --kind focus → MUST find the event. This is the
    //     spec acceptance; it goes through `read_analysis` (the skip point), so it
    //     only passes when the analysis sibling exists.
    let by_bundle = search_events(
        &events_dir,
        Some(key.clone()),
        &RecallFilter { query: bundle, kind: Some("focus"), since: None, mode: RecallMode::Keyword },
        50,
    )
    .expect("search must not error");
    assert!(
        by_bundle.iter().any(|h| h.event_id == id),
        "focus event must be VISIBLE via `spectyn recall {bundle} --kind focus` \
         (got {} hit(s): {:?}) — fails without the analysis-sibling fix",
        by_bundle.len(),
        by_bundle
    );
    let hit = by_bundle.iter().find(|h| h.event_id == id).unwrap();
    assert_eq!(hit.kind, "focus", "recall reports the focus kind");
    assert!(
        hit.summary.contains(bundle),
        "deterministic summary carries the bundle id (no window title): {:?}",
        hit.summary
    );

    // (b) empty query under --kind focus → still found (lists all focus events).
    let by_kind = search_events(
        &events_dir,
        Some(key),
        &RecallFilter { query: "", kind: Some("focus"), since: None, mode: RecallMode::Keyword },
        50,
    )
    .expect("search must not error");
    assert!(
        by_kind.iter().any(|h| h.event_id == id),
        "focus event must be listed under an empty `--kind focus` recall \
         (got {} hit(s): {:?})",
        by_kind.len(),
        by_kind
    );
}

/// 1b. read-error resilience (FINDING 1): a transient `read_frontmost()` failure
///     must NOT be fed to the sampler as `None`. The production loop SKIPS the
///     tick on error, so the currently-tracked app and its accumulated duration
///     SURVIVE — no false flush, no fragmentation. Here we replay the loop's
///     decision (skip the error tick → don't call `on_sample`) and prove the
///     same app accumulates one continuous interval across the gap.
#[test]
fn read_error_tick_skipped_does_not_fragment_focus_interval() {
    let mut s = ActiveAppSampler::new();

    // t=1000: A becomes frontmost (read OK).
    assert!(s.on_sample(1_000, Some("com.editor".into())).is_none());

    // t=1060: read_frontmost() returns Err → the loop SKIPS this tick. We model
    // that exactly: we do NOT call on_sample. (Feeding None here would have
    // wrongly flushed A at 60s and reset state.)

    // t=1120: A read OK again. Still the tracked app, still since t=1000 →
    // emits nothing (no fragmentation introduced by the skipped error tick).
    assert!(
        s.on_sample(1_120, Some("com.editor".into())).is_none(),
        "same app after a skipped read-error tick must not emit"
    );

    // t=1180: switch to B → A's single interval spans the WHOLE 180s including
    // the read-error gap, not a fragmented 60s + 120s.
    let emitted = s
        .on_sample(1_180, Some("com.browser".into()))
        .expect("A's completed interval on switch");
    assert_eq!(emitted.bundle_id, "com.editor");
    assert_eq!(
        emitted.focus_secs, 180,
        "A accumulated continuously across the skipped read-error tick (1180-1000)"
    );
}

/// 1c. shutdown-flush behavior (FINDING 2): the production loop flushes the
///     in-flight interval when its `CancellationToken` is cancelled. That
///     cancellation is REACHABLE — `spectyn serve` cancels the retained token on
///     its Ctrl-C graceful-shutdown path. The flush itself is `on_sample(now,
///     None)`, which closes out the tracked app. We assert that contract at the
///     state-machine level: a final `None` (the shutdown flush) emits the
///     in-progress app's interval so a quit does not silently drop it.
#[test]
fn shutdown_flush_emits_in_flight_interval() {
    let mut s = ActiveAppSampler::new();
    // App C is frontmost from t=2000.
    assert!(s.on_sample(2_000, Some("com.inflight".into())).is_none());
    // Shutdown at t=2090: the loop's `shutdown.cancelled()` arm calls
    // `on_sample(now, None)` to flush. The in-flight interval IS emitted (90s),
    // not dropped — this is the path serve's Ctrl-C cancellation now reaches.
    let flushed = s
        .on_sample(2_090, None)
        .expect("shutdown flush emits the in-flight app's interval");
    assert_eq!(flushed.bundle_id, "com.inflight");
    assert_eq!(flushed.focus_secs, 90);
}

/// 1d. shutdown-flush COMPLETION (FINDING 2, runtime half): the previous fix
///     relied on a fixed `sleep(500ms)` and DISCARDED the sampler's
///     `JoinHandle`, so `main` could return mid-write. serve now AWAITS the
///     handle, which only works if `run_active_app_sampler` actually RETURNS
///     (its future completes) promptly after the token is cancelled. This test
///     drives the REAL async loop with a `CancellationToken`, cancels it, and
///     asserts the spawned task's handle resolves well inside serve's 5s
///     shutdown budget. Hermetic: a TEMP `$HOME` (so any flush write lands in a
///     temp dir, never the real keychain/identity) and NO GUI. A NON-macOS run
///     read-errors every tick (so nothing is written); on macOS the long
///     interval means a tick is never the exit — either way the cancel arm is
///     what completes the future, proving it returns promptly on cancel. (The
///     sampler no longer takes a `port`: it is armed only AFTER a confirmed bind,
///     so there is no HTTP readiness gate to wait on.)
#[tokio::test]
async fn sampler_future_completes_promptly_after_cancel() {
    use tokio_util::sync::CancellationToken;

    let tmp = tempfile::tempdir().unwrap();
    let token = CancellationToken::new();

    // Long interval so a tick can't be the exit; the only way this future
    // completes is via the cancel arm (the shutdown-flush path serve awaits).
    let handle = tokio::spawn(spectyn_mesh::capture_focus_wire::run_active_app_sampler(
        tmp.path().to_path_buf(),
        3_600,
        token.clone(),
    ));

    // Cancel and assert the JoinHandle resolves promptly (the awaited handle in
    // serve's shutdown path depends on exactly this). 5s mirrors serve's budget.
    token.cancel();
    let joined = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
    assert!(
        joined.is_ok(),
        "run_active_app_sampler must RETURN promptly after cancel() so serve's awaited JoinHandle resolves within the 5s shutdown budget"
    );
    joined
        .unwrap()
        .expect("sampler task should join cleanly (no panic) after cancellation");
}

/// 1e. BIND-BEFORE-ARM — `bind_http_listener` FAILS on a port already owned by
///     another service. This is the invariant that replaces the old HTTP
///     readiness gate: the capture serve path calls `bind_http_listener` FIRST,
///     and only arms the sampler if THIS process confirms it bound the port. When
///     the port is held by another listener, `bind_http_listener` must return
///     `Err` (so the `?` returns before the sampler is ever spawned → zero events
///     for a never-served daemon). A bare HTTP probe could NOT distinguish this
///     case — the squatting service answers GET `/` while our bind fails.
///
///     We bind a throwaway `std::net::TcpListener` on an ephemeral port, learn
///     its address, then assert `bind_http_listener("127.0.0.1", that_port)` is
///     `Err`. NOTE: with SO_REUSEADDR + a 15s retry, the busy-port case can take
///     up to ~15s before it gives up — that's the intended "give up clearly"
///     behaviour, so this single assertion is allowed to be the slow one. The
///     companion `bind_http_listener_succeeds_on_free_port` covers the fast happy
///     path (free ephemeral port → Ok, listener is bound).
#[tokio::test]
async fn bind_http_listener_errs_when_port_already_bound() {
    // Hold an ephemeral port with a throwaway std listener (kept alive for the
    // whole test so the port stays genuinely occupied).
    let squatter = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let busy_port = squatter.local_addr().unwrap().port();

    // THIS process cannot claim a port another listener already owns → Err.
    // (May take up to the 15s retry budget before giving up — intended.)
    let result = spectyn_mesh::bind_http_listener("127.0.0.1", busy_port).await;
    assert!(
        result.is_err(),
        "bind_http_listener must FAIL on a port owned by another service \
         (this is the case where the caller must NEVER arm the sampler)"
    );
    drop(squatter);
}

/// 1f. BIND happy path — `bind_http_listener` SUCCEEDS on a free ephemeral port
///     and the returned listener is actually bound (its `local_addr` reports the
///     loopback host). Port 0 lets the OS pick a free port, so this is fast and
///     never flakes on a busy fixed port.
#[tokio::test]
async fn bind_http_listener_succeeds_on_free_port() {
    let listener = spectyn_mesh::bind_http_listener("127.0.0.1", 0)
        .await
        .expect("bind on a free ephemeral port (port 0) must succeed");
    let addr = listener.local_addr().expect("bound listener has a local_addr");
    assert!(addr.ip().is_loopback(), "bound to loopback host, got {addr}");
    assert_ne!(addr.port(), 0, "OS assigned a concrete ephemeral port, got {addr}");
}

/// 3. no-op guard: with NO EventKey available (no identity.key on disk), the
///    writer logs + writes NOTHING — no event dir, no plaintext file.
#[test]
fn no_event_key_is_a_logged_no_op_no_plaintext() {
    let tmp = tempfile::tempdir().unwrap();
    let spectyn_dir = tmp.path().join(".spectyn-mesh");
    let events_dir = spectyn_dir.join("events");
    let identity_path = spectyn_dir.join("identity.key"); // deliberately absent
    std::fs::create_dir_all(&events_dir).unwrap();
    assert!(!identity_path.exists(), "precondition: no identity.key");

    let focus = ActiveAppFocus { bundle_id: "com.apple.Terminal".into(), focus_secs: 99 };
    let result = write_focus_event(&events_dir, &identity_path, "test-node", &focus)
        .expect("no-key path must NOT error — it is a graceful no-op");

    // No event id returned → nothing was written.
    assert!(result.is_none(), "no key → no event should be written, got {result:?}");

    // The events dir must still be EMPTY: no plaintext focus event leaked.
    let entries: Vec<_> = std::fs::read_dir(&events_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        entries.is_empty(),
        "no-key no-op must write NOTHING; found {} entry(ies) in events dir",
        entries.len()
    );
}

/// 3b. no-op guard, corrupt-key variant: identity.key PRESENT but too short to
///     derive a key → also a no-op (refuse to downgrade to plaintext), no write.
#[test]
fn corrupt_event_key_is_a_logged_no_op_no_plaintext() {
    let tmp = tempfile::tempdir().unwrap();
    let spectyn_dir = tmp.path().join(".spectyn-mesh");
    let events_dir = spectyn_dir.join("events");
    let identity_path = spectyn_dir.join("identity.key");
    std::fs::create_dir_all(&events_dir).unwrap();
    // Present but <16 bytes → derive fails; we must NOT fall back to plaintext.
    std::fs::write(&identity_path, [0x01u8; 5]).unwrap();

    let focus = ActiveAppFocus { bundle_id: "com.apple.Terminal".into(), focus_secs: 7 };
    let result = write_focus_event(&events_dir, &identity_path, "test-node", &focus)
        .expect("corrupt-key path must NOT error — graceful no-op");
    assert!(result.is_none(), "corrupt key → no event written");

    let entries: Vec<_> = std::fs::read_dir(&events_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(entries.is_empty(), "corrupt-key no-op must write NOTHING");
}
