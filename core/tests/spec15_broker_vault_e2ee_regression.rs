// SPEC-15 — Broker Vault E2EE regression suite.
//
// 中文: SPEC-15 端到端加密（E2EE）回歸測試。守住兩條不可違反的鐵律：
//   1. broker（中介伺服器）永遠拿不到明文 — 上線結構只搬 sealed ciphertext +
//      HMAC（雜湊訊息驗證碼）+ 非機密 metadata。
//   2. seal → (broker 原樣中轉) → unseal 的 round-trip 只在本機 core 內完成。
//
// WHY THIS FILE EXISTS (drift it closes):
//   `core/src/broker_vault_wire.rs` shipped `seal_vault_value` with NO inverse,
//   so the whole module was dead code — nothing could complete a client-side
//   round trip and the live plaintext broker path (`GET /api/me/settings/raw`,
//   server `deriveUserKey`/`decryptForUser`, `ENV_VAULT_KEY`) was never retired.
//   This suite (a) exercises the seal/unseal pair on the live read+write path so
//   it stops being dead code, and (b) asserts as a *regression guard* that the
//   broker-facing wire types can never carry vault plaintext.
//
// MIGRATION TODO (out of this test's scope — server + client-caller agents):
//   - Retire `spectynmesh-io` `getSettingsRaw` / `deriveUserKey` /
//     `decryptForUser` / `ENV_VAULT_KEY`; replace with dumb `/vault/*` storage
//     that never decrypts (see docs/integration/2026-05-29-spec15-vault-verification.md).
//   - Migrate live plaintext callers: core/src/cli_config.rs and
//     app/src-tauri/src/commands/broker_login.rs onto seal/unseal here.
//   - These deletions are NOT done by this commit — the security swap is a
//     separate, human-reviewed step. This file only proves the E2EE primitive
//     is sound and locks the no-plaintext invariant.

use spectyn_mesh::broker_vault_wire::{
    compute_client_hmac, generate_vault_seal_key, seal_vault_value, unseal_vault_value,
    VaultGetResponse, VaultSetRequest,
};

const AGE_V1_MAGIC: &[u8] = b"age-encryption.org/v1\n";

/// The single most important property: a secret sealed on one device can be
/// recovered on the same (or a key-shared) device after a full broker
/// round-trip — and the value the broker held in between was opaque ciphertext.
///
/// This exercises the live seal + unseal path so neither helper is dead code.
#[test]
fn seal_then_unseal_round_trips_client_side() {
    let key = generate_vault_seal_key();
    // Use a placeholder, clearly-fake secret — never a real key (OSS-safe).
    let plaintext = b"sk-test-PLACEHOLDER-not-a-real-key-0000";

    // --- WRITE PATH (device A -> POST /vault/set) ---
    let value_sealed = seal_vault_value(plaintext, &key).expect("seal");

    // What the broker actually receives is base64url age v1 ciphertext, which
    // must NOT contain the cleartext anywhere.
    let on_wire = VaultSetRequest {
        service: "cerebras".to_string(),
        key: "default".to_string(),
        value_sealed: value_sealed.clone(),
        client_hmac_hex: compute_client_hmac(
            &key,
            "cerebras",
            "default",
            &value_sealed,
            1_700_000_000_000,
        ),
        ts_ms: 1_700_000_000_000,
    };
    let wire_json = serde_json::to_string(&on_wire).expect("serialize set req");
    assert!(
        !wire_json.contains("sk-test-PLACEHOLDER"),
        "VaultSetRequest must NOT carry plaintext on the wire: {wire_json}"
    );

    // --- READ PATH (broker echoes ciphertext verbatim -> GET /vault/get) ---
    // Simulate the broker storing and returning the sealed value untouched.
    let echoed = VaultGetResponse {
        service: "cerebras".to_string(),
        key: "default".to_string(),
        value_sealed: on_wire.value_sealed.clone(),
        ts_ms: on_wire.ts_ms,
        age_recipient_hint: None,
    };
    let recovered = unseal_vault_value(&echoed.value_sealed, &key).expect("unseal");
    assert_eq!(
        recovered,
        plaintext.to_vec(),
        "seal->unseal must round-trip the exact plaintext client-side"
    );
}

