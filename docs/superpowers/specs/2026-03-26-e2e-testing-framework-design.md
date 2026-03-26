# E2E Testing Framework Design

## Goal

Add a layered end-to-end testing framework to phantom-mesh that exercises full flows — agent loop, HTTP API, Telegram commands, cron scheduling — using MockProvider for deterministic, offline, fast execution.

## Context

phantom-mesh has 3895 unit tests but lacks true end-to-end tests that cross subsystem boundaries. Existing `tests/e2e_wiring.rs` tests individual subsystems in isolation. The 6 Python scripts in `tests/integration/` require a running server and real credentials. There is no shared test helper module — helpers are duplicated across 12 test files.

## Architecture

Three-layer harness system with shared helpers:

```
Layer           What it tests                    LLM           HTTP              DB
─────────────   ──────────────────────────────   ───────────   ───────────────   ──────────────
CoreHarness     Agent loop + tools + providers   MockProvider  None              In-memory SQLite
ApiHarness      HTTP endpoints + responses       MockProvider  Real server (:0)  Temp-dir SQLite
SystemHarness   Telegram + cron + triggers       MockProvider  Real server (:0)  Temp-dir SQLite
```

All layers use MockProvider (Echo/Scripted/Fixed/Error modes). No real LLM calls. All databases isolated per test via temp directories.

## Prerequisites: Production Code Seams

The following modifications to production code are required to make the harnesses constructible. Each is a small, focused change that adds a test seam without altering runtime behavior.

### P1. ProviderRouter: add `register_provider()` method

**File:** `src/providers/router.rs`

The `providers: HashMap<String, Box<dyn Provider>>` field is private and providers can only be added during `new()` from a config file. Add:

```rust
/// Register a provider programmatically (used by test harnesses and plugin system)
pub fn register_provider(&mut self, name: &str, provider: Box<dyn Provider>) {
    self.providers.insert(name.to_string(), provider);
}
```

### P2. LlmRouter: add test constructor

**File:** `src/llm_router.rs`

`LlmRouter::new(config_path)` requires a valid TOML file. Add a constructor that accepts a pre-built ProviderRouter:

```rust
/// Create LlmRouter from a pre-built ProviderRouter (for testing and programmatic setup)
pub fn from_router(router: ProviderRouter) -> Self {
    Self {
        inner: router,
        circuit_breaker: None,
        trajectory_logger: None,
    }
}

/// Mutable access to inner router (for provider registration)
pub fn inner_mut(&mut self) -> &mut ProviderRouter {
    &mut self.inner
}
```

### P3. ProviderRouter: add empty constructor

**File:** `src/providers/router.rs`

```rust
/// Create an empty router (no providers, no config file)
pub fn empty() -> Self {
    Self {
        providers: HashMap::new(),
        routes: HashMap::new(),
        auto_order: Vec::new(),
        rotation: None,
        circuit_breaker: None,
        codex_token_manager: None,
        codex_base_url: None,
        classifier: None,
        simple_providers: Vec::new(),
        medium_providers: Vec::new(),
        complex_providers: Vec::new(),
        budget_ratio: std::sync::atomic::AtomicU32::new(50),
    }
}
```

### P4. Scheduler: add `tick_now()` method

**File:** `src/cron.rs`

The existing `run()` / `run_with_triggers()` are infinite async loops. Add a single-tick method:

```rust
/// Evaluate all jobs once against current time. Returns names of triggered jobs.
/// Used by test harnesses to drive cron without sleeping.
pub async fn tick_now(&self, executor: &JobExecutor) -> Vec<String> {
    let now = chrono::Utc::now();
    let mut triggered = Vec::new();
    let jobs = self.jobs.read().await;
    for job in jobs.iter() {
        if job.is_paused { continue; }
        if let Some(next) = job.next_run {
            if now >= next {
                executor.execute(&job.action).await.ok();
                triggered.push(job.name.clone());
            }
        }
    }
    triggered
}
```

