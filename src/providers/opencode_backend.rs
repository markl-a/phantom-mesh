//! OpenCode Backend Provider: uses OpenCode CLI subprocess for free LLM access.
//!
//! Runs `opencode run --format json -m <model>` as a subprocess, parsing JSONL events.
//! Free models: opencode/minimax-m2.5-free, opencode/mimo-v2-flash-free, opencode/nemotron-3-super-free
//! Also supports paid models via OpenCode's auth (opencode/gpt-5.4, opencode/claude-sonnet-4-6, etc.)

use std::pin::Pin;

use anyhow::Result;
use async_trait::async_trait;
use futures_util::Stream;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, warn};

use super::traits::{
    ChatMessage, ChatResponse, Provider, ProviderCapabilities, StreamChunk, TokenUsage,
};

// ── OpenCode CLI helpers ─────────────────────────────────────────────────────

/// Build a prompt string from ChatMessages for OpenCode CLI.
fn messages_to_prompt(messages: &[ChatMessage]) -> String {
    let mut parts = Vec::new();
    for msg in messages {
        match msg.role.as_str() {
            "system" => parts.push(format!("[System instruction: {}]", msg.content)),
            "user" => parts.push(msg.content.clone()),
            "assistant" => parts.push(format!("[Previous response: {}]", msg.content)),
            "tool" => parts.push(format!("[Tool result: {}]", msg.content)),
            _ => parts.push(msg.content.clone()),
        }
    }
    parts.join("\n\n")
}

/// Parse OpenCode CLI JSONL output into (response_text, token_usage).
///
/// Expected JSONL events:
/// - `{"type":"step_start","timestamp":...}`
/// - `{"type":"text","part":{"text":"..."}}`
/// - `{"type":"step_finish","part":{"tokens":{"total":N,"input":N,"output":N}}}`
fn parse_opencode_jsonl(output: &str) -> Result<(String, Option<TokenUsage>)> {
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
            "text" => {
                if let Some(text) = parsed["part"]["text"].as_str() {
                    // Accumulate text parts (some models stream in chunks)
                    if response_text.is_empty() {
                        response_text = text.to_string();
                    } else {
                        response_text.push_str(text);
                    }
                }
            }
            "step_finish" => {
                if let Some(tokens) = parsed["part"]["tokens"].as_object() {
                    let input = tokens.get("input")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                    let output_t = tokens.get("output")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                    let total = tokens.get("total")
                        .and_then(|v| v.as_u64())
                        .unwrap_or((input + output_t) as u64) as u32;
                    usage = Some(TokenUsage {
                        prompt_tokens: input,
                        completion_tokens: output_t,
                        total_tokens: total,
                    });
                }
            }
            _ => {} // step_start, etc.
        }
    }

    if response_text.is_empty() {
        anyhow::bail!("No response text in OpenCode CLI output");
    }

    Ok((response_text, usage))
}

/// Build a sanitized environment for subprocess execution.
fn safe_subprocess_env() -> Vec<(String, String)> {
    let sensitive_prefixes = [
        "ANTHROPIC_", "OPENAI_", "GEMINI_", "GROQ_", "DEEPSEEK_",
        "SERPER_", "TAVILY_", "BRAVE_", "EXA_", "STRIPE_",
        "PHANTOM_MESH_SECRET", "PHANTOM_MESH_HUB_KEY",
        "TWITTER_", "SMTP_", "EMAIL_",
    ];
    let sensitive_exact = [
        "SECRET_KEY", "API_KEY", "ACCESS_TOKEN", "REFRESH_TOKEN",
        "BOT_TOKEN", "WEBHOOK_SECRET",
    ];

    std::env::vars()
        .filter(|(key, _)| {
            let upper = key.to_uppercase();
            !sensitive_prefixes.iter().any(|p| upper.starts_with(p))
                && !sensitive_exact.iter().any(|s| upper.contains(s))
        })
        .collect()
}

/// OpenCode binary resolution result.
struct OpenCodeBinary {
    program: String,
    script: Option<String>,
}

