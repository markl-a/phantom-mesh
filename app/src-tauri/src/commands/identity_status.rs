// Tauri command surfacing the REAL on-device cryptographic identity to the app
// (BIG-GOAL P4). The app's email "login" is a cosmetic display profile; the
// actual identity is the per-device root key at ~/.spectyn-mesh/identity.key
// (the HKDF seed that also decrypts Life Node events). This command reports its
// public fingerprint + age + keystore backend so the UI can show the user their
// true identity instead of a localStorage stub.
//
// Uses the SAME scheme as the TUI `/identity` pane (identity_view_from_state):
// load_pub_hex → hex::decode → identity_wire::fingerprint_short (SHA-256, 12
// hex chars); created_at = identity.key mtime. Read-only; never returns the
// secret seed or private key.

use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityStatus {
    /// True when a per-device identity.key exists on disk.
    pub has_identity: bool,
    /// Public key fingerprint (12 hex chars) or "—" when no key.
    pub fingerprint: String,
    /// identity.key file mtime (YYYY-MM-DD) or "—".
    pub created_at: String,
    /// Intended OS keystore backend for this platform.
    pub keystore: String,
    /// Human summary of an active auth session, if any (else null).
    pub identity_line: Option<String>,
}

#[tauri::command]
pub async fn identity_status() -> Result<IdentityStatus, String> {
    let identity_line =
        spectyn_mesh::auth::load().map(|s| spectyn_mesh::auth::human_summary(&s));

    let fingerprint = spectyn_mesh::identity::load_pub_hex()
        .ok()
        .and_then(|h| hex::decode(h.trim()).ok())
        .map(|bytes| spectyn_mesh::identity_wire::fingerprint_short(&bytes))
        .unwrap_or_else(|| "—".to_string());

    let key_path = dirs::home_dir()
        .map(|h| h.join(".spectyn-mesh").join("identity.key"));
    let has_identity = key_path.as_ref().map(|p| p.exists()).unwrap_or(false);
    let created_at = key_path
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
        .map(|t| {
            let dt: chrono::DateTime<chrono::Local> = t.into();
            dt.format("%Y-%m-%d").to_string()
        })
        .unwrap_or_else(|| "—".to_string());

    let keystore = if cfg!(target_os = "macos") {
        "macos-keychain"
    } else if cfg!(target_os = "windows") {
        "windows-dpapi"
    } else if cfg!(target_os = "linux") {
        "linux-secret-service (or file-chmod-0600 fallback)"
    } else {
        "file-chmod-0600"
    }
    .to_string();

    Ok(IdentityStatus {
        has_identity,
        fingerprint,
        created_at,
        keystore,
        identity_line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn identity_status_returns_wellformed() {
        let s = identity_status().await.expect("identity_status ok");
        // Never panics; fingerprint is either the 12-hex digest or the dash.
        assert!(s.fingerprint == "—" || s.fingerprint.len() == 12, "fp: {}", s.fingerprint);
        assert!(!s.keystore.is_empty());
    }
}
