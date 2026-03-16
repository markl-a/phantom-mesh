# CLI Agent Orchestrator 架構文檔

## 1. 專案概覽

**CLI Agent Orchestrator (CAO)** 是一個 Python 工具編排框架，用於在 tmux 終端中併聯式管理多個 AI CLI 代理（Kiro CLI、Claude Code、Codex、Amazon Q CLI）。它提供統一的 CLI 介面、MCP 伺服器支持和 HTTP API，讓使用者可以在一個 tmux 會話中同時操作四種不同的 AI 提供者，並實現跨代理的工作流自動化。

**核心設計理念：**
- 可見性優先 (WYSIWYG)：所有互動都在分窗口中顯示，完全可控
- 狀態驅動：SQLite 儲存所有終端和信件狀態
- 非同步通信：終端間的輕量級訊息系統
- 多入口點：CLI、MCP Server、FastAPI HTTP API
- 代理無關：抽象化 Provider 層支持新代理快速集成

## 2. 目錄結構

```
cli-agent-orchestrator/
├── src/cli_agent_orchestrator/
│   ├── cli/                    # [P0] CLI 入口
│   │   ├── main.py             # 命令路由 (cao launch, cao info, etc.)
│   │   └── commands/
│   │       ├── launch.py       # 啟動 tmux 會話和終端
│   │       ├── info.py         # 顯示會話資訊
│   │       ├── mcp_server.py   # 啟動 MCP 伺服器
│   │       ├── flow.py         # 流程執行
│   │       ├── init.py         # 初始化資料庫
│   │       ├── install.py      # 安裝依賴
│   │       └── shutdown.py     # 停止服務
│   │
│   ├── mcp_server/             # [P0] MCP 伺服器入口
│   │   ├── server.py           # FastMCP 伺服器註冊 (handoff, send_message 工具)
│   │   └── models.py           # HandoffResult 資料模型
│   │
│   ├── api/                    # [P0] HTTP API 入口
│   │   └── main.py             # FastAPI 應用 (port 9889)
│   │
│   ├── services/               # [P1] 商業邏輯層
│   │   ├── session_service.py  # 會話 CRUD (list, get, delete)
│   │   ├── terminal_service.py # 終端生命週期 (create, get, send_input, get_output, delete)
│   │   ├── inbox_service.py    # 終端間信件系統 (watchdog 檔案監控)
│   │   ├── flow_service.py     # 排程流程執行
│   │   └── cleanup_service.py  # 清理過期會話和終端
│   │
│   ├── providers/              # [P1] 代理實現層 (可擴展)
│   │   ├── base.py             # 抽象 Provider 介面 (mark_input_received hook)
│   │   ├── manager.py          # terminal_id → provider 映射
│   │   ├── kiro_cli.py         # Kiro CLI (預設)
│   │   ├── claude_code.py      # Claude Code (❯ 提示, 信任提示)
│   │   ├── codex.py            # Codex/ChatGPT CLI (› 提示 + • 項目偵測)
│   │   ├── q_cli.py            # Amazon Q CLI
│   │   └── gemini_cli.py       # Gemini CLI
│   │
│   ├── clients/                # [P1] 外部系統
│   │   ├── tmux.py             # libtmux 包裝 (CAO_TERMINAL_ID 設置, 括號貼上)
│   │   └── database.py         # SQLite (terminals, inbox_messages 表)
│   │
│   ├── models/                 # [P2] 資料模型
│   │   ├── terminal.py         # Terminal, TerminalStatus
│   │   ├── session.py          # Session
│   │   ├── inbox.py            # InboxMessage, MessageStatus
│   │   ├── flow.py             # Flow (排程)
│   │   ├── agent_profile.py    # AgentProfile
│   │   ├── provider.py         # Provider 配置
│   │   ├── kiro_agent.py       # Kiro 特定選項
│   │   └── q_agent.py          # Q CLI 特定選項
│   │
│   ├── agent_store/            # [P2] 代理設定檔案
│   │   ├── developer.md        # 開發者角色定義
│   │   ├── reviewer.md         # 審查員角色定義
│   │   └── code_supervisor.md  # 監督員角色定義
│   │
│   ├── utils/                  # [P2] 工具函式
│   │   ├── terminal.py         # ID 生成, 狀態等待
│   │   ├── logging.py          # 檔案日誌
│   │   ├── agent_profiles.py   # 載入設定檔案
│   │   └── template.py         # Jinja2 模板渲染
│   │
│   └── constants.py            # 全域常數
│
├── test/                       # [P2] 測試
│   ├── providers/              # 代理單元測試
│   ├── services/               # 服務測試
│   ├── e2e/                    # 端到端測試
│   └── conftest.py             # pytest 設定
│
├── pyproject.toml              # 專案元資料 (uv 管理)
├── CODEBASE.md                 # 架構說明
└── DEVELOPMENT.md              # 開發指南
```

