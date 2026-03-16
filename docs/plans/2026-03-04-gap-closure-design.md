# Gap Closure Design — Clawtex vs Reference Projects

Date: 2026-03-04

## Phase 1 (1-2 days) — Core Runtime Improvements

### 1.1 Parallel Tool Execution
- Replace sequential `for tc in tool_calls` loop in `agent_runtime.rs:278` with `futures::future::join_all()`
- All tool calls in a single LLM response execute concurrently
- File: `agent_runtime.rs` (~20 lines changed)
- Dependency: `futures` crate

### 1.2 Budget Enforcement (Hard Limit)
- Add `check_budget(agent, daily_limit_usd)` to `CostTracker`
- Call at top of each agent loop iteration, before LLM call
- Budget config: `daily_budget_usd` field in `agents.toml` `[settings]`
- Files: `cost_tracker.rs`, `agent_runtime.rs`

### 1.3 Auto Memory Injection
- At start of `run_with_config()`, call `memory.recall(prompt, 5)`
- Prepend relevant memories to system prompt as `[Relevant memories]` block
- Add `Option<Arc<MemoryStore>>` to `AgentRuntime`
- Files: `agent_runtime.rs`

### 1.4 Loop Detection
- New `LoopDetector` struct tracking recent tool call signatures
- If 3 consecutive rounds have identical tool+args, force exit with warning
- Lives in `agent_runtime.rs`
- Signature = hash of sorted (tool_name, args) tuples

## Phase 2 (2-3 days) — Dispatch + Security + Channels

### 2.1 XML Tool Dispatcher
- `ToolDispatcher` trait with `NativeToolDispatcher` and `XmlToolDispatcher`
- Auto-select based on provider capabilities
- New file: `src/dispatcher.rs`

### 2.2 Conversation Summarization
- Replace destructive `trim_messages()` with `summarize_and_trim()`
- Uses LLM to compress old turns before evicting
- File: `context.rs`

### 2.3 Autonomy Levels
- `AutonomyLevel` enum: `ReadOnly`, `Supervised`, `Full`
- Per-agent config in `agents.toml`
- Wired to tool execution gate
- Files: `security/mod.rs`, `agent_runtime.rs`

### 2.4 RBAC (Role-Based Access Control)
- Roles: `owner`, `admin`, `operator`, `viewer`
- Per-Telegram-user role assignment
- Tool access filtered by role
- Files: `security/roles.rs`, `telegram.rs`

## Phase 3 (1 week+) — Architecture Extensions

### 3.1 Channel Trait + Multi-Channel
### 3.2 WASM Tool Sandbox
### 3.3 Plugin System
### 3.4 A2A Protocol
### 3.5 Hybrid Memory Search (FTS + Vector RRF)
### 3.6 SOP Conditional Gates
### 3.7 Observability (OpenTelemetry/Prometheus)
### 3.8 Git Branch Per Task
### 3.9 Context Compaction (duplicate of 2.2, merged)
