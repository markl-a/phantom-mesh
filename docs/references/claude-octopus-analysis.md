# Claude Octopus 深度技術分析

> **專案來源**: `nyldn/claude-octopus` | 版本 v8.53.0 | MIT License
> **分析日期**: 2026-03-12
> **分析目的**: 為 clawtex-core 提供架構參考與設計模式借鑑

---

## 1. 專案結構

### 1.1 語言與定位

Claude Octopus 是一套 **純 Bash/Shell + Markdown** 構成的 Claude Code 外掛。它不包含任何編譯型語言程式碼，核心引擎 `orchestrate.sh` 約有 **19,000 行** Bash，搭配 JSON 設定檔、Markdown 技能/指令定義、以及少量 Python 輔助腳本。

**核心定位**: 多 AI 模型協作編排器 (Multi-AI Orchestrator)，將 Codex (OpenAI)、Gemini (Google)、Claude (Anthropic) 三個模型以不同角色整合進統一工作流程中。

### 1.2 目錄樹

```
claude-octopus/
├── CLAUDE.md                          # Claude Code 系統指令 (視覺指示器、檔案政策)
├── README.md                          # 使用者文件
├── CHANGELOG.md                       # 版本歷程 (v8.22.0 以降)
├── CONTRIBUTING.md                    # 貢獻指南
├── SECURITY.md                        # 安全政策與威脅模型
├── LICENSE                            # MIT
├── Makefile                           # 測試入口 (smoke/unit/integration/e2e/live/performance)
├── package.json                       # npm 發布配置 (v8.53.0)
├── .mcp.json                          # MCP Server 配置 (Node.js)
├── .coderabbit.yaml                   # CodeRabbit 自動審查配置
├── .gitmodules                        # 子模組 (ui-ux-pro-max-skill)
│
├── .claude/                           # Claude Code 外掛核心
│   ├── settings.json                  # 外掛啟用
│   ├── claude-octopus.local.md        # 本地模式配置 (Dev/Knowledge)
│   ├── DEVELOPMENT.md                 # 開發者指南
│   ├── commands/                      # 39 個斜線指令
│   │   ├── brainstorm.md
│   │   ├── claw.md                    # OpenClaw 管理
│   │   ├── debug.md
│   │   ├── deck.md                    # 簡報生成
│   │   ├── dev.md
│   │   ├── docs.md
│   │   ├── doctor.md                  # 診斷工具
│   │   ├── extract.md
│   │   ├── factory.md                 # Dark Factory (自動管線)
│   │   ├── km.md                      # Knowledge Mode
│   │   ├── loop.md
│   │   ├── meta-prompt.md
│   │   ├── multi.md
│   │   ├── parallel.md               # Team of Teams
│   │   ├── pipeline.md
│   │   ├── prd.md
│   │   ├── prd-score.md
│   │   ├── quick.md
│   │   ├── scheduler.md
│   │   ├── spec.md
│   │   ├── staged-review.md
│   │   └── tdd.md
│   ├── skills/                        # 50 個技能檔案
│   │   ├── skill-parallel-agents.md   # 主要編排技能
│   │   ├── skill-architecture.md
│   │   ├── skill-prd.md
│   │   ├── skill-claw.md
│   │   ├── skill-quick.md
│   │   ├── skill-adversarial-security.md
│   │   ├── skill-context-detection.md
│   │   ├── skill-debate-integration.md
│   │   ├── skill-decision-support.md
│   │   ├── skill-deck.md
│   │   ├── skill-doc-delivery.md
│   │   ├── skill-security-audit.md
│   │   ├── skill-security-framing.md
│   │   ├── skill-visual-feedback.md
│   │   ├── skill-writing-plans.md
│   │   └── ... (更多)
│   ├── hooks/                         # Claude Code 鉤子
│   │   ├── pre-commit.sh             # 外掛名稱鎖定驗證
│   │   └── visual-feedback.sh        # 視覺回饋注入
│   ├── references/                    # 內部參考文件
│   │   ├── stub-detection.md         # Stub 偵測模式
│   │   └── validation-gates.md       # 驗證閘門標準模式
│   └── state/
│       └── state-manager.md          # 狀態管理器技能
│
├── .claude-plugin/                    # 外掛元資料
│   ├── plugin.json                    # 名稱必須是 "octo" (命令前綴)
│   ├── marketplace.json
│   ├── hooks.json                     # 13 個事件類型、29 個鉤子腳本
│   ├── settings.json                  # 16 個可配置項目
│   └── PLUGIN_NAME_LOCK.md           # 名稱鎖定文件 (防止破壞性更改)
│
├── agents/                            # 32 個角色代理
│   └── personas/                      # 角色檔案
│       ├── academic-writer.md
│       ├── ai-engineer.md
│       ├── backend-architect.md       # readonly: true 範例
│       ├── business-analyst.md
│       ├── cloud-architect.md
│       ├── code-reviewer.md
│       ├── context-manager.md
│       ├── database-architect.md
│       ├── debugger.md
│       ├── deployment-engineer.md
│       ├── devops-troubleshooter.md
│       ├── docs-architect.md
│       ├── exec-communicator.md
│       ├── frontend-developer.md
│       ├── graphql-architect.md
│       ├── incident-responder.md
│       ├── mermaid-expert.md
│       ├── openclaw-admin.md
│       ├── performance-engineer.md
│       ├── product-writer.md
│       ├── security-auditor.md        # 最詳細的角色範例
│       └── ... (共 32 個)
│
├── scripts/                           # 核心腳本 (透過 git 追蹤但可能為 sparse checkout)
│   ├── orchestrate.sh                 # 主要引擎 (~19K 行)
│   ├── state-manager.sh              # 狀態管理
│   ├── agent-registry.sh             # Agent 生命週期追蹤
│   ├── reactions.sh                   # 反應引擎
│   ├── build-openclaw.sh             # OpenClaw 構建
│   ├── build-factory-skills.sh       # Factory AI 技能生成
│   ├── coordinator.py                # Python 協調器
│   └── lib/
│       └── routing.sh                # 智慧路由
│
├── mcp-server/                        # Model Context Protocol Server (Node.js)
│   ├── src/
│   │   ├── index.js                  # MCP 進入點
│   │   └── schema/
│   │       └── skill-schema.json     # 統一技能 Schema
│   └── dist/                         # 編譯產物
│
├── openclaw/                          # OpenClaw 擴展
│   ├── package.json                  # @octo-claw/octo-claw
│   ├── openclaw.plugin.json          # OpenClaw 外掛清單
│   └── dist/                         # 編譯產物
│
├── config/                            # 模組化 CLAUDE.md 結構
│   ├── providers/
│   │   ├── codex/CLAUDE.md
│   │   ├── gemini/CLAUDE.md
│   │   └── claude/CLAUDE.md
│   └── workflows/CLAUDE.md
│
├── hooks/                             # Git/CI 鉤子腳本
├── workflows/                         # 工作流程定義
├── templates/                         # 範本
├── tests/                             # 測試套件
│   ├── smoke/                        # <30 秒
│   ├── unit/                         # 1-2 分鐘
│   ├── integration/                  # 5-10 分鐘
│   ├── e2e/                          # 15-30 分鐘
│   ├── benchmark/                    # 效能基準
│   ├── helpers/
│   │   └── test-framework.sh
│   └── fixtures/
│
├── .github/
│   ├── FUNDING.yml                   # GitHub Sponsors
│   ├── ISSUE_TEMPLATE/
│   └── PULL_REQUEST_TEMPLATE.md
│
└── vendors/
    └── ui-ux-pro-max-skill/          # BM25 設計智慧資料庫 (子模組)
```

