# Phantom Mesh Architecture

> This document describes the **current, implemented** daemon architecture based on
> the actual source code in `core/src/`. It is the authoritative reference for
> developers working on the codebase.

---

## 1. Overview

Phantom Mesh is a distributed AI agent daemon written in Rust. Each node runs a
self-contained HTTP server (Axum, port 7878) that exposes a tool-calling agent
loop backed by any OpenAI-compatible LLM provider. Multiple nodes can form a P2P
mesh over Tailscale or any shared network — tasks are authenticated with a shared
HMAC secret and forwarded to the least-loaded peer. Clients reach the daemon via
HTTP REST, a Tauri desktop app, or Telegram.

```
┌─────────────────────────────────────────────────────────────┐
│                     phantom-mesh node                        │
│                                                              │
│  ┌──────────┐    ┌──────────────────┐   ┌────────────────┐  │
│  │  Tauri   │    │   Axum HTTP      │   │  Telegram Bot  │  │
│  │  Desktop │    │   :7878          │   │  (long-poll)   │  │
│  └────┬─────┘    └────────┬─────────┘   └──────┬─────────┘  │
│       │                   │                    │             │
│       └───────────────────▼────────────────────┘             │
│                      AgentRuntime                            │
│                  (up to 20-round tool loop)                  │
│                           │                                  │
│              ┌────────────▼────────────┐                     │
│              │  call_with_fallback()   │                     │
│              │  primary → others       │                     │
│              │  (exp. backoff, 429     │                     │
│              │   Retry-After honored)  │                     │
│              └────────────┬────────────┘                     │
│                           │ OpenAI-compat POST               │
│          ┌────────────────▼───────────────────┐              │
│          │   Configured LLM Provider(s)        │              │
│          │  openai / groq / gemini / compat    │              │
│          └─────────────────────────────────────┘              │
│                                                              │
│    ┌────────────────────────────────────────────────────┐    │
│    │            Tool Executor  (13 tools)                │    │
│    │  shell │ file_read │ file_write │ file_edit         │    │
│    │  content_search │ glob_search │ web_search          │    │
│    │  memory_store │ memory_recall                       │    │
│    │  git_status │ git_diff │ git_log │ git_commit        │    │
│    └────────────────────────────────────────────────────┘    │
│                                                              │
│  ┌────────────────┐  ┌───────────────┐  ┌───────────────┐   │
│  │ConversationStore│  │  CostTracker  │  │ClusterManager │   │
│  │ JSONL on disk  │  │ costs.json    │  │ /rpc/* routes │   │
│  └────────────────┘  └───────────────┘  └───────────────┘   │
└─────────────────────────────────────────────────────────────┘
              │  Tailscale VPN (or any reachable IP)  │
         ┌────▼────┐                            ┌─────▼────┐
         │ Node B  │                            │  Node C  │
         │  GCP    │                            │  iPhone  │
         └─────────┘                            └──────────┘
```

---

## 2. Core Components

### AppState

The single shared state object cloned into every Axum handler via
`axum::extract::State`. It is constructed once in `main()`, hydrated by
`load_config_toml()`, and then Arc-wrapped so clones are cheap.

**Key fields:**

| Field | Type | Purpose |
|---|---|---|
| `agent_runtime` | `AgentRuntime` | Runs the tool-calling loop |
| `llm_router` | `LLMRouter` | Holds provider health summaries |
| `tool_registry` | `ToolRegistry` | List of enabled tool names |
| `conversations` | `ConversationStore` | Persistent per-chat history |
| `cost_tracker` | `CostTracker` | Cumulative token accounting |
| `cluster_manager` | `ClusterManager` | P2P peer manager + auth |
| `job_store` | `JobStore` | In-memory async job status map |
| `telegram_config` | `Option<TelegramConfig>` | Bot config from agents.toml |

**File:** `core/src/lib.rs`

---

### AgentRuntime

The execution engine. `AgentRuntime::run()` drives a multi-round conversation
with the LLM, executing tool calls between rounds until the model stops issuing
them or the guard limits are hit.

**Guard limits:**

