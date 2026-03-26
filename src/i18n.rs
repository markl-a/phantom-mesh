// i18n.rs — Internationalization framework for Phantom Mesh
//
// Provides a global `I18n` instance with locale-aware translation lookup,
// fallback to the default locale, and a `t!` macro for ergonomic access.

use std::collections::HashMap;
use std::sync::Mutex;

use once_cell::sync::Lazy;

// ---------------------------------------------------------------------------
// Built-in translations
// ---------------------------------------------------------------------------

fn builtin_en() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("welcome".into(), "Welcome to Phantom Mesh!".into());
    m.insert("error.not_found".into(), "Resource not found.".into());
    m.insert("error.permission_denied".into(), "Permission denied.".into());
    m.insert("tool.executing".into(), "Executing tool…".into());
    m.insert("tool.complete".into(), "Tool execution complete.".into());
    m.insert("hand.starting".into(), "Starting workflow…".into());
    m.insert("hand.phase_complete".into(), "Phase complete.".into());
    m.insert("hand.complete".into(), "Workflow complete.".into());
    m.insert("cluster.connected".into(), "Connected to cluster.".into());
    m.insert("cluster.disconnected".into(), "Disconnected from cluster.".into());
    m
}

fn builtin_zh_tw() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("welcome".into(), "歡迎使用 Phantom Mesh！".into());
    m.insert("error.not_found".into(), "找不到資源。".into());
    m.insert("error.permission_denied".into(), "存取遭拒。".into());
    m.insert("tool.executing".into(), "工具執行中…".into());
    m.insert("tool.complete".into(), "工具執行完成。".into());
    m.insert("hand.starting".into(), "工作流程啟動中…".into());
    m.insert("hand.phase_complete".into(), "階段完成。".into());
    m.insert("hand.complete".into(), "工作流程完成。".into());
    m.insert("cluster.connected".into(), "已連線至叢集。".into());
    m.insert("cluster.disconnected".into(), "已從叢集斷線。".into());
    m
}

fn builtin_ja() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("welcome".into(), "Phantom Mesh へようこそ！".into());
    m.insert("error.not_found".into(), "リソースが見つかりません。".into());
    m.insert("error.permission_denied".into(), "アクセスが拒否されました。".into());
    m.insert("tool.executing".into(), "ツール実行中…".into());
    m.insert("tool.complete".into(), "ツール実行完了。".into());
    m.insert("hand.starting".into(), "ワークフロー開始中…".into());
    m.insert("hand.phase_complete".into(), "フェーズ完了。".into());
    m.insert("hand.complete".into(), "ワークフロー完了。".into());
    m.insert("cluster.connected".into(), "クラスタに接続しました。".into());
    m.insert("cluster.disconnected".into(), "クラスタから切断されました。".into());
    m
}

fn builtin_zh_cn() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("welcome".into(), "欢迎使用 Phantom Mesh！".into());
    m.insert("error.not_found".into(), "找不到资源。".into());
    m.insert("error.permission_denied".into(), "访问被拒绝。".into());
    m.insert("tool.executing".into(), "工具执行中…".into());
    m.insert("tool.complete".into(), "工具执行完成。".into());
    m.insert("hand.starting".into(), "工作流程启动中…".into());
    m.insert("hand.phase_complete".into(), "阶段完成。".into());
    m.insert("hand.complete".into(), "工作流程完成。".into());
    m.insert("cluster.connected".into(), "已连接至集群。".into());
    m.insert("cluster.disconnected".into(), "已从集群断开。".into());
    m
}

fn builtin_ko() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("welcome".into(), "Phantom Mesh에 오신 것을 환영합니다!".into());
    m.insert("error.not_found".into(), "리소스를 찾을 수 없습니다.".into());
    m.insert("error.permission_denied".into(), "접근이 거부되었습니다.".into());
    m.insert("tool.executing".into(), "도구 실행 중…".into());
    m.insert("tool.complete".into(), "도구 실행 완료.".into());
    m.insert("hand.starting".into(), "워크플로우 시작 중…".into());
    m.insert("hand.phase_complete".into(), "단계 완료.".into());
    m.insert("hand.complete".into(), "워크플로우 완료.".into());
    m.insert("cluster.connected".into(), "클러스터에 연결되었습니다.".into());
    m.insert("cluster.disconnected".into(), "클러스터에서 연결이 끊어졌습니다.".into());
    m
}

// ---------------------------------------------------------------------------
// I18n struct
// ---------------------------------------------------------------------------

