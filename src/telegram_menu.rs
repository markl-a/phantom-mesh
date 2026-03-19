// telegram_menu.rs — Inline keyboard menus for Telegram Bot API
//
// Provides `InlineKeyboard` builder and pre-built menus for the Telegram bot:
// hand selector, status dashboard, confirmation dialog, provider selector.
// Also provides `parse_callback` to decode callback_data strings into typed actions.

use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// InlineButton
// ---------------------------------------------------------------------------

/// A single inline keyboard button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineButton {
    pub text: String,
    pub callback_data: String,
}

impl InlineButton {
    pub fn new(text: impl Into<String>, callback_data: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            callback_data: callback_data.into(),
        }
    }

    /// Serialize to the Telegram API JSON format.
    pub fn to_json(&self) -> Value {
        json!({
            "text": self.text,
            "callback_data": self.callback_data
        })
    }
}

// ---------------------------------------------------------------------------
// InlineKeyboard
// ---------------------------------------------------------------------------

/// An inline keyboard composed of rows of buttons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineKeyboard {
    pub rows: Vec<Vec<InlineButton>>,
}

impl InlineKeyboard {
    /// Create an empty keyboard.
    pub fn new() -> Self {
        Self { rows: Vec::new() }
    }

    /// Create a keyboard from pre-built rows.
    pub fn from_rows(rows: Vec<Vec<InlineButton>>) -> Self {
        Self { rows }
    }

    /// Add a row of buttons.
    pub fn add_row(&mut self, row: Vec<InlineButton>) -> &mut Self {
        self.rows.push(row);
        self
    }

    /// Serialize to the Telegram API `reply_markup` JSON format.
    ///
    /// Produces:
    /// ```json
    /// {
    ///   "inline_keyboard": [
    ///     [{"text": "A", "callback_data": "a"}, ...],
    ///     ...
    ///   ]
    /// }
    /// ```
    pub fn to_json(&self) -> Value {
        let rows: Vec<Value> = self
            .rows
            .iter()
            .map(|row| {
                let buttons: Vec<Value> = row.iter().map(|btn| btn.to_json()).collect();
                Value::Array(buttons)
            })
            .collect();

        json!({ "inline_keyboard": rows })
    }
}

impl Default for InlineKeyboard {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// CallbackAction
// ---------------------------------------------------------------------------

/// Parsed callback action from an inline keyboard button press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallbackAction {
    /// Run a hand by name.
    RunHand(String),
    /// Show a status panel (e.g. "cost", "revenue", "cluster", "health").
    ShowStatus(String),
    /// Confirm an action.
    Confirm,
    /// Cancel an action.
    Cancel,
    /// Select a provider by name.
    SelectProvider(String),
    /// Unrecognized callback data.
    Unknown(String),
}

/// Parse a `callback_data` string into a typed `CallbackAction`.
///
/// Callback data format:
/// - `hand:<name>`       -> `RunHand(name)`
/// - `status:<panel>`    -> `ShowStatus(panel)`
/// - `confirm`           -> `Confirm`
/// - `cancel`            -> `Cancel`
/// - `provider:<name>`   -> `SelectProvider(name)`
/// - anything else       -> `Unknown(raw)`
pub fn parse_callback(data: &str) -> CallbackAction {
    if let Some(name) = data.strip_prefix("hand:") {
        CallbackAction::RunHand(name.to_string())
    } else if let Some(panel) = data.strip_prefix("status:") {
        CallbackAction::ShowStatus(panel.to_string())
    } else if data == "confirm" {
        CallbackAction::Confirm
    } else if data == "cancel" {
        CallbackAction::Cancel
    } else if let Some(name) = data.strip_prefix("provider:") {
        CallbackAction::SelectProvider(name.to_string())
    } else {
        CallbackAction::Unknown(data.to_string())
    }
}

// ---------------------------------------------------------------------------
// Pre-built menus
// ---------------------------------------------------------------------------

