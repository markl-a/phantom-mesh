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
/// Follows the de-facto `NO_COLOR` convention and only emits color
/// when stdout is a real terminal.
pub fn is_colored() -> bool {
    use std::io::IsTerminal;

    color_enabled(
        std::env::var("NO_COLOR").is_err(),
        std::io::stdout().is_terminal(),
    )
}

fn color_enabled(no_color_unset: bool, is_tty: bool) -> bool {
    no_color_unset && is_tty
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
    paint(text, code, is_colored())
}

fn paint(text: &str, code: u8, enabled: bool) -> String {
    if enabled {
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
    fn color_enabled_requires_no_color_unset_and_tty() {
        assert!(color_enabled(true, true));
        assert!(!color_enabled(true, false));
        assert!(!color_enabled(false, true));
    }

    #[test]
    fn color_enabled_false_when_not_tty() {
        assert!(!color_enabled(true, false));
    }

    #[test]
    fn paint_wraps_only_when_enabled() {
        assert_eq!(paint("hi", 32, true), "\x1b[32mhi\x1b[0m");
        assert_eq!(paint("hi", 32, false), "hi");
    }
}
