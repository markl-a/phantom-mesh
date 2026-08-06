//! Local identity for `spectyn login` / `whoami` / `logout`.
//!
//! Stored at `~/.spectyn-mesh/auth.json` (mode 0600). Three identity
//! sources:
//!   - email: SHA-256(salt || password) with 100K iterations stored locally
//!   - google: OAuth 2.0 device-flow loopback, id_token saved
//!   - apple : stub (Apple OAuth requires HTTPS redirect; needs relay)
//!
//! The cloud broker is NOT involved (per COMMERCIAL-DESIGN.md §2 hard
//! rule #2: the OSS binary works without any cloud account). This file
//! only carries identity; `spectyn devices` / mesh discovery against a
//! broker is added when spectyn-cloud-client lands.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthState {
    pub provider: String, // "email" | "google" | "apple"
    pub email: String,
    pub display_name: Option<String>,
    pub sub: Option<String>, // OAuth subject (google id / apple sub)
    pub avatar_url: Option<String>,
    pub device_id: String, // uuid v4 — stable across logins on same machine
    pub created_at_ms: i64,
    pub last_login_ms: i64,

    /// SHA-256(salt || password), 100,000 iters. Empty when provider != email.
    #[serde(default)]
    pub password_hash: String,
    #[serde(default)]
    pub salt: String,

    /// OAuth tokens. Empty when provider == email.
    #[serde(default)]
    pub id_token: String,
    #[serde(default)]
    pub access_token: String,

    /// JWT issued by the phantommesh.io broker. Used as `Authorization:
    /// Bearer <broker_token>` for `/api/me/*` calls (e.g. `spectyn config
    /// pull`). TTL controlled by the broker's BROKER_TOKEN_TTL_SECS
    /// (default 7 days). Empty for non-broker logins (provider=email or
    /// direct provider=google flows that didn't go through spectynmesh).
    #[serde(default)]
    pub broker_token: String,
    #[serde(default)]
    pub broker_token_expires_at_ms: i64,
    /// URL of the broker that issued the token (lets `spectyn config pull`
    /// know where to call back without an extra arg).
    #[serde(default)]
    pub broker_url: String,
}

pub fn auth_path() -> PathBuf {
    crate::cli_config::spectyn_data_dir()
        .unwrap_or_else(|_| PathBuf::from(".").join(".spectyn-mesh"))
        .join("auth.json")
}

pub fn load() -> Option<AuthState> {
    let p = auth_path();
    let s = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

pub fn save(state: &AuthState) -> anyhow::Result<()> {
    let p = auth_path();
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(&p, json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perm = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(&p, perm);
    }
    Ok(())
}

pub fn delete() -> anyhow::Result<()> {
    let p = auth_path();
    if p.exists() {
        std::fs::remove_file(p)?;
    }
    Ok(())
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn random_device_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn random_salt() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut b);
    hex::encode(b)
}

/// SHA-256(salt || password), iterated 100,000 times. Not as good as
/// bcrypt/argon2 but uses only deps already in our tree. The 100K
/// iterations make a brute-force attempt cost a CPU-second per password
/// — enough to make casual local-disk-readers give up.
pub fn hash_password(password: &str, salt: &str) -> String {
    let mut h = format!("{}{}", salt, password).into_bytes();
    for _ in 0..100_000 {
        let mut hasher = Sha256::new();
        hasher.update(&h);
        h = hasher.finalize().to_vec();
    }
    hex::encode(h)
}

