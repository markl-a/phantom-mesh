use std::sync::Mutex;

use phantom_mesh::encryption_wire::install_event_key_from_seed;
use phantom_mesh::vault::conversation_seal::{
    conversations_e2ee_enabled, open_line, seal_line,
};

static SEAL_TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn round_trip_recovers_exact_plaintext() {
    let _g = SEAL_TEST_LOCK.lock().unwrap();
    let _ = conversations_e2ee_enabled();

    install_event_key_from_seed(&[7u8; 32]).expect("install test EventKey");
    let line = r#"{"role":"user","content":"hello at rest","tool_calls":null}"#;

    let sealed = seal_line(line).expect("seal");

    assert!(
        !sealed.trim_start().starts_with('{'),
        "sealed line must not look like plaintext json"
    );
    let opened = open_line(&sealed).expect("open");
    assert_eq!(opened, line);
}

#[test]
fn plaintext_line_passes_through() {
    let _g = SEAL_TEST_LOCK.lock().unwrap();

    install_event_key_from_seed(&[7u8; 32]).expect("install test EventKey");
    let pt = r#"{"role":"assistant","content":"legacy","tool_calls":null}"#;

    assert_eq!(open_line(pt).unwrap(), pt);
}

#[test]
fn wrong_key_fails_closed() {
    let _g = SEAL_TEST_LOCK.lock().unwrap();

    install_event_key_from_seed(&[1u8; 32]).expect("install first test EventKey");
    let sealed = seal_line(r#"{"role":"user","content":"secret","tool_calls":null}"#).unwrap();

    install_event_key_from_seed(&[2u8; 32]).expect("install second test EventKey");
    match open_line(&sealed) {
        Err(_) => {}
        Ok(s) => panic!("must fail closed, but recovered: {s}"),
    }
}

// No dedicated `SealError::NoKey` assertion here: in integration tests the
// library is compiled without `cfg(test)`, so `lookup_or_derive_event_key` may
// derive from a real on-disk `~/.phantom-mesh/identity.key` after
// `clear_event_key_cache()`. Tests above cover round-trip, legacy plaintext
// passthrough, and wrong-key fail-closed behavior without depending on HOME.
