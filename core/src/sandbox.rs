//! CO-EVO Phase 1 sandbox guard (CO-EVOLUTION.md §38-69, SPEC-FREEZE-V1.1 §4.1-d).
//!
//! When `enable(true)` is called, file-mutating tools (file_write,
//! file_edit, multi_file_edit, apply_patch) refuse paths under any of
//! the protected prefixes:
//!
//!   core/         core Rust source — modifies daemon behaviour
//!   app/          Tauri shell + frontend — modifies UI
//!   templates/    systemd / launchd unit templates — modifies install
//!   scripts/      build / install scripts — modifies deploy
//!
//! Refusal returns a clear error message, NOT a panic. The agent
//! receives the error in its tool output and can decide whether to
//! pivot to a different approach OR escalate by passing
//! `--allow-core-evolve` on the next `spectyn evolve` invocation.
//!
//! Default: DISABLED. Existing TUI / REPL / MCP callers are unaffected.
//! Only `run_evolve_local` and `run_autoevolve` opt in by default.
//!
//! This is the v0.1.0 down-payment on Tier 1 sandbox. v0.2 adds:
//!   - configurable extra protected prefixes
//!   - allow-list per-recipe (recipe declares which paths it touches;
//!     adopt verifies the patch only touches those paths)
//!   - audit log of each refusal (to surface "agent kept trying to
//!     write core/serve.rs even though sandboxed")

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

static SANDBOX_ENABLED: AtomicBool = AtomicBool::new(false);

/// Globally enable / disable the sandbox guard. Idempotent;
/// `run_evolve_local` calls `enable(true)` at start unless
/// `--allow-core-evolve` was passed, then `enable(false)` on exit
/// so the surrounding session (e.g. interactive REPL) isn't
/// affected after the autoevolve loop completes.
pub fn enable(on: bool) {
    SANDBOX_ENABLED.store(on, Ordering::SeqCst);
}

pub fn is_enabled() -> bool {
    SANDBOX_ENABLED.load(Ordering::SeqCst)
}

/// Protected path prefixes (relative to repo root). When sandbox is
/// enabled, ANY path that resolves to a location under one of these
/// is rejected.
const PROTECTED_PREFIXES: &[&str] = &["core/", "app/", "templates/", "scripts/"];

/// Most explicitly sensitive sub-paths (per CO-EVOLUTION.md §107) —
/// these get rejected with extra emphasis even with --allow-core-evolve
/// in v0.2 unless `--I-really-mean-it` is also passed. v0.1.0 just
/// flags them in the error message.
pub const SENSITIVE_SUB_PATHS: &[&str] = &[
    "core/src/auth/",
    "core/src/mesh.rs",
    "core/src/keys.rs",
    "core/src/serve.rs",
    "core/src/identity.rs",
];

/// Result type for the guard check. `Allowed` = proceed; `Denied(msg)`
/// = caller should return the message as the tool output (no panic).
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Allowed,
    Denied(String),
}