- `MAX_ROUNDS = 20` — hard ceiling on tool-call rounds
- `STALL_THRESHOLD = 2` — consecutive rounds with identical output break the loop early
- `TOKEN_BUDGET = 60_000` (estimated tokens) — triggers context compaction

**Context compaction** (triggered when estimated tokens > 60 K):

```
system messages preserved
summary injection: "[Context compacted: N earlier messages dropped]"
last 12 conversation messages kept
any leading "tool" role messages stripped (would confuse the LLM)
```

**Tool-calling loop per round:**

```
call_with_fallback() → LLM response JSON
  ├─ record token usage → CostTracker
  ├─ extract content text (may be empty when tool calls present)
  └─ tool_calls array present?
       yes → execute_tool() for each call
             → append tool result messages
             → continue loop
       no  → stall check → break
```

**Key structs:** `AgentRuntime`, `AgentResult`

**File:** `core/src/lib.rs` (lines 663–1649)

---

### LLMRouter + call_with_fallback()

`LLMRouter` is a thin wrapper around a `Vec<ProviderHealthSummary>` built at
config load time. Its primary use is reporting provider health to the dashboard.

The actual provider selection is done at runtime inside
`AgentRuntime::call_with_fallback()`:

```
provider_names = [agent.provider] + sorted(all_other_providers)

for each provider (attempt 0, 1, 2, …):
  1. look up api_key (direct or from env var)
  2. resolve endpoint URL:
       explicit url   → use as-is (append /v1/chat/completions if needed)
       type=openai    → https://api.openai.com/v1/chat/completions
       type=groq      → https://api.groq.com/openai/v1/chat/completions
       type=gemini    → https://generativelanguage.googleapis.com/v1beta/openai/...
       _              → https://openrouter.ai/api/v1/chat/completions
  3. exponential backoff before retries: 0s, 1s, 2s, 4s, …
  4. POST OpenAI-compat JSON body
  5. HTTP 2xx → return (response_json, model_name)
  6. HTTP 429 → honour Retry-After (≤30s), then try next provider
  7. HTTP 5xx → try next provider immediately
```

All providers must expose an OpenAI-compatible `/v1/chat/completions` endpoint.

**Key structs:** `LLMRouter`, `LLMRouterInner`, `ProviderHealthSummary`

**File:** `core/src/lib.rs`

---

### Tool Executor

`execute_tool(name, args, tools_config)` is a large `match` block — no dynamic
dispatch, no trait objects. Each tool is a simple async function body embedded in
one match arm.

**13 tools:**

| Tool | Description |
|---|---|
| `shell` | Run a shell command. Blocklist guards against `rm -rf /`, fork bombs, etc. Sequences of `;`/`&&` commands are split and run individually. 30 s default timeout. Output truncated at 20 K chars. |
| `file_read` | Read a file. Path resolved via `safe_path()` (canonicalize if exists). |
| `file_write` | Write a file; creates parent directories automatically. |
| `file_edit` | Replace an exact string in a file; errors if match count ≠ 1. |
| `content_search` | Regex/literal search via `rg` (falls back to `grep`). Up to 50 matches. |
| `glob_search` | Find files by glob pattern via `find`. Excludes `node_modules`, `.git`, `target`. |
| `web_search` | Brave Search API if `brave_search_api_key` configured, otherwise DuckDuckGo instant-answer API (no key required). |
| `memory_store` | Write a key→value pair to `~/.phantom-mesh/memory.json`. |
| `memory_recall` | Read a key from `~/.phantom-mesh/memory.json`. |
| `git_status` | `git status --short` for a path. |
| `git_diff` | `git diff --stat` (optionally `--cached`, optionally scoped to a file). |
| `git_log` | `git log --oneline -N` for a path. |
| `git_commit` | `git commit -am <message>` for a path. |

**File:** `core/src/lib.rs` (lines 904–1313)

---

### ClusterManager

Manages configured peer nodes. Peers are declared as plain URLs in `agents.toml`
— no automatic discovery. Tailscale or any routable IP is the transport.

**Responsibilities:**

