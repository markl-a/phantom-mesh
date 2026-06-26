# phantom-mesh — 入門指南

在五分鐘內讓一個可運作的代理（agent）、以及選用的第二台機器組成的 mesh（網狀叢集）運作起來。

---

## 1. 安裝

### 選項 A — 一行指令安裝（推薦用於第二台 Mac）

如果你已經在另一台機器上跑著 phantom-mesh coordinator（協調者）
（姑且稱它為「Mac 1」），這個 coordinator 會透過 HTTP 提供一個
bootstrap（啟動引導）腳本和一個二進位檔。在 Mac 2 上執行：

```bash
curl -fsSL http://<coordinator-tailscale-ip>:7878/scripts/install-mac.sh \
  | COORD=http://<coordinator-tailscale-ip>:7878 bash
```

這會拉取：
- 將 `phantom` 二進位檔放入 `~/.cargo/bin/`
- 一份 cluster-bootstrap（叢集啟動引導）`~/.phantom-mesh/agents.toml`（含 cluster_secret + peers，
  **不含任何 API 金鑰**）
- 一個 launchd 項目，讓 `phantom serve` 在登入時自動啟動

它**不會**動到你的供應商（provider）金鑰 — 之後請在 REPL（讀取-求值-輸出循環互動介面）裡
互動式設定那些金鑰（`/keys add groq` 等）。

需求條件：Apple Silicon（`arm64`）、`curl`，以及（推薦）已加入與 coordinator 同一個
tailnet（Tailscale 網路）的 Tailscale。

### 選項 B — 從原始碼建置

```bash
git clone https://github.com/markl-a/phantom-mesh
cd phantom-mesh/core
cargo build --release --bin phantom
cp target/release/phantom ~/.cargo/bin/phantom
```

確認 `~/.cargo/bin` 在你的 `PATH` 上。首次執行會引導你完成
互動式的供應商設定。

### 驗證

```bash
phantom --version          # prints the build commit + date
phantom doctor             # checks config, binary, daemon, tailnet, peers
```

`phantom doctor` 是最快的「有沒有哪裡出錯」檢查 — 它會逐一巡查每個
子系統，並印出 OK / warn / fail 以及提示。

---

## 2. 最初的 5 分鐘

```bash
phantom                    # launches the TUI
```

你會看到一個金色字、黑底的輸入提示。輸入一則訊息並按 Enter
— 回應會即時串流（streaming）顯示在原處。

接著試試：

```
/help                      # list every slash command
/keys add groq             # paste a Groq API key; it's auto-tested
/model fast                # switch to the fastest available model
What's the current git status of my home dir?
```

最後那個提示應該會觸發 `git_status` 工具並顯示真實輸出。
你完成了 — 本文件其餘部分都是參考資料。

---

## 3. 進階使用者指令

以下指令在 TUI（文字使用者介面）以及 `phantom --repl` 中都可使用
（後者多了一些額外功能，例如互動式選取器）。

| 指令 | 功能說明 |
|---|---|
| `/model` | 顯示目前的模型 + 供應商預設值 |
| `/model fast` / `smart` / `cheap` | 切換到你設定中最快 / 最聰明 / 最便宜的模型 |
| `/model fetch <provider>` | 從供應商拉取即時模型清單（`groq`、`openrouter`、`gemini`、`anthropic`） |
| `/model pick` | 互動式編號選取器（僅限 REPL） |
| `/keys list` | 顯示哪些供應商已設定金鑰 |
| `/keys add <provider>` | 貼上金鑰；存檔前會自動測試 |
| `/keys test <provider>` | 對已儲存的金鑰做煙霧測試（smoke-test，基本可用性測試） |
| `/keys remove <provider>` | 刪除已儲存的金鑰 |
| `/copy` | 將最後一則助理回應複製到剪貼簿 |
| `/copy all` | 複製整個工作階段（session） |
| `/copy turn` | 複製最後一輪使用者+助理的對話 |
| `/export [path]` | 將工作階段儲存為 Markdown |
| `/compact` | 用 LLM（大型語言模型）摘要較舊的對話輪次，逐字保留最後 6 輪 |
| `/sessions` | 列出已儲存的工作階段 |
| `/resume <prefix>` | 依 ID 前綴切換到某個工作階段（不帶參數＝最近的一個） |
| `/fork` | 將目前工作階段分支（branch）成一個新的（REPL） |
| `/plan` | 切換 plan 模式（計畫模式）— 代理在任何工具呼叫前先預覽其計畫 |
| `/agent [name]` | 顯示或切換目前作用中的代理 |
| `/agents` | 列出已設定的代理 |
| `/tools` | 列出作用中代理已啟用的工具 |
| `/perm ask\|allow\|deny` | 工具呼叫的權限模式 |
| `/cost` | 工作階段 + 總計花費的金額、請求次數 |
| `/density compact\|full` | 單行 vs 多行的工具輸出 |
| `/theme <name>` | `dark`、`light`、`claude`、`codex`、`gemini`、`mono` |
| `/init` | 在 cwd（目前工作目錄）產生一份專案 `PHANTOM.md` |
| `/clear` | 清除對話記錄 + 移除工作階段歷史 |
| `/exit` | Ctrl-C 也可以 |