/// Check whether `path` is allowed to be mutated under the current
/// sandbox state. When sandbox is disabled, always Allowed. When
/// enabled, denies under any PROTECTED_PREFIXES.
///
/// `path` is checked as both literal-substring and canonicalised-form
/// to defeat trivial workarounds like `./core/src/...` or absolute
/// paths that resolve back into the repo.
pub fn check<P: AsRef<Path>>(path: P) -> Verdict {
    if !is_enabled() {
        return Verdict::Allowed;
    }
    let p = path.as_ref();
    let s = p.to_string_lossy();

    // Try canonicalising; if it fails (file not yet existing for
    // file_write), fall back to the literal string.
    let canonical = std::fs::canonicalize(p)
        .ok()
        .map(|c| c.to_string_lossy().to_string())
        .unwrap_or_else(|| s.to_string());

    let needles: Vec<&str> = PROTECTED_PREFIXES.to_vec();
    let mut hit: Option<&str> = None;
    for prefix in &needles {
        // Match either "core/" anywhere in the literal path, or
        // "/core/" / "\\core\\" in the canonical absolute path.
        if s.contains(prefix)
            || canonical.contains(&format!("/{}", prefix))
            || canonical.contains(&format!("\\{}", prefix.replace('/', "\\")))
        {
            hit = Some(prefix);
            break;
        }
    }

    let Some(prefix) = hit else {
        return Verdict::Allowed;
    };

    let is_sensitive = SENSITIVE_SUB_PATHS
        .iter()
        .any(|sp| s.contains(sp) || canonical.contains(sp));

    let extra = if is_sensitive {
        " — and this path is in the SENSITIVE list (auth / mesh / keys / serve)"
    } else {
        ""
    };

    let msg = format!(
        "sandbox guard: refusing to write `{}` (under `{}`{}); \
         autoevolve runs sandboxed by default — pass `--allow-core-evolve` \
         to opt out, or write to ~/.spectyn-mesh/extensions/ which is the \
         intended Tier 1 surface (see docs/CONTRIBUTOR-FUNNEL.md §4)",
        s, prefix, extra
    );
    Verdict::Denied(msg)
}

/// [C5/T74] Process-wide lock used by tests in OTHER modules (e.g.
/// `tools::multi_edit`, `tools::patch`, `tools::fs`) to serialize their
/// sandbox enable/disable around `check()` calls. SANDBOX_ENABLED is a
/// process-global atomic, so any tests that flip it must hold this lock
/// to avoid racing one another.
#[cfg(test)]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_enabled<F: FnOnce()>(f: F) {
        let _g = test_lock();
        enable(true);
        f();
        enable(false);
    }

    #[test]
    fn disabled_allows_everything() {
        let _g = test_lock();
        enable(false);
        assert!(matches!(check("core/src/serve.rs"), Verdict::Allowed));
        assert!(matches!(check("anywhere.rs"), Verdict::Allowed));
    }

    #[test]
    fn enabled_blocks_protected_prefixes() {
        with_enabled(|| {
            assert!(matches!(check("core/src/serve.rs"), Verdict::Denied(_)));
            assert!(matches!(check("./core/src/main.rs"), Verdict::Denied(_)));
            assert!(matches!(check("app/src/index.tsx"), Verdict::Denied(_)));
            assert!(matches!(
                check("templates/spectyn-mesh.service.tmpl"),
                Verdict::Denied(_)
            ));
            assert!(matches!(check("scripts/build-mac.sh"), Verdict::Denied(_)));
        });
    }

    #[test]
    fn enabled_allows_extensions_and_other_paths() {
        with_enabled(|| {
            // ~/.spectyn-mesh/extensions/ is the intended Tier 1 path
            assert!(matches!(
                check("/Users/me/.spectyn-mesh/extensions/prompts/coder.md"),
                Verdict::Allowed
            ));
            // /tmp / non-repo paths are fine
            assert!(matches!(check("/tmp/foo.txt"), Verdict::Allowed));
            // README at repo root is fine (only listed prefixes blocked)
            assert!(matches!(check("README.md"), Verdict::Allowed));
            assert!(matches!(
                check("docs/CONTRIBUTOR-FUNNEL.md"),
                Verdict::Allowed
            ));
        });
    }

    #[test]
    fn sensitive_path_is_flagged_in_error_text() {
        with_enabled(|| {
            let v = check("core/src/keys.rs");
            match v {
                Verdict::Denied(msg) => assert!(
                    msg.contains("SENSITIVE"),
                    "sensitive path should mention SENSITIVE list, got: {msg}"
                ),
                Verdict::Allowed => panic!("must be denied"),
            }
            // Non-sensitive but protected — should not mention SENSITIVE
            let v = check("core/src/cost.rs");
            match v {
                Verdict::Denied(msg) => assert!(
                    !msg.contains("SENSITIVE"),
                    "non-sensitive path should not flag SENSITIVE, got: {msg}"
                ),
                Verdict::Allowed => panic!("must be denied"),
            }
        });
    }
}
