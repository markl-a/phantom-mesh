// core/src/vault/mod.rs
//
// Secret-storage abstraction for the phantom-mesh auth + identity layer.
//
// Today `auth.json` lives in `~/.phantom-mesh/auth.json` mode 0600
// (Unix) / NTFS ACL (Windows). The `Vault` trait abstracts that storage
// so future per-OS implementations can swap in:
//
//   FileVault       Unix 0600 + Win NTFS ACL (existing v0.5.0 behavior)
//   KeychainVault   macOS Keychain Service (P-MAC-4 / MAC-F1, v0.7.0)
//   DpapiVault      Windows DPAPI (P-WIN-3, v0.7.0)
//   SecretSvcVault  Linux Secret Service (freedesktop, v0.7.0+)
//
// PF-5 (this commit): defines the trait + ships `FileVault` matching
// current behavior. Does NOT yet swap `auth.rs` callers — that's a
// follow-up to keep blast radius small.

pub mod conversation_seal;
pub mod file;

pub use file::FileVault;

use serde::{de::DeserializeOwned, Serialize};

/// Generic credential vault. Stores opaque payloads keyed by a string
/// identifier (e.g. `"auth"` for the main auth.json, `"telegram_persona"`
/// for v0.6.0 B3 binding state).
///
/// Implementations MUST:
///   - persist the payload across phantom restarts
///   - protect against other-user reads where the OS allows
///     (Unix 0600 / Win NTFS ACL / macOS Keychain ACL / etc.)
///   - serialize as JSON (for FileVault + cross-impl debuggability)
///
/// Implementations MUST NOT:
///   - log payload values (only key names)
///   - expose values via any introspection API beyond `load()`
pub trait Vault: Send + Sync {
    /// Load the payload stored under `key`. Returns `Ok(None)` if no
    /// such entry exists. Returns `Err` only for transport / parse
    /// errors (file unreadable, JSON malformed, etc.).
    fn load<T: DeserializeOwned>(&self, key: &str) -> anyhow::Result<Option<T>>;

    /// Save (overwrite) the payload under `key`. Atomic where the
    /// underlying store supports it (FileVault uses
    /// write-temp-then-rename).
    fn save<T: Serialize>(&self, key: &str, value: &T) -> anyhow::Result<()>;

    /// Remove the entry under `key`. Returns `Ok(())` whether or not
    /// the entry existed (idempotent).
    fn delete(&self, key: &str) -> anyhow::Result<()>;

    /// `true` if an entry exists under `key`. Cheap probe — should
    /// not load the value.
    fn contains(&self, key: &str) -> bool;
}
