//! Sanctioned Claude-subscription access via the official `claude -p` CLI.
//!
//! Unlike the gray-zone [`super::claude_cli`] provider (which reuses the cached
//! OAuth token to hit `api.anthropic.com` directly — "routing subscription
//! credentials", against Anthropic ToS), this provider shells out to the
//! official Claude Code CLI in print mode. That draws from the account's
//! **Agent SDK credit pool**, which Anthropic explicitly sanctions for
//! third-party agents (Pro/Max/Team/Enterprise).
//!
//! Trade-off: `claude -p` runs Claude Code's *own* agent (with its own tools),
//! so this is a one-shot "delegate to Claude" completion rather than a raw
//! token-streaming LLM. It implements [`LlmProvider::complete`]; the streaming
//! methods are intentionally unsupported and the agent routes this provider
//! through the non-streaming path.

use crate::providers::llm_provider::{BuildRequestOpts, BuildRequestParts, LlmProvider};
use crate::providers::traits::{ChatMessage, ProviderError};
use async_trait::async_trait;
use serde_json::Value;

/// Locate the `claude` binary (PATH, then the common `~/.local/bin` install).
pub fn which_claude() -> Option<String> {
    // `which` is Unix-only; Windows ships `where` instead.
    let finder = if cfg!(windows) { "where" } else { "which" };
    if let Ok(out) = std::process::Command::new(finder).arg("claude").output() {
        if out.status.success() {
            // `where` can print several matches (one per line); take the first.
            if let Some(p) = String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
            {
                return Some(p.to_string());
            }
        }
    }
    let home = super::credential_scanner::home_dir_lenient()?;
    let mut cands = vec![
        home.join(".local").join("bin").join("claude"),
        home.join(".claude").join("local").join("claude"),
    ];
    if cfg!(windows) {
        // The native Windows installer drops `claude.exe` under ~/.local/bin.
        cands.push(home.join(".local").join("bin").join("claude.exe"));
    }
    for cand in cands {
        if cand.exists() {
            return Some(cand.to_string_lossy().into_owned());
        }
    }
    None
}

/// True when the official Claude Code CLI is installed (sanctioned path usable).
pub fn claude_agent_available() -> bool {
    which_claude().is_some()
}

/// Parsed outcome of a `claude -p --output-format json` run.
#[derive(Debug, Clone)]
pub struct ClaudePrintResult {
    pub text: String,
    pub is_error: bool,
    pub session_id: Option<String>,
    pub cost_usd: Option<f64>,
}

/// Parse the JSON object `claude -p --output-format json` prints.
pub fn parse_print_json(stdout: &str) -> Result<ClaudePrintResult, ProviderError> {
    let v: Value = serde_json::from_str(stdout.trim()).map_err(|e| {
        ProviderError::Unknown(format!(
            "claude -p: could not parse JSON output ({e}); first 200 chars: {}",
            stdout.chars().take(200).collect::<String>()
        ))
    })?;
    let is_error = v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false)
        || v.get("subtype").and_then(|s| s.as_str()) == Some("error");
    let text = v
        .get("result")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    Ok(ClaudePrintResult {
        text,
        is_error,
        session_id: v.get("session_id").and_then(|s| s.as_str()).map(String::from),
        cost_usd: v.get("total_cost_usd").and_then(|c| c.as_f64()),
    })
}

/// Render a `[ChatMessage]` conversation into a single prompt string for
/// `claude -p` (which takes one prompt). System messages are returned
/// separately so the caller can pass them via `--append-system-prompt`.
fn render_prompt(messages: &[ChatMessage]) -> (String, String) {
    let mut system = String::new();
    let mut convo = String::new();
    for m in messages {
        match m.role.as_str() {
            "system" => {
                if !system.is_empty() {
                    system.push_str("\n\n");
                }
                system.push_str(&m.content);
            }
            "assistant" => {
                convo.push_str("\n\nAssistant: ");
                convo.push_str(&m.content);
            }
            _ => {
                convo.push_str("\n\nUser: ");
                convo.push_str(&m.content);
            }
        }
    }
    (system, convo.trim().to_string())
}

/// Render OpenAI-style `Value` messages (the shape `agent.rs` carries) into a
/// `(system, prompt)` pair for `claude -p`. Handles string and array (text
/// parts) content.
pub fn render_value_messages(messages: &[serde_json::Value]) -> (String, String) {
    let text_of = |m: &serde_json::Value| -> String {
        if let Some(s) = m.get("content").and_then(|c| c.as_str()) {
            return s.to_string();
        }
        m.get("content")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default()
    };
    let mut system = String::new();
    let mut convo = String::new();
    for m in messages {
        let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        let content = text_of(m);
        match role {
            "system" => {
                if !system.is_empty() {
                    system.push_str("\n\n");
                }
                system.push_str(&content);
            }
            "assistant" => {
                convo.push_str("\n\nAssistant: ");
                convo.push_str(&content);
            }
            _ => {
                convo.push_str("\n\nUser: ");
                convo.push_str(&content);
            }
        }
    }
    (system, convo.trim().to_string())
}

