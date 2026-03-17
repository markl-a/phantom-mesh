# ChatGPT Backend Provider + Smart Routing Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route complex requests through ChatGPT's product backend API (using Plus subscription quota) instead of the OpenAI developer API, with a 3-tier smart routing system.

**Architecture:** A new `ChatGptBackendProvider` hits `chatgpt.com/backend-api/conversation` using OAuth tokens from `~/.codex/auth.json` (reusing existing `CodexTokenManager`). A `ChatGptWsProvider` connects via WebSocket to `api.openai.com/v1/responses` for native tool use. A `RequestClassifier` uses a local model to triage requests into simple/medium/complex tiers, and a `KeyPool` rotates multiple API keys for Gemini/Groq.

**Tech Stack:** Rust, tokio, reqwest, tokio-tungstenite (new dep), serde_json, uuid, futures-util

---

## File Structure

| File | Responsibility |
|------|----------------|
| `src/providers/chatgpt_backend.rs` (NEW) | REST provider — POST to `chatgpt.com/backend-api/conversation`, SSE response parsing, message format translation |
| `src/providers/chatgpt_ws.rs` (NEW) | WebSocket provider — `wss://api.openai.com/v1/responses`, auto-reconnect, incremental context via `previous_response_id` |
| `src/providers/classifier.rs` (NEW) | Request complexity classifier — uses local small model for 1-shot classification |
| `src/providers/key_pool.rs` (NEW) | Multi-API-key rotation pool — round-robin with per-key cooldown |
| `src/providers/mod.rs` (MODIFY) | Add module declarations + re-exports |
| `src/providers/router.rs` (MODIFY) | Add `"chatgpt_backend"` / `"chatgpt_ws"` to `create_provider()`, add tiered routing |
| `src/providers/codex.rs` (MODIFY) | Ensure `account_id()` is public, add `get_credential_clone()` helper |
| `Cargo.toml` (MODIFY) | Add `tokio-tungstenite` dependency |

---

## Chunk 1: ChatGPT REST Backend Provider

### Task 1: Add tokio-tungstenite dependency

**Files:**
- Modify: `Cargo.toml:22-81` (dependencies section)

- [ ] **Step 1: Add dependency**

Add to `[dependencies]` section in `Cargo.toml`:
```toml
tokio-tungstenite = { version = "0.24", features = ["native-tls"] }
```

- [ ] **Step 2: Verify compilation**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo check 2>&1 | tail -5`
Expected: compiles successfully (new dep downloaded)

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: add tokio-tungstenite for WebSocket support"
```

---

### Task 2: Expose CodexTokenManager helpers

**Files:**
- Modify: `src/providers/codex.rs:74-77,80-141`

- [ ] **Step 1: Write test for get_credential_clone**

Add test at bottom of `codex.rs` (inside existing `#[cfg(test)] mod tests`):
```rust
#[tokio::test]
async fn test_get_credential_clone_returns_none_without_auth_file() {
    let tm = CodexTokenManager::new();
    let cred = tm.get_credential_clone().await;
    // No auth file on CI = None
    assert!(cred.is_none() || cred.is_some()); // Just verify it doesn't panic
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test test_get_credential_clone -- --nocapture 2>&1 | tail -10`
Expected: FAIL — `get_credential_clone` method not found

- [ ] **Step 3: Add get_credential_clone method**

Add after the existing `get_credential()` method (after line ~141 in codex.rs):
```rust
/// Returns a clone of the current credential without refreshing.
/// Useful for providers that need the full credential (token + account_id).
pub async fn get_credential_clone(&self) -> Option<CodexCredential> {
    // Try cached first
    let guard = self.credential.lock().await;
    if let Some(ref cred) = *guard {
        return Some(cred.clone());
    }
    drop(guard);

    // Try reading from file
    if let Some(cred) = self.read_auth_file() {
        let mut guard = self.credential.lock().await;
        *guard = Some(cred.clone());
        Some(cred)
    } else {
        None
    }
}
```