### P5. AppState: extract to lib.rs and make public

**File:** `src/main.rs` → `src/app_state.rs` (new file)

Move `AppState` struct definition and `build_router()` function to a separate module so integration tests can construct them. The struct has ~59 fields, most of which are `Option<Arc<T>>`. Add a test-friendly constructor:

```rust
/// Create AppState with minimal required fields, all optional fields set to None.
/// Used by test harnesses.
pub fn test_default(
    llm_router: Arc<LlmRouter>,
    agent_runtime: Arc<AgentRuntime>,
    tool_registry: Arc<ToolRegistry>,
    temp_dir: &Path,
) -> Result<Self> { ... }
```

Extract the Axum router builder into a standalone function:
```rust
pub fn build_router(state: AppState) -> axum::Router { ... }
```

### P6. MockTelegram: implement Channel trait

**File:** `src/channel.rs` (or new `src/channels/mock.rs`)

The `Channel` trait requires `send()`, `send_reply()`, `edit_message()`, `listen()`. MockTelegram implements all of them:

- `send()` / `send_reply()` / `edit_message()` → push text into `replies: Arc<Mutex<Vec<String>>>`
- `listen()` → spawn a task that forwards from internal mpsc to the provided tx

This captures **outbound** replies (bot → user) while the mpsc sender handles **inbound** messages (user → bot).

## File Structure

```
src/
  app_state.rs            — AppState struct + build_router() (extracted from main.rs)
  channels/mock.rs        — MockTelegram implementing Channel trait (cfg(test))

tests/
  common/
    mod.rs                — re-exports all submodules
    harness.rs            — CoreHarness, ApiHarness, SystemHarness + builders
    fixtures.rs           — Reusable test data (agent configs, messages, profiles)
    assertions.rs         — Domain-specific assert macros
  e2e_core.rs             — 7 tests: agent loop flows
  e2e_butler.rs           — 11 tests: Butler Platform feature flows
  e2e_api.rs              — 10 tests: HTTP API endpoint flows
  e2e_system.rs           — 7 tests: Telegram + cron system flows
```

Note: Rust integration tests import shared helpers via `mod common;` in each test file.

## Component Design

### CoreHarness

In-process harness. No HTTP server. Tests agent runtime + tool execution + provider interaction.

```rust
pub struct CoreHarness {
    pub agent_runtime: Arc<AgentRuntime>,
    pub tool_registry: Arc<ToolRegistry>,
    pub provider: Arc<MockProvider>,
    pub llm_router: Arc<LlmRouter>,
    pub user_profile: Arc<RwLock<UserProfile>>,
    _temp_dir: TempDir,  // auto-cleanup on Drop
}
```

**Construction call path:**
1. Create `TempDir`
2. Create `MockProvider` (caller-specified mode)
3. Create `ProviderRouter::empty()`, call `register_provider("mock", Box::new(mock.clone()))`
4. Create `LlmRouter::from_router(provider_router)`
5. Create `SecurityConfig` with temp workspace
6. Create `ToolRegistry::new(security)` — this internally creates `FileSnapshots` and `ShellSessionManager`
7. Create `AgentRuntime::new(test_config_path)` with a minimal test agents.toml that routes to "mock" provider

Builder API:
```rust
CoreHarness::builder()
    .provider(MockProvider::scripted(vec![...]))
    .tools(&["shell", "file_read", "file_edit"])  // default: all tools
    .with_user_profile(profile)
    .build().await
```

Key methods:
- `run_agent(prompt) → AgentResult` — calls `agent_runtime.run("master", prompt, &[], &llm_router, &tool_registry, extra_context)`
- `run_agent_with_history(prompt, history) → AgentResult` — with conversation context
- `run_tool(name, args) → ToolResult` — calls `tool_registry.execute_tool(name, args)`
- `provider_call_count() → usize` — delegates to `MockProvider::call_count()`
- `provider_call(index) → MockCallRecord` — delegates to `MockProvider::get_call(index)`
- `workspace_path() → PathBuf` — temp workspace for file tools

