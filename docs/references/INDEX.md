# Reference Project Index — Feature → Implementation Guide

> 開發 clawtex-core 時，如果要實作某個功能，查這個表找到對應的參考專案和分析文件。
> 每份分析都在 `docs/references/<name>-analysis.md`。

---

## Agent Runtime / Core Loop

| 功能 | 參考專案 | 分析文件 | 重點 |
|------|---------|---------|------|
| **Unified Agentic Loop** (多消費者共用) | IronClaw | [ironclaw](ironclaw-analysis.md) | `LoopDelegate` trait，3 個消費者共用同一 loop |
| **CompletionRunner Iterator** (agent 控制 loop) | Anda | [anda](anda-analysis.md) | 迭代器模式，agent 控制何時繼續 |
| **SQ/EQ Event Queue** | Codex CLI | [codex-cli](codex-cli-analysis.md) | Submission Queue / Event Queue 解耦 |
| **Agent State Machine** (5 states) | Swarm | [swarm](swarm-analysis.md) | Thinking/Executing/Evaluating/Correcting/Finished |
| **Stuck Detection** (5 scenarios) | OpenHands | [openhands](openhands-analysis.md) | 5 種迴圈偵測 vs clawtex 的 3 種 |
| **Loop Detection** (LLM-based) | Gemini CLI | [gemini-cli](gemini-cli-analysis.md) | 用 LLM 偵測 agent 卡住 |
| **Reflection Loop** (error retry) | Aider | [aider](aider-analysis.md) | 失敗自動重試帶 error feedback（最多 3 次） |

## Provider / LLM Integration

| 功能 | 參考專案 | 分析文件 | 重點 |
|------|---------|---------|------|
| **Per-provider Parameter Filtering** | IronClaw | [ironclaw](ironclaw-analysis.md) | `UnsupportedParam` typed enum，集中化過濾 |
| **LLMLayer Pipeline** (middleware chain) | AutoAgents | [autoagents](autoagents-analysis.md) | cache → retry → fallback → guardrails 可組合 |
| **Smart Routing** (13-dim scorer) | IronClaw | [ironclaw](ironclaw-analysis.md) | 純計算 complexity scorer，不需 LLM 呼叫 |
| **75+ Provider Support** (OpenAI-compat reuse) | OpenCode | [opencode](opencode-analysis.md) | 實際只有 7-8 個獨立實作，其餘改 base URL |
| **4-method ApiHandler** | Cline | [cline](cline-analysis.md) | createMessage/getModel/getApiStreamUsage/abort |
| **Provider Decorator Chain** | IronClaw | [ironclaw](ironclaw-analysis.md) | Raw→Retry→SmartRouting→Failover→CircuitBreaker→Cache→Recording |
| **Auth Profile Rotation** | OpenClaw | [openclaw](openclaw-analysis.md) | multi-key round-robin + cooldown + failover |
| **Three-Model Architecture** | Aider | [aider](aider-analysis.md) | Main（推理）+ Weak（commit msg）+ Editor（編輯）|
| **Model Metadata System** | Aider | [aider](aider-analysis.md) | YAML-driven model settings + 3-tier caching |
| **Token Budget Cap** | IronClaw | [ironclaw](ironclaw-analysis.md) | 原子化 max_tokens 上限防止成本失控 |

## Tool System

