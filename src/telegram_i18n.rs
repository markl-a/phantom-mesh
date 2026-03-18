// telegram_i18n.rs — Per-chat locale management for Telegram integration
//
// Provides `TelegramI18n` for per-chat locale storage, `/lang` command parsing,
// and automatic locale detection based on Unicode script heuristics.

use std::collections::HashMap;

use crate::i18n::I18N;

// ---------------------------------------------------------------------------
// TelegramI18n
// ---------------------------------------------------------------------------

/// Per-chat locale registry for the Telegram bot.
///
/// Each Telegram chat (identified by `chat_id: i64`) can have its own locale.
/// When no locale is set for a chat, the default locale (`"en"`) is used.
pub struct TelegramI18n {
    /// chat_id -> locale tag
    locales: HashMap<i64, String>,
    /// Fallback locale for chats that haven't set one.
    default_locale: String,
}

impl TelegramI18n {
    /// Create a new instance with `"en"` as the default locale.
    pub fn new() -> Self {
        Self {
            locales: HashMap::new(),
            default_locale: "en".to_string(),
        }
    }

    /// Create a new instance with a custom default locale.
    pub fn with_default(default_locale: &str) -> Self {
        Self {
            locales: HashMap::new(),
            default_locale: default_locale.to_string(),
        }
    }

    /// Set the locale for a specific chat.
    ///
    /// Returns `Ok(())` if the locale is recognized (loaded in the global I18n),
    /// or `Err` with a descriptive message if not.
    pub fn set_locale(&mut self, chat_id: i64, locale: &str) -> Result<(), String> {
        let i18n = I18N.lock().expect("i18n mutex poisoned");
        if !i18n.has_locale(locale) {
            let available = i18n.available_locales().join(", ");
            return Err(format!(
                "Unknown locale '{}'. Available: {}",
                locale, available
            ));
        }
        drop(i18n);
        self.locales.insert(chat_id, locale.to_string());
        Ok(())
    }

    /// Get the locale for a specific chat. Returns the default if not set.
    pub fn get_locale(&self, chat_id: i64) -> String {
        self.locales
            .get(&chat_id)
            .cloned()
            .unwrap_or_else(|| self.default_locale.clone())
    }

    /// Translate a key for a specific chat, using that chat's locale.
    ///
    /// Looks up the chat's locale, then delegates to the global `I18n`'s
    /// `t_for_locale` method.
    pub fn translate(&self, chat_id: i64, key: &str) -> String {
        let locale = self.get_locale(chat_id);
        let i18n = I18N.lock().expect("i18n mutex poisoned");
        i18n.t_for_locale(&locale, key).to_owned()
    }

    /// Remove a chat's locale override (reverts to default).
    pub fn clear_locale(&mut self, chat_id: i64) {
        self.locales.remove(&chat_id);
    }

    /// Return the number of chats with explicit locale overrides.
    pub fn active_overrides(&self) -> usize {
        self.locales.len()
    }
}

