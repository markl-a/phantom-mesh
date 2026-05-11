# Changelog

All notable changes to phantom-mesh are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0-alpha] - 2026-05-01

### Added — 2026-04-27 sprint (REPL UX, web frontend, TUI, self-evolve)

**REPL UX**
- Status line shows agent / cost / session / `· PLAN` mode
- Tab completion for `/cmd` and `@path/to/...`
- Ctrl-C cancels in-flight LLM stream (REPL stays alive)
- Markdown rendering in stream: bullets, numbered lists, blockquotes, links, code spans, fenced blocks
- Slash commands: `/show <n>` (expand captured tool output), `/perm ask|allow|deny|list|reset`, `/density compact|full`, `/theme <name>`, `/resume <prefix>`, `/plan` (real gating — denies tools until "go"), `/agent`, `/agents`, `/todo`
- Multi-line input via trailing `\`
- `@image.png` attaches PNG/JPG as multimodal `image_url` (OpenAI / Gemini / Anthropic auto-shaped); one-shot mode also expands `@file`

**Web frontend**
- xterm.js terminal panel with ANSI streaming
- **Cmd+K** (Ctrl+K) command palette (Terminal / Tasks / Sessions / Cost / Settings / Help / Reload)
- Info tab sub-panels: Todo, Sessions, Cost, **Tools** (captured tool-call history)
- Live peer-ping dots in the sidebar (green / red / grey)
- `@image` multimodal attach available in browser terminal too

**Tools** — total now 45 (+5)
- `web_fetch` — HTML → text
- `bash_run_background`, `bash_output`, `bash_kill` — long-running shell handles
- `ask_user` — pause agent, prompt human via stdin

**TUI** — `phantom tui` opens a full-screen ratatui interface (persistent input box, scrollable transcript, status bar, slash commands)

**Self-iteration** — `phantom evolve` validated end-to-end on this repo: autonomously fixed a `core/src/cost.rs` warning at $0 cost on Groq free tier (see `docs/SELF-EVOLVE.md`)

### Fixed — 2026-04-27
- B1: agent model fallback now respects per-provider default when route omits model
- B2: opencode model name normalization
- `max_tokens` default raised 256 → 4096 (unblocks reasoning models)
- `AGENTS.md` auto-loads from working directory alongside `PHANTOM.md`

### Added — agent runtime
- Multi-LLM provider fallback (Anthropic, OpenCode, OpenAI-compat, Gemini, Groq, Ollama)
- 45 MCP-protocol tools surfaced via `phantom mcp` (read, write, shell, grep, fetch, hardware scan, scaffold, mesh ops, etc.)
- 30-round agentic tool-calling loop with stall detection
- Token-aware context compaction (80K token budget default)
- Real-time cost tracking with per-model price table

### Added — interfaces (Day 2 + Day 3 sprint)
- **Claude Code-style REPL** (`phantom`):
  - rustyline editor with 12 slash commands (/help /clear /exit /add /cost /tools /sessions /session /list /init /model /compact)
  - **Streaming output** with inline tool calls (`● tool(args) … ✓ result`)
  - **Multi-line input** via trailing `\` continuation
  - `@<path>` file inlining
  - Polished welcome banner showing provider count, cluster peers, agent, session, dir
- **First-run terminal onboarding wizard** — auto-prompts when no agents.toml is found
- **Browser-based onboarding** (`phantom onboarding`) — auto-spawns serve, opens browser to settings page, waits for config write
- **Embedded web dashboard** (`phantom serve` → `http://localhost:7878`):
  - Single-page app: header status bar, sidebar with cluster nodes, tabbed main pane (Terminal | Tasks | Settings)
  - Streaming chat via Server-Sent Events
  - Settings page = web onboarding form with merge-on-save (preserves cluster peers + agent definitions)
- **MCP stdio server** (`phantom mcp`) — drop-in subagent for Claude Code, Cursor, any MCP client
- **Codex-compatible WebSocket JSON-RPC** (`phantom serve` → `ws://host:7878/ws`)

### Added — mesh
- P2P compute mesh via HTTP cluster with SHA-256 HMAC authentication
- Async job queue for cross-node task assignment (POST /rpc/task/assign → job_id polling)
- `phantom evolve --distributed` parallel agent swarm across cluster
- `phantom coordinator` zero-config peer discovery via mDNS
- Persistent conversation history (JSONL per chat_id)

### Added — integrations
- Telegram bot channel with user allowlist
- PHANTOM.md and AGENTS.md project context files (auto-loaded from working directory)
- Tauri desktop app (macOS, Linux, Windows) with React frontend
- Tauri Android app (foreground service worker)
- Tauri iOS app (sideload via free Apple developer cert)
- Cross-platform binaries (Mac arm64, Win x86_64, Linux arm64/x86_64, Android arm64/armv7/i686/x86_64, iOS arm64)

### Security
- Shell command blocklist (rm -rf /, fork bombs, curl|sh, etc.)
- Path traversal prevention (safe_path() canonicalization)
- Constant-time HMAC comparison (subtle crate)
- Cluster auth required (no default secret fallback)
- Telegram allowed_users enforcement

### Infrastructure
- GitHub Actions CI for all platforms (`ci-fast` / `ci-medium` / `ci-desktop` / `ci-mobile`)
- Release workflows for daemon, desktop, mobile (`release-daemon` / `release-desktop` / `release-mobile`)
- Cross-compilation via `cross` (Cross.toml: Android aarch64/armv7, Windows x86_64, Linux aarch64/armv7/x86_64)
- Tailscale mesh setup script
- Integration + smoke test suite (`scripts/integration-test.sh`, `scripts/smoke-test.sh`)
- Pre-open-source checklist (`scripts/pre-open-source-checklist.sh`)
- gitleaks pre-commit hook for secret scanning
