# Phantom Mesh — Architecture (as built)

[繁體中文版](ARCHITECTURE.zh-TW.md)

> Describes the **implemented** architecture of `core/src/` as of **v0.6.0 (2026-07)**.
> Honest markers: 🟢 solid & tested · 🟡 works, still evolving · 🧪 experimental / feature-gated.
> Deep per-subsystem notes live in [`docs/architecture/`](architecture/).

---

## 1. Overview

Phantom Mesh is a **single Rust binary** (`phantom`) that acts as a personal AI runtime:
an interactive CLI/TUI, a long-running daemon, and a mesh peer — all in one executable.
Several machines you own (Windows / macOS / Linux / Android / iOS clients) form a private
**mesh** over Tailscale or any shared network; tasks are authenticated with a shared
HMAC cluster secret and routed to whichever node is best suited.

```
┌────────────────────────────── phantom node ──────────────────────────────┐
│                                                                          │
│  entrypoints    phantom (TUI) · repl · exec · serve · mcp · evolve ·     │
│                 swarm · service · status · inbox …                       │
│                                                                          │
│  serve (Axum)   /ws  /api/*  /rpc/*  /mcp  ·  /m  = mobile war-room PWA  │
│                 (manifest.webmanifest + sw.js → installable, standalone) │
│                                                                          │
│                    ┌────────────────────────────┐                        │
│                    │        AgentRuntime        │  tool-calling loop     │
│                    └──────────────┬─────────────┘                        │
│                                   │                                      │
│       ┌───────────────────────────┴───────────────────────────┐          │
│       │  provider resolver · failover chain · circuit breaker │          │
│       └──────┬──────────────────────────────────┬─────────────┘          │
│              │ HTTP providers                   │ subscription-CLI       │
│  openai-compat / gemini / groq / OAuth device   │ backends (L0           │
│  flows / local Ollama · extras 🧪 (mistral,     │ cli_session PTY):      │
│  cohere, fireworks, together, nvidia,           │ claude · codex ·       │
│  perplexity, xai, ai21) behind a feature flag   │ gemini-cli · opencode  │
│                                                 │ … zero API keys on disk│
│                                                                          │
│  tool layer     ~60 built-in tools + cluster RPC, ALL behind:            │
│                 tool_gate (process-wide) · `Tool(specifier)` permission  │
│                 DSL · project trust · node capabilities                  │
│                                                                          │
│  governance     risk-tiered pending approvals (approve / stop) ·         │
│                 governed_run flight recorder: signed transcripts +       │
│                 task events + approval decisions                         │
│                                                                          │
│  identity /     per-device 64-byte root IKM (identity.key) + ed25519     │
│  encryption     signing keys · age + HKDF-SHA256 · OS keystore cutover   │
│                 (macOS/iOS Keychain 🟢 · Win DPAPI 🟢 · Android 🟡)        │
│                                                                          │
│  memory         owned-memory on SQLite FTS5 — cross-session recall,      │
│                 default-on, kill switch                                  │
│                                                                          │
│  MCP            server (stdio + /mcp): exposes tools/memory/cluster ·    │
│                 client: external MCP servers become agent tools          │
│                                                                          │
│  channels       remote_control: Telegram 🟢 · Slack 🟢 · WhatsApp stub · │
│                 persona bindings · rate limiting · webhook auth          │
│                                                                          │
│  self-improve   evolve (test-driven fix loop) · autoevolve daemon ·      │
│                 checkpoints / replay / peer handoff                      │
│                                                                          │
│  multi-node     mesh peer manager · swarm fan-out · crew multi-CLI       │
│                 pipelines · fleet shared-backlog dev loop                │
└──────────────────────────────────────────────────────────────────────────┘
          │ Tailscale VPN (or any reachable IP), HMAC-authenticated │
     ┌────▼─────┐        ┌──────────┐        ┌───────────┐
     │  node B  │  ···   │  node C  │  ···   │ mobile /  │
     └──────────┘        └──────────┘        │ web PWA   │
                                             └───────────┘
```

## 2. Core loop — `agent.rs` / `runtime.rs` 🟢

`AgentRuntime` drives the LLM conversation: prompt → model → tool calls → results →
model, up to a bounded number of rounds with stall detection and context compaction.
Sessions persist to disk (`session.rs`, JSONL) with `/compact` LLM summarization.
`context.rs` scopes workspace state; `cost.rs` tracks per-turn and lifetime spend
(`/cost` in the REPL).

## 3. Providers — `providers/` 🟢

One trait, two families:

- **HTTP providers** — a default core (OpenAI-compatible, Gemini, Groq, OAuth device
  flows for subscription accounts, local Ollama) plus an extra adapter family
  (Mistral, Cohere, Fireworks, Together, NVIDIA, Perplexity, xAI, AI21, …) gated
  behind `experimental-extra-providers` 🧪. Keys come from env vars or the built-in
  vault — never from the repo.
- **Subscription-CLI backends** (`claude_agent`, `codex_agent`, `opencode_agent`, …) —
  phantom drives a locally signed-in coding-agent CLI through the **L0 `cli_session`
  substrate** (PTY bridge). No API keys are stored at all; the marginal cost of a
  flat-rate subscription is $0.