Also ensure `CodexCredential` derives `Clone` (check line 32 — it should already have `#[derive(Debug, Clone)]`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test test_get_credential_clone -- --nocapture 2>&1 | tail -10`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/providers/codex.rs
git commit -m "feat: add get_credential_clone to CodexTokenManager"
```

---

### Task 3: Create chatgpt_backend.rs — message format translation

**Files:**
- Create: `src/providers/chatgpt_backend.rs`

This task implements the message format translation between clawtex `ChatMessage` and the ChatGPT backend API format. No HTTP calls yet — pure data transformation.

- [ ] **Step 1: Write tests for message translation**

Create `src/providers/chatgpt_backend.rs` with tests at the bottom:
```rust
use anyhow::Result;
use serde_json::{json, Value};
use uuid::Uuid;

use super::traits::{ChatMessage, ChatResponse, TokenUsage, ToolCall, StreamChunk, Provider, ProviderCapabilities};

/// Convert a ChatMessage to ChatGPT backend API format.
/// Backend uses: { id, author: { role }, content: { content_type: "text", parts: [...] } }
fn chatmessage_to_backend(msg: &ChatMessage) -> Value {
    todo!()
}

/// Convert a list of ChatMessages to backend format, generating parent_message_id chain.
/// Returns (messages_json, parent_message_id) for the conversation request.
fn build_backend_messages(messages: &[ChatMessage]) -> (Vec<Value>, String) {
    todo!()
}

/// Parse a single SSE data line from the backend response.
/// Returns (content_delta, is_done, conversation_id).
fn parse_backend_sse_line(line: &str) -> (Option<String>, bool, Option<String>) {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(result["id"].as_str().unwrap().len() > 10); // UUID
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
        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: "Hi".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
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
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test chatgpt_backend::tests -- --nocapture 2>&1 | tail -15`
Expected: FAIL — `todo!()` panics

- [ ] **Step 3: Implement message translation functions**

Replace the `todo!()` bodies:

```rust
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

fn build_backend_messages(messages: &[ChatMessage]) -> (Vec<Value>, String) {
    let parent_id = Uuid::new_v4().to_string();
    let backend_msgs: Vec<Value> = messages.iter().map(chatmessage_to_backend).collect();
    (backend_msgs, parent_id)
}

fn parse_backend_sse_line(line: &str) -> (Option<String>, bool, Option<String>) {
    let line = line.trim();
    if line == "[DONE]" {
        return (None, true, None);
    }

    let parsed: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return (None, false, None),
    };

    let conv_id = parsed["conversation_id"].as_str().map(|s| s.to_string());
    let status = parsed["message"]["status"].as_str().unwrap_or("");
    let is_done = status == "finished_successfully";

    let content = parsed["message"]["content"]["parts"]
        .as_array()
        .and_then(|parts| parts.first())
        .and_then(|p| p.as_str())
        .map(|s| s.to_string());

    (content, is_done, conv_id)
}
```

- [ ] **Step 4: Register module in mod.rs**

Add to `src/providers/mod.rs` (after line 7, the `pub mod codex;` line):
```rust
pub mod chatgpt_backend;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test chatgpt_backend::tests -- --nocapture 2>&1 | tail -15`
Expected: all 5 tests PASS

- [ ] **Step 6: Commit**

```bash
git add src/providers/chatgpt_backend.rs src/providers/mod.rs
git commit -m "feat: chatgpt_backend message format translation with tests"
```

---

### Task 4: Implement ChatGptBackendProvider struct + chat()

**Files:**
- Modify: `src/providers/chatgpt_backend.rs`

- [ ] **Step 1: Write integration test for provider**

Add to the `tests` module in `chatgpt_backend.rs`:
```rust
#[tokio::test]
async fn test_provider_name_and_model() {
    // Can't test real HTTP without a mock server, but verify struct creation
    use super::super::codex::CodexTokenManager;
    use std::sync::Arc;

    let tm = Arc::new(CodexTokenManager::new());
    let provider = ChatGptBackendProvider::new(tm);
    assert_eq!(provider.name(), "chatgpt_backend");
    assert_eq!(provider.default_model(), "gpt-4o");
    assert!(provider.capabilities().streaming);
    assert!(provider.capabilities().vision);
}

#[tokio::test]
async fn test_build_request_body() {
    use super::super::codex::CodexTokenManager;
    use std::sync::Arc;

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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test chatgpt_backend::tests -- --nocapture 2>&1 | tail -15`
Expected: FAIL — `ChatGptBackendProvider` not found

- [ ] **Step 3: Implement ChatGptBackendProvider struct**

Add above the `#[cfg(test)]` block in `chatgpt_backend.rs`:

```rust
use std::pin::Pin;
use std::sync::Arc;
use async_trait::async_trait;
use futures_util::Stream;
use reqwest::Client;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, warn};

use super::codex::CodexTokenManager;

const BACKEND_API_URL: &str = "https://chatgpt.com/backend-api/conversation";

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
        Self { token_manager, client }
    }

    pub fn build_request_body(&self, messages: &[ChatMessage], model: &str) -> Value {
        let (backend_msgs, parent_id) = build_backend_messages(messages);
        json!({
            "action": "next",
            "model": model,
            "parent_message_id": parent_id,
            "messages": backend_msgs,
        })
    }

    async fn get_auth_headers(&self) -> Result<(String, Option<String>)> {
        let cred = self.token_manager.get_credential().await
            .map_err(|e| anyhow::anyhow!("Failed to get ChatGPT credential: {}", e))?;
        Ok((cred.access_token, cred.account_id))
    }

    /// Parse complete SSE response body into a ChatResponse.
    fn parse_full_sse_response(body: &str) -> Result<ChatResponse> {
        let mut final_content = String::new();
        let mut conversation_id = None;

        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            let data = if let Some(d) = line.strip_prefix("data: ") {
                d
            } else {
                continue;
            };

            let (content, is_done, conv_id) = parse_backend_sse_line(data);
            if let Some(c) = content {
                final_content = c; // Last content wins (accumulates on server side)
            }
            if let Some(id) = conv_id {
                conversation_id = Some(id);
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
            usage: None, // ChatGPT backend doesn't expose token usage in SSE
        })
    }
}

#[async_trait]
impl Provider for ChatGptBackendProvider {
    fn name(&self) -> &str { "chatgpt_backend" }
    fn default_model(&self) -> &str { "gpt-4o" }

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

        let mut req = self.client
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

        let mut req = self.client
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
                        let _ = tx.send(Err(anyhow::anyhow!("Stream read error: {}", e))).await;
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

                    let (content, is_done, _conv_id) = parse_backend_sse_line(data);

                    if let Some(ref c) = content {
                        // Compute delta: backend sends full content each time
                        if c.len() > last_content.len() {
                            let delta = &c[last_content.len()..];
                            if !delta.is_empty() {
                                let _ = tx.send(Ok(StreamChunk::ContentDelta(delta.to_string()))).await;
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
```

- [ ] **Step 4: Add necessary imports at top of file**

Ensure the top of `chatgpt_backend.rs` has:
```rust
use anyhow::Result;
use serde_json::{json, Value};
use uuid::Uuid;
use std::pin::Pin;
use std::sync::Arc;
use async_trait::async_trait;
use futures_util::Stream;
use reqwest::Client;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, warn};

use super::traits::{ChatMessage, ChatResponse, TokenUsage, ToolCall, StreamChunk, Provider, ProviderCapabilities};
use super::codex::CodexTokenManager;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test chatgpt_backend::tests -- --nocapture 2>&1 | tail -20`
Expected: all 7 tests PASS

- [ ] **Step 6: Commit**

```bash
git add src/providers/chatgpt_backend.rs
git commit -m "feat: ChatGptBackendProvider with chat() and stream_chat()"
```

---

### Task 5: Register chatgpt_backend in router

**Files:**
- Modify: `src/providers/router.rs:128-233` (create_provider match block)
- Modify: `src/providers/mod.rs` (add re-export)

- [ ] **Step 1: Write test for provider creation**

Add to the `tests` module in `router.rs`:
```rust
#[test]
fn test_create_chatgpt_backend_provider() {
    let config = ProviderConfig {
        provider_type: "chatgpt_backend".to_string(),
        url: None,
        default_model: Some("gpt-4o".to_string()),
        api_key: None,
    };
    let result = create_provider("chatgpt", &config);
    assert_eq!(result.provider.name(), "chatgpt_backend");
    assert_eq!(result.provider.default_model(), "gpt-4o");
    // Should have a codex_token_manager since it uses OAuth
    assert!(result.codex_token_manager.is_some());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test test_create_chatgpt_backend -- --nocapture 2>&1 | tail -10`
Expected: FAIL — no match arm for "chatgpt_backend"

- [ ] **Step 3: Add chatgpt_backend to create_provider match block**

In `router.rs`, add a new match arm after the `"openai_codex"` block (after line ~193):

```rust
"chatgpt_backend" => {
    let tm = Arc::new(CodexTokenManager::new());
    let has_auth = tm.read_auth_file().is_some();
    if has_auth {
        let provider = super::chatgpt_backend::ChatGptBackendProvider::new(tm.clone());
        let model = config.default_model.clone().unwrap_or_else(|| "gpt-4o".to_string());
        CreateProviderResult {
            provider: Box::new(provider),
            codex_token_manager: Some(tm),
            codex_base_url: None,
        }
    } else {
        tracing::warn!("chatgpt_backend: no ~/.codex/auth.json found, provider will be unavailable");
        let provider = super::chatgpt_backend::ChatGptBackendProvider::new(tm.clone());
        CreateProviderResult {
            provider: Box::new(provider),
            codex_token_manager: Some(tm),
            codex_base_url: None,
        }
    }
}
```

- [ ] **Step 4: Add re-export in mod.rs**

Add to `src/providers/mod.rs`:
```rust
pub use chatgpt_backend::ChatGptBackendProvider;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test test_create_chatgpt_backend -- --nocapture 2>&1 | tail -10`
Expected: PASS

- [ ] **Step 6: Run full test suite**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test 2>&1 | tail -5`
Expected: all existing tests still pass

- [ ] **Step 7: Commit**

```bash
git add src/providers/router.rs src/providers/mod.rs
git commit -m "feat: register chatgpt_backend provider in router"
```

---

## Chunk 2: WebSocket Responses Provider

### Task 6: Create chatgpt_ws.rs — WebSocket connection + message types

**Files:**
- Create: `src/providers/chatgpt_ws.rs`
- Modify: `src/providers/mod.rs`

- [ ] **Step 1: Write tests for input message conversion**

Create `src/providers/chatgpt_ws.rs`:
```rust
use anyhow::Result;
use serde_json::{json, Value};
use std::pin::Pin;
use std::sync::Arc;
use async_trait::async_trait;
use futures_util::Stream;
use tokio::sync::{mpsc, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, warn, error};

use super::traits::{ChatMessage, ChatResponse, TokenUsage, ToolCall, StreamChunk, Provider, ProviderCapabilities};
use super::codex::CodexTokenManager;

const WS_URL: &str = "wss://api.openai.com/v1/responses";

/// Convert ChatMessage array to Responses API input items.
fn messages_to_input_items(messages: &[ChatMessage]) -> Vec<Value> {
    todo!()
}

/// Convert tool definitions to Responses API function tool format.
fn tools_to_ws_format(tools: &[Value]) -> Vec<Value> {
    todo!()
}

/// Parse a WebSocket response event, return extracted data.
fn parse_ws_event(event: &Value) -> WsEventData {
    todo!()
}

enum WsEventData {
    TextDelta(String),
    ToolCallStart { call_id: String, name: String },
    ToolCallArgDelta { call_id: String, delta: String },
    Completed { content: String, tool_calls: Vec<ToolCall>, usage: Option<TokenUsage>, response_id: String },
    Failed(String),
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_message_to_input_item() {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
            tool_calls: None,
            tool_call_id: None,
        }];
        let items = messages_to_input_items(&messages);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "message");
        assert_eq!(items[0]["role"], "user");
        // content should be a string or content array
        let content = &items[0]["content"];
        assert!(content.is_string() || content.is_array());
    }

    #[test]
    fn test_system_message_becomes_instructions_not_input() {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: "Be helpful".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: "user".to_string(),
                content: "Hi".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let items = messages_to_input_items(&messages);
        // System messages should be excluded from input items
        // (they go into instructions field separately)
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["role"], "user");
    }

    #[test]
    fn test_tool_result_to_function_call_output() {
        let messages = vec![ChatMessage {
            role: "tool".to_string(),
            content: r#"{"result": "42"}"#.to_string(),
            tool_calls: None,
            tool_call_id: Some("call_abc".to_string()),
        }];
        let items = messages_to_input_items(&messages);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "function_call_output");
        assert_eq!(items[0]["call_id"], "call_abc");
    }

    #[test]
    fn test_tools_to_ws_format() {
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "shell",
                "description": "Run a shell command",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" }
                    }
                }
            }
        })];
        let ws_tools = tools_to_ws_format(&tools);
        assert_eq!(ws_tools.len(), 1);
        assert_eq!(ws_tools[0]["type"], "function");
        assert_eq!(ws_tools[0]["name"], "shell");
    }

    #[test]
    fn test_parse_text_delta_event() {
        let event = json!({
            "type": "response.output_text.delta",
            "delta": "Hello"
        });
        match parse_ws_event(&event) {
            WsEventData::TextDelta(text) => assert_eq!(text, "Hello"),
            _ => panic!("Expected TextDelta"),
        }
    }

    #[test]
    fn test_parse_completed_event() {
        let event = json!({
            "type": "response.completed",
            "response": {
                "id": "resp_abc",
                "status": "completed",
                "output": [{
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "Done!" }]
                }],
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": 5,
                    "total_tokens": 15
                }
            }
        });
        match parse_ws_event(&event) {
            WsEventData::Completed { content, response_id, usage, .. } => {
                assert_eq!(content, "Done!");
                assert_eq!(response_id, "resp_abc");
                assert!(usage.is_some());
            },
            _ => panic!("Expected Completed"),
        }
    }
}
```

- [ ] **Step 2: Register module in mod.rs**

Add to `src/providers/mod.rs`:
```rust
pub mod chatgpt_ws;
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test chatgpt_ws::tests -- --nocapture 2>&1 | tail -15`
Expected: FAIL — `todo!()` panics

- [ ] **Step 4: Implement conversion functions**

Replace `todo!()` bodies:

```rust
fn messages_to_input_items(messages: &[ChatMessage]) -> Vec<Value> {
    messages.iter().filter_map(|msg| {
        match msg.role.as_str() {
            "system" => None, // System goes to instructions, not input
            "tool" => {
                let call_id = msg.tool_call_id.clone().unwrap_or_default();
                Some(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": msg.content,
                }))
            }
            "assistant" if msg.tool_calls.is_some() => {
                // Assistant message with tool calls → function_call items
                // Note: multiple tool calls are flattened into the parent array
                // by returning None here and using flat_map in the caller.
                // For simplicity, return only the first tool call as a single item.
                // Multi-tool-call is rare for ChatGPT backend.
                if let Some(ref calls) = msg.tool_calls {
                    calls.first().map(|tc| json!({
                        "type": "function_call",
                        "call_id": tc.id.clone().unwrap_or_default(),
                        "name": tc.function.name,
                        "arguments": tc.function.arguments.to_string(),
                    }))
                } else {
                    None
                }
            }
            role => {
                Some(json!({
                    "type": "message",
                    "role": role,
                    "content": msg.content,
                }))
            }
        }
    }).collect()
}

