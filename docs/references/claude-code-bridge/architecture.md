# Claude Code Bridge 架構文檔

## 1. 專案概覽

**Claude Code Bridge (ccb) v5.2.6** 是一個多模型協作框架，採用分窗口終端設計，支持 Claude、Codex、Gemini CLI、OpenCode、Droid 等多個 AI 提供者的實時互動。與 MCP 或 API 導向的方式不同，ccb 提供完全的 WYSIWYG (所見即所得) 分窗口體驗，每個互動都可見且可控。

**核心價值主張：**
- **可見性優先：** 分窗口 CLI 展示所有互動，沒有黑盒子
- **持久化記憶：** 每個 AI 維護獨立上下文，支持 `-r` 旗標恢復會話
- **token 節省：** 發送輕量級提示而非完整檔案歷史
- **原生工作流：** WezTerm (推薦) 或 tmux 原生集成，無需複雜伺服器
- **非同步通信：** 統一的異步守衛，確保 Claude 在后台任務完成前不會輪詢

## 2. 目錄結構

```
claude_code_bridge/
├── bin/                        # [P0] 可執行指令碼
│   ├── ask                     # 統一命令發送器 (所有提供者)
│   ├── lask                    # Claude 異步守衛程式
│   ├── cask                    # Codex 異步守衛程式
│   ├── gask                    # Gemini 異步守衛程式
│   ├── oask                    # OpenCode 異步守衛程式
│   ├── dask                    # Droid 異步守衛程式
│   ├── qask                    # Qwen 異步守衛程式
│   │
│   ├── lping                   # Claude 健康檢查
│   ├── cping                   # Codex 健康檢查
│   ├── gping                   # Gemini 健康檢查
│   ├── oping                   # OpenCode 健康檢查
│   ├── dping                   # Droid 健康檢查
│   │
│   ├── lpend                   # Claude 掛起等待 (RPC)
│   ├── cpend                   # Codex 掛起等待
│   ├── gpend                   # Gemini 掛起等待
│   ├── opend                   # OpenCode 掛起等待
│   ├── dpend                   # Droid 掛起等待
│   │
│   ├── ccb-arch                # 架構圖生成器
│   ├── ccb-ping                # 統一 Ping 工具
│   ├── ccb-mounted             # 掛載狀態檢查
│   ├── ccb-cleanup             # 清理僵屍程式
│   ├── ccb-web                 # Web 儀表板
│   ├── ctx-transfer            # 會話上下文轉移
│   │
│   ├── maild                   # [P1] 郵件守衛程式 (遠程 AI 訪問)
│   ├── laskd                   # [P1] Claude 統一守衛程式
│   ├── askd                    # [P1] 統一守衛程式 (all providers)
│   │
│   ├── autonew                 # 自動新建會話
│   └── cping                   # 提供者 ping
│
├── lib/                        # [P1] Python 工具庫
│   ├── compat.py               # Windows UTF-8 相容性
│   ├── cli_output.py           # 終止碼 (EXIT_OK, EXIT_ERROR)
│   ├── providers.py            # 提供者解析邏輯
│   ├── session_utils.py        # 會話檔案搜尋
│   │
│   ├── askd_runtime.py         # 統一守衛執行時
│   ├── askd_rpc.py             # askd JSON-RPC 客戶端
│   ├── askd/
│   │   ├── daemon.py           # 守衛程式啟動/ping
│   │   └── server.py           # 守衛伺服器迴圈
│   │
│   ├── providers/
│   │   ├── claude_comm.py      # Claude 會話 IPC
│   │   ├── codex_comm.py       # Codex 會話 IPC
│   │   ├── gemini_comm.py      # Gemini 會話 IPC
│   │   ├── opencode_comm.py    # OpenCode 會話 IPC (CCB_DONE 解析)
│   │   ├── droid_comm.py       # Droid 會話 IPC
│   │   └── qwen_comm.py        # Qwen 會話 IPC
│   │
│   ├── mail/
│   │   ├── daemon.py           # 郵件守衛迴圈
│   │   ├── imap.py             # IMAP IDLE 連線
│   │   ├── smtp.py             # SMTP 回覆傳送
│   │   ├── presets.py          # Gmail, Outlook, QQ 預設
│   │   └── secrets.py          # 系統鑰匙圈儲存
│   │
│   └── compat_windows.py       # Windows 特定修復
│
├── claude_skills/              # [P1] Claude Code 技能
│   ├── ask/SKILL.md            # 詢問任何提供者
│   ├── continue/SKILL.md       # 繼續上一個會話
│   ├── file-op/SKILL.md        # 檔案操作
│   ├── mounted/SKILL.md        # 掛載狀態檢查
│   ├── pend/SKILL.md           # 等待回覆
│   ├── review/SKILL.md         # 審查代碼
│   ├── tp/SKILL.md             # 上下文轉移計劃
│   ├── tr/SKILL.md             # 上下文轉移執行
│   ├── all-plan/SKILL.md       # 多代理計劃
│   └── cping/SKILL.md          # Ping 檢查
│
├── codex_skills/               # [P1] Codex 技能
│   └── (相同於 claude_skills)
│
├── .claude/                    # [P2] Claude Code 插件配置
│   ├── DEVELOPMENT.md          # 開發說明
│   ├── claude-octopus.local.md # 本地測試
│   ├── settings.json           # 插件設定
│   └── commands/               # 其他命令
│
├── .claude-plugin/             # [P2] 插件打包
│   └── plugin.json             # 插件描述符
│
├── CHANGELOG_4.0.md            # 變更日誌
├── README.md                   # 使用手冊
└── LICENSE                     # MIT 授權
```