## 3. 核心模組詳解

### 3.1 Provider 抽象層

```
BaseProvider (抽象基類)
├── execute(input: str) → str      # 執行 CLI 命令 + 解析輸出
├── mark_input_received() → None   # 確認輸入已接收 (hook)
├── build_command() → List[str]    # 構建 CLI 命令列
├── parse_output(output: str) → str # 解析輸出 (提示符偵測)
└── [provider_id, default_model]   # 提供者識別

KiroCliProvider
├── 命令格式: kiro_cli --model <model> exec "<prompt>"
├── 提示符: 無特殊提示符
└── 信任提示: 無

ClaudeCodeProvider
├── 命令格式: claude --with-tool <tool> <prompt>
├── 提示符: ❯ (偵測輸入已準備)
├── 信任提示: 「Trust workspace?」
└── mark_input_received: 檢測 ❯ 並標記

CodexProvider
├── 命令格式: codex --model <model> exec "<prompt>"
├── 提示符: › (執行指示器)
├── 項目偵測: • 格式的項目清單
└── 信任提示: 「Allow?」

QCliProvider
├── 命令格式: q cli chat "<prompt>"
├── 整合方式: 直接 CLI 呼叫
└── 模型選擇: 環境變數
```

### 3.2 終端生命週期

```
會話初始化 (launch.py)
    ↓
建立 Tmux 會話
    ↓
為每個代理建立終端窗格
    ↓
設置 CAO_TERMINAL_ID 環境變數
    ↓
啟動代理 CLI (e.g., claude, kiro_cli)
    ↓
終端服務監控 (terminal_service.py)
    │
    ├─ get_output() → 讀取窗格輸出
    ├─ send_input() → 發送鍵序列
    ├─ mark_input_received() → 提供者特定確認
    └─ delete() → 清理終端
```

### 3.3 信件系統 (Inbox)

```
發送方終端        接收方終端
    │                 │
    ├─ send_message() │
    │   ↓             │
    └─ InboxMessage   │
        (DB entry)    │
        ↓             │
    Watchdog 檔案監控 │
        ↓             │
    on_create()      │
    on_modify()      │
        ↓             │
        └─→ 接收方讀取 ← receive_message()

訊息狀態: PENDING → RECEIVED → PROCESSED
```

## 4. 啟動流程

### 4.1 CLI 啟動序列

```
cao launch [--provider kiro_cli] [--agent-profile developer]
    ↓
main.py 解析命令
    ↓
load_agent_profiles() → 讀取 .md 設定檔案
    ↓
Session 建立
    ├─ session_id = UUID
    ├─ created_at = now()
    └─ status = "active"
    ↓
create_tmux_session(session_name)
    ├─ tmux new-session -d -s <name>
    └─ 設置 TMUX 環境變數
    ↓
為每個代理建立終端 (for provider in providers)
    ├─ tmux new-window -t <session> -n <provider>
    ├─ 設置 CAO_TERMINAL_ID 環境變數
    ├─ 執行代理 CLI (e.g., claude, kiro_cli)
    └─ 保存 Terminal(id, session_id, provider_id, status)
    ↓
session_service.add_session(session)
    └─ SQLite INSERT
    ↓
啟動 MCP 伺服器 (可選)
    └─ mcp_server.py → FastMCP register_tool()
```

### 4.2 MCP 伺服器啟動

```
cao mcp-server
    ↓
mcp_server.py 主程式
    ↓
create_server()
    ├─ register handoff_tool
    │   ├─ 參數: from_terminal_id, to_terminal_id, message
    │   └─ 實作: inbox_service.send_message()
    │
    ├─ register send_message_tool
    │   ├─ 參數: terminal_id, message
    │   └─ 實作: terminal_service.send_input()
    │
    └─ register list_terminals_resource
        └─ 查詢所有終端 (for CLAUDE.md context)
    ↓
stdio transport 連接
    └─ 接收 JSON-RPC 2.0 請求
```

### 4.3 HTTP API 啟動