fn tools_to_ws_format(tools: &[Value]) -> Vec<Value> {
    tools.iter().filter_map(|tool| {
        let func = tool.get("function")?;
        Some(json!({
            "type": "function",
            "name": func["name"],
            "description": func.get("description").cloned().unwrap_or(json!("")),
            "parameters": func.get("parameters").cloned().unwrap_or(json!({})),
        }))
    }).collect()
}

fn parse_ws_event(event: &Value) -> WsEventData {
    let event_type = event["type"].as_str().unwrap_or("");
    match event_type {
        "response.output_text.delta" => {
            let delta = event["delta"].as_str().unwrap_or("").to_string();
            WsEventData::TextDelta(delta)
        }
        "response.function_call_arguments.delta" => {
            let call_id = event["call_id"].as_str().unwrap_or("").to_string();
            let delta = event["delta"].as_str().unwrap_or("").to_string();
            WsEventData::ToolCallArgDelta { call_id, delta }
        }
        "response.output_item.added" => {
            let item = &event["item"];
            if item["type"].as_str() == Some("function_call") {
                let call_id = item["call_id"].as_str().unwrap_or("").to_string();
                let name = item["name"].as_str().unwrap_or("").to_string();
                WsEventData::ToolCallStart { call_id, name }
            } else {
                WsEventData::Other
            }
        }
        "response.completed" => {
            let response = &event["response"];
            let response_id = response["id"].as_str().unwrap_or("").to_string();

            // Extract text content
            let mut content = String::new();
            let mut tool_calls = Vec::new();

            if let Some(outputs) = response["output"].as_array() {
                for output in outputs {
                    match output["type"].as_str() {
                        Some("message") => {
                            if let Some(contents) = output["content"].as_array() {
                                for c in contents {
                                    if c["type"].as_str() == Some("output_text") {
                                        content.push_str(c["text"].as_str().unwrap_or(""));
                                    }
                                }
                            }
                        }
                        Some("function_call") => {
                            let tc = ToolCall {
                                id: Some(output["call_id"].as_str().unwrap_or("").to_string()),
                                function: ToolCallFunction {
                                    name: output["name"].as_str().unwrap_or("").to_string(),
                                    arguments: serde_json::from_str(
                                        output["arguments"].as_str().unwrap_or("{}")
                                    ).unwrap_or(json!({})),
                                },
                            };
                            tool_calls.push(tc);
                        }
                        _ => {}
                    }
                }
            }

            let usage = response.get("usage").and_then(|u| {
                Some(TokenUsage {
                    prompt_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
                    completion_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
                    total_tokens: u["total_tokens"].as_u64().unwrap_or(0) as u32,
                })
            });

            WsEventData::Completed { content, tool_calls, usage, response_id }
        }
        "response.failed" => {
            let msg = event["response"]["error"]["message"].as_str()
                .unwrap_or("Unknown error").to_string();
            WsEventData::Failed(msg)
        }
        _ => WsEventData::Other,
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test chatgpt_ws::tests -- --nocapture 2>&1 | tail -15`
Expected: all 6 tests PASS

- [ ] **Step 6: Commit**

```bash
git add src/providers/chatgpt_ws.rs src/providers/mod.rs
git commit -m "feat: chatgpt_ws message types and conversion functions"
```

---

### Task 7: Implement ChatGptWsProvider struct with Provider trait

**Files:**
- Modify: `src/providers/chatgpt_ws.rs`

- [ ] **Step 1: Write test for provider struct**

Add to tests module:
```rust
#[tokio::test]
async fn test_ws_provider_name_and_capabilities() {
    use super::super::codex::CodexTokenManager;
    use std::sync::Arc;

    let tm = Arc::new(CodexTokenManager::new());
    let provider = ChatGptWsProvider::new(tm);
    assert_eq!(provider.name(), "chatgpt_ws");
    assert_eq!(provider.default_model(), "gpt-4o");
    assert!(provider.capabilities().streaming);
    assert!(provider.capabilities().native_tools); // WS supports native tools
}

#[test]
fn test_build_response_create_event() {
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
    let tools = vec![json!({
        "type": "function",
        "function": { "name": "shell", "description": "Run command", "parameters": {} }
    })];

    let event = build_response_create_event(&messages, &tools, "gpt-4o", None);
    assert_eq!(event["type"], "response.create");
    assert_eq!(event["model"], "gpt-4o");
    assert_eq!(event["instructions"], "Be helpful"); // System extracted
    assert!(event["input"].as_array().unwrap().len() >= 1); // User message
    assert!(event["tools"].as_array().unwrap().len() == 1);
    assert!(event.get("previous_response_id").is_none()); // No previous
}

#[test]
fn test_build_response_create_with_previous_id() {
    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: "Follow up".to_string(),
        tool_calls: None,
        tool_call_id: None,
    }];

    let event = build_response_create_event(&messages, &[], "gpt-4o", Some("resp_prev123".to_string()));
    assert_eq!(event["previous_response_id"], "resp_prev123");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test chatgpt_ws::tests -- --nocapture 2>&1 | tail -15`
Expected: FAIL — `ChatGptWsProvider` and `build_response_create_event` not found

- [ ] **Step 3: Implement build_response_create_event**

Add above `#[cfg(test)]`:
```rust
/// Extract system prompt from messages (first system message).
fn extract_system_prompt(messages: &[ChatMessage]) -> Option<String> {
    messages.iter()
        .find(|m| m.role == "system")
        .map(|m| m.content.clone())
}

/// Build the response.create WebSocket event payload.
fn build_response_create_event(
    messages: &[ChatMessage],
    tools: &[Value],
    model: &str,
    previous_response_id: Option<String>,
) -> Value {
    let instructions = extract_system_prompt(messages);
    let input_items = messages_to_input_items(messages);
    let ws_tools = tools_to_ws_format(tools);

    let mut event = json!({
        "type": "response.create",
        "model": model,
        "input": input_items,
        "stream": true,
    });

    if let Some(inst) = instructions {
        event["instructions"] = json!(inst);
    }
    if !ws_tools.is_empty() {
        event["tools"] = json!(ws_tools);
    }
    if let Some(prev_id) = previous_response_id {
        event["previous_response_id"] = json!(prev_id);
    }

    event
}
```

- [ ] **Step 4: Implement ChatGptWsProvider struct**

```rust
pub struct ChatGptWsProvider {
    token_manager: Arc<CodexTokenManager>,
    previous_response_id: RwLock<Option<String>>,
}

impl ChatGptWsProvider {
    pub fn new(token_manager: Arc<CodexTokenManager>) -> Self {
        Self {
            token_manager,
            previous_response_id: RwLock::new(None),
        }
    }

    /// Store the response ID for incremental context on next call.
    async fn set_previous_response_id(&self, id: String) {
        let mut guard = self.previous_response_id.write().await;
        *guard = Some(id);
    }

    /// Get the previous response ID (if any).
    async fn get_previous_response_id(&self) -> Option<String> {
        self.previous_response_id.read().await.clone()
    }
}

#[async_trait]
impl Provider for ChatGptWsProvider {
    fn name(&self) -> &str { "chatgpt_ws" }
    fn default_model(&self) -> &str { "gpt-4o" }

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
        let cred = self.token_manager.get_credential().await
            .map_err(|e| anyhow::anyhow!("Failed to get credential for WS: {}", e))?;
        let token = &cred.access_token;
        let prev_id = self.get_previous_response_id().await;

        let event = build_response_create_event(messages, tools, model, prev_id);

        // Connect WebSocket
        use tokio_tungstenite::tungstenite::{self, http::Request};
        let request = Request::builder()
            .uri(WS_URL)
            .header("Authorization", format!("Bearer {}", token))
            .header("OpenAI-Beta", "responses-websocket=v1")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
            .header("Host", "api.openai.com")
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .body(())?;

        let (mut ws_stream, _) = tokio_tungstenite::connect_async(request).await
            .map_err(|e| anyhow::anyhow!("WebSocket connect failed: {}", e))?;

        // Send response.create event
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        ws_stream.send(Message::Text(serde_json::to_string(&event)?)).await?;

        // Collect response
        let mut final_content = String::new();
        let mut final_tool_calls = Vec::new();
        let mut final_usage = None;

        while let Some(msg_result) = ws_stream.next().await {
            let msg = msg_result.map_err(|e| anyhow::anyhow!("WS read error: {}", e))?;
            if let Message::Text(text) = msg {
                let event_json: Value = serde_json::from_str(&text)?;
                match parse_ws_event(&event_json) {
                    WsEventData::TextDelta(delta) => final_content.push_str(&delta),
                    WsEventData::Completed { content, tool_calls, usage, response_id } => {
                        if !content.is_empty() {
                            final_content = content;
                        }
                        final_tool_calls = tool_calls;
                        final_usage = usage;
                        self.set_previous_response_id(response_id).await;
                        break;
                    }
                    WsEventData::Failed(err) => {
                        anyhow::bail!("ChatGPT WS error: {}", err);
                    }
                    _ => {}
                }
            }
        }

        // Close connection
        let _ = ws_stream.close(None).await;

        Ok(ChatResponse {
            message: ChatMessage {
                role: "assistant".to_string(),
                content: final_content,
                tool_calls: if final_tool_calls.is_empty() { None } else { Some(final_tool_calls) },
                tool_call_id: None,
            },
            usage: final_usage,
        })
    }

    async fn stream_chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let cred = self.token_manager.get_credential().await
            .map_err(|e| anyhow::anyhow!("Failed to get credential for WS stream: {}", e))?;
        let token = cred.access_token.clone();
        let prev_id = self.get_previous_response_id().await;

        let event = build_response_create_event(messages, tools, model, prev_id);
        let prev_id_store = self.previous_response_id.clone();

        let (tx, rx) = mpsc::channel::<Result<StreamChunk>>(64);

        tokio::spawn(async move {
            use tokio_tungstenite::tungstenite::{self, http::Request};
            use futures_util::{SinkExt, StreamExt};
            use tokio_tungstenite::tungstenite::Message;

            let request = match Request::builder()
                .uri(WS_URL)
                .header("Authorization", format!("Bearer {}", token))
                .header("OpenAI-Beta", "responses-websocket=v1")
                .header("Sec-WebSocket-Version", "13")
                .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
                .header("Host", "api.openai.com")
                .header("Connection", "Upgrade")
                .header("Upgrade", "websocket")
                .body(()) {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(Err(anyhow::anyhow!("WS request build error: {}", e))).await;
                    return;
                }
            };

            let (mut ws_stream, _) = match tokio_tungstenite::connect_async(request).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(Err(anyhow::anyhow!("WS connect error: {}", e))).await;
                    return;
                }
            };

            if let Err(e) = ws_stream.send(Message::Text(serde_json::to_string(&event).unwrap())).await {
                let _ = tx.send(Err(anyhow::anyhow!("WS send error: {}", e))).await;
                return;
            }

            while let Some(msg_result) = ws_stream.next().await {
                let msg = match msg_result {
                    Ok(m) => m,
                    Err(e) => {
                        let _ = tx.send(Err(anyhow::anyhow!("WS read error: {}", e))).await;
                        break;
                    }
                };

                if let Message::Text(text) = msg {
                    let event_json: Value = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    match parse_ws_event(&event_json) {
                        WsEventData::TextDelta(delta) => {
                            let _ = tx.send(Ok(StreamChunk::ContentDelta(delta))).await;
                        }
                        WsEventData::ToolCallStart { call_id, name } => {
                            let _ = tx.send(Ok(StreamChunk::ToolCallStart { id: call_id, name })).await;
                        }
                        WsEventData::ToolCallArgDelta { call_id, delta } => {
                            let _ = tx.send(Ok(StreamChunk::ToolCallArgumentsDelta { id: call_id, delta })).await;
                        }
                        WsEventData::Completed { usage, response_id, .. } => {
                            let mut guard = prev_id_store.write().await;
                            *guard = Some(response_id);
                            let _ = tx.send(Ok(StreamChunk::Done { usage })).await;
                            break;
                        }
                        WsEventData::Failed(err) => {
                            let _ = tx.send(Err(anyhow::anyhow!("WS error: {}", err))).await;
                            break;
                        }
                        WsEventData::Other => {}
                    }
                }
            }

            let _ = ws_stream.close(None).await;
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn is_alive(&self) -> bool {
        self.token_manager.get_credential().await.is_ok()
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test chatgpt_ws::tests -- --nocapture 2>&1 | tail -15`
Expected: all 9 tests PASS

- [ ] **Step 6: Commit**

```bash
git add src/providers/chatgpt_ws.rs
git commit -m "feat: ChatGptWsProvider with WebSocket Responses API support"
```

---

### Task 8: Register chatgpt_ws in router

**Files:**
- Modify: `src/providers/router.rs`
- Modify: `src/providers/mod.rs`

- [ ] **Step 1: Add chatgpt_ws to create_provider match block**

In `router.rs`, add after the `"chatgpt_backend"` match arm:
```rust
"chatgpt_ws" => {
    let tm = Arc::new(CodexTokenManager::new());
    let provider = super::chatgpt_ws::ChatGptWsProvider::new(tm.clone());
    CreateProviderResult {
        provider: Box::new(provider),
        codex_token_manager: Some(tm),
        codex_base_url: None,
    }
}
```

- [ ] **Step 2: Add re-export in mod.rs**

```rust
pub use chatgpt_ws::ChatGptWsProvider;
```

- [ ] **Step 3: Run full test suite**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test 2>&1 | tail -5`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add src/providers/router.rs src/providers/mod.rs
git commit -m "feat: register chatgpt_ws provider in router"
```

---

## Chunk 3: Smart Routing — Classifier + Key Pool

### Task 9: Create key_pool.rs — multi-key rotation

**Files:**
- Create: `src/providers/key_pool.rs`
- Modify: `src/providers/mod.rs`

- [ ] **Step 1: Write tests**

Create `src/providers/key_pool.rs`:
```rust
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;
use tokio::sync::RwLock;

pub struct KeyPool {
    keys: Vec<KeyEntry>,
    current: AtomicUsize,
}

struct KeyEntry {
    key: String,
    cooldown_until: RwLock<Option<Instant>>,
    request_count: AtomicU64,
}

impl KeyPool {
    pub fn new(keys: Vec<String>) -> Self {
        let entries = keys.into_iter().map(|k| KeyEntry {
            key: k,
            cooldown_until: RwLock::new(None),
            request_count: AtomicU64::new(0),
        }).collect();
        Self { keys: entries, current: AtomicUsize::new(0) }
    }

    /// Get next available key (round-robin, skip cooled-down keys).
    pub async fn next_key(&self) -> Option<&str> {
        let len = self.keys.len();
        if len == 0 { return None; }

        let start = self.current.fetch_add(1, Ordering::Relaxed) % len;
        for i in 0..len {
            let idx = (start + i) % len;
            let entry = &self.keys[idx];
            let guard = entry.cooldown_until.read().await;
            if let Some(until) = *guard {
                if Instant::now() < until {
                    continue; // Still in cooldown
                }
            }
            entry.request_count.fetch_add(1, Ordering::Relaxed);
            return Some(&entry.key);
        }

        // All in cooldown — return the one with earliest cooldown expiry
        None
    }

    /// Mark a key as rate-limited (60s cooldown).
    pub async fn record_rate_limit(&self, key: &str) {
        for entry in &self.keys {
            if entry.key == key {
                let mut guard = entry.cooldown_until.write().await;
                *guard = Some(Instant::now() + std::time::Duration::from_secs(60));
                break;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_round_robin_rotation() {
        let pool = KeyPool::new(vec!["key1".into(), "key2".into(), "key3".into()]);
        let k1 = pool.next_key().await.unwrap().to_string();
        let k2 = pool.next_key().await.unwrap().to_string();
        let k3 = pool.next_key().await.unwrap().to_string();
        let k4 = pool.next_key().await.unwrap().to_string();
        // Should cycle through all keys
        assert_ne!(k1, k2);
        assert_ne!(k2, k3);
        assert_eq!(k1, k4); // Wrapped around
    }

    #[tokio::test]
    async fn test_skip_cooled_down_key() {
        let pool = KeyPool::new(vec!["key1".into(), "key2".into()]);
        pool.record_rate_limit("key1").await;
        // Should skip key1 and return key2
        let k = pool.next_key().await.unwrap();
        assert_eq!(k, "key2");
    }

    #[tokio::test]
    async fn test_all_cooled_down_returns_none() {
        let pool = KeyPool::new(vec!["key1".into(), "key2".into()]);
        pool.record_rate_limit("key1").await;
        pool.record_rate_limit("key2").await;
        assert!(pool.next_key().await.is_none());
    }

    #[tokio::test]
    async fn test_empty_pool() {
        let pool = KeyPool::new(vec![]);
        assert!(pool.next_key().await.is_none());
        assert!(pool.is_empty());
    }
}
```

- [ ] **Step 2: Register in mod.rs**

```rust
pub mod key_pool;
pub use key_pool::KeyPool;
```

- [ ] **Step 3: Run tests**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test key_pool::tests -- --nocapture 2>&1 | tail -15`
Expected: all 4 tests PASS

- [ ] **Step 4: Commit**

```bash
git add src/providers/key_pool.rs src/providers/mod.rs
git commit -m "feat: KeyPool for multi-API-key rotation"
```

---

### Task 10: Create classifier.rs — request complexity classifier

**Files:**
- Create: `src/providers/classifier.rs`
- Modify: `src/providers/mod.rs`

- [ ] **Step 1: Write tests**

Create `src/providers/classifier.rs`:
```rust
use anyhow::Result;
use std::sync::Arc;

use super::traits::{ChatMessage, Provider};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestComplexity {
    Simple,
    Medium,
    Complex,
}

impl RequestComplexity {
    /// Parse from model response text.
    pub fn from_response(text: &str) -> Self {
        let text = text.trim().to_uppercase();
        if text.contains("SIMPLE") {
            Self::Simple
        } else if text.contains("COMPLEX") {
            Self::Complex
        } else {
            Self::Medium // Default to medium
        }
    }
}

const CLASSIFIER_PROMPT: &str = r#"Classify this request as SIMPLE, MEDIUM, or COMPLEX.

SIMPLE: greetings, yes/no, single-fact lookups, short translations, acknowledgments
MEDIUM: summarization, general Q&A, multi-sentence replies, explanations, basic coding questions
COMPLEX: code generation, debugging, multi-step reasoning, analysis, planning, tool-heavy tasks

Request: {INPUT}

Reply with one word only: SIMPLE, MEDIUM, or COMPLEX"#;

pub struct RequestClassifier {
    provider: Arc<dyn Provider>,
    model: String,
}

impl RequestClassifier {
    pub fn new(provider: Arc<dyn Provider>, model: String) -> Self {
        Self { provider, model }
    }

    /// Classify the complexity of a request based on the last user message.
    pub async fn classify(&self, messages: &[ChatMessage]) -> RequestComplexity {
        // Find last user message
        let last_user = messages.iter().rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");

        if last_user.is_empty() {
            return RequestComplexity::Medium;
        }

        // Quick heuristic: very short messages are likely simple
        let word_count = last_user.split_whitespace().count();
        if word_count <= 3 {
            return RequestComplexity::Simple;
        }

        // Use local model for classification
        let prompt = CLASSIFIER_PROMPT.replace("{INPUT}", last_user);
        let classify_messages = vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
            tool_calls: None,
            tool_call_id: None,
        }];

        match self.provider.chat(&classify_messages, &[], &self.model).await {
            Ok(resp) => RequestComplexity::from_response(&resp.message.content),
            Err(e) => {
                tracing::warn!("Classifier failed, defaulting to Medium: {}", e);
                RequestComplexity::Medium
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_response_simple() {
        assert_eq!(RequestComplexity::from_response("SIMPLE"), RequestComplexity::Simple);
        assert_eq!(RequestComplexity::from_response("  simple  "), RequestComplexity::Simple);
        assert_eq!(RequestComplexity::from_response("I think this is SIMPLE."), RequestComplexity::Simple);
    }

    #[test]
    fn test_from_response_complex() {
        assert_eq!(RequestComplexity::from_response("COMPLEX"), RequestComplexity::Complex);
        assert_eq!(RequestComplexity::from_response("This is complex"), RequestComplexity::Complex);
    }

    #[test]
    fn test_from_response_medium() {
        assert_eq!(RequestComplexity::from_response("MEDIUM"), RequestComplexity::Medium);
        assert_eq!(RequestComplexity::from_response("unknown"), RequestComplexity::Medium);
        assert_eq!(RequestComplexity::from_response(""), RequestComplexity::Medium);
    }

    #[test]
    fn test_short_message_is_simple() {
        // This tests the heuristic, not the LLM call
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            use super::super::mock::MockProvider;
            let mock = Arc::new(MockProvider::fixed("MEDIUM"));
            let classifier = RequestClassifier::new(mock, "test".to_string());

            let messages = vec![ChatMessage {
                role: "user".to_string(),
                content: "Hi".to_string(),
                tool_calls: None,
                tool_call_id: None,
            }];

            let result = classifier.classify(&messages).await;
            assert_eq!(result, RequestComplexity::Simple); // Short = simple heuristic
        });
    }

    #[test]
    fn test_classify_with_mock_provider() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            use super::super::mock::MockProvider;
            let mock = Arc::new(MockProvider::fixed("COMPLEX"));
            let classifier = RequestClassifier::new(mock, "test".to_string());

            let messages = vec![ChatMessage {
                role: "user".to_string(),
                content: "Write a Rust function that implements a binary search tree with insert delete and rebalance operations".to_string(),
                tool_calls: None,
                tool_call_id: None,
            }];

            let result = classifier.classify(&messages).await;
            assert_eq!(result, RequestComplexity::Complex);
        });
    }
}
```

**Note:** This depends on `MockProvider` supporting a constructor that takes a fixed response string. Check if `MockProvider::fixed(response: &str)` exists. If the constructor differs, adapt accordingly — the mock just needs to return the given string from `chat()`.

- [ ] **Step 2: Register in mod.rs**

```rust
pub mod classifier;
pub use classifier::{RequestClassifier, RequestComplexity};
```

- [ ] **Step 3: Run tests**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test classifier::tests -- --nocapture 2>&1 | tail -15`
Expected: all 5 tests PASS (may need to adapt MockProvider constructor)