impl Default for TelegramI18n {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// /lang command parsing
// ---------------------------------------------------------------------------

/// Result of parsing a potential `/lang` command from Telegram message text.
#[derive(Debug, Clone, PartialEq)]
pub enum LangCommand {
    /// User issued `/lang <locale>` — switch to this locale.
    Switch(String),
    /// User issued `/lang` with no argument — list available locales.
    List,
    /// The message is not a `/lang` command.
    NotACommand,
}

/// Parse a Telegram message for `/lang` commands.
///
/// Recognized forms:
/// - `/lang`          -> `LangCommand::List`
/// - `/lang en`       -> `LangCommand::Switch("en")`
/// - `/lang zh-TW`    -> `LangCommand::Switch("zh-TW")`
/// - `/lang ja`       -> `LangCommand::Switch("ja")`
/// - anything else    -> `LangCommand::NotACommand`
pub fn parse_lang_command(text: &str) -> LangCommand {
    let trimmed = text.trim();

    // Must start with /lang (case-insensitive for the command itself).
    if !trimmed.starts_with("/lang") {
        return LangCommand::NotACommand;
    }

    // Could be "/language" or "/lang_foo" — only accept "/lang" or "/lang ".
    let rest = &trimmed[5..];
    if rest.is_empty() {
        return LangCommand::List;
    }

    // Next char must be whitespace to separate the command from the argument.
    if !rest.starts_with(char::is_whitespace) {
        return LangCommand::NotACommand;
    }

    let locale = rest.trim();
    if locale.is_empty() {
        return LangCommand::List;
    }

    LangCommand::Switch(locale.to_string())
}

// ---------------------------------------------------------------------------
// Locale detection heuristic
// ---------------------------------------------------------------------------

/// Detect a likely locale from free-form text using Unicode script analysis.
///
/// Returns `Some(locale_tag)` when the text is predominantly in a recognized
/// script, or `None` when detection is inconclusive (e.g. Latin-only text,
/// which could be any European language).
///
/// Detection rules (applied to non-whitespace, non-ASCII-punctuation chars):
/// - Hiragana or Katakana present -> `"ja"` (Japanese)
/// - Hangul present              -> `"ko"` (Korean)
/// - CJK Unified Ideographs      -> further analysis:
///   - If simplified-only indicators found -> `"zh-CN"`
///   - Default CJK -> `"zh-TW"` (Traditional Chinese as the existing default)
///
/// This is intentionally a fast heuristic, not a full language detector.
pub fn detect_locale(text: &str) -> Option<String> {
    let mut has_hiragana_katakana = false;
    let mut has_hangul = false;
    let mut has_cjk = false;
    let mut has_simplified_indicator = false;

    for ch in text.chars() {
        match ch {
            // Hiragana: U+3040..U+309F
            '\u{3040}'..='\u{309F}' => has_hiragana_katakana = true,
            // Katakana: U+30A0..U+30FF
            '\u{30A0}'..='\u{30FF}' => has_hiragana_katakana = true,
            // Hangul Syllables: U+AC00..U+D7AF
            '\u{AC00}'..='\u{D7AF}' => has_hangul = true,
            // Hangul Jamo: U+1100..U+11FF
            '\u{1100}'..='\u{11FF}' => has_hangul = true,
            // Hangul Compatibility Jamo: U+3130..U+318F
            '\u{3130}'..='\u{318F}' => has_hangul = true,
            // CJK Unified Ideographs: U+4E00..U+9FFF
            '\u{4E00}'..='\u{9FFF}' => {
                has_cjk = true;
                // Check for common simplified Chinese indicators.
                // These are chars that appear in simplified but not traditional.
                if matches!(ch, '\u{8FD9}' | '\u{7684}' | '\u{4E86}'
                    | '\u{4E0D}' | '\u{4EEC}' | '\u{8BF7}' | '\u{8FDB}'
                    | '\u{8BA9}' | '\u{5C06}' | '\u{5BF9}' | '\u{5173}'
                    | '\u{5F00}' | '\u{4E1C}' | '\u{8BA8}' | '\u{7EC4}'
                    | '\u{8BBA}' | '\u{6B22}' | '\u{8FBE}' | '\u{4E0E}'
                    | '\u{8D44}' | '\u{8BD5}' | '\u{8BC6}')
                {
                    has_simplified_indicator = true;
                }
            }
            _ => {}
        }
    }

    // Priority: Japanese > Korean > CJK
    // Japanese text almost always contains hiragana/katakana even when using kanji.
    if has_hiragana_katakana {
        return Some("ja".to_string());
    }
    if has_hangul {
        return Some("ko".to_string());
    }
    if has_cjk {
        if has_simplified_indicator {
            return Some("zh-CN".to_string());
        }
        return Some("zh-TW".to_string());
    }

    None
}

/// List of all supported locale tags.
pub fn supported_locales() -> Vec<String> {
    let i18n = I18N.lock().expect("i18n mutex poisoned");
    i18n.available_locales()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // TelegramI18n basic operations
    // -----------------------------------------------------------------------

    #[test]
    fn test_default_locale_is_en() {
        let ti = TelegramI18n::new();
        assert_eq!(ti.get_locale(12345), "en");
    }

    #[test]
    fn test_custom_default_locale() {
        let ti = TelegramI18n::with_default("ja");
        assert_eq!(ti.get_locale(12345), "ja");
    }

    #[test]
    fn test_set_and_get_locale() {
        let mut ti = TelegramI18n::new();
        ti.set_locale(100, "ja").unwrap();
        assert_eq!(ti.get_locale(100), "ja");
        // Other chats still get the default.
        assert_eq!(ti.get_locale(200), "en");
    }

    #[test]
    fn test_set_locale_invalid() {
        let mut ti = TelegramI18n::new();
        let result = ti.set_locale(100, "xx-INVALID");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Unknown locale"));
        assert!(err.contains("xx-INVALID"));
    }

    #[test]
    fn test_set_locale_all_five() {
        let mut ti = TelegramI18n::new();
        for locale in &["en", "zh-TW", "ja", "zh-CN", "ko"] {
            ti.set_locale(1, locale).unwrap();
            assert_eq!(ti.get_locale(1), *locale);
        }
    }

    #[test]
    fn test_translate_per_chat() {
        let mut ti = TelegramI18n::new();
        ti.set_locale(1, "ja").unwrap();
        ti.set_locale(2, "ko").unwrap();

        let ja_welcome = ti.translate(1, "welcome");
        let ko_welcome = ti.translate(2, "welcome");
        let en_welcome = ti.translate(3, "welcome"); // default

        assert!(ja_welcome.contains("ようこそ"), "ja welcome: {}", ja_welcome);
        assert!(ko_welcome.contains("환영"), "ko welcome: {}", ko_welcome);
        assert!(en_welcome.contains("Welcome"), "en welcome: {}", en_welcome);
    }

    #[test]
    fn test_translate_zh_cn() {
        let mut ti = TelegramI18n::new();
        ti.set_locale(1, "zh-CN").unwrap();
        let msg = ti.translate(1, "welcome");
        assert!(msg.contains("欢迎"), "zh-CN welcome: {}", msg);
    }

    #[test]
    fn test_clear_locale() {
        let mut ti = TelegramI18n::new();
        ti.set_locale(100, "ko").unwrap();
        assert_eq!(ti.get_locale(100), "ko");
        ti.clear_locale(100);
        assert_eq!(ti.get_locale(100), "en");
    }

    #[test]
    fn test_active_overrides() {
        let mut ti = TelegramI18n::new();
        assert_eq!(ti.active_overrides(), 0);
        ti.set_locale(1, "ja").unwrap();
        ti.set_locale(2, "ko").unwrap();
        assert_eq!(ti.active_overrides(), 2);
        ti.clear_locale(1);
        assert_eq!(ti.active_overrides(), 1);
    }

    // -----------------------------------------------------------------------
    // /lang command parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_lang_list() {
        assert_eq!(parse_lang_command("/lang"), LangCommand::List);
        assert_eq!(parse_lang_command("/lang "), LangCommand::List);
        assert_eq!(parse_lang_command("/lang   "), LangCommand::List);
    }

