# Claude Octopus 架構文檔

## 1. 專案概覽

**Claude Octopus** 是一個 Claude Code 插件（v8.53.0），實現雙鑽石 (Double Diamond) 方法論的多 AI 編排系統。它將 Codex、Gemini 和 Claude 三個模型編排為特定角色（實現深度、生態廣度、綜合），實施 75% 共識門來防止單一模型的盲點。支持 32 個專業人設、39 個命令、50+ 技能，無需外部提供者即可工作，可選與 Codex 和 Gemini 集成以解鎖多 AI 功能。

**核心設計：**
- **Double Diamond 方法論：** Discover → Define → Develop → Deliver (4 個清晰階段)
- **角色分配：** Codex (實現) vs Gemini (生態) vs Claude (綜合)
- **品質門：** 75% 共識通過、跨模型驗證、質量注入
- **自主流程：** Factory 模式完整自動化 (spec → shipping code)
- **人設系統：** 32 個專業人設 (security-auditor, backend-architect, ...)
- **命令路由：** 智能路由 (`/octo:octo <what-you-need>` → 自動選擇正確命令)

## 2. 目錄結構

```
claude-octopus/
├── .claude/                    # [P0] Claude Code 插件配置
│   ├── DEVELOPMENT.md          # 開發指南
│   ├── CLAUDE.md               # 系統指令 (visual indicators, file creation policy)
│   ├── settings.json           # 插件設定
│   │
│   ├── commands/               # [P0] 39 個命令定義
│   │   ├── brainstorm.md       # 腦力激盪 (Discover)
│   │   ├── claw.md             # CLAW 流程 (全自動)
│   │   ├── debate.md           # AI 辯論 (3 方共識)
│   │   ├── debug.md            # 調試 (Develop)
│   │   ├── deck.md             # 簡報生成 (Deliver)
│   │   ├── design.md           # UI/UX 設計 (BM25 風格智能)
│   │   ├── develop.md          # 開發階段 (全流程)
│   │   ├── discover.md         # 探索階段 (multi-source research)
│   │   ├── embrace.md          # 全 4 階段 (Discover→Define→Dev→Deliver)
│   │   ├── extract.md          # 知識提取
│   │   ├── factory.md          # 自主工廠模式 (spec → code)
│   │   ├── prd.md              # 產品規格 (100 分評分)
│   │   ├── quick.md            # 快速單動 (不走流程)
│   │   ├── research.md         # 深度研究
│   │   ├── review.md           # 代碼審查 (Deliver)
│   │   ├── security.md         # OWASP 安全掃描
│   │   ├── tdd.md              # 測試驅動開發
│   │   ├── pipeline.md         # 流程管道執行
│   │   ├── parallel.md         # 並聯代理執行
│   │   ├── scheduler.md        # 排程與自動化
│   │   ├── sentinel.md         # 監哨 (CI/CD 故障響應)
│   │   ├── staged-review.md    # 分階段審查
│   │   ├── doctor.md           # 健康檢查
│   │   ├── loop.md             # 自動迴圈
│   │   ├── meta-prompt.md      # Meta 提示生成
│   │   ├── multi.md            # 多角色任務
│   │   ├── spec.md             # 規格生成
│   │   ├── docs.md             # 文檔自動生成
│   │   ├── prd-score.md        # 規格品質評分
│   │   └── ... (共 39 個)
│   │
│   ├── skills/                 # [P1] 50+ 技能
│   │   ├── skill-architecture.md        # 架構分析
│   │   ├── skill-adversarial-security.md # 對抗性安全
│   │   ├── skill-claw.md               # CLAW (4 層)
│   │   ├── skill-context-detection.md  # 上下文偵測
│   │   ├── skill-debate-integration.md # 辯論整合
│   │   ├── skill-decision-support.md   # 決策支持
│   │   ├── skill-deck.md               # 簡報技能
│   │   ├── skill-doc-delivery.md       # 文檔交付
│   │   ├── skill-parallel-agents.md    # 並聯代理
│   │   ├── skill-prd.md                # PRD 生成
│   │   ├── skill-quick.md              # 快速技能
│   │   ├── skill-security-audit.md     # 安全審計
│   │   ├── skill-security-framing.md   # 安全框架
│   │   ├── skill-visual-feedback.md    # 視覺反饋
│   │   ├── skill-writing-plans.md      # 計劃編寫
│   │   └── ... (共 50+ 個)
│   │
│   ├── state/                  # [P2] 狀態管理
│   │   └── state-manager.md    # Agent 狀態追蹤
│   │
│   ├── references/             # [P2] 參考資料
│   │   ├── stub-detection.md   # 虛設函數偵測
│   │   └── validation-gates.md # 品質門驗證
│   │
│   ├── hooks/                  # [P2] 事件掛鉤
│   │   ├── pre-commit.sh       # Git commit 掛鉤
│   │   └── visual-feedback.sh  # 視覺反饋
│   │
│   └── config/                 # [P2] 配置模組
│       ├── providers/
│       │   ├── codex/CLAUDE.md
│       │   ├── gemini/CLAUDE.md
│       │   └── claude/CLAUDE.md
│       └── workflows/CLAUDE.md
│
├── agents/                     # [P1] 32 個人設
│   └── personas/
│       ├── academic-writer.md
│       ├── ai-engineer.md
│       ├── backend-architect.md
│       ├── business-analyst.md
│       ├── cloud-architect.md
│       ├── code-reviewer.md
│       ├── database-architect.md
│       ├── debugger.md
│       ├── deployment-engineer.md
│       ├── devops-troubleshooter.md
│       ├── frontend-developer.md
│       ├── graphql-architect.md
│       ├── security-auditor.md
│       ├── ui-ux-designer.md
│       ├── performance-engineer.md
│       └── ... (共 32 個)
│
├── docs/                       # [P2] 文檔
│   ├── COMMAND-REFERENCE.md    # 39 個命令完整參考
│   ├── FACTORY-AI.md           # Factory AI (Droid) 支持
│   ├── CLAUDE-CODE.md          # Claude Code 特定說明
│   └── CHANGELOG.md            # 變更日誌
│
├── .claude-plugin/             # [P2] 插件打包
│   ├── PLUGIN_NAME_LOCK.md     # 插件識別
│   └── settings.json           # 插件設定
│
├── CONTRIBUTING.md             # 貢獻指南
├── LICENSE                     # MIT 授權
├── Makefile                    # 開發命令
├── SECURITY.md                 # 安全政策
└── README.md                   # 快速開始
```