### 1.3 規模

| 指標 | 數量 |
|------|------|
| 斜線指令 | 39 |
| 技能 (Skills) | 50 |
| 角色代理 (Personas) | 32 |
| 鉤子事件類型 | 13 |
| 鉤子腳本 | 29 |
| 功能旗標 | 86+ |
| 版本檢查點 | 28 |
| 測試 | 89+ (smoke/unit/integration/e2e) |
| `orchestrate.sh` 行數 | ~19,000 |

---

## 2. 進入點

### 2.1 Claude Code 外掛載入

外掛透過 Claude Code 的 marketplace 安裝機制載入:

```
/plugin marketplace add https://github.com/nyldn/claude-octopus.git
/plugin install claude-octopus@nyldn-plugins
```

**檔案路徑**: `package.json`
```json
{
  "name": "@anthropic-plugins/claude-octopus",
  "version": "8.53.0",
  "main": "scripts/orchestrate.sh"
}
```

`main` 指向 `scripts/orchestrate.sh`，這是整個系統的核心引擎。

### 2.2 指令觸發機制

使用者有三種方式觸發:

1. **斜線指令**: `/octo:embrace`, `/octo:discover`, `/octo:factory` 等
2. **自然語言路由**: 說 "octo research X" 或 "octo build Y"，由智慧路由 (`auto` 子指令) 解析意圖並分派
3. **MCP Server**: 透過 `.mcp.json` 自動啟動 Node.js MCP Server，公開 10 個工具

