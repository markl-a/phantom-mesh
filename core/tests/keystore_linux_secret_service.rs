// keystore_linux_secret_service.rs — live round-trip of the Linux Secret
// Service keystore arm (LIN-KS-1), exercised through the PUBLIC keystore API
// (`write_to_keystore` / `read_from_keystore` with
// `KeystoreBackend::LinuxSecretService`).
//
// This complements the in-module `libsecret_round_trip_linux_only` unit test by
// driving the same backend through the crate's public surface — the same path a
// real `phantom keys init` takes once it picks `LinuxSecretService`.
//
// `#[ignore]` because it needs a live D-Bus session bus + an unlocked Secret
// Service (gnome-keyring / kwallet / KeePassXC). On a headless box (CI, WSL) you
// stand one up yourself. Verified on z13/WSL 2026-06-16 with:
//
//   wsl: sudo apt-get install -y gnome-keyring libsecret-tools dbus-x11
//   cd core
//   dbus-run-session -- bash -c '
//     eval "$(printf "\n" | gnome-keyring-daemon --unlock --components=secrets,pkcs11)"
//     export GNOME_KEYRING_CONTROL DBUS_SESSION_BUS_ADDRESS
//     CARGO_TARGET_DIR=$HOME/pm-wsl-target \
//       cargo test --test keystore_linux_secret_service -- --ignored --nocapture
//   '
//
// On a host WITHOUT a session bus the backend returns `KeystoreUnavailable`
// (connect fails) — that fail-closed path is covered by the in-crate
// no-D-Bus unit tests, not here.

#![cfg(target_os = "linux")]

use phantom_mesh::identity_wire::{read_from_keystore, write_to_keystore, KeystoreBackend};

/// A keystore account string unique to this test process + invocation, so the
/// test never collides with a real `identity-master` record or a parallel run.
fn unique_account() -> String {
    format!(
        "phantom-it-ks-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    )
}

#[test]
#[ignore = "needs a live D-Bus Secret Service; run under dbus-run-session + gnome-keyring (see file header)"]
fn linux_secret_service_round_trip_via_public_api() {
    let backend = KeystoreBackend::LinuxSecretService;
    let account = unique_account();
    // A 32-byte master-seed-shaped payload (the real callers store a 32-byte seed).
    let secret: [u8; 32] = *b"phantom-it-secret-32-bytes-pad!!";

    // write -> read recovers the exact bytes from the live keyring.
    write_to_keystore(backend, &account, &secret)
        .expect("write_to_keystore(LinuxSecretService) must succeed on an unlocked Secret Service");
    let got = read_from_keystore(backend, &account)
        .expect("read_from_keystore must return the bytes just written");
    assert_eq!(got, secret, "round-trip must recover the exact secret bytes");

    // A never-written account reads back as MasterNotFound (fail-closed: the
    // backend reports absence, it does not invent a value).
    let missing = read_from_keystore(backend, &format!("{account}-absent"));
    assert!(
        matches!(missing, Err(phantom_mesh::identity_wire::KeyDerivationError::MasterNotFound)),
        "unknown account must be MasterNotFound, got {missing:?}"
    );
}
