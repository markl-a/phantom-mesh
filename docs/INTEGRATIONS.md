# 整合（Integrations）

Phantom-Mesh 設計上可在四種模式下使用：

1. **獨立 REPL / TUI（終端使用者介面）** — `phantom`（單行 REPL）或 `phantom tui`（全螢幕 ratatui）
2. **作為 Claude Code 的子代理（subagent）** — 透過 MCP（模型上下文協議）stdio（`phantom mcp`）
3. **作為 Codex CLI 的子代理（subagent）** — 透過 MCP stdio（同樣是 `phantom mcp`）
4. **WebSocket / 網頁儀表板（web dashboard）** — `phantom serve`

本指南涵蓋第 2、3、4 種接線方式。

## v0.1.0-alpha 的新功能（2026-04-27）

- 工具數量現為 **45** 個（新增 `web_fetch`、`bash_run_background`、
  `bash_output`、`bash_kill`、`ask_user`）。
- REPL 新增 markdown 渲染、Tab 自動補全、Ctrl-C 取消、計畫模式（plan-mode）
  把關、`@image.png` 多模態（multimodal）附加，以及斜線指令 `/show /perm
  /density /theme /agent /agents /todo /plan /resume`。
- 新的 `phantom tui` ratatui 全螢幕介面。
- 網頁儀表板新增 Cmd+K 命令面板（palette）、Info 分頁中的 Tools/Sessions/Cost
  面板、xterm.js 終端機，以及即時的對等節點 ping 狀態點（peer-ping dots）。
- `phantom evolve` 自我迭代（self-iteration）已端對端驗證完成（見
  [SELF-EVOLVE.md](SELF-EVOLVE.md)）。

---

## 1. Claude Code（推薦）

Claude Code 把 phantom 當作 MCP 伺服器來使用。設定完成後，全部 45 個 phantom
工具都能在任何 Claude Code 工作階段（session）中被呼叫。完整指南見
[CLAUDE-CODE-SETUP.md](CLAUDE-CODE-SETUP.md)；以下是精簡版。

### 設定

編輯 `~/.claude.json`（若你的 Claude Code 版本支援，也可使用 `claude mcp add`）：

```json
{
  "mcpServers": {
    "phantom": {
      "command": "/usr/local/bin/phantom",
      "args": ["mcp"],
      "env": {
        "GROQ_API_KEY": "gsk_...",
        "GEMINI_API_KEY": "AIza..."
      }
    }
  }
}
```

把 `/usr/local/bin/phantom` 換成你機器上 `which phantom` 的結果。

### 驗證

重新啟動 Claude Code，然後在任何工作階段中執行 `/mcp`。你應該會看到 `phantom` 連同 45 個工具一起列出（shell、file_*、content_search、web_fetch、web_search、hardware、scaffold、mesh、mcp、ask_user、bash_run_background 等等）。

### 使用

Phantom 工具以名稱呼叫。以下是 Claude Code 可以為你執行的範例：

```
"Use phantom's hardware tool to check this Mac's specs"
"Have phantom run a parallel grep across the home cluster for TODO"
"Use phantom mesh delegate to run cargo test on the node-a worker"
```

Phantom 把自己的工作階段狀態保存在 `~/.phantom-mesh/conversations/`，獨立於 Claude Code 的歷史記錄之外。

### Mesh（網狀網路）使用情境（獨特價值所在）

如果你的 phantom 節點是某個 mesh（網狀網路）的一部分（`agents.toml` 的 `[cluster]` 區塊已設定對等節點），你可以請 Claude Code 把繁重工作委派（delegate）給其他節點，同時讓你的筆電保持高效運作：

- 「請 phantom 把測試套件派送到核心數較多的工作節點（worker）」
- 「用 phantom mesh swarm 詢問所有對等節點的 CPU/RAM，並做摘要」
- 「把這份資料集分析委派給 node-a（它有 GPU）」

---

## 2. Codex CLI（0.39 以上）

Codex 現在原生支援 MCP stdio。一道指令即可接入 phantom：

```bash
codex mcp add phantom $(which phantom) mcp
```

這會在 `~/.codex/config.toml` 寫入一個 `[mcp_servers.phantom]` 區塊。
驗證：

```bash
grep -A2 "mcp_servers.phantom" ~/.codex/config.toml
# command = "/Users/.../bin/phantom"
# args = ["mcp"]
```

接著在 `codex` 內使用 `/mcp` 確認 phantom 顯示 45 個工具，並像呼叫其他工具一樣
呼叫它們：`Use phantom shell to run pwd`。

### WebSocket 後備方案（較舊的 Codex / 自訂客戶端）

Phantom 也提供一個與 Codex 相容的 WebSocket JSON-RPC 端點（endpoint）：