`resolver.rs` builds an explicit failover order, with 429/5xx retry backoff and a
circuit breaker. `credential_scanner.rs` detects which
subscription CLIs are signed in on the host (used by onboarding — the generated
`agents.toml` contains provider *types* only).

## 4. Tools & enforcement — `tools/`, `tool_gate.rs`, `permission.rs`, `capabilities/` 🟢

~60 built-in tools (shell, file I/O, search, git, memory, web, …) plus cluster-RPC
tools. Every call funnels through a single **process-wide tool gate**:

1. `permission.rs` — Claude-Code-style `Tool(specifier)` allow / ask / deny rules
2. `project_trust.rs` — per-directory trust before enforcement applies
3. `capabilities/` — tasks may declare `required_caps`; nodes advertise detected
   hardware/OS capabilities and refuse work they cannot honor

## 5. Serve daemon & clients — `serve.rs`, `web/` 🟢

`phantom serve` starts an Axum server: WebSocket (`/ws`), REST (`/api/*`), cluster RPC
(`/rpc/*`), an MCP endpoint, and the **mobile war-room console at `/m`** — a real
installable PWA (standalone display mode, service worker) showing the node grid,
orchestrator plans, live costs, MCP tool exposure, and the governance flight recorder.
Native clients: the Tauri desktop app (`app/`), a generated iOS Xcode project, and
Telegram.

## 6. Mesh — `mesh.rs`, `swarm.rs`, `crew/`, `fleet/` 🟡

Peers authenticate with a shared **HMAC cluster secret** (unset ⇒ refuse, fail-closed).
The peer manager tracks health and routes tasks by load and capabilities. `swarm` fans
one prompt out to every online node and synthesizes the answers. `crew` composes
multi-CLI pipelines (writer / reviewer roles across different vendor CLIs). `fleet`
implements the shared-backlog development loop (atomic claim → work → verified done →
cross-review) that phantom's own development runs on.

## 7. Governance — `approval.rs`, `governed_run/` 🟢

Tool actions classified at high risk tiers (e.g. shell commands rated `execute_high`) enter a **pending-approval queue**
surfaced in the TUI, the web console, and the mobile app (approve / stop). Every
governed run is written to a **flight recorder**: signed transcripts, task events, and
approval decisions with their enforcement mode (`auto`, `pre_action_blocking`) —
auditable after the fact.

## 8. Identity, encryption & vault — `identity*.rs`, `encryption_wire.rs`, `vault/` 🟢

Each device mints a **64-byte root IKM** (`identity.key`); separate **ed25519 signing
keys** live alongside it. Event payloads are encrypted with **age** using keys derived
via **HKDF-SHA256**. The root identity is being cut
over from a 0600 file to **OS-native keystores** — macOS/iOS Keychain and Windows
DPAPI have landed, Android Keystore is in progress — with byte-identical migration and
recovery guards (a restored keystore value must derive the *same* event key, never a
fresh one). `broker_vault_wire.rs` wraps per-device secrets for the optional cloud
broker.

## 9. Memory — FTS5 owned-memory 🟢

Two layers. The simple agent tools (`memory_store` / `memory_recall`) persist a JSON
key-value store under `~/.phantom-mesh/`. The **owned-memory** layer indexes events,
captures, and skills into SQLite FTS5 (event storage / skill / capture wires) for
cross-session recall — default-on with a kill switch. This is the "compounds the
longer you use it" layer.

## 10. MCP — `mcp.rs` / `mcp_client.rs` 🟢

Phantom is both an **MCP server** (stdio for Claude Desktop / Cursor, plus `/mcp` over
HTTP — exposing tools, memory, and cluster dispatch) and an **MCP client** (external
MCP servers become tools inside the agent loop). The satellite ecosystem (secops,
finance, quant, tutor, …) plugs in through this interface.

## 11. Remote-control channels — `remote_control/` 🧪

Chat channels as mesh remotes, feature-gated per channel: **Telegram is live**
(long-poll bot, agent dispatcher, media handling) and **Slack is live** (outbound
`chat.postMessage`, HMAC-verified inbound webhooks); WhatsApp is still a
compile-checked stub behind its flag. A shared `Channel` trait, per-channel
**persona** bindings, and token-bucket rate limiting sit underneath.

## 12. Self-improvement — `evolve*`, `autoevolve` 🟡

`phantom evolve` runs a test-driven fix loop (optionally distributed across the mesh,
with LLM-judge ensembles and skill extraction). `autoevolve` is the daemon form:
watch → fix → auto-commit when tests are green, with OS-scheduler integration and
JSONL logs. Checkpoints can be listed, replayed, and handed off to a peer node.

## 13. Life-Node layer — `life_node/` 🟡

The personal-data plane: multimodal capture (image / audio / text via Groq, Gemini,
Ollama with fallback), focus / food / habit wires, daily review, and coach delivery —
the Life Track features, riding on the same runtime, storage, and encryption.

## 14. Skillbank 🧪

An experimental six-step skill loop (curate → store → recall → compose → verify →
evolve) behind `experimental-skillbank`; the calculator / unit-convert / json-query
tool family lives here with hard test gates.

---

### Reading order for new contributors

`lib.rs` → `agent.rs` → `providers/resolver.rs` → `tool_gate.rs` → `serve.rs` →
`mesh.rs` → `governed_run/` → `identity_wire.rs`.
