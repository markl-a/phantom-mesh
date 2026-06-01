//! Local identity for `phantom login` / `whoami` / `logout`.
//!
//! Stored at `~/.phantom-mesh/auth.json` (mode 0600). Three identity
//! sources:
//!   - email: SHA-256(salt || password) with 100K iterations stored locally
//!   - google: OAuth 2.0 device-flow loopback, id_token saved
//!   - apple : stub (Apple OAuth requires HTTPS redirect; needs relay)
//!
//! The cloud broker is NOT involved (per COMMERCIAL-DESIGN.md §2 hard
//! rule #2: the OSS binary works without any cloud account). This file
//! only carries identity; `phantom devices` / mesh discovery against a
//! broker is added when phantom-cloud-client lands.

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
    /// Bearer <broker_token>` for `/api/me/*` calls (e.g. `phantom config
    /// pull`). TTL controlled by the broker's BROKER_TOKEN_TTL_SECS
    /// (default 7 days). Empty for non-broker logins (provider=email or
    /// direct provider=google flows that didn't go through phantommesh).
    #[serde(default)]
    pub broker_token: String,
    #[serde(default)]
    pub broker_token_expires_at_ms: i64,
    /// URL of the broker that issued the token (lets `phantom config pull`
    /// know where to call back without an extra arg).
    #[serde(default)]
    pub broker_url: String,
}

pub fn auth_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".phantom-mesh/auth.json")
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

/// Public-facing summary printed by `phantom whoami` and `phantom doctor`.
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
}
