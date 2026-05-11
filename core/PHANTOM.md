# Project: phantom-mesh (core)

## Overview

phantom-mesh is a cross-platform AI agent mesh written in Rust. A long-running
daemon (`core/`) exposes an HTTP API on `:7878`; agents run tool-augmented LLM
loops against one or more LLM providers (Anthropic, OpenAI-compat, Gemini) with
automatic fallback. Multiple nodes connect over Tailscale to form a P2P compute
mesh. A Tauri + React desktop app (`app/`) and a Telegram bot channel round out
the surface area.

Config lives at `~/.phantom-mesh/agents.toml` (see `agents.toml.example` for
the full reference).

## Build & Test

```bash
# Compile-check (fast; run this after every .rs edit)
cargo check --manifest-path core/Cargo.toml

# Full build
cargo build --manifest-path core/Cargo.toml

# Release build
cargo build --release --manifest-path core/Cargo.toml

# Run tests
cargo test --manifest-path core/Cargo.toml

# Run the REPL binary directly
cargo run --manifest-path core/Cargo.toml --bin phantom-mesh
```

Always run `cargo check` after editing any `.rs` file. The project has a small
test suite; `cargo test` is the gate before committing.

## Key Files

```
core/
  Cargo.toml                   — package manifest; add dependencies here
  src/
    main.rs                    — Axum HTTP server entry point; defines build_router()
    lib.rs                     — AppState, re-exports, JobStore; public API surface
    agent.rs                   — AgentRuntime, AgentEvent, the tool-call loop
    config.rs                  — AgentsConfig, AgentEntry, ProviderEntry, ToolsConfig
    context.rs                 — WorkspaceContext (cwd, git root, PHANTOM.md loader)
    cost.rs                    — CostTracker (token + USD accounting)
    session.rs                 — ConversationStore (JSONL persistence)
    streaming.rs               — SSE / streaming helpers
    scaffold.rs                — `phantom init` PHANTOM.md generator
    project_context.rs         — PHANTOM.md / .phantom-mesh/context.md loader
    mesh.rs                    — ClusterManager, PeerStatus, HMAC auth
    hardware.rs                — System hardware scan
    oauth.rs                   — OAuth2 Google/Apple (partial)
    bin/
      phantom.rs               — `phantom` CLI entry point (REPL + one-shot mode)
    channels/
      telegram.rs              — Long-poll Telegram bot channel
    providers/
      traits.rs                — ChatProvider trait, ChatMessage
      anthropic.rs             — Anthropic Claude
      openai.rs                — OpenAI-compatible (OpenRouter, Groq, XAI, Ollama)
      gemini.rs                — Google Gemini
      claude_cli.rs            — Claude CLI bridge provider
      credential_scanner.rs    — Scan env for API keys
    tools/
      mod.rs                   — Tool registry: execute() dispatch + schema() definitions
      shell.rs                 — shell — run arbitrary commands (with blocklist)
      file.rs                  — file_read, file_write, file_edit
      search.rs                — content_search (ripgrep), glob_search
      web.rs                   — web_search (Brave API + DuckDuckGo fallback)
      memory.rs                — memory_store, memory_recall (~/.phantom-mesh/memory.json)
      git.rs                   — git_status, git_diff, git_log, git_commit
      fetch.rs                 — HTTP fetch tool
      fs.rs                    — Extended filesystem helpers
      ls.rs                    — Directory listing
      diff_view.rs             — Diff rendering
      patch.rs                 — Patch apply
      multi_edit.rs            — Batch file edits
      task.rs                  — Subtask spawning helpers
      diagnostic.rs            — Self-diagnostic tool
      http_client.rs           — Shared HTTP client utilities
```

## Architecture Decisions

### Tool Layer

All tool dispatch flows through `core/src/tools/mod.rs`:

- `execute(name, args, config)` — async dispatch via a `match` on tool name
- `schema(name)` — returns the JSON Schema for each tool (sent to the LLM)

**When adding a new tool:**
1. Create `core/src/tools/<toolname>.rs` with an async handler function.
2. Declare it as `pub mod <toolname>;` in `tools/mod.rs`.
3. Add a match arm in the `execute()` function.
4. Add a `schema()` match arm returning a valid JSON Schema object.
5. Run `cargo check` to verify.