## 3. 核心模組詳解

### 3.1 Double Diamond 方法論

```
┌─────────────────────────────────────────────────────────┐
│                   PROBLEM SPACE                         │
│  ╱─────────────────────────────────────────────────╲   │
│ │   Discover (寬)      │      Define (焦點)        │   │
│ │                      │                           │   │
│ │ 🔍 多源研究          │ 🎯 需求精準              │   │
│ │ • Codex & Gemini     │ • 協商共識                │   │
│ │   並聯搜尋           │ • 75% 品質門              │   │
│ │ • Claude 綜合        │ • PRD 生成                │   │
│ │ • 發現盲點           │ • 範圍確認                │   │
│  ╲─────────────────────────────────────────────────╱   │
│                                                         │
│                   SOLUTION SPACE                        │
│  ╱─────────────────────────────────────────────────╲   │
│ │    Develop (寬)      │      Deliver (焦點)       │   │
│ │                      │                           │   │
│ │ 🛠️ 實現              │ ✅ 交付                   │   │
│ │ • TDD / 測試優先     │ • 審查與驗證              │   │
│ │ • Codex 代碼生成     │ • Holdout 測試            │   │
│ │ • 並聯編寫           │ • 滿意度評分              │   │
│ │ • 集成測試           │ • 文檔與簡報              │   │
│  ╲─────────────────────────────────────────────────╱   │
│                                                         │
└─────────────────────────────────────────────────────────┘

命令映射:
  /octo:embrace    → 全 4 階段自動執行
  /octo:discover   → Discover 重點
  /octo:define     → Define 重點
  /octo:develop    → Develop 重點
  /octo:deliver    → Deliver 重點 (Review)
  /octo:factory    → 完全自主 (所有 4 階段 + 自動化)
```

### 3.2 提供者編排