### ApiHarness

Extends CoreHarness with a real Axum HTTP server on `127.0.0.1:0` (OS-assigned port).

```rust
pub struct ApiHarness {
    pub core: CoreHarness,
    pub client: reqwest::Client,
    pub base_url: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    _server_handle: JoinHandle<()>,
}
```

**Construction call path:**
1. Build `CoreHarness`
2. Create `AppState::test_default(llm_router, agent_runtime, tool_registry, temp_dir)`
3. Call `build_router(app_state)` to get the Axum `Router`
4. Bind `TcpListener` to `127.0.0.1:0`, extract assigned port
5. Spawn `axum::serve(listener, router)` with graceful shutdown via oneshot channel
6. Create `reqwest::Client` with base URL `http://127.0.0.1:{port}`

Builder API:
```rust
ApiHarness::builder()
    .provider(MockProvider::echo())
    .with_auth_token("test-token")
    .build().await
```

Key methods:
- `get(path) → Response` — GET with auth header
- `post(path, body) → Response` — POST with auth header
- `url(path) → String` — builds full URL

Drop implementation sends shutdown signal and awaits server handle.

### SystemHarness

Extends ApiHarness with mock Telegram and cron fast-forward.

```rust
pub struct SystemHarness {
    pub api: ApiHarness,
    pub telegram: MockTelegram,
    pub scheduler: Arc<Scheduler>,
    pub trigger_manager: Arc<Mutex<EventTriggerManager>>,
}
```

**Construction call path:**
1. Build `ApiHarness`
2. Create `MockTelegram::new()` — implements `Channel` trait
3. Create `CronStore::new(temp_dir.join("cron.db"))`
4. Create `Scheduler::new(cron_store)`
5. Create `EventTriggerManager` with temp SQLite

### MockTelegram

Implements `Channel` trait for both inbound and outbound message capture.

```rust
pub struct MockTelegram {
    inbound_tx: mpsc::Sender<ChannelMessage>,   // test → handler (inject messages)
    replies: Arc<Mutex<Vec<(String, String)>>>,  // (chat_id, text) captured from send()
}

#[async_trait]
impl Channel for MockTelegram {
    fn name(&self) -> &str { "mock_telegram" }
    fn channel_type(&self) -> ChannelType { ChannelType::Telegram }

    async fn send(&self, chat_id: &str, text: &str) -> Result<()> {
        self.replies.lock().unwrap().push((chat_id.into(), text.into()));
        Ok(())
    }

    async fn send_reply(&self, chat_id: &str, text: &str, _reply_to: &str) -> Result<()> {
        self.send(chat_id, text).await
    }

    async fn edit_message(&self, _chat_id: &str, _msg_id: &str, _text: &str) -> Result<()> {
        Ok(())  // no-op for tests
    }

    async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        // Forward from inbound_tx to the handler's rx
        ...
    }
}
```

Test methods:
- `send_message(chat_id, text)` — injects a user message via `inbound_tx`
- `send_command(chat_id, command)` — injects a `/command`
- `drain_replies() → Vec<(String, String)>` — returns and clears captured replies
- `wait_for_reply(timeout) → Option<String>` — async wait with timeout

### Database Isolation

Each harness creates isolated SQLite databases in a temp directory:
```
{TempDir}/
  core.db, costs.db, revenue.db, trajectories.db,
  pricing.db, goals.db, observations.db, triggers.db, cron.db
```

`TempDir` auto-cleans on Drop. No cross-test contamination. For cron trigger tests, fixtures seed the database with known trigger records before test execution.

### Assertion Macros

```rust
// Assert agent used a specific tool during execution
assert_agent_used_tool!(result, "file_read");

// Assert HTTP response is 200 OK with JSON body
assert_http_ok!(response);

// Assert mock Telegram received a reply containing text
assert_telegram_replied!(mock, "Profile updated");

// Assert agent result contains expected text
assert_agent_output_contains!(result, "successfully");
```