## 3. 核心模組詳解

### 3.1 統一命令 (ask)

```python
# bin/ask 流程
def main(argv):
    # 1. 解析提供者
    provider = parse_qualified_provider(argv[1])
    # 格式: "codex:3" 或 "gemini" (單一) → (base_provider, instance)

    # 2. 偵測呼叫者 (調用 AI 代理)
    caller = _detect_caller()  # 優先級:
        # ① CCB_CALLER 環境變數
        # ② 郵件元資料 (email)
        # ③ Tmux/WezTerm 窗格 ID
        # ④ 環境變數提示 (CLAUDE_SESSION_ID 等)
        # ⑤ 會話檔案搜尋

    # 3. 判斷模式
    if notify_mode:        # --notify
        foreground = False
    elif --foreground:
        foreground = True
    else:
        foreground = _default_foreground()
        # Claude → async (默認), 其他 → foreground

    # 4. 執行
    if _use_unified_daemon():
        if foreground:
            _send_via_unified_daemon(provider, message, timeout, caller)
        else:
            # 後臺非同步
            _spawn_background_task(provider, message, caller, task_id)
    else:
        # 舊有路徑: 特定守衛程式 (lask, cask, gask, ...)
        subprocess.run([daemon_cmd, ...])

# 範例呼叫
ask claude "What is X?"           # Claude (foreground)
ask gemini "Explain Y"             # Gemini (foreground)
ask codex --background "Code Z"   # Codex (background)
```

### 3.2 提供者通信層 (lib/providers/)

```
claude_comm.py
├── find_session_path()      # ~/.claude/ 搜尋會話資料夾
├── read_session_json()      # 解析 session.json (Codex CLI 0.29 SHA-256 路徑)
├── IPC 傳輸
│   ├── Session ID 管道
│   ├── Request ID 配對
│   └── Reply 緩衝區
└── 解析 Claude 原生 JSON 輸出

codex_comm.py
├── find_session_path()      # ~/.codex/sessions/
├── request.json → response.json 檔案交換
├── req_id 配對 (hex 或 timestamp 格式)
└── OpenAI ChatCompletion 回覆格式

gemini_comm.py
├── find_session_path()      # ~/.gemini/
├── 輪詢會話狀態
├── Gemini Idle Timeout      # 15秒檢測 (可配置 CCB_GEMINI_IDLE_TIMEOUT)
└── CCB_DONE 標記解析

opencode_comm.py
├── 會話 ID 固定修復 (v5.2.6)
├── req_id 雙雜湊策略
│   ├─ 舊格式: hex session ID
│   └─ 新格式: timestamp-based
├── CCB_DONE 緩解
│   ├─ 完全匹配優先
│   └─ 降級模式 (缺少 CCB_DONE 時)
└── 非同步死鎖修復

droid_comm.py
├── Droid 執行時發現
├── 輕量級 JSON 通信
└── 快速模型轉換
```