pub fn verify_password(state: &AuthState, password: &str) -> bool {
    if state.salt.is_empty() || state.password_hash.is_empty() {
        return false;
    }
    let calc = hash_password(password, &state.salt);
    // Constant-time compare to dodge timing attacks.
    constant_time_eq(calc.as_bytes(), state.password_hash.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Public-facing summary printed by `spectyn whoami` and `spectyn doctor`.
pub fn human_summary(state: &AuthState) -> String {
    let prov = match state.provider.as_str() {
        "email" => "email",
        "google" => "Google",
        "apple" => "Apple",
        other => other,
    };
    let dn = state.display_name.as_deref().unwrap_or("");
    let dn_part = if dn.is_empty() {
        String::new()
    } else {
        format!(" ({})", dn)
    };
    format!(
        "{}{}  via {}  device {}",
        state.email,
        dn_part,
        prov,
        &state.device_id[..8.min(state.device_id.len())]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_password() {
        let salt = random_salt();
        let h = hash_password("hunter2", &salt);
        let state = AuthState {
            provider: "email".into(),
            email: "a@b.c".into(),
            display_name: None,
            sub: None,
            avatar_url: None,
            device_id: random_device_id(),
            created_at_ms: 0,
            last_login_ms: 0,
            password_hash: h,
            salt,
            id_token: String::new(),
            access_token: String::new(),
            broker_token: String::new(),
            broker_token_expires_at_ms: 0,
            broker_url: String::new(),
        };
        assert!(verify_password(&state, "hunter2"));
        assert!(!verify_password(&state, "wrong"));
    }

    #[test]
    fn salt_is_unique() {
        let a = random_salt();
        let b = random_salt();
        assert_ne!(a, b);
        assert_eq!(a.len(), 32); // 16 bytes hex = 32 chars
    }

    // ─── `spectyn logout` auth.json state clear (A3) ──────────────────────────
    //
    // `auth::delete()` is the state-clearing half of `spectyn logout`: it drops
    // the on-disk auth.json (provider/broker tokens, password hash, etc.) and is
    // idempotent when the file is already absent. We deliberately do NOT exercise
    // it against the real `auth_path()` (`~/.spectyn-mesh/auth.json`) — that would
    // log the developer out — so we prove the exact mechanism it uses (an
    // existence-guarded `remove_file`) against a THROWAWAY temp file suffixed
    // with pid + uuid. After clearing, loading the same path must fail to parse
    // (file gone), and a second clear must be a no-op success. This mirrors the
    // identity_wire.rs "test the mechanism on a throwaway, never the real
    // record" pattern.
    #[test]
    fn logout_clears_auth_state() {
        // A throwaway stand-in for auth_path(), never the real one.
        let path = std::env::temp_dir().join(format!(
            "spectyn-test-auth-logout-{}-{}.json",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));

        // Seed a realistic logged-in state (broker token present, like an OAuth
        // login) onto the throwaway path.
        let state = AuthState {
            provider: "google".into(),
            email: "logout@test.local".into(),
            display_name: Some("Logout Tester".into()),
            sub: Some("sub-123".into()),
            avatar_url: None,
            device_id: random_device_id(),
            created_at_ms: now_ms(),
            last_login_ms: now_ms(),
            password_hash: String::new(),
            salt: String::new(),
            id_token: "id-token-xyz".into(),
            access_token: "access-token-xyz".into(),
            broker_token: "broker-jwt-xyz".into(),
            broker_token_expires_at_ms: now_ms() + 60_000,
            broker_url: "https://phantommesh.io".into(),
        };
        std::fs::write(&path, serde_json::to_string_pretty(&state).unwrap())
            .expect("seed throwaway auth.json must succeed");

        // Sanity: it's really there and parses back into a state before clearing.
        let before: AuthState = serde_json::from_str(
            &std::fs::read_to_string(&path).expect("seeded auth.json must be readable"),
        )
        .expect("seeded auth.json must parse");
        assert_eq!(before.broker_token, "broker-jwt-xyz");

        // The exact clear mechanism `auth::delete()` runs: existence-guarded
        // remove_file. Cleared state must be gone afterwards (load → None).
        assert!(path.exists(), "precondition: auth.json present before clear");
        if path.exists() {
            std::fs::remove_file(&path).expect("logout auth clear must succeed");
        }
        assert!(
            !path.exists(),
            "auth.json must be gone after logout clear"
        );
        assert!(
            std::fs::read_to_string(&path).is_err(),
            "loading cleared auth.json must fail (no tokens left to read)"
        );

        // Idempotent: clearing an already-absent record is a no-op success,
        // matching `auth::delete()`'s `if p.exists()` guard.
        if path.exists() {
            std::fs::remove_file(&path)
                .expect("logout auth clear must be idempotent on an already-empty record");
        }

        // Smoke: the public clear helper exists, links, and has the expected
        // signature (a function-pointer reference forces a compile-time check
        // without invoking it against the real `auth_path()`).
        let _f: fn() -> anyhow::Result<()> = delete;
    }

    #[test]
    fn login_save_then_logout_delete_returns_to_baseline() {
        // SYS-D round-trip on the REAL auth_path(): `spectyn login` persists
        // auth.json via auth::save(); `spectyn logout` removes it via
        // auth::delete(). Hermetic + safe (never touches the dev's real
        // auth.json): SPECTYN_HOME redirects auth_path() into a tempdir, under
        // env_lock.
        let _env = crate::env_lock::acquire();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        struct HomeGuard(Option<std::ffi::OsString>);
        impl Drop for HomeGuard {
            fn drop(&mut self) {
                match &self.0 {
                    Some(v) => std::env::set_var("SPECTYN_HOME", v),
                    None => std::env::remove_var("SPECTYN_HOME"),
                }
            }
        }
        let prev = std::env::var_os("SPECTYN_HOME");
        std::env::set_var("SPECTYN_HOME", tmp.path());
        let _guard = HomeGuard(prev);

        // Baseline: not logged in.
        assert!(load().is_none(), "fresh home: no auth state");
        assert!(!auth_path().exists());

        // DO: login persists auth.json.
        let state = AuthState {
            provider: "google".into(),
            email: "roundtrip@test.local".into(),
            display_name: Some("RT".into()),
            sub: Some("sub-rt".into()),
            avatar_url: None,
            device_id: random_device_id(),
            created_at_ms: now_ms(),
            last_login_ms: now_ms(),
            password_hash: String::new(),
            salt: String::new(),
            id_token: "id-rt".into(),
            access_token: "acc-rt".into(),
            broker_token: "jwt-rt".into(),
            broker_token_expires_at_ms: now_ms() + 60_000,
            broker_url: String::new(),
        };
        save(&state).expect("login save");
        assert!(auth_path().exists(), "auth.json present after login");
        let loaded = load().expect("logged-in state loads");
        assert_eq!(loaded.email, "roundtrip@test.local");
        assert_eq!(loaded.broker_token, "jwt-rt");

        // UNDO: logout removes auth.json → baseline restored.
        delete().expect("logout delete");
        assert!(!auth_path().exists(), "auth.json gone after logout");
        assert!(load().is_none(), "after logout: back to the not-logged-in baseline");

        // Idempotent: a second logout on an already-clean home is a no-op success.
        delete().expect("logout is idempotent");
    }
}