**檔案路徑**: `.mcp.json`
```json
{
  "mcpServers": {
    "octo-claw": {
      "command": "node",
      "args": ["./mcp-server/dist/index.js"],
      "env": {
        "CLAUDE_OCTOPUS_MCP_MODE": "true"
      }
    }
  }
}
```

### 2.3 命令前綴鎖定

**檔案路徑**: `.claude-plugin/PLUGIN_NAME_LOCK.md`

外掛名稱必須是 `"octo"`（不是 `"claude-octopus"`），因為 Claude Code 的指令路徑形成規則為 `/[plugin-name]:[command-name]`。這個約束有專門的測試保護 (`make test-plugin-name`) 和 pre-commit 鉤子。

---

## 3. 核心架構

### 3.1 設計哲學: Double Diamond

**檔案路徑**: `.claude/skills/skill-parallel-agents.md`

採用英國設計委員會的 Double Diamond 方法論，將所有任務分為四個階段:

```
  DISCOVER (probe)    DEFINE (grasp)     DEVELOP (tangle)    DELIVER (ink)

    \         /     \         /     \         /     \         /
     \   *   /       \   *   /       \   *   /       \   *   /
      \ * * /         \     /         \ * * /         \     /
       \   /           \   /           \   /           \   /
        \ /             \ /             \ /             \ /

   發散後收斂        收斂到問題        發散出方案        收斂到交付
```

| 階段 | 內部代號 | 功能 |
|------|---------|------|
| Discover | `probe` | 多 AI 平行研究 |
| Define | `grasp` | 共識建立 |
| Develop | `tangle` | Map-Reduce 實作 + 品質閘門 |
| Deliver | `ink` | 對抗式審查 + 最終交付 |
| 全部四階段 | `embrace` | 完整 Double Diamond |

### 3.2 orchestrate.sh -- 核心引擎

**檔案路徑**: `scripts/orchestrate.sh` (約 19,000 行)

這是整個系統的大腦。關鍵功能包括:

```bash
# 智慧路由
orchestrate.sh auto "research OAuth patterns"       # -> probe
orchestrate.sh auto "build user login"               # -> tangle + ink
orchestrate.sh auto "review the auth code"           # -> ink

# Double Diamond 階段
orchestrate.sh probe "Research topic"
orchestrate.sh grasp "Define requirements"
orchestrate.sh tangle "Implement feature"
orchestrate.sh ink "Validate and deliver"
orchestrate.sh embrace "Full lifecycle"

# 對抗式審查
orchestrate.sh grapple "implement JWT auth"          # Codex vs Gemini 辯論
orchestrate.sh squeeze "review auth.ts"              # 紅隊 vs 藍隊

# Dark Factory (自動管線)
orchestrate.sh factory --spec "spec.md"

# 供應商偵測
orchestrate.sh detect-providers
```

**關鍵內部函式** (根據 `.coderabbit.yaml` 和文件推斷):

| 函式 | 功能 |
|------|------|
| `detect_claude_code_version()` | CC 版本偵測 + Factory AI 支援 |
| `build_provider_env()` | 供應商環境變數隔離 |
| `spawn_agent()` | 代理生成 (含 persona 套用) |
| `validate_model_allowed()` | 模型白名單驗證 |
| `get_agent_model()` / `_get_agent_model_raw()` | 模型選擇 (含 env var 優先序) |
| `apply_persona()` | 角色套用 (含 readonly 支援) |
| `apply_tool_policy()` | 工具存取控制 |
| `sanitize_external_content()` | 防注入 (隨機 hex nonce 邊界) |
| `aggregate_results()` | 結果合成 (Gemini 摘要，退化為拼接) |
| `tangle_develop()` | 任務分解 + 平行執行 |
| `map_reduce()` | Map-Reduce 模式 |
| `parse_factory_spec()` | 規格解析 |
| `factory_run()` | 完整自動管線 |
| `recommend_persona_agent()` | 基於正則的角色推薦 |

