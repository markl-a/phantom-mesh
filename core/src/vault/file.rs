// core/src/vault/file.rs
//
// File-based `Vault` impl: payloads land in
// `<dir>/<key>.json` (e.g. `~/.phantom-mesh/auth.json`). Permissions
// are 0600 on Unix; NTFS ACL hardening on Windows is P-WIN-4 (v0.6.0
// follow-up) and DPAPI encryption is P-WIN-3 (v0.7.0).
//
// Matches the v0.5.0 `core/src/auth.rs` behavior verbatim except for
// the key-based generalization (auth.rs hard-codes "auth"; FileVault
// allows other keys like "telegram_persona" for v0.6.0 B3).

use super::Vault;
use anyhow::Context;
use serde::{de::DeserializeOwned, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct FileVault {
    /// Root directory holding `<key>.json` files.
    dir: PathBuf,
}

impl FileVault {
    /// Construct a FileVault rooted at `~/.phantom-mesh/`.
    /// Returns an error if `$HOME` is unset (rare).
    pub fn new() -> anyhow::Result<Self> {
        let data = crate::cli_config::phantom_data_dir()?;
        Self::new_in_dir(data)
    }

    /// Construct a FileVault rooted at `dir`. The dir is created if
    /// missing. Used by tests + alternative install paths.
    pub fn new_in_dir(dir: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create vault dir {}", dir.display()))?;
        Ok(Self { dir })
    }

    /// Path for a given key. Public so callers that need to point
    /// external tools (e.g. `phantom doctor`) at the file can.
    pub fn path_for(&self, key: &str) -> PathBuf {
        // Sanitize key the same way the tracer does — prevent
        // path traversal if `key` ever comes from untrusted source
        // (e.g. broker registration handler).
        let safe: String = key
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(format!("{}.json", safe))
    }

    /// Apply Unix mode 0600 to a freshly-written file. Best-effort: a
    /// failure here doesn't abort save (file is at least written), but
    /// `phantom doctor` should warn.
    #[cfg(unix)]
    fn restrict_perms(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut p = meta.permissions();
            p.set_mode(0o600);
            let _ = std::fs::set_permissions(path, p);
        }
    }

    #[cfg(not(unix))]
    fn restrict_perms(_path: &Path) {
        // Windows ACL hardening is P-WIN-4. Until then, rely on the
        // user-profile directory's default ACL (only current user has
        // read by default on Win10+).
    }
}

impl Vault for FileVault {
    fn load<T: DeserializeOwned>(&self, key: &str) -> anyhow::Result<Option<T>> {
        let path = self.path_for(key);
        if !path.exists() {
            return Ok(None);
        }
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("read vault file {}", path.display()))?;
        let value: T = serde_json::from_str(&body)
            .with_context(|| format!("parse vault file {}", path.display()))?;
        Ok(Some(value))
    }

    fn save<T: Serialize>(&self, key: &str, value: &T) -> anyhow::Result<()> {
        let path = self.path_for(key);
        // Atomic write: write to <path>.tmp then rename. This way
        // a phantom crash mid-write can't leave a corrupt file
        // (rename is atomic on the same filesystem).
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(value).context("serialize vault payload")?;
        {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&tmp)
                .with_context(|| format!("open vault tmp {}", tmp.display()))?;
            f.write_all(&body)
                .with_context(|| format!("write vault tmp {}", tmp.display()))?;
            f.sync_all().ok();
        }
        Self::restrict_perms(&tmp);
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
        Self::restrict_perms(&path);
        Ok(())
    }

    fn delete(&self, key: &str) -> anyhow::Result<()> {
        let path = self.path_for(key);
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("remove vault file {}", path.display()))?;
        }
        Ok(())
    }

    fn contains(&self, key: &str) -> bool {
        self.path_for(key).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tempfile::TempDir;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct DummyAuth {
        provider: String,
        email: String,
        device_id: String,
    }

    fn fixture() -> DummyAuth {
        DummyAuth {
            provider: "google".into(),
            email: "test@example.com".into(),
            device_id: "abc123".into(),
        }
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = TempDir::new().unwrap();
        let v = FileVault::new_in_dir(dir.path()).unwrap();

        let payload = fixture();
        v.save("auth", &payload).unwrap();
        let loaded: Option<DummyAuth> = v.load("auth").unwrap();
        assert_eq!(loaded.as_ref(), Some(&payload));
    }

    #[test]
    fn load_returns_none_when_key_absent() {
        let dir = TempDir::new().unwrap();
        let v = FileVault::new_in_dir(dir.path()).unwrap();
        let loaded: Option<DummyAuth> = v.load("nonexistent").unwrap();
        assert_eq!(loaded, None);
    }

    #[test]
    fn delete_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let v = FileVault::new_in_dir(dir.path()).unwrap();

        // Delete on empty
        v.delete("auth").unwrap();
        // Save then delete
        v.save("auth", &fixture()).unwrap();
        assert!(v.contains("auth"));
        v.delete("auth").unwrap();
        assert!(!v.contains("auth"));
        // Delete again
        v.delete("auth").unwrap();
    }

    #[test]
    fn contains_reports_existence_correctly() {
        let dir = TempDir::new().unwrap();
        let v = FileVault::new_in_dir(dir.path()).unwrap();
        assert!(!v.contains("auth"));
        v.save("auth", &fixture()).unwrap();
        assert!(v.contains("auth"));
    }

    #[test]
    fn path_traversal_key_sanitizes() {
        let dir = TempDir::new().unwrap();
        let v = FileVault::new_in_dir(dir.path()).unwrap();

        // Attempt to escape via "../"
        let malicious = "../../etc/passwd";
        v.save(malicious, &fixture()).unwrap();
        // Resulting path stays inside vault dir (underscores replace /
        // and . and ..).
        let path = v.path_for(malicious);
        assert!(
            path.starts_with(dir.path()),
            "path must not escape vault dir: {:?}",
            path
        );
        assert!(
            !path.to_string_lossy().contains("etc/passwd"),
            "sanitized path should not contain unescaped traversal"
        );
    }

    #[test]
    fn save_uses_atomic_write_no_tmp_leftover() {
        let dir = TempDir::new().unwrap();
        let v = FileVault::new_in_dir(dir.path()).unwrap();
        v.save("auth", &fixture()).unwrap();
        // Only auth.json should exist, not auth.json.tmp
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
            .collect();
        assert!(
            entries.iter().any(|n| n == "auth.json"),
            "auth.json must exist: {:?}",
            entries
        );
        assert!(
            !entries.iter().any(|n| n.ends_with(".tmp")),
            "no .tmp leftover: {:?}",
            entries
        );
    }

    #[test]
    fn save_then_load_with_different_types_is_independent() {
        let dir = TempDir::new().unwrap();
        let v = FileVault::new_in_dir(dir.path()).unwrap();

        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        struct Persona {
            agent: String,
            allowed_users: Vec<String>,
        }

        v.save("auth", &fixture()).unwrap();
        v.save(
            "telegram_persona",
            &Persona {
                agent: "master".into(),
                allowed_users: vec!["123".into()],
            },
        )
        .unwrap();

        // Both round-trip independently
        let a: DummyAuth = v.load("auth").unwrap().unwrap();
        let p: Persona = v.load("telegram_persona").unwrap().unwrap();
        assert_eq!(a.email, "test@example.com");
        assert_eq!(p.agent, "master");
    }
}