| 功能 | 參考專案 | 分析文件 | 重點 |
|------|---------|---------|------|
| **Builder+Invocation Pattern** (驗證/執行分離) | Gemini CLI | [gemini-cli](gemini-cli-analysis.md) | Tool 定義拆成 builder + invoker |
| **`#[tool]` Proc Macro** | AutoAgents | [autoagents](autoagents-analysis.md) | derive macro 宣告式定義 tool |
| **ToolServer Actor** (background + hot-reload) | Rig | [rig](rig-analysis.md) | tool 跑在 background actor，支援 hot-reload |
| **Agent-as-Tool** | Rig | [rig](rig-analysis.md) | `Agent<M>` 實作 `Tool` trait |
| **Tool RAG** (vector search 選工具) | Rig | [rig](rig-analysis.md) | 太多 tool 時用 vector store 動態選擇 |
| **Dynamic Fold** (上下文自適應工具注入) | VCPToolBox | [vcptoolbox](vcptoolbox-analysis.md) | cosine similarity 過濾，只注入相關 tool 描述省 token |
| **Tool Isolation per Agent** | OWL | [owl](owl-analysis.md) | 不同角色給不同 tool whitelist |
| **Tool Policy Pipeline** (多層過濾) | OpenClaw | [openclaw](openclaw-analysis.md) | owner-only → provider → allow/deny → sandbox → depth |
| **ToolStream** (Future + notification Stream) | Goose | [goose](goose-analysis.md) | tool 邊執行邊推送進度 |
| **Taint Tracking** | OpenFang | [openfang](openfang-analysis.md) | TaintLabel + TaintSink 追蹤資料污染 |
| **Sensitive Param Redaction** | IronClaw | [ironclaw](ironclaw-analysis.md) | `sensitive_params()` 宣告式遮蔽 |
| **Orphaned tool_result Repair** | IronClaw | [ironclaw](ironclaw-analysis.md) | `sanitize_tool_messages()` 修復孤立 tool result |
| **Zod Schema-driven Tools** | All Agents MCP | [all-agents-mcp](all-agents-mcp-analysis.md) | type safety + validation + JSON Schema generation |

## Memory / Context Management

| 功能 | 參考專案 | 分析文件 | 重點 |
|------|---------|---------|------|
| **Observational Memory** (3-40x compression) | Mastra | [mastra](mastra-analysis.md) | Observer + Reflector 雙 agent，LongMemEval 95% |
| **Condenser Pipeline** (10 strategies) | OpenHands | [openhands](openhands-analysis.md) | NoOp/RecentEvents/LLMSummarizing/AmortizedForgetting 等可組合 |
| **Pluggable ContextEngine** | OpenClaw | [openclaw](openclaw-analysis.md) | ingest/assemble/compact/afterTurn 生命週期 |
| **Vector Memory Search** (SQLite-vec + MMR) | OpenClaw | [openclaw](openclaw-analysis.md) | semantic search + temporal decay + MMR deduplication |
| **LIF Spike Propagation** (神經激發記憶傳播) | VCPToolBox | [vcptoolbox](vcptoolbox-analysis.md) | co-occurrence graph + leaky integrate-and-fire，找拓撲相關記憶 |
| **EPA Semantic Axis Projection** | VCPToolBox | [vcptoolbox](vcptoolbox-analysis.md) | PCA 投影判斷 query 屬於哪個概念世界 |
| **Residual Pyramid** (殘差能量分析) | VCPToolBox | [vcptoolbox](vcptoolbox-analysis.md) | Gram-Schmidt 正交分解，偵測新概念 vs 已知概念 |
| **Agent Dream** (離線記憶整理) | VCPToolBox | [vcptoolbox](vcptoolbox-analysis.md) | 空閒時合併/清理/反思記憶，需管理員批准 |
| **Unified Memory** (background write + consolidation) | CrewAI | [crewai](crewai-analysis.md) | shallow vector search + deep RecallFlow |
| **Memory-First Three-Tier** (L1/L2/L3) | Claude Code Bridge | [claude-code-bridge](claude-code-bridge-analysis.md) | 三層記憶架構 |
| **Message Visibility** (agent vs user) | Goose | [goose](goose-analysis.md) | agent_visible / user_visible 分離 |
| **Feature Trait Separation** (context splitting) | Anda | [anda](anda-analysis.md) | State/Keys/Store/Cache/HTTP 分離 |
| **Repo Map** (tree-sitter + PageRank) | Aider | [aider](aider-analysis.md) | code graph → PageRank 排序 → token budget 控制 |
| **Auto-Compact Trigger** (95% threshold) | OpenCode | [opencode](opencode-analysis.md) | context window 95% 時自動壓縮 |
| **Multi-Tag Memory + Rollback** | Agency Agents | [agency-agents](agency-agents-analysis.md) | remember/recall/rollback/search，多 tag 跨 agent 記憶共享 |