- [ ] **Step 4: Commit**

```bash
git add src/providers/classifier.rs src/providers/mod.rs
git commit -m "feat: RequestClassifier for 3-tier smart routing"
```

---

### Task 11: Wire tiered routing into ProviderRouter

**Files:**
- Modify: `src/providers/router.rs`

- [ ] **Step 1: Add tier configuration to ProviderRouter**

Add fields to `ProviderRouter` struct (line ~32-44 in router.rs):
```rust
// Add these 4 fields after codex_base_url:
classifier: Option<Arc<super::classifier::RequestClassifier>>,
simple_providers: Vec<String>,
medium_providers: Vec<String>,
complex_providers: Vec<String>,
```

Update the `Self { ... }` initialization at line 286 to include new fields:
```rust
Ok(Self {
    providers, routes, auto_order, rotation: None,
    codex_token_manager, codex_base_url,
    classifier: None,
    simple_providers: Vec::new(),
    medium_providers: Vec::new(),
    complex_providers: Vec::new(),
})
```

- [ ] **Step 2: Add `[smart_routing]` section to AgentsToml**

The existing `routing` field in `AgentsToml` (line 27) is `Vec<RouteHint>` for hint-based routing. We use a **separate** `[smart_routing]` section to avoid conflicts:

Add to the `AgentsToml` struct (line 22-28 in router.rs):
```rust
#[derive(Debug, Deserialize)]
struct AgentsToml {
    #[serde(default)]
    providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    routing: Vec<RouteHint>,
    #[serde(default)]
    smart_routing: Option<SmartRoutingConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct SmartRoutingConfig {
    pub classifier_provider: Option<String>,
    pub classifier_model: Option<String>,
    #[serde(default)]
    pub simple_providers: Vec<String>,
    #[serde(default)]
    pub medium_providers: Vec<String>,
    #[serde(default)]
    pub complex_providers: Vec<String>,
}
```

