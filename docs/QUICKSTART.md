# Spectyn Mesh 快速上手

> 從 clone（複製倉庫）到第一次 agent（代理）執行，只需五分鐘。完整的操作者
> 操作導覽請見 [GETTING-STARTED.md](GETTING-STARTED.md)；若要快速
> 驗證今天新增的每項功能，請見
> [VERIFY-CHEATSHEET.md](VERIFY-CHEATSHEET.md)。

## 1. 安裝

```bash
# Build from source (Rust 1.75+)
git clone https://github.com/your-org/spectyn-mesh.git
cd spectyn-mesh
cargo install --path core --bin spectyn

# Verify
spectyn --version    # → 0.1.0
which spectyn        # → ~/.cargo/bin/spectyn
```

## 2. 設定

在 `~/.spectyn-mesh/env` 中至少設定一組供應商（provider）API 金鑰：

```bash
mkdir -p ~/.spectyn-mesh
cat >> ~/.spectyn-mesh/env <<'EOF'
GROQ_API_KEY=gsk_...          # free tier, fast — recommended for first run
GEMINI_API_KEY=AIza...        # optional
ANTHROPIC_API_KEY=sk-ant-...  # optional
EOF
```

或者執行 onboarding（新手導引）精靈，它會幫你寫好 `agents.toml`：

```bash
spectyn onboarding             # opens browser
# or
spectyn                        # terminal wizard auto-runs on first launch
```

## 3. 首次執行 — 挑選一種介面

### 獨立 REPL（讀取-求值-輸出循環，Claude Code 風格）

```bash
spectyn
> use shell to run "ls" and summarize
```

串流輸出、行內工具呼叫、markdown 渲染、Tab 自動補全、
透過結尾 `\` 進行多行輸入、Ctrl-C 取消執行中的串流。

### 全螢幕 TUI（終端使用者介面，採用 ratatui）

```bash
spectyn tui
```

持久的多行輸入框、可捲動的對話記錄、狀態列。
斜線命令（slash command）與 REPL 相同。

### 網頁儀表板（dashboard）

```bash
spectyn serve                  # default :7878
open http://localhost:7878
```

xterm.js 終端機面板、**Cmd+K** 命令面板（command palette）、
帶有 Todo / Sessions / Cost / Tools 子面板的 Info 分頁、即時 peer-ping（節點探測）小點。

### 一次性執行（one-shot）

```bash
spectyn "find all TODO comments in core/src and group by file"
```

### 作為 Claude Code 或 Codex CLI 的子代理（subagent）

請見 [INTEGRATIONS.md](INTEGRATIONS.md)。簡短版本：

```bash
# Claude Code: edit ~/.claude.json (see dev/CLAUDE-CODE-SETUP.md)
# Codex CLI 0.39+:
codex mcp add spectyn $(which spectyn) mcp
```

兩者都會暴露 45 個工具。

### 自我迭代（self-iteration）

```bash
spectyn evolve "fix the warning in core/src/cost.rs" --max-rounds 3 --agent coder
```

讀取檔案、編輯程式碼、重試直到完成。worked example（完整範例）
請見 [SELF-EVOLVE.md](SELF-EVOLVE.md)（在 Groq 免費方案上 $0 成本）。

## 4. REPL/TUI 內好用的斜線命令

| 命令 | 效果 |
|---|---|
| `/help` | 完整清單（24 個命令） |
| `/agents`, `/agent <name>` | 列出 / 切換目前作用中的 agent |
| `/tools` | 可用工具的分類清單 |
| `/sessions`, `/resume <prefix>` | session（工作階段）管理 |
| `/todo` | 傾印 `~/.spectyn-mesh/todos.json` |
| `/plan` | 切換 plan-mode（計畫模式）閘控（在你說 `go` 之前拒絕工具） |
| `/show`, `/show <n>` | 列出 / 展開已擷取的工具呼叫 |
| `/density compact\|full` | 工具結果預覽長度 |
| `/theme <name>` | 配色方案 |
| `/perm ask\|allow\|deny\|list\|reset` | 各工具的權限閘門 |
| `/cost` | 目前為止的 session 成本 |

## 5. 常用設定

```bash
SPECTYN_PERM=ask spectyn         # launch with permission-prompt mode on
SPECTYN_DENSITY=compact spectyn  # compact tool results
SPECTYN_MD=0 spectyn             # disable markdown highlight
NO_COLOR=1 spectyn               # disable all ANSI colors
```

## 6. 叢集（Cluster）

把 Tailscale peer（節點）URL 加進 `agents.toml` 的 `[cluster]`，並共用一組密鑰（secret）：

```toml
[cluster]
peers = ["http://100.x.x.2:7878", "http://100.x.x.3:7878"]
cluster_secret = "openssl rand -hex 32"
```

`spectyn coordinator` 會透過 mDNS（多播網域名稱服務）進行零設定的節點探索。

完整的 mesh（網狀網路）故事請見主要的 [README.md](../README.md)，
多節點啟用（multi-node bring-up）的操作導覽請見
[deploy/DEPLOYMENT.md](deploy/DEPLOYMENT.md)。
