//! CUJ-04 · MAC-CUJ04-DB-001 — SQLite corruption auto-recovery.
//!
//! The corruption-recovery path (claimed shipped as #143) lives in
//! `event_storage_wire::sqlite_open_pseudo` + `rotate_corrupt_sqlite`:
//!
//!   1. `Connection::open` the file. If it is not a valid sqlite at all
//!      (truncated / garbage bytes), the open fails and we rotate.
//!   2. Otherwise `PRAGMA integrity_check`; anything but the single row
//!      `"ok"` is treated as corruption.
//!   3. On corruption: rename the bad file to `<path>.corrupt-<unix-ts>`
//!      so the user can forensically recover, then reopen a FRESH empty db
//!      and (re)create the FTS5 schema. No panic; the open returns Ok.
//!
//! Before this test the feature had ZERO coverage. This file proves the
//! contract end-to-end through the PUBLIC entry point `index_fts5`, which is
//! what `spectyn habit ...` and the coach indexer call: it opens
//! `~/.spectyn-mesh/events.sqlite` (triggering the integrity gate) and then
//! writes an FTS5 row.
//!
//! Hermetic: `$HOME` is pointed at a unique temp dir, so `expand_tilde`
//! ("~/.spectyn-mesh/events.sqlite") resolves under our sandbox and we never
//! touch the developer's real db. The process-global HOME + EventKey mutation
//! is serialised behind `ENV_LOCK` so it cannot race other integration tests
//! sharing this binary.
//!
//! VERIFIES (MAC-CUJ04-DB-001):
//!   - planting garbage bytes at events.sqlite does NOT panic the open path
//!   - a sibling `events.sqlite.corrupt-*` file appears (the bad db is
//!     rotated aside, not deleted — forensic recovery stays possible)
//!   - a fresh, USABLE events.sqlite is created in its place (a subsequent
//!     `search_fts5` against the new index returns Ok, proving the FTS5
//!     schema was rebuilt on the clean db)

use spectyn_mesh::event_storage_wire::{index_fts5, search_fts5};
use std::sync::Mutex;

// Serialise tests that mutate the process-global HOME + EventKey so they
// don't race (cargo runs integration tests on threads within one binary).
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

fn unique_home() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "spectyn-cuj04-dbrec-{}-{}",
        std::process::id(),
        nanos()
    ))
}

/// Count sibling files matching `events.sqlite.corrupt-*` in the dir.
fn corrupt_siblings(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("events.sqlite.corrupt-") {
                out.push(e.path());
            }
        }
    }
    out
}