/// Find the `opencode` executable, resolving `.cmd` wrappers on Windows.
fn find_opencode_binary() -> Option<OpenCodeBinary> {
    if cfg!(windows) {
        // Try to resolve opencode.cmd -> node + JS entry point
        if let Ok(output) = std::process::Command::new("where")
            .arg("opencode.cmd")
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
                    if let Ok(content) = std::fs::read_to_string(&cmd_path) {
                        for line in content.lines() {
                            if let Some(start) = line.find("node_modules") {
                                let rest = &line[start..];
                                let end = rest.find('"')
                                    .or_else(|| rest.find('%'))
                                    .unwrap_or(rest.len());
                                let rel_path = rest[..end].trim();
                                let dir = std::path::Path::new(&cmd_path).parent()?;
                                let js_path = dir.join(rel_path);
                                if js_path.exists() {
                                    return Some(OpenCodeBinary {
                                        program: "node".to_string(),
                                        script: Some(js_path.to_string_lossy().to_string()),
                                    });
                                }
                            }
                        }
                    }
                    return Some(OpenCodeBinary {
                        program: cmd_path,
                        script: None,
                    });
                }
            }
        }
        None
    } else {
        let check = std::process::Command::new("which")
            .arg("opencode")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if let Ok(s) = check {
            if s.success() {
                return Some(OpenCodeBinary {
                    program: "opencode".to_string(),
                    script: None,
                });
            }
        }
        None
    }
}

// ── OpenCodeBackendProvider ─────────────────────────────────────────────────

/// Provider that uses OpenCode CLI (`opencode run --format json`) as a subprocess
/// to access free and paid LLM models.
pub struct OpenCodeBackendProvider {
    default_model_name: String,
}

impl OpenCodeBackendProvider {
    pub fn new() -> Self {
        Self {
            default_model_name: "opencode/minimax-m2.5-free".to_string(),
        }
    }

    pub fn with_model(model: &str) -> Self {
        Self {
            default_model_name: model.to_string(),
        }
    }

    /// Run the OpenCode CLI subprocess and return JSONL output.
    async fn run_opencode_cli(&self, prompt: &str, model: &str) -> Result<String> {
        let bin = find_opencode_binary()
            .ok_or_else(|| anyhow::anyhow!("OpenCode CLI not found. Install with: npm install -g @anthropic-ai/opencode"))?;

        debug!("Running OpenCode CLI: model={}, prompt_len={}", model, prompt.len());

        let mut args: Vec<String> = Vec::new();
        if let Some(ref script) = bin.script {
            args.push(script.clone());
        }
        args.push("run".to_string());
        args.push("--format".to_string());
        args.push("json".to_string());
        if !model.is_empty() && model != "auto" && model != "default" {
            args.push("-m".to_string());
            args.push(model.to_string());
        }

        // Windows has ~32K command-line limit. Use stdin for long prompts.
        let use_stdin = prompt.len() > 8000;
        if !use_stdin {
            args.push(prompt.to_string());
        }

        let mut cmd = tokio::process::Command::new(&bin.program);
        cmd.args(&args)
            .env_clear()
            .envs(safe_subprocess_env())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let output = if use_stdin {
            debug!("OpenCode CLI: prompt too long ({}), using stdin", prompt.len());
            cmd.stdin(std::process::Stdio::piped());
            let mut child = cmd.spawn()
                .map_err(|e| anyhow::anyhow!("Failed to spawn OpenCode CLI: {}", e))?;
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(prompt.as_bytes()).await;
                drop(stdin);
            }
            tokio::time::timeout(
                std::time::Duration::from_secs(300),
                child.wait_with_output(),
            )
            .await
            .map_err(|_| anyhow::anyhow!("OpenCode CLI timed out after 300s"))?
            .map_err(|e| anyhow::anyhow!("Failed to run OpenCode CLI: {}", e))?
        } else {
            tokio::time::timeout(
                std::time::Duration::from_secs(300),
                cmd.output(),
            )
            .await
            .map_err(|_| anyhow::anyhow!("OpenCode CLI timed out after 300s"))?
            .map_err(|e| anyhow::anyhow!("Failed to run OpenCode CLI: {}", e))?
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            warn!("OpenCode CLI stderr: {}", stderr);
            // Check for error in JSONL
            for line in stdout.lines() {
                if let Ok(v) = serde_json::from_str::<Value>(line) {
                    if v["type"].as_str() == Some("error") {
                        let msg = v["message"].as_str()
                            .or_else(|| v["part"]["text"].as_str())
                            .unwrap_or("Unknown error");
                        anyhow::bail!("OpenCode CLI error: {}", msg);
                    }
                }
            }
            anyhow::bail!(
                "OpenCode CLI exited with status {}: {}",
                output.status,
                if stderr.is_empty() { &stdout } else { &stderr }
            );
        }

