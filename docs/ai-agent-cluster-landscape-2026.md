# AI Agent Cluster & Self-Evolving系統市場分析

> 2026-03-22 | 研究目的：分析現有生產級 AI agent 集群 / 自我進化系統，找出 Phantom Mesh 可借鑑的架構模式與功能方向。

---

## 一、市場概覽

AI agent 集群從 2024 年的概念驗證快速成長為 2025–2026 年的生產工具。主要分為三個方向：

| 方向 | 代表專案 | 成熟度 |
|------|----------|--------|
| **多 Agent 編排框架** | Swarms, CrewAI, LangGraph | 生產級，活躍社群 |
| **自我進化 Agent** | SAGE, Agent Zero, SoA | 研究/實驗階段 |
| **去中心化 Agent 網路** | AgentNet, desplega-ai/agent-swarm | 早期研究 |

---

## 二、重點專案分析

### 2.1 Swarms（kyegomez/swarms）

**定位：** Enterprise-grade multi-agent orchestration framework

**技術特點：**
- Python，支援 OpenAI / Anthropic / 開源模型
- 多種 Swarm 架構：Sequential, Concurrent, Hierarchical, Mixture of Agents (MoA), Auto-Swarm
- Agent 可透過 YAML/JSON 定義，支援動態生成
- 內建 tool 系統、memory（短期 + 長期）、RAG 整合
- 支援 MCP (Model Context Protocol)

**與 Phantom Mesh 比較：**
| 維度 | Swarms | Phantom Mesh |
|------|--------|---------|
| 語言 | Python | Rust |
| 部署模式 | Library / API | 獨立 daemon + cluster |
| Agent 定義 | YAML/JSON/code | agents.toml + hand.toml |
| 路由策略 | Auto-Swarm (LLM-based) | Smart Router (classifier) |
| 工具系統 | 函數裝飾器 | Tool trait + SecurityConfig |
| 記憶體 | 短期/長期 + RAG | Context compaction |

**可借鑑：**
- **Mixture of Agents (MoA)**：多個 agent 獨立回答後由 aggregator 合成最佳答案，可改進 Phantom Mesh 的 cluster dispatch 品質
- **Auto-Swarm**：根據任務自動選擇最適合的 swarm 架構，可整合進 Phantom Mesh 的 hand 選擇邏輯
- **Agent marketplace**：預定義的專業 agent 模板，Phantom Mesh 的 hands 系統可朝這方向發展

---

### 2.2 CrewAI

**定位：** Role-based multi-agent collaboration platform

**技術特點：**
- 每個 agent 有明確 role、goal、backstory
- Task 有明確輸入/輸出/驗證條件
- Sequential / Hierarchical process（manager agent 分配任務）
- 內建 delegation + human-in-the-loop
- 商業化成功（CrewAI Enterprise）

**與 Phantom Mesh 比較：**
| 維度 | CrewAI | Phantom Mesh |
|------|--------|---------|
| Agent 個性化 | role + backstory | instructions 欄位 |
| 工作流 | process（seq/hierarch） | hands（多階段） |
| Human-in-loop | delegation callback | approval.rs gate |
| 商業模式 | SaaS + Enterprise | Self-hosted |

**可借鑑：**
- **結構化任務輸出**：每個 task 有 `expected_output` schema，Phantom Mesh hands 的 phase 也可加入 output validation
- **Hierarchical Process**：manager agent 動態分配子任務，比 Phantom Mesh 現有的固定 hand phases 更彈性
- **Agent 記憶共享**：crew 內的 agent 可以讀取其他 agent 的輸出，Phantom Mesh cluster 可實現 worker 間結果共享

---

### 2.3 SAGE（Self-evolving Agents）

**定位：** 研究性質的自我進化 agent 框架

**技術特點：**
- 四角色共同進化：**Challenger**（出題）→ **Planner**（拆解）→ **Solver**（解決）→ **Critic**（評估）
- 每次迭代後根據表現更新各角色的 system prompt
- 使用 trajectory logs 分析失敗模式
- 在 coding benchmarks 上超越 GPT-4 + ReAct baseline

