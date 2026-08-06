//! P0-8 integration tests — at-rest encryption closure for the skill FTS5
//! owned-memory store (`hermes_memory` / `hermes_memory_fts`).
//!
//! These run against the public `spectyn_mesh::skillbank::memory::SkillMemory`
//! API plus raw on-disk inspection, with `SPECTYN_ENCRYPT_MEMORY=1` and a
//! per-process `EventKey` installed directly.
//!
//! Process-global state (the EventKey cache + the env flag) means these MUST run
//! single-threaded — the harness is invoked with `-- --test-threads=1`, and an
//! in-file serial lock is belt-and-suspenders. Each test also sets `SPECTYN_HOME`
//! to a temp dir with NO `identity.key` so the lib's derive-on-miss path cannot
//! pick up the operator's real key (the integration build does not get the
//! `#[cfg(test)]` "never read identity.key" guard the lib's own tests have).
//!
//! `#![cfg(feature = ...)]` makes this file compile-empty without the memory
//! feature, so `cargo test --no-default-features` skips it cleanly.

#![cfg(feature = "experimental-memory")]
// These tests deliberately hold a std::sync::Mutex across `.await` to serialize
// the process-global EventKey cache + SPECTYN_ENCRYPT_MEMORY env for the whole
// async body (an async-aware mutex would release between awaits and defeat the
// serialization). Suppress the (correct-in-general) lint for this test-only use.
#![allow(clippy::await_holding_lock)]

use spectyn_mesh::encryption_wire::{clear_event_key_cache, install_event_key_from_seed};
use spectyn_mesh::skillbank::memory::{SkillMemory, NewMemory};

/// Serialize all key/env-touching tests on one process-global mutex.
fn serial_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Point the data-root at an isolated temp dir (with no `identity.key`) so the
/// lib's derive-on-miss path can never load the operator's real key.
fn isolate_home(td: &std::path::Path) {
    std::env::set_var("SPECTYN_HOME", td);
    std::env::set_var("SPECTYN_ENCRYPT_MEMORY", "1");
}

/// Reset the process-global state a test mutated.
fn teardown() {
    std::env::remove_var("SPECTYN_ENCRYPT_MEMORY");
    std::env::remove_var("SPECTYN_HOME");
    clear_event_key_cache();
}

/// Scenario 1 — at-rest e2e: write encrypted, reopen (restart), read back, and
/// FTS recall over the sealed row still works via the de-PII'd index form.
#[tokio::test]
async fn at_rest_e2e_write_encrypted_read_back() {
    let _g = serial_lock();
    let td = tempfile::tempdir().unwrap();
    isolate_home(td.path());
    install_event_key_from_seed(&[1u8; 32]).unwrap();

    let db = td.path().join("hermes-runtime.db");
    let id = {
        let mem = SkillMemory::open_at(db.clone()).unwrap();
        mem.insert(NewMemory {
            kind: "skill",
            source: "auto-evolve",
            text: "prefer ripgrep over grep for code search",
            tags: "tools",
        })
        .await
        .unwrap()
    };

    // Reopen (simulates restart) → plaintext recovered.
    let mem2 = SkillMemory::open_at(db).unwrap();
    let row = mem2.get_by_id(id).await.unwrap().unwrap();
    assert_eq!(row.text, "prefer ripgrep over grep for code search");
    assert_eq!(row.source, "auto-evolve");

    // FTS recall over the sealed row still works via the index form.
    let hits = mem2.search("ripgrep", 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].text, "prefer ripgrep over grep for code search");

    teardown();
}

/// Scenario 2 — wrong-key fail-closed: insert under key A, reopen under key B,
/// assert every read path returns Err and never surfaces ciphertext as text.
#[tokio::test]
async fn wrong_key_fails_closed_on_every_read_path() {
    let _g = serial_lock();
    let td = tempfile::tempdir().unwrap();
    isolate_home(td.path());
    install_event_key_from_seed(&[0xA1u8; 32]).unwrap();

    let db = td.path().join("hermes-runtime.db");
    let id = {
        let mem = SkillMemory::open_at(db.clone()).unwrap();
        mem.insert(NewMemory {
            kind: "fact",
            source: "secret-src",
            text: "classified payload sentinel",
            tags: "verdict",
        })
        .await
        .unwrap()
    };

    // Swap to a different (wrong) key.
    clear_event_key_cache();
    install_event_key_from_seed(&[0xB2u8; 32]).unwrap();
    let mem2 = SkillMemory::open_at(db).unwrap();

    // get_by_id must Err.
    let r1 = mem2.get_by_id(id).await;
    assert!(r1.is_err(), "get_by_id must fail closed under wrong key");
    let m1 = format!("{:#}", r1.unwrap_err());
    assert!(!m1.contains("classified payload"), "plaintext in error: {m1}");

    // list_by_kind must Err.
    let r2 = mem2.list_by_kind("fact", 10, 0).await;
    assert!(r2.is_err(), "list_by_kind must fail closed under wrong key");

    // list_since must Err.
    let r3 = mem2.list_since("fact", 0, 10).await;
    assert!(r3.is_err(), "list_since must fail closed under wrong key");

    // search: the FTS index form is plaintext tokens (not sealed), so MATCH may
    // hit, but materializing the row decrypts text → must Err. The contract:
    // no ciphertext is ever returned as MemoryRow.text.
    let r4 = mem2.search("payload", 10).await;
    assert!(
        r4.is_err(),
        "search must fail closed when the matched row won't decrypt"
    );

    teardown();
}

