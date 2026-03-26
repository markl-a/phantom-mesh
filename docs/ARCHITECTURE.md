# Phantom-Mesh Architecture

> Last updated: 2026-03-16
> Source: 233 .rs files, ~142,000 LOC

## System Overview

```
┌─────────────────────────────────────────────────────────────┐
│                      User Interfaces                        │
│  Telegram Bot ──── HTTP API (:7878) ──── CLI (interactive)  │
└─────────────┬──────────────┬──────────────┬─────────────────┘
              │              │              │
              ▼              ▼              ▼
┌─────────────────────────────────────────────────────────────┐
│                     Agent Runtime                           │
│  agent_runtime.rs — multi-round tool-calling agentic loop   │
│  ├─ dispatcher.rs   (Native/XML/function-tag parsing)       │
│  ├─ loop_detection.rs (taint tracking, break infinite loops)│
│  ├─ context.rs       (context window management)            │
│  ├─ context_compactor.rs (compaction strategies)            │
│  └─ think_filter.rs  (streaming <thinking> block filter)    │
└────────┬───────────────────┬────────────────────────────────┘
         │                   │
    ┌────▼────┐         ┌────▼────┐
    │ Tools   │         │Providers│
    │ (25)    │         │ (10)    │
    └────┬────┘         └────┬────┘
         │                   │
         ▼                   ▼
┌─────────────────┐  ┌──────────────────────────┐
│ Hands Engine    │  │ Provider Routing          │
│ (20 workflows)  │  │ router.rs → rotation.rs   │
│ hands/mod.rs    │  │ → classifier.rs           │
└─────────────────┘  │ → circuit_breaker.rs      │
                     └──────────────────────────┘
         │
         ▼
┌─────────────────────────────────────────────────────────────┐
│                    Cluster Layer                             │
│  cluster_hub.rs (Hub @ Z13:7878)                            │
│  cluster_worker.rs (Workers @ M1:7879, Acer:7881, etc.)     │
│  ├─ ToolRouting: Local / AnyWorker / FullWorkerOnly / Mobile│
│  ├─ Load balancing: effective_load = cpu + (inflight * 0.15)│
│  └─ Shared mobile pool: 4 phones compete for task queue     │
└─────────────────────────────────────────────────────────────┘
```

## Directory Structure

```
phantom-mesh/
├── src/
│   ├── main.rs              # CLI entry, daemon startup, HTTP server
│   ├── lib.rs               # Public API (47 module exports)
│   ├── agent_runtime.rs     # Core agentic loop
│   ├── dispatcher.rs        # Tool call parsing (3 modes)
│   │
│   ├── tools/               # 26 files, 25 tools
│   │   ├── mod.rs           # Tool trait, registry, rate limiter
│   │   ├── shell.rs         # Shell command execution
│   │   ├── file_read.rs     # File operations
│   │   ├── web_search.rs    # Multi-backend search
│   │   ├── ai_code.rs       # Claude/Gemini/Codex code gen
│   │   ├── computer_use.rs  # Desktop automation
│   │   ├── cli_anything.rs  # CLI-Anything integration
│   │   └── ...              # 19 more tools
│   │
│   ├── providers/           # 18 files, 10 providers
│   │   ├── mod.rs           # Provider exports
│   │   ├── traits.rs        # Provider trait, ChatMessage, ToolCall
│   │   ├── router.rs        # Smart routing + load balancing
│   │   ├── rotation.rs      # Rate limit cooldown rotation
│   │   ├── classifier.rs    # Error classification
│   │   ├── ollama.rs        # Local Ollama
│   │   ├── gemini.rs        # Google Gemini (native tool calling)
│   │   ├── chatgpt_backend.rs # ChatGPT via Codex CLI
│   │   └── ...              # 9 more providers
│   │
│   ├── hands/               # Workflow engine
│   │   └── mod.rs           # Hand, Phase, HandRunner, approval gates
│   │
│   ├── security/            # 5 files
│   │   ├── mod.rs           # SecretManager, AutonomyLevel, Role
│   │   ├── secrets.rs       # ChaCha20-Poly1305 encryption
│   │   ├── autonomy.rs      # Autonomy enforcement
│   │   ├── roles.rs         # RBAC registry
│   │   └── privacy.rs       # PII detection, privacy tiers
│   │
│   ├── memory/              # 3 files
│   │   ├── mod.rs           # MemoryStore trait
│   │   ├── sqlite.rs        # SQLite backend
│   │   └── pgvector.rs      # PostgreSQL + pgvector (optional)
│   │
│   ├── hooks/               # 5 files — pre/post tool lifecycle
│   ├── sandbox/             # 3 files — Docker + WASM sandbox
│   ├── mcp/                 # 1 file — JSON-RPC 2.0 MCP client
│   ├── plugins/             # 1 file — dynamic plugin loading
│   │
│   ├── cluster_hub.rs       # Hub: dispatch, polling, load balance
│   ├── cluster_worker.rs    # Worker: registration, task execution
│   ├── cluster.rs           # ClusterRegistry, ClusterNode
│   ├── telegram.rs          # Telegram Bot API client
│   ├── cron.rs              # Scheduler, hand scheduling
│   ├── approval.rs          # Async Telegram approval gate
│   ├── cost_tracker.rs      # Token counting, cost estimation
│   ├── revenue_tracker.rs   # Revenue accounting
│   ├── revenue_engine.rs    # ROI analysis, budget optimization
│   ├── trajectory.rs        # Self-evolution trajectory logging
│   ├── circuit_breaker.rs   # Provider circuit breaker (3-state)
│   ├── watchdog.rs          # Worker SSH auto-recovery
│   ├── prompt_optimizer.rs  # DSPy-style prompt evolution
│   ├── guardrail.rs         # L1 content safety checks
│   ├── evaluate.rs          # L2 LLM-as-Judge quality eval
│   ├── error_codes.rs       # Standardized error codes
│   ├── think_filter.rs      # Streaming <thinking> filter
│   ├── loop_detection.rs    # Infinite loop detector
│   ├── response_cache.rs    # Response dedup cache
│   ├── context_compactor.rs # Context compaction strategies
│   └── ...                  # 10 more supporting modules
│
├── docs/
│   ├── ARCHITECTURE.md      # This file
│   ├── plans/               # Active planning docs
│   │   ├── roadmap-2026-sprint.md
│   │   ├── parallel-dev-strategy.md
│   │   ├── pricing-strategy-2026.md
│   │   ├── tech-backlog.md
│   │   └── archive/         # 16 superseded design docs
│   ├── references/          # 28 analysis files + INDEX.md
│   └── *.md                 # Revenue strategy, execution plans
│
├── deploy/                  # Cluster deployment scripts & packages
├── tests/integration/       # Python integration tests
├── Cargo.toml               # Rust dependencies
├── Dockerfile               # Container image
├── docker-compose.yml       # Multi-service compose
└── CLAUDE.md                # AI assistant development guide
```