/// Run `claude -p` once (non-interactive), feeding the prompt via stdin so long
/// conversations don't hit argv limits.
pub async fn run_claude_print(
    prompt: &str,
    model: Option<&str>,
    system: Option<&str>,
) -> Result<ClaudePrintResult, ProviderError> {
    let bin = which_claude().ok_or_else(|| {
        ProviderError::Unknown(
            "claude CLI not found — install Claude Code and run `claude login` (sanctioned \
             subscription path uses the Agent SDK credit pool)"
                .into(),
        )
    })?;

    let mut cmd = tokio::process::Command::new(bin);
    cmd.arg("-p")
        .arg(prompt) // pass as arg (works headlessly; stdin left empty)
        .arg("--output-format")
        .arg("json")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // Run from a neutral dir so `claude -p` doesn't load/explore whatever
    // project spectyn is running in (that was minutes-slow in a big repo and
    // is surprising for a plain LLM call).
    if let Some(home) = super::credential_scanner::home_dir_lenient() {
        cmd.current_dir(home);
    }
    if let Some(m) = model.filter(|s| !s.is_empty()) {
        cmd.arg("--model").arg(m);
    }
    if let Some(s) = system.filter(|s| !s.is_empty()) {
        cmd.arg("--append-system-prompt").arg(s);
    }
    // If spectyn itself was launched from inside a Claude Code session, these
    // markers are inherited; passing them on makes the spawned `claude` think
    // it's nested and hang. Strip them so the print run is clean. (We keep
    // CLAUDE_CODE_OAUTH_TOKEN — that's auth, not a nesting marker.)
    for var in ["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SSE_PORT"] {
        cmd.env_remove(var);
    }

    let child = cmd
        .spawn()
        .map_err(|e| ProviderError::Unknown(format!("spawn `claude` failed: {e}")))?;
    let out = match tokio::time::timeout(
        std::time::Duration::from_secs(180),
        child.wait_with_output(),
    )
    .await
    {
        Ok(r) => r.map_err(|e| ProviderError::Unknown(format!("`claude -p` wait failed: {e}")))?,
        Err(_) => {
            return Err(ProviderError::Unknown(
                "`claude -p` timed out after 180s".into(),
            ))
        }
    };

    if !out.status.success() && out.stdout.is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(ProviderError::Unknown(format!(
            "`claude -p` exited {}: {}",
            out.status,
            err.chars().take(300).collect::<String>()
        )));
    }
    let result = parse_print_json(&String::from_utf8_lossy(&out.stdout))?;
    if result.is_error {
        return Err(ProviderError::Unknown(format!(
            "claude -p reported an error: {}",
            result.text
        )));
    }
    Ok(result)
}

/// Provider that delegates to the official `claude -p` CLI (subscription via
/// Agent SDK credits). Non-streaming: only [`complete`](LlmProvider::complete).
pub(crate) struct ClaudeAgentProvider;

impl ClaudeAgentProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl LlmProvider for ClaudeAgentProvider {
    async fn stream(
        &self,
        _api_key: &str,
        _model: &str,
        _messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> Result<reqwest::Response, ProviderError> {
        Err(ProviderError::Unknown(
            "claude_agent is non-streaming; the agent routes it through complete()".into(),
        ))
    }

    async fn complete(
        &self,
        _api_key: &str,
        model: &str,
        messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> Result<(ChatMessage, serde_json::Value), ProviderError> {
        let (system, prompt) = render_prompt(messages);
        let res = run_claude_print(
            &prompt,
            Some(model).filter(|s| !s.is_empty()),
            (!system.is_empty()).then_some(system.as_str()),
        )
        .await?;
        let msg = ChatMessage {
            role: "assistant".into(),
            content: res.text,
            tool_calls: None,
        };
        let raw = serde_json::json!({
            "claude_agent": true,
            "session_id": res.session_id,
            "total_cost_usd": res.cost_usd,
        });
        Ok((msg, raw))
    }

    fn provider_type(&self) -> &'static str {
        "claude_agent"
    }

    fn build_stream_request(
        &self,
        _opts: &BuildRequestOpts<'_>,
    ) -> Result<BuildRequestParts, ProviderError> {
        Err(ProviderError::Unknown(
            "claude_agent is non-streaming; route via complete()".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_success_result() {
        let json = r#"{"type":"result","subtype":"success","is_error":false,
            "result":"hello world","session_id":"abc","total_cost_usd":0.046}"#;
        let r = parse_print_json(json).unwrap();
        assert_eq!(r.text, "hello world");
        assert!(!r.is_error);
        assert_eq!(r.session_id.as_deref(), Some("abc"));
        assert_eq!(r.cost_usd, Some(0.046));
    }

    #[test]
    fn flags_error_subtype() {
        let json = r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":"stopped"}"#;
        let r = parse_print_json(json).unwrap();
        assert!(r.is_error);
    }

    #[test]
    fn render_prompt_splits_system() {
        let msgs = vec![
            ChatMessage { role: "system".into(), content: "be terse".into(), tool_calls: None },
            ChatMessage { role: "user".into(), content: "hi".into(), tool_calls: None },
        ];
        let (system, prompt) = render_prompt(&msgs);
        assert_eq!(system, "be terse");
        assert!(prompt.contains("User: hi"));
        assert!(!prompt.contains("be terse"));
    }

    /// Live sanity check against the real CLI + subscription. Ignored by default
    /// (needs `claude login` + spends a few cents of Agent SDK credit).
    #[tokio::test]
    #[ignore]
    async fn live_claude_print() {
        let r = run_claude_print("Reply with exactly: pong", None, None)
            .await
            .unwrap();
        assert!(r.text.to_lowercase().contains("pong"), "got: {}", r.text);
    }
}