### 3.3 三供應商架構

**檔案路徑**: `CLAUDE.md`

```
┌─────────────────────────────────────────────────────────────────┐
│                      Claude Code (主程序)                       │
│                                                                 │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐        │
│  │   Codex CLI  │  │  Gemini CLI  │  │    Claude    │        │
│  │   (OpenAI)   │  │  (Google)    │  │  (Anthropic) │        │
│  │              │  │              │  │              │        │
│  │  🔴 實作深度  │  │  🟡 生態廣度  │  │  🔵 合成協調  │        │
│  │  程式碼模式   │  │  替代方案    │  │  品質閘門    │        │
│  │  技術分析     │  │  安全審查    │  │  共識建立    │        │
│  └──────────────┘  └──────────────┘  └──────────────┘        │
│                                                                 │
│  75% 共識閘門: 至少 3/4 供應商必須同意才能推進                   │
└─────────────────────────────────────────────────────────────────┘
```

**驗證方式**: OAuth (免費，使用現有訂閱) 或 API Key (按量計費)

| 供應商 | OAuth | API Key | 每次查詢成本 |
|--------|-------|---------|-------------|
| Codex | `codex login` (ChatGPT 訂閱) | `OPENAI_API_KEY` | $0.01-0.15 |
| Gemini | Google 帳號 | `GEMINI_API_KEY` | $0.01-0.03 |
| Perplexity | -- | `PERPLEXITY_API_KEY` | $0.01-0.05 |
| Claude | 內建 (Claude Code) | 內建 | 包含在訂閱中 |

### 3.4 品質閘門

**檔案路徑**: `.claude/references/validation-gates.md`

所有呼叫 `orchestrate.sh` 的技能必須遵循「驗證閘門模式」(Validation Gate Pattern):

1. **frontmatter 宣告**: `execution_mode: enforced`
2. **執行契約**: 嚴格的 4 步驟流程
3. **禁止替代**: 不得跳過 orchestrate.sh 直接執行
4. **產物驗證**: 檢查合成檔案是否存在

```yaml
# 範例 frontmatter
execution_mode: enforced
pre_execution_contract:
  - interactive_questions_answered
  - visual_indicators_displayed
validation_gates:
  - orchestrate_sh_executed
  - synthesis_file_exists
```

### 3.5 角色系統 (Personas)

**檔案路徑**: `agents/personas/security-auditor.md` (範例)

32 個特化角色，每個都有:

```yaml
---
name: security-auditor
description: "Expert security auditor..."
model: opus                              # 使用的模型等級
memory: project                          # 記憶範圍 (project / user)
tools: ["Read", "Glob", "Grep", "Bash", "Task(Explore)"]
when_to_use: |                           # 啟用條件
  - Security audits and vulnerability scanning
  - OWASP Top 10 compliance checks
avoid_if: |                              # 不適用場景
  - General code quality review (use code-reviewer)
examples:                                # 範例交互
  - prompt: "Audit this login endpoint"
    outcome: "SQL injection risk, XSS vectors..."
hooks:                                   # 角色專屬鉤子
  PostToolUse:
    - matcher:
        tool: Bash
      command: "${CLAUDE_PLUGIN_ROOT}/hooks/security-gate.sh"
---
```

**v8.53.0 新功能**: `readonly: true` frontmatter 可強制角色只讀（禁止 Write/Edit/Bash 修改）。

### 3.6 狀態管理

**檔案路徑**: `.claude/state/state-manager.md`

透過 `state-manager.sh` 實現跨會話持久化:

```json
{
  "version": "1.0.0",
  "project_id": "unique-hash",
  "current_workflow": "flow-develop",
  "current_phase": "develop",
  "decisions": [...],
  "blockers": [...],
  "context": {
    "discover": "researched auth patterns, chose JWT",
    "define": "user wants passwordless magic links",
    "develop": "implementing backend API first",
    "deliver": null
  },
  "metrics": {
    "phases_completed": 2,
    "total_execution_time_minutes": 45,
    "provider_usage": { "codex": 12, "gemini": 10, "claude": 25 }
  }
}
```

支援: 原子寫入、JSON 驗證、自動備份、損壞恢復。

### 3.7 反應引擎 (v8.45.0)

**檔案路徑**: `scripts/reactions.sh`

自動回應 Agent 生命週期事件:

| 事件 | 反應 | 限制 |
|------|------|------|
| CI 失敗 | 收集日誌到 Agent 收件箱 | 3 次重試，30 分鐘後升級 |
| 變更要求 | 收集審查評論 | 2 次重試，60 分鐘後升級 |
| Agent 卡住 | 升級到人工 | 15 分鐘無進展 |
| PR 已核准 + CI 綠燈 | 通知可合併 | -- |
| PR 已合併 | 標記 Agent 完成 | -- |

追蹤 **13 個生命週期狀態**: `running` -> `pr_open` -> `ci_pending` -> `ci_failed` / `review_pending` -> `changes_requested` / `approved` -> `mergeable` -> `merged` -> `done`

### 3.8 功能旗標系統

透過 `detect_claude_code_version()` 偵測 Claude Code 版本，設定 86+ 個 `SUPPORTS_*` 變數:

```bash
SUPPORTS_VSCODE_PLAN_VIEW        # v2.1.70+
SUPPORTS_NATIVE_LOOP             # v2.1.71+
SUPPORTS_PERSISTENT_MEMORY       # v2.1.33+
SUPPORTS_FAST_OPUS               # v2.1.36+
SUPPORTS_OPUS_MEDIUM_EFFORT      # v2.1.68+
SUPPORTS_CONTINUATION            # v2.1.55+
SUPPORTS_HTTP_HOOKS              # v2.1.63+
SUPPORTS_SKILL_DEADLOCK_FIX      # v2.1.73+
# ... 共 86 個旗標
```

---

## 4. 整合點

### 4.1 外部服務

| 服務 | 介面 | 用途 |
|------|------|------|
| **OpenAI Codex CLI** | `codex exec --json` | 程式碼生成、技術分析 |
| **Google Gemini CLI** | `gemini` 命令 | 生態研究、替代方案、長上下文 |
| **Perplexity Sonar** | REST API | 網路搜尋 + 引用 |
| **OpenRouter** | REST API | 400+ 模型通用後備 |
| **Claude Code** | 內建 (Task/Skill tool) | 編排、合成、品質閘門 |
| **GitHub** | `gh` CLI | PR 評論、Agent 註冊表 |
| **CodeRabbit** | YAML 配置 | 自動程式碼審查 |
| **Factory AI (Droid)** | 外掛介面 | 雙平台相容 |

### 4.2 OpenClaw 相容層

**檔案路徑**: `openclaw/`, `mcp-server/`

三層架構實現跨平台:

```
Claude Code Plugin (不變)
  └── .mcp.json ─── MCP Server ─── orchestrate.sh
                                        ^
OpenClaw Extension ────────────────────┘
```

MCP Server 公開 10 個工具:
- `octopus_discover`, `octopus_define`, `octopus_develop`, `octopus_deliver`
- `octopus_embrace`
- `octopus_debate`, `octopus_review`, `octopus_security`
- `octopus_list_skills`, `octopus_status`

### 4.3 供應商認證

```bash
# Codex -- OAuth (推薦)
codex login

# Codex -- API Key
export OPENAI_API_KEY="sk-..."

# Gemini -- OAuth (推薦)
gemini  # 首次執行觸發 OAuth

# Gemini -- API Key
export GEMINI_API_KEY="AIza..."

# Perplexity -- API 專用
export PERPLEXITY_API_KEY="pplx-..."

# OpenRouter -- 通用後備
export OPENROUTER_API_KEY="sk-or-..."
```

---

## 5. 發布系統

### 5.1 版本管理

**檔案路徑**: `package.json`, `.claude-plugin/plugin.json`, `.claude-plugin/marketplace.json`, `README.md`

版本號同步在四個位置。`release.sh` (v8.22.6 引入) 自動化整個流程:

1. 更新版本號 (四個檔案)
2. 更新 CHANGELOG.md
3. Git commit + tag
4. Push + GitHub Release

### 5.2 版本號策略

採用 SemVer，但迭代極快:
- v8.22.0 (2026-02-22) -> v8.53.0 (2026-03-11) = **17 天 31 個版本**
- 每天平均 ~1.8 個版本

### 5.3 測試層級

**檔案路徑**: `Makefile`

```makefile
test: test-smoke test-unit                    # 預設: 快速回饋
test-all: test-smoke test-unit test-integration test-e2e

test-smoke: test-plugin-name                  # <30s (外掛名稱鎖定)
test-unit:                                    # 1-2min
test-integration:                             # 5-10min
test-e2e:                                     # 15-30min
test-live:                                    # 2-5min/test (真實 API 呼叫)
test-performance:                             # 效能基準
test-regression:                              # 回歸測試
```

