//! `ask_user` tool — pause agent execution and prompt the human for an
//! answer.
//!
//! Args:
//!   - `question` (string, required): the prompt shown to the user
//!   - `default`  (string, optional): returned when the user just presses
//!     Enter without typing
//!
//! Returns the user's input as a plain string. If stdin is not a TTY (e.g.
//! the agent is running headless via `phantom serve` or a piped job), the
//! tool returns `default` if set, otherwise the literal string
//! `"[ask_user unavailable: not running interactively]"` so the model knows
//! to choose a different approach.
//!
//! UX note: the prompt is rendered to stderr so it doesn't interleave with
//! tool stdout being captured by the model. The same convention as the
//! REPL's permission gate (see bin/phantom.rs).

use serde_json::Value;
use std::io::{IsTerminal, Write};

const NON_TTY_MSG: &str = "[ask_user unavailable: not running interactively]";

pub async fn ask(args: &Value) -> String {
    let question = args.get("question").and_then(|v| v.as_str()).unwrap_or("(no question)");
    let default  = args.get("default").and_then(|v| v.as_str()).map(|s| s.to_string());

    // Only prompt when stdin is a real terminal — otherwise we'd hang
    // forever waiting on EOF.
    if !std::io::stdin().is_terminal() {
        return default.unwrap_or_else(|| NON_TTY_MSG.to_string());
    }

    // Render the prompt. Use ANSI manually here (this tool isn't tied to
    // bin/phantom.rs's `colored()` helper).
    let dim    = "\x1b[2m";
    let yellow = "\x1b[33m";
    let cyan   = "\x1b[36m";
    let reset  = "\x1b[0m";

    eprintln!();
    eprintln!("  {}{}{} agent asks:", yellow, "⌘", reset);
    eprintln!("    {}{}{}", cyan, question, reset);
    if let Some(ref d) = default {
        eprint!("    {}(default: {}){}  > ", dim, d, reset);
    } else {
        eprint!("    > ");
    }
    let _ = std::io::stderr().flush();

    let mut buf = String::new();
    match std::io::stdin().read_line(&mut buf) {
        Ok(0) => default.unwrap_or_else(|| "[user closed stdin]".into()),
        Ok(_) => {
            let trimmed = buf.trim().to_string();
            if trimmed.is_empty() {
                default.unwrap_or_default()
            } else {
                trimmed
            }
        }
        Err(e) => format!("[ask_user error: {}]", e),
    }
}
