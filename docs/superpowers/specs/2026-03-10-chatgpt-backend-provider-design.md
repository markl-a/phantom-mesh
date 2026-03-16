# ChatGPT Backend Provider + Smart Routing Design

**Date**: 2026-03-10
**Status**: Approved

## Problem

- Local AI (Ollama/LM Studio) is too slow for complex tasks
- Don't want all traffic going to Codex (OpenAI API) — expensive + quota issues
- Gemini and Claude rate-limit/ban with heavy usage
- Existing `resolve_codex_credential()` gets a valid OAuth token but hits `insufficient_quota` on `api.openai.com/v1/` because that requires separate API billing

## Solution

Use the ChatGPT **product** backend (`chatgpt.com/backend-api`) instead of the developer API (`api.openai.com/v1`). This uses the ChatGPT Plus subscription quota ($20/month), not API billing. Combined with a 3-tier smart routing system that classifies requests by complexity and routes them to the appropriate provider tier.

## Architecture

```
Incoming message
  │
  ├─ Classifier (local small model, ~100ms)
  │
  ├─ SIMPLE  → Local (ollama / lmstudio)
  │             Free, slow but fine for simple tasks
  │
  ├─ MEDIUM  → Groq free tier + Gemini multi-key rotation
  │             Fast, free, rate-limit managed via key pool
  │
  └─ COMPLEX → ChatGPT Backend Provider
                ├── REST (Phase 1): chatgpt.com/backend-api/conversation
                └── WebSocket (Phase 2): wss://api.openai.com/v1/responses
```

## Phase 1: REST Backend Provider (`chatgpt_backend.rs`)

### Endpoint

```
POST https://chatgpt.com/backend-api/conversation

Headers:
  Authorization: Bearer <oauth_access_token>
  ChatGPT-Account-Id: <account_id>
  Content-Type: application/json
  Accept: text/event-stream
  User-Agent: Mozilla/5.0 ...
```

### Request Format

ChatGPT backend uses a different message format than the OpenAI API:

```json
{
  "action": "next",
  "model": "gpt-4o",
  "conversation_id": null,
  "parent_message_id": "<uuid>",
  "messages": [
    {
      "id": "<uuid>",
      "author": { "role": "user" },
      "content": {
        "content_type": "text",
        "parts": ["Hello, help me with..."]
      }
    }
  ]
}
```

Key differences from OpenAI API:
- Messages use UUIDs as IDs
- Content is wrapped in `{ content_type, parts }` structure
- `author.role` instead of top-level `role`
- `parent_message_id` for threading (UUID, generate fresh for new conversations)
- `action: "next"` for new messages, `"continue"` to extend

### Response Format (SSE)

```
data: {"message": {"id": "uuid", "author": {"role": "assistant"}, "content": {"content_type": "text", "parts": ["Hello!"]}, "status": "in_progress"}, "conversation_id": "uuid", ...}
data: {"message": {..., "status": "finished_successfully"}, ...}
data: [DONE]
```

### Message Translation

```rust
fn chatmessage_to_backend(msg: &ChatMessage) -> Value {
    // ChatMessage { role, content, tool_calls, tool_call_id }
    //   → { id: uuid, author: { role }, content: { content_type: "text", parts: [content] } }
}

fn backend_response_to_chatresponse(sse_events: Vec<Value>) -> ChatResponse {
    // Extract final message.content.parts[0] as content
    // Extract token usage if available
}
```

### Token Management

Reuse existing `CodexTokenManager` from `codex.rs`:
- Reads `~/.codex/auth.json` (access_token, refresh_token, account_id)
- Auto-refreshes expired tokens via `https://auth.openai.com/oauth/token`
- Client ID: `app_EMoamEEZ73f0CkXaXp7hrann` (from OpenClaw reference)
- 5-minute buffer before expiry triggers refresh

### Provider Implementation