/// Build a hand selector grid with up to 3 buttons per row.
///
/// Each button's `callback_data` is `hand:<name>`.
pub fn hand_selector(hand_names: &[&str]) -> InlineKeyboard {
    let mut rows: Vec<Vec<InlineButton>> = Vec::new();

    for chunk in hand_names.chunks(3) {
        let row: Vec<InlineButton> = chunk
            .iter()
            .map(|name| InlineButton::new(*name, format!("hand:{}", name)))
            .collect();
        rows.push(row);
    }

    InlineKeyboard::from_rows(rows)
}

/// Build the status dashboard menu.
///
/// Buttons: Cost, Revenue, Cluster, Health — arranged as a single row.
pub fn status_dashboard() -> InlineKeyboard {
    let row = vec![
        InlineButton::new("Cost", "status:cost"),
        InlineButton::new("Revenue", "status:revenue"),
        InlineButton::new("Cluster", "status:cluster"),
        InlineButton::new("Health", "status:health"),
    ];
    InlineKeyboard::from_rows(vec![row])
}

/// Build a confirmation dialog.
///
/// Includes the action description in the callback data for traceability.
/// Buttons: [Confirm] [Cancel]
pub fn confirmation(action: &str) -> InlineKeyboard {
    let _ = action; // reserved for future use in callback data
    let row = vec![
        InlineButton::new("\u{2705} Confirm", "confirm"),
        InlineButton::new("\u{274c} Cancel", "cancel"),
    ];
    InlineKeyboard::from_rows(vec![row])
}