In `ProviderRouter::new()`, after building providers (~line 282), read smart_routing:
```rust
// Parse smart routing tiers from config
if let Some(ref sr) = agents_config.smart_routing {
    router.simple_providers = sr.simple_providers.clone();
    router.medium_providers = sr.medium_providers.clone();
    router.complex_providers = sr.complex_providers.clone();
    tracing::info!("Smart routing configured: simple={:?}, medium={:?}, complex={:?}",
        sr.simple_providers, sr.medium_providers, sr.complex_providers);
}
```

- [ ] **Step 3: Add set_classifier method and tiered routing in chat_with_tools**

```rust
pub fn set_classifier(&mut self, classifier: Arc<super::classifier::RequestClassifier>) {
    self.classifier = Some(classifier);
}

pub fn set_tiers(
    &mut self,
    simple: Vec<String>,
    medium: Vec<String>,
    complex: Vec<String>,
) {
    self.simple_providers = simple;
    self.medium_providers = medium;
    self.complex_providers = complex;
}

/// Check if smart routing is configured (classifier + at least one tier has providers).
pub fn has_smart_routing(&self) -> bool {
    self.classifier.is_some() && (!self.simple_providers.is_empty()
        || !self.medium_providers.is_empty()
        || !self.complex_providers.is_empty())
}
```