### 3.3 非同步守衛模式

```
CLI 呼叫
    ↓
ask --background <provider> "message"
    ↓
┌──────────────────────────────────────────┐
│ 統一守衛程式 (askd / 特定 laskd)        │
├──────────────────────────────────────────┤
│ TCP 伺服器 (port 動態分配)               │
│ ✓ CCB_CALLER 隔離 (呼叫者追蹤)          │
│ ✓ task_id 配對                           │
│ ✓ 超時管理 (CC_ASKD_AUTOSTART)         │
│ ✓ 會話恢復 (-r 旗標)                     │
│ ✓ 環境變數 pass-through                  │
└──────────────────────────────────────────┘
    ↓
後臺執行:
    1. 啟動指定提供者 CLI
    2. 發送訊息
    3. 等待回覆 (監控會話檔案)
    4. 檢測 CCB_DONE 或 Idle Timeout
    5. 返回回覆
    ↓
[CCB_ASYNC_SUBMITTED provider=gemini]
[CCB_ASYNC_PID task=abc123 pid=12345]
[CCB_ASYNC_STATUS_FILE] /tmp/ccb-tasks/ask-gemini-abc123.status

使用者可即時檢查進度:
    tail -f /tmp/ccb-tasks/ask-gemini-abc123.status
    tail -f /tmp/ccb-tasks/ask-gemini-abc123.log
```

### 3.4 郵件系統 (lib/mail/)

```
maild (Email Daemon)
    ↓
IMAP IDLE 連線 (支持 Gmail, Outlook, QQ)
    ├─ 郵件到達通知
    └─ 實時推動 (非輪詢)
    ↓
郵件提取:
    ├─ 解析主題: [CLAUDE], [CODEX], [GEMINI]
    ├─ 路由到指定提供者
    └─ 設置 CCB_EMAIL_REQ_ID (追蹤)
    ↓
執行 ask <provider>:
    ├─ CCB_EMAIL_REQ_ID=<uuid>
    ├─ CCB_EMAIL_MSG_ID=<msg_id>
    └─ CCB_EMAIL_FROM=<sender@example.com>
    ↓
回覆流程:
    ├─ 等待完成
    ├─ SMTP 傳送回覆
    └─ 標記郵件為已處理
```

## 4. 啟動流程

### 4.1 WezTerm 集成啟動

```
WezTerm 配置
    ↓
ccb 初始化指令碼
    ├─ source ~/.ccb/init.sh (bash)
    ├─ source ~/.ccb/init.fish (fish)
    └─ . ~/.ccb/init.ps1 (PowerShell)
    ↓
設置環境:
    ├─ CCB_RUN_DIR = ~/.ccb/workspace/<project>
    ├─ CCB_ASKD_AUTOSTART = 1 (自動啟動統一守衛)
    ├─ TMUX_PANE / WEZTERM_PANE (窗格識別)
    └─ 提供者路徑搜尋 (claude, codex, gemini, ...)
    ↓
啟動統一守衛 (asdk):
    └─ 監聽 TCP port (state → askd.json)
    ↓
會話初始化:
    ├─ .claude-session / .codex-session (本地)
    ├─ ~/.ccb/history/ (上下文轉移)
    └─ ~/.ccb/workspace/ (每個專案隔離)
```

### 4.2 Tmux 集成啟動

```
tmux 會話建立
    ↓
~/.tmux.conf 設置:
    ├─ set-environment -g CCB_RUN_DIR
    ├─ bind-key 快速鍵 (ask, ask-continue)
    └─ run-shell "askd &"
    ↓
建立分窗:
    ├─ split-window -h (Claude 左, Codex 右)
    ├─ send-keys -t 左窗 "claude"
    ├─ send-keys -t 右窗 "codex"
    └─ 設置 TMUX_PANE 環境變數
    ↓
分窗通信:
    ├─ ask gemini --background "research X"
    │   → [CCB_ASYNC_SUBMITTED provider=gemini]
    │
    └─ lpend                # Claude 等待回覆
        → 輪詢 Gemini 會話
        → 檢測 CCB_DONE
        → 讀取並顯示回覆
```

