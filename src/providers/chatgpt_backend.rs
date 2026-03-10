//! ChatGPT Backend API provider: message format translation and Provider implementation.
//!
//! Talks directly to chatgpt.com/backend-api/conversation using Codex OAuth tokens,
//! translating between clawtex ChatMessage format and the backend SSE protocol.

use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use futures_util::Stream;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::debug;
use uuid::Uuid;

use super::codex::CodexTokenManager;
use super::traits::{
    ChatMessage, ChatResponse, ProviderCapabilities, Provider, StreamChunk,
};

// ── Message format translation ───────────────────────────────────────────────

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
///
/// Returns (content, is_done, conversation_id):
/// - content: the full text from the message parts (if present)
/// - is_done: true if this is [DONE] or status == "finished_successfully"
/// - conversation_id: the conversation ID (if present)
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

// ── ChatGptBackendProvider ───────────────────────────────────────────────────

const BACKEND_API_URL: &str = "https://chatgpt.com/backend-api/conversation";

/// Provider that talks to the ChatGPT backend-api (chatgpt.com/backend-api/conversation)
/// using Codex OAuth tokens for authentication.
pub struct ChatGptBackendProvider {
    token_manager: Arc<CodexTokenManager>,
    client: Client,
}

impl ChatGptBackendProvider {
    pub fn new(token_manager: Arc<CodexTokenManager>) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_default();
        Self {
            token_manager,
            client,
        }
    }

    /// Build the JSON request body for the backend-api conversation endpoint.
    pub fn build_request_body(&self, messages: &[ChatMessage], model: &str) -> Value {
        let (backend_msgs, parent_id) = build_backend_messages(messages);
        json!({
            "action": "next",
            "model": model,
            "parent_message_id": parent_id,
            "messages": backend_msgs,
        })
    }

    /// Get authentication headers (token, optional account_id) from the token manager.
    async fn get_auth_headers(&self) -> Result<(String, Option<String>)> {
        let cred = self
            .token_manager
            .get_credential()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get ChatGPT credential: {}", e))?;
        Ok((cred.access_token, cred.account_id))
    }

    /// Parse a complete SSE response body into a ChatResponse.
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
}

#[async_trait]
impl Provider for ChatGptBackendProvider {
    fn name(&self) -> &str {
        "chatgpt_backend"
    }

    fn default_model(&self) -> &str {
        "gpt-4o"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            native_tools: false,
            vision: true,
        }
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        _tools: &[Value],
        model: &str,
    ) -> Result<ChatResponse> {
        let (token, account_id) = self.get_auth_headers().await?;
        let body = self.build_request_body(messages, model);
        debug!("ChatGPT backend request: model={}", model);

        let mut req = self
            .client
            .post(BACKEND_API_URL)
            .bearer_auth(&token)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream");

        if let Some(ref acct_id) = account_id {
            req = req.header("ChatGPT-Account-Id", acct_id);
        }

        let resp = req.json(&body).send().await?;
        let status = resp.status();

        if !status.is_success() {
            let error_body = resp.text().await.unwrap_or_default();
            anyhow::bail!("ChatGPT backend error {}: {}", status, error_body);
        }

        let sse_body = resp.text().await?;
        Self::parse_full_sse_response(&sse_body)
    }

    async fn stream_chat(
        &self,
        messages: &[ChatMessage],
        _tools: &[Value],
        model: &str,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let (token, account_id) = self.get_auth_headers().await?;
        let body = self.build_request_body(messages, model);

        let mut req = self
            .client
            .post(BACKEND_API_URL)
            .bearer_auth(&token)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream");

        if let Some(ref acct_id) = account_id {
            req = req.header("ChatGPT-Account-Id", acct_id);
        }

        let resp = req.json(&body).send().await?;
        let status = resp.status();

        if !status.is_success() {
            let error_body = resp.text().await.unwrap_or_default();
            anyhow::bail!("ChatGPT backend stream error {}: {}", status, error_body);
        }

        let (tx, rx) = mpsc::channel::<Result<StreamChunk>>(64);
        let byte_stream = resp.bytes_stream();

        tokio::spawn(async move {
            use futures_util::StreamExt;

            let mut stream = byte_stream;
            let mut buffer = String::new();
            let mut last_content = String::new();

            while let Some(chunk_result) = stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx
                            .send(Err(anyhow::anyhow!("Stream read error: {}", e)))
                            .await;
                        break;
                    }
                };

                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(newline_pos) = buffer.find('\n') {
                    let line = buffer[..newline_pos].trim().to_string();
                    buffer = buffer[newline_pos + 1..].to_string();

                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }

                    let data = match line.strip_prefix("data: ") {
                        Some(d) => d,
                        None => continue,
                    };

                    let (content, is_done, _) = parse_backend_sse_line(data);

                    if let Some(ref c) = content {
                        // Backend sends full accumulated text each time; compute delta
                        if c.len() > last_content.len() {
                            let delta = &c[last_content.len()..];
                            if !delta.is_empty() {
                                let _ = tx
                                    .send(Ok(StreamChunk::ContentDelta(delta.to_string())))
                                    .await;
                            }
                        }
                        last_content = c.clone();
                    }

                    if is_done {
                        let _ = tx.send(Ok(StreamChunk::Done { usage: None })).await;
                        return;
                    }
                }
            }

            // If stream ends without explicit [DONE], still send Done
            let _ = tx.send(Ok(StreamChunk::Done { usage: None })).await;
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn is_alive(&self) -> bool {
        match self.token_manager.get_credential().await {
            Ok(cred) => !cred.access_token.is_empty(),
            Err(_) => false,
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Message format translation tests ──

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

    // ── Provider struct tests ──

    #[tokio::test]
    async fn test_provider_name_and_model() {
        let tm = Arc::new(CodexTokenManager::new());
        let provider = ChatGptBackendProvider::new(tm);
        assert_eq!(provider.name(), "chatgpt_backend");
        assert_eq!(provider.default_model(), "gpt-4o");
        assert!(provider.capabilities().streaming);
        assert!(provider.capabilities().vision);
        assert!(!provider.capabilities().native_tools);
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