- Keep a cached `Vec<PeerStatus>` for each peer (online/offline, active tasks, uptime)
- `ping_peer(url)` — POST `{peer}/rpc/ping`, update cached status
- `refresh_all()` — ping all peers in parallel
- `make_auth_token(body)` — SHA-256(`cluster_secret` ‖ `body`) → hex string
- `verify_auth(token, body)` — constant-time comparison via the `subtle` crate
- `assign_task_to_best_peer(agent, prompt)` — pick the online peer with the fewest active tasks, POST to its `/rpc/task/assign` with `X-Cluster-Auth` header

**Key structs:** `ClusterManager`, `ClusterConfig`, `PeerStatus`

**File:** `core/src/mesh.rs`

---

### ConversationStore

Persistent per-conversation history backed by JSONL files on disk. One file per
`chat_id`; lines are newline-delimited JSON `ChatMessage` objects.

**Write path:** disk first, then update in-memory cache (disk is authoritative).

**Read path:** if `chat_id` not in cache, load from disk into cache, then return.

**Storage location:** `~/.phantom-mesh/conversations/{chat_id}.jsonl`

`chat_id` examples:
- `daemon` — default for direct HTTP calls
- `tg:{telegram_chat_id}` — Telegram conversations
- `rpc` — tasks received from peer nodes

**Key structs:** `ConversationStore`, `ChatMessage` (in `providers/traits.rs`)

**File:** `core/src/lib.rs` (lines 534–637)

---

### CostTracker

Accumulates token usage and USD cost estimates across all LLM calls. Data is
persisted to `~/.phantom-mesh/costs.json` after every `record()` call (synchronous
`fs::write`).

**Pricing table** (April 2026, per million tokens — input / output):

| Model family | Input | Output |
|---|---|---|
| claude-opus-4 | $15 | $75 |
| claude-sonnet-4 | $3 | $15 |
| claude-haiku-4 | $0.80 | $4 |
| gpt-4o | $2.50 | $10 |
| gpt-4.1 | $2 | $8 |
| gemini-2.5-pro | $1.25 | $10 |
| gemini-2.0-flash | $0.10 | $0.40 |
| groq / llama | $0.05 | $0.08 |
| (default) | $1 | $3 |

**Key structs:** `CostTracker`, `CostTrackerInner`

**File:** `core/src/lib.rs` (lines 417–506)

---

### TelegramBot

A minimal long-poll Telegram bot backed by plain `reqwest`. No third-party
Telegram library is used. The polling loop runs in a `tokio::spawn` task started
from `main()` when `[telegram]` is configured.

**Polling loop (in `main.rs`):**

```
loop:
  poll_updates(offset) → Vec<(chat_id, user_id, text, update_id)>
  for each update:
    if user not in allowed_users → skip (advance offset)
    load history from ConversationStore("tg:{chat_id}")
    AgentRuntime::run(agent_name, text, history, …, extra="Be concise.")
    send_message(chat_id, result.output)  // splits at 4000-char boundaries
  on error: sleep 5s, retry
```

Messages are sent as HTML (`parse_mode: HTML`). If parsing fails Telegram-side,
the bot automatically retries as plain text.

**Key structs:** `TelegramBot`

**File:** `core/src/channels/telegram.rs`

---

### ProjectContext

Walks up the directory tree from the current working directory looking for project
context files to inject into the agent system prompt.

**Search order per directory:**
1. `PHANTOM.md`
2. `.phantom-mesh/context.md`

Stops at the user's home directory or filesystem root.

**Usage:** the loaded context string is passed as `extra_context` to
`AgentRuntime::run()`, where it is appended to the system prompt with two
newlines as separator.

**Key functions:** `load_project_context()`, `load_cwd_context()`, `load_from_path()`

**File:** `core/src/project_context.rs`

---

## 3. Request Flow

Typical agent request via `POST /agent/master/run`:

```
1.  HTTP POST /agent/{name}/run
      body: { "prompt": "...", "chat_id": "..." }

2.  agent_run() handler (main.rs)
      ├─ load conversation history from ConversationStore(chat_id)
      └─ call AgentRuntime::run_tracked(name, prompt, history, …)

3.  AgentRuntime::run_with_cost() — tool-calling loop begins
      ├─ look up agent config by name (fall back to "master")
      ├─ build tool_defs[] from agent.tools list (OpenAI function schemas)
      └─ assemble messages[]:
           [system]  agent.instructions
                     + CRITICAL RULES (if tools enabled)
                     + extra_context (PHANTOM.md if found)
           [history] prior ChatMessage turns
           [user]    current prompt

4.  Round 0: call_with_fallback(agent_cfg, messages, tool_defs)
      ├─ try agent's primary provider
      ├─ on failure: exponential backoff → try remaining providers in order
      └─ return (response_json, model_name)

5.  Record prompt_tokens + completion_tokens → CostTracker

6.  Parse response:
      ├─ content text → saved as final_output candidate
      └─ tool_calls[] present?

7a. No tool_calls → stall check → break loop
      final_output is the response text

7b. tool_calls present → for each call:
      execute_tool(fn_name, fn_args, tools_config) → result string
      truncate result to 20 K chars
      append { role:"tool", tool_call_id, content:result } to messages[]

8.  Back to step 4 with the extended messages[] (next round)
    Repeat until: no tool_calls | stall detected | MAX_ROUNDS (20) reached

9.  Return AgentResult { output, tool_calls_made, elapsed_secs }

10. agent_run() handler:
      ├─ append (user_msg, assistant_msg) to ConversationStore
      └─ return JSON { agent, output, tool_calls, elapsed }
```

---

## 4. P2P Mesh Protocol

### Node Discovery

Nodes are **config-based** — there is no automatic mDNS or DNS-SD discovery in
the current implementation. Each node lists its peers explicitly in `agents.toml`:

```toml
[cluster]
peers = ["http://100.64.0.2:7878", "http://100.64.0.3:7878"]
cluster_secret = "shared-hmac-key"
node_name = "my-node"
```

Tailscale provides the VPN layer so nodes can reach each other by stable IP
across networks without exposing ports to the public internet.

### Authentication

Every RPC request between nodes must include an `X-Cluster-Auth` header:

```
token = SHA-256(cluster_secret_bytes || request_body_bytes)
      formatted as lowercase hex
```

Verification uses constant-time comparison (via the `subtle` crate) to prevent
timing oracle attacks. Requests with a missing or incorrect token are rejected
with HTTP 401. If `cluster_secret` is empty or absent, **all** inbound cluster
RPC requests are rejected.

### Task Assignment Protocol

```
Caller node                          Callee node
    │                                     │
    │  POST /rpc/task/assign              │
    │  X-Cluster-Auth: <token>            │
    │  { agent:"master", prompt:"..." }   │
    │─────────────────────────────────────▶│
    │                                     ├─ verify_auth()
    │                                     ├─ generate job_id (UUID v4)
    │                                     ├─ JobStore.insert(job_id, "running")
    │                                     ├─ tokio::spawn(AgentRuntime::run())
    │  202 Accepted                       │
    │  { "job_id": "uuid-..." }           │
    │◀─────────────────────────────────────│
    │                                     │
    │  (poll until done)                  │
    │  GET /rpc/task/status/{job_id}      │
    │─────────────────────────────────────▶│
    │  { status:"running" }               │
    │◀─────────────────────────────────────│
    │                                     │   (agent finishes)
    │  GET /rpc/task/status/{job_id}      │
    │─────────────────────────────────────▶│
    │  { status:"done", output:"..." }    │
    │◀─────────────────────────────────────│
```

### Peer Health

`ClusterManager::refresh_all()` pings all configured peers in parallel via
`POST {peer}/rpc/ping`. Each peer responds with:

```json
{
  "name": "node-name",
  "version": "0.x.y",
  "uptime_secs": 3600,
  "active_tasks": 2,
  "online": true
}
```

`assign_task_to_best_peer()` selects the online peer with the lowest
`active_tasks` count.

### RPC Endpoints

| Method | Path | Purpose |
|---|---|---|
| POST | `/rpc/ping` | Return this node's status (no auth required) |
| GET | `/rpc/peers` | List all configured peers with cached status |
| POST | `/rpc/task/assign` | Accept a task from a peer (auth required) |
| GET | `/rpc/task/status/:job_id` | Poll async task result |

---

