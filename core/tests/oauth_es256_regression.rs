//! T56c — regression guard for `jsonwebtoken` ES256 sign path.
//!
//! ## Why this test exists
//!
//! `core/src/oauth.rs::generate_apple_client_secret` (private fn) signs an
//! Apple `client_secret` JWT using `jsonwebtoken::Algorithm::ES256`.
//!
//! Starting with `jsonwebtoken` 10, signing backends are **feature-gated** —
//! the crate compiles with **no crypto backend** by default. With the
//! `default-features = false` and no `rust_crypto`, ES256 sign breaks.
//!
//! PR #114 (T56) bumped `jsonwebtoken` 9 → 10 and added
//! `features = ["rust_crypto"]` to keep ES256 working. PR #115 (T56b)
//! restored the feature after it was briefly dropped.
//!
//! ### What this guard catches (verified on 2026-05-16)
//!
//! With `jsonwebtoken = "10"` (default features only, `rust_crypto` removed),
//! the regression actually manifests as a **library compile error** —
//! `EncodingKey::from_ec_pem` is feature-gated and disappears from the API
//! surface, so `core/src/oauth.rs` fails to build. That alone protects
//! `main`. This test stays useful because:
//!
//! * It pins the **runtime** sign path (`jsonwebtoken::encode` with ES256),
//!   not just `from_ec_pem`. If anyone refactors `oauth.rs` to load the key
//!   through a different code path, the lib-level compile guard goes away
//!   but the sign call is still vulnerable — the test would then become the
//!   only thing catching the missing-backend panic.
//! * The failing-test message points the next maintainer at exactly the
//!   Cargo.toml line to restore, instead of leaving them to chase the
//!   panic backwards.
//!
//! ## What this test does
//!
//! 1. Loads a hard-coded valid PKCS8 EC P-256 PEM (a *test fixture*, not a
//!    real secret — generated locally with `openssl ecparam -name prime256v1
//!    -genkey -noout | openssl pkcs8 -topk8 -nocrypt`).
//! 2. Builds an `EncodingKey` and signs a small JWT with `Algorithm::ES256`,
//!    mirroring the exact call sequence in `generate_apple_client_secret`.
//! 3. Asserts no panic, the token has 3 dot-separated segments, and the
//!    header decodes to JSON containing `"ES256"`.
//!
//! If the `rust_crypto` feature is dropped, step 2 panics and this test
//! fails — exactly the regression we want to catch.

use base64::Engine;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::Serialize;

#[derive(Serialize)]
struct AppleClaims {
    iss: String,
    sub: String,
    aud: String,
    exp: u64,
    iat: u64,
}

/// Test fixture: a valid PKCS8 EC P-256 private key generated specifically
/// for this regression test. NOT a real secret — has never been used for
/// anything other than this in-process unit test. Safe to commit.
///
/// Generated with:
///   openssl ecparam -name prime256v1 -genkey -noout \
///     | openssl pkcs8 -topk8 -nocrypt -outform PEM
const TEST_EC_P256_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg0a/p3bs4v58v0+8n
88HpJMh4rVMReGFpysmOvnmLEWWhRANCAATetKy113wgaIeSEZ0dvUuduWy5TNls
8bpdqwAsvaLILxRh/Qc5mh7S9M4RlAZj0Uh1lS6EGhV4HUBp3meBaTAn
-----END PRIVATE KEY-----";

#[test]
fn jsonwebtoken_es256_sign_does_not_panic() {
    // Step 1: Parse the PEM. This works without any crypto backend (parsing
    // is pure DER decoding), so a `from_ec_pem` failure means the fixture
    // itself is malformed — not a regression in the feature flag.
    let key = EncodingKey::from_ec_pem(TEST_EC_P256_PEM)
        .expect("test fixture must be valid PKCS8 EC P-256 PEM");

    // Step 2: Build claims and header exactly the way
    // `generate_apple_client_secret` does.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let claims = AppleClaims {
        iss: "TEAM_ID_TEST".to_string(),
        sub: "com.test.bundle".to_string(),
        aud: "https://appleid.apple.com".to_string(),
        exp: now + 3600,
        iat: now,
    };

    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some("KEY_ID_TEST".to_string());

    // Step 3: Sign. This is the call that panics if `rust_crypto` (or any
    // other ES256 backend) is missing from jsonwebtoken's feature set.
    let token = encode(&header, &claims, &key).expect(
        "ES256 sign must not fail — if this panics with \
         'no crypto backend enabled', the jsonwebtoken `rust_crypto` \
         feature has been dropped from core/Cargo.toml. Restore it: \
         jsonwebtoken = { version = \"10\", features = [\"rust_crypto\"] }",
    );

    // Sanity: a JWT has three base64url segments separated by dots.
    assert_eq!(
        token.matches('.').count(),
        2,
        "JWT must have exactly 3 segments, got: {}",
        token
    );

    // Sanity: the header (first segment) decodes to JSON that names ES256.
    let header_b64 = token.split('.').next().unwrap();
    let header_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(header_b64)
        .expect("JWT header must be valid base64url");
    let header_str = String::from_utf8(header_bytes).expect("JWT header bytes must be UTF-8 JSON");
    assert!(
        header_str.contains("ES256"),
        "JWT header should declare alg=ES256, got: {}",
        header_str
    );
}