## Workflow / Orchestration

| 功能 | 參考專案 | 分析文件 | 重點 |
|------|---------|---------|------|
| **DAG Workflow Engine** | Swarm | [swarm](swarm-analysis.md) | 拓撲排序 + `{{output}}` 參數插值 |
| **Graph-Based Workflow** (StateGraph) | LangGraph | [langgraph](langgraph-analysis.md) | Channel/Reducer pattern，conditional edges |
| **Recipe YAML** (composable) | Goose | [goose](goose-analysis.md) | sub-recipe 巢套 + MiniJinja template |
| **Role-Based Agent** (role/goal/backstory) | CrewAI | [crewai](crewai-analysis.md) | 結構化角色定義 |
| **Hierarchical Process** | CrewAI | [crewai](crewai-analysis.md) | manager agent 動態分配任務 |
| **Checkpoint/Resume** | LangGraph | [langgraph](langgraph-analysis.md) | graph state 持久化與恢復 |
| **Human-in-the-Loop** (`interrupt/resume`) | LangGraph | [langgraph](langgraph-analysis.md) | 結構化的中斷/恢復協議 |
| **ConditionalTask** | CrewAI | [crewai](crewai-analysis.md) | 有條件的任務分支 |
| **Validation Gate** (防 LLM 偷懶) | Claude Octopus | [claude-octopus](claude-octopus-analysis.md) | 強制執行合約，防止跳過步驟 |
| **Config Snapshot** (execution-time freeze) | tsk | [tsk](tsk-analysis.md) | 執行時快照 config |
| **Trigger System** (take/restore) | OpenFang | [openfang](openfang-analysis.md) | hand 重啟時觸發器重新綁定 |
| **Structured Agent Persona** (8-section definition) | Agency Agents | [agency-agents](agency-agents-analysis.md) | YAML frontmatter + Identity/Mission/Rules/Metrics/Workflow |
| **NEXUS 7-Phase Pipeline** (multi-agent orchestration) | Agency Agents | [agency-agents](agency-agents-analysis.md) | Discovery→Strategy→Foundation→Build→Hardening→Launch→Operate |
| **Evidence-Based Quality Gate** (3-layer escalation) | Agency Agents | [agency-agents](agency-agents-analysis.md) | Evidence Collector → Reality Checker → Escalation Report |

## Multi-Agent / Delegation

| 功能 | 參考專案 | 分析文件 | 重點 |
|------|---------|---------|------|
| **Subagent Depth Control** | OpenClaw | [openclaw](openclaw-analysis.md) | 越深層 tool 越受限 |
| **Iterative Delegate** (multi-round) | OWL | [owl](owl-analysis.md) | 從單次改為多輪對話式委派 |
| **Semantic Dispatch** | OWL | [owl](owl-analysis.md) | 用 agent description 自動路由 |
| **handoff vs assign** (sync/async) | CLI Agent Orchestrator | [cli-agent-orchestrator](cli-agent-orchestrator-analysis.md) | 同步阻塞 vs 非同步非阻塞委派 |
| **Three-tier Invoker** | Swarm | [swarm](swarm-analysis.md) | Agent/Tool/Task invoker 解耦 |
| **Actor Model Multi-Agent** | AutoAgents | [autoagents](autoagents-analysis.md) | Ractor actor，typed messaging，Topic pub/sub |
| **Multi-Instance Provider** | Claude Code Bridge | [claude-code-bridge](claude-code-bridge-analysis.md) | `codex:auth` 格式多實例隔離 |
| **WorkerPool** (semaphore concurrency) | tsk | [tsk](tsk-analysis.md) | Semaphore-based 並行控制 |
| **Completion Timeout No-Retry** | OpenClaw | [openclaw](openclaw-analysis.md) | 完成通知 timeout 不重試 |
| **Consensus Gate** (75%) | Claude Octopus | [claude-octopus](claude-octopus-analysis.md) | 多 provider 共識達標才出貨 |
| **Parallel-then-Synthesize** (multi-agent coordination) | Agency Agents | [agency-agents](agency-agents-analysis.md) | 多 agent 同一 brief → 並行執行 → 共識偵測 + 衝突表 |
| **Structured Handoff Protocol** (3-layer context) | Agency Agents | [agency-agents](agency-agents-analysis.md) | Metadata + Context + Deliverable 三層交接模板 |