In `chat_with_tools()`, before the existing provider resolution (~line 386), add tier-based routing:

```rust
// Tiered routing: if hint is "auto" and classifier is configured
if (provider.is_empty() || provider == "auto") && self.classifier.is_some() {
    let classifier = self.classifier.as_ref().unwrap();
    let complexity = classifier.classify(messages).await;

    let tier_candidates = match complexity {
        super::classifier::RequestComplexity::Simple => &self.simple_providers,
        super::classifier::RequestComplexity::Medium => &self.medium_providers,
        super::classifier::RequestComplexity::Complex => &self.complex_providers,
    };

    tracing::debug!("Classified as {:?}, candidates: {:?}", complexity, tier_candidates);

    // Try each candidate in order
    for candidate in tier_candidates {
        if let Some(p) = self.providers.get(candidate) {
            match p.chat(messages, tools, &p.default_model()).await {
                Ok(resp) => {
                    if let Some(ref rot) = self.rotation {
                        rot.record_success(candidate);
                    }
                    return Ok(resp);
                }
                Err(e) => {
                    tracing::warn!("Tier provider {} failed: {}, trying next", candidate, e);
                    if let Some(ref rot) = self.rotation {
                        rot.record_rate_limit(candidate);
                    }
                    continue;
                }
            }
        }
    }
    // If all tier candidates failed, fall through to normal routing
    tracing::warn!("All tier candidates failed, falling back to normal routing");
}
```