    #[test]
    fn test_parse_lang_switch() {
        assert_eq!(
            parse_lang_command("/lang en"),
            LangCommand::Switch("en".into())
        );
        assert_eq!(
            parse_lang_command("/lang zh-TW"),
            LangCommand::Switch("zh-TW".into())
        );
        assert_eq!(
            parse_lang_command("/lang ja"),
            LangCommand::Switch("ja".into())
        );
        assert_eq!(
            parse_lang_command("/lang zh-CN"),
            LangCommand::Switch("zh-CN".into())
        );
        assert_eq!(
            parse_lang_command("/lang ko"),
            LangCommand::Switch("ko".into())
        );
    }

    #[test]
    fn test_parse_lang_not_command() {
        assert_eq!(parse_lang_command("hello"), LangCommand::NotACommand);
        assert_eq!(parse_lang_command("/help"), LangCommand::NotACommand);
        assert_eq!(parse_lang_command("/language en"), LangCommand::NotACommand);
        assert_eq!(parse_lang_command(""), LangCommand::NotACommand);
    }

    #[test]
    fn test_parse_lang_with_whitespace() {
        assert_eq!(
            parse_lang_command("  /lang ja  "),
            LangCommand::Switch("ja".into())
        );
    }

    // -----------------------------------------------------------------------
    // Locale detection
    // -----------------------------------------------------------------------

    #[test]
    fn test_detect_japanese() {
        assert_eq!(detect_locale("こんにちは"), Some("ja".to_string()));
        assert_eq!(detect_locale("カタカナ"), Some("ja".to_string()));
        assert_eq!(detect_locale("漢字とひらがな"), Some("ja".to_string()));
    }

    #[test]
    fn test_detect_korean() {
        assert_eq!(detect_locale("안녕하세요"), Some("ko".to_string()));
        assert_eq!(detect_locale("한국어 텍스트"), Some("ko".to_string()));
    }

    #[test]
    fn test_detect_simplified_chinese() {
        // 这 (U+8FD9) is a simplified-only indicator
        assert_eq!(detect_locale("这是简体中文"), Some("zh-CN".to_string()));
        assert_eq!(detect_locale("请进来讨论"), Some("zh-CN".to_string()));
    }

    #[test]
    fn test_detect_traditional_chinese() {
        // Pure traditional characters without simplified indicators
        assert_eq!(detect_locale("繁體中文"), Some("zh-TW".to_string()));
    }

    #[test]
    fn test_detect_latin_returns_none() {
        assert_eq!(detect_locale("Hello world"), None);
        assert_eq!(detect_locale(""), None);
        assert_eq!(detect_locale("12345"), None);
    }

    #[test]
    fn test_detect_mixed_ja_priority() {
        // Japanese has priority when hiragana/katakana present alongside kanji
        assert_eq!(
            detect_locale("日本語のテキスト"),
            Some("ja".to_string())
        );
    }

    // -----------------------------------------------------------------------
    // supported_locales
    // -----------------------------------------------------------------------

    #[test]
    fn test_supported_locales_returns_five() {
        let locales = supported_locales();
        assert_eq!(locales.len(), 5);
        assert!(locales.contains(&"en".to_string()));
        assert!(locales.contains(&"ja".to_string()));
        assert!(locales.contains(&"ko".to_string()));
        assert!(locales.contains(&"zh-CN".to_string()));
        assert!(locales.contains(&"zh-TW".to_string()));
    }
}