**與 Phantom Mesh 比較：**
| 維度 | SAGE | Phantom Mesh |
|------|------|---------|
| 進化機制 | 4-role co-evolution | trajectory.rs 單一 agent |
| 評估 | Critic agent | 無自動評估 |
| 進化目標 | prompt 優化 | prompt evolution（已有基礎） |
| 進化頻率 | 每批次 | 需手動觸發 |

**可借鑑：**
- **Challenger-Critic 模式**：Phantom Mesh 的 Ralph Loop 可以加入 Challenger（自動生成測試場景）和 Critic（評估結果品質）
- **Co-evolution**：不只優化單一 agent 的 prompt，而是同時優化 router、tool 選擇、context compaction 等多個子系統
- **自動化進化循環**：Ralph Loop 可以定期分析 trajectory logs → 識別弱點 → 生成改進方案 → 測試 → 部署

---

### 2.4 AgentNet（去中心化 Agent 網路）

**定位：** 研究論文中的去中心化 RAG-based agent 演化系統

**技術特點：**
- Agent 組成 DAG 網路，每個 agent 有自己的知識庫
- 使用演化算法（crossover + mutation）在 agent 間傳播有效策略
- RAG-based：agent 從鄰居的成功經驗中檢索相關策略
- 適應性：網路拓撲根據任務類型動態調整

**與 Phantom Mesh 比較：**
| 維度 | AgentNet | Phantom Mesh |
|------|----------|---------|
| 拓撲 | DAG（去中心化） | Hub-Worker（中心化） |
| 知識傳播 | RAG crossover | 無（各 worker 獨立） |
| 適應性 | 演化算法 | 手動配置 |

**可借鑑：**
- **策略傳播**：Hub 可以收集各 worker 的成功策略（trajectory），用 RAG 方式讓其他 worker 學習
- **動態拓撲**：根據 worker 能力自動調整任務分配，不是固定的 round-robin
- **跨 worker 知識共享**：成功的 prompt 優化可以從一個 worker 傳播到整個 cluster

---

### 2.5 SoA（Self-Organized Agents）

**定位：** 可擴展的自組織 agent 架構（2024 論文，Microsoft Research 相關）

**技術特點：**
- Agent 數量根據任務複雜度動態增減
- 每個 agent 自主決定是否需要 spawn 子 agent
- 在大型 code generation 任務上優於固定數量 agent
- 無需預定義 agent 角色

**與 Phantom Mesh 比較：**
| 維度 | SoA | Phantom Mesh |
|------|-----|---------|
| Agent 數量 | 動態 | 固定（手動註冊 worker） |
| 自組織 | 自主 spawn | Hub 中心分配 |
| 角色 | 動態生成 | 預定義 hand |

**可借鑑：**
- **動態 worker 數量**：Hub 可以根據任務複雜度自動 spawn 本地 worker 線程
- **任務分解自主性**：worker 可以自行判斷任務是否需要拆分，並請求 Hub 分配更多資源

---

### 2.6 Agent Zero

**定位：** 個人助理型自我修正 agent

**技術特點：**
- 完全自主，可自行創建和修改自己的工具
- 即時修正：不等待整個任務完成，中途發現錯誤立即修正
- 支援多模態（文字、圖片、語音）
- 記憶系統：向量資料庫 + 知識圖譜
- Docker sandbox 隔離執行環境

**與 Phantom Mesh 比較：**
| 維度 | Agent Zero | Phantom Mesh |
|------|-----------|---------|
| 工具系統 | 動態自創工具 | 53 個預定義 tools |
| 自我修正 | 即時 | trajectory-based |
| 沙箱 | Docker | SecurityConfig |
| 記憶 | 向量 DB + KG | context compaction |

**可借鑑：**
- **動態工具創建**：Phantom Mesh 的 tool 系統可以支援 agent 在運行時定義新工具（已有 hands 的雛形）
- **即時修正**：不等到任務結束才分析 trajectory，在執行中就檢查中間結果
- **知識圖譜記憶**：比單純的 context compaction 更結構化，可以跨 session 保留實體關係

