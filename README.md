# Phantom Mesh

**A self-hostable AI agent runtime that runs across your devices — Mac, Linux, Windows, Android, iOS — learns from each session, and lets you talk to it through CLI, chat channels, or the local web dashboard.**

[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)
[![Version](https://img.shields.io/badge/version-0.5.0-blue.svg)](core/Cargo.toml)
[![License](https://img.shields.io/badge/license-Apache--2.0-green.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/platforms-Mac%20·%20Linux%20·%20Win%20·%20Android%20·%20iOS-success.svg)](docs/)
[![Status](https://img.shields.io/badge/status-alpha-orange.svg)](RELEASE-NOTES.md)

> ⚠️ **Alpha quality.** APIs, flags, and tool surface can change between releases. Don't run untrusted prompts against a production machine without sandboxing. The stable surfaces (Hermes core loop, anti-hallucination V1, ~30-tool catalog, 12-provider fallback, cluster RPC with cap-aware forwarding) are flagged in §What's new. Many features sit behind `experimental-*` cargo flags by design.

```bash
git clone https://github.com/markl-a/phantom-mesh && \
  cd phantom-mesh/core && cargo install --path .
phantom onboarding              # interactive wizard, ~90 s
phantom service install         # auto-start at every login
phantom doctor                  # health check
```

That's the install. After it, your Mac (or Linux box / Windows desktop / Termux-on-Android / sideloaded iPhone) runs an agent that can talk to peers over Tailscale, route tasks by capability, and (behind the `experimental-hermes-*` flags) extract reusable skills from successful and failed sessions.

**Try this next**:

```bash
phantom serve &                              # start the local HTTP daemon
open http://127.0.0.1:7878/projects          # local web dashboard
phantom doctor --json | jq .status           # machine-readable health
```

Or use it from Claude Code as an MCP server:

```bash
claude mcp add phantom $(which phantom) mcp  # ~30 tools via mcp__phantom__*
```

## What's new in v0.5.0 (2026-05-17)

- **Hermes self-improvement loop wired end-to-end** (behind `experimental-hermes-curator` + `experimental-hermes-memory`) — judge, extract Skill from success or failure, store in FTS5 memory, recall into agent context next time. Alpha; the loop runs, but operator visibility into the skill bank ships in v0.6.0.
- **Cluster RPC with cap-aware forwarding** and **peer heartbeat with health-aware peer selection** (the heartbeat is behind `experimental-cluster-heartbeat`). Single-host two-port acceptance proven; multi-host real-mesh smoke targeted for v0.6.0.
- **OpenClaw real channels** (behind `experimental-openclaw`) — operator-verified live Telegram round-trip, real Slack `chat.postMessage` + signing-secret HMAC + replay defense, WhatsApp adapter as stub awaiting Meta Business verification.
- **30 Hermes tools** + **12 LLM providers** (Anthropic, Groq, OpenAI-compat, Mistral, xAI, Together, Fireworks, Cohere, Perplexity, AI21, NVIDIA NIM, `claude_cli`) with retry + rate-limit middleware.
- **4 of 4 CRITICAL security findings closed** (Tauri CSP, OAuth state binding, install-script SHA256+HTTPS). **4 of 5 V8 Tauri HIGH findings closed** post-cut. Full audit set under [`docs/superpowers/audits/`](docs/superpowers/audits/).
- **`cargo audit`**: 5 of 6 transitive RUSTSEC advisories cleared (teloxide upgrade). The 1 remaining (`rsa` Marvin Attack via the desktop OAuth path) is not reachable in our usage; rationale at `docs/superpowers/security/`.

See [RELEASE-NOTES.md](RELEASE-NOTES.md) for the full diff vs v0.4.0.

---

## 🌐 Ecosystem

phantom-mesh is the **agent runtime**. Around it, several satellite
projects extend it into specific application domains. They share the
same agent runtime, the same TOML-config style, and the same provider
fallback machinery.

| Repo | Layer | Status |
|---|---|---|
| **[phantom-mesh](https://github.com/markl-a/phantom-mesh)** | Core agent runtime (Rust, 5 platforms) | 🟢 Public, active |
| **[phantom-secops](https://github.com/markl-a/phantom-secops)** | Red/blue-team agent simulation toolkit, built on phantom-mesh runtime | 🟡 Public, alpha |
| **[phantom-mobile](https://github.com/markl-a/phantom-mobile)** | Android E2E testing with vision-LLM scenario judge, built on phantom-mesh | 🟡 Public, alpha |
| **[Data-Analysis-with-Agents](https://github.com/markl-a/Data-Analysis-with-Agents)** | Data-science / clustering / agent telemetry analytics (Streamlit) | 🟢 Public, active |
| **[Automation_with_Agent](https://github.com/markl-a/Automation_with_Agent)** | Applied automation + AIOps + MLOps tools (Python) | 🟢 Public, active |
| **[My-AI-Learning-Notes](https://github.com/markl-a/My-AI-Learning-Notes)** | 繁中 AI 學習路徑 + 面試準備教材 (17 ⭐) | 🟢 Public, active |

**Reading order for a new visitor**

1. Start here (phantom-mesh) — what the runtime does, how to install.
2. Pick a domain that matches your interest:
   - Security ops / red-blue team simulation → **phantom-secops**
   - Mobile / Android E2E testing → **phantom-mobile**
   - Data science / analytics → **Data-Analysis-with-Agents**
   - DevOps / RPA / agent tooling → **Automation_with_Agent**
   - 中文學習路徑 → **My-AI-Learning-Notes**

---

## Editions

phantom-mesh ships in two editions sharing the same OSS core:

- **OSS Edition** *(this repo)* — single self-hostable binary. Cluster
  authentication via Tailscale + HMAC shared secret. No account, no
  cloud broker, runs offline once a model is on disk. Apache 2.0
  forever, see §Contributing for the binding hard rules.
- **Cloud / Enterprise Edition** *(planned)* — a managed broker on top of
  the OSS core for OAuth identity, cross-mesh routing, and per-seat
  billing. Self-hostable for Enterprise so the IP-sensitive deployments
  never leave your perimeter. Design notes are private during the OSS-
  first phase; will publish a high-level spec before any paid surface
  ships.

The OSS Edition is the only path you need to evaluate, install, or
deploy phantom-mesh today. The Cloud Edition exists to make the OSS
core sustainable; nothing about the OSS surface is gated by a future
paid tier (six binding rules below).

---

## Why phantom-mesh

| | Claude Code | Codex CLI | Gemini CLI | OpenCode | **phantom** |
|---|---|---|---|---|---|
| Single-machine agent | ✅ | ✅ | ✅ | ✅ | ✅ |
| Cross-machine cluster | — | — | — | — | **✅** |
| Self-hostable | — | — | — | partial | **✅** |
| BYOK + offline (MLX) | — | — | — | — | **✅** |
| Auto-start daemon | — | — | — | — | **✅** |
| Auto-recovery on test fail | — | — | — | — | **✅ autoevolve** |
| APFS rollback safety net | — | — | — | — | **✅** |
| Mobile native (APK + IPA) | indirect | indirect | — | — | **✅** |

phantom is the only OSS coding agent designed from day 1 to span
**multiple machines, multiple platforms, and multiple LLM providers**
without phoning home to a vendor cloud. The whole binary works without
internet once a model is on disk.

---

## Five ways to drive it

1. **`phantom`** — full-screen ratatui TUI: streaming tokens, slash
   commands (`/cost /copy /undo /settings /provider /model /tasks
   /todo /agent /show /perm /density /theme /resume /fork /init …`),
   `@file` autocomplete, multi-line input, plan-mode gating.
2. **`phantom serve`** — HTTP+WebSocket daemon at `:7878` with a web
   UI (`/`), mobile chat (`/m`), embedded xterm.js terminal, Cmd+K
   palette, REST `/api/*` for cost / sessions / todos / nodes /
   tools/history / status / providers/health / version.
3. **`phantom mcp`** — MCP stdio server for Claude Code / Codex / Cursor.
   Exposes 50 tools as `mcp__phantom__*`. One-line registration via
   `claude mcp add phantom $(which phantom) mcp`.
4. **Mobile** — Signed Tauri APK (96 MB) + signed IPA (7 MB) at
   `<coordinator>/dist/phantom-mesh-{android.apk,ios.ipa}`. The
   webview redirects to your coordinator's mobile UI. See
   [docs/INSTALL-{ANDROID,IOS}.md](docs/INSTALL-ANDROID.md).
5. **`phantom autoevolve`** — daemon-mode self-improvement: poll
   `cargo check`, spawn `phantom evolve` when red, auto-commit when
   green. Optional hourly LaunchAgent / systemd timer / Scheduled Task.

---

## 50 built-in tools (12 categories)

| Category | Tools |
|---|---|
| Shell / file | `shell` `file_read` `file_write` `file_edit` `multi_file_edit` `apply_patch` `ls` `stat` |
| Search | `content_search` (ripgrep) `glob_search` `web_search` |
| Memory | `memory_store` `memory_recall` `memory_list` `memory_delete` `memory_search` |
| Git | `git_status` `git_diff` `git_log` `git_commit` `git_add` `git_checkout` `git_branch_list` `git_show` `git_blame` `git_stash_list` |
| Diagnostics | `cargo_check` `cargo_test` `tsc_check` `run_tests` |
| TODO | `todo_add` `todo_update` `todo_list` `todo_clear` |
| Diff | `diff_files` `diff_strings` |
| HTTP | `http_get` `http_post` `web_fetch` |
| Background | `bash_run_background` `bash_output` `bash_kill` |
| Interactive | `ask_user` |
| Multi-agent | `task` `subagent` `parallel_tasks` (with optional `node:` for cross-mesh) |
| **macOS-only** | **`spotlight_search`** (mdfind, sub-100ms system-wide), **`xcode_simctl`** |

Plus `phantom_swarm` and `phantom_evolve_distributed` exposed only
through the MCP wire protocol — that's where the 50 figure comes
from.

---

## Self-improvement loop

```bash
phantom evolve                            # one-shot test fix loop
phantom evolve --distributed              # decompose + dispatch to peers
phantom autoevolve --once                 # one iteration: cargo check + maybe-fix + commit
phantom autoevolve --watch --interval 300 # every 5 min until Ctrl-C
phantom autoevolve schedule install       # macOS LaunchAgent: --once hourly
phantom autoevolve log --n 10             # JSONL log inspector
```

Every iteration appends to `~/.phantom-mesh/autoevolve.log`; the
next round reads the last 8 entries back into the LLM prompt as
"recent fixes" memory. Auto-commits on green; `/undo` rolls back via
APFS local snapshot if the fix went sideways. Cross-mesh
`--distributed` mode fans the failing-test set to every peer in
parallel and the master commits the winning fix.

[docs/SCENARIOS-MULTIAGENT.md](docs/SCENARIOS-MULTIAGENT.md) maps
**95 use-case scenarios** (work + life across SWE / DS / firmware
personas) onto the four multi-agent patterns: single-agent, single-
machine `parallel_tasks`, cross-mesh `subagent({node})`, and
scheduled / event-driven daemons.

---

## Local LLM on Apple Silicon (zero per-token cost)

```bash
pip3 install mlx-lm
phantom mlx pull            # default Llama 3.1 8B 4-bit, 4.2 GB
phantom mlx serve           # OpenAI-compat at localhost:8080
```

Add to `agents.toml`:

```toml
[providers.mlx-local]
type          = "openai"
base_url      = "http://127.0.0.1:8080/v1"
api_key       = "mlx"
default_model = "mlx-community/Llama-3.1-8B-Instruct-4bit"

[agent.local]
provider = "mlx-local"
```

`phantom autoevolve --agent local` then runs the entire self-fix
loop on-device, fully offline. No competing CLI agent has this.

---

## Cluster

```toml
# ~/.phantom-mesh/agents.toml
[cluster]
node_name      = "mac-coordinator"
cluster_secret = "<shared HMAC string>"
peers = [
  "http://100.84.223.59:7879",   # ROG (Termux phantom)
  "http://100.107.205.98:7878",  # ayaneo (Win)
  "http://100.87.70.65:7879",    # yoyogood (Win)
]
```

Then dispatch:

```ts
mcp__phantom__subagent({
  agent: "master",
  prompt: "compile the iOS app",
  node:   "100.84.223.59:7879",   // run it on the Android worker
})
```

Authentication is SHA-256 HMAC on the request body with
`cluster_secret`. Designed for Tailscale-overlay tailnets
(stable IPs across NAT, end-to-end encryption).

---

## Five-platform install (one-liners)

| Platform | Command |
|---|---|
| **macOS** | `cargo install --path core && phantom onboarding && phantom service install` |
| **Windows** | `iex (iwr "$COORD/scripts/install-phantom-windows.ps1").Content` |
| **Linux** | `cargo install --path core && phantom service install` (systemd --user unit) |
| **Android (Termux)** | `curl -fsSL "$COORD/scripts/termux-setup.sh" \| sh` |
| **Android (Tauri APK)** | `curl -O $COORD/dist/phantom-mesh-android.apk` + open |
| **iOS** | `curl -O $COORD/dist/phantom-mesh-ios.ipa` + Apple Configurator 2 |

Per-platform docs:

- [docs/INSTALL-MAC.md](docs/INSTALL-MAC.md) — every file phantom creates, perf baseline, uninstall recipe
- [docs/INSTALL-LINUX.md](docs/INSTALL-LINUX.md) — apt/dnf prereqs, systemd --user unit, headless server (loginctl enable-linger)
- [docs/INSTALL-WINDOWS.md](docs/INSTALL-WINDOWS.md) — winget prereqs, Scheduled Task setup, Defender firewall rule, cross-compile shortcut
- [docs/INSTALL-ANDROID.md](docs/INSTALL-ANDROID.md) — Termux real worker + Tauri thin client + combining both
- [docs/INSTALL-IOS.md](docs/INSTALL-IOS.md) — sandbox limits, sideload paths, 7-day re-sign loop
- [docs/TROUBLESHOOTING-MAC.md](docs/TROUBLESHOOTING-MAC.md) — every macOS footgun we hit + fix
- [docs/MLX-PROVIDER.md](docs/MLX-PROVIDER.md) — Apple Silicon local LLM, perf table by model size
- [docs/SCENARIOS-MULTIAGENT.md](docs/SCENARIOS-MULTIAGENT.md) — 95 use-cases mapped to multi-agent commands
- [docs/PERMISSIONS.md](docs/PERMISSIONS.md) — Tool(specifier) DSL reference + worked policy examples

---

## Configuration

`~/.phantom-mesh/agents.toml`:

```toml
[core]
host = "127.0.0.1"
port = 7878

[providers.anthropic]
type          = "anthropic"
api_key       = "sk-ant-..."
default_model = "claude-sonnet-4-5-20251022"

[providers.groq]
type          = "groq"
api_key       = "gsk_..."
default_model = "llama-3.3-70b-versatile"

[agent.master]
provider     = "anthropic"
instructions = "You are phantom, a helpful AI agent."
tools        = ["shell", "file_read", "file_edit", "content_search",
                "git_status", "git_diff", "git_commit", "task"]
```

Multiple providers fall back in `agents.toml` order. The fastest
free option to start with is **Groq Llama 3.3 70B** (free tier 1M
TPD per model). For local-first, run `phantom mlx serve` and route
agents to it.

---

## Quality gates

```bash
phantom doctor                 # 9-section single-screen check
./scripts/test-mac.sh          # 51 automated checks (Mac)
./scripts/validate-mcp.sh      # MCP-specific 4-check
```

Output: `PASS / FAIL / SKIP` glyphs + final tally + names of any
failures. All three are read-only and safe to run while phantom serve
is live.

---

## Architecture

```
phantom binary  (Rust, single static)
├── REPL / TUI / line REPL  ← rustyline + ratatui
├── HTTP + WebSocket server ← axum, embedded web bundle
├── MCP stdio server        ← JSON-RPC 2024-11-05
├── Agent runtime           ← provider-agnostic ChatMessage flow
│   ├── streaming + tool calls + thinking trace
│   ├── 50 tools (cfg-gated by platform)
│   └── subagent + parallel_tasks (multi-agent)
├── Cluster                 ← HMAC-auth'd /rpc/* RPCs over Tailscale
├── Cost tracker            ← per-request, per-task, per-session, per-model
├── Memory                  ← key-value with namespaces, ~/.phantom-mesh/memory.json
├── Snapshot                ← APFS local snapshot (macOS), apply with sudo
├── Self-evolve             ← cargo check/test → fix loop → auto-commit
└── Service manager         ← launchd / systemd --user / Scheduled Task
```

50 tools, 12 subcommands, 30 slash commands, 17 HTTP routes, 5
multi-agent topologies. ~10k lines of Rust + 600 lines of embedded
web (mobile + desktop).

---

## Building from source

```bash
git clone https://github.com/markl-a/phantom-mesh
cd phantom-mesh/core
cargo build --release --bin phantom
./target/release/phantom --version
```

Cross-compile for the cluster:

```bash
# Windows (Apple Silicon Mac → x86_64-pc-windows-gnu)
brew install mingw-w64 && rustup target add x86_64-pc-windows-gnu
cargo build --release --bin phantom --target x86_64-pc-windows-gnu

# Linux aarch64
cross build --release --bin phantom --target aarch64-unknown-linux-gnu

# Android aarch64
cargo install cargo-ndk
cargo ndk -t arm64-v8a build --release
```

---

## Contributing

phantom is Apache 2.0; a future commercial layer (managed Cloud /
Enterprise broker) is planned but never gates the OSS binary. PRs
welcome.

Hard rules for contributors (the binding-forever list):

1. Every shipped OSS feature stays Apache 2.0 forever.
2. The OSS binary works fully without any cloud account.
3. Telemetry off by default + forever opt-in + local-first.
4. The commercial broker is self-hostable for Enterprise.
5. No artificial cluster-size gates.
6. Provider keys never leave the user's machine.

If a PR would force a cloud-only path, the reviewer template asks
"does this also work in OSS-only mode?" — if not, it moves to the
`phantom-cloud` repo (planned, not yet open).

---

## License

[Apache 2.0](LICENSE)
