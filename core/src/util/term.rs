// core/src/util/term.rs
//
// Terminal ANSI helpers shared across the binary + lib modules.
//
// PF-2d (this commit): consolidates 3 private copies that PF-2a +
// PF-2b had to duplicate while the binary↔lib boundary made calling
// the bin's local `colored()` impossible from `core/src/service/*`.
// Single canonical home eliminates drift.

/// `true` when ANSI color escapes should be emitted to stdout/stderr.
///
/// Follows the de-facto `NO_COLOR` convention: any non-empty value
/// disables color. Matches the original bin-local implementation
/// 1:1 (do NOT switch to `IsTerminal`-based logic without
/// confirming `phantom doctor` output, which depends on the
/// env-only branch for CI / pipe-redirected runs).
pub fn is_colored() -> bool {
    std::env::var("NO_COLOR").is_err()
}

/// Wrap `text` in an ANSI SGR escape with the given color `code`,
/// or return `text` unchanged when color is disabled.
///
/// `code` is the SGR foreground attribute (e.g. 31=red, 32=green,
/// 33=yellow, 35=magenta, 36=cyan, 90=bright-black). Background +
/// 256-color + truecolor are intentionally out of scope; callers
/// that need richer formatting should use a proper crate (we don't
/// pull `colored`/`termcolor` for this one-liner).
pub fn colored(text: &str, code: u8) -> String {
    if is_colored() {
        format!("\x1b[{}m{}\x1b[0m", code, text)
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// We can't reliably toggle `NO_COLOR` from a unit test without
    /// racing other tests (env is process-global). Instead, exercise
    /// both branches by calling the impl directly with explicit
    /// assertions on the format string shape.
    ///
    /// `crate::env_lock` guards the env mutation. See its module
    /// comment in `core/src/lib.rs` for the rationale.
    #[test]
    fn colored_emits_sgr_when_no_color_unset() {
        let _g = crate::env_lock::acquire();
        let prev = std::env::var("NO_COLOR").ok();
        std::env::remove_var("NO_COLOR");

        let out = colored("hi", 32);
        assert_eq!(out, "\x1b[32mhi\x1b[0m");
        assert!(is_colored());

        // Restore env so other tests aren't affected.
        if let Some(v) = prev {
            std::env::set_var("NO_COLOR", v);
        }
    }

    #[test]
    fn colored_returns_plain_when_no_color_set() {
        let _g = crate::env_lock::acquire();
        let prev = std::env::var("NO_COLOR").ok();
        std::env::set_var("NO_COLOR", "1");

        let out = colored("hi", 32);
        assert_eq!(out, "hi");
        assert!(!is_colored());

        match prev {
            Some(v) => std::env::set_var("NO_COLOR", v),
            None => std::env::remove_var("NO_COLOR"),
        }
    }

    #[test]
    fn colored_with_different_codes_uses_each_correctly() {
        let _g = crate::env_lock::acquire();
        std::env::remove_var("NO_COLOR");
        assert_eq!(colored("err", 31), "\x1b[31merr\x1b[0m");
        assert_eq!(colored("ok", 32), "\x1b[32mok\x1b[0m");
        assert_eq!(colored("warn", 33), "\x1b[33mwarn\x1b[0m");
    }
}