```
cao-server (FastAPI 應用)
    ↓
Uvicorn 監聽 port 9889
    ↓
路由掛載:
    ├─ POST /sessions        → list_sessions()
    ├─ GET /sessions/{id}    → get_session()
    ├─ DELETE /sessions/{id} → delete_session()
    │
    ├─ POST /terminals           → create_terminal()
    ├─ GET /terminals/{id}       → get_terminal()
    ├─ POST /terminals/{id}/input → send_input()
    ├─ GET /terminals/{id}/output → get_output()
    └─ DELETE /terminals/{id}    → delete_terminal()
    │
    └─ POST /inbox/send → send_message()
```

## 5. 資料流 ASCII 圖

### 5.1 多代理並聯流程

```
用戶 (Claude/终端)
    │
    ├─────────────────────────────────────────┐
    │                                         │
    ↓                                         ↓
Claude 代理 (窗格 A)              Codex 代理 (窗格 B)
    │                                 │
    ├─ 輸入: "審查代碼"              ├─ 輸入: "生成方案"
    │                                 │
    ├─ CAO_TERMINAL_ID=term_a        ├─ CAO_TERMINAL_ID=term_b
    │ CAO_SESSION_ID=sess_1          │ CAO_SESSION_ID=sess_1
    │                                 │
    └─ /claude/...                   └─ /codex/...
          ↓                               ↓
      終端服務                       終端服務
          │                             │
          ├─ send_input()              ├─ send_input()
          ├─ get_output()              ├─ get_output()
          └─ mark_input_received()     └─ mark_input_received()


信件系統 (Tmux 間通信)
    ↑                                       ↑
    │ inbox_service.send_message()        │
    │ (term_a → term_b)                   │
    └───────────────────────────────────┘

SQLite Database
├─ sessions table          (session_id, created_at, status)
├─ terminals table         (terminal_id, session_id, provider_id, status, pane_id)
└─ inbox_messages table    (from_id, to_id, message, status, created_at)
```

### 5.2 終端間握手流程

```
Claude (term_a)          Codex (term_b)
    │                         │
    ├─ "向 Codex 詢問意見"    │
    │                         │
    ├─ handoff_tool()        │
    │   (from=term_a,        │
    │    to=term_b,          │
    │    msg="...") →        │
    │                         │
    ├─ inbox_service         │
    │   .send_message()      │
    │   ↓                     │
    │   DB: INSERT inbox_    │
    │        message (PEND.) │
    │                        ↓
    │                   term_b 輪詢
    │                   (via Watchdog)
    │                        ↓
    │                   on_create:
    │                   inbox_message
    │                        ↓
    │                   mark RECEIVED
    │                        ↓
    │  ← ─ ─ ─ ─ ─ ─ ─ ─ ─  讀取訊息
    │                        ↓
    │                   send_input()
    │                   "Codex: ..."
    │                        ↓
    │                   mark PROCESSED
    │
    └─ 等待 term_b 回覆...
```

## 6. 子系統清單

### 6.1 P0 優先級 (核心)

| 子系統 | 功能 | 檔案 | 狀態 |
|--------|------|------|------|
| CLI Entry | `cao launch` 命令 | `cli/commands/launch.py` | ✅ |
| MCP Server | Handoff + send_message 工具 | `mcp_server/server.py` | ✅ |
| HTTP API | REST 端點 (port 9889) | `api/main.py` | ✅ |
| Provider Manager | 多代理路由 | `providers/manager.py` | ✅ |
| Terminal Service | 終端 CRUD + I/O | `services/terminal_service.py` | ✅ |
| Inbox Service | 終端間訊息 | `services/inbox_service.py` | ✅ |

### 6.2 P1 優先級 (重要功能)

| 子系統 | 功能 | 檔案 | 狀態 |
|--------|------|------|------|
| Kiro CLI Provider | Kiro 集成 | `providers/kiro_cli.py` | ✅ |
| Claude Code Provider | Claude Code 整合 (❯ 提示符) | `providers/claude_code.py` | ✅ |
| Codex Provider | Codex 整合 (› 提示符) | `providers/codex.py` | ✅ |
| Q CLI Provider | Amazon Q 整合 | `providers/q_cli.py` | ✅ |
| Gemini CLI Provider | Gemini 整合 | `providers/gemini_cli.py` | ✅ |
| Session Service | 會話 CRUD | `services/session_service.py` | ✅ |
| Flow Service | 排程執行 | `services/flow_service.py` | ✅ |
| Tmux Client | libtmux 包裝 | `clients/tmux.py` | ✅ |
| SQLite Client | 資料庫操作 | `clients/database.py` | ✅ |