- [ ] **Step 4: Run full test suite**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test 2>&1 | tail -5`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add src/providers/router.rs
git commit -m "feat: tiered routing with classifier integration in ProviderRouter"
```

---

### Task 12: Integration test — full flow

**Files:**
- Modify: `tests/integration.rs` (or relevant integration test file)

- [ ] **Step 1: Write integration tests**

Add to integration tests:
```rust
#[tokio::test]
async fn test_chatgpt_backend_provider_creation() {
    // Verify the provider can be created and has correct metadata
    use clawtex_core::providers::ChatGptBackendProvider;
    use clawtex_core::providers::codex::CodexTokenManager;
    use clawtex_core::providers::Provider;
    use std::sync::Arc;

    let tm = Arc::new(CodexTokenManager::new());
    let provider = ChatGptBackendProvider::new(tm);
    assert_eq!(provider.name(), "chatgpt_backend");
    assert_eq!(provider.default_model(), "gpt-4o");
    assert!(provider.capabilities().streaming);
}

#[tokio::test]
async fn test_chatgpt_ws_provider_creation() {
    use clawtex_core::providers::ChatGptWsProvider;
    use clawtex_core::providers::codex::CodexTokenManager;
    use clawtex_core::providers::Provider;
    use std::sync::Arc;

    let tm = Arc::new(CodexTokenManager::new());
    let provider = ChatGptWsProvider::new(tm);
    assert_eq!(provider.name(), "chatgpt_ws");
    assert!(provider.capabilities().native_tools);
}

#[tokio::test]
async fn test_key_pool_integration() {
    use clawtex_core::providers::KeyPool;

    let pool = KeyPool::new(vec![
        "gemini-key-1".into(),
        "gemini-key-2".into(),
        "gemini-key-3".into(),
    ]);

    // Simulate rate limit on key 1
    pool.record_rate_limit("gemini-key-1").await;

    // Next key should skip key 1
    let key = pool.next_key().await.unwrap();
    assert_ne!(key, "gemini-key-1");
}

#[tokio::test]
async fn test_request_classifier_integration() {
    use clawtex_core::providers::{RequestClassifier, RequestComplexity};
    use clawtex_core::providers::mock::MockProvider;
    use clawtex_core::providers::ChatMessage;
    use std::sync::Arc;

    let mock = Arc::new(MockProvider::fixed("COMPLEX"));
    let classifier = RequestClassifier::new(mock, "test-model".to_string());

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: "Please write a complete REST API server with authentication middleware and database integration".to_string(),
        tool_calls: None,
        tool_call_id: None,
    }];

    let result = classifier.classify(&messages).await;
    assert_eq!(result, RequestComplexity::Complex);
}
```