/// Locale-aware translation registry.
///
/// Stores translations as `locale -> key -> text`. When a key is missing in
/// the active locale the lookup falls back to `default_locale`. When it is
/// also absent there the key itself is returned so callers always receive a
/// non-empty string.
pub struct I18n {
    /// locale -> (key -> translation)
    translations: HashMap<String, HashMap<String, String>>,
    current_locale: String,
    default_locale: String,
}

impl I18n {
    /// Create a new registry with the given default locale.
    ///
    /// Built-in translations for `en` and `zh-TW` are pre-loaded; the
    /// current locale is initialised to `default_locale`.
    pub fn new(default_locale: &str) -> Self {
        let mut translations: HashMap<String, HashMap<String, String>> = HashMap::new();
        translations.insert("en".into(), builtin_en());
        translations.insert("zh-TW".into(), builtin_zh_tw());
        translations.insert("ja".into(), builtin_ja());
        translations.insert("zh-CN".into(), builtin_zh_cn());
        translations.insert("ko".into(), builtin_ko());

        Self {
            translations,
            current_locale: default_locale.to_string(),
            default_locale: default_locale.to_string(),
        }
    }

    /// Load (or replace) a full set of translations for `locale`.
    pub fn load_locale(&mut self, locale: &str, translations: HashMap<String, String>) {
        self.translations.insert(locale.to_string(), translations);
    }

    /// Look up `key` in the current locale.
    ///
    /// Fall-back chain:
    /// 1. current locale
    /// 2. default locale
    /// 3. the key string itself
    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        // Try current locale first.
        if let Some(map) = self.translations.get(&self.current_locale) {
            if let Some(v) = map.get(key) {
                return v.as_str();
            }
        }

        // Fall back to default locale (only when it differs from current).
        if self.current_locale != self.default_locale {
            if let Some(map) = self.translations.get(&self.default_locale) {
                if let Some(v) = map.get(key) {
                    return v.as_str();
                }
            }
        }

        // Last resort: return the key itself.
        key
    }

    /// Change the active locale.  The locale does not need to be pre-loaded;
    /// unknown locales will always fall back to the default.
    pub fn set_locale(&mut self, locale: &str) {
        self.current_locale = locale.to_string();
    }

    /// Return the currently active locale tag.
    pub fn current_locale(&self) -> &str {
        &self.current_locale
    }

    /// Return the default locale tag.
    pub fn default_locale(&self) -> &str {
        &self.default_locale
    }

    /// Check whether a locale has been loaded.
    pub fn has_locale(&self, locale: &str) -> bool {
        self.translations.contains_key(locale)
    }

    /// Return a sorted list of all loaded locale tags.
    pub fn available_locales(&self) -> Vec<String> {
        let mut locales: Vec<String> = self.translations.keys().cloned().collect();
        locales.sort();
        locales
    }

    /// Translate `key` for a specific locale (without changing the current locale).
    pub fn t_for_locale<'a>(&'a self, locale: &str, key: &'a str) -> &'a str {
        if let Some(map) = self.translations.get(locale) {
            if let Some(val) = map.get(key) {
                return val.as_str();
            }
        }
        // Fallback to default locale
        if let Some(map) = self.translations.get(&self.default_locale) {
            if let Some(val) = map.get(key) {
                return val.as_str();
            }
        }
        key
    }
}

// ---------------------------------------------------------------------------
// Global instance
// ---------------------------------------------------------------------------

/// Process-wide `I18n` instance, defaulting to `en`.
///
/// Obtain a guard with `I18N.lock().unwrap()` or use the [`t!`] macro.
pub static I18N: Lazy<Mutex<I18n>> = Lazy::new(|| Mutex::new(I18n::new("en")));