```bash
phantom serve --bind 127.0.0.1:7878
# WebSocket endpoint: ws://localhost:7878/ws
```

若 `agents.toml` 包含 `[cluster] cluster_secret`，該密鑰會被用作傳入 WS 連線上的
持有者權杖（bearer token）；請從你的客戶端傳入它。

---

## 3. 獨立 REPL（不需要客戶端）

```bash
# Interactive REPL, Claude Code style
phantom

# One-shot prompt
phantom "find all TODO comments in src/"

# Resume the last session
phantom -c

# Switch agent (default: master)
phantom --agent reviewer "review this PR diff: ..."
```

REPL 功能：

- **串流輸出（streaming output）**，含行內工具呼叫（`● shell(cargo test)` ... `✓ ok`）
- **Markdown 渲染** — 項目符號、編號清單、引用區塊（blockquote）、連結、行內程式碼
- **多行輸入** — 在行尾加 `\` 以延續到下一行
- **Tab 自動補全** — `/cmd` 與 `@path/to/...` 在按 Tab 時展開
- **Ctrl-C** 取消進行中的 LLM 串流（REPL 仍保持運作）
- **計畫模式（plan mode）** — `/plan` 切換為開啟；代理必須先輸出計畫，你說 `go` 才執行
- **斜線指令** — `/help` 可看完整清單（24 個指令）
- **`@path/to/file`** — 把檔案內容行內嵌入提示詞中
- **`@image.png`** — 把 PNG/JPG 以多模態 `image_url` 形式附加（在 OpenAI、Gemini、Anthropic 上皆可用）
- **逐工具權限** — `/perm ask|allow|deny|list|reset`
- **工作階段延續** — `phantom -c` 或 `/resume <prefix>`
- **成本追蹤** — 每一輪之後顯示 `[↑ $0.0023  ∑ $0.0145  3.2s]`

若需要全螢幕的替代方案，`phantom tui` 會開啟一個 ratatui 介面
（持久的輸入框、可捲動的對話記錄、狀態列）。關於自主自我迭代，
見 [SELF-EVOLVE.md](SELF-EVOLVE.md)。

---

## 4. 供應商（provider）設定

在 `~/.phantom-mesh/env` 中設定 API 金鑰：

```bash
GROQ_API_KEY=gsk_xxx
GEMINI_API_KEY=AIzaSy_xxx
ANTHROPIC_API_KEY=sk-ant-xxx   # optional
```

或在 `~/.phantom-mesh/agents.toml` 中為每個節點寫死設定：

```toml
[[providers]]
name = "groq"
base_url = "https://api.groq.com/openai/v1"
api_key = "gsk_xxx"
default_model = "llama-3.3-70b-versatile"
primary = true
```

目前可用的免費方案：
- **Groq** — 快速的 Llama 3.3 70B，免費額度寬裕
- **Gemini** — 長上下文，免費方案每日額度（較少）

---

## 5. 快速參考

| 模式 | 指令 | 端點 | 搭配使用 |
|---|---|---|---|
| MCP stdio | `phantom mcp` | stdin/stdout | Claude Code、Codex CLI 0.39+、Cursor、任何 MCP 客戶端 |
| WebSocket | `phantom serve` | `ws://host:7878/ws` | 較舊的 Codex、自訂客戶端 |
| 網頁儀表板 | `phantom serve` | `http://host:7878` | 瀏覽器（Cmd+K 命令面板、xterm.js、Info 面板） |
| REPL | `phantom` | 終端機 | 人類直接使用 |
| TUI | `phantom tui` | 終端機 | 全螢幕 ratatui 介面 |
| 單次執行（One-shot） | `phantom "..."` | 終端機 | 指令稿、cron、自動化 |
| 自我迭代 | `phantom evolve "..."` | 終端機 | 在目前的程式碼倉庫上自主編輯循環 |

## 附加圖片

REPL 的 `@<path>` 語法現在會特別處理圖片檔（`.png`、`.jpg`、`.jpeg`、
`.gif`、`.webp`）：不再把位元組以文字形式行內嵌入，而是將檔案以
base64 編碼，並作為多模態 `image_url` 內容部分附加到送出的聊天訊息上。OpenAI、
Gemini 的 OpenAI 相容端點，以及原生的 Anthropic Messages API 都會被透明地處理
——Anthropic 請求會被改寫成等價的 `image` / `source.base64` 形狀。非圖片的 `@`
展開維持先前的行為（檔案以文字讀取並包裹在
`<file path="…">…</file>` 區塊中）。

範例：

```bash
phantom "describe @/path/to/screenshot.png"
```

在互動式 REPL 內同樣的語法也可用；你可以在單一提示詞中混用多張圖片
與自由格式文字。檔案在送出時才讀取，因此每則提示詞看到的都是磁碟上的當前內容。