## Core Subsystems

### 1. Agent Runtime (`agent_runtime.rs`)

Multi-round tool-calling loop. Each turn:
1. Send messages + tool schemas to LLM provider
2. Parse response for tool calls (via `dispatcher.rs`)
3. Execute tools, collect results
4. Feed results back as next turn
5. Loop until LLM returns final text (no tool calls)

Key features:
- Automatic trajectory logging for self-evolution
- Circuit breaker integration for provider reliability
- Context window management with auto-compaction
- Cost tracking per interaction

### 2. Tool System (`src/tools/`)

25 tools implementing the `Tool` trait:

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;
    async fn execute(&self, args: Value) -> Result<ToolResult>;
}
```

**Categories:**
| Category | Tools |
|----------|-------|
| File I/O | file_read, file_write, file_edit, glob_search, content_search |
| Shell | shell |
| Web | web_search, http_request, browser |
| AI | ai_code, computer_use, vision, cli_anything |
| Memory | memory_store, memory_recall, memory_forget |
| Delegation | delegate, delegate_to_provider, run_hand |
| Publishing | twitter, blog_publish, email, pdf_export |
| SaaS | stripe, render_deploy, scaffold_saas |
| Parallel | skeleton_generate |

Rate limiting: `ActionTracker` enforces max_actions_per_hour (600) and max_per_tool_per_hour (200).

### 3. Provider System (`src/providers/`)

10 providers with smart routing:

| Provider | Type | Model | Use Case |
|----------|------|-------|----------|
| ollama | Local CPU | llama3.2:1b | Classifier, simple tasks |
| lmstudio | Local GPU | qwen3-coder-next | Medium tasks, tool calling |
| npu | AMD XDNA | Mistral-7B | NPU acceleration |
| gemini | Cloud | gemini-2.5-flash | Primary free tier, native tools |
| groq | Cloud | llama-3.3-70b | Ultra-fast inference |
| openrouter | Cloud | Various free | Fallback, text-based tools |
| cerebras | Cloud | llama-3.3-70b | High TPM tasks |
| codex | CLI | gpt-4o | Code generation |
| chatgpt | CLI | gpt-5.4 | Master agent, complex tasks |
| opencode | CLI | minimax-m2.5 | Free models via OpenCode |

**Routing flow:**
```
Request → Classifier (ollama:1b) → simple/medium/complex
  simple  → [ollama, lmstudio]
  medium  → [groq, gemini, openrouter, chatgpt]
  complex → [gemini, groq, openrouter, chatgpt]
