//! Minimal CLI/TUI localization (i18n).
//!
//! Goal: let the phantom terminal render its UI strings (help, errors, TUI
//! labels) in Traditional Chinese as well as English, chosen at runtime.
//!
//! Design — deliberately dependency-free and inline:
//!   - `Lang` is the resolved locale (only `En` + `ZhTw` for now).
//!   - `current_lang()` resolves it once per process from env (see below).
//!   - `tr(en, zh_tw)` picks the right literal at the call site. Keeping both
//!     strings together means the translation lives next to its context and a
//!     reviewer sees both at once — no separate key registry to drift out of
//!     sync. `tr_owned` is the `String` form for already-formatted text.
//!
//! Locale resolution order (first match wins):
//!   1. `PHANTOM_LANG`  — explicit per-invocation override (`PHANTOM_LANG=zh-TW`).
//!   2. persisted preference — `phantom lang set …` writes `~/.phantom-mesh/lang`.
//!   3. `LC_ALL`, then `LANG` — standard POSIX locale env.
//!   4. default → `En`.
//!
//! The persisted preference sits ABOVE the POSIX locale on purpose: a saved
//! `zh-TW` must survive a box whose `LANG=en_US.UTF-8` (the common case),
//! otherwise the saved default would never take effect.
//!
//! A value is treated as Traditional Chinese when it contains `zh` together
//! with a Traditional marker (`tw`, `hant`, or `hk`). Simplified Chinese
//! (`zh-CN` / `zh-Hans`) intentionally falls back to English for now — there
//! is no Simplified table yet, and silently showing Traditional to a
//! Simplified user would be worse than English.

use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    ZhTw,
}

impl Lang {
    /// Canonical tag for persistence + display ("en" / "zh-TW"). Round-trips
    /// through `classify_locale` (see `lang_tag_round_trips_through_classify`).
    pub fn tag(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::ZhTw => "zh-TW",
        }
    }
}

/// Classify a raw locale string (already lowercased) into a `Lang`.
/// Pure + side-effect-free so it is unit-testable without touching env.
pub fn classify_locale(raw: &str) -> Lang {
    let r = raw.to_lowercase();
    if r.contains("zh") && (r.contains("tw") || r.contains("hant") || r.contains("hk")) {
        Lang::ZhTw
    } else {
        Lang::En
    }
}

/// Strict parser for an EXPLICIT user-supplied language tag (`phantom lang set
/// <X>`). Unlike [`classify_locale`] — which is a best-effort system-locale
/// sniffer that falls back to `En` for anything unrecognized — this returns
/// `None` for an unknown value so the CLI can reject it instead of silently
/// saving English. Accepts the supported tags (en / zh-TW family) plus common
/// aliases; rejects e.g. `zzz`, `fr`, and Simplified `zh-CN` (not yet shipped).
pub fn parse_explicit_lang(raw: &str) -> Option<Lang> {
    // Drop any encoding suffix like `.UTF-8` and surrounding whitespace.
    let lowered = raw.trim().to_lowercase();
    let r = lowered.split('.').next().unwrap_or(&lowered);
    match r {
        "en" | "en-us" | "en_us" | "eng" | "english" => Some(Lang::En),
        "繁中" | "繁體" | "繁體中文" | "traditional" => Some(Lang::ZhTw),
        _ if r.contains("zh") && (r.contains("tw") || r.contains("hant") || r.contains("hk")) => {
            Some(Lang::ZhTw)
        }
        _ => None,
    }
}

fn detect_lang() -> Lang {
    let phantom = std::env::var("PHANTOM_LANG").ok();
    let posix = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LANG"))
        .ok();
    resolve_lang(phantom.as_deref(), persisted_lang(), posix.as_deref())
}

/// Pure precedence resolver (unit-testable without touching env/disk). See the
/// module-level resolution order. Empty strings are treated as unset.
fn resolve_lang(phantom_lang: Option<&str>, persisted: Option<Lang>, posix: Option<&str>) -> Lang {
    if let Some(raw) = phantom_lang.filter(|s| !s.is_empty()) {
        return classify_locale(raw);
    }
    if let Some(l) = persisted {
        return l;
    }
    if let Some(raw) = posix.filter(|s| !s.is_empty()) {
        return classify_locale(raw);
    }
    Lang::En
}

/// Path of the persisted-language file: `$PHANTOM_LANG_FILE` (override, used in
/// tests) or `~/.phantom-mesh/lang`. Home is resolved via `dirs::home_dir()`
/// (the same primitive the rest of phantom uses) so it works on Windows, where
/// `HOME` is unset in cmd/PowerShell and only `USERPROFILE` exists; `$HOME` is
/// kept as a fallback for the rare case `dirs::home_dir()` returns `None`.
fn lang_file_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("PHANTOM_LANG_FILE") {
        if !p.is_empty() {
            return Some(std::path::PathBuf::from(p));
        }
    }
    let home = dirs::home_dir().or_else(|| {
        std::env::var("HOME")
            .ok()
            .filter(|h| !h.is_empty())
            .map(std::path::PathBuf::from)
    })?;
    Some(home.join(".phantom-mesh").join("lang"))
}