## Security

| 功能 | 參考專案 | 分析文件 | 重點 |
|------|---------|---------|------|
| **Three-Platform Sandbox** | Codex CLI | [codex-cli](codex-cli-analysis.md) | Linux Landlock+seccomp, macOS seatbelt, Windows restricted tokens |
| **12-Module Security Subsystem** | ZeroClaw | [zeroclaw](zeroclaw-analysis.md) | LeakDetector, PromptGuard, E-Stop, 4 sandbox backends |
| **Inspector Pipeline** (3-layer) | Goose | [goose](goose-analysis.md) | Security → Permission → Repetition 檢查鏈 |
| **URL Stripping** (entropy false-positive) | ZeroClaw | [zeroclaw](zeroclaw-analysis.md) | credential detector 排除 URL 路徑 |
| **Subprocess Env Var Stripping** | OpenFang | [openfang](openfang-analysis.md) | 清除其他 provider API key |
| **Shell Bleed Detection** | OpenFang | [openfang](openfang-analysis.md) | 偵測 script 環境變數洩漏 |
| **Anti-Injection Nonces** | Claude Octopus | [claude-octopus](claude-octopus-analysis.md) | 防注入 nonce 機制 |
| **AES-256-GCM Vault + OS Keychain** | OpenCrust | [opencrust](opencrust-analysis.md) | 比 ChaCha20 更完整的加密方案 |
| **Prompt Injection Defense** (4-stage) | IronClaw | [ironclaw](ironclaw-analysis.md) | 4 階段注入防禦管線 |
| **Multi-tier Approval** | Cline | [cline](cline-analysis.md) | YOLO → Auto → per-tool → path-aware |
| **ApprovalStore** (cache decisions) | Codex CLI | [codex-cli](codex-cli-analysis.md) | approval 結果快取 |

## Streaming / Events

| 功能 | 參考專案 | 分析文件 | 重點 |
|------|---------|---------|------|
| **StreamingThinkFilter** (`<think>` tag) | OpenFang | [openfang](openfang-analysis.md) | 狀態機過濾 `<think>` 標籤，含 partial match |
| **Internal→External Event Mapping** | Codex CLI | [codex-cli](codex-cli-analysis.md) | 50+ internal → 8 stable external events |
| **7 Stream Modes** | LangGraph | [langgraph](langgraph-analysis.md) | values/updates/messages/custom/checkpoints/tasks/debug |
| **Draft Updates** (typing indicator) | ZeroClaw | [zeroclaw](zeroclaw-analysis.md) | channel 支援即時更新中間狀態 |
| **Three-Layer Streaming** | AutoAgents | [autoagents](autoagents-analysis.md) | LLM StreamChunk → TurnDelta → Agent Stream |

## Channel / Communication

| 功能 | 參考專案 | 分析文件 | 重點 |
|------|---------|---------|------|
| **Channel Abstraction** (20+ channels) | ZeroClaw | [zeroclaw](zeroclaw-analysis.md) | `Channel` trait 支援 draft/reaction/thread/typing |
| **HostProvider DI** (platform abstraction) | Cline | [cline](cline-analysis.md) | 零平台依賴的核心邏輯 |
| **Telegram File Upload** (sendDocument) | OpenFang | [openfang](openfang-analysis.md) | multipart upload for PDF/files |
| **Telegram Throttle** (rate limiting) | Teloxide | [teloxide](teloxide-analysis.md) | 防 API 封鎖的 rate limiter |
| **Dialogue State Machine** | Teloxide | [teloxide](teloxide-analysis.md) | enum-based 多步驟對話狀態機 |
| **Relay Channel** (SSE + backoff) | IronClaw | [ironclaw](ironclaw-analysis.md) | exponential backoff reconnection |
| **Config Hot-Reload** | OpenCrust | [opencrust](opencrust-analysis.md) | `notify` + `tokio::sync::watch` |

