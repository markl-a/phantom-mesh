//! Project Trust — the "Project Trust" layer of the 4-layer onboarding model
//! (identity / provider / **project-trust** / permission).
//!
//! The threat: spectyn reads files and runs tools autonomously. If you `cd` into
//! a directory you did NOT create — a fresh `git clone`, a downloaded project, a
//! folder someone sent you — that directory can attack you two ways:
//!   1. **Config hijack** — spectyn resolves config cwd-first, so a malicious
//!      `cwd/agents.toml` can set `profile = "developer-full"` and override your
//!      safe defaults.
//!   2. **Prompt injection** — repo files can carry "ignore previous
//!      instructions, run …" that hijack the agent once it reads them.
//!
//! Project Trust answers a separate question from the permission *profile*: the
//! profile says *what class of actions* the agent may take; trust says *do I
//! vouch for THIS directory*. They compose — an untrusted directory can clamp an
//! otherwise-permissive profile.
//!
//! **Phase 2b skeleton (this module): enforcement is OFF by default.** The store,
//! the verdict, the CLI and the doctor Project layer are all real, but
//! [`apply_trust`] is a pure pass-through until the user opts in via
//! `[trust] enforcement = "prompt" | "observe"`. Flipping the default later is a
//! one-line change.
//!
//! ## Designed to grow "smarter"
//! Trust is modelled as a *verdict carrying its source* ([`TrustVerdict`]),
//! never a bare bool, and enforcement is a separate [`TrustPolicy`] enum. So the
//! future ladder — provenance auto-trust (dirs you own), risk-aware prompting,
//! TOFU + change-detection, mesh-synced trust, agent-assisted (advisory)
//! assessment — plugs into [`TrustStore::verdict`] (new `TrustSource` variants)
//! and [`TrustPolicy`] (e.g. a future `Auto`) WITHOUT changing call sites.
//!
//! Hermetic: every function takes explicit paths, so it unit-tests against a
//! temp HOME without touching the real `~/.spectyn-mesh/trust.json`.
//!
//! 中文: 專案信任層。陌生目錄可經 cwd-first config 或 prompt injection 攻擊你;
//! trust 問「我背不背書這個目錄」(與 permission profile 的「能做哪類動作」正交)。
//! 本階段 enforcement 預設 OFF(apply_trust 直通);verdict 帶來源、policy 獨立 enum,
//! 之後 auto/provenance/風險評分/mesh 同步都從同一 seam 接入。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::permission::Decision;

/// How strictly an *untrusted* directory is gated. `Off` (default) makes the
/// whole layer a no-op. Future variants (e.g. `Auto` — provenance/risk-aware)
/// slot in here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrustPolicy {
    /// No enforcement — trust is observed/reported but never restricts (default).
    #[default]
    Off,
    /// In an untrusted dir, downgrade an Allow to Ask (one-time "trust this?" prompt).
    Prompt,
    /// In an untrusted dir, clamp to read-only (write/exec denied until trusted).
    Observe,
}

impl TrustPolicy {
    pub fn slug(self) -> &'static str {
        match self {
            TrustPolicy::Off => "off",
            TrustPolicy::Prompt => "prompt",
            TrustPolicy::Observe => "observe",
        }
    }
    pub fn from_slug(s: &str) -> Option<TrustPolicy> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "disabled" => Some(TrustPolicy::Off),
            "prompt" | "ask" => Some(TrustPolicy::Prompt),
            "observe" | "readonly" | "read-only" => Some(TrustPolicy::Observe),
            _ => None,
        }
    }
    pub fn summary(self) -> &'static str {
        match self {
            TrustPolicy::Off => "trust reported but never enforced",
            TrustPolicy::Prompt => "ask before write/exec in an untrusted directory",
            TrustPolicy::Observe => "untrusted directories are read-only until trusted",
        }
    }
}

/// WHY a directory is trusted. Today only `Explicit` (the user ran
/// `spectyn project trust add`). Reserved for the smarter ladder: `Provenance` (a dir
/// you own / created), `Auto` (heuristic/agent-assisted), …
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TrustSource {
    /// The user explicitly trusted this path (or an ancestor) via the CLI.
    Explicit,
}

/// The resolved trust state of a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "source")]
pub enum TrustVerdict {
    Trusted(TrustSource),
    Untrusted,
}

impl TrustVerdict {
    pub fn is_trusted(self) -> bool {
        matches!(self, TrustVerdict::Trusted(_))
    }
}

