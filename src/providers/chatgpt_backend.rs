//! ChatGPT Backend Provider: uses Codex CLI subprocess for ChatGPT Plus subscription access.
//!
//! Primary mode: runs `codex exec --json` as a subprocess, capturing JSONL streaming events.
//! This bypasses the chatgpt.com sentinel/anti-bot pipeline by using the official Codex CLI
//! which handles WebSocket connections to api.openai.com natively.
//!
//! Also includes REST backend-api message format translation functions for future use
//! when/if direct REST access becomes viable.

use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use futures_util::Stream;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, warn};

use uuid::Uuid;

use super::codex::CodexTokenManager;
use super::traits::{
    ChatMessage, ChatResponse, ProviderCapabilities, Provider, StreamChunk, TokenUsage,
};

// ── Message format translation (for future REST backend-api use) ────────────

/// Convert a single ChatMessage to the ChatGPT backend-api message format.
fn chatmessage_to_backend(msg: &ChatMessage) -> Value {
    let id = Uuid::new_v4().to_string();
    json!({
        "id": id,
        "author": { "role": msg.role },
        "content": {
            "content_type": "text",
            "parts": [msg.content]
        }
    })
}

/// Convert a slice of ChatMessages to backend-api format, returning (messages, parent_message_id).
fn build_backend_messages(messages: &[ChatMessage]) -> (Vec<Value>, String) {
    let parent_id = Uuid::new_v4().to_string();
    let backend_msgs: Vec<Value> = messages.iter().map(chatmessage_to_backend).collect();
    (backend_msgs, parent_id)
}

/// Parse a single SSE data line from the backend-api response.
fn parse_backend_sse_line(line: &str) -> (Option<String>, bool, Option<String>) {
    let line = line.trim();

    if line == "[DONE]" {
        return (None, true, None);
    }

    let parsed: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return (None, false, None),
    };

    let conv_id = parsed["conversation_id"]
        .as_str()
        .map(|s| s.to_string());

    let status = parsed["message"]["status"].as_str().unwrap_or("");
    let is_done = status == "finished_successfully";

    let content = parsed["message"]["content"]["parts"]
        .as_array()
        .and_then(|parts| parts.first())
        .and_then(|p| p.as_str())
        .map(|s| s.to_string());

    (content, is_done, conv_id)
}

// ── Codex CLI subprocess helpers ────────────────────────────────────────────

/// Build a prompt string from ChatMessages for the Codex CLI.
fn messages_to_prompt(messages: &[ChatMessage]) -> String {
    let mut parts = Vec::new();
    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                parts.push(format!("[System instruction: {}]", msg.content));
            }
            "user" => {
                parts.push(msg.content.clone());
            }
            "assistant" => {
                parts.push(format!("[Previous assistant response: {}]", msg.content));
            }
            "tool" => {
                parts.push(format!("[Tool result: {}]", msg.content));
            }
            _ => {
                parts.push(msg.content.clone());
            }
        }
    }
    parts.join("\n\n")
}

/// Parse Codex CLI JSONL output into (response_text, token_usage).
///
/// Expected JSONL events:
/// - `{"type":"thread.started","thread_id":"..."}`
/// - `{"type":"turn.started"}`
/// - `{"type":"item.completed","item":{"id":"...","type":"agent_message","text":"..."}}`
/// - `{"type":"turn.completed","usage":{"input_tokens":N,"output_tokens":N,...}}`
fn parse_codex_jsonl(output: &str) -> Result<(String, Option<TokenUsage>)> {
    let mut response_text = String::new();
    let mut usage = None;

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parsed: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let event_type = parsed["type"].as_str().unwrap_or("");

        match event_type {
            "item.completed" => {
                if let Some(text) = parsed["item"]["text"].as_str() {
                    response_text = text.to_string();
                }
            }
            "turn.completed" => {
                if let Some(u) = parsed.get("usage") {
                    let input = u["input_tokens"].as_u64().unwrap_or(0) as u32;
                    let output_t = u["output_tokens"].as_u64().unwrap_or(0) as u32;
                    usage = Some(TokenUsage {
                        prompt_tokens: input,
                        completion_tokens: output_t,
                        total_tokens: input + output_t,
                    });
                }
            }
            _ => {} // thread.started, turn.started, etc.
        }
    }

    if response_text.is_empty() {
        anyhow::bail!("No response text in Codex CLI output");
    }

    Ok((response_text, usage))
}

