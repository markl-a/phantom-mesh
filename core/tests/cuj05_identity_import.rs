// CUJ-05 · reinstall identity import — integration test.
//
// 對應 [`docs/cuj/05-export-and-uninstall.md`] Happy path C (re-install
// with identity recovery). Verifies `phantom identity import --from PATH`:
//   1. Validates the source is exactly 64 bytes (per-device IKM length).
//      Wrong-length sources fail-loud rather than producing an unreadable
//      EventStore.
//   2. Writes ~/.phantom-mesh/identity.key with mode 0600 (Unix).
//   3. Refuses to clobber an existing identity.key without --force.
//   4. With --force, backs up the existing key as identity.key.bak-<ts>
//      so the operator can recover from "imported the wrong file".
//   5. Returns a stable fingerprint (sha256[0..8]) so the user can match
//      against the value printed at `phantom backup` time.
//
// VERIFIES (CUJ-05 Happy path C):
//   - MAC-CUJ05-REI-001 from docs/test-cases/mac.md v2

use phantom_mesh::identity::{fingerprint_identity, import_root_identity_key_in};
use std::fs;
use tempfile::TempDir;

/// 64-byte sample IKM. Distinct byte pattern so a length mistake or a
/// silent zero-fill bug shows up obviously in the assertion failure.
const SAMPLE_64: [u8; 64] = [
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe,
    0x21, 0x43, 0x65, 0x87, 0xa9, 0xcb, 0xed, 0x0f, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00,
    0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10, 0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 0x01,
];

#[test]
fn cuj05_identity_import_happy_path_writes_key_and_returns_fingerprint() {
    let home = TempDir::new().expect("tempdir");
    let dir = home.path().join(".phantom-mesh");
    fs::create_dir_all(&dir).expect("mkdir .phantom-mesh");

    let fp = import_root_identity_key_in(&dir, &SAMPLE_64, false)
        .expect("first import on a fresh tempdir should succeed");
    assert_eq!(
        fp,
        fingerprint_identity(&SAMPLE_64),
        "returned fingerprint should match computed fingerprint of the input bytes"
    );

    // identity.key is exactly the input bytes — roundtrip is byte-identical.
    let written = fs::read(dir.join("identity.key")).expect("read back identity.key");
    assert_eq!(
        written.as_slice(),
        &SAMPLE_64[..],
        "stored bytes should match input exactly"
    );

    // Mode 0600 on Unix — group/other must be 0.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dir.join("identity.key"))
            .expect("stat identity.key")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "identity.key should be mode 0600 (got {:o})",
            mode & 0o777
        );
    }
}

#[test]
fn cuj05_identity_import_wrong_length_errors() {
    let home = TempDir::new().expect("tempdir");
    let dir = home.path().join(".phantom-mesh");
    fs::create_dir_all(&dir).expect("mkdir");

    let too_short = [0u8; 32];
    let err = import_root_identity_key_in(&dir, &too_short, false)
        .expect_err("32-byte input must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("32") && msg.contains("64"),
        "error should mention both actual and expected lengths; got: {}",
        msg
    );
    assert!(
        !dir.join("identity.key").exists(),
        "no file should be written on length validation failure"
    );

    let too_long = vec![0u8; 128];
    let err = import_root_identity_key_in(&dir, &too_long, false)
        .expect_err("128-byte input must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("128") && msg.contains("64"),
        "error should mention both actual and expected lengths; got: {}",
        msg
    );
}

#[test]
fn cuj05_identity_import_refuses_clobber_without_force() {
    let home = TempDir::new().expect("tempdir");
    let dir = home.path().join(".phantom-mesh");
    fs::create_dir_all(&dir).expect("mkdir");

    // Pre-existing key (e.g. a fresh `phantom habit ...` already created one).
    let existing = [0xAAu8; 64];
    fs::write(dir.join("identity.key"), existing).expect("write existing");

    let err = import_root_identity_key_in(&dir, &SAMPLE_64, /* force */ false)
        .expect_err("import without --force must refuse to clobber");
    let msg = err.to_string();
    assert!(
        msg.contains("force") || msg.contains("exists") || msg.contains("--force"),
        "error should mention --force flag; got: {}",
        msg
    );

    // Existing key is untouched.
    let after = fs::read(dir.join("identity.key")).expect("read after");
    assert_eq!(
        after.as_slice(),
        &existing[..],
        "existing identity.key must not be modified on non-force refuse"
    );
}

#[test]
fn cuj05_identity_import_force_backs_up_old_then_writes_new() {
    let home = TempDir::new().expect("tempdir");
    let dir = home.path().join(".phantom-mesh");
    fs::create_dir_all(&dir).expect("mkdir");

    let existing = [0xAAu8; 64];
    fs::write(dir.join("identity.key"), existing).expect("write existing");

    let fp = import_root_identity_key_in(&dir, &SAMPLE_64, /* force */ true)
        .expect("--force import should succeed");
    assert_eq!(fp, fingerprint_identity(&SAMPLE_64));

    // New key is in place.
    let after = fs::read(dir.join("identity.key")).expect("read new");
    assert_eq!(after.as_slice(), &SAMPLE_64[..]);

    // The existing key was renamed to identity.key.bak-<ts>; find it and
    // assert it still has the original bytes (so a wrong-file import can
    // be undone).
    let entries: Vec<_> = fs::read_dir(&dir)
        .expect("readdir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("identity.key.bak-"))
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "expected exactly one identity.key.bak-<ts>; got: {:?}",
        entries
    );
    let bak_bytes = fs::read(dir.join(&entries[0])).expect("read bak");
    assert_eq!(
        bak_bytes.as_slice(),
        &existing[..],
        "backup file should preserve the original bytes verbatim"
    );
}

#[test]
fn cuj05_identity_import_fingerprint_is_stable_and_short() {
    let fp1 = fingerprint_identity(&SAMPLE_64);
    let fp2 = fingerprint_identity(&SAMPLE_64);
    assert_eq!(fp1, fp2, "fingerprint should be deterministic");
    assert_eq!(
        fp1.len(),
        16,
        "fingerprint should be 16 hex chars (sha256[0..8] as lower hex)"
    );
    assert!(
        fp1.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
        "fingerprint should be lowercase hex only; got: {}",
        fp1
    );

    // Different bytes → different fingerprint.
    let mut other = SAMPLE_64;
    other[0] ^= 0xff;
    assert_ne!(
        fingerprint_identity(&other),
        fp1,
        "fingerprint must distinguish different keys"
    );
}