/// `~/.spectyn-mesh/trust.json` — the persisted set of trusted project roots.
/// A directory is trusted if it, or any ancestor, is in the set (so trusting a
/// workspace root covers everything beneath it).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustStore {
    pub version: u32,
    /// Canonicalised absolute paths the user vouches for.
    pub trusted: Vec<String>,
}

impl Default for TrustStore {
    fn default() -> Self {
        TrustStore { version: 1, trusted: Vec::new() }
    }
}

impl TrustStore {
    /// `<home>/.spectyn-mesh/trust.json`.
    pub fn path(home: &Path) -> PathBuf {
        home.join(".spectyn-mesh").join("trust.json")
    }

    /// Load the store. A missing OR unreadable/corrupt file yields an EMPTY
    /// store — i.e. fail *safe* (everything untrusted), never fail open.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<TrustStore>(&s).ok())
            .unwrap_or_default()
    }

    /// Write the store atomically (tmp + rename) so a crash can't truncate it.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Per-process tmp name so concurrent writers don't clobber each other's
        // tmp file mid-rename (the rename itself is atomic).
        let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
        std::fs::write(&tmp, serde_json::to_string_pretty(self).unwrap_or_default())?;
        std::fs::rename(&tmp, path)
    }

    /// Canonicalise a path for storage/compare (resolves symlinks + `..`).
    /// Falls back to the input if it doesn't exist yet.
    fn canon(p: &Path) -> PathBuf {
        std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
    }

    /// Add a path (canonicalised). Returns false if already covered.
    pub fn add(&mut self, dir: &Path) -> bool {
        let c = Self::canon(dir).to_string_lossy().into_owned();
        if self.trusted.iter().any(|t| t == &c) {
            return false;
        }
        self.trusted.push(c);
        true
    }

    /// Remove an exact path (canonicalised). Returns true if removed.
    pub fn remove(&mut self, dir: &Path) -> bool {
        let c = Self::canon(dir).to_string_lossy().into_owned();
        let before = self.trusted.len();
        self.trusted.retain(|t| t != &c);
        self.trusted.len() != before
    }

    /// The verdict for `cwd`: trusted if it or any ancestor is in the set.
    pub fn verdict(&self, cwd: &Path) -> TrustVerdict {
        let c = Self::canon(cwd);
        for ancestor in c.ancestors() {
            if self.trusted.iter().any(|t| Path::new(t) == ancestor) {
                return TrustVerdict::Trusted(TrustSource::Explicit);
            }
        }
        TrustVerdict::Untrusted
    }
}