Failing to update both `execute()` and `schema()` will leave the tool silently
unreachable or invisible to the LLM.

### Agent Loop

`AgentRuntime` in `agent.rs` drives the tool-call loop:

- `MAX_ROUNDS = 20` — hard limit on tool-call iterations per request
- `STALL_THRESHOLD = 2` — consecutive rounds with no tool calls before the loop exits
- `TOKEN_BUDGET = 60_000` — context compaction threshold
- Provider fallback: primary → next in config order on HTTP error / 429 / 503
- `MAX_RETRIES = 3` per provider before falling back

### Provider Abstraction

`ChatProvider` trait in `providers/traits.rs`. Each provider implements it.
`AgentsConfig.providers` is a `HashMap<String, ProviderEntry>`; agents reference
providers by name. Provider type is determined by `ProviderEntry.provider_type`
(`"anthropic"`, `"openai_compat"`, `"gemini"`).

### AppState

`AppState` is `Clone` — every field must be `Arc<…>` or implement `Clone`.
This is required because Axum passes state by value to handlers.

### Session Persistence

`ConversationStore` writes conversations as JSONL files under
`~/.phantom-mesh/conversations/<session_id>.jsonl`. The store enforces a 500-job
eviction limit on `JobStore`.

### HTTP API

Routes are defined in `build_router()` in `core/src/main.rs`.
All routes are prefixed under `:7878`. Frontend proxy is configured in
`app/vite.config.ts` to forward to `http://localhost:7878`.

### P2P Mesh Auth

Cluster nodes authenticate via `X-Cluster-Auth: sha256(cluster_secret + body)`.
Peers are listed in `[cluster]` in `agents.toml`.
`POST /rpc/task/assign` is async — returns `job_id` immediately.
`GET /rpc/task/status/:job_id` polls completion.

## Coding Conventions

- **Commit style:** Conventional commits — `feat:`, `fix:`, `chore:`, `docs:`, `refactor:`
- **File edits:** use `file_edit` (exact string replacement), not `file_write`, when modifying existing files
- **Cargo deps:** edit `core/Cargo.toml`, then `cargo check` to validate
- **Staging:** never use `git commit -am`; stage specific files with `git add <path>`
- **No orphan tool calls:** do not end a conversation turn mid-tool-call
- **One concern per module:** keep providers, tools, and channels in their own files

## Known Gotchas

- `tools/mod.rs` must be updated in BOTH `execute()` and `schema()` when adding a tool — missing either causes silent failure or an invisible tool.
- `AppState` must stay `Clone`; wrapping new state in `Arc<TokioRwLock<_>>` is the standard pattern.
- `cargo test` is the gate; `cargo check` is fast but does not run tests.
- The `phantom init` subcommand in `bin/phantom.rs` has a TODO stub — `scaffold.rs` must be wired up before it works.
- API keys: always use `api_key_env = "VAR_NAME"` in config; never inline keys in `agents.toml`.
- `cluster_secret` is required for P2P RPC; nodes without it reject all inbound cluster auth.
- `allowed_users` in the Telegram config must be set in production deployments.
- Provider type `"anthropic"` automatically adds the `anthropic-version` header; `"openai_compat"` does not.
- Context compaction triggers at `TOKEN_BUDGET = 60_000` tokens; tool_call messages must not be orphaned during compaction (assistant tool_call and its tool result must appear as a pair).

## Testing Strategy

- Unit tests live in `core/tests/` (integration) and inline `#[cfg(test)]` modules.
- The primary gate before any commit is `cargo test --manifest-path core/Cargo.toml`.
- For tool changes, manually exercise the tool via the REPL (`cargo run --bin phantom-mesh`) before committing.
- For provider changes, use a one-shot prompt: `cargo run --bin phantom-mesh "hello"` with the target provider configured.
- For mesh/RPC changes, use the smoke-test scripts in `scripts/`.

## Security Notes

- Use `api_key_env` in config, never inline keys.
- `cluster_secret` is required; set it in `[cluster]` for every node.
- Set `allowed_users` in the Telegram config before exposing the bot publicly.
- The `shell` tool has an internal blocklist for dangerous command patterns — do not remove it.
- `hub_api_key` in `[core]` enables bearer-token auth on the HTTP API; recommended for any externally-exposed node.