## 5. 資料流 ASCII 圖

### 5.1 多提供者分窗口流程

```
WezTerm / Tmux
┌───────────────────────────────────────────┐
│                                           │
│  ┌─────────────────┐  ┌─────────────────┐│
│  │  Claude Pane    │  │  Codex Pane     ││
│  │  ❯ prompt      │  │  › prompt       ││
│  │  session.json   │  │  session.json   ││
│  │                 │  │                 ││
│  │ (CLAUDE_      │  │ (CODEX_        ││
│  │  SESSION_ID)  │  │  SESSION_ID)    ││
│  └─────────────────┘  └─────────────────┘
│
│  ┌─────────────────┐  ┌─────────────────┐
│  │  Gemini Pane    │  │  OpenCode Pane  │
│  │  ~ prompt       │  │  $ prompt       │
│  │  session.json   │  │  session.json   │
│  │                 │  │                 │
│  │ (GEMINI_      │  │ (OPENCODE_    │
│  │  SESSION_ID)  │  │  SESSION_ID)  │
│  └─────────────────┘  └─────────────────┘
│
└───────────────────────────────────────────┘

使用者命令:
    $ ask codex "generate function"
        ↓
    bin/ask 解析提供者
        ├─ provider = "codex"
        ├─ caller = "claude" (偵測)
        └─ message = "generate function"
        ↓
    統一守衛程式 (askd)
        ├─ 查找 codex 會話路徑
        ├─ 建立 request.json
        ├─ 監控 response.json (Codex 迴圈)
        ├─ 檢測 CCB_DONE 或回覆完成
        └─ 返回結果
        ↓
    顯示結果於當前終端
    (或非同步: [CCB_ASYNC_SUBMITTED provider=codex])
```

### 5.2 上下文轉移流程

```
會話 A (Project X)          會話 B (Project Y)
Claude 開發者角色           Claude 審查角色
    │                           │
    ├─ /tr (transfer)           │
    │   └─ 導出到 ~./ccb/
    │       history/claude-
    │       <timestamp>-
    │       <session_id>.md
    │           ↓              │
    │       自動前置提示       │
    │       (分析架構)         │
    │           ↓              │
    └─────────────────────────→
                                ├─ /continue
                                │   (讀取歷史)
                                ├─ @ 附加上下文
                                └─ 繼續會話

歷史檔案結構:
    .ccb/history/
    ├─ claude-20250313-145020-abc123.md
    │   ├─ ## Context
    │   │   └─ 架構分析
    │   ├─ ## Actions Taken
    │   │   └─ 完成的工作
    │   └─ ## Intermediate Outputs
    │       └─ 工件
    │
    └─ (下次會話可從此恢復)
```

## 6. 子系統清單

### 6.1 P0 優先級 (核心)

| 子系統 | 功能 | 檔案 | 狀態 |
|--------|------|------|------|
| 統一 ask 命令 | 路由所有提供者 | `bin/ask` | ✅ |
| Claude 通信 | 會話 IPC | `lib/providers/claude_comm.py` | ✅ |
| Codex 通信 | 檔案交換 | `lib/providers/codex_comm.py` | ✅ |
| Gemini 通信 | 會話監控 + Idle 檢測 | `lib/providers/gemini_comm.py` | ✅ |
| OpenCode 通信 | 雙雜湊策略 (v5.2.6) | `lib/providers/opencode_comm.py` | ✅ |
| 統一守衛 (askd) | TCP 伺服器 + 自動啟動 | `bin/askd`, `lib/askd/` | ✅ |
| 呼叫者偵測 | 環境推理 | `bin/ask` (_detect_caller) | ✅ |

### 6.2 P1 優先級 (重要功能)