/// Build a provider selector with one button per row.
///
/// Each button's `callback_data` is `provider:<name>`.
pub fn provider_selector(providers: &[&str]) -> InlineKeyboard {
    let rows: Vec<Vec<InlineButton>> = providers
        .iter()
        .map(|name| vec![InlineButton::new(*name, format!("provider:{}", name))])
        .collect();
    InlineKeyboard::from_rows(rows)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // InlineButton tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_inline_button_new() {
        let btn = InlineButton::new("Click me", "action:1");
        assert_eq!(btn.text, "Click me");
        assert_eq!(btn.callback_data, "action:1");
    }

    #[test]
    fn test_inline_button_to_json() {
        let btn = InlineButton::new("OK", "ok");
        let j = btn.to_json();
        assert_eq!(j["text"], "OK");
        assert_eq!(j["callback_data"], "ok");
    }

    // -----------------------------------------------------------------------
    // InlineKeyboard tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_empty_keyboard_to_json() {
        let kb = InlineKeyboard::new();
        let j = kb.to_json();
        let arr = j["inline_keyboard"].as_array().unwrap();
        assert!(arr.is_empty());
    }

    #[test]
    fn test_keyboard_add_row() {
        let mut kb = InlineKeyboard::new();
        kb.add_row(vec![InlineButton::new("A", "a"), InlineButton::new("B", "b")]);
        assert_eq!(kb.rows.len(), 1);
        assert_eq!(kb.rows[0].len(), 2);
    }

    #[test]
    fn test_keyboard_to_json_structure() {
        let kb = InlineKeyboard::from_rows(vec![
            vec![InlineButton::new("X", "x")],
            vec![InlineButton::new("Y", "y"), InlineButton::new("Z", "z")],
        ]);
        let j = kb.to_json();
        let rows = j["inline_keyboard"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].as_array().unwrap().len(), 1);
        assert_eq!(rows[1].as_array().unwrap().len(), 2);
        assert_eq!(rows[1][0]["text"], "Y");
        assert_eq!(rows[1][1]["callback_data"], "z");
    }

    // -----------------------------------------------------------------------
    // parse_callback tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_callback_run_hand() {
        assert_eq!(
            parse_callback("hand:seo_content"),
            CallbackAction::RunHand("seo_content".to_string())
        );
    }

    #[test]
    fn test_parse_callback_show_status() {
        assert_eq!(
            parse_callback("status:cost"),
            CallbackAction::ShowStatus("cost".to_string())
        );
    }

    #[test]
    fn test_parse_callback_confirm() {
        assert_eq!(parse_callback("confirm"), CallbackAction::Confirm);
    }

    #[test]
    fn test_parse_callback_cancel() {
        assert_eq!(parse_callback("cancel"), CallbackAction::Cancel);
    }

    #[test]
    fn test_parse_callback_select_provider() {
        assert_eq!(
            parse_callback("provider:gemini"),
            CallbackAction::SelectProvider("gemini".to_string())
        );
    }

    #[test]
    fn test_parse_callback_unknown() {
        assert_eq!(
            parse_callback("garbage_data"),
            CallbackAction::Unknown("garbage_data".to_string())
        );
    }

    // -----------------------------------------------------------------------
    // Pre-built menu tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_hand_selector_grid_layout() {
        let names = &["seo", "content", "outreach", "lead", "report"];
        let kb = hand_selector(names);
        // 5 items -> 2 rows (3 + 2)
        assert_eq!(kb.rows.len(), 2);
        assert_eq!(kb.rows[0].len(), 3);
        assert_eq!(kb.rows[1].len(), 2);
        assert_eq!(kb.rows[0][0].callback_data, "hand:seo");
        assert_eq!(kb.rows[1][1].text, "report");
    }

    #[test]
    fn test_hand_selector_empty() {
        let kb = hand_selector(&[]);
        assert!(kb.rows.is_empty());
    }

    #[test]
    fn test_hand_selector_exact_multiple_of_three() {
        let kb = hand_selector(&["a", "b", "c", "d", "e", "f"]);
        assert_eq!(kb.rows.len(), 2);
        assert_eq!(kb.rows[0].len(), 3);
        assert_eq!(kb.rows[1].len(), 3);
    }

    #[test]
    fn test_status_dashboard_buttons() {
        let kb = status_dashboard();
        assert_eq!(kb.rows.len(), 1);
        assert_eq!(kb.rows[0].len(), 4);
        let labels: Vec<&str> = kb.rows[0].iter().map(|b| b.text.as_str()).collect();
        assert_eq!(labels, vec!["Cost", "Revenue", "Cluster", "Health"]);
        assert_eq!(kb.rows[0][2].callback_data, "status:cluster");
    }

    #[test]
    fn test_confirmation_menu() {
        let kb = confirmation("delete_hand");
        assert_eq!(kb.rows.len(), 1);
        assert_eq!(kb.rows[0].len(), 2);
        assert_eq!(kb.rows[0][0].callback_data, "confirm");
        assert_eq!(kb.rows[0][1].callback_data, "cancel");
    }

    #[test]
    fn test_provider_selector_one_per_row() {
        let kb = provider_selector(&["gemini", "groq", "ollama"]);
        assert_eq!(kb.rows.len(), 3);
        for row in &kb.rows {
            assert_eq!(row.len(), 1);
        }
        assert_eq!(kb.rows[0][0].callback_data, "provider:gemini");
        assert_eq!(kb.rows[2][0].text, "ollama");
    }

    #[test]
    fn test_provider_selector_empty() {
        let kb = provider_selector(&[]);
        assert!(kb.rows.is_empty());
    }

    #[test]
    fn test_roundtrip_hand_selector_to_json_and_parse() {
        let names = &["seo_content", "market_intel"];
        let kb = hand_selector(names);
        let j = kb.to_json();

        // Extract callback_data from JSON and parse it
        let first_btn = &j["inline_keyboard"][0][0];
        let cb = first_btn["callback_data"].as_str().unwrap();
        assert_eq!(
            parse_callback(cb),
            CallbackAction::RunHand("seo_content".to_string())
        );
    }

    #[test]
    fn test_roundtrip_status_dashboard_to_json_and_parse() {
        let kb = status_dashboard();
        let j = kb.to_json();
        let revenue_btn = &j["inline_keyboard"][0][1];
        let cb = revenue_btn["callback_data"].as_str().unwrap();
        assert_eq!(
            parse_callback(cb),
            CallbackAction::ShowStatus("revenue".to_string())
        );
    }

    #[test]
    fn test_roundtrip_provider_selector_to_json_and_parse() {
        let kb = provider_selector(&["mistral"]);
        let j = kb.to_json();
        let btn = &j["inline_keyboard"][0][0];
        let cb = btn["callback_data"].as_str().unwrap();
        assert_eq!(
            parse_callback(cb),
            CallbackAction::SelectProvider("mistral".to_string())
        );
    }
}