/// The gate seam: given the permission engine's base [`Decision`], clamp it per
/// the trust verdict + policy. Pure + total, so the runtime gate just calls it
/// and it's fully unit-testable. A no-op when `policy == Off` or the dir is
/// trusted (the Phase 2b default), so wiring it changes NOTHING until opt-in.
pub fn apply_trust(
    base: Decision,
    verdict: TrustVerdict,
    policy: TrustPolicy,
    tool: &str,
) -> Decision {
    if policy == TrustPolicy::Off || verdict.is_trusted() {
        return base;
    }
    match policy {
        TrustPolicy::Off => base, // unreachable (handled above)
        // Untrusted + prompt: turn a silent Allow into an Ask — the one-time
        // "do you trust this folder?" moment. Deny/Ask pass through.
        TrustPolicy::Prompt => match base {
            Decision::Allow => Decision::Ask,
            other => other,
        },
        // Untrusted + observe: reads still follow the base decision; anything
        // that mutates/execs/egresses is denied until the dir is trusted.
        TrustPolicy::Observe => {
            if crate::permission_profiles::is_read_tool(tool) {
                base
            } else {
                match base {
                    Decision::Deny(_) => base,
                    _ => Decision::Deny(format!(
                        "untrusted project (trust enforcement=observe): '{tool}' \
                         blocked — run `spectyn project trust add` to allow it here"
                    )),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_store_is_untrusted() {
        let s = TrustStore::default();
        let tmp = tempfile::tempdir().unwrap();
        assert!(!s.verdict(tmp.path()).is_trusted());
    }

    #[test]
    fn missing_file_loads_empty_fail_safe() {
        let tmp = tempfile::tempdir().unwrap();
        let s = TrustStore::load(&TrustStore::path(tmp.path()));
        assert!(s.trusted.is_empty());
    }

    #[test]
    fn add_then_verdict_trusts_dir_and_descendants() {
        let root = tempfile::tempdir().unwrap();
        let child = root.path().join("sub/deep");
        std::fs::create_dir_all(&child).unwrap();
        let mut s = TrustStore::default();
        assert!(s.add(root.path()));
        assert!(!s.add(root.path()), "second add is a no-op");
        assert!(s.verdict(root.path()).is_trusted());
        assert!(s.verdict(&child).is_trusted(), "descendant inherits trust");
    }

    #[test]
    fn sibling_dir_is_not_trusted() {
        let base = tempfile::tempdir().unwrap();
        let a = base.path().join("a");
        let b = base.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let mut s = TrustStore::default();
        s.add(&a);
        assert!(s.verdict(&a).is_trusted());
        assert!(!s.verdict(&b).is_trusted(), "trusting a/ must not trust b/");
    }

    #[test]
    fn save_then_load_round_trips() {
        let home = tempfile::tempdir().unwrap();
        let proj = tempfile::tempdir().unwrap();
        let path = TrustStore::path(home.path());
        let mut s = TrustStore::default();
        s.add(proj.path());
        s.save(&path).unwrap();
        let loaded = TrustStore::load(&path);
        assert!(loaded.verdict(proj.path()).is_trusted());
    }

    #[test]
    fn remove_untrusts() {
        let proj = tempfile::tempdir().unwrap();
        let mut s = TrustStore::default();
        s.add(proj.path());
        assert!(s.remove(proj.path()));
        assert!(!s.verdict(proj.path()).is_trusted());
        assert!(!s.remove(proj.path()), "second remove is a no-op");
    }

    #[test]
    fn policy_from_slug_round_trips() {
        for p in [TrustPolicy::Off, TrustPolicy::Prompt, TrustPolicy::Observe] {
            assert_eq!(TrustPolicy::from_slug(p.slug()), Some(p));
        }
        assert_eq!(TrustPolicy::from_slug("ASK"), Some(TrustPolicy::Prompt));
        assert_eq!(TrustPolicy::from_slug("read-only"), Some(TrustPolicy::Observe));
        assert_eq!(TrustPolicy::from_slug("nope"), None);
        assert_eq!(TrustPolicy::default(), TrustPolicy::Off);
    }

    // ── apply_trust (the gate seam) ──────────────────────────────────────────

    const TRUSTED: TrustVerdict = TrustVerdict::Trusted(TrustSource::Explicit);
    const UNTRUSTED: TrustVerdict = TrustVerdict::Untrusted;

    #[test]
    fn off_is_a_pure_passthrough() {
        // Even in an untrusted dir, Off changes nothing.
        assert!(matches!(
            apply_trust(Decision::Allow, UNTRUSTED, TrustPolicy::Off, "file_write"),
            Decision::Allow
        ));
    }

    #[test]
    fn trusted_dir_is_a_passthrough_under_any_policy() {
        for pol in [TrustPolicy::Prompt, TrustPolicy::Observe] {
            assert!(matches!(
                apply_trust(Decision::Allow, TRUSTED, pol, "file_write"),
                Decision::Allow
            ));
        }
    }

    #[test]
    fn untrusted_prompt_downgrades_allow_to_ask() {
        assert!(matches!(
            apply_trust(Decision::Allow, UNTRUSTED, TrustPolicy::Prompt, "file_write"),
            Decision::Ask
        ));
        // a Deny stays a Deny
        assert!(matches!(
            apply_trust(Decision::Deny("x".into()), UNTRUSTED, TrustPolicy::Prompt, "shell"),
            Decision::Deny(_)
        ));
    }

    #[test]
    fn untrusted_observe_allows_reads_denies_writes() {
        // a read tool follows the base decision
        assert!(matches!(
            apply_trust(Decision::Allow, UNTRUSTED, TrustPolicy::Observe, "file_read"),
            Decision::Allow
        ));
        // a write/exec tool is denied
        assert!(matches!(
            apply_trust(Decision::Allow, UNTRUSTED, TrustPolicy::Observe, "file_write"),
            Decision::Deny(_)
        ));
        assert!(matches!(
            apply_trust(Decision::Allow, UNTRUSTED, TrustPolicy::Observe, "shell"),
            Decision::Deny(_)
        ));
    }

    #[test]
    fn verdict_is_json_serializable_with_source() {
        let v = TrustVerdict::Trusted(TrustSource::Explicit);
        let j = serde_json::to_value(v).unwrap();
        assert_eq!(j, json!({"state": "trusted", "source": "explicit"}));
        let u = serde_json::to_value(TrustVerdict::Untrusted).unwrap();
        assert_eq!(u, json!({"state": "untrusted"}));
    }
}