## Code Editing / AI Coding

| 功能 | 參考專案 | 分析文件 | 重點 |
|------|---------|---------|------|
| **SEARCH/REPLACE Edit Format** | Aider | [aider](aider-analysis.md) | 多層容錯的搜尋替換 |
| **LSP Diagnostic Feedback** | OpenCode | [opencode](opencode-analysis.md) | 編輯後即時看到編譯/型別錯誤 |
| **Persistent Shell Session** | OpenCode | [opencode](opencode-analysis.md) | 跨命令保持環境狀態 |
| **File Version History** (per session) | OpenCode | [opencode](opencode-analysis.md) | 支援 undo |
| **Guardrail Validation** | CrewAI | [crewai](crewai-analysis.md) | function-based + LLM validation chains |
| **LLM-as-Judge Retry** | Swarm | [swarm](swarm-analysis.md) | score < 3 自動重試（最多 3 次）|

## Serialization / Portability

| 功能 | 參考專案 | 分析文件 | 重點 |
|------|---------|---------|------|
| **Agent File (.af) Format** | Letta | [letta-agent-file](letta-agent-file-analysis.md) | JSON 格式打包 agent + memory + tools |
| **Structured Memory Blocks** | Letta | [letta-agent-file](letta-agent-file-analysis.md) | personality/user info 作為可編輯 blocks |
| **tool_rules Workflow Engine** | Letta | [letta-agent-file](letta-agent-file-analysis.md) | run_first/constrain/conditional/exit_loop |
| **Persona Auto-Routing** | Claude Octopus | [claude-octopus](claude-octopus-analysis.md) | 32 personas with frontmatter routing |
| **Cross-Tool Agent Format** (10 platform converter) | Agency Agents | [agency-agents](agency-agents-analysis.md) | 一份 MD → Cursor/Aider/Windsurf/Gemini/OpenClaw 等 10 種格式 |

## MCP (Model Context Protocol)

| 功能 | 參考專案 | 分析文件 | 重點 |
|------|---------|---------|------|
| **MCP Server Implementation** | All Agents MCP | [all-agents-mcp](all-agents-mcp-analysis.md) | one-file-per-tool 註冊 pattern |
| **Anti-Recursion Guard** | All Agents MCP | [all-agents-mcp](all-agents-mcp-analysis.md) | 偵測呼叫者排除自己 |
| **MCP Auth Providers** | Gemini CLI | [gemini-cli](gemini-cli-analysis.md) | Google/OAuth/Service Account |
| **MCP Resilient Runtime** | Swarm | [swarm](swarm-analysis.md) | 5-state agent with self-correction |
| **Extension Manager** (5 types) | Goose | [goose](goose-analysis.md) | Stdio/SSE/Builtin/Frontend/Platform |

## Rust-Specific Patterns