## Test Coverage

### A. Core Agent Loop (`tests/e2e_core.rs`) — 7 tests

| Test | Flow | MockProvider |
|------|------|-------------|
| `agent_single_turn_text` | prompt → agent → text response | Fixed |
| `agent_tool_call_roundtrip` | prompt → tool call → tool exec → response | Scripted(ToolCall→Text) |
| `agent_multi_tool_chain` | prompt → tool1 → tool2 → tool3 → response | Scripted(3×ToolCall→Text) |
| `agent_idle_detection` | same output repeated → auto-exit | Scripted(repeat) |
| `agent_max_rounds_exit` | 10 rounds → forced stop | Scripted(infinite ToolCalls) |
| `agent_context_injection` | UserProfile in system prompt | Echo(inspect call record) |
| `agent_error_recovery` | provider error → graceful failure | Error |

### B. Butler Features (`tests/e2e_butler.rs`) — 11 tests

| Test | Feature | Validates |
|------|---------|-----------|
| `profile_injects_timezone` | UserProfile | system prompt contains timezone + locale |
| `profile_persona_routing` | UserProfile | persona config affects behavior |
| `cache_hints_applied` | Prompt Caching | messages contain cache_control blocks |
| `toctou_read_then_edit` | File Validation | read → edit succeeds |
| `toctou_external_modify_blocked` | File Validation | read → external write → edit fails |
| `toctou_edit_updates_snapshot` | File Validation | edit → second edit succeeds |
| `shell_session_persists_cwd` | Shell Sessions | cd → pwd persists across commands |
| `shell_session_persists_env` | Shell Sessions | export → echo persists across commands |
| `shell_markers_hidden` | Shell Sessions | PHANTOM_MESH markers not in user output |
| `trigger_fires_on_condition` | Event Triggers | condition met → trigger fires |
| `trigger_enable_disable` | Event Triggers | disabled trigger skips evaluation |

### C. HTTP API (`tests/e2e_api.rs`) — 10 tests

| Test | Endpoint | Validates |
|------|----------|-----------|
| `health_check` | GET /health | 200 + JSON with version |
| `agent_run_http` | POST /agent/:name/run | Full agent execution via HTTP |
| `tools_list` | GET /tools | Returns 50+ tools |
| `hands_list` | GET /hands | Returns hand definitions |
| `hand_run` | POST /hand/:name/run | Workflow execution via HTTP |
| `cost_tracking` | GET /costs | Cost data present after agent run |
| `auth_required` | POST /agent/run (no token) | 401 when hub_api_key configured |
| `cluster_register_heartbeat` | POST /cluster/register → /heartbeat | Worker lifecycle |
| `goals_crud` | POST/GET/PUT/DELETE /goals | Full CRUD cycle |
| `memory_observe_search` | POST /memory/observe → GET /memory/observations | Memory roundtrip |

### D. System Tests (`tests/e2e_system.rs`) — 7 tests

| Test | Flow | Validates |
|------|------|-----------|
| `telegram_chat_flow` | mock msg → handler → agent → reply | Full message handling |
| `telegram_clear_command` | /clear → conversation reset | Command handling |
| `telegram_profile_command` | /profile → profile display | Profile integration |
| `telegram_alerts_command` | /alerts → trigger list | Trigger management |
| `telegram_lang_switch` | /lang zh-TW → i18n change | Localization |
| `cron_tick_fires_job` | scheduler tick → hand execution | Scheduled workflow |
| `cron_trigger_evaluation` | tick → trigger eval → action | Event trigger pipeline |

## Execution