/// Scenario 3 — secret/PII grep sweep (the flagship privacy invariant). After a
/// WAL checkpoint, NO file in the db dir (db / -wal / -shm) may contain the
/// plaintext text, the plaintext source, the derived EventKey bytes, or the
/// identity seed.
#[tokio::test]
async fn no_plaintext_or_key_leaks_into_memdb_file() {
    let _g = serial_lock();
    let td = tempfile::tempdir().unwrap();
    isolate_home(td.path());
    let seed = [0xABu8; 32];
    let key = install_event_key_from_seed(&seed).unwrap();
    let key_bytes = key.as_bytes().to_vec();

    let db = td.path().join("hermes-runtime.db");
    let needle = "PRIVATE-NEEDLE-午餐-秘密-payload";
    {
        let mem = SkillMemory::open_at(db.clone()).unwrap();
        mem.insert(NewMemory {
            kind: "fact",
            source: "SECRET-SOURCE-NEEDLE",
            text: needle,
            tags: "x",
        })
        .await
        .unwrap();
    } // close → flush

    // Force a checkpoint so any WAL contents fold into the main db file too,
    // then close that connection as well.
    {
        let c = rusqlite::Connection::open(&db).unwrap();
        let _ = c.pragma_update(None, "wal_checkpoint", "TRUNCATE");
    }

    // Read EVERY file in the db dir (db, -wal, -shm, and the identity dir).
    let mut blobs = Vec::new();
    for e in std::fs::read_dir(td.path()).unwrap().flatten() {
        if e.path().is_file() {
            if let Ok(b) = std::fs::read(e.path()) {
                blobs.push(b);
            }
        }
    }
    assert!(!blobs.is_empty(), "expected at least the db file on disk");

    let has = |hay: &[u8], n: &[u8]| !n.is_empty() && hay.windows(n.len()).any(|w| w == n);
    for b in &blobs {
        assert!(
            !has(b, needle.as_bytes()),
            "plaintext text leaked into a db-dir file"
        );
        assert!(
            !has(b, b"SECRET-SOURCE-NEEDLE"),
            "plaintext source leaked into a db-dir file"
        );
        assert!(
            !has(b, &key_bytes),
            "derived EventKey leaked into a db-dir file"
        );
        assert!(!has(b, &seed), "identity seed leaked into a db-dir file");
    }

    teardown();
}

/// Scenario 4a — row delete purges the FTS index (the AFTER DELETE trigger
/// survives the 0010 trigger retirement).
#[tokio::test]
async fn delete_purges_index_and_row() {
    let _g = serial_lock();
    let td = tempfile::tempdir().unwrap();
    isolate_home(td.path());
    install_event_key_from_seed(&[0xC3u8; 32]).unwrap();

    let mem = SkillMemory::open_at(td.path().join("hermes-runtime.db")).unwrap();
    let id = mem
        .insert(NewMemory {
            kind: "fact",
            source: "s",
            text: "disposable widget memo",
            tags: "",
        })
        .await
        .unwrap();
    assert_eq!(mem.search("widget", 10).await.unwrap().len(), 1);

    mem.delete_by_id(id).await.unwrap();
    assert!(mem.get_by_id(id).await.unwrap().is_none(), "row must be gone");
    assert_eq!(
        mem.search("widget", 10).await.unwrap().len(),
        0,
        "delete must purge the FTS index"
    );

    teardown();
}

/// Scenario 4b — key-cache wipe (the kill-switch) immediately renders sealed
/// rows unreadable, even before the file is shredded. Mirrors the
/// `spectyn data delete --all` path's `clear_event_key_cache()`.
#[tokio::test]
async fn key_cache_wipe_makes_rows_unreadable() {
    let _g = serial_lock();
    let td = tempfile::tempdir().unwrap();
    isolate_home(td.path());
    install_event_key_from_seed(&[0xD4u8; 32]).unwrap();

    let db = td.path().join("hermes-runtime.db");
    let id = {
        let mem = SkillMemory::open_at(db.clone()).unwrap();
        mem.insert(NewMemory {
            kind: "fact",
            source: "s",
            text: "memory only readable while the key is loaded",
            tags: "",
        })
        .await
        .unwrap()
    };

    // Wipe the key cache (kill-switch). With SPECTYN_HOME pointing at a temp dir
    // that has NO identity.key, the derive-on-miss path also yields nothing, so
    // the sealed row can no longer be opened.
    clear_event_key_cache();

    let mem2 = SkillMemory::open_at(db).unwrap();
    let r = mem2.get_by_id(id).await;
    assert!(
        r.is_err(),
        "after key wipe, a sealed row must fail closed (kill-switch)"
    );

    teardown();
}