        Ok(stdout)
    }
}

#[async_trait]
impl Provider for OpenCodeBackendProvider {
    fn name(&self) -> &str {
        "opencode_backend"
    }

    fn default_model(&self) -> &str {
        &self.default_model_name
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            native_tools: false,
            vision: false,
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

        let jsonl_output = self.run_opencode_cli(&prompt, model).await?;
        let (text, usage) = parse_opencode_jsonl(&jsonl_output)?;

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

        let bin = find_opencode_binary()
            .ok_or_else(|| anyhow::anyhow!("OpenCode CLI not found"))?;

        let (tx, rx) = mpsc::channel::<Result<StreamChunk>>(64);

        let bin_program = bin.program;
        let bin_script = bin.script;

        tokio::spawn(async move {
            let mut args: Vec<String> = Vec::new();
            if let Some(ref script) = bin_script {
                args.push(script.clone());
            }
            args.push("run".to_string());
            args.push("--format".to_string());
            args.push("json".to_string());
            if !model_str.is_empty() && model_str != "auto" && model_str != "default" {
                args.push("-m".to_string());
                args.push(model_str.clone());
            }
            args.push(prompt.clone());

            let mut child = match tokio::process::Command::new(&bin_program)
                .args(&args)
                .env_clear()
                .envs(safe_subprocess_env())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(Err(anyhow::anyhow!("Spawn failed: {}", e))).await;
                    return;
                }
            };

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

            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                if let Ok(parsed) = serde_json::from_str::<Value>(&line) {
                    let event_type = parsed["type"].as_str().unwrap_or("");
                    match event_type {
                        "text" => {
                            if let Some(text) = parsed["part"]["text"].as_str() {
                                let chunk = StreamChunk::ContentDelta(text.to_string());
                                if tx.send(Ok(chunk)).await.is_err() {
                                    break;
                                }
                            }
                        }
                        "step_finish" => {
                            let mut usage = None;
                            if let Some(tokens) = parsed["part"]["tokens"].as_object() {
                                let input = tokens.get("input")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0) as u32;
                                let output_t = tokens.get("output")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0) as u32;
                                usage = Some(TokenUsage {
                                    prompt_tokens: input,
                                    completion_tokens: output_t,
                                    total_tokens: input + output_t,
                                });
                            }
                            let chunk = StreamChunk::Done { usage };
                            let _ = tx.send(Ok(chunk)).await;
                            break;
                        }
                        _ => {}
                    }
                }
            }

            let _ = child.wait().await;
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn is_alive(&self) -> bool {
        find_opencode_binary().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_name() {
        let provider = OpenCodeBackendProvider::new();
        assert_eq!(provider.name(), "opencode_backend");
        assert_eq!(provider.default_model(), "opencode/minimax-m2.5-free");
    }

    #[test]
    fn test_with_model() {
        let provider = OpenCodeBackendProvider::with_model("opencode/nemotron-3-super-free");
        assert_eq!(provider.default_model(), "opencode/nemotron-3-super-free");
    }

    #[test]
    fn test_parse_opencode_jsonl() {
        let jsonl = r#"{"type":"step_start","timestamp":1773569543178,"sessionID":"ses_test","part":{"type":"step-start"}}
{"type":"text","timestamp":1773569548553,"sessionID":"ses_test","part":{"type":"text","text":"Hello world"}}
{"type":"step_finish","timestamp":1773569548625,"sessionID":"ses_test","part":{"type":"step-finish","reason":"stop","cost":0,"tokens":{"total":100,"input":80,"output":20,"reasoning":0}}}"#;

        let (text, usage) = parse_opencode_jsonl(jsonl).unwrap();
        assert_eq!(text, "Hello world");
        assert!(usage.is_some());
        let u = usage.unwrap();
        assert_eq!(u.prompt_tokens, 80);
        assert_eq!(u.completion_tokens, 20);
        assert_eq!(u.total_tokens, 100);
    }

    #[test]
    fn test_parse_empty_output() {
        let result = parse_opencode_jsonl("");
        assert!(result.is_err());
    }

    #[test]
    fn test_messages_to_prompt() {
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
}
