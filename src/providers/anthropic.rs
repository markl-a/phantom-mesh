use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::pin::Pin;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::debug;

use super::traits::*;

const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic Messages API provider
pub struct AnthropicProvider {
    provider_name: String,
    base_url: String,
    default_model: String,
    api_key: String,
    client: Client,
}

impl AnthropicProvider {
    pub fn new(name: String, base_url: String, default_model: String, api_key: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .expect("Failed to build HTTP client");
        Self {
            provider_name: name,
            base_url,
            default_model,
            api_key,
            client,
        }
    }

    fn resolve_model<'a>(&'a self, model: &'a str) -> &'a str {
        if model.is_empty() { &self.default_model } else { model }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
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
            vision: true,
        }
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<ChatResponse> {
        let model = self.resolve_model(model);
        let url = format!("{}/v1/messages", self.base_url);

        let (system_prompt, api_messages) = messages_to_anthropic_json(messages);

        let mut body = json!({
            "model": model,
            "messages": api_messages,
            "max_tokens": 4096,
        });

        if let Some(sys) = &system_prompt {
            body["system"] = json!(sys);
        }

        if !tools.is_empty() {
            body["tools"] = json!(tools_to_anthropic_json(tools));
        }

        debug!("Anthropic chat: model={}, {} messages, {} tools", model, messages.len(), tools.len());

        let resp = self.client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let json: Value = resp.json().await?;

        if !status.is_success() {
            let err_type = json.get("error").and_then(|e| e.get("message")).and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(anyhow!("Anthropic error ({}): {}", status, err_type));
        }

        // Parse response content blocks
        let mut content_text = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        if let Some(blocks) = json.get("content").and_then(|v| v.as_array()) {
            for block in blocks {
                match block.get("type").and_then(|v| v.as_str()) {
                    Some("text") => {
                        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                            content_text.push_str(text);
                        }
                    }
                    Some("tool_use") => {
                        let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let input = block.get("input").cloned().unwrap_or(json!({}));
                        tool_calls.push(ToolCall {
                            id: Some(id),
                            function: ToolCallFunction { name, arguments: input },
                        });
                    }
                    _ => {}
                }
            }
        }

        // Parse usage
        let usage = json.get("usage").map(|u| {
            let input = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let output = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            TokenUsage {
                prompt_tokens: input,
                completion_tokens: output,
                total_tokens: input + output,
            }
        });

        Ok(ChatResponse {
            message: ChatMessage {
                role: "assistant".to_string(),
                content: content_text,
                tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
                tool_call_id: None,
            },
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
        let url = format!("{}/v1/messages", self.base_url);

        let (system_prompt, api_messages) = messages_to_anthropic_json(messages);

        let mut body = json!({
            "model": model,
            "messages": api_messages,
            "max_tokens": 4096,
            "stream": true,
        });

        if let Some(sys) = &system_prompt {
            body["system"] = json!(sys);
        }

        if !tools.is_empty() {
            body["tools"] = json!(tools_to_anthropic_json(tools));
        }

        debug!("Anthropic stream_chat: model={}, {} messages", model, messages.len());

        let resp = self.client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Anthropic stream error: {}", text));
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

                        while let Some(pos) = buffer.find("\n") {
                            let line = buffer[..pos].trim().to_string();
                            buffer = buffer[pos + 1..].to_string();

                            if line.is_empty() || line.starts_with("event:") {
                                continue;
                            }

                            if let Some(data) = line.strip_prefix("data: ") {
                                match serde_json::from_str::<Value>(data) {
                                    Ok(json) => {
                                        match json.get("type").and_then(|v| v.as_str()) {
                                            Some("content_block_start") => {
                                                if let Some(block) = json.get("content_block") {
                                                    if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                                                        let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                        let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                        let _ = tx.send(Ok(StreamChunk::ToolCallStart { id, name })).await;
                                                    }
                                                }
                                            }
                                            Some("content_block_delta") => {
                                                if let Some(delta) = json.get("delta") {
                                                    match delta.get("type").and_then(|v| v.as_str()) {
                                                        Some("text_delta") => {
                                                            if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                                                                if !text.is_empty() {
                                                                    let _ = tx.send(Ok(StreamChunk::ContentDelta(text.to_string()))).await;
                                                                }
                                                            }
                                                        }
                                                        Some("input_json_delta") => {
                                                            if let Some(partial) = delta.get("partial_json").and_then(|v| v.as_str()) {
                                                                let _ = tx.send(Ok(StreamChunk::ToolCallArgumentsDelta {
                                                                    id: String::new(), // Will be associated by caller
                                                                    delta: partial.to_string(),
                                                                })).await;
                                                            }
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }
                                            Some("message_delta") => {
                                                let usage = json.get("usage").map(|u| {
                                                    let output = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                                                    TokenUsage {
                                                        prompt_tokens: 0,
                                                        completion_tokens: output,
                                                        total_tokens: output,
                                                    }
                                                });
                                                let _ = tx.send(Ok(StreamChunk::Done { usage })).await;
                                                return;
                                            }
                                            Some("message_stop") => {
                                                let _ = tx.send(Ok(StreamChunk::Done { usage: None })).await;
                                                return;
                                            }
                                            _ => {}
                                        }
                                    }
                                    Err(e) => {
                                        debug!("Anthropic SSE parse error: {}", e);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(anyhow!("Anthropic stream read error: {}", e))).await;
                        break;
                    }
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn is_alive(&self) -> bool {
        // Anthropic doesn't have a simple health endpoint; check with a minimal request
        let url = format!("{}/v1/messages", self.base_url);
        // Just check if we can reach the server (any non-network-error is "alive")
        match self.client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&json!({
                "model": self.default_model,
                "messages": [{"role": "user", "content": "ping"}],
                "max_tokens": 1,
            }))
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status().as_u16();
                // 200 = OK, 401 = bad key (but server is alive), 429 = rate limited (alive)
                status == 200 || status == 401 || status == 429
            }
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anthropic_provider_name() {
        let p = AnthropicProvider::new(
            "anthropic".into(),
            "https://api.anthropic.com".into(),
            "claude-sonnet-4-6".into(),
            "sk-test".into(),
        );
        assert_eq!(p.name(), "anthropic");
        assert_eq!(p.default_model(), "claude-sonnet-4-6");
    }

    #[test]
    fn test_anthropic_capabilities() {
        let p = AnthropicProvider::new(
            "anthropic".into(),
            "https://api.anthropic.com".into(),
            "claude-sonnet-4-6".into(),
            "sk-test".into(),
        );
        let caps = p.capabilities();
        assert!(caps.streaming);
        assert!(caps.native_tools);
        assert!(caps.vision);
    }

    #[test]
    fn test_resolve_model() {
        let p = AnthropicProvider::new(
            "anthropic".into(),
            "https://api.anthropic.com".into(),
            "claude-sonnet-4-6".into(),
            "sk-test".into(),
        );
        assert_eq!(p.resolve_model(""), "claude-sonnet-4-6");
        assert_eq!(p.resolve_model("claude-opus-4-6"), "claude-opus-4-6");
    }

    #[test]
    fn test_anthropic_version_constant() {
        assert_eq!(ANTHROPIC_VERSION, "2023-06-01");
    }
}