```
任務分類 (Complexity Scoring)
    ↓
    ├─ Simple (token < 500)
    │   └─ Claude 單獨處理
    │
    ├─ Complex (token 500-2000)
    │   └─ Claude + Codex OR Claude + Gemini
    │
    └─ Large (token > 2000)
        └─ 3 方完整編排

3 方編排流程
┌──────────────────────────────────────────────────┐
│                                                  │
│  並聯執行 (Promise.all)                         │
│  ├─ 🔴 Codex    → 實現細節 (code generation)   │
│  ├─ 🟡 Gemini   → 生態視角 (alternatives)      │
│  └─ 🔵 Claude   → 綜合 (synthesis + gates)     │
│                                                  │
│  結果聚合                                        │
│  ├─ Markdown 比較表                            │
│  ├─ 分項評分                                    │
│  └─ 共識決策 (75% threshold)                   │
│                                                  │
│  品質門驗證                                      │
│  ├─ Reference Integrity (引用檢查)             │
│  ├─ Stub Detection (虛設偵測)                  │
│  ├─ Adversarial Review (對抗審查)             │
│  └─ Execution Enforcement (強制執行)           │
│                                                  │
└──────────────────────────────────────────────────┘

成本感知:
  🔴 Codex (GPT-5.4): $2.50/$15 MTok
  🟡 Gemini: $0.01-0.03 per query
  🔵 Claude Sonnet: 包含於 Claude Code 訂閱
  🔵 Claude Opus: $5/$25 MTok (或 Fast $30/$150 Extra)
```

### 3.3 人設系統 (Personas)

```
Persona 架構:
┌─ academic-writer      → 學術寫作
├─ ai-engineer          → AI/ML 工程
├─ backend-architect    → 後端架構
├─ business-analyst     → 商業分析
├─ cloud-architect      → 雲架構
├─ code-reviewer        → 代碼審查
├─ database-architect   → 資料庫架構
├─ debugger             → 調試
├─ deployment-engineer  → 部署工程
├─ devops-troubleshooter → DevOps 故障排查
├─ frontend-developer   → 前端開發
├─ graphql-architect    → GraphQL 架構
├─ incident-responder   → 事件回應
├─ mermaid-expert       → Mermaid 圖表
├─ openclaw-admin       → OpenClaw 管理
├─ performance-engineer → 性能工程
├─ product-writer       → 產品寫作
├─ security-auditor     → OWASP 安全
├─ ui-ux-designer       → UI/UX (BM25 風格)
├─ ... (共 32 個)

自動選擇:
  用戶: "audit my API"
    ↓
  智能路由器 (Smart Router)
    ├─ 解析意圖: "security" + "api"
    ├─ 載入 Persona: security-auditor
    ├─ 選擇命令: /octo:security
    └─ 應用系統提示
```

### 3.4 品質門系統

```
Reference Integrity Gate
    ├─ 檢查代碼中所有引用
    ├─ 驗證函數/類別存在
    ├─ 確保 import 路徑正確
    └─ Fail: 拒絕前進

Stub Detection Gate
    ├─ 偵測虛設函數 (pass / ...)
    ├─ 檢查 TODO 備註
    ├─ 驗證實現完整性
    └─ Warn: 提示需要完成

Adversarial Review Gate (3 方)
    ├─ 🔴 Codex: 代碼品質? 是否有邊界情況?
    ├─ 🟡 Gemini: 性能? 安全隱患?
    └─ 🔵 Claude: 一致性? 設計決策?

    共識檢查:
    ├─ 全部同意 (3/3) → PASS
    ├─ 多數同意 (2/3) → PASS (記錄異議)
    └─ 多數反對 → FAIL (返回開發)

Execution Enforcement
    ├─ 不能跳過品質門
    ├─ 無法返回上一階段
    ├─ 必須顯示視覺指標
    └─ 強制使用 Validation Gate Pattern
```

## 4. 啟動流程

### 4.1 插件安裝與設置

```
用戶: /plugin marketplace add https://github.com/nyldn/claude-octopus
    ↓
Claude Code 市場下載
    ↓
/plugin install claude-octopus@nyldn-plugins
    ↓
插件初始化:
    ├─ 複製 .claude/ 到 ~/.claude/plugins/claude-octopus/
    ├─ 載入 settings.json
    ├─ 檢測已安裝提供者 (claude, codex, gemini)
    └─ 生成 ~/octopus-config.toml
    ↓
/octo:setup
    ├─ 提示提供者檢測結果
    ├─ 配置 OPENAI_API_KEY (for Codex)
    ├─ 配置 GEMINI_API_KEY (for Gemini)
    ├─ 測試連接
    └─ 儲存配置
```

