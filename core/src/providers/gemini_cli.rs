//! Gemini / Antigravity CLI (Google subscription) credential discovery.
//!
//! The Gemini CLI / Antigravity stores a Google OAuth credential at
//! `~/.gemini/oauth_creds.json` (standard Google token shape):
//! ```json
//! { "access_token": "ya29...", "refresh_token": "1//...",
//!   "scope": "...", "token_type": "Bearer", "id_token": "...",
//!   "expiry_date": 1717200000000 }
//! ```
//! The free/subscription tier drives the **Gemini Code Assist** backend
//! (`cloudcode-pa.googleapis.com`), which is keyed to a GCP project recorded in
//! `~/.gemini/projects.json` / `~/.gemini/google_accounts.json`.
//!
//! NOTE: using these credentials from a non-Google client is against Google's
//! terms and may get the account flagged. We only *read* the token the official
//! CLI already cached; we never run our own OAuth flow here.

use serde_json::Value;

/// Resolved Gemini Code Assist credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeminiCliAuth {
    /// Google OAuth `access_token` (Bearer).
    pub access_token: String,
    /// GCP project the Code Assist quota is bound to, if discoverable.
    pub project_id: Option<String>,
}

/// Extract the access token from a parsed `oauth_creds.json`, optionally pairing
/// it with a project id discovered from `projects.json`.
pub fn extract_gemini_auth(creds: &Value, projects: Option<&Value>) -> Option<GeminiCliAuth> {
    let access_token = creds
        .get("access_token")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?
        .to_string();
    Some(GeminiCliAuth {
        access_token,
        project_id: projects.and_then(extract_project_id),
    })
}

/// Best-effort project id from `projects.json`. The file maps accounts/projects
/// in a shape that has varied across versions, so we accept either a top-level
/// `project`/`projectId` string or the first such value found one level deep.
fn extract_project_id(projects: &Value) -> Option<String> {
    for key in ["project", "projectId", "project_id"] {
        if let Some(p) = projects.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            return Some(p.to_string());
        }
    }
    if let Some(obj) = projects.as_object() {
        for (_k, v) in obj {
            for key in ["project", "projectId", "project_id"] {
                if let Some(p) = v.get(key).and_then(|x| x.as_str()).filter(|s| !s.is_empty()) {
                    return Some(p.to_string());
                }
            }
        }
    }
    None
}

/// Locate the current Gemini CLI credential from `~/.gemini/`.
pub fn find_gemini_auth() -> Option<GeminiCliAuth> {
    let gemini = super::credential_scanner::home_dir_lenient()?.join(".gemini");
    let creds: Value =
        serde_json::from_str(&std::fs::read_to_string(gemini.join("oauth_creds.json")).ok()?)
            .ok()?;
    let projects: Option<Value> = std::fs::read_to_string(gemini.join("projects.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    extract_gemini_auth(&creds, projects.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_access_token() {
        let creds = serde_json::json!({
            "access_token": "ya29.abc", "refresh_token": "1//x",
            "token_type": "Bearer", "expiry_date": 1717200000000u64
        });
        let a = extract_gemini_auth(&creds, None).unwrap();
        assert_eq!(a.access_token, "ya29.abc");
        assert!(a.project_id.is_none());
    }

    #[test]
    fn pairs_project_id_nested() {
        let creds = serde_json::json!({ "access_token": "ya29.abc" });
        let projects = serde_json::json!({ "me@example.com": { "project": "my-proj-123" } });
        let a = extract_gemini_auth(&creds, Some(&projects)).unwrap();
        assert_eq!(a.project_id.as_deref(), Some("my-proj-123"));
    }

    #[test]
    fn none_without_access_token() {
        assert!(extract_gemini_auth(&serde_json::json!({ "refresh_token": "x" }), None).is_none());
    }
}
