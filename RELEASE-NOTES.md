# Release notes

## v0.5.0 — 2026-05-17

First release after the post-v0.4.0 batch wave. Alpha; many surfaces still
behind `experimental-*` cargo flags. v0.6.0 (target 2026-06-15) adds the
mobile + web cluster-dispatch UIs and the cross-host real-mesh smoke proof.

### Headline changes vs v0.4.0

- **Hermes self-improvement loop wired end-to-end** — judge → extract Skill
  (from both success and failure polarities) → store in FTS5 memory → recall
  into next agent context. Behind `experimental-hermes-curator` +
  `experimental-hermes-memory`. The Curator V2 (multi-LLM-judge consensus)
  ship with it. Operator UI for the skill bank is v0.6.0.
- **Cluster mesh** — RPC capability-aware task forwarding (cycle-guard +
  HMAC re-sign + idempotency) + peer heartbeat with health-aware peer
  selection. Single-host two-port acceptance proven; cross-host real-mesh
  smoke targeted for v0.6.0. Heartbeat is behind
  `experimental-cluster-heartbeat`.
- **OpenClaw real channels** (behind `experimental-openclaw`) —
  operator-verified live Telegram round-trip, real Slack `chat.postMessage`
  with signing-secret HMAC + replay defense, persona system, per-channel
  rate-limit token bucket, multimodal media handling, multi-bot dispatcher.
  WhatsApp ships as a stub adapter; real impl waits on Meta Business
  verification.
- **Anti-hallucination V1** — Shape-1 deterministic scanner (`hallucination::scanner`)
  emits `AgentEvent::ConsistencyWarning` for repeated assertion patterns.
  Behind `experimental-anti-hallucination`. Shapes 2–6 are v0.7.0.
- **12 LLM providers** — Anthropic, Groq, OpenAI-compat, Mistral, xAI,
  Together, Fireworks, Cohere, Perplexity, AI21, NVIDIA NIM, `claude_cli`.
  Hermes-extended providers are behind `experimental-hermes-providers`.
  Provider retry + streaming SSE retry middleware shared across all of them.
- **30 Hermes tools** — adds `jq`, `xml_to_json`, `yaml_to_json`,
  `url_parse`, `color_hex_rgb`, `uuid_v7`, `jaro_winkler`, `template_render`,
  `text_summarize`, `regex_extract`, `string_metrics`, etc. on top of the
  existing catalog. Input-size caps + depth limits enforced per tool.

### Security

- **4 of 4 CRITICAL findings closed** — Tauri CSP narrow (V8 C-1), OAuth
  state HMAC binding across start→finish (V8 C-2), install scripts
  SHA256+HTTPS (V10 CRIT-1 + CRIT-2).
- **4 of 5 V8 Tauri HIGH findings closed** post-cut — `shell:*` capability
  drop + `open_external_url` URL validator hardening + `asset://` protocol
  scope lock-down + deep-link callback handler validation.
- **TUI-1/TUI-2 closed** — `/clear` and `/resume` cancel `current_task`,
  `tui-history` created at `0o600`.
- **`cargo audit`** — 5 of 6 transitive RUSTSEC advisories cleared via the
  `teloxide 0.13 → 0.17` upgrade. The 1 remaining (`rsa 0.9.10` Marvin
  Attack via `jsonwebtoken` in `app/src-tauri`) is not reachable in our
  desktop OAuth `id_token` verify path; rationale and re-evaluation
  triggers at `docs/superpowers/security/`.
- **Sandbox hardening** — V9 H-1 sandbox path guard, V9 H-3 background
  shell honors confirmation, V11 H-1 apksigner credentials via
  `pass:env:`, multiple MCP HIGH follow-ups (`isError` + mutex + stderr
  drain).
- **CI security regression watch** — `cargo-audit-nightly.yml` covers
  `core/` + `crates/pm-types/` + `app/src-tauri/` (the lockfile that
  actually carries the `rsa` transitive).

### Operator surfaces

- **Android M1 verification** generalized off ROG-Phone-6 to any aarch64
  Android with Termux; AVD-CI workflow is the ~80% unattended substitute.
- **Cloudflare-creds (L1)** — runbook at `docs/superpowers/runbooks/` for
  the operator to unblock the R2 binary download path.

### Breaking / behavioral notes

- `phantom autoevolve schedule install` is still available but the
  underlying Hermes integration matured significantly — re-run
  `phantom doctor` after upgrade to verify the loop wires up on your
  config.
- `experimental-cluster-heartbeat` (default OFF) — when enabled, peers
  marked Unhealthy after `heartbeat_failure_threshold × heartbeat_interval_secs`
  (defaults 3 × 30s = 90s). Disabled by default to preserve exact pre-v0.5.0
  selection behavior.

### What's NOT in v0.5.0 (and where it lands)

- Mobile cluster dispatch UI (4 new app screens) → v0.6.0 E002
- Web cluster dashboard (`phantommesh.io/app`) → v0.6.0 E003
- Cross-host real-mesh smoke (2-node testbed) → v0.6.0 E001
- Hermes Skill Bank UI (browse, search, timeline) → v0.6.0 E005
- 30-second hello-world install + first-dispatch flow → v0.6.0 E006

---

## v0.1.0-alpha

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
