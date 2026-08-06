use super::DiscoveredProviderInfo;
use std::path::PathBuf;

pub struct DiscoveredCredential {
    pub name: String,
    pub provider_type: String,
    pub source: String,
}

impl DiscoveredCredential {
    pub fn to_frontend_info(&self) -> DiscoveredProviderInfo {
        DiscoveredProviderInfo {
            name: self.name.clone(),
            provider_type: self.provider_type.clone(),
            source: self.source.clone(),
        }
    }
}

pub async fn scan_all() -> Vec<DiscoveredCredential> {
    let mut found = Vec::new();

    if std::env::var("OPENAI_API_KEY")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        found.push(DiscoveredCredential {
            name: "openai".into(),
            provider_type: "openai".into(),
            source: "env".into(),
        });
    }
    if std::env::var("ANTHROPIC_API_KEY")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        found.push(DiscoveredCredential {
            name: "anthropic".into(),
            provider_type: "anthropic".into(),
            source: "env".into(),
        });
    }
    if std::env::var("OPENROUTER_API_KEY")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        found.push(DiscoveredCredential {
            name: "openrouter".into(),
            provider_type: "openrouter".into(),
            source: "env".into(),
        });
    }

    found
}

pub fn copilot_token_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = home_dir_lenient() {
        paths.push(
            home.join(".config")
                .join("github-copilot")
                .join("hosts.json"),
        );
        paths.push(
            home.join(".config")
                .join("github-copilot")
                .join("apps.json"),
        );
    }
    paths
}

pub fn claude_cli_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = home_dir_lenient() {
        // Claude Code has used both the dot-prefixed and plain file names across
        // versions; on macOS the credential usually lives in the Keychain
        // instead (see `claude_cli_keychain_json`).
        paths.push(home.join(".claude").join(".credentials.json"));
        paths.push(home.join(".claude").join("credentials.json"));
        paths.push(home.join(".config").join("claude").join("credentials.json"));
    }
    paths
}

/// Read the Claude Code login token JSON from the macOS Keychain.
///
/// Claude Code stores its credentials as a generic-password item named
/// `Claude Code-credentials`. Returns the raw JSON string, or `None` on
/// non-macOS platforms, when the item is absent, or when access is denied.
pub fn claude_cli_keychain_json() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("security")
            .args([
                "find-generic-password",
                "-s",
                "Claude Code-credentials",
                "-w",
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Candidate paths for the OpenAI Codex CLI credential cache.
/// `$CODEX_HOME/auth.json` (if set) takes precedence over `~/.codex/auth.json`.
pub fn codex_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        if !codex_home.is_empty() {
            paths.push(PathBuf::from(codex_home).join("auth.json"));
        }
    }
    if let Some(home) = home_dir_lenient() {
        paths.push(home.join(".codex").join("auth.json"));
        // spectyn-minted "Sign in with ChatGPT" token (opt-in OAuth) — checked
        // AFTER the official codex cache so a real `codex` login wins. Never
        // clobbers ~/.codex/auth.json. See providers::openai_oauth.
        paths.push(home.join(".spectyn-mesh").join("openai_oauth.json"));
    }
    paths
}

/// Resolve the user's home directory, leniently.
///
/// Prefers the `HOME` env var when set and non-empty — preserving Unix
/// behaviour and the `$HOME`-redirect isolation many tests rely on — and
/// falls back to [`dirs::home_dir()`], which resolves correctly on Windows
/// where `HOME` is normally unset (dirs queries Win32 / `USERPROFILE`).
///
/// All provider credential discovery should resolve home through this helper;
/// a bare `std::env::var("HOME")` silently finds nothing on Windows.
pub fn home_dir_lenient() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Restores the saved env var value on drop (panic-safe cleanup).
    struct VarGuard(&'static str, Option<String>);
    impl VarGuard {
        fn save(key: &'static str) -> Self {
            Self(key, std::env::var(key).ok())
        }
    }
    impl Drop for VarGuard {
        fn drop(&mut self) {
            match &self.1 {
                Some(v) => std::env::set_var(self.0, v),
                None => std::env::remove_var(self.0),
            }
        }
    }

    #[test]
    fn home_dir_lenient_prefers_home_env() {
        let _g = crate::env_lock::acquire();
        let _saved = VarGuard::save("HOME");
        std::env::set_var("HOME", "/tmp/spectyn-home-lenient-test");
        assert_eq!(
            home_dir_lenient(),
            Some(PathBuf::from("/tmp/spectyn-home-lenient-test"))
        );
    }

    #[test]
    fn home_dir_lenient_falls_back_when_home_unset() {
        let _g = crate::env_lock::acquire();
        let _saved = VarGuard::save("HOME");
        std::env::remove_var("HOME");
        // Must agree with dirs::home_dir() (Win32/USERPROFILE on Windows,
        // passwd fallback on Unix) instead of returning None.
        assert_eq!(home_dir_lenient(), dirs::home_dir());
    }

    #[test]
    fn home_dir_lenient_treats_empty_home_as_unset() {
        let _g = crate::env_lock::acquire();
        let _saved = VarGuard::save("HOME");
        std::env::set_var("HOME", "");
        assert_eq!(home_dir_lenient(), dirs::home_dir());
    }

    /// Regression: with `HOME` unset (the Windows reality), codex credential
    /// discovery must still yield a home-based path via the dirs fallback.
    /// Before routing through `home_dir_lenient()` this returned no
    /// `~/.codex/auth.json` candidate at all.
    #[test]
    fn codex_paths_resolve_without_home_env() {
        let _g = crate::env_lock::acquire();
        let _home = VarGuard::save("HOME");
        let _codex = VarGuard::save("CODEX_HOME");
        std::env::remove_var("HOME");
        std::env::remove_var("CODEX_HOME");
        if let Some(home) = dirs::home_dir() {
            assert!(
                codex_paths().contains(&home.join(".codex").join("auth.json")),
                "codex_paths must fall back to dirs::home_dir() when HOME is unset"
            );
        }
    }
}