```rust
pub struct ChatGptBackendProvider {
    token_manager: Arc<CodexTokenManager>,
    client: Client,
    conversation_cache: RwLock<HashMap<String, String>>,  // agent → conversation_id
}

#[async_trait]
impl Provider for ChatGptBackendProvider {
    fn name(&self) -> &str { "chatgpt_backend" }
    fn default_model(&self) -> &str { "gpt-4o" }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            native_tools: false,  // Phase 1: tool use handled by agent runtime
            vision: true,
        }
    }

    async fn chat(&self, messages, tools, model) -> Result<ChatResponse> {
        let token = self.token_manager.get_valid_token().await?;
        let account_id = self.token_manager.account_id();
        let backend_messages = messages.iter().map(chatmessage_to_backend).collect();
        let parent_id = Uuid::new_v4().to_string();

        let body = json!({
            "action": "next",
            "model": model,
            "parent_message_id": parent_id,
            "messages": backend_messages,
        });

        let resp = self.client.post("https://chatgpt.com/backend-api/conversation")
            .bearer_auth(&token)
            .header("ChatGPT-Account-Id", &account_id)
            .json(&body)
            .send().await?;

        // Parse SSE stream, collect final message
        parse_backend_sse(resp).await
    }

    async fn stream_chat(&self, messages, tools, model) -> Result<StreamOutput> {
        // Same as chat() but emit StreamChunk via channel as SSE events arrive
    }
}
```

### Available Models (ChatGPT Plus)

| Model | Best For |
|-------|----------|
| `gpt-4o` | General complex tasks, coding, analysis |
| `gpt-4o-mini` | Medium complexity, faster |
| `o1-mini` | Reasoning-heavy tasks |
| `o3-mini` | Advanced reasoning |

### Error Handling

| Error | Action |
|-------|--------|
| 401 Unauthorized | Refresh token, retry once |
| 403 Forbidden | Account issue, fall back to other provider |
| 429 Rate Limited | Record in rotation, cooldown, try other tier |
| Network error | Retry with backoff, max 3 attempts |

## Phase 2: WebSocket Responses Provider (`chatgpt_ws.rs`)

### Connection

```
wss://api.openai.com/v1/responses

Headers:
  Authorization: Bearer <oauth_token>
  OpenAI-Beta: responses-websocket=v1
```

### Event Protocol

**Client → Server:**
```json
{
  "type": "response.create",
  "model": "gpt-4o",
  "input": [
    { "type": "message", "role": "user", "content": "..." }
  ],
  "tools": [
    { "type": "function", "name": "shell", "description": "...", "parameters": {...} }
  ],
  "previous_response_id": "resp_xxx"
}
```

**Server → Client events:**
- `response.created` — Response initialized
- `response.output_text.delta` — Text content streaming
- `response.function_call_arguments.delta` — Tool call argument streaming
- `response.completed` — Response finished with full object
- `response.failed` — Error occurred

### Key Feature: Incremental Context (`previous_response_id`)

Instead of resending the full conversation each turn, reference the previous response:

```json
{
  "type": "response.create",
  "previous_response_id": "resp_abc123",
  "input": [
    { "type": "function_call_output", "call_id": "call_xyz", "output": "{\"result\": \"...\"}" }
  ]
}
```

This saves significant tokens on multi-turn tool-use conversations.

### Connection Management

```rust
pub struct ChatGptWsProvider {
    token_manager: Arc<CodexTokenManager>,
    ws_manager: RwLock<Option<WebSocketConnection>>,
    previous_response_id: RwLock<Option<String>>,
    reconnect_delays: [u64; 5],  // [1000, 2000, 4000, 8000, 16000] ms
}
```

- Auto-reconnect with exponential backoff (max 5 retries)
- Track `previous_response_id` for incremental context
- Warm-up: send `generate: false` to pre-load model
- Fallback: if WebSocket fails, fall back to REST provider

### Native Tool Use

WebSocket Responses API supports native tool calling:
- Tool definitions sent in `response.create` event
- Tool calls received as `function_call` output items
- Tool results sent back as `function_call_output` input items
- No need for agent runtime to handle tool prompt injection

```rust
fn capabilities(&self) -> ProviderCapabilities {
    ProviderCapabilities {
        streaming: true,
        native_tools: true,   // WebSocket supports native tools
        vision: true,
    }
}
```

## Phase 3: Request Classifier (`classifier.rs`)

### Design

Use a local small model (qwen3-0.6b or phi-4-mini) for fast 1-shot classification:

```rust
pub enum RequestComplexity {
    Simple,   // Classification, extraction, short replies, greetings
    Medium,   // General conversation, summarization, translation
    Complex,  // Code generation, long reasoning, multi-step analysis, tool-heavy
}

pub struct RequestClassifier {
    provider: Arc<dyn Provider>,  // Local small model
    cache: RwLock<LruCache<u64, RequestComplexity>>,  // Hash-based cache
}

impl RequestClassifier {
    pub async fn classify(&self, messages: &[ChatMessage]) -> RequestComplexity {
        // 1. Check cache (hash of last message content)
        // 2. If miss, ask local model with classification prompt
        // 3. Parse response, cache result, return
        // 4. Fallback to Medium on parse failure
    }
}
```

### Classification Prompt

```
Classify this request as SIMPLE, MEDIUM, or COMPLEX.

SIMPLE: greetings, yes/no questions, single-fact lookups, short translations
MEDIUM: summarization, general Q&A, multi-sentence replies, explanations
COMPLEX: code generation, debugging, multi-step reasoning, analysis, planning

Request: {last_user_message}

Reply with one word: SIMPLE, MEDIUM, or COMPLEX
```

### Router Integration

```rust
// In ProviderRouter::chat_with_tools():
if provider_hint == "auto" || provider_hint.is_empty() {
    let complexity = self.classifier.classify(messages).await;
    let tier_providers = match complexity {
        Simple => &self.local_providers,
        Medium => &self.medium_providers,
        Complex => &self.complex_providers,
    };
    // Select from tier using rotation
}
```

## Phase 4: Multi-Key Pool (`key_pool.rs`)

### Design

```rust
pub struct KeyPool {
    keys: Vec<ApiKeyEntry>,
    current: AtomicUsize,  // Round-robin index
}

struct ApiKeyEntry {
    key: String,
    cooldown_until: RwLock<Option<Instant>>,
    request_count: AtomicU64,
    rate_limit_count: AtomicU64,
}

impl KeyPool {
    pub fn next_available(&self) -> Option<&str> {
        // Round-robin through keys, skip those in cooldown
    }

    pub fn record_rate_limit(&self, key_index: usize) {
        // Set cooldown for this specific key
    }
}
```

### Configuration (`agents.toml`)

```toml
[providers.chatgpt]
type = "chatgpt_backend"
default_model = "gpt-4o"
# No api_key needed — uses OAuth from ~/.codex/auth.json

[smart_routing]
classifier_provider = "ollama"
classifier_model = "qwen3:0.6b"
simple_providers = ["ollama", "lmstudio"]
medium_providers = ["groq", "gemini"]
complex_providers = ["chatgpt"]
```

## Implementation Order

1. **`chatgpt_backend.rs`** — REST provider, message format translation, SSE parsing
2. **Router integration** — Register `"chatgpt_backend"` type in `create_provider()`
3. **Tests** — Unit tests for message translation + mock SSE parsing
4. **`chatgpt_ws.rs`** — WebSocket provider with auto-reconnect
5. **`classifier.rs`** — Request complexity classifier
6. **`key_pool.rs`** — Multi-key rotation for Gemini/Groq
7. **Router tiered routing** — Wire classifier + tiers into router

## Files Modified

| File | Change |
|------|--------|
| `src/providers/mod.rs` | Add `pub mod chatgpt_backend`, `chatgpt_ws`, `classifier`, `key_pool` |
| `src/providers/router.rs` | Add `"chatgpt_backend"` / `"chatgpt_ws"` to `create_provider()`, add tiered routing logic |
| `src/providers/codex.rs` | Make `CodexTokenManager` methods public for reuse (if not already) |
| `src/providers/rotation.rs` | No changes needed (already supports any provider name) |

## Testing Strategy

- Unit tests: message format translation, SSE parsing, classifier prompt parsing, key pool rotation
- Integration tests: mock HTTP server returning SSE, verify full chat() flow
- E2E: actual ChatGPT backend call with real token (manual, not CI)

## Security

- OAuth tokens stored in `~/.codex/auth.json` (existing, no new storage)
- Tokens never logged (existing credential scrubbing)
- `ChatGPT-Account-Id` treated as sensitive
- File-locked token refresh prevents concurrent corruption

## Risks

| Risk | Mitigation |
|------|------------|
| ChatGPT backend API format changes | Version-specific parsing, graceful fallback errors |
| Rate limiting on Plus plan | Rotation + cooldown, fall back to medium tier |
| Token refresh race conditions | Reuse existing file-lock mechanism in CodexTokenManager |
| WebSocket disconnects | Auto-reconnect with backoff, REST fallback |
| Classifier accuracy | Conservative: default to MEDIUM on uncertainty |
