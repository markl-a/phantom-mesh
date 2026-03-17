# Clawtex-Core Development Guide

## Project Overview
Rust-based AI agent daemon with Telegram bot interface, 25 tools, 20 hands (workflows), 10 LLM providers, 8-device cluster support, and self-evolution system.

## Quick Reference
- **Daemon**: `cargo run -- daemon` (port 7878)
- **Config**: `~/.clawtex/agents.toml`
- **Workspace**: `~/.clawtex/workspace/`
- **Hands**: `~/.clawtex/hands/<name>/hand.toml`
- **Tests**: `cargo test` (820+ tests)
- **Restart**: `taskkill //F //IM clawtex-core.exe && cargo run --release -- --host 0.0.0.0 daemon`
- **Source**: 102 .rs files, ~44,200 LOC

## Architecture
```
Telegram Bot API → clawtex-core (Rust) → Ollama/Anthropic/OpenAI/Gemini/Groq → models
                        ↓
              src/providers/  (18 files, 10 providers)
              src/tools/      (26 files, 25 tools)
              src/hands/      (20 workflow definitions)
              src/cluster_hub.rs + cluster_worker.rs
```

## Key Source Files
| Area | File | Purpose |
|------|------|---------|
| Entry | `src/main.rs` | CLI + daemon startup |
| Agent | `src/agent_runtime.rs` | Multi-round tool-calling loop |
| Dispatch | `src/dispatcher.rs` | Native/XML/function-tag tool call parsing |
| Tools | `src/tools/mod.rs` | Tool trait, SecurityConfig, path normalization |
| Hands | `src/hands/mod.rs` | Multi-phase workflow execution |
| Providers | `src/providers/router.rs` | Smart routing + classifier |
| Telegram | `src/telegram.rs` | Bot interface |
| Cluster | `src/cluster_hub.rs` | Hub dispatch to workers |
| Context | `src/context.rs` | Context compaction |
| Approval | `src/approval.rs` | Human-in-the-loop gate |
| Self-Evolve | `src/trajectory.rs` | Trajectory logging + prompt evolution |
| Circuit Breaker | `src/circuit_breaker.rs` | Provider reliability (3-state) |
| Guardrail | `src/guardrail.rs` | L1 content safety + L2 LLM-as-Judge |
| Security | `src/security/mod.rs` | Encryption, RBAC, privacy guard |

## Reference Project Index

When implementing new features, check the reference analysis docs for proven patterns:

**Full index**: [`docs/references/INDEX.md`](docs/references/INDEX.md)

### Quick Lookup by Feature Area

| Building... | Look at | Why |
|------------|---------|-----|
| Agent loop improvements | [IronClaw](docs/references/ironclaw-analysis.md), [Codex CLI](docs/references/codex-cli-analysis.md) | LoopDelegate trait, SQ/EQ pattern |
| Provider system | [IronClaw](docs/references/ironclaw-analysis.md), [AutoAgents](docs/references/autoagents-analysis.md) | UnsupportedParam filtering, LLMLayer pipeline |
| Tool system | [Rig](docs/references/rig-analysis.md), [Gemini CLI](docs/references/gemini-cli-analysis.md), [VCPToolBox](docs/references/vcptoolbox-analysis.md) | ToolServer actor, Builder+Invocation, Dynamic Fold |
| Memory/Context | [Mastra](docs/references/mastra-analysis.md), [OpenHands](docs/references/openhands-analysis.md), [VCPToolBox](docs/references/vcptoolbox-analysis.md) | Observational Memory, Condenser Pipeline, LIF Spike Propagation |
| Hands/Workflow | [LangGraph](docs/references/langgraph-analysis.md), [Swarm](docs/references/swarm-analysis.md), [CrewAI](docs/references/crewai-analysis.md) | Graph workflow, DAG engine, role-based agents |
| Multi-agent | [OWL](docs/references/owl-analysis.md), [AutoAgents](docs/references/autoagents-analysis.md) | Semantic dispatch, Actor model |
| Security | [ZeroClaw](docs/references/zeroclaw-analysis.md), [Codex CLI](docs/references/codex-cli-analysis.md) | 12-module security, 3-platform sandbox |
| Streaming | [OpenFang](docs/references/openfang-analysis.md), [AutoAgents](docs/references/autoagents-analysis.md) | ThinkFilter, 3-layer streaming |
| Telegram | [Teloxide](docs/references/teloxide-analysis.md), [OpenFang](docs/references/openfang-analysis.md) | Dialogue state machine, file upload |
| MCP | [All Agents MCP](docs/references/all-agents-mcp-analysis.md), [Goose](docs/references/goose-analysis.md) | MCP server impl, anti-recursion |
| Serialization | [Letta](docs/references/letta-agent-file-analysis.md) | .af format for portable agents |
| Code editing | [Aider](docs/references/aider-analysis.md), [OpenCode](docs/references/opencode-analysis.md) | Repo Map, LSP feedback, edit formats |
| CLI orchestration | [CLI Agent Orchestrator](docs/references/cli-agent-orchestrator-analysis.md), [Claude Code Bridge](docs/references/claude-code-bridge-analysis.md) | External agent management |

