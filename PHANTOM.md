# Phantom Mesh — Project Context

## What This Is
phantom-mesh is a cross-platform AI agent mesh (macOS/Windows/Linux/Android/iOS).
A Rust daemon (`:7878`) + Tauri desktop app + Web UI (`:5173`) + Telegram bot channel.
Multiple nodes connect over Tailscale to form a P2P compute mesh.
Config lives in `~/.phantom-mesh/agents.toml`.

## Architecture

```
app/         — Tauri + React desktop/web frontend (TypeScript, Vite)
core/        — Rust workspace
  src/
    lib.rs           — AppState, AgentRuntime, tool implementations, provider fallback
    main.rs          — Axum HTTP server (daemon entry point)
    mesh.rs          — ClusterManager, PeerStatus, SHA-256 HMAC auth
    channels/
      telegram.rs    — Long-poll Telegram bot channel
    providers/
      traits.rs      — ChatProvider trait, ChatMessage, ProviderEntry
      anthropic.rs   — Anthropic Claude
      openai.rs      — OpenAI-compatible (OpenRouter, Groq, XAI, Ollama)
      gemini.rs      — Google Gemini
      credential_scanner.rs — Scan env for API keys
    hardware.rs      — System hardware scan
    oauth.rs         — OAuth2 Google/Apple (partial)
    project_context.rs — PHANTOM.md / .phantom-mesh/context.md loader
  Cargo.toml
configs/     — Sample agent configs for coordinator/cloud/worker nodes
scripts/     — GCP deploy, smoke test, security history cleanup
docs/        — DEPLOYMENT.md, architecture docs
mobile/      — Mobile client (in development)
```

## Tools Available (13 total)
- `shell` — run any command (with blocklist for dangerous patterns)
- `file_read`, `file_write`, `file_edit` — filesystem ops with path canonicalization
- `content_search` — regex search in files (ripgrep-style)
- `glob_search` — find files by pattern
- `web_search` — Brave Search API (fallback: DuckDuckGo)
- `memory_store`, `memory_recall` — persist notes to `~/.phantom-mesh/memory.json`
- `git_status`, `git_diff`, `git_log`, `git_commit` — git operations

## Coding Conventions
- Conventional commits: `feat:` / `fix:` / `chore:` / `docs:`
- Run `cargo build` (not `cargo test`) after Rust edits — test suite is minimal
- `AppState` is `Clone` — all fields must be `Arc<…>` or `Clone`
- New tools go in `execute_tool()` match in `core/src/lib.rs`, plus the `tool_definitions()` function
- New HTTP routes go in `build_router()` in `core/src/main.rs`
- Frontend API calls go to `http://localhost:7878` (proxy configured in `app/vite.config.ts`)

## Agent Rules
- Always call `cargo build` after editing any `.rs` file to verify it compiles
- Use `file_edit` not `file_write` for modifying existing files
- When adding Cargo dependencies: edit `core/Cargo.toml` then rebuild
- Before committing: run `cargo build` and confirm exit 0
- For git operations: use `git_status` first, then `git_commit` with a specific message
- Do NOT use `git commit -am` — always stage specific files

## Key Config File
`~/.phantom-mesh/agents.toml` — loaded at daemon startup.
See `agents.toml.example` for full reference with all options.

## P2P Mesh
Nodes authenticate via `X-Cluster-Auth: sha256(cluster_secret + body)`.
Peers are listed in `[cluster]` section of agents.toml.
`POST /rpc/task/assign` returns a job_id immediately (async execution).
`GET /rpc/task/status/:job_id` polls job completion.

## Security Notes
- API keys: use `api_key_env = "VAR_NAME"` to read from env (preferred over inline keys)
- `cluster_secret` is required; nodes with no secret reject all RPC auth
- Telegram `allowed_users` should always be set in production