```bash
# Individual suites
CARGO_TARGET_DIR=target2 cargo test --test e2e_core       # ~2s
CARGO_TARGET_DIR=target2 cargo test --test e2e_butler     # ~3s
CARGO_TARGET_DIR=target2 cargo test --test e2e_api        # ~5s
CARGO_TARGET_DIR=target2 cargo test --test e2e_system     # ~5s

# All e2e tests
CARGO_TARGET_DIR=target2 cargo test --test 'e2e_*'        # ~15s total (execution only)
```

All tests:
- Use `#[tokio::test]`
- Use MockProvider only (no real LLM calls)
- Run offline, deterministic, repeatable
- Auto-clean temp directories
- Require `CARGO_TARGET_DIR=target2` (exFAT workaround)
- Import shared helpers via `mod common;`

## Dependencies

No new crates expected. Already in deps:
- `reqwest` (HTTP client for API tests)
- `tempfile` (temp directories)
- `tokio` (async runtime)

## Success Criteria

1. 35 e2e tests pass with `cargo test --test 'e2e_*'`
2. Total execution runtime under 30 seconds
3. No real LLM API calls, no network dependencies
4. Each test is self-contained — can run in any order, in parallel
5. Shared test helpers in `tests/common/` eliminate duplication
6. All 6 prerequisite production code seams (P1-P6) implemented and tested

## Relationship to Existing Tests

The new `tests/common/` module does NOT replace existing test helpers in `tests/e2e_wiring.rs`, `tests/integration.rs`, etc. Those files remain as-is. Existing helpers like `make_hand_result()`, `make_messages()`, `make_chat_response()` in those files are specific to their test contexts. The new shared module provides higher-level harnesses for cross-subsystem e2e tests.

---

## Implementation Status (2026-03-26)

### Completed

| Item | Status | Details |
|------|--------|---------|
| P1. ProviderRouter::empty() + register_provider() | ✅ | `src/providers/router.rs` |
| P2. LlmRouter::from_router() + inner_mut() | ✅ | `src/llm_router.rs` |
| P3. Scheduler::tick_now() | ✅ | `src/cron.rs` |
| P4. MockProvider Arc refactor + Clone | ✅ | `src/providers/mock.rs` |
| P5. MockChannel (Channel trait) | ✅ | `src/channel.rs` |
| P6. AppState extraction + test_default() | ✅ | `src/app_state.rs` (new) |
| P7. Telegram handler → dyn Channel | ✅ | `src/main.rs` |
| CoreHarness | ✅ | `tests/common/harness.rs` |
| tests/common/ infrastructure | ✅ | fixtures.rs, assertions.rs, harness.rs |
| e2e_core.rs | ✅ | 8 tests passing |
| e2e_butler.rs | ✅ | 11 tests passing |

### Deferred

| Item | Status | Reason |
|------|--------|--------|
| ApiHarness | ⏸ Deferred | `build_router()` requires 130+ handler functions that live in `src/main.rs` (binary crate), inaccessible from library code. Extracting handlers to library modules is a large refactor beyond e2e scope. |
| SystemHarness | ⏸ Deferred | Depends on ApiHarness. |
| e2e_api.rs (10 tests) | ⏸ Deferred | Requires ApiHarness. |
| e2e_system.rs (7 tests) | ⏸ Deferred | Requires SystemHarness. |

### Actual Results vs Success Criteria

| Criterion | Target | Actual |
|-----------|--------|--------|
| e2e tests passing | 35 | **19** (8 core + 11 butler) |
| Execution runtime | < 30s | **< 1s** |
| No real LLM calls | ✅ | ✅ |
| Self-contained tests | ✅ | ✅ |
| Shared test helpers | ✅ | ✅ |
| Production code seams | P1-P6 | **P1-P7** (7 seams implemented) |

### Unblocking Path for Deferred Items

To implement ApiHarness/SystemHarness, the HTTP handler functions need to be extracted from `src/main.rs` into library modules (e.g., `src/handlers/`). This is a separate refactoring task estimated at ~2000 LOC moved, affecting 130+ route handlers.