### 27 Reference Projects (sorted by relevance)

**Rust** (highest relevance):
1. [ZeroClaw](docs/references/zeroclaw-analysis.md) — Primary inspiration, 35+ tools, 12-module security
2. [IronClaw](docs/references/ironclaw-analysis.md) — Unified agentic loop, provider decorator chain
3. [AutoAgents](docs/references/autoagents-analysis.md) — Actor model, LLMLayer, `#[tool]` macro
4. [Goose](docs/references/goose-analysis.md) — MCP-first, Recipe workflow, Inspector pipeline
5. [OpenFang](docs/references/openfang-analysis.md) — ThinkFilter, taint tracking, 14-crate workspace
6. [Anda](docs/references/anda-analysis.md) — Dual-trait pattern, CompletionRunner, feature traits
7. [Rig](docs/references/rig-analysis.md) — Typestate builder, ToolServer actor, Agent-as-Tool
8. [tsk](docs/references/tsk-analysis.md) — WorkerPool, config snapshot, DAG task chaining
9. [OpenCrust](docs/references/opencrust-analysis.md) — Config hot-reload, AES-256 vault, channel split
10. [Swarm](docs/references/swarm-analysis.md) — DAG workflow, 3-tier invoker, LLM-as-Judge
11. [Teloxide](docs/references/teloxide-analysis.md) — Dialogue state machine, DI handlers, Throttle
12. [Codex CLI](docs/references/codex-cli-analysis.md) — SQ/EQ, 3-platform sandbox, ApprovalStore

**Coding Agents**:
13. [OpenClaw](docs/references/openclaw-analysis.md) — Pluggable ContextEngine, tool policy, skills system
14. [OpenCode](docs/references/opencode-analysis.md) — LSP feedback, persistent shell, auto-compact
15. [Gemini CLI](docs/references/gemini-cli-analysis.md) — CoreToolScheduler, hooks lifecycle, 1M context
16. [Aider](docs/references/aider-analysis.md) — Repo Map, SEARCH/REPLACE, 3-model architecture
17. [Cline](docs/references/cline-analysis.md) — HostProvider DI, multi-tier approval, Focus Chain

**Agent Frameworks**:
18. [Mastra](docs/references/mastra-analysis.md) — Observational Memory (3-40x compression)
19. [OpenHands](docs/references/openhands-analysis.md) — Condenser Pipeline, Action/Observation, microagents
20. [CrewAI](docs/references/crewai-analysis.md) — Role-based agents, hierarchical process, guardrails
21. [LangGraph](docs/references/langgraph-analysis.md) — Graph workflow, Channel/Reducer, checkpoint
22. [OWL](docs/references/owl-analysis.md) — Multi-agent cooperation, semantic dispatch
23. [Letta Agent File](docs/references/letta-agent-file-analysis.md) — .af portable agent format

**Utilities**:
24. [CLI Agent Orchestrator](docs/references/cli-agent-orchestrator-analysis.md) — External CLI agent management
25. [Claude Code Bridge](docs/references/claude-code-bridge-analysis.md) — Multi-instance, pane self-healing
26. [Claude Octopus](docs/references/claude-octopus-analysis.md) — Validation gates, consensus, personas
27. [All Agents MCP](docs/references/all-agents-mcp-analysis.md) — MCP server implementation patterns
28. [VCPToolBox](docs/references/vcptoolbox-analysis.md) — LIF memory propagation, Dynamic Fold tool injection, model-specific prompts
