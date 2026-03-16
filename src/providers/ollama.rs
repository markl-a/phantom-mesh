use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::pin::Pin;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, warn};

use super::traits::*;

/// Ollama LLM provider (/api/chat, /api/generate)
pub struct OllamaProvider {
    provider_name: String,
    base_url: String,
    default_model: String,
    client: Client,
}

impl OllamaProvider {
    pub fn new(name: String, base_url: String, default_model: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .expect("Failed to build HTTP client");
        Self {
            provider_name: name,
            base_url,
            default_model,
            client,
        }
    }

    fn resolve_model<'a>(&'a self, model: &'a str) -> &'a str {
        if model.is_empty() { &self.default_model } else { model }
    }
}

#[async_trait]
impl Provider for OllamaProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            native_tools: true,
            vision: false,
        }
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<ChatResponse> {
        let model = self.resolve_model(model);
        let url = format!("{}/api/chat", self.base_url);

        let mut body = json!({
            "model": model,
            "messages": messages,
            "stream": false
        });

        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }

        debug!("Ollama chat: model={}, {} messages, {} tools", model, messages.len(), tools.len());

        let resp = self.client.post(&url).json(&body).send().await?;
        let status = resp.status();
        let json: Value = resp.json().await?;

        if !status.is_success() {
            let err = json.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error");
            warn!("Ollama chat error (status={}): {}", status, err);
            return Err(anyhow!("Ollama chat error: {}", err));
        }

        let msg = json.get("message").ok_or_else(|| anyhow!("Ollama response missing 'message'"))?;
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("assistant").to_string();
        let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();

        let tool_calls = parse_ollama_tool_calls(msg);

        let usage = {
            let prompt_tokens = json.get("prompt_eval_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let completion_tokens = json.get("eval_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            if prompt_tokens > 0 || completion_tokens > 0 {
                Some(TokenUsage {
                    prompt_tokens,
                    completion_tokens,
                    total_tokens: prompt_tokens + completion_tokens,
                })
            } else {
                None
            }
        };

        Ok(ChatResponse {
            message: ChatMessage { role, content, tool_calls, tool_call_id: None },
            usage,
        })
    }

    async fn stream_chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<Pin<Box<dyn futures_util::Stream<Item = Result<StreamChunk>> + Send>>> {
        let model = self.resolve_model(model).to_string();
        let url = format!("{}/api/chat", self.base_url);

        let mut body = json!({
            "model": model,
            "messages": messages,
            "stream": true
        });

        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }

        debug!("Ollama stream_chat: model={}, {} messages", model, messages.len());

        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Ollama stream error: {}", text));
        }

        let (tx, rx) = mpsc::channel::<Result<StreamChunk>>(64);
        let byte_stream = resp.bytes_stream();

        tokio::spawn(async move {
            let mut stream = Box::pin(byte_stream);
            let mut buffer = String::new();

            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));
                        // Process complete lines (NDJSON)
                        while let Some(pos) = buffer.find('\n') {
                            let line = buffer[..pos].trim().to_string();
                            buffer = buffer[pos + 1..].to_string();
                            if line.is_empty() {
                                continue;
                            }
                            match serde_json::from_str::<Value>(&line) {
                                Ok(json) => {
                                    let done = json.get("done").and_then(|v| v.as_bool()).unwrap_or(false);
                                    if done {
                                        let usage = parse_ollama_stream_usage(&json);
                                        let _ = tx.send(Ok(StreamChunk::Done { usage })).await;
                                    } else if let Some(msg) = json.get("message") {
                                        let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
                                        if !content.is_empty() {
                                            let _ = tx.send(Ok(StreamChunk::ContentDelta(content.to_string()))).await;
                                        }
                                    }
                                }
                                Err(e) => {
                                    debug!("Ollama NDJSON parse error: {} (line: {})", e, &line[..line.len().min(100)]);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(anyhow!("Ollama stream read error: {}", e))).await;
                        break;
                    }
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn is_alive(&self) -> bool {
        let url = format!("{}/api/tags", self.base_url);
        self.client
            .get(&url)
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

fn parse_ollama_tool_calls(msg: &Value) -> Option<Vec<ToolCall>> {
    let arr = msg.get("tool_calls")?.as_array()?;
    if arr.is_empty() {
        return None;
    }
    let calls: Vec<ToolCall> = arr.iter().filter_map(|call| {
        let id = call.get("id").and_then(|v| v.as_str()).map(String::from);
        let func = call.get("function")?;
        let name = func.get("name")?.as_str()?.to_string();
        let arguments = func.get("arguments").cloned().unwrap_or(json!({}));
        Some(ToolCall {
            id,
            function: ToolCallFunction { name, arguments },
        })
    }).collect();
    if calls.is_empty() { None } else { Some(calls) }
}

fn parse_ollama_stream_usage(json: &Value) -> Option<TokenUsage> {
    let prompt_tokens = json.get("prompt_eval_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let completion_tokens = json.get("eval_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if prompt_tokens > 0 || completion_tokens > 0 {
        Some(TokenUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ollama_provider_name() {
        let p = OllamaProvider::new("ollama".into(), "http://localhost:11434".into(), "qwen3:8b".into());
        assert_eq!(p.name(), "ollama");
        assert_eq!(p.default_model(), "qwen3:8b");
    }

    #[test]
    fn test_ollama_capabilities() {
        let p = OllamaProvider::new("ollama".into(), "http://localhost:11434".into(), "qwen3:8b".into());
        let caps = p.capabilities();
        assert!(caps.streaming);
        assert!(caps.native_tools);
    }

    #[test]
    fn test_resolve_model() {
        let p = OllamaProvider::new("ollama".into(), "http://localhost:11434".into(), "qwen3:8b".into());
        assert_eq!(p.resolve_model(""), "qwen3:8b");
        assert_eq!(p.resolve_model("llama3:8b"), "llama3:8b");
    }

    #[test]
    fn test_parse_ollama_tool_calls_none() {
        let msg = json!({"role": "assistant", "content": "hello"});
        assert!(parse_ollama_tool_calls(&msg).is_none());
    }

    #[test]
    fn test_parse_ollama_tool_calls_empty() {
        let msg = json!({"role": "assistant", "content": "", "tool_calls": []});
        assert!(parse_ollama_tool_calls(&msg).is_none());
    }

    #[test]
    fn test_parse_ollama_tool_calls_valid() {
        let msg = json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{
                "function": {
                    "name": "shell",
                    "arguments": {"command": "ls"}
                }
            }]
        });
        let calls = parse_ollama_tool_calls(&msg).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "shell");
    }

    #[test]
    fn test_parse_stream_usage_none() {
        let json = json!({"done": true});
        assert!(parse_ollama_stream_usage(&json).is_none());
    }

    #[test]
    fn test_parse_stream_usage_present() {
        let json = json!({"done": true, "prompt_eval_count": 10, "eval_count": 20});
        let usage = parse_ollama_stream_usage(&json).unwrap();
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 20);
        assert_eq!(usage.total_tokens, 30);
    }
}