/// Decode of `value_sealed` MUST begin with the age v1 magic line — proving the
/// broker only ever sees age ciphertext, never anything it could read.
#[test]
fn sealed_payload_is_age_v1_ciphertext_not_plaintext() {
    use base64::Engine as _;
    let key = generate_vault_seal_key();
    let value_sealed = seal_vault_value(b"super-secret", &key).expect("seal");
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value_sealed)
        .expect("base64url no-pad");
    assert!(
        raw.starts_with(AGE_V1_MAGIC),
        "value_sealed must decode to age v1 ciphertext, got first bytes {:?}",
        &raw[..raw.len().min(AGE_V1_MAGIC.len())]
    );
    // Defense in depth: the literal cleartext must not survive into ciphertext.
    let needle = b"super-secret";
    assert!(
        !raw.windows(needle.len()).any(|w| w == needle),
        "cleartext must not appear inside the sealed ciphertext"
    );
}

/// A wrong key (a different device that never received the wrapped seal key)
/// must NOT be able to unseal — proves the broker, which holds no key at all,
/// is even more powerless than a wrong-keyed peer.
#[test]
fn unseal_with_wrong_key_fails() {
    let key_a = generate_vault_seal_key();
    let key_b = generate_vault_seal_key();
    let sealed = seal_vault_value(b"PLACEHOLDER-secret", &key_a).expect("seal");

    let r = unseal_vault_value(&sealed, &key_b);
    assert!(
        r.is_err(),
        "unseal with a different VaultSealKey must fail (no key => broker cannot decrypt)"
    );
}

/// Tampering with the stored ciphertext must be detectable. The broker stores
/// `client_hmac_hex` opaquely (it cannot verify — it has no key); the client
/// re-derives the HMAC over the downloaded payload to catch tampering. A flipped
/// ciphertext yields a different client-recomputed HMAC AND fails to decrypt.
#[test]
fn tampered_ciphertext_is_detected_client_side() {
    let key = generate_vault_seal_key();
    let ts_ms = 1_700_000_000_000u64;
    let sealed = seal_vault_value(b"PLACEHOLDER-secret", &key).expect("seal");
    let uploaded_hmac = compute_client_hmac(&key, "svc", "k", &sealed, ts_ms);

    // Broker (or a MITM) flips one byte of the stored base64url ciphertext.
    let mut tampered: Vec<u8> = sealed.clone().into_bytes();
    let last = tampered.len() - 1;
    tampered[last] = if tampered[last] == b'A' { b'B' } else { b'A' };
    let tampered = String::from_utf8(tampered).unwrap();

    // §8.C integrity step: client recomputes HMAC over what it downloaded and
    // compares to the stored `server_hmac_hex` (== the uploaded client HMAC).
    let recomputed = compute_client_hmac(&key, "svc", "k", &tampered, ts_ms);
    assert_ne!(
        uploaded_hmac, recomputed,
        "tampered ciphertext must change the client-recomputed HMAC"
    );

    // And the tampered payload must not silently decrypt to the original.
    match unseal_vault_value(&tampered, &key) {
        Err(_) => {}
        Ok(out) => assert_ne!(
            out,
            b"PLACEHOLDER-secret".to_vec(),
            "tampered ciphertext must not decrypt back to the original plaintext"
        ),
    }
}

/// REGRESSION GUARD: the broker-facing wire structs expose no field that could
/// carry vault plaintext. If a future refactor adds a `value_clear` /
/// `plaintext` / `decrypted` field to the request or response, this test breaks
/// loudly. (The E2EE secret path must never send/return cleartext — SPEC-15 §0.)
#[test]
fn broker_wire_structs_carry_no_plaintext_field() {
    let key = generate_vault_seal_key();
    let value_sealed = seal_vault_value(b"PLACEHOLDER-secret-value", &key).expect("seal");

    let set_req = VaultSetRequest {
        service: "anthropic".to_string(),
        key: "api_key".to_string(),
        value_sealed: value_sealed.clone(),
        client_hmac_hex: compute_client_hmac(&key, "anthropic", "api_key", &value_sealed, 1),
        ts_ms: 1,
    };
    let get_resp = VaultGetResponse {
        service: "anthropic".to_string(),
        key: "api_key".to_string(),
        value_sealed,
        ts_ms: 1,
        age_recipient_hint: None,
    };

    for json in [
        serde_json::to_string(&set_req).unwrap(),
        serde_json::to_string(&get_resp).unwrap(),
    ] {
        for forbidden in [
            "value_clear",
            "valueClear",
            "plaintext",
            "plainText",
            "decrypted",
            "clearText",
            "clear_text",
        ] {
            assert!(
                !json.contains(forbidden),
                "broker wire struct must not expose a plaintext field `{forbidden}`: {json}"
            );
        }
        // The literal secret value must never appear on the wire.
        assert!(
            !json.contains("PLACEHOLDER-secret-value"),
            "broker wire struct leaked the cleartext secret: {json}"
        );
    }
}