/// Build a sanitized environment for subprocess execution.
/// Strips sensitive keys (API tokens, secrets) to prevent leakage.
/// Reference: OpenFang subprocess env stripping pattern.
fn safe_subprocess_env() -> Vec<(String, String)> {
    // Keys that are safe to pass through
    let sensitive_prefixes = [
        "ANTHROPIC_", "OPENAI_", "GEMINI_", "GROQ_", "DEEPSEEK_",
        "SERPER_", "TAVILY_", "BRAVE_", "EXA_", "STRIPE_",
        "CLAWTEX_SECRET", "CLAWTEX_HUB_KEY",
        "TWITTER_", "SMTP_", "EMAIL_",
    ];
    let sensitive_exact = [
        "SECRET_KEY", "API_KEY", "ACCESS_TOKEN", "REFRESH_TOKEN",
        "BOT_TOKEN", "WEBHOOK_SECRET",
    ];

    std::env::vars()
        .filter(|(key, _)| {
            let upper = key.to_uppercase();
            // Keep the key if it does NOT match any sensitive pattern
            !sensitive_prefixes.iter().any(|p| upper.starts_with(p))
                && !sensitive_exact.iter().any(|s| upper.contains(s))
        })
        .collect()
}

/// Codex binary resolution result.
/// On Windows we bypass `.cmd` wrappers (they break on Unicode args)
/// and call `node codex.js` directly.
struct CodexBinary {
    program: String,
    /// If set, this JS file is prepended to args (for `node <script>` invocation)
    script: Option<String>,
}

/// Find the `codex` executable, resolving `.cmd` wrappers on Windows.
fn find_codex_binary() -> Option<CodexBinary> {
    if cfg!(windows) {
        // 1. Try to resolve codex.cmd → extract the JS entry point → call node directly
        if let Ok(output) = std::process::Command::new("where")
            .arg("codex.cmd")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
        {
            if output.status.success() {
                let cmd_path = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                if !cmd_path.is_empty() {
                    // Read .cmd to find the JS entry point
                    if let Ok(content) = std::fs::read_to_string(&cmd_path) {
                        // npm .cmd wrappers contain a line like:
                        //   "%_prog%"  "%dp0%\node_modules\@openai\codex\bin\codex.js" %*
                        for line in content.lines() {
                            if let Some(start) = line.find("node_modules") {
                                // Extract path between quotes or until %*
                                let rest = &line[start..];
                                let end = rest.find('"')
                                    .or_else(|| rest.find('%'))
                                    .unwrap_or(rest.len());
                                let rel_path = rest[..end].trim();
                                let dir = std::path::Path::new(&cmd_path).parent()?;
                                let js_path = dir.join(rel_path);
                                if js_path.exists() {
                                    return Some(CodexBinary {
                                        program: "node".to_string(),
                                        script: Some(js_path.to_string_lossy().to_string()),
                                    });
                                }
                            }
                        }
                    }
                    // Fallback: use codex.cmd (may fail on Unicode)
                    return Some(CodexBinary {
                        program: cmd_path,
                        script: None,
                    });
                }
            }
        }
        None
    } else {
        // Unix: check for `codex` in PATH
        let check = std::process::Command::new("which")
            .arg("codex")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if let Ok(s) = check {
            if s.success() {
                return Some(CodexBinary {
                    program: "codex".to_string(),
                    script: None,
                });
            }
        }
        None
    }
}

// ── ChatGptBackendProvider ───────────────────────────────────────────────────

/// Provider that uses the Codex CLI (`codex exec --json`) as a subprocess
/// to access ChatGPT models via the Plus subscription.
///
/// Falls back to REST backend-api if configured (future, requires sentinel pipeline).
pub struct ChatGptBackendProvider {
    token_manager: Arc<CodexTokenManager>,
    default_model_name: String,
}

impl ChatGptBackendProvider {
    pub fn new(token_manager: Arc<CodexTokenManager>) -> Self {
        Self {
            token_manager,
            default_model_name: "gpt-5.4".to_string(),
        }
    }