/// Translate `$key` using the global [`I18N`] instance.
///
/// Returns an owned `String` so that the mutex guard is not held across the
/// call site.
///
/// # Examples
/// ```
/// use phantom_mesh::t;
/// let msg: String = t!("welcome");
/// assert!(!msg.is_empty());
/// ```
#[macro_export]
macro_rules! t {
    ($key:expr) => {{
        $crate::i18n::I18N
            .lock()
            .expect("i18n mutex poisoned")
            .t($key)
            .to_owned()
    }};
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fr() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("welcome".into(), "Bienvenue dans Phantom Mesh !".into());
        m.insert("error.not_found".into(), "Ressource introuvable.".into());
        m
    }

    // -----------------------------------------------------------------------
    // load_locale + basic lookup
    // -----------------------------------------------------------------------

    #[test]
    fn test_load_and_get_translation() {
        let mut i18n = I18n::new("en");
        i18n.load_locale("fr", make_fr());

        i18n.set_locale("fr");
        assert_eq!(i18n.t("welcome"), "Bienvenue dans Phantom Mesh !");
        assert_eq!(i18n.t("error.not_found"), "Ressource introuvable.");
    }

    // -----------------------------------------------------------------------
    // Fallback to default locale when key missing in active locale
    // -----------------------------------------------------------------------

    #[test]
    fn test_fallback_to_default_locale() {
        let mut i18n = I18n::new("en");
        // "fr" locale has no "tool.executing" entry
        i18n.load_locale("fr", make_fr());
        i18n.set_locale("fr");

        // Should fall back to the English translation.
        assert_eq!(i18n.t("tool.executing"), "Executing tool…");
    }

    // -----------------------------------------------------------------------
    // Missing key returns the key itself
    // -----------------------------------------------------------------------

    #[test]
    fn test_missing_key_returns_key() {
        let i18n = I18n::new("en");
        assert_eq!(i18n.t("nonexistent.key"), "nonexistent.key");
    }

    // -----------------------------------------------------------------------
    // Switch locale
    // -----------------------------------------------------------------------

    #[test]
    fn test_switch_locale() {
        let mut i18n = I18n::new("en");

        // Default locale is "en".
        assert_eq!(i18n.current_locale(), "en");
        assert_eq!(i18n.t("welcome"), "Welcome to Phantom Mesh!");

        // Switch to zh-TW (built-in).
        i18n.set_locale("zh-TW");
        assert_eq!(i18n.current_locale(), "zh-TW");
        assert_eq!(i18n.t("welcome"), "歡迎使用 Phantom Mesh！");
    }

    // -----------------------------------------------------------------------
    // Built-in locales contain all required keys
    // -----------------------------------------------------------------------

    #[test]
    fn test_builtin_en_keys() {
        let i18n = I18n::new("en");
        let required = [
            "welcome",
            "error.not_found",
            "error.permission_denied",
            "tool.executing",
            "tool.complete",
            "hand.starting",
            "hand.phase_complete",
            "hand.complete",
            "cluster.connected",
            "cluster.disconnected",
        ];
        for key in &required {
            let val = i18n.t(key);
            assert_ne!(val, *key, "key '{}' is missing from en locale", key);
        }
    }

    #[test]
    fn test_builtin_zh_tw_keys() {
        let mut i18n = I18n::new("en");
        i18n.set_locale("zh-TW");
        let required = [
            "welcome",
            "error.not_found",
            "error.permission_denied",
            "tool.executing",
            "tool.complete",
            "hand.starting",
            "hand.phase_complete",
            "hand.complete",
            "cluster.connected",
            "cluster.disconnected",
        ];
        for key in &required {
            let val = i18n.t(key);
            assert_ne!(val, *key, "key '{}' is missing from zh-TW locale", key);
        }
    }

    // -----------------------------------------------------------------------
    // Unknown locale falls back to default without panicking
    // -----------------------------------------------------------------------

    #[test]
    fn test_unknown_locale_falls_back() {
        let mut i18n = I18n::new("en");
        i18n.set_locale("xx-YY"); // truly unknown locale
        // "welcome" exists in en (the default), so fallback kicks in.
        assert_eq!(i18n.t("welcome"), "Welcome to Phantom Mesh!");
        // Completely unknown key still returns the key itself.
        assert_eq!(i18n.t("no.such.key"), "no.such.key");
    }

    // -----------------------------------------------------------------------
    // load_locale replaces existing entries
    // -----------------------------------------------------------------------

    #[test]
    fn test_load_locale_replaces_existing() {
        let mut i18n = I18n::new("en");
        let mut custom = HashMap::new();
        custom.insert("welcome".into(), "Hi from custom!".into());
        i18n.load_locale("en", custom);

        assert_eq!(i18n.t("welcome"), "Hi from custom!");
        // Keys removed from the replacement map fall back to key string.
        assert_eq!(i18n.t("error.not_found"), "error.not_found");
    }

    // -----------------------------------------------------------------------
    // t! macro
    // -----------------------------------------------------------------------

    #[test]
    fn test_t_macro_returns_string() {
        // The global instance defaults to "en".
        let val = t!("welcome");
        // We only check that we get a non-empty String; the exact text depends
        // on whatever locale is set globally when this test runs.
        assert!(!val.is_empty());
    }
}