### 5.4 CI/CD

- GitHub Actions: smoke -> unit -> integration -> e2e (級聯)
- 分支保護: `main` 要求 Smoke/Unit/Integration 通過
- Pre-push hook: 推送前執行完整測試套件
- CodeRabbit: 自動審查，阻擋合併直到評論解決

---

## 6. 值得採用的關鍵模式

### 6.1 驗證閘門模式 (Validation Gate Pattern)

**檔案路徑**: `.claude/references/validation-gates.md`

這是 Claude Octopus 最具創新性的模式。它解決了一個根本問題: **如何確保 LLM 確實執行預期的工作流程，而非偷懶跳過?**

```yaml
execution_mode: enforced
pre_execution_contract:
  - visual_indicators_displayed
validation_gates:
  - orchestrate_sh_executed
  - synthesis_file_exists
```

要點:
- 使用 **強制性語言** ("MUST", "PROHIBITED", "CANNOT SKIP")，而非建議性語言
- 透過 **檔案系統驗證** (檢查輸出檔案是否在 10 分鐘內建立) 而非信任 LLM 的自我報告
- 明確禁止替代行為的清單

**clawtex 應用**: 可以在 Hands 引擎中引入類似的驗證閘門，確保每個階段確實執行了預期的工具呼叫。

### 6.2 反注入 Nonce 機制

**檔案路徑**: `CHANGELOG.md` (v8.42.0)

```bash
sanitize_external_content()
# 用隨機 hex 邊界標記包裝外部內容
# 防止記憶體檔案、供應商歷史、earned skills 中的 prompt injection
```

**clawtex 應用**: clawtex-core 處理使用者透過 Telegram 輸入的內容時，應考慮類似的淨化機制，特別是在 memory_recall 和 delegate 工具中。

### 6.3 角色自動路由

基於正則表達式的意圖偵測，自動選擇最適合的角色:

```bash
# scripts/lib/routing.sh
# 使用複合正則避免誤判
# 例: 使用 marketing.?strategy 而非裸露的 marketing
recommend_persona_agent()
```

**clawtex 應用**: clawtex-core 的 `classifier.rs` (Simple/Medium/Complex) 可以擴展為更細緻的角色路由，類似 Octopus 的 32 角色模型。

### 6.4 漸進降級 (Graceful Degradation)

供應商不可用時的處理策略:

```
三個供應商 -> 完整多 AI 協作
兩個供應商 -> 雙模型對比
一個供應商 -> 單模型 + 多輪查詢
零個外部 -> Claude-only 模式 (仍有角色、工作流、技能)
```

**clawtex 應用**: clawtex-core 的 `ReliableProvider` + `ProviderRouter` 已有類似機制，但 Octopus 的 "零外部仍可用" 設計值得學習。

### 6.5 對抗式審查 (Crossfire)

```
GRAPPLE (辯論):
  Codex 提案 <-> Gemini 提案
  Gemini 評論 <-> Codex 評論
  -> 合成 (勝者 + 修正)

SQUEEZE (紅隊):
  藍隊 (Codex) 實作安全方案
  紅隊 (Gemini) 攻擊找漏洞
  -> 修復 + 驗證
```

**clawtex 應用**: clawtex-core 可以在 `delegate_to_provider` 工具中實現類似的對抗模式，讓不同供應商互相審查。

### 6.6 Dark Factory 模式

**檔案路徑**: `.claude/commands/factory.md`

規格輸入 -> 軟體輸出的全自動管線:

```
解析規格 -> 生成測試場景 -> 80/20 切分 -> 四階段實作 -> 盲測 -> 評分 -> 報告
```

評分權重: 行為 40% + 約束 20% + 盲測 25% + 品質 15%
判定: PASS (>= 目標) / WARN (>= 目標 - 0.05) / FAIL

**clawtex 應用**: clawtex-core 的 Hands 引擎 (`product_spec -> code_gen -> saas_deploy` 鏈) 已有類似概念，但 Octopus 的 holdout testing (保留測試) 和 satisfaction scoring (滿意度評分) 值得參考。

### 6.7 Session State 跨會話持久化

**檔案路徑**: `.claude/state/state-manager.md`