    pub fn with_model(token_manager: Arc<CodexTokenManager>, model: &str) -> Self {
        Self {
            token_manager,
            default_model_name: model.to_string(),
        }
    }

    /// Build the JSON request body for the backend-api (kept for future REST use).
    pub fn build_request_body(&self, messages: &[ChatMessage], model: &str) -> Value {
        let (backend_msgs, parent_id) = build_backend_messages(messages);
        json!({
            "action": "next",
            "model": model,
            "parent_message_id": parent_id,
            "messages": backend_msgs,
        })
    }

    /// Parse a complete SSE response body (kept for future REST use).
    fn parse_full_sse_response(body: &str) -> Result<ChatResponse> {
        let mut final_content = String::new();

        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            let data = match line.strip_prefix("data: ") {
                Some(d) => d,
                None => continue,
            };
            let (content, is_done, _) = parse_backend_sse_line(data);
            if let Some(c) = content {
                final_content = c;
            }
            if is_done {
                break;
            }
        }

        if final_content.is_empty() {
            anyhow::bail!("No content in ChatGPT backend response");
        }

        Ok(ChatResponse {
            message: ChatMessage {
                role: "assistant".to_string(),
                content: final_content,
                tool_calls: None,
                tool_call_id: None,
            },
            usage: None,
        })
    }

    /// Run the Codex CLI subprocess and return JSONL output.
    async fn run_codex_cli(&self, prompt: &str, model: &str) -> Result<String> {
        let codex_bin = find_codex_binary()
            .ok_or_else(|| anyhow::anyhow!("Codex CLI not found. Install with: npm install -g @openai/codex"))?;

        debug!("Running Codex CLI: model={}, prompt_len={}", model, prompt.len());

        let mut args: Vec<String> = Vec::new();
        // If resolved to node + script, prepend the JS file
        if let Some(ref script) = codex_bin.script {
            args.push(script.clone());
        }
        args.push("exec".to_string());
        args.push("--json".to_string());
        // Only pass --model if explicitly set (not empty/auto/default)
        if !model.is_empty() && model != "auto" && model != "default" {
            args.push("--model".to_string());
            args.push(model.to_string());
        }
        args.push("--skip-git-repo-check".to_string());
        args.push("--ephemeral".to_string());

        // Windows has ~32K command-line limit (OS error 206 when exceeded).
        // For long prompts, write to a temp file and pass via stdin.
        let use_stdin = prompt.len() > 8000;
        if !use_stdin {
            args.push(prompt.to_string());
        }

        let mut cmd = tokio::process::Command::new(&codex_bin.program);
        cmd.args(&args)
            .env_clear()
            .envs(safe_subprocess_env())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let output = if use_stdin {
            debug!("Codex CLI: prompt too long ({}), using stdin", prompt.len());
            cmd.stdin(std::process::Stdio::piped());
            let mut child = cmd.spawn()
                .map_err(|e| anyhow::anyhow!("Failed to spawn Codex CLI: {}", e))?;
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(prompt.as_bytes()).await;
                drop(stdin);
            }
            // 5 minute timeout
            tokio::time::timeout(
                std::time::Duration::from_secs(300),
                child.wait_with_output(),
            )
            .await
            .map_err(|_| anyhow::anyhow!("Codex CLI timed out after 300s"))?
            .map_err(|e| anyhow::anyhow!("Failed to run Codex CLI: {}", e))?
        } else {
            // 5 minute timeout
            tokio::time::timeout(
                std::time::Duration::from_secs(300),
                cmd.output(),
            )
            .await
            .map_err(|_| anyhow::anyhow!("Codex CLI timed out after 300s"))?
            .map_err(|e| anyhow::anyhow!("Failed to run Codex CLI: {}", e))?
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            warn!("Codex CLI stderr: {}", stderr);
            // Try to extract error from JSONL or stderr
            if !stdout.is_empty() {
                // Check for error events in JSONL
                for line in stdout.lines() {
                    if let Ok(v) = serde_json::from_str::<Value>(line) {
                        if v["type"].as_str() == Some("error") {
                            let msg = v["message"].as_str().unwrap_or("Unknown error");
                            anyhow::bail!("Codex CLI error: {}", msg);
                        }
                    }
                }
            }
            anyhow::bail!(
                "Codex CLI exited with status {}: {}",
                output.status,
                if stderr.is_empty() { &stdout } else { &stderr }
            );
        }

        Ok(stdout)
    }
}

