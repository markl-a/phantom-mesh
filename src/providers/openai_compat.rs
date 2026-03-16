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

/// OpenAI-compatible provider (LM Studio, vLLM, Groq, Cerebras, etc.)
pub struct OpenAiCompatProvider {
    provider_name: String,
    base_url: String,
    default_model: String,
    api_key: Option<String>,
    client: Client,
}

impl OpenAiCompatProvider {
    pub fn new(name: String, base_url: String, default_model: String, api_key: Option<String>) -> Self {
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

    /// Chat with an explicit bearer token (for Codex OAuth token override).
    /// Same as `chat()` but uses the provided token instead of `self.api_key`.
    pub async fn chat_with_token(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
        token: &str,
    ) -> anyhow::Result<ChatResponse> {
        let model = self.resolve_model(model);
        let url = format!("{}/v1/chat/completions", self.base_url);
        let openai_messages = messages_to_openai_json(messages);
        let mut body = serde_json::json!({
            "model": model,
            "messages": openai_messages,
            "stream": false,
            "max_tokens": 4096
        });
        if !tools.is_empty() {
            body["tools"] = serde_json::json!(tools);
        }
        let req = self.client.post(&url).json(&body).bearer_auth(token);
        debug!("OpenAI-compat chat_with_token: model={}, {} messages", model, messages.len());
        let resp = req.send().await?;
        let json: Value = resp.json().await?;
        if let Some(err) = json.get("error") {
            return Err(anyhow::anyhow!("OpenAI-compat error: {}", err));
        }
        let msg = json.pointer("/choices/0/message")
            .ok_or_else(|| anyhow::anyhow!("OpenAI response missing choices[0].message: {}", json))?;
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("assistant").to_string();
        let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let tool_calls = parse_openai_tool_calls(msg);
        let usage = parse_openai_usage(&json);
        Ok(ChatResponse {
            message: ChatMessage { role, content, tool_calls, tool_call_id: None },
            usage,
        })
    }

    /// Streaming chat with an explicit bearer token (for Codex OAuth token override).
    pub async fn stream_chat_with_token(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
        token: &str,
    ) -> anyhow::Result<Pin<Box<dyn futures_util::Stream<Item = anyhow::Result<StreamChunk>> + Send>>> {
        let model = self.resolve_model(model).to_string();
        let url = format!("{}/v1/chat/completions", self.base_url);
        let openai_messages = messages_to_openai_json(messages);
        let mut body = serde_json::json!({
            "model": model,
            "messages": openai_messages,
            "stream": true,
            "max_tokens": 4096
        });
        if !tools.is_empty() {
            body["tools"] = serde_json::json!(tools);
        }
        let req = self.client.post(&url).json(&body).bearer_auth(token);
        debug!("OpenAI-compat stream_chat_with_token: model={}, {} messages", model, messages.len());
        let resp = req.send().await?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("OpenAI-compat stream error: {}", text));
        }
        let (tx, rx) = mpsc::channel::<anyhow::Result<StreamChunk>>(64);
        let byte_stream = resp.bytes_stream();
        tokio::spawn(async move {
            let mut stream = Box::pin(byte_stream);
            let mut buffer = String::new();
            while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
                match chunk {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));
                        while let Some(pos) = buffer.find("\n") {
                            let line = buffer[..pos].trim().to_string();
                            buffer = buffer[pos + 1..].to_string();
                            if line.is_empty() || line.starts_with(':') { continue; }
                            if let Some(data) = line.strip_prefix("data: ") {
                                if data.trim() == "[DONE]" {
                                    let _ = tx.send(Ok(StreamChunk::Done { usage: None })).await;
                                    return;
                                }
                                if let Ok(json) = serde_json::from_str::<Value>(data) {
                                    if let Some(content) = json.pointer("/choices/0/delta/content").and_then(|v| v.as_str()) {
                                        if !content.is_empty() {
                                            let _ = tx.send(Ok(StreamChunk::ContentDelta(content.to_string()))).await;
                                        }
                                    }
                                    if let Some(tcs) = json.pointer("/choices/0/delta/tool_calls").and_then(|v| v.as_array()) {
                                        for tc in tcs {
                                            let idx = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                                            let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                            if let Some(func) = tc.get("function") {
                                                if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                                                    let _ = tx.send(Ok(StreamChunk::ToolCallStart {
                                                        id: if id.is_empty() { format!("call_{}", idx) } else { id.clone() },
                                                        name: name.to_string(),
                                                    })).await;
                                                }
                                                if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                                                    if !args.is_empty() {
                                                        let _ = tx.send(Ok(StreamChunk::ToolCallArgumentsDelta {
                                                            id: if id.is_empty() { format!("call_{}", idx) } else { id },
                                                            delta: args.to_string(),
                                                        })).await;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    if let Some(usage) = json.get("usage") {
                                        if let Some(u) = parse_openai_usage_from_value(usage) {
                                            let _ = tx.send(Ok(StreamChunk::Done { usage: Some(u) })).await;
                                            return;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(anyhow::anyhow!("OpenAI stream read error: {}", e))).await;
                        break;
                    }
                }
            }
            let _ = tx.send(Ok(StreamChunk::Done { usage: None })).await;
        });
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

#[async_trait]
impl Provider for OpenAiCompatProvider {
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
        let url = format!("{}/v1/chat/completions", self.base_url);

        let openai_messages = messages_to_openai_json(messages);

        let mut body = json!({
            "model": model,
            "messages": openai_messages,
            "stream": false,
            "max_tokens": 4096
        });

        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }

        let mut req = self.client.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        debug!("OpenAI-compat chat: model={}, {} messages, {} tools", model, messages.len(), tools.len());

        let resp = req.send().await?;
        let json: Value = resp.json().await?;

        if let Some(err) = json.get("error") {
            return Err(anyhow!("OpenAI-compat error: {}", err));
        }

        let msg = json.pointer("/choices/0/message")
            .ok_or_else(|| anyhow!("OpenAI response missing choices[0].message: {}", json))?;

        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("assistant").to_string();
        let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();

        let tool_calls = parse_openai_tool_calls(msg);

        let usage = parse_openai_usage(&json);

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
        let url = format!("{}/v1/chat/completions", self.base_url);

        let openai_messages = messages_to_openai_json(messages);

        let mut body = json!({
            "model": model,
            "messages": openai_messages,
            "stream": true,
            "max_tokens": 4096
        });

        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }

        let mut req = self.client.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        debug!("OpenAI-compat stream_chat: model={}, {} messages", model, messages.len());

        let resp = req.send().await?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("OpenAI-compat stream error: {}", text));
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
                        // Process complete SSE lines
                        while let Some(pos) = buffer.find("\n") {
                            let line = buffer[..pos].trim().to_string();
                            buffer = buffer[pos + 1..].to_string();

                            if line.is_empty() || line.starts_with(':') {
                                continue; // SSE comment or blank
                            }

                            if let Some(data) = line.strip_prefix("data: ") {
                                if data.trim() == "[DONE]" {
                                    let _ = tx.send(Ok(StreamChunk::Done { usage: None })).await;
                                    return;
                                }

                                match serde_json::from_str::<Value>(data) {
                                    Ok(json) => {
                                        // Check for content delta
                                        if let Some(content) = json.pointer("/choices/0/delta/content")
                                            .and_then(|v| v.as_str())
                                        {
                                            if !content.is_empty() {
                                                let _ = tx.send(Ok(StreamChunk::ContentDelta(content.to_string()))).await;
                                            }
                                        }

                                        // Check for tool call deltas
                                        if let Some(tcs) = json.pointer("/choices/0/delta/tool_calls")
                                            .and_then(|v| v.as_array())
                                        {
                                            for tc in tcs {
                                                let idx = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                                                let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();

                                                if let Some(func) = tc.get("function") {
                                                    if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                                                        let _ = tx.send(Ok(StreamChunk::ToolCallStart {
                                                            id: if id.is_empty() { format!("call_{}", idx) } else { id.clone() },
                                                            name: name.to_string(),
                                                        })).await;
                                                    }
                                                    if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                                                        if !args.is_empty() {
                                                            let _ = tx.send(Ok(StreamChunk::ToolCallArgumentsDelta {
                                                                id: if id.is_empty() { format!("call_{}", idx) } else { id },
                                                                delta: args.to_string(),
                                                            })).await;
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                        // Check for usage in the final chunk
                                        if let Some(usage) = json.get("usage") {
                                            if let Some(u) = parse_openai_usage_from_value(usage) {
                                                let _ = tx.send(Ok(StreamChunk::Done { usage: Some(u) })).await;
                                                return;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        debug!("OpenAI SSE parse error: {} (data: {})", e, &data[..data.len().min(100)]);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(anyhow!("OpenAI stream read error: {}", e))).await;
                        break;
                    }
                }
            }
            // If stream ends without [DONE], send Done anyway
            let _ = tx.send(Ok(StreamChunk::Done { usage: None })).await;
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn is_alive(&self) -> bool {
        let url = format!("{}/v1/models", self.base_url);
        let mut req = self.client.get(&url).timeout(Duration::from_secs(3));
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        req.send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

fn parse_openai_tool_calls(msg: &Value) -> Option<Vec<ToolCall>> {
    let arr = msg.get("tool_calls")?.as_array()?;
    if arr.is_empty() {
        return None;
    }
    let calls: Vec<ToolCall> = arr.iter().filter_map(|call| {
        let id = call.get("id").and_then(|v| v.as_str()).map(String::from);
        let func = call.get("function")?;
        let name = func.get("name")?.as_str()?.to_string();
        let args_str = func.get("arguments")?.as_str()?;
        let arguments: Value = serde_json::from_str(args_str).ok()?;
        Some(ToolCall {
            id,
            function: ToolCallFunction { name, arguments },
        })
    }).collect();
    if calls.is_empty() { None } else { Some(calls) }
}

fn parse_openai_usage(json: &Value) -> Option<TokenUsage> {
    json.get("usage").and_then(parse_openai_usage_from_value)
}

fn parse_openai_usage_from_value(u: &Value) -> Option<TokenUsage> {
    let prompt_tokens = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let completion_tokens = u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let total = u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if prompt_tokens > 0 || completion_tokens > 0 {
        Some(TokenUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: if total > 0 { total } else { prompt_tokens + completion_tokens },
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_compat_provider_name() {
        let p = OpenAiCompatProvider::new(
            "lmstudio".into(),
            "http://localhost:1234".into(),
            "default".into(),
            None,
        );
        assert_eq!(p.name(), "lmstudio");
        assert_eq!(p.default_model(), "default");
    }

    #[test]
    fn test_openai_compat_capabilities() {
        let p = OpenAiCompatProvider::new("test".into(), "http://localhost:1234".into(), "m".into(), None);
        let caps = p.capabilities();
        assert!(caps.streaming);
        assert!(caps.native_tools);
    }

    #[test]
    fn test_resolve_model() {
        let p = OpenAiCompatProvider::new("test".into(), "http://localhost:1234".into(), "default-model".into(), None);
        assert_eq!(p.resolve_model(""), "default-model");
        assert_eq!(p.resolve_model("custom"), "custom");
    }

    #[test]
    fn test_parse_openai_tool_calls_none() {
        let msg = json!({"role": "assistant", "content": "hello"});
        assert!(parse_openai_tool_calls(&msg).is_none());
    }

    #[test]
    fn test_parse_openai_tool_calls_valid() {
        let msg = json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {
                    "name": "shell",
                    "arguments": "{\"command\":\"ls\"}"
                }
            }]
        });
        let calls = parse_openai_tool_calls(&msg).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "shell");
        assert_eq!(calls[0].id.as_deref(), Some("call_1"));
    }

    #[test]
    fn test_parse_openai_usage_present() {
        let json = json!({
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "total_tokens": 150
            }
        });
        let usage = parse_openai_usage(&json).unwrap();
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 50);
        assert_eq!(usage.total_tokens, 150);
    }

    #[test]
    fn test_parse_openai_usage_missing() {
        let json = json!({"choices": []});
        assert!(parse_openai_usage(&json).is_none());
    }

    #[test]
    fn test_parse_openai_usage_zero() {
        let json = json!({"usage": {"prompt_tokens": 0, "completion_tokens": 0}});
        assert!(parse_openai_usage(&json).is_none());
    }
}