## 5. Configuration

All configuration lives in `~/.phantom-mesh/agents.toml` (default) or a path
supplied via `--config`.

```toml
# ── Core server settings ──────────────────────────────────────────────────
[core]
host = "0.0.0.0"      # bind address (default: 0.0.0.0)
port = 7878           # HTTP port    (default: 7878)
hub_api_key = "..."   # optional key for external hub integrations

# ── LLM provider definitions ──────────────────────────────────────────────
# Keys under [providers.*] become provider names referenced in [agent.*].
[providers.anthropic]
type = "openai_compat"
url = "https://api.anthropic.com"          # optional; inferred from type if absent
api_key_env = "ANTHROPIC_API_KEY"          # env var name (preferred over api_key)
default_model = "claude-sonnet-4-5"        # used when this is a fallback provider

[providers.openai]
type = "openai"
api_key_env = "OPENAI_API_KEY"
default_model = "gpt-4o"

[providers.groq]
type = "groq"
api_key_env = "GROQ_API_KEY"
default_model = "llama-3.3-70b-versatile"

# ── Agent definitions ─────────────────────────────────────────────────────
# "master" is the default agent name used by the HTTP handler and Telegram.
[agent.master]
provider = "anthropic"                     # primary provider key
model = "claude-sonnet-4-5"               # model for this agent
instructions = "You are a helpful AI agent..."
tools = [                                  # controls which tools are available
  "shell", "file_read", "file_write", "file_edit",
  "content_search", "glob_search", "web_search",
  "memory_store", "memory_recall",
  "git_status", "git_diff", "git_log", "git_commit",
]

# ── Tool settings ─────────────────────────────────────────────────────────
[tools]
brave_search_api_key = "BSA-..."          # if set, web_search uses Brave instead of DDG

# ── Cluster / P2P mesh ────────────────────────────────────────────────────
[cluster]
node_name = "my-macbook"
peers = ["http://100.64.0.2:7878"]
cluster_secret = "change-me-before-production"

# ── Telegram bot ──────────────────────────────────────────────────────────
[telegram]
bot_token_env = "TELEGRAM_BOT_TOKEN"      # env var holding the bot token
allowed_users = [123456789]               # Telegram user IDs; empty = allow all
agent = "master"                          # which agent handles Telegram messages
```

**Configuration controls at a glance:**

| Section | Controls |
|---|---|
| `[core]` | Bind address, port, optional hub API key |
| `[providers.*]` | LLM endpoints, API keys, fallback models |
| `[agent.*]` | Per-agent model, provider, system prompt, tool list |
| `[tools]` | Web search backend (Brave vs DuckDuckGo) |
| `[cluster]` | Peer URLs, HMAC secret, local node name |
| `[telegram]` | Bot token, user allowlist, routing agent |

---

## 6. Data Storage

All persistent state lives under `~/.phantom-mesh/`:

```
~/.phantom-mesh/
├── agents.toml                  — main configuration file
├── conversations/
│   ├── daemon.jsonl             — default HTTP chat history
│   ├── tg:123456789.jsonl       — Telegram chat (chat_id prefixed with "tg:")
│   └── rpc.jsonl                — tasks received from peer nodes
├── memory.json                  — key→value store written by memory_store tool
└── costs.json                   — cumulative token + USD cost tracking
```

### conversations/{chat_id}.jsonl

Newline-delimited JSON. Each line is a `ChatMessage`:

```json
{"role":"user","content":"what is 2+2?"}
{"role":"assistant","content":"4"}
```

Appended on every successful agent run. Loaded lazily into memory on first access
for a given `chat_id`. The cache is write-through: disk is written first, then
the in-memory cache is updated.

### memory.json

A flat JSON object. Written atomically (full file rewrite) on each `memory_store`
tool call:

```json
{
  "project_name": "phantom-mesh",
  "last_deploy": "2026-04-20"
}
```

### costs.json

A flat JSON object updated after every LLM call:

```json
{
  "total_usd": 0.0423,
  "requests": 17,
  "prompt_tokens": 42310,
  "completion_tokens": 8940
}
```

`total_usd` is rounded to 4 decimal places in API responses.
