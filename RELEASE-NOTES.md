# phantom-mesh v0.1.0-alpha

> An open-source AI agent runtime — Claude Code-style REPL + MCP/WS subagent +
> embedded web dashboard, designed to run as a mesh across all your devices.

## What is it?

`phantom` is a single Rust binary that gives you:

- A **conversational REPL** in your terminal (Claude Code / Codex style)
- An **MCP server** — drop-in subagent for Claude Code, Cursor, any MCP host
- A **WebSocket JSON-RPC daemon** — Codex-compatible client surface
- An **embedded web dashboard** at `http://localhost:7878` — node status,
  terminal, settings — accessible from any device on the network
- A **mesh** — multiple `phantom` nodes discover each other and delegate work

## What makes it different?

Unlike single-machine agent CLIs:

- **Runs across all your devices** — macOS, Linux (x86_64/arm64), Windows,
  Android (Tauri foreground service), iOS (Tauri sideload)
- **Multi-provider fallback** — Groq, Gemini, OpenCode, Anthropic, OpenAI-compat,
  Ollama; automatic failover on rate-limit / error
- **P2P compute mesh** — distribute tasks across nodes via HTTP+HMAC, async
  job queue, least-loaded routing
- **Local-first** — no telemetry; all data in `~/.phantom-mesh/`
- **Both subagent AND standalone** — embed in Claude Code via MCP, or run
  it as your own primary agent

## Quick start

```bash
# 1. Download a binary for your platform from the Releases page, then:
./phantom              # terminal walks you through provider setup
                       # → drops into REPL with welcome banner

# Or open the web onboarding:
./phantom onboarding   # spawns serve, opens browser to settings page

# Subagent for Claude Code: add to ~/.claude.json
#   "mcpServers": { "phantom": { "command": "phantom", "args": ["mcp"] } }
```

Full guide: [docs/INTEGRATIONS.md](docs/INTEGRATIONS.md) ·
Quickstart: [docs/QUICKSTART.md](docs/QUICKSTART.md)

## What's in this release

- `phantom` — REPL with streaming, multi-line, slash commands, inline tool calls
- `phantom mcp` — MCP stdio server (40 tools)
- `phantom serve` — WebSocket + embedded web dashboard (`http://host:7878`)
- `phantom onboarding` — browser-based setup wizard
- `phantom evolve --distributed` — parallel agent swarm across mesh
- `phantom coordinator` — zero-config peer discovery via mDNS
- `phantom swarm` / `phantom peer` — cluster delegation utilities

## Platforms

Pre-built binaries for v0.1.0-alpha:

- **macOS arm64** (`phantom-aarch64-apple-darwin`)
- **Windows x86_64** (`phantom-x86_64-pc-windows.exe`)
- **Linux arm64** (`phantom-aarch64-unknown-linux`)
- **Android arm64** (`phantom-aarch64-linux-android`)
- **iOS** — Tauri IPA (sideload via free Apple developer cert)

Other platforms (Linux x86_64, Android armv7, Windows arm64) — build from
source with `cargo build --release --target <triple>`.

## Known limitations (alpha)

- iOS sideload requires a free Apple developer cert (re-sign every 7 days)
- Windows Groq streaming has a 30s per-chunk timeout workaround
- Gemini free tier has daily quota limits
- Web dashboard's Tasks tab is a placeholder (v0.2)
- Tauri desktop app's React UI is separate from the web dashboard;
  unification is on the v0.2 roadmap

## What's next (v0.2)

- iOS Tauri app loads the same web dashboard as Mac/Win/Android
- xterm.js terminal panel inside the web dashboard
- Web Tasks tab — live task queue + history
- WASM sandbox for tool isolation
- Tool permission prompts (allow once / always)
- Markdown rendering in REPL output
- MCP client mode (consume other MCP servers)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Issues + PRs welcome.

## Security

Found a security issue? See [SECURITY.md](SECURITY.md).