### 6.3 P2 優先級 (增強功能)

| 子系統 | 功能 | 檔案 | 狀態 |
|--------|------|------|------|
| Cleanup Service | 過期會話清理 | `services/cleanup_service.py` | ✅ |
| Agent Profiles | .md 設定檔案 | `agent_store/*.md` | ✅ |
| Profile Loader | 設定檔案解析 | `utils/agent_profiles.py` | ✅ |
| Template Renderer | Jinja2 模板 | `utils/template.py` | ✅ |
| File Logging | 檔案日誌 | `utils/logging.py` | ✅ |
| Terminal Utils | ID 生成, 狀態等待 | `utils/terminal.py` | ✅ |
| Data Models | Pydantic 模型 | `models/*.py` | ✅ |
| Unit Tests | pytest 測試套件 | `test/providers/` | ✅ |
| E2E Tests | 端到端測試 | `test/e2e/` | ✅ |

### 6.4 已知限制

- **信任提示 (Trust Prompt)：** Claude Code 和 Codex 提供者需要手動確認「Trust workspace?」，目前通過 `--yolo` 標誌自動跳過
- **提示符變異性：** 不同 Shell 和平臺上提示符可能不同，需要代理特定配置
- **輸出截斷：** 大型輸出在 tmux 窗格中可能被截斷，需要實現頁面大小適應
- **會話持久化：** 重啟後需要重新初始化所有終端，無法恢復中斷的會話

## 7. 擴展指南

### 7.1 新增代理

1. 在 `providers/` 建立 `<agent>_provider.py`
2. 繼承 `BaseProvider`，實現：
   - `build_command(prompt: str) → List[str]`
   - `parse_output(output: str) → str`
   - `mark_input_received() → None`
3. 在 `providers/manager.py` 註冊
4. 新增單元測試到 `test/providers/`

### 7.2 新增服務

1. 在 `services/` 建立 `<service>_service.py`
2. 使用 `session_service` 和 `terminal_service` 的 API
3. 在 `api/main.py` 掛載路由
4. 如需排程，在 `services/flow_service.py` 註冊

## 8. 技術棧

- **Framework:** FastAPI + libtmux + FastMCP
- **Language:** Python 3.10+
- **Package Manager:** uv (快速依賴解析)
- **Database:** SQLite
- **Concurrency:** asyncio + APScheduler
- **Testing:** pytest + pytest-asyncio
- **Logging:** Python logging

## 9. 關鍵設計決策

1. **Tmux 分窗口而非 subprocess.Popen：** 提供使用者完整的終端交互控制
2. **狀態機驅動的終端生命週期：** 清晰的狀態轉移，便於故障排除
3. **watchdog 檔案監控：** 高效的事件驅動信件系統，避免輪詢
4. **代理無關的 Provider 抽象：** 易於支持新的 AI CLI 工具
5. **多入口點 (CLI/MCP/HTTP)：** 靈活集成到不同工作流

## 10. 開發工作流

```bash
# 安裝開發環境
uv sync

# 執行單元測試
uv run pytest test/providers/ -v

# 執行 E2E 測試 (需要 tmux + 已認證 CLI)
uv run pytest -m e2e test/e2e/ -v

# 啟動 CLI
uv run cao launch --provider kiro_cli --agent-profile developer

# 啟動 MCP 伺服器
uv run cao mcp-server

# 啟動 HTTP API
uv run cao-server
```

## 11. 故障排除

| 問題 | 原因 | 解決方案 |
|------|------|---------|
| tmux 會話無法建立 | tmux 未安裝 | `apt install tmux` (Linux) 或 `brew install tmux` (macOS) |
| 終端無輸出 | 提示符不匹配 | 檢查 Provider 的 `parse_output()` 邏輯 |
| 信件未送達 | Watchdog 未監控 | 檢查 `CAO_INBOX_DIR` 環境變數 |
| 代理認證失敗 | 尚未執行 `<agent> login` | 執行 `claude auth login` 或 `codex login` |

## 12. 版本資訊

- **當前版本:** 1.1.0
- **發佈日期:** 2025-03-13
- **主要功能:** 多代理並聯、MCP 伺服器、HTTP API、狀態驅動終端管理