### 4.2 命令執行流程 (例: /octo:embrace)

```
用戶: /octo:embrace build user authentication
    ↓
Smart Router (command.md)
    ├─ 解析意圖: "build" → Develop phase
    ├─ 檢測 embrace 模式 (4 phase)
    ├─ 載入對應的 skill
    └─ 執行 embrace.md
    ↓
Phase 1: Discover
    ├─ 系統提示: "research user auth patterns"
    ├─ 啟動多源研究:
    │   ├─ 🔴 Codex: "auth implementations"
    │   ├─ 🟡 Gemini: "industry standards"
    │   └─ 🔵 Claude: "综合分析"
    ├─ 聚集結果
    └─ 品質門: Reference Integrity
    ↓
Phase 2: Define
    ├─ 系統提示: "define requirements"
    ├─ 共識決定:
    │   ├─ 認證方式 (JWT vs OAuth)
    │   ├─ 安全級別
    │   └─ 集成點
    ├─ 生成 PRD (100 分評分)
    └─ 品質門: 75% 共識
    ↓
Phase 3: Develop
    ├─ TDD: 先寫測試
    ├─ 並聯代碼生成:
    │   ├─ 🔴 Codex: "generate auth service"
    │   ├─ 🟡 Gemini: "alternative approach"
    │   └─ 🔵 Claude: "review + integrate"
    ├─ 測試驅動迭代
    └─ 品質門: Stub Detection + Adversarial
    ↓
Phase 4: Deliver
    ├─ 最終審查
    ├─ Holdout 測試 (新測試用例)
    ├─ 文檔生成
    ├─ 簡報生成 (deck)
    └─ 品質門: 執行強制
    ↓
結果:
    ├─ 代碼 + 測試
    ├─ 文檔
    ├─ 簡報
    ├─ 滿意度評分
    └─ 決策日誌 (JSONL)
```

## 5. 資料流 ASCII 圖

### 5.1 3 方編排流程

```
用戶提示
    ↓
Claude Octopus 分析
    │
    ├─→ 複雜度評分
    │    ├─ 簡單 → Claude 單獨
    │    ├─ 中等 → Claude + 1 提供者
    │    └─ 複雜 → 3 方全開
    │
    ├─→ Persona 自動偵測
    │    └─ 載入 (security-auditor, backend-architect, ...)
    │
    └─→ 並聯執行
        │
        ├─ 🔴 Codex 處理
        │   ├─ 實現代碼
        │   ├─ 邊界情況
        │   └─ 完成度檢查
        │
        ├─ 🟡 Gemini 處理
        │   ├─ 生態選項
        │   ├─ 性能考量
        │   └─ 安全分析
        │
        ├─ 🔵 Claude 綜合
        │   ├─ 閱讀三份輸出
        │   ├─ 共識決定
        │   └─ 品質門檢查
        │
        └─ 結果聚集
            ├─ 比較表
            ├─ 共識評分
            └─ 最終建議
```

### 5.2 Factory 自主流程

```
用戶: /octo:factory "build CSV to JSON CLI"
    ↓
Spec Generation (自動 Discover + Define)
    ├─ 分析需求
    ├─ 生成 PRD
    └─ 確認需求清單
    ↓
Autonomous Develop
    ├─ 並聯代碼生成
    │   ├─ 🔴 Codex → 主實現
    │   ├─ 🟡 Gemini → 替代方案
    │   └─ 🔵 Claude → 審查 + 選擇
    │
    ├─ TDD 迴圈
    │   ├─ 自動生成測試
    │   ├─ 檢查覆蓋率
    │   └─ 迭代改善
    │
    ├─ 集成測試
    │   └─ Holdout 測試集
    │
    └─ 性能驗證
        └─ 基準測試
        ↓
    Deliver (自動)
        ├─ 文檔生成
        ├─ API 規格
        ├─ 簡報生成
        └─ 部署指南
        ↓
完整產品 (ready-to-ship)
    ├─ 代碼 + 測試 (100% 覆蓋)
    ├─ 文檔
    ├─ 簡報
    └─ 滿意度評分: 92/100
```

## 6. 子系統清單

### 6.1 P0 優先級 (核心)