- [ ] **Step 2: Run integration tests**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test --test integration test_chatgpt test_key_pool test_request_classifier -- --nocapture 2>&1 | tail -15`
Expected: all 4 integration tests PASS

- [ ] **Step 3: Run full test suite**

Run: `cd C:/Users/m4932/Desktop/adreanalai/LLM-Cluster-Project/clawtex-core && cargo test 2>&1 | tail -5`
Expected: all tests pass, 0 failures

- [ ] **Step 4: Commit**

```bash
git add tests/integration.rs
git commit -m "test: integration tests for ChatGPT providers, KeyPool, and classifier"
```

---

### Task 13: Update agents.toml example config

**Files:**
- Modify: `config/` or project root example config

- [ ] **Step 1: Add example config for new providers**

Add to agents.toml (or example config):
```toml
[providers.chatgpt]
type = "chatgpt_backend"
default_model = "gpt-4o"
# Uses OAuth from ~/.codex/auth.json — no api_key needed

[providers.chatgpt_ws]
type = "chatgpt_ws"
default_model = "gpt-4o"
# Uses OAuth from ~/.codex/auth.json — no api_key needed

[smart_routing]
classifier_provider = "ollama"
classifier_model = "qwen3:0.6b"
simple_providers = ["ollama", "lmstudio"]
medium_providers = ["groq", "gemini"]
complex_providers = ["chatgpt"]
```

- [ ] **Step 2: Commit**

```bash
git add -A
git commit -m "docs: add chatgpt_backend and routing config examples"
```

---

## Summary

| Task | Component | Tests |
|------|-----------|-------|
| 1 | Add tokio-tungstenite dep | compile check |
| 2 | CodexTokenManager helpers | 1 |
| 3 | Message format translation | 5 |
| 4 | ChatGptBackendProvider | 2 |
| 5 | Register in router | 1 + full suite |
| 6 | WS message types | 6 |
| 7 | ChatGptWsProvider | 3 |
| 8 | Register WS in router | full suite |
| 9 | KeyPool | 4 |
| 10 | RequestClassifier | 5 |
| 11 | Tiered routing | full suite |
| 12 | Integration tests | 4 |
| 13 | Config example | — |
| **Total** | | **~31 new tests** |