```

**Reliability stack:**
- `ProviderRotation`: Rate-limit cooldown with 15-120s backoff
- `ProviderCircuitBreaker`: 3-state (closed/open/half-open) per provider
- `ErrorClassifier`: Maps errors to RateLimited/Timeout/Quota/Context/etc.

### 4. Hands Workflow Engine (`src/hands/`)

20 multi-phase workflows defined in TOML:

```toml
[hand]
name = "content"
description = "Generate marketing content"
provider = "gemini"
model = "gemini-2.5-flash"
output_format = "md"
tools = ["web_search", "file_write", "twitter", "blog_publish"]

[[phases]]
name = "topic_research"
system_prompt = "Research trending topics..."
tools = ["web_search"]
```

**Execution:** HandRunner iterates phases sequentially, passing output as input to next phase. Supports:
- Approval gates (Telegram human-in-the-loop)
- Condition gates (skip phases based on criteria)
- L1/L2 quality checks between phases
- Auto-save to `~/.phantom-mesh/workspace/{hand}_{datetime}.{ext}`
- Chain-to (trigger next hand on completion)

### 5. Cluster Architecture

**8-device cluster:**

| Node | Role | Port | Connection |
|------|------|------|------------|
| Z13 | Hub + Main LLM | 7878 | localhost |
| M1 Mac | Full Worker | 7879 | Tailscale |
| AYANEO | NPU Worker | 7880 | LAN |
| Acer | Light Worker | 7881 | LAN |
| ROG6 | Mobile Worker | — | Polling |
| MiPad | Mobile Worker | — | Polling |
| iPhone | Mobile Worker | — | Polling |
| iPad | Mobile Worker | — | Polling |

**Tool Routing:**
- `LocalOnly`: file_*, memory_*, glob/content_search → Z13 only
- `AnyWorker`: web_search, http_request, email → prefer lightest worker
- `FullWorkerOnly`: shell, ai_code, browser, skeleton → Z13/M1
- `MobileOnly`: sensor_gps, camera, local_llm → phone workers

**Load Balancing:**
```
effective_load = cpu_load + (inflight_tasks * 0.15)
→ route to worker with lowest effective_load
```

### 6. Self-Evolution System

Nightly compound improvement loop:

```
1:00 AM  review_agents  → Analyze hand execution, find weakest performer
2:00 AM  self_evolve    → Review conversations, implement 1 improvement
3:00 AM  cluster_evolve → Distributed improvements across all workers
Sun 4AM  prompt_evolve  → DSPy-style prompt optimization with A/B variants
```

Supporting infrastructure:
- `trajectory.rs`: Log every agent interaction quality score
- `circuit_breaker.rs`: Auto-disable failing providers
- `watchdog.rs`: SSH auto-recovery for crashed workers
- `prompt_optimizer.rs`: Generate/test/deploy improved prompts

### 7. Security Model

**Layers:**
1. **Encryption**: ChaCha20-Poly1305 for all secrets (`enc2:` prefix in config)
2. **Autonomy Levels**: Disabled → Limited → Full → Escalated
3. **RBAC**: Role-based access control per agent
4. **Privacy Guard**: Route by data sensitivity (critical→local, public→cloud)
5. **Rate Limiter**: Per-tool and global action limits
6. **Approval Gate**: Telegram human-in-the-loop for sensitive operations
7. **Credential Scrubbing**: Auto-redact API keys from tool output

## Database Layout

| Database | Path | Tables | Purpose |
|----------|------|--------|---------|
| core.db | `~/.phantom-mesh/core.db` | sessions, cron_jobs | Session state, scheduling |
| costs.db | `~/.phantom-mesh/costs.db` | cost_records | Token/cost tracking |
| memory.db | `~/.phantom-mesh/memory.db` | memories | Semantic memory store |
| revenue.db | `~/.phantom-mesh/revenue.db` | revenue entries | Revenue accounting |
| trajectories.db | `~/.phantom-mesh/trajectories.db` | trajectory logs | Self-evolution feedback |

## Cron Schedule (11 jobs)

| Time (UTC+8) | Hand | Frequency |
|--------------|------|-----------|
| 01:00 | review_agents | Daily |
| 02:00 | self_evolve | Daily |
| 03:00 | cluster_evolve | Daily |
| 08:00 | content | Daily |
| 09:00 | freelancer | Daily |
| 10:00 | lead | Monday |
| 11:00 | seo_content | Tue/Thu |
| 14:00 | researcher | Daily |
| 09:00 | market_intel | Wednesday |
| 15:00 | outreach | Mon/Wed/Fri |
| 04:00 | prompt_evolve | Sunday |