| 子系統 | 功能 | 檔案 | 狀態 |
|--------|------|------|------|
| Ping 工具 | 健康檢查 (lping, cping, gping, ...) | `bin/*ping` | ✅ |
| Pend 工具 | 等待回覆 RPC | `bin/*pend` | ✅ |
| 郵件守衛 | IMAP IDLE + SMTP 回覆 | `bin/maild`, `lib/mail/` | ✅ |
| 會話恢復 | `-r` 旗標支持 | `bin/ask` | ✅ |
| 上下文轉移 | `ccb tr` / `/continue` | `bin/ctx-transfer` | ✅ |
| Claude 技能 | `/ask`, `/continue`, `/review` | `claude_skills/` | ✅ |
| Codex 技能 | (同上) | `codex_skills/` | ✅ |
| 異步守衛 | 非同步任務追蹤 | `lib/askd/daemon.py` | ✅ |
| Windows 相容性 | UTF-8 編碼修復 | `lib/compat.py` | ✅ |

### 6.3 P2 優先級 (增強功能)

| 子系統 | 功能 | 檔案 | 狀態 |
|--------|------|------|------|
| Web 儀表板 | 會話監控 UI | `bin/ccb-web` | ✅ |
| 架構圖生成 | Mermaid 圖表 | `bin/ccb-arch` | ✅ |
| 清理工具 | 僵屍程式移除 | `bin/ccb-cleanup` | ✅ |
| 掛載檢查 | 自動啟動 | `bin/ccb-mounted` | ✅ |
| Droid 支持 | Factory AI 集成 | `lib/providers/droid_comm.py` | ✅ |
| Qwen 支持 | 多語言 LLM | `lib/providers/qwen_comm.py` | ✅ |
| 日誌記錄 | 任務追蹤 | `/tmp/ccb-tasks/` | ✅ |
| 會話持久化 | 本地會話檔案 | `~/.ccb/workspace/` | ✅ |

### 6.4 已知限制與修復 (v5.2.6)

| 問題 | 影響 | 修復 | 狀態 |
|------|------|------|------|
| Gemini CLI 0.29 路徑 | 會話查找失敗 | SHA-256 + basename 雙策略 | ✅ |
| OpenCode 會話 ID 固定 | 第二次呼叫失敗 | 解除 ID 固定，改用 req_id | ✅ |
| Gemini 缺少 CCB_DONE | 完成檢測失敗 | 15秒 Idle Timeout + 提示強化 | ✅ |
| Codex req_id 配對 | 非同步死鎖 | hex + timestamp 雙格式支持 | ✅ |
| Lpend 會話過期 | 舊註冊表 | 優先新路徑搜尋 | ✅ |
| 跨平臺提示符 | 提示符變異 | 代理特定配置 (WIP) | ⚠️ |

## 7. 技術棧

- **Shell Language:** Python 3.8+
- **IPC Mechanism:** 檔案 (request.json/response.json) + JSON-RPC (TCP)
- **Terminal Multiplexer:** WezTerm (推薦) 或 tmux
- **Database:** 本地會話檔案 (JSON) + ~/.ccb/history/
- **Utilities:** watchdog (檔案監控), systemd (可選 maild 守衛)
- **Protocols:** 自定義 CCB 標記 (CCB_ASYNC_SUBMITTED, CCB_DONE)

## 8. 關鍵設計決策

1. **分窗口而非單執行緒 API：** 完全透明，使用者掌控每個模型
2. **會話持久化而非上下文轉移：** 支持 `-r` 恢復，無需重新認證
3. **自動呼叫者偵測：** 多環境支持 (tmux, WezTerm, 郵件, 手動)
4. **統一 ask 命令 + 特定守衛程式：** 漸進式採用 (legacy → askd)
5. **Gemini Idle Timeout：** 應對 API 不穩定性
6. **雙雜湊策略 (OpenCode)：** 向後相容性 + 新格式支持

## 9. 開發工作流

```bash
# 安裝 ccb
pip install -e .

# 測試 ask 命令
python -m bin.ask claude "hello"

# 啟動統一守衛
python lib/askd/daemon.py

# 測試郵件守衛 (需要郵件設置)
python bin/maild

# 執行技能測試
pytest tests/ -v

# 查看日誌
tail -f /tmp/ccb-tasks/ask-*.log
```

## 10. 版本資訊

- **當前版本:** 5.2.6
- **發佈日期:** 2025-03-13
- **新功能:** Gemini 0.29 支持、OpenCode 死鎖修復、IPC 穩定性增強、郵件系統完成度