透過 JSON 檔案持久化:
- 當前工作流階段
- 已做決策 (含理由)
- 阻礙清單
- 每階段上下文摘要
- 供應商使用指標

支援原子寫入、備份、損壞偵測與恢復。

**clawtex 應用**: clawtex-core 已有 `src/hands/` 階段追蹤，但缺乏跨會話決策追蹤。可以考慮類似的 state.json 機制。

### 6.8 功能旗標驅動的版本適配

86 個 `SUPPORTS_*` 旗標讓 Octopus 能夠:
- 在舊版 Claude Code 上優雅降級
- 自動啟用新功能
- 在 `/octo:doctor` 中提供精確的診斷建議

**clawtex 應用**: clawtex-core 處理不同 Ollama/LLM 版本差異時可借鑑此模式。

### 6.9 視覺指示器系統

**檔案路徑**: `CLAUDE.md`

```
🐙 CLAUDE OCTOPUS ACTIVATED - Multi-provider research mode
🔍 Discover Phase: Researching OAuth authentication patterns

Providers:
🔴 Codex CLI - Technical implementation analysis
🟡 Gemini CLI - Ecosystem and community research
🔵 Claude - Strategic synthesis
```

每次操作前顯示:
- 正在使用哪些供應商
- 成本影響
- 當前階段

**clawtex 應用**: clawtex-core 的 Telegram 介面可以引入類似的視覺回饋，讓使用者知道正在使用哪個供應商和模型。

### 6.10 Stub 偵測模式

**檔案路徑**: `.claude/references/stub-detection.md`

四層驗證確保程式碼完整性:

| 層級 | 檢查內容 |
|------|---------|
| Level 1: Exists | 檔案存在且非空 |
| Level 2: Substantive | 有實質實作，無 stub 模式 |
| Level 3: Wired | 被其他模組匯入/使用 |
| Level 4: Functional | 可執行，通過測試 |

偵測模式: TODO/FIXME、空函式、return null、mock 資料、console.log stub、有註解掉的邏輯。

---

## 7. 與 clawtex-core 的相關性分析

### 7.1 架構對比

| 面向 | Claude Octopus | clawtex-core |
|------|---------------|--------------|
| 語言 | Bash (~19K行) + Markdown | Rust (~6K行+) |
| 宿主 | Claude Code 外掛 | 獨立 daemon |
| 使用者介面 | Claude Code CLI | Telegram Bot |
| 供應商數量 | 3 (Codex/Gemini/Claude) + OpenRouter | 10+ (Ollama/Anthropic/OpenAI/Gemini/Groq/ChatGPT等) |
| 工作流引擎 | Double Diamond (4 階段) | Hands 引擎 (多階段 TOML) |
| 角色系統 | 32 個 Markdown 角色 | agents.toml 配置 |
| 工具數量 | 透過 Claude Code 內建 | 24 個自訂工具 |
| 測試 | Bash 腳本 (89+) | Rust (707+) |
| 叢集 | 無 (單機) | ClusterHub + ClusterWorker |
| 安全 | 輸入驗證 + nonce | ChaCha20-Poly1305 加密 + 核准閘門 |

### 7.2 可直接借鑑的模式

#### (1) 驗證閘門 -> Hands 階段驗證

**優先級: 高**

在 clawtex-core 的 `src/hands/mod.rs` 中，可以為每個階段添加驗證閘門，確認:
- 預期的工具確實被呼叫
- 輸出檔案確實產生
- 品質分數達到閾值

```rust
// 概念: 在 HandRunner::run_phase() 中
struct ValidationGate {
    required_tool_calls: Vec<String>,
    required_artifacts: Vec<PathBuf>,
    quality_threshold: f32,
}
```

#### (2) 對抗式審查 -> 多供應商交叉驗證

**優先級: 中**

利用 clawtex-core 已有的 `delegate_to_provider` 工具，實現類似 grapple/squeeze 的模式:
- 讓 Ollama 本地模型提出方案
- 讓 Anthropic/Gemini 進行對抗審查
- 在 Hands 階段間實現自動的 "紅隊 vs 藍隊" 模式

#### (3) 反注入 Nonce -> Memory/Delegate 安全

**優先級: 高**

clawtex-core 的 `memory_recall` 和 `delegate` 工具處理的內容可能包含惡意 prompt injection。可以借鑑 Octopus 的 `sanitize_external_content()` 機制:

```rust
fn sanitize_content(content: &str) -> String {
    let nonce = generate_hex_nonce();
    format!("---BEGIN-EXTERNAL-{nonce}---\n{}\n---END-EXTERNAL-{nonce}---", content)
}
```

#### (4) 視覺指示器 -> Telegram 回饋增強

**優先級: 中**

在 Telegram 回應中加入供應商/模型指示:

```
🔷 使用 Ollama (qwen3:8b) 本地推理中...
🟢 切換到 Gemini (gemini-2.5-flash) 進行視覺分析...
🟡 使用 Groq (llama-4-scout) 快速回應...
```

#### (5) 功能旗標 -> 供應商能力偵測

**優先級: 低**

在 clawtex-core 的 Provider trait 中添加能力查詢:

```rust
trait Provider {
    fn capabilities(&self) -> ProviderCapabilities;
    // supports_vision, supports_function_calling,
    // supports_streaming, max_context_window, etc.
}
```

#### (6) Dark Factory -> 全自動 SaaS 管線增強

**優先級: 中**

clawtex-core 已有 `product_spec -> code_gen -> saas_deploy` 鏈，但可以借鑑 Octopus 的:
- **Holdout testing**: 隨機保留 20% 測試場景做盲測
- **Satisfaction scoring**: 多維度加權評分
- **自動重試**: 失敗時帶修復上下文重新執行

### 7.3 不適合直接借鑑的部分

| 模式 | 原因 |
|------|------|
| 19K 行單檔 Bash | clawtex-core 的 Rust 模組化更優 |
| Claude Code 外掛機制 | clawtex-core 是獨立 daemon，不需要 |
| Markdown 角色定義 | clawtex-core 的 TOML agents.toml 更結構化 |
| `${CLAUDE_PLUGIN_ROOT}` 路徑系統 | clawtex-core 用 Rust 模組匯入 |
| 瀏覽器為基礎的 OAuth | clawtex-core 用 API key / 檔案認證 |

---

## 附錄 A: 關鍵檔案路徑索引

| 用途 | 檔案 |
|------|------|
| 專案概述 | `README.md` |
| 系統指令 | `CLAUDE.md` |
| 核心引擎 | `scripts/orchestrate.sh` |
| 外掛配置 | `.claude-plugin/settings.json` |
| 名稱鎖定 | `.claude-plugin/PLUGIN_NAME_LOCK.md` |
| 主要技能 | `.claude/skills/skill-parallel-agents.md` |
| 驗證閘門 | `.claude/references/validation-gates.md` |
| Stub 偵測 | `.claude/references/stub-detection.md` |
| 狀態管理 | `.claude/state/state-manager.md` |
| 角色範例 | `agents/personas/security-auditor.md` |
| 開發指南 | `.claude/DEVELOPMENT.md` |
| 安全政策 | `SECURITY.md` |
| 測試框架 | `Makefile` |
| 版本歷程 | `CHANGELOG.md` |
| CodeRabbit | `.coderabbit.yaml` |
| MCP 配置 | `.mcp.json` |

## 附錄 B: 版本演進時間線

```
v8.22.0 (02-22) -- OpenClaw 相容層
v8.25.0 (02-24) -- Dark Factory 自動管線
v8.27.0 (02-26) -- Context Compaction 存活 + XML 強制標籤
v8.33.0 (03-04) -- UI/UX 設計 + BM25 設計智慧
v8.36.0 (03-05) -- Factory AI 雙平台支援
v8.37.0 (03-05) -- Perplexity Sonar 網路搜尋
v8.39.0 (03-05) -- GPT-5.4 模型支援
v8.41.0 (03-07) -- 3 新鉤子、10 原生 Agent、自動記憶
v8.42.0 (03-08) -- 強制合規、反注入 nonce、辯論閘門
v8.43.0 (03-08) -- 品質注入、BM25 自動注入、參考完整性閘門
v8.44.0 (03-09) -- Agent 註冊表、Worktree 隔離、PR 評論
v8.45.0 (03-09) -- 反應引擎、13 狀態 PR 生命週期
v8.49.0 (03-10) -- 相關性感知合成
v8.53.0 (03-11) -- readonly 角色、使用者範圍 Agent、Agent 續接
```

---

*本分析基於 Claude Octopus v8.53.0 的公開原始碼。所有檔案路徑均相對於 `references/claude-octopus/` 目錄。*