#[test]
fn cuj04_garbage_sqlite_is_rotated_aside_and_a_fresh_db_is_created() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    // ── Setup: isolated $HOME with a deliberately CORRUPT events.sqlite. ──
    let home = unique_home();
    let pm = home.join(".spectyn-mesh");
    std::fs::create_dir_all(&pm).expect("create .spectyn-mesh");

    let db_path = pm.join("events.sqlite");
    // Not a valid sqlite header at all — `Connection::open` may succeed (it is
    // lazy) but `PRAGMA integrity_check` / first statement will report the
    // damage. Either way the recovery branch must fire.
    std::fs::write(&db_path, b"\x00\xde\xad\xbe\xefnot a sqlite file at all \xff\xff garbage")
        .expect("plant garbage events.sqlite");

    std::env::set_var("HOME", &home);
    // EventKey isn't strictly required by index_fts5 (it only opens sqlite +
    // inserts a plaintext summary), but install one so the harness matches the
    // other cuj04 tests and any future key-touching code path stays happy.
    let _ = spectyn_mesh::encryption_wire::install_event_key_from_seed(&[9u8; 32]);

    // Sanity: precondition — no .corrupt-* siblings exist yet.
    assert!(
        corrupt_siblings(&pm).is_empty(),
        "precondition: there should be no .corrupt-* file before recovery"
    );

    // ── Act: drive the PUBLIC open path. This must NOT panic. ──
    // index_fts5 -> sqlite_open_pseudo("~/.spectyn-mesh/events.sqlite")
    //   -> Connection::open + PRAGMA integrity_check
    //   -> rotate_corrupt_sqlite(...) on corruption -> fresh db + FTS5 table.
    let receipt = index_fts5("recovery-probe-uuid", "smoke summary after corruption")
        .expect("index_fts5 must recover from a corrupt db and return Ok, not error/panic");
    assert_eq!(receipt.event_id, "recovery-probe-uuid");

    // ── Verify 1: the bad file was rotated to a sibling .corrupt-* ──
    let siblings = corrupt_siblings(&pm);
    assert_eq!(
        siblings.len(),
        1,
        "exactly one events.sqlite.corrupt-* sibling should appear after recovery; found {:?}",
        siblings
    );
    // And it must carry the ORIGINAL garbage bytes (rotated, not regenerated).
    let rotated_bytes = std::fs::read(&siblings[0]).expect("read rotated corrupt file");
    assert!(
        rotated_bytes.starts_with(b"\x00\xde\xad\xbe\xef"),
        "the rotated file must preserve the original (garbage) bytes for forensic recovery"
    );

    // ── Verify 2: a FRESH, usable events.sqlite now sits at the original path ──
    assert!(
        db_path.exists(),
        "a fresh events.sqlite must be created at the original path after rotation"
    );
    let fresh_bytes = std::fs::read(&db_path).expect("read fresh events.sqlite");
    // A real sqlite db starts with the 16-byte magic "SQLite format 3\0".
    assert!(
        fresh_bytes.starts_with(b"SQLite format 3\x00"),
        "the replacement db must be a real sqlite file (magic header present), got {} bytes",
        fresh_bytes.len()
    );

    // ── Verify 3: the rebuilt FTS5 index is actually queryable (no schema loss) ──
    let hits = search_fts5("smoke", 10)
        .expect("search against the freshly-rebuilt FTS5 index must succeed");
    assert!(
        hits.iter().any(|id| id == "recovery-probe-uuid"),
        "the row written through the recovered db must be searchable; got {:?}",
        hits
    );

    // Best-effort cleanup of the temp HOME.
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn cuj04_truncated_zero_byte_sqlite_also_recovers() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    let home = unique_home();
    let pm = home.join(".spectyn-mesh");
    std::fs::create_dir_all(&pm).expect("create .spectyn-mesh");

    // A 0-byte file: a classic "died mid-fsync / OS-killed VACUUM" artifact.
    let db_path = pm.join("events.sqlite");
    std::fs::write(&db_path, b"").expect("plant empty events.sqlite");

    std::env::set_var("HOME", &home);
    let _ = spectyn_mesh::encryption_wire::install_event_key_from_seed(&[9u8; 32]);

    // A 0-byte file is, per sqlite, a valid EMPTY database (integrity_check ->
    // "ok"), so this case may legitimately NOT rotate. The non-negotiable
    // contract is: the open path never panics and yields a usable db.
    let receipt = index_fts5("empty-probe-uuid", "zero byte recovery")
        .expect("index_fts5 must handle a 0-byte db without panicking");
    assert_eq!(receipt.event_id, "empty-probe-uuid");

    assert!(db_path.exists(), "events.sqlite must exist after open");
    let bytes = std::fs::read(&db_path).expect("read events.sqlite");
    assert!(
        bytes.starts_with(b"SQLite format 3\x00"),
        "a usable sqlite db must exist after handling the empty file"
    );

    let _ = std::fs::remove_dir_all(&home);
}

// MAC-CUJ04-DB-002: first-run auto-create. On a brand-new install there is no
// events.sqlite yet; the first capture (index_fts5) must transparently create a
// fresh, usable db — no "file not found", no panic, no rotation (nothing was
// corrupt). This is the happy-path twin of the recovery tests above.
#[test]
fn cuj04_first_run_creates_events_sqlite_when_absent() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    let home = unique_home();
    let pm = home.join(".spectyn-mesh");
    std::fs::create_dir_all(&pm).expect("create .spectyn-mesh");

    let db_path = pm.join("events.sqlite");
    // Precondition: the db genuinely does not exist on first run.
    assert!(
        !db_path.exists(),
        "precondition: events.sqlite must be absent before the first capture"
    );

    std::env::set_var("HOME", &home);
    let _ = spectyn_mesh::encryption_wire::install_event_key_from_seed(&[7u8; 32]);

    // First capture: must create the db (not error on a missing file).
    let receipt = index_fts5("first-run-uuid", "first capture on a clean install")
        .expect("index_fts5 must create events.sqlite on first run, not fail");
    assert_eq!(receipt.event_id, "first-run-uuid");

    // The db now exists and is a real sqlite file.
    assert!(
        db_path.exists(),
        "events.sqlite must be auto-created on first capture"
    );
    let bytes = std::fs::read(&db_path).expect("read events.sqlite");
    assert!(
        bytes.starts_with(b"SQLite format 3\x00"),
        "the auto-created db must be a real sqlite file"
    );

    // Nothing was corrupt, so NO .corrupt-* sibling should have been produced.
    assert!(
        corrupt_siblings(&pm).is_empty(),
        "first-run create must not rotate anything aside"
    );

    // And the freshly-created FTS5 index is queryable.
    let hits =
        search_fts5("capture", 10).expect("search against the auto-created index must succeed");
    assert!(
        hits.iter().any(|id| id == "first-run-uuid"),
        "the first-run row must be searchable; got {:?}",
        hits
    );

    let _ = std::fs::remove_dir_all(&home);
}