#[async_trait]
impl Provider for ChatGptBackendProvider {
    fn name(&self) -> &str {
        "chatgpt_backend"
    }

    fn default_model(&self) -> &str {
        &self.default_model_name
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            native_tools: false,
            vision: false, // Codex CLI doesn't support image input easily
        }
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        _tools: &[Value],
        model: &str,
    ) -> Result<ChatResponse> {
        let model = if model.is_empty() { &self.default_model_name } else { model };
        let prompt = messages_to_prompt(messages);

        let jsonl_output = self.run_codex_cli(&prompt, model).await?;
        let (text, usage) = parse_codex_jsonl(&jsonl_output)?;

        Ok(ChatResponse {
            message: ChatMessage {
                role: "assistant".to_string(),
                content: text,
                tool_calls: None,
                tool_call_id: None,
            },
            usage,
        })
    }

    async fn stream_chat(
        &self,
        messages: &[ChatMessage],
        _tools: &[Value],
        model: &str,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let model_str = if model.is_empty() {
            self.default_model_name.clone()
        } else {
            model.to_string()
        };
        let prompt = messages_to_prompt(messages);

        let codex_bin = find_codex_binary()
            .ok_or_else(|| anyhow::anyhow!("Codex CLI not found"))?;

        let (tx, rx) = mpsc::channel::<Result<StreamChunk>>(64);

        let bin_program = codex_bin.program;
        let bin_script = codex_bin.script;

        tokio::spawn(async move {
            let mut args: Vec<String> = Vec::new();
            if let Some(ref script) = bin_script {
                args.push(script.clone());
            }
            args.push("exec".to_string());
            args.push("--json".to_string());
            if !model_str.is_empty() && model_str != "auto" && model_str != "default" {
                args.push("--model".to_string());
                args.push(model_str.clone());
            }
            args.push("--skip-git-repo-check".to_string());
            args.push("--ephemeral".to_string());

            let use_stdin = prompt.len() > 8000;
            if !use_stdin {
                args.push(prompt.clone());
            }

            let mut cmd = tokio::process::Command::new(&bin_program);
            cmd.args(&args)
                .env_clear()
                .envs(safe_subprocess_env())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null());
            if use_stdin {
                cmd.stdin(std::process::Stdio::piped());
            }

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(Err(anyhow::anyhow!("Spawn failed: {}", e))).await;
                    return;
                }
            };

            // Feed prompt via stdin for long prompts
            if use_stdin {
                if let Some(mut stdin) = child.stdin.take() {
                    use tokio::io::AsyncWriteExt;
                    let _ = stdin.write_all(prompt.as_bytes()).await;
                    drop(stdin);
                }
            }

            let stdout = match child.stdout.take() {
                Some(s) => s,
                None => {
                    let _ = tx.send(Err(anyhow::anyhow!("No stdout"))).await;
                    return;
                }
            };

            use tokio::io::{AsyncBufReadExt, BufReader};
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut usage = None;

            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }

                let parsed: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let event_type = parsed["type"].as_str().unwrap_or("");

                match event_type {
                    "item.completed" => {
                        if let Some(text) = parsed["item"]["text"].as_str() {
                            let _ = tx
                                .send(Ok(StreamChunk::ContentDelta(text.to_string())))
                                .await;
                        }
                    }
                    "turn.completed" => {
                        if let Some(u) = parsed.get("usage") {
                            let input = u["input_tokens"].as_u64().unwrap_or(0) as u32;
                            let output_t = u["output_tokens"].as_u64().unwrap_or(0) as u32;
                            usage = Some(TokenUsage {
                                prompt_tokens: input,
                                completion_tokens: output_t,
                                total_tokens: input + output_t,
                            });
                        }
                    }
                    "error" => {
                        let msg = parsed["message"]
                            .as_str()
                            .unwrap_or("Codex error")
                            .to_string();
                        let _ = tx.send(Err(anyhow::anyhow!("{}", msg))).await;
                        return;
                    }
                    _ => {}
                }
            }

            let _ = tx.send(Ok(StreamChunk::Done { usage })).await;
            let _ = child.wait().await;
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn is_alive(&self) -> bool {
        // Check if Codex CLI exists and auth is valid
        if find_codex_binary().is_none() {
            return false;
        }
        match self.token_manager.get_credential_clone().await {
            Some(cred) => !cred.access_token.is_empty(),
            None => false,
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Message format translation tests (REST backend-api) ──

    #[test]
    fn test_user_message_to_backend() {
        let msg = ChatMessage {
            role: "user".to_string(),
            content: "Hello world".to_string(),
            tool_calls: None,
            tool_call_id: None,
        };
        let result = chatmessage_to_backend(&msg);
        assert_eq!(result["author"]["role"], "user");
        assert_eq!(result["content"]["content_type"], "text");
        assert_eq!(result["content"]["parts"][0], "Hello world");
        assert!(result["id"].as_str().unwrap().len() > 10);
    }

    #[test]
    fn test_system_message_to_backend() {
        let msg = ChatMessage {
            role: "system".to_string(),
            content: "You are helpful".to_string(),
            tool_calls: None,
            tool_call_id: None,
        };
        let result = chatmessage_to_backend(&msg);
        assert_eq!(result["author"]["role"], "system");
        assert_eq!(result["content"]["parts"][0], "You are helpful");
    }

    #[test]
    fn test_build_backend_messages_returns_parent_id() {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "Hi".to_string(),
            tool_calls: None,
            tool_call_id: None,
        }];
        let (msgs, parent_id) = build_backend_messages(&messages);
        assert_eq!(msgs.len(), 1);
        assert!(!parent_id.is_empty());
    }

    #[test]
    fn test_parse_sse_content_delta() {
        let line = r#"{"message":{"id":"msg-abc","author":{"role":"assistant"},"content":{"content_type":"text","parts":["Hello"]},"status":"in_progress"},"conversation_id":"conv-123"}"#;
        let (content, done, conv_id) = parse_backend_sse_line(line);
        assert_eq!(content, Some("Hello".to_string()));
        assert!(!done);
        assert_eq!(conv_id, Some("conv-123".to_string()));
    }

    #[test]
    fn test_parse_sse_done() {
        let line = "[DONE]";
        let (content, done, conv_id) = parse_backend_sse_line(line);
        assert!(content.is_none());
        assert!(done);
        assert!(conv_id.is_none());
    }

    #[test]
    fn test_parse_sse_finished_message() {
        let line = r#"{"message":{"id":"msg-abc","author":{"role":"assistant"},"content":{"content_type":"text","parts":["Full response here"]},"status":"finished_successfully"},"conversation_id":"conv-123"}"#;
        let (content, done, conv_id) = parse_backend_sse_line(line);
        assert_eq!(content, Some("Full response here".to_string()));
        assert!(done);
        assert_eq!(conv_id, Some("conv-123".to_string()));
    }

    #[test]
    fn test_parse_sse_invalid_json() {
        let line = "not valid json";
        let (content, done, conv_id) = parse_backend_sse_line(line);
        assert!(content.is_none());
        assert!(!done);
        assert!(conv_id.is_none());
    }

    #[test]
    fn test_parse_sse_empty_line() {
        let line = "";
        let (content, done, conv_id) = parse_backend_sse_line(line);
        assert!(content.is_none());
        assert!(!done);
        assert!(conv_id.is_none());
    }

    #[test]
    fn test_parse_full_sse_response_success() {
        let body = "data: {\"message\":{\"id\":\"msg-1\",\"author\":{\"role\":\"assistant\"},\"content\":{\"content_type\":\"text\",\"parts\":[\"Hello\"]},\"status\":\"in_progress\"},\"conversation_id\":\"conv-1\"}\n\ndata: {\"message\":{\"id\":\"msg-1\",\"author\":{\"role\":\"assistant\"},\"content\":{\"content_type\":\"text\",\"parts\":[\"Hello world\"]},\"status\":\"finished_successfully\"},\"conversation_id\":\"conv-1\"}\n\ndata: [DONE]\n";
        let resp = ChatGptBackendProvider::parse_full_sse_response(body).unwrap();
        assert_eq!(resp.message.role, "assistant");
        assert_eq!(resp.message.content, "Hello world");
        assert!(resp.usage.is_none());
    }

    #[test]
    fn test_parse_full_sse_response_empty() {
        let body = "data: [DONE]\n";
        let result = ChatGptBackendProvider::parse_full_sse_response(body);
        assert!(result.is_err());
    }

    // ── Codex CLI JSONL parsing tests ──

    #[test]
    fn test_parse_codex_jsonl_simple() {
        let output = r#"{"type":"thread.started","thread_id":"abc"}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"Hello!"}}
{"type":"turn.completed","usage":{"input_tokens":100,"output_tokens":10}}"#;
        let (text, usage) = parse_codex_jsonl(output).unwrap();
        assert_eq!(text, "Hello!");
        let u = usage.unwrap();
        assert_eq!(u.prompt_tokens, 100);
        assert_eq!(u.completion_tokens, 10);
        assert_eq!(u.total_tokens, 110);
    }

    #[test]
    fn test_parse_codex_jsonl_no_usage() {
        let output = r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"World"}}"#;
        let (text, usage) = parse_codex_jsonl(output).unwrap();
        assert_eq!(text, "World");
        assert!(usage.is_none());
    }

    #[test]
    fn test_parse_codex_jsonl_empty() {
        let result = parse_codex_jsonl("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_codex_jsonl_multiple_items() {
        let output = r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"First"}}
{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"Second"}}"#;
        let (text, _) = parse_codex_jsonl(output).unwrap();
        // Last item wins
        assert_eq!(text, "Second");
    }

    #[test]
    fn test_parse_codex_jsonl_with_cached_tokens() {
        let output = r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"OK"}}
{"type":"turn.completed","usage":{"input_tokens":8625,"cached_input_tokens":7040,"output_tokens":48}}"#;
        let (text, usage) = parse_codex_jsonl(output).unwrap();
        assert_eq!(text, "OK");
        let u = usage.unwrap();
        assert_eq!(u.prompt_tokens, 8625);
        assert_eq!(u.completion_tokens, 48);
    }

    // ── Prompt building tests ──

    #[test]
    fn test_messages_to_prompt_user_only() {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
            tool_calls: None,
            tool_call_id: None,
        }];
        let prompt = messages_to_prompt(&messages);
        assert_eq!(prompt, "Hello");
    }

    #[test]
    fn test_messages_to_prompt_system_and_user() {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: "Be helpful".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let prompt = messages_to_prompt(&messages);
        assert!(prompt.contains("[System instruction: Be helpful]"));
        assert!(prompt.contains("Hello"));
    }

    #[test]
    fn test_messages_to_prompt_multi_turn() {
        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: "What is 2+2?".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: "4".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: "user".to_string(),
                content: "And 3+3?".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let prompt = messages_to_prompt(&messages);
        assert!(prompt.contains("What is 2+2?"));
        assert!(prompt.contains("[Previous assistant response: 4]"));
        assert!(prompt.contains("And 3+3?"));
    }

    // ── Provider struct tests ──

    #[tokio::test]
    async fn test_provider_name_and_model() {
        let tm = Arc::new(CodexTokenManager::new());
        let provider = ChatGptBackendProvider::new(tm);
        assert_eq!(provider.name(), "chatgpt_backend");
        assert_eq!(provider.default_model(), "gpt-5.4");
        assert!(provider.capabilities().streaming);
        assert!(!provider.capabilities().native_tools);
    }

    #[tokio::test]
    async fn test_provider_with_custom_model() {
        let tm = Arc::new(CodexTokenManager::new());
        let provider = ChatGptBackendProvider::with_model(tm, "gpt-5.4");
        assert_eq!(provider.default_model(), "gpt-5.4");
    }

    #[tokio::test]
    async fn test_build_request_body() {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: "You are helpful".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let tm = Arc::new(CodexTokenManager::new());
        let provider = ChatGptBackendProvider::new(tm);
        let body = provider.build_request_body(&messages, "gpt-4o");
        assert_eq!(body["action"], "next");
        assert_eq!(body["model"], "gpt-4o");
        assert!(body["parent_message_id"].as_str().unwrap().len() > 10);
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
    }
}
