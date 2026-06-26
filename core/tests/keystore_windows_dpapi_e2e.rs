#![cfg(target_os = "windows")]

use phantom_mesh::identity_wire::{
    build_init_outcome, delete_from_keystore, derive_subkey, fingerprint_short,
    read_from_keystore, write_to_keystore, KeyDerivationError, KeyPurpose, KeystoreBackend,
};

fn fingerprint_for_seed(seed: &[u8; 32]) -> String {
    let verifying = ed25519_dalek::SigningKey::from_bytes(seed)
        .verifying_key()
        .to_bytes();
    fingerprint_short(&verifying)
}

#[test]
fn windows_dpapi_credman_round_trip_throwaway_account() {
    let account = format!(
        "phantom-test-dpapi-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    );
    let secret = [0x42u8; 32];

    write_to_keystore(
        KeystoreBackend::WindowsCredentialManager,
        &account,
        &secret,
    )
    .expect("DPAPI/Credential Manager write must succeed");

    let got = read_from_keystore(KeystoreBackend::WindowsCredentialManager, &account)
        .expect("DPAPI/Credential Manager read must succeed");
    assert_eq!(got, secret);

    let cmdkey = std::process::Command::new("cmdkey")
        .arg("/list")
        .output()
        .expect("cmdkey /list must run on Windows");
    assert!(
        cmdkey.status.success(),
        "cmdkey /list failed with status {:?}",
        cmdkey.status.code()
    );
    let stdout = String::from_utf8_lossy(&cmdkey.stdout);
    assert!(
        stdout.contains(&account),
        "Credential Manager entry must be visible in cmdkey /list output"
    );

    delete_from_keystore(
        KeystoreBackend::WindowsCredentialManager,
        &account,
        &fingerprint_for_seed(&secret),
    )
    .expect("DPAPI/Credential Manager delete must succeed");

    match read_from_keystore(KeystoreBackend::WindowsCredentialManager, &account) {
        Err(KeyDerivationError::MasterNotFound) => {}
        other => panic!("read after delete must be MasterNotFound, got {other:?}"),
    }
}

#[test]
#[ignore]
fn windows_identity_bootstrap_uses_credman_and_preserves_event_key() {
    let _ = phantom_mesh::identity_wire::logout_clear_keystore();

    let first = build_init_outcome(false).expect("initial Windows identity bootstrap");
    assert!(first.created, "first bootstrap should create identity: {first:?}");
    assert_eq!(first.keystore_backend, "windows-credman");

    let event_key_first =
        derive_subkey(KeyPurpose::EventEncrypt).expect("event key derives after bootstrap");
    let stored = read_from_keystore(KeystoreBackend::WindowsCredentialManager, "identity-master")
        .expect("identity-master must be readable from Credential Manager");
    assert_eq!(stored.len(), 32, "master seed remains a 32-byte seed");

    let second = build_init_outcome(false).expect("second Windows identity bootstrap");
    assert!(!second.created, "second bootstrap must reuse existing identity");
    assert_eq!(second.fingerprint, first.fingerprint);
    assert_eq!(second.public_key_hex, first.public_key_hex);

    let event_key_second =
        derive_subkey(KeyPurpose::EventEncrypt).expect("event key derives after reload");
    assert_eq!(
        event_key_second, event_key_first,
        "read must derive the same EventEncrypt key from the stored master seed"
    );

    delete_from_keystore(
        KeystoreBackend::WindowsCredentialManager,
        "identity-master",
        &second.fingerprint,
    )
    .expect("cleanup identity-master after ignored live test");
}