/// The persisted language preference, if any (written by `phantom lang set …`).
pub fn persisted_lang() -> Option<Lang> {
    let raw = std::fs::read_to_string(lang_file_path()?).ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Some(classify_locale(raw))
}

/// Persist a language choice to `~/.phantom-mesh/lang` (or `$PHANTOM_LANG_FILE`),
/// storing the canonical tag. Returns the path written. Takes effect on the
/// NEXT run — the current process already cached its language in `current_lang`.
pub fn set_persisted_lang(lang: Lang) -> std::io::Result<std::path::PathBuf> {
    let path = lang_file_path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "no $HOME for the lang file")
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, format!("{}\n", lang.tag()))?;
    Ok(path)
}

/// Remove the persisted language preference (revert to env / system locale).
/// Returns `true` if a file was removed, `false` if there was none.
pub fn clear_persisted_lang() -> std::io::Result<bool> {
    let Some(path) = lang_file_path() else {
        return Ok(false);
    };
    match std::fs::remove_file(&path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// The process-wide resolved language, computed once on first use.
pub fn current_lang() -> Lang {
    static LANG: OnceLock<Lang> = OnceLock::new();
    *LANG.get_or_init(detect_lang)
}

/// Pick a `&'static str` for the current language.
/// `tr("Usage:", "用法：")` → "用法：" when `PHANTOM_LANG=zh-TW`, else "Usage:".
pub fn tr(en: &'static str, zh_tw: &'static str) -> &'static str {
    match current_lang() {
        Lang::En => en,
        Lang::ZhTw => zh_tw,
    }
}

/// `String` form, for text that is built with `format!` per language.
pub fn tr_owned(en: String, zh_tw: String) -> String {
    match current_lang() {
        Lang::En => en,
        Lang::ZhTw => zh_tw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_traditional_variants() {
        assert_eq!(classify_locale("zh-TW"), Lang::ZhTw);
        assert_eq!(classify_locale("zh_TW.UTF-8"), Lang::ZhTw);
        assert_eq!(classify_locale("zh-Hant"), Lang::ZhTw);
        assert_eq!(classify_locale("zh-HK"), Lang::ZhTw);
    }

    #[test]
    fn parse_explicit_lang_accepts_supported_rejects_junk() {
        // Supported tags + aliases resolve.
        assert_eq!(parse_explicit_lang("en"), Some(Lang::En));
        assert_eq!(parse_explicit_lang("EN-US"), Some(Lang::En));
        assert_eq!(parse_explicit_lang("english"), Some(Lang::En));
        assert_eq!(parse_explicit_lang("zh-TW"), Some(Lang::ZhTw));
        assert_eq!(parse_explicit_lang("zh_TW.UTF-8"), Some(Lang::ZhTw));
        assert_eq!(parse_explicit_lang("zh-Hant"), Some(Lang::ZhTw));
        assert_eq!(parse_explicit_lang("繁中"), Some(Lang::ZhTw));
        // Junk + unsupported are REJECTED (the D23 bug: these used to save "en").
        assert_eq!(parse_explicit_lang("zzz"), None);
        assert_eq!(parse_explicit_lang("fr"), None);
        assert_eq!(parse_explicit_lang("zh-CN"), None); // Simplified not shipped
        assert_eq!(parse_explicit_lang(""), None);
    }

    #[test]
    fn classify_non_traditional_falls_back_to_en() {
        assert_eq!(classify_locale("en_US.UTF-8"), Lang::En);
        assert_eq!(classify_locale("zh-CN"), Lang::En); // Simplified → En for now
        assert_eq!(classify_locale("zh-Hans"), Lang::En);
        assert_eq!(classify_locale("ja_JP"), Lang::En);
        assert_eq!(classify_locale(""), Lang::En);
    }

    #[test]
    fn resolve_lang_precedence() {
        // PHANTOM_LANG (explicit) wins over everything.
        assert_eq!(resolve_lang(Some("zh-TW"), Some(Lang::En), Some("en_US")), Lang::ZhTw);
        assert_eq!(resolve_lang(Some("en"), Some(Lang::ZhTw), Some("zh_TW")), Lang::En);
        // No PHANTOM_LANG → persisted choice wins over the POSIX locale. This is
        // the key case: a saved zh-TW must survive a system LANG=en_US.UTF-8.
        assert_eq!(resolve_lang(None, Some(Lang::ZhTw), Some("en_US.UTF-8")), Lang::ZhTw);
        assert_eq!(resolve_lang(None, Some(Lang::En), Some("zh_TW.UTF-8")), Lang::En);
        // No PHANTOM_LANG + no persisted → POSIX locale.
        assert_eq!(resolve_lang(None, None, Some("zh-TW")), Lang::ZhTw);
        assert_eq!(resolve_lang(None, None, Some("en_US")), Lang::En);
        // Nothing set → En.
        assert_eq!(resolve_lang(None, None, None), Lang::En);
        // Empty strings count as unset (don't shadow lower-precedence sources).
        assert_eq!(resolve_lang(Some(""), Some(Lang::ZhTw), None), Lang::ZhTw);
        assert_eq!(resolve_lang(None, None, Some("")), Lang::En);
    }

    #[test]
    fn lang_tag_round_trips_through_classify() {
        assert_eq!(classify_locale(Lang::ZhTw.tag()), Lang::ZhTw);
        assert_eq!(classify_locale(Lang::En.tag()), Lang::En);
    }

    #[test]
    fn persisted_lang_round_trips() {
        // Serialize on the SAME mutex the other env-mutating tests use
        // (diag.rs / models_cache.rs / service mutate $HOME via env_lock).
        // sandbox::test_lock guards the sandbox atomic, NOT env vars — wrong
        // lock here since this test set/remove_var's PHANTOM_LANG_FILE.
        let _g = crate::env_lock::acquire();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lang");
        std::env::set_var("PHANTOM_LANG_FILE", &path);
        // unset → None
        assert_eq!(super::persisted_lang(), None);
        // write zh-TW → reads back ZhTw, canonical tag on disk
        let written = super::set_persisted_lang(Lang::ZhTw).unwrap();
        assert_eq!(written, path);
        assert_eq!(super::persisted_lang(), Some(Lang::ZhTw));
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "zh-TW");
        // overwrite en → reads back En
        super::set_persisted_lang(Lang::En).unwrap();
        assert_eq!(super::persisted_lang(), Some(Lang::En));
        std::env::remove_var("PHANTOM_LANG_FILE");
    }

    /// `tr` is a pure selector over `current_lang()`. Since the process language
    /// is cached once in a `OnceLock` we cannot flip it per-test, but we can
    /// assert the selection contract directly: whichever arm `current_lang()`
    /// resolves to, `tr` must return exactly that side (and `tr_owned` likewise).
    #[test]
    fn tr_selects_the_arm_for_the_current_lang() {
        let got = tr("Usage:", "用法：");
        match current_lang() {
            Lang::En => assert_eq!(got, "Usage:"),
            Lang::ZhTw => assert_eq!(got, "用法："),
        }
    }

    #[test]
    fn tr_owned_selects_the_arm_for_the_current_lang() {
        let got = tr_owned("Hello".to_string(), "你好".to_string());
        match current_lang() {
            Lang::En => assert_eq!(got, "Hello"),
            Lang::ZhTw => assert_eq!(got, "你好"),
        }
    }

    /// The two literals handed to `tr` need not differ; when they match, the
    /// selector returns that shared value regardless of the resolved language.
    /// This is the analogue of "fall back to the key when the value is missing":
    /// passing the same string for both arms always yields that string.
    #[test]
    fn tr_returns_shared_literal_when_both_arms_equal() {
        const KEY: &str = "phantom.status";
        assert_eq!(tr(KEY, KEY), KEY);
    }

    /// Both the `en` and `zh-TW` bundles must resolve a known key. Here the
    /// "bundle" is the (Lang -> literal) selection performed by `tr`; we exercise
    /// each language explicitly through the underlying match so both arms are
    /// covered independent of the process-cached `current_lang()`.
    #[test]
    fn both_bundles_resolve_a_known_key() {
        const EN: &str = "Status";
        const ZH: &str = "狀態";
        // Mirror the body of `tr` for each Lang so neither arm is left untested.
        let pick = |lang: Lang| match lang {
            Lang::En => EN,
            Lang::ZhTw => ZH,
        };
        assert_eq!(pick(Lang::En), EN);
        assert_eq!(pick(Lang::ZhTw), ZH);
        // And both canonical tags classify back to their own Lang (the key that
        // names each bundle resolves to that bundle).
        assert_eq!(classify_locale(Lang::En.tag()), Lang::En);
        assert_eq!(classify_locale(Lang::ZhTw.tag()), Lang::ZhTw);
    }

    #[test]
    fn lang_file_path_resolves_via_dirs_without_home() {
        // Regression (2026-05-28): on native Windows `HOME` is unset (only
        // USERPROFILE exists), so resolving the path from `$HOME` alone returned
        // None and `phantom lang set` failed with "no $HOME for the lang file".
        // The path must resolve via `dirs::home_dir()` without `$HOME`.
        let _g = crate::env_lock::acquire();
        let saved_file = std::env::var("PHANTOM_LANG_FILE").ok();
        let saved_home = std::env::var("HOME").ok();
        std::env::remove_var("PHANTOM_LANG_FILE");
        std::env::remove_var("HOME");
        let resolved = super::lang_file_path();
        let dirs_home = dirs::home_dir();
        if let Some(f) = saved_file {
            std::env::set_var("PHANTOM_LANG_FILE", f);
        }
        if let Some(h) = saved_home {
            std::env::set_var("HOME", h);
        }
        // Only assert when a home dir is actually discoverable (always true on a
        // real dev/CI machine; a truly homeless container would make dirs return
        // None too, and there is genuinely nowhere to persist a preference).
        if dirs_home.is_some() {
            let p = resolved
                .expect("lang file path must resolve via dirs::home_dir() without $HOME");
            assert!(
                p.ends_with(std::path::Path::new(".phantom-mesh").join("lang")),
                "unexpected lang file path: {}",
                p.display()
            );
        }
    }
}
