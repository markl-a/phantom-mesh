//! Android identity keystore E2E checks.
//!
//! These tests are meaningful only inside the Android app process, where
//! `identity_wire` can reach the Kotlin `ai.phantommesh.app.IdentityKeystore`
//! wrapper over JNI. The wrapper contract is:
//! write(account, base64) -> EncryptedSharedPreferences.putString(account, base64)
//! read(account) -> stored base64 string or null
//! delete(account) -> remove(account), idempotent

#[cfg(target_os = "android")]
use phantom_mesh::identity_wire::{
    derive_subkey, fingerprint_short, read_from_keystore, write_to_keystore, KeyDerivationError,
    KeyPurpose, KeystoreBackend,
};

#[cfg(not(target_os = "android"))]
use phantom_mesh::identity_wire::KeystoreBackend;

#[cfg(not(target_os = "android"))]
#[test]
fn android_keystore_e2e_is_android_only() {
    assert_eq!(
        KeystoreBackend::AndroidEncryptedSharedPreferences.name(),
        "android-encshpref"
    );
}

#[cfg(target_os = "android")]
fn fingerprint_for_seed(seed: &[u8; 32]) -> String {
    let verifying = ed25519_dalek::SigningKey::from_bytes(seed)
        .verifying_key()
        .to_bytes();
    fingerprint_short(&verifying)
}

#[cfg(target_os = "android")]
#[test]
fn android_encrypted_shared_preferences_round_trips_master_seed() {
    use phantom_mesh::identity_wire::delete_from_keystore;

    let account = format!(
        "phantom-test-android-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    );
    let mut secret = [0u8; 32];
    for (i, b) in secret.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(3).wrapping_add(7);
    }
    let fingerprint = fingerprint_for_seed(&secret);

    write_to_keystore(
        KeystoreBackend::AndroidEncryptedSharedPreferences,
        &account,
        &secret,
    )
    .expect("Android EncryptedSharedPreferences write must succeed");

    let got = read_from_keystore(KeystoreBackend::AndroidEncryptedSharedPreferences, &account)
        .expect("Android EncryptedSharedPreferences read must succeed");
    assert_eq!(got, secret.to_vec());

    delete_from_keystore(
        KeystoreBackend::AndroidEncryptedSharedPreferences,
        &account,
        &fingerprint,
    )
    .expect("Android EncryptedSharedPreferences delete must succeed");
    match read_from_keystore(KeystoreBackend::AndroidEncryptedSharedPreferences, &account) {
        Err(KeyDerivationError::MasterNotFound) => {}
        other => panic!("read after delete must be MasterNotFound, got {other:?}"),
    }
}

#[cfg(target_os = "android")]
#[test]
#[ignore]
fn android_plaintext_identity_migrates_and_event_key_stays_stable() {
    use phantom_mesh::identity_wire::{delete_from_keystore, logout_clear_keystore};

    eprintln!(
        "[isolation] this test temporarily replaces account=\"identity-master\"; \
         run only on a dev Android device after backing up identity."
    );

    let backend = KeystoreBackend::AndroidEncryptedSharedPreferences;
    let account = "identity-master";
    let original_android = read_from_keystore(backend, account).ok();
    let original_file = read_from_keystore(KeystoreBackend::FileChmod0600, account).ok();
    let seed = [0x42u8; 32];
    let seed_fingerprint = fingerprint_for_seed(&seed);

    let result = (|| -> Result<(), KeyDerivationError> {
        logout_clear_keystore()?;
        let _ = delete_from_keystore(
            KeystoreBackend::FileChmod0600,
            account,
            &seed_fingerprint,
        );
        write_to_keystore(KeystoreBackend::FileChmod0600, account, &seed)?;

        let from_migration = derive_subkey(KeyPurpose::EventEncrypt)?;
        let after_migration = derive_subkey(KeyPurpose::EventEncrypt)?;
        assert_eq!(
            from_migration, after_migration,
            "EventKey must not change after plaintext seed migrates to Android keystore"
        );
        assert_eq!(read_from_keystore(backend, account)?, seed.to_vec());
        match read_from_keystore(KeystoreBackend::FileChmod0600, account) {
            Err(KeyDerivationError::MasterNotFound) => {}
            other => panic!("plaintext seed must be removed after migration, got {other:?}"),
        }
        Ok(())
    })();

    let _ = logout_clear_keystore();
    let _ = delete_from_keystore(
        KeystoreBackend::FileChmod0600,
        account,
        &seed_fingerprint,
    );
    if let Some(seed) = original_android {
        let _ = write_to_keystore(backend, account, &seed);
    }
    if let Some(seed) = original_file {
        let _ = write_to_keystore(KeystoreBackend::FileChmod0600, account, &seed);
    }

    result.expect("Android plaintext migration must preserve the identity seed");
}