---

### 2.7 fcn06/swarm（Rust Agent SDK）

**定位：** Rust 實作的 agent SDK，支援 MCP/A2A 協議

**技術特點：**
- 純 Rust 實作，性能導向
- 支援 MCP (Model Context Protocol) 和 A2A (Agent-to-Agent) 標準協議
- Agent 透過 handoff 機制在 agent 間傳遞控制權
- Tool 系統基於 `#[tool]` macro

**與 Phantom Mesh 比較：**
| 維度 | fcn06/swarm | Phantom Mesh |
|------|------------|---------|
| 語言 | Rust | Rust |
| 協議 | MCP + A2A | 自定義 HTTP API |
| Agent 間通訊 | handoff | cluster dispatch |
| Tool 定義 | `#[tool]` macro | Tool trait |

**可借鑑：**
- **MCP 支援**：Phantom Mesh 已有 MCP 相關參考（All Agents MCP），可以作為 worker 間標準通訊協議
- **A2A 協議**：Google 的 Agent-to-Agent 標準，適合 Phantom Mesh cluster 跨節點通訊
- **Handoff 機制**：比 Hub 中心分配更靈活，worker 之間可以直接傳遞任務

---

### 2.8 desplega-ai/agent-swarm

**定位：** Lead agent + Docker worker agents 的編碼任務系統

**技術特點：**
- Lead agent 接收任務，分解後分配給 Docker 容器中的 worker agents
- 每個 worker 在隔離環境中執行
- 支援 git 操作（fork, branch, PR）
- 自動化 code review

**與 Phantom Mesh 比較：**
| 維度 | desplega-ai | Phantom Mesh |
|------|------------|---------|
| 架構 | Lead + Docker workers | Hub + remote workers |
| 隔離 | Docker 容器 | process-level |
| 版本控制 | 內建 git | 透過 tools |

**可借鑑：**
- **容器化 worker**：每個 worker 在 Docker 中執行，資源隔離更徹底
- **git-native workflow**：任務結果自動 commit + PR，可整合進 Phantom Mesh 的開發工作流

---

### 2.9 NVIDIA OpenShell

**定位：** Policy-based security framework for autonomous agents

**技術特點：**
- 針對自主 agent 的安全控制框架
- Policy engine 定義 agent 可以/不可以做什麼
- 特別關注 self-evolving agent 的安全邊界
- 確保 agent 自我修改不會突破安全限制

**可借鑑：**
- **進化邊界**：Phantom Mesh 的 self-evolution（trajectory.rs）需要安全邊界，確保 prompt evolution 不會導致越權
- **Policy engine**：SecurityConfig 可以擴展為更完整的 policy 系統，控制 agent 的自我修改範圍

---

## 三、關鍵趨勢與模式

### 3.1 架構趨勢

```
2024                    2025                    2026
─────────────────────────────────────────────────────
單 Agent + Tools  →  Multi-Agent 編排   →  自組織 Agent 網路
固定 Prompt       →  Prompt 優化        →  Co-evolution
手動部署          →  Container 隔離     →  自動 Scaling
HTTP API          →  MCP / A2A          →  標準化協議
```

### 3.2 共通模式

1. **Hub-Worker 是主流架構**：幾乎所有生產級系統都用中心化調度（Hub/Manager/Lead Agent）。去中心化（AgentNet）仍在研究階段。

2. **Trajectory-based 進化是共識**：SAGE、Agent Zero、Phantom Mesh 都用執行軌跡來驅動改進。差異在自動化程度。

3. **MCP 正在成為標準**：Model Context Protocol 讓不同 agent 框架可以互通工具。Phantom Mesh 應該優先支援。

4. **安全是瓶頸**：自我進化 agent 的最大挑戰不是技術，而是如何確保進化後的行為仍在安全範圍內。

5. **Memory 系統分層**：Working memory → Short-term → Long-term → Knowledge Graph，比單一 context window 更有效。

### 3.3 生產級 vs 研究級

