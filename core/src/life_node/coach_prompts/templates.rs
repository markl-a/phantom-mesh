//! Coach prompt templates — frozen at compile time, lint-gated.
//!
//! Each template is checked by [`super::lint::check`] before the LLM
//! second-pass (tomorrow-action) ships. This closes E003 acceptance #3:
//! "Coach prompt template stored in `core/src/life_node/coach_prompts/`
//! passes the shame-free check".
//!
//! Adding a new template? Drop it as a `pub const` here and add it to
//! the `ALL_TEMPLATES` list — the unit test below will lint it
//! automatically. The lint is a build-time invariant; a shame-pattern
//! sneaking into a template is now a test failure, not a silent
//! production ship.

/// System prompt for the `agent.coach` role. Frames the Coach as a
/// supportive observer that surfaces patterns in the user's own data —
/// never blame, never sarcasm, never disbelief.
pub const COACH_SYSTEM_PROMPT: &str = "\
你是 Phantom 個人教練節點。你看得到使用者今天捕捉的事件資料與分析。\n\
\n\
你的角色：\n\
- 觀察事實、指出與目標的關係，不評價人。\n\
- 用「今天的選擇跟昨天比、哪裡好/哪裡可以微調」的角度說話。\n\
- 提到行為時用具體例子，不用抽象標籤。\n\
\n\
你絕對不會：\n\
- 把使用者的選擇貼上「失敗」「偷懶」「沒救」之類的標籤。\n\
- 用反問或反話讓使用者感到羞愧。\n\
- 假設使用者今天的選擇是出於懶惰或缺乏意志力。\n\
\n\
語言：繁體中文，平實、簡潔、第二人稱。\n\
";

/// Tomorrow-action prompt template. Given today's daily review brief
/// (Markdown), the LLM is asked for exactly ONE smallest action that
/// moves the goal — per BIG-GOAL Operational Principle "shame-free" +
/// "smallest action that moves the goal".
///
/// Placeholder: `{BRIEF}` — replaced at runtime with the brief text.
pub const TOMORROW_ACTION_PROMPT: &str = "\
這是今天的事件摘要：\n\
\n\
{BRIEF}\n\
\n\
請給「明天一個最小的可做行動」。要求：\n\
1. 只給一個動作，不要清單。\n\
2. 行動要小到不會讓人想拖延（5-10 分鐘可完成的等級）。\n\
3. 跟今天事件裡某個 goal-tag 直接相關。\n\
4. 用一句話，繁體中文，第二人稱，不要評價今天做得好不好。\n\
\n\
直接給出那一個動作，不要前綴/後綴/標題。\n\
";

/// All shipped templates — used by the lint test below.
pub const ALL_TEMPLATES: &[(&str, &str)] = &[
    ("COACH_SYSTEM_PROMPT", COACH_SYSTEM_PROMPT),
    ("TOMORROW_ACTION_PROMPT", TOMORROW_ACTION_PROMPT),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::life_node::coach_prompts::lint;

    /// Every shipped template must pass the shame-free lint. A failure
    /// here means a template was edited to include a blame/sarcasm/
    /// judgment pattern — fix the template before the build can ship.
    #[test]
    fn all_templates_pass_shame_free_lint() {
        for (name, body) in ALL_TEMPLATES {
            if let Err(reason) = lint::check(body) {
                panic!(
                    "template `{}` failed shame-free lint:\n  {}\n\nbody was:\n{}",
                    name, reason, body
                );
            }
        }
    }

    /// Tomorrow-action template must have the `{BRIEF}` placeholder so
    /// callers know where to splice the daily review.
    #[test]
    fn tomorrow_action_has_brief_placeholder() {
        assert!(
            TOMORROW_ACTION_PROMPT.contains("{BRIEF}"),
            "TOMORROW_ACTION_PROMPT must contain `{{BRIEF}}` placeholder"
        );
    }
}