| 子系統 | 功能 | 檔案 | 狀態 |
|--------|------|------|------|
| Smart Router | 意圖→命令映射 | `commands/` | ✅ |
| 39 Commands | 所有工作流 | `commands/*.md` | ✅ |
| Double Diamond | 4 階段方法論 | `flow-*.md` | ✅ |
| Provider Manager | 3 方編排 | `providers/` | ✅ |
| Complexity Scorer | 任務分類 | `skill-*.md` | ✅ |

### 6.2 P1 優先級 (重要功能)

| 子系統 | 功能 | 檔案 | 狀態 |
|--------|------|------|------|
| 32 Personas | 專業人設 | `agents/personas/` | ✅ |
| 50+ Skills | 技能工具庫 | `skills/` | ✅ |
| Consensus Gate | 75% 品質門 | `validation-gates.md` | ✅ |
| TDD Framework | 測試驅動 | `skill-tdd.md` | ✅ |
| Factory AI | 完全自主 | `commands/factory.md` | ✅ |
| Factory Droid | Droid 支持 | `docs/FACTORY-AI.md` | ✅ |

### 6.3 P2 優先級 (增強功能)

| 子系統 | 功能 | 檔案 | 狀態 |
|--------|------|------|------|
| Stub Detection | 虛設偵測 | `references/stub-detection.md` | ✅ |
| Reference Integrity | 引用檢查 | `validation-gates.md` | ✅ |
| Decision Logging | JSONL 決策記錄 | (auto-generated) | ✅ |
| State Manager | Agent 追蹤 | `state/state-manager.md` | ✅ |
| Reaction Engine | CI/CD 回應 | (v8.45+) | ✅ |
| PR Integration | PR 評論發佈 | (v8.44+) | ✅ |
| BM25 Style Intel | UI 設計 (320+ 規則) | `agents/personas/ui-ux-designer.md` | ✅ |
| Visual Feedback | 指標顯示 | `hooks/visual-feedback.sh` | ✅ |

### 6.4 功能矩陣

| 功能 | 僅 Claude | + Codex | + Gemini | 3 方全開 |
|------|-----------|---------|----------|---------|
| Discover | ✅ | ✅ | ✅ | ✅ (最佳) |
| Define | ✅ | ✅ | ✅ | ✅ (共識) |
| Develop | ✅ | ✅ (代碼) | ✅ | ✅ (選擇最優) |
| Deliver | ✅ | ✅ | ✅ | ✅ (3 方審查) |
| Factory | ✅ | ✅✅ (快速) | ✅ | ✅✅ (最優) |
| Cost | $5/$25 MTok | + API | + API | 3 x cost |

## 7. 技術棧

- **Framework:** Claude Code Plugin SDK
- **Language:** Markdown (CLAUDE.md) + Shell Scripts
- **Method:** Double Diamond + Adversarial Debate
- **Providers:** Claude, Codex (OpenAI), Gemini (Google), Factory AI (Droid)
- **Logging:** JSONL 決策記錄
- **Testing:** Vitest (for automation)

## 8. 視覺指標強制

**MANDATORY:** 每個工作流必須顯示:

```
🐙 **CLAUDE OCTOPUS ACTIVATED** - [Workflow Type]
🔍 Discover Phase: Researching OAuth patterns

Providers:
🔴 Codex CLI - Technical implementation
🟡 Gemini CLI - Ecosystem research
🔵 Claude - Strategic synthesis
```

**成本展示:**
- 🔴 Codex: 標示 (外部 API 成本)
- 🟡 Gemini: 標示 (外部 API 成本)
- 🔵 Claude Opus Fast: ⚠️ 特別警告 (6x 更昂貴)

## 9. 檔案建立政策

**禁止：** 在插件目錄中建立:
- `PHASE*_PROGRESS.md`
- `*_PROGRESS.md`
- `*_TODO.md`
- 任何臨時工作檔案

**使用：** `~/.claude/scratchpad/[session-id]/` 用於工作檔案

## 10. 開發工作流

```bash
# 測試本地設置
/octo:setup

# 快速研究 (測試)
/octo:quick "explain CQRS"

# 完整流程
/octo:embrace "build notification system"

# Factory 自主
/octo:factory "build React component library"

# 查看決策日誌
/octo:review [output_file]
```

## 11. 版本資訊

- **當前版本:** 8.53.0
- **發佈日期:** 2025-03-13
- **新功能:** readonly personas、user-scope agents、/octo:resume、Reaction Engine 增強
- **相容性:** Claude Code v2.1.50+、Factory AI Droid