需要阻塞式輸入（blocking input）的 REPL 專屬指令：`/login`、`/logout`、`/add`、
`/undo`、`/keys add`、`/model pick`。如果你想在行模式（line-mode）下使用這些指令
而非 TUI，請執行 `phantom --repl`。

---

## 4. Mesh — 連接另一台機器

phantom-mesh 的核心目的，就是跨你的多台機器派發子代理（subagent）。
設定方式：

### 前置需求：兩台機器都裝 Tailscale

```bash
brew install tailscale
sudo tailscale up
tailscale ip -4              # note this IP — the coordinator URL
```

### 在新機器上

執行 §1A 的一行安裝程式，把 `COORD` 指向既有
機器的 tailscale IP。腳本會寫入 `~/.phantom-mesh/agents.toml`，
內含 cluster_secret 與 peer（對等節點）清單。

### 驗證 mesh

```bash
phantom peer list            # online/offline + active tasks per peer
phantom peer discover        # mDNS + Tailscale scan, no config needed
phantom peer ping http://<peer-ip>:7878
```

### 派工作給某個 peer

```bash
# Send a one-shot job to whichever peer scores best:
phantom peer assign --agent master "summarise the README.md in 5 bullets"

# Async — get a job ID back, poll later:
phantom peer send-async --agent master "long task..."
phantom peer poll http://<peer-ip>:7878 <job-id>
```

在 TUI 中，當作用中代理的 `parallel_tasks` 預算允許時，它能自動
生成（spawn）跨 mesh 的子代理 — 詳見 `/tasks`。

---

## 5. 把 phantom 當成 Claude Code 的子代理使用

`phantom mcp` 透過 stdio（標準輸入輸出）以 MCP（模型上下文協議）溝通，
向上層的 Claude Code 工作階段揭露每一個工具（shell、file_*、
git_*、web_*、memory_*、subagent、parallel_tasks 等）。

在 `~/.claude.json` 裡：

```json
{
  "mcpServers": {
    "phantom": {
      "command": "/Users/<you>/.cargo/bin/phantom",
      "args": ["mcp"]
    }
  }
}
```

接著在 Claude Code 中：

> Use the phantom MCP server to run `cargo test` in `~/projects/foo`,
> and if it fails, open the failing test file and explain the failure.

Claude Code 會呼叫 `mcp__phantom__shell` 來執行測試、並呼叫
`mcp__phantom__file_read` 來開啟檔案。你不必離開既有的編輯器，
就能取得 phantom 的工具沙箱（sandboxing）+ 叢集路由（cluster routing）。

---

## 6. 疑難排解

**建置失敗。** 確認你有最新的 Rust 工具鏈（toolchain）：
```bash
rustup update stable
```

**`phantom serve` 登入時沒有啟動。** 檢查 launchd：
```bash
phantom doctor
launchctl list | grep phantommesh
```
如果該單元（unit）不見了，重新註冊它：
```bash
phantom service install
```

**Mesh peer 離線。** 在怪罪 phantom 之前，先驗證網路路徑：
```bash
tailscale status                       # peer up on the tailnet?
phantom peer ping http://<peer-ip>:7878
curl -fsS http://<peer-ip>:7878/healthz
```
如果 `healthz` 有回應但 `peer ping` 失敗，代表兩端的 cluster_secret
不一致 — 在離線的節點上重新執行安裝腳本。

**免費方案供應商出現「Model not supported」。** 免費供應商
（opencode router、Groq、Gemini）經常下架並輪替模型。重新整理：
```
/model fetch groq
/model fetch openrouter
/model pick
```

**找不到設定檔。** phantom 會（依序）尋找：
1. `$PHANTOM_MESH_CONFIG`（若有設定）
2. `~/.phantom-mesh/agents.toml`
3. `~/Library/Application Support/ai.phantommesh.app/agents.toml`（macOS）
4. `~/.config/phantom-mesh/agents.toml`（Linux）

`phantom doctor` 會印出實際載入的是哪一個路徑。

---

## 後續文件

| 目標 | 閱讀 |
|---|---|
| 多節點 Tailscale 拓撲（topology） | [mesh/TAILSCALE-SETUP.md](mesh/TAILSCALE-SETUP.md) |
| 24/7 雲端節點 | [deploy/DEPLOYMENT.md](deploy/DEPLOYMENT.md)（部署）／ [deploy/DEPLOY-AUTOUPDATE.md](deploy/DEPLOY-AUTOUPDATE.md)（簽章 + release CI + OTA） |
| 架構總覽 | [ARCHITECTURE.md](ARCHITECTURE.md) |
| 各角色設定範本 | [`configs/`](../configs/) |