| 功能 | 參考專案 | 分析文件 | 重點 |
|------|---------|---------|------|
| **Dual-Trait Pattern** (async object-safe) | Anda | [anda](anda-analysis.md) | `Agent<C>` + `AgentDyn<C>` + `AgentWrapper` |
| **Typestate Builder** | Rig | [rig](rig-analysis.md) | 編譯期強制配置正確性 |
| **ToolDispatcher Trait** (XML/Native dual) | ZeroClaw | [zeroclaw](zeroclaw-analysis.md) | 抽象化雙模式 tool dispatch |
| **WASM Sandbox** (capability-based) | AutoAgents | [autoagents](autoagents-analysis.md) | WASM runtime 執行 sandboxed tools |
| **AppBuilder Startup** | IronClaw | [ironclaw](ironclaw-analysis.md) | 元件組裝從 main.rs 提取出來 |
| **PubSub Event Bus** (generic Broker) | OpenCode | [opencode](opencode-analysis.md) | `Broker<T>` typed channels |
| **AgentHooks** (10 lifecycle hooks) | AutoAgents | [autoagents](autoagents-analysis.md) | create/shutdown/run/turn/tool 各階段 |
| **ChannelLifecycle/Sender Split** | OpenCrust | [opencrust](opencrust-analysis.md) | Arc-safe sharing 的分離設計 |

---

## All Analysis Files

| # | Project | Language | File |
|---|---------|----------|------|
| 1 | Goose (Block) | Rust+Python | [goose-analysis.md](goose-analysis.md) |
| 2 | AutoAgents | Rust | [autoagents-analysis.md](autoagents-analysis.md) |
| 3 | ZeroClaw | Rust | [zeroclaw-analysis.md](zeroclaw-analysis.md) |
| 4 | IronClaw (NEAR AI) | Rust | [ironclaw-analysis.md](ironclaw-analysis.md) |
| 5 | OpenFang | Rust | [openfang-analysis.md](openfang-analysis.md) |
| 6 | Anda | Rust | [anda-analysis.md](anda-analysis.md) |
| 7 | OpenCode | Go | [opencode-analysis.md](opencode-analysis.md) |
| 8 | Codex CLI (OpenAI) | Rust+TS | [codex-cli-analysis.md](codex-cli-analysis.md) |
| 9 | Gemini CLI (Google) | TypeScript | [gemini-cli-analysis.md](gemini-cli-analysis.md) |
| 10 | OpenClaw (Claude Code) | TypeScript | [openclaw-analysis.md](openclaw-analysis.md) |
| 11 | Aider | Python | [aider-analysis.md](aider-analysis.md) |
| 12 | Cline | TypeScript | [cline-analysis.md](cline-analysis.md) |
| 13 | Mastra | TypeScript | [mastra-analysis.md](mastra-analysis.md) |
| 14 | OpenHands | Python | [openhands-analysis.md](openhands-analysis.md) |
| 15 | CrewAI | Python | [crewai-analysis.md](crewai-analysis.md) |
| 16 | LangGraph | Python | [langgraph-analysis.md](langgraph-analysis.md) |
| 17 | Letta Agent File | Python | [letta-agent-file-analysis.md](letta-agent-file-analysis.md) |
| 18 | OWL (CAMEL-AI) | Python | [owl-analysis.md](owl-analysis.md) |
| 19 | Rig | Rust | [rig-analysis.md](rig-analysis.md) |
| 20 | tsk | Rust | [tsk-analysis.md](tsk-analysis.md) |
| 21 | OpenCrust | Rust | [opencrust-analysis.md](opencrust-analysis.md) |
| 22 | Teloxide | Rust | [teloxide-analysis.md](teloxide-analysis.md) |
| 23 | Swarm | Rust | [swarm-analysis.md](swarm-analysis.md) |
| 24 | CLI Agent Orchestrator | Python | [cli-agent-orchestrator-analysis.md](cli-agent-orchestrator-analysis.md) |
| 25 | Claude Octopus | Bash/Shell | [claude-octopus-analysis.md](claude-octopus-analysis.md) |
| 26 | Claude Code Bridge | Python | [claude-code-bridge-analysis.md](claude-code-bridge-analysis.md) |
| 27 | All Agents MCP | TypeScript | [all-agents-mcp-analysis.md](all-agents-mcp-analysis.md) |
| 28 | VCPToolBox | Node.js+Rust | [vcptoolbox-analysis.md](vcptoolbox-analysis.md) |
| 29 | Agency Agents | Markdown+Bash | [agency-agents-analysis.md](agency-agents-analysis.md) |
