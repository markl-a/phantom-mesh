# Agency Agents 深度技術分析

> 分析日期: 2026-03-14
> 專案位置: `github.com/msitarzewski/agency-agents`
> 語言: Markdown + Bash (agent 定義庫，非框架)
> 作者: msitarzewski (GitHub)
> Stars: 42.1k | Forks: 6.3k | License: MIT

---

## 目錄

1. [專案概述與定位](#1-專案概述與定位)
2. [Repository 結構](#2-repository-結構)
3. [Agent 定義格式 — YAML Frontmatter + Markdown Sections](#3-agent-定義格式--yaml-frontmatter--markdown-sections)
4. [Agent 分類體系 — 11 部門 144+ 專家](#4-agent-分類體系--11-部門-144-專家)
5. [NEXUS 多代理協調框架](#5-nexus-多代理協調框架)
6. [跨工具整合系統 — 10 平台轉換器](#6-跨工具整合系統--10-平台轉換器)
7. [MCP Memory 持久化記憶](#7-mcp-memory-持久化記憶)
8. [品質控制 — Linter + 貢獻規範](#8-品質控制--linter--貢獻規範)
9. [工作流範例分析](#9-工作流範例分析)
10. [Clawtex 差距對比與實作建議](#10-clawtex-差距對比與實作建議)

---

## 1. 專案概述與定位

### 1.1 什麼是 Agency Agents

Agency Agents 不是一個程式框架，而是一個**結構化的 AI 代理人格庫**。它提供 144+ 個精心設計的 agent 定義文件，每個 agent 具有：

- 獨立的人格特質和溝通風格
- 領域專業知識和工作流程
- 具體的技術交付物（含程式碼範例）
- 可衡量的成功指標

### 1.2 核心哲學

```
傳統 prompt: "你是一個有幫助的助手，幫我寫後端程式"
Agency pattern: "你是 Backend Architect，偏好防禦性架構設計，
                要求 API 回應 < 200ms P95，所有端點必須
                RBAC + rate limit，交付物包含 schema、API spec、
                和部署架構圖"
```

**關鍵差異**: 不是泛化的助手，而是**深度角色化的專家**，帶有強制性的工作流程和質量標準。

### 1.3 與其他參考專案的定位差異

| 維度 | Agency Agents | CrewAI | Swarm | Clawtex |
|------|--------------|--------|-------|---------|
| 類型 | Agent 定義庫 | Agent 框架 | DAG 引擎 | Agent 運行時 |
| 核心產物 | Markdown 文件 | Python 類 | Rust Crate | Rust 二進制 |
| Agent 數量 | 144+ | 自定義 | 自定義 | agents.toml 配置 |
| 協調方式 | NEXUS 策略文檔 | Hierarchical/Sequential | DAG 拓撲 | Hands 工作流 |
| 執行引擎 | 無（靠外部 IDE/CLI） | 內建 | 內建 | 內建 |

---

## 2. Repository 結構

### 2.1 目錄樹

```
agency-agents/
├── .github/                         # CI 配置
├── design/                          # 8 agents: UI, UX, Brand, Visual...
├── engineering/                     # 23 agents: Frontend, Backend, AI, DevOps...
├── marketing/                       # 26 agents: SEO, Growth, 各平台策略師...
├── paid-media/                      # 7 agents: PPC, Programmatic, Creative...
├── sales/                           # 8 agents: Outbound, Discovery, Pipeline...
├── product/                         # 4 agents: Sprint, Trend, Feedback...
├── project-management/              # 6 agents: Orchestrator, Shepherd...
├── testing/                         # 8 agents: Evidence Collector, Reality Checker...
├── support/                         # 6 agents: Support, Finance, Legal...
├── spatial-computing/               # 6 agents: XR, visionOS, WebXR...
├── game-development/                # 5 通用 + 5 引擎子目錄 (Unity/Unreal/Godot/Blender/Roblox)
├── specialized/                     # 23 agents: Orchestrator, Blockchain, MCP Builder...
├── strategy/                        # NEXUS 協調框架
│   ├── coordination/
│   │   ├── agent-activation-prompts.md   # 標準化的 agent 啟動模板
│   │   └── handoff-templates.md          # 結構化的 agent 交接協議
│   ├── playbooks/
│   │   ├── phase-0-discovery.md          # 7 階段流水線的每個階段指南
│   │   ├── phase-1-strategy.md
│   │   ├── phase-2-foundation.md
│   │   ├── phase-3-build.md
│   │   ├── phase-4-hardening.md
│   │   ├── phase-5-launch.md
│   │   └── phase-6-operate.md
│   ├── runbooks/
│   │   ├── scenario-startup-mvp.md       # 4 個場景 runbook
│   │   ├── scenario-enterprise-feature.md
│   │   ├── scenario-marketing-campaign.md
│   │   └── scenario-incident-response.md
│   ├── EXECUTIVE-BRIEF.md
│   ├── QUICKSTART.md
│   └── nexus-strategy.md                # 核心策略文檔
├── examples/
│   ├── nexus-spatial-discovery.md        # 8 agent 並行協作範例
│   ├── workflow-startup-mvp.md           # MVP 開發多 agent 流程
│   ├── workflow-landing-page.md
│   ├── workflow-book-chapter.md
│   └── workflow-with-memory.md           # MCP 記憶整合範例
├── integrations/
│   ├── aider/                            # 10 個工具的轉換輸出
│   ├── antigravity/
│   ├── claude-code/
│   ├── cursor/
│   ├── gemini-cli/
│   ├── github-copilot/
│   ├── mcp-memory/                       # MCP 記憶伺服器整合
│   ├── openclaw/
│   ├── opencode/
│   └── windsurf/
├── scripts/
│   ├── convert.sh                        # 跨平台格式轉換器
│   ├── install.sh                        # 互動式安裝器
│   └── lint-agents.sh                    # Agent 品質驗證器
├── CONTRIBUTING.md
├── LICENSE (MIT)
└── README.md
```

### 2.2 規模統計

| 指標 | 數值 |
|------|------|
| Agent 定義文件 | 144+ |
| 分部 (Division) | 11 |
| 策略文檔 | 12+ |
| 整合平台 | 10 |
| 範例工作流 | 5 |
| Commits | 183 |
| Contributors | 社群活躍 |

---

## 3. Agent 定義格式 — YAML Frontmatter + Markdown Sections

### 3.1 Frontmatter Schema

每個 agent 都以 YAML frontmatter 開頭：

```yaml
---
name: Backend Architect
description: Scalable system design, database architecture, and cloud infrastructure
color: blue
emoji: "🏗️"
vibe: Strategic, security-first, performance-obsessed
services:          # 可選，用於 MCP 或工具配置
  - name: memory
    type: mcp
---
```

**必填欄位** (lint 驗證):
- `name` — Agent 識別名
- `description` — 角色摘要
- `color` — 視覺標識 (cyan/blue/green/yellow/red/purple/orange/pink/teal/gray)

**可選欄位**:
- `emoji` — 快速辨識
- `vibe` — 人格精華描述
- `services` — 外部服務整合

### 3.2 Markdown 內容結構

Frontmatter 之後是結構化的 Markdown sections，分為兩組：

**Persona 組** (人格面，放前面):
1. **Identity & Memory** — 角色定義、人格特質、領域背景
2. **Communication Style** — 語氣、用語習慣、溝通模式
3. **Critical Rules** — 不可違反的操作約束

**Operations 組** (操作面，放後面):
4. **Core Mission** — 主要目標與職責
5. **Technical Deliverables** — 程式碼範例、模板、具體產出
6. **Workflow Process** — 分步驟的工作流程
7. **Success Metrics** — 可衡量的成功標準
8. **Advanced Capabilities** — 進階功能

### 3.3 Backend Architect 範例摘錄

```markdown
## Core Mission

### Data/Schema Engineering
- 高效大數據集結構設計
- ETL pipeline 實作
- 持久化層 < 20ms 查詢效能
- WebSocket 即時串流更新

### Scalable Architecture
- 水平擴展微服務設計
- 版本化 API
- 事件驅動系統
- 安全性作為預設需求

## Success Metrics
- API response times under 200ms (95th percentile)
- 99.9%+ system uptime with monitoring
- Database queries averaging under 100ms
- Zero critical security vulnerabilities in audits
- System handling 10x traffic spikes
```

### 3.4 Agents Orchestrator 特殊結構

Orchestrator agent 額外包含：
- **Decision Logic** — 條件分支邏輯
- **Status Reporting** — 進度報告模板
- **Learning & Memory** — 模式識別能力
- **Available Specialist Agents** — 可調度的 agent 清單（按部門分類）

```markdown
## Decision Logic
- Maximum 3 retries per task before escalation
- Quality gates with evidence requirements
- Conditional branching for task progression
- Error handling with specific recovery paths

## Available Specialist Agents
### Engineering Division
- Frontend Developer
- Backend Architect
- AI Engineer
- DevOps Automator
...
```

---

## 4. Agent 分類體系 — 11 部門 144+ 專家

### 4.1 部門與 Agent 分布

| 部門 | Agent 數 | 代表角色 | 特色 |
|------|---------|---------|------|
| Engineering | 23 | Backend Architect, AI Engineer, Security Engineer, SRE | 涵蓋全端+基礎設施 |
| Marketing | 26 | SEO Specialist, Growth Hacker, 微信/抖音/快手/B站策略師 | 中國市場深度覆蓋 |
| Game Dev | 25+ | Game Designer + Unity/Unreal/Godot/Blender/Roblox 專家 | 按引擎子分類 |
| Specialized | 23 | Agents Orchestrator, MCP Builder, ZK Steward | 跨領域+前沿技術 |
| Design | 8 | UX Researcher, Brand Guardian, Whimsy Injector | 設計全流程 |
| Sales | 8 | Outbound Strategist, Deal Strategist, Pipeline Analyst | 銷售全漏斗 |
| Testing | 8 | Evidence Collector, Reality Checker, API Tester | QA 全覆蓋 |
| Paid Media | 7 | PPC Strategist, Programmatic Buyer | 付費廣告專業 |
| PM | 6 | Studio Producer, Project Shepherd, Jira Steward | 專案管理 |
| Support | 6 | Finance Tracker, Legal Compliance, Executive Summary | 營運支援 |
| Spatial | 6 | XR Architect, visionOS Engineer, WebXR Developer | AR/VR/XR |

### 4.2 值得注意的 Agent 設計

**LSP Index Engineer** (`specialized/lsp-index-engineer.md`):
- 專門處理 Language Server Protocol 索引
- 對 coding agent 的程式碼理解直接相關

**MCP Builder** (`specialized/specialized-mcp-builder.md`):
- 專門設計/建造 MCP Server 的 agent
- 理解 JSON-RPC 2.0、stdio/SSE 傳輸

**Agents Orchestrator** (`specialized/agents-orchestrator.md`):
- 多代理協調的元 agent
- 管理 agent 啟動、交接、品質門檻
- 包含可用 agent 的完整清單

**Model QA** (`specialized/specialized-model-qa.md`):
- LLM 模型品質驗證
- 偏見測試、推理延遲、幻覺偵測

---

## 5. NEXUS 多代理協調框架

### 5.1 概述

NEXUS 不是程式碼，而是一套**結構化的多代理協調策略**，包含：
- 7 階段流水線 (Phase 0-6)
- 結構化交接協議
- 標準化啟動模板
- 品質門檻機制
- 3 種部署模式

### 5.2 七階段流水線

```
Phase 0 (Discovery)    → 市場驗證 + 用戶需求確認
  │ Quality Gate: 市場機會證據
Phase 1 (Strategy)     → 架構定義 + 組合對齊
  │ Quality Gate: 技術方案批准
Phase 2 (Foundation)   → 基礎設施 + 技術腳手架
  │ Quality Gate: 環境就緒確認
Phase 3 (Build)        → 功能開發 (Dev↔QA Loop)
  │ Quality Gate: 每個 task 通過 Evidence Collector
Phase 4 (Hardening)    → 整合測試 (Reality Checker)
  │ Quality Gate: "overwhelming evidence" 才放行
Phase 5 (Launch)       → 跨渠道 GTM 執行
  │ Quality Gate: 上線檢查清單
Phase 6 (Operate)      → 持續營運 + 改進循環
```

### 5.3 三種部署模式

| 模式 | 時長 | Agent 數 | 適用場景 |
|------|------|---------|---------|
| NEXUS-Full | 12-24 週 | 全部 75+ | 完整產品開發 |
| NEXUS-Sprint | 2-6 週 | 15-25 | 功能/MVP 開發 |
| NEXUS-Micro | 1-5 天 | 5-10 | 單一任務（修 bug、稽核、行銷活動） |

### 5.4 結構化交接協議 (Handoff Protocol)

交接模板包含三層資料：

```markdown
## Handoff: [Source Agent] → [Destination Agent]

### Metadata Layer
| Field | Value |
|-------|-------|
| Source Agent | Backend Architect |
| Destination Agent | Frontend Developer |
| Phase | Phase 3 (Build) |
| Task ID | TASK-042 |
| Priority | High |
| Timestamp | 2026-03-14T10:00:00Z |

### Context Layer
- Completed Work: API spec v2, database schema, auth middleware
- File Locations: /api/spec.yaml, /db/schema.sql
- Dependencies: Redis cache, PostgreSQL 16
- Constraints: API response < 200ms P95

### Deliverable Layer
- [ ] React component implementing GET /api/boards
- [ ] WebSocket integration for real-time sync
- [ ] Error handling matching API error schema
- Quality: WCAG 2.1 AA, Core Web Vitals pass
```

### 5.5 品質門檻機制

```
Task 完成 → Evidence Collector 驗證
  │
  ├─ PASS → 進入下一個 task
  │
  └─ FAIL → 附帶具體反饋 → 重試 (最多 3 次)
              │
              └─ 3 次失敗 → Escalation Report → 人工審查
```

**Reality Checker 原則**: 預設為 "NEEDS WORK"，要求 "overwhelming evidence across all criteria" 才給 READY。

### 5.6 Agent 啟動模板

標準化的 prompt 模板，包含填充欄位：

```markdown
## Backend Architect Activation

You are the Backend Architect in [PROJECT NAME], Phase [CURRENT PHASE].

### Reference Materials
- Project Spec: [PATH TO SPEC]
- Architecture Decision Records: [PATH TO ADR]

### Implementation Requirements
- API response times < 200ms P95
- Defense-in-depth security architecture
- Schema versioning with migration scripts

### Quality Standards
- Evidence required for all assessments
- Maximum 3 retries per task before escalation
```

---

## 6. 跨工具整合系統 — 10 平台轉換器

### 6.1 支援平台

| 平台 | 轉換格式 | 輸出路徑 |
|------|---------|---------|
| Claude Code | 原生 Markdown | `~/.claude/agents/` |
| Cursor | `.mdc` 規則文件 | `integrations/cursor/rules/` |
| Aider | 單一 `CONVENTIONS.md` | `integrations/aider/` |
| Windsurf | `.windsurfrules` | `integrations/windsurf/` |
| Gemini CLI | `SKILL.md` + `gemini-extension.json` | `integrations/gemini-cli/skills/` |
| OpenCode | Markdown + 色彩映射 | `integrations/opencode/agents/` |
| OpenClaw | 三文件分割 (SOUL/AGENTS/IDENTITY) | `integrations/openclaw/` |
| Antigravity | YAML frontmatter SKILL.md | `integrations/antigravity/` |
| GitHub Copilot | 平台格式 | `integrations/github-copilot/` |
| Qwen Code | Markdown + tools 欄位 | `integrations/qwen/agents/` |

### 6.2 轉換器架構 (`convert.sh`)

```bash
# 核心流程
for each agent file in 11 directories:
    1. extract_frontmatter()  → name, description, color, emoji, vibe
    2. extract_body()         → Markdown 內容
    3. slugify(name)          → kebab-case 標識符
    4. dispatch to per-tool converter:
       - to_cursor()    → .mdc with alwaysApply flag
       - to_openclaw()  → SOUL.md + AGENTS.md + IDENTITY.md (按關鍵字分割)
       - to_aider()     → 追加到單一 CONVENTIONS.md
       - to_opencode()  → color name → hex 映射
       ...
```

**OpenClaw 三文件分割邏輯**:
- `SOUL.md` — 匹配 "identity", "communication", "style" 關鍵字的段落
- `AGENTS.md` — 匹配 "operations", "workflow", "mission", "deliverables" 的段落
- `IDENTITY.md` — emoji + name + vibe 或 fallback description

### 6.3 安裝方式

```bash
# 生成所有平台整合文件
./scripts/convert.sh

# 互動式安裝（選擇目標工具）
./scripts/install.sh

# 指定工具安裝
./scripts/install.sh --tool cursor
```

---

## 7. MCP Memory 持久化記憶

### 7.1 四個核心操作

| 操作 | 用途 |
|------|------|
| `remember` | 儲存交付物/決策，附帶多個 tag |
| `recall` | 按 tag 檢索之前的記憶 |
| `rollback` | 回退到已知好的檢查點 |
| `search` | 按專案 tag 搜索跨 agent 的記憶 |

### 7.2 Tag 協議

```
Agent 儲存: remember("API spec v2", tags: ["backend-architect", "retroboard", "api-spec", "frontend-developer"])
                                             ^^^^^^^^^^^^^^^^^   ^^^^^^^^^^   ^^^^^^^^   ^^^^^^^^^^^^^^^^^^
                                             來源 agent           專案名      內容類型    目標 agent

下游 Agent: recall(tags: ["retroboard", "api-spec"])
                          → 自動獲得上游 agent 的交付物
```

### 7.3 跨 Session 持久化

```
Session 1: Backend Architect → remember(schema, API spec)
  --- session timeout ---
Session 2: Frontend Developer → recall("retroboard") → 自動獲得所有上游交付物
  --- QA 失敗 ---
Session 3: Backend Architect → rollback(checkpoint_id) → 恢復到已知好的版本
```

### 7.4 整合方式

不需要修改程式碼——只需在 agent 定義的 prompt 中加入 Memory Integration 段落：

```markdown
## Memory Integration

At the start of each session:
- search(tags: ["[PROJECT]", "[YOUR-ROLE]"])

When completing deliverables:
- remember(deliverable, tags: ["[YOUR-ROLE]", "[PROJECT]", "[CONTENT-TYPE]", "[NEXT-AGENT]"])

When receiving QA feedback:
- recall previous checkpoint
- Apply feedback
- remember updated version
```

---

## 8. 品質控制 — Linter + 貢獻規範

### 8.1 Linter 規則 (`lint-agents.sh`)

**Error (blocking)**:
- 缺少 frontmatter `---` 定界符
- 缺少 `name` 欄位
- 缺少 `description` 欄位
- 缺少 `color` 欄位
- frontmatter 為空

**Warning (non-blocking)**:
- 缺少 "Identity" 段落
- 缺少 "Core Mission" 段落
- 缺少 "Critical Rules" 段落
- 正文少於 50 字

### 8.2 貢獻規範五原則

1. **Strong Personality** — 有獨特的語氣和個性，不是泛化助手
2. **Clear Deliverables** — 具體的程式碼範例和模板
3. **Success Metrics** — 可衡量的指標（如 "Page load < 3s"）
4. **Proven Workflows** — 經過實戰驗證的步驟流程
5. **Learning Memory** — 能識別模式並追蹤改進

**反模式** (拒絕合併):
- "I will help you with..." 泛化描述
- 缺少程式碼範例
- 過於寬泛的角色範圍
- 未經測試的理論概念

---

## 9. 工作流範例分析

### 9.1 並行協作模式 (`nexus-spatial-discovery.md`)

```
                    ┌─ Product Trend Researcher → 市場分析
                    ├─ Backend Architect → 技術架構
                    ├─ Brand Guardian → 品牌定位
同一 Brief ─────────┼─ Growth Hacker → GTM 策略
(~10 min wall time) ├─ Support Responder → 客戶體驗
                    ├─ UX Researcher → 用戶研究
                    ├─ Project Shepherd → 時間線
                    └─ XR Interface Architect → 空間介面
                              │
                              ▼
                    Cross-Agent Synthesis
                    ├─ 5 共識點 (所有 agent 獨立得出相同結論)
                    └─ 衝突表 (如定價 $29 vs $99 — 需人類決策)
```

**模式**: Parallel-then-Synthesize (非 Choreographed)

### 9.2 Sequential Handoff 模式 (`workflow-startup-mvp.md`)

```
Week 1: Sprint Prioritizer ──┐
        UX Researcher ───────┤
                             ├──▶ Backend Architect
Week 2: Frontend Developer ◀─┘    Rapid Prototyper
        │ midpoint quality gate (Reality Checker)
Week 3: Frontend (refine) + Growth Hacker
Week 4: Reality Checker final gate → Launch
```

**模式**: Sequential with Parallel Branches + Quality Gates

### 9.3 Memory-Augmented 模式 (`workflow-with-memory.md`)

```
Sprint Prioritizer → remember(sprint_plan)
UX Researcher → remember(research_brief)
Backend Architect → recall(sprint_plan, research_brief)
                  → remember(api_spec, schema)
Frontend Developer → recall(api_spec)  ← 自動無需手動複製
Reality Checker → recall(ALL project deliverables)

QA Reject → rollback → fix → remember(updated)
```

**模式**: MCP Memory as Shared State Bus

---

## 10. Clawtex 差距對比與實作建議

### 10.1 Agent 人格定義 vs agents.toml

**Agency Agents**: 每個 agent 一個 Markdown 文件，包含 8+ 結構化段落
**Clawtex**: `agents.toml` 中每個 agent 只有 `system_prompt`（一段文字）

```toml
# 目前的 clawtex agents.toml
[agents.seo]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
system_prompt = "你是 SEO 專家..."
```

**Clawtex 實作建議**: 擴展 agent 定義，從單一 `system_prompt` 改為結構化欄位

```toml
[agents.backend_architect]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
division = "engineering"
color = "blue"

[agents.backend_architect.persona]
identity = "Senior systems architect, security-first mindset"
communication_style = "Technical, precise, schema-driven"
critical_rules = [
    "All endpoints require RBAC + rate limit",
    "API response < 200ms P95",
    "Defense-in-depth as default",
]

[agents.backend_architect.operations]
system_prompt = "你是 Backend Architect..."
success_metrics = [
    "API response < 200ms P95",
    "99.9%+ uptime",
    "Zero critical CVE",
]
deliverables = ["schema.sql", "api-spec.yaml", "deployment-arch.md"]
```

> **好處**: 結構化的 agent 定義讓 `self_evolve` hand 可以針對性地優化每個欄位（比如根據歷史數據自動調整 success_metrics），而不是盲目修改整段 system_prompt。

---

### 10.2 NEXUS 品質門檻 vs Hands Quality Gate

**Agency Agents**: 分層品質門檻 (Evidence Collector → Reality Checker → Escalation)
**Clawtex**: L1 Guardrail + L2 LLM-as-Judge (已實作，但缺少 Escalation 機制)

```
Agency NEXUS 品質流:
Task → Evidence Collector (PASS/FAIL)
         │
         ├─ PASS → next task
         └─ FAIL → retry (max 3)
                    └─ 3x fail → Escalation Report → Human Review

Clawtex 目前:
Phase → L1 Guardrail (regex/rule) → L2 LLM-as-Judge (score)
         │
         ├─ pass → next phase
         └─ fail → retry (max 3)
                    └─ 3x fail → ??? (目前只是 log error)
```

**Clawtex 實作建議**: 在 `src/hands/runner.rs` 的重試耗盡路徑加入 Escalation：

```rust
// hands/runner.rs — phase 執行後
if retry_count >= max_retries {
    // 生成 Escalation Report
    let report = EscalationReport {
        hand_name: hand.name.clone(),
        phase_name: phase.name.clone(),
        attempts: retry_count,
        last_error: last_error.clone(),
        deliverables_so_far: phase_outputs.clone(),
    };
    // 透過 Telegram 通知用戶
    approval::request_escalation_review(&report).await?;
    // 等待人工決策: retry / skip / abort
}
```

---

### 10.3 結構化交接 vs Hands Phase Chaining

**Agency Agents**: Metadata + Context + Deliverable 三層交接模板
**Clawtex**: Phase 之間只有 `{{previous_output}}` 文字傳遞

**Clawtex 實作建議**: 在 Hands 中引入結構化的 phase 交接物件

```rust
#[derive(Serialize, Deserialize)]
pub struct PhaseHandoff {
    pub source_phase: String,
    pub target_phase: String,
    pub completed_work: Vec<String>,
    pub file_locations: HashMap<String, String>,
    pub constraints: Vec<String>,
    pub acceptance_criteria: Vec<String>,
}

// 在 phase 完成時生成
let handoff = PhaseHandoff {
    source_phase: "backend_design".into(),
    target_phase: "frontend_build".into(),
    completed_work: vec!["API spec v2".into(), "DB schema".into()],
    file_locations: [("api_spec", "workspace/api-spec.yaml")].into(),
    constraints: vec!["API response < 200ms P95".into()],
    acceptance_criteria: vec!["All endpoints tested".into()],
};

// 注入下一個 phase 的 prompt
let enriched_prompt = format!(
    "{}\n\n## Context from Previous Phase\n{}",
    phase.prompt,
    serde_json::to_string_pretty(&handoff)?
);
```

---

### 10.4 跨平台 Agent 格式 vs Clawtex 專用格式

**Agency Agents**: 一份 Markdown → 10 種工具格式轉換
**Clawtex**: `agents.toml` 專用格式，不可移植

**Clawtex 實作建議**: 如果未來要開源 agent 定義，可以加入 import/export：

```rust
// 新增 src/agent_format.rs
pub fn import_agency_agent(markdown: &str) -> Result<AgentConfig> {
    let frontmatter = parse_yaml_frontmatter(markdown)?;
    let sections = parse_markdown_sections(markdown)?;

    Ok(AgentConfig {
        name: frontmatter.name,
        system_prompt: sections.get("Core Mission")
            .map(|s| format_as_system_prompt(s)),
        persona: AgentPersona {
            identity: sections.get("Identity & Memory").cloned(),
            rules: sections.get("Critical Rules")
                .map(|s| parse_bullet_list(s)),
            metrics: sections.get("Success Metrics")
                .map(|s| parse_bullet_list(s)),
        },
        ..Default::default()
    })
}

pub fn export_to_agency_format(config: &AgentConfig) -> String {
    format!("---\nname: {}\ndescription: {}\ncolor: {}\n---\n\n{}",
        config.name,
        config.description,
        config.color,
        format_as_agency_markdown(config))
}
```

---

### 10.5 MCP Memory Tag 協議 vs Clawtex memory_store

**Agency Agents**: Multi-tag 記憶，支持跨 agent 檢索 + rollback
**Clawtex**: `memory_store`/`memory_recall` 工具，key-value 簡單儲存

```
Agency tag 模式:
remember("API spec", tags: ["backend-architect", "retroboard", "api-spec", "frontend-developer"])
                            ^^^^^^^^^^^^^^^^^^^   ^^^^^^^^^^   ^^^^^^^^   ^^^^^^^^^^^^^^^^^^
                            source                project      type       target

Clawtex 目前:
memory_store(key: "api_spec", content: "...", category: "work")
             → 只有一個 key，沒有多 tag、沒有 target agent、沒有 rollback
```

**Clawtex 實作建議**: 擴展 memory 表支援多 tag

```sql
-- 擴展 memory.db schema
ALTER TABLE memories ADD COLUMN tags TEXT; -- JSON array: ["backend", "retroboard", "api-spec"]

-- 查詢: 找所有與 "retroboard" 專案相關的記憶
SELECT * FROM memories WHERE tags LIKE '%"retroboard"%';

-- 查詢: 找上游 agent 留給我的交付物
SELECT * FROM memories WHERE tags LIKE '%"frontend-developer"%'
                        AND tags LIKE '%"retroboard"%';
```

---

### 10.6 總體差距矩陣

| 特性 | Agency Agents | Clawtex | 差距 | 優先級 |
|------|--------------|---------|------|--------|
| Agent 人格深度 | 8+ 結構化段落 | 單一 system_prompt | **大** | 中 |
| 品質門檻升級 | 3 層 (Evidence→Reality→Escalation) | L1+L2 (無 Escalation) | 中 | 高 |
| 結構化交接 | Metadata+Context+Deliverable | 文字串接 | **大** | 中 |
| 多 Tag 記憶 | remember/recall/rollback/search | key-value store | 中 | 中 |
| 跨工具格式 | 10 平台轉換器 | 專用 TOML | 小 | 低 |
| Agent 庫規模 | 144+ 預定義 | 自定義 | 不同策略 | — |
| 並行 Agent 協作 | Parallel-then-Synthesize | 線性 Hands | 中 | 高 |
| 場景 Runbook | 4 種場景模板 | 17 hands 定義 | 不同策略 | — |
| 人格一致性驗證 | Linter 強制 | 無 | 小 | 低 |
| Consensus Detection | 跨 agent 共識偵測 | 無 | 中 | 低 |

---

## 附錄 A: 完整 Agent 清單 (按部門)

### Engineering (23)
ai-data-remediation-engineer, ai-engineer, autonomous-optimization-architect, backend-architect, code-reviewer, data-engineer, database-optimizer, devops-automator, embedded-firmware-engineer, feishu-integration-developer, frontend-developer, git-workflow-master, incident-response-commander, mobile-app-builder, rapid-prototyper, security-engineer, senior-developer, software-architect, solidity-smart-contract-engineer, sre, technical-writer, threat-detection-engineer, wechat-mini-program-developer

### Marketing (26)
app-store-optimizer, baidu-seo-specialist, bilibili-content-strategist, book-co-author, carousel-growth-engine, china-ecommerce-operator, content-creator, cross-border-ecommerce, douyin-strategist, growth-hacker, instagram-curator, kuaishou-strategist, linkedin-content-creator, livestream-commerce-coach, podcast-strategist, private-domain-operator, reddit-community-builder, seo-specialist, short-video-editing-coach, social-media-strategist, tiktok-strategist, twitter-engager, wechat-official-account, weibo-strategist, xiaohongshu-specialist, zhihu-strategist

### Specialized (23)
accounts-payable-agent, agentic-identity-trust, agents-orchestrator, automation-governance-architect, blockchain-security-auditor, compliance-auditor, corporate-training-designer, data-consolidation-agent, government-digital-presales-consultant, healthcare-marketing-compliance, identity-graph-operator, lsp-index-engineer, recruitment-specialist, report-distribution-agent, sales-data-extraction-agent, cultural-intelligence-strategist, developer-advocate, document-generator, mcp-builder, model-qa, study-abroad-advisor, supply-chain-strategist, zk-steward

---

## 附錄 B: Feature Extraction Table

| Feature | Agency Agents 實作方式 | Clawtex 適用性 | 優先級 |
|---------|----------------------|---------------|--------|
| **Structured Agent Persona** | YAML frontmatter + 8 Markdown sections | 擴展 agents.toml 加入 persona/operations 結構 | 中 |
| **NEXUS 7-Phase Pipeline** | Strategy 文檔 + Playbooks | 作為 Hand template 預設流程 | 低 |
| **Handoff Protocol** | Metadata + Context + Deliverable 三層模板 | PhaseHandoff struct 注入 prompt | 中 |
| **Evidence-Based Quality Gate** | Evidence Collector + Reality Checker + Escalation | 擴展現有 L1+L2 加入 Escalation → Telegram | 高 |
| **Parallel-then-Synthesize** | 多 agent 同 brief → 合成層 | Hands DAG 並行 phase + 合成 phase | 高 |
| **Multi-Tag Memory** | remember(data, tags:[...]) + recall + rollback | 擴展 memory.db schema 加 tags column | 中 |
| **Cross-Tool Format Export** | convert.sh → 10 平台 | 可選的 agent 導入/導出 | 低 |
| **Agent Linter** | lint-agents.sh 驗證必填欄位 | 可在 `self_evolve` 中驗證 agent 定義完整性 | 低 |
| **Activation Templates** | 標準化的 prompt 模板 + `[PLACEHOLDER]` | 擴展 Hand TOML 的 prompt_template 支援變數 | 中 |
| **Division-Based Organization** | 11 部門分類 | agents.toml 加入 `division` 欄位，支援按部門路由 | 低 |
| **Consensus Detection** | Cross-Agent Synthesis 偵測共識/衝突 | 多 agent 並行後用 LLM 做共識分析 | 低 |
| **Scenario Runbooks** | 4 種場景 (MVP/Enterprise/Marketing/Incident) | 預建 hand template 庫 | 低 |
| **Agent Roster for Orchestrator** | 可用 agent 清單注入 orchestrator prompt | `list_available_resources()` 動態注入 | 中 |
| **Dev-QA Loop** | Developer → Evidence Collector 迭代 | Hand phase 內建 test→fix 子循環 | 中 |
| **Rollback Checkpoint** | rollback(checkpoint_id) | memory_store 加入版本號 + rollback 操作 | 中 |