| 特徵 | 生產級（Swarms, CrewAI） | 研究級（SAGE, AgentNet） |
|------|--------------------------|--------------------------|
| 可靠性 | 高，有 fallback | 實驗性 |
| 部署 | pip install + API | 需要自行搭建 |
| 文檔 | 完整 | 論文 + 範例 |
| 社群 | 活躍 | 學術 |
| 自進化 | 無/有限 | 核心功能 |

---

## 四、Phantom Mesh 的定位與建議

### 4.1 Phantom Mesh 的獨特優勢

1. **Rust 性能**：目前市場上幾乎沒有 Rust 實作的完整 agent cluster（fcn06/swarm 是 SDK 不是完整系統）
2. **53 tools + 48 hands**：工具和工作流覆蓋面廣
3. **多層安全**：SecurityConfig + Approval Gate + Guardrail 三層防護
4. **Telegram 原生**：適合個人/小團隊使用場景
5. **Self-hosted**：隱私優先，適合企業內部部署

### 4.2 建議發展方向

#### 短期（可立即開始）

| 優先級 | 項目 | 參考 | 預期效果 |
|--------|------|------|----------|
| P0 | **Ralph Loop 自動化** | SAGE Challenger-Critic | 自動分析 trajectory → 識別弱點 → 建議改進 |
| P0 | **Worker 間結果共享** | AgentNet RAG crossover | cluster 內 worker 可以學習彼此的成功策略 |
| P1 | **MCP 協議支援** | fcn06/swarm, Goose | 標準化 tool 通訊，可接入外部 MCP server |
| P1 | **動態 worker scaling** | SoA | Hub 根據任務複雜度自動增減本地 worker 線程 |

#### 中期（1-3 個月）

| 優先級 | 項目 | 參考 | 預期效果 |
|--------|------|------|----------|
| P1 | **分層記憶系統** | Agent Zero, Mastra | 比 context compaction 更結構化的記憶管理 |
| P1 | **Mixture of Agents** | Swarms MoA | 多個 worker 獨立回答 → aggregator 合成最佳答案 |
| P2 | **Task output validation** | CrewAI expected_output | Hands phase 的輸出品質自動驗證 |
| P2 | **A2A 協議** | Google A2A spec | Worker 間直接通訊，減少 Hub 瓶頸 |

#### 長期（3-6 個月）

| 優先級 | 項目 | 參考 | 預期效果 |
|--------|------|------|----------|
| P2 | **Co-evolution** | SAGE 4-role | 同時優化 router + tool 選擇 + context + prompt |
| P2 | **Agent marketplace** | Swarms agents | 預定義的專業 agent 模板社群 |
| P3 | **去中心化模式** | AgentNet DAG | 大規模部署時的替代拓撲 |

### 4.3 風險評估

| 風險 | 嚴重度 | 緩解策略 |
|------|--------|----------|
| 自我進化失控 | 高 | NVIDIA OpenShell 的 policy engine 思路，限制進化邊界 |
| 過度複雜化 | 中 | YAGNI 原則，每次只加一個功能 |
| 生態系競爭 | 中 | 專注 Rust 性能 + self-hosted 差異化 |
| 協議碎片化 | 低 | 押注 MCP/A2A 標準 |

---

## 五、結論

Phantom Mesh 在 Rust agent cluster 領域幾乎沒有直接競爭者。市場上的 Python 框架（Swarms, CrewAI）在易用性和社群上領先，但 Phantom Mesh 的 Rust 性能 + 完整安全架構 + self-hosted 模式在企業場景有獨特價值。

**最重要的三件事：**

1. **Ralph Loop 自動化**（借鑑 SAGE）：讓 Phantom Mesh 能自動分析自己的表現並改進，這是與 Swarms/CrewAI 差異化的關鍵
2. **MCP 協議**（借鑑 fcn06/swarm, Goose）：加入標準生態系，不要閉門造車
3. **分層記憶**（借鑑 Agent Zero, Mastra）：context compaction 不夠，需要跨 session 的結構化記憶

這三個方向可以在不大幅重構現有架構的前提下，顯著提升 Phantom Mesh 的競爭力。
