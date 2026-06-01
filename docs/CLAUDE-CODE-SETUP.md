# Claude Code → phantom-mesh MCP 設定（公司 Mac）

把你的 `phantom` 二進位檔接進 Claude Code，作為 MCP（Model Context Protocol，模型上下文協議）伺服器。完成後，Claude Code 就能把全部 45 個 phantom 工具（shell、file_edit、content_search、web_fetch、http_client、cargo_test、git_*、ask_user、bash_run_background、agent runner（代理執行器）等）當作原生工具呼叫。

## 1. 前置條件檢查清單

```bash
# 1. 把 phantom 二進位檔安裝在一個穩定的路徑（build/copy 或 symlink 皆可）。確認：
which phantom    # expect: /usr/local/bin/phantom

# 2. 在你的 shell rc（~/.zshrc）裡 export 你的 provider（供應商）金鑰：
export GROQ_API_KEY=... GEMINI_API_KEY=... OPENCODE_API_KEY=... ANTHROPIC_API_KEY=...

# 3. 對 MCP 傳輸層做煙霧測試（按 Ctrl-C 結束；應印出 "phantom MCP server started"）：
phantom mcp
```

## 2. 要放進 `~/.claude.json` 的 JSON 片段

打開 `~/.claude.json`，在 `mcpServers` 底下新增一個 `phantom` 項目。如果 `mcpServers` 還不存在，就在最上層建立它。

### 選項 A — 從你的 shell 繼承環境變數（建議）

最乾淨的做法。Claude Code 會把 `phantom` 當作子行程（child process）啟動，你在 `~/.zshrc` 裡 export 的金鑰會自動流進去。`env` 物件保持空白，因此機密（secret）永遠不會出現在 JSON 裡。

```json
{
  "mcpServers": {
    "phantom": {
      "command": "/usr/local/bin/phantom",
      "args": ["mcp"],
      "env": {}
    }
  }
}
```

### 選項 B — 在 JSON 裡明確指定環境變數

如果 Claude Code 是從 GUI 情境（Spotlight、dock）啟動、不會繼承你互動式 shell 的環境變數，就用這個做法。把佔位符（placeholder）換成實際值。把 `~/.claude.json` 當作機密檔案對待（`chmod 600 ~/.claude.json`）。

```json
{
  "mcpServers": {
    "phantom": {
      "command": "/usr/local/bin/phantom",
      "args": ["mcp"],
      "env": {
        "GROQ_API_KEY": "<YOUR_GROQ_KEY>",
        "GEMINI_API_KEY": "<YOUR_GEMINI_KEY>",
        "OPENCODE_API_KEY": "<YOUR_OPENCODE_KEY>",
        "ANTHROPIC_API_KEY": "<YOUR_ANTHROPIC_KEY>"
      }
    }
  }
}
```

注意事項：
- `args` 必須是 `["mcp"]` — 這會選用 stdio（標準輸入輸出）MCP 子命令（規格版本 2024-11-05）。
- 路徑必須是絕對路徑。MCP 啟動器不會展開 `~` 和 `$HOME`。
- `GROQ_API_KEY` / `GEMINI_API_KEY` / `OPENCODE_API_KEY` / `ANTHROPIC_API_KEY` 至少要設定其中一個，否則 agent 工具會拒絕啟動（shell/file/git 工具沒有金鑰也能運作）。

## 3. 驗證

```bash
# 1. Fully restart Claude Code (quit, not just close window).
# 2. Inside Claude Code, list registered MCP servers:
/mcp
# Expected:  phantom   connected   45 tools
```

然後在 Claude Code 對話裡觸發一次工具呼叫：

> Use the phantom `shell` tool to run `date` and show me the output.

你應該會看到 Claude 呼叫 `mcp__phantom__shell`（或類似名稱）並回傳當前日期。再試一個：

> Use phantom's `content_search` tool to find "TODO" in /tmp.

## 4. 疑難排解

| 症狀 | 原因／修法 |
|---|---|
| `/mcp` 顯示 `phantom: failed to start` | `command` 路徑錯誤。執行 `which phantom`，把絕對路徑原封不動貼進 JSON。不要用 `~` 或 `$HOME`。 |
| Claude Code 記錄裡出現 `permission denied` | 執行 `chmod +x /usr/local/bin/phantom`。若是從下載的 zip 用 `cp` 安裝的，也要清掉隔離（quarantine）旗標：`xattr -d com.apple.quarantine /usr/local/bin/phantom`。 |
| 伺服器啟動了，但 agent 工具報錯 `no providers configured` | 環境變數沒被讀到。改用選項 B（在 JSON 裡明確指定 env），或從終端機啟動 Claude Code：在已 export 金鑰的 shell 裡執行 `open -a "Claude Code"`。 |
| stderr 出現 `EADDRINUSE`／連接埠衝突 | 有一個殘留的 `phantom serve` 還在 7878 埠上跑。執行 `pkill -f "phantom serve"` 後重新載入 `/mcp`。`mcp` 子命令本身用 stdio、不綁定任何連接埠。 |
| `tools/list` 回傳 0 個工具 | 你用的是舊版 build。從 `phase1-r1-foundations` 或更新的版本重新編譯（`cargo build --release -p phantom-mesh --bin phantom`），再重新複製二進位檔。 |
| Claude Code 在第一次工具呼叫就卡住 | 在終端機手動執行 `phantom mcp` 並貼入 JSON-RPC 的 `initialize` 請求來檢查 stderr。如果它在那裡也卡住，代表二進位檔壞了，請重新編譯。如果手動能跑、但從 Claude Code 不行，代表環境變數被剝除了 — 改用選項 B。 |
| 工具能用，但連不到區域網路（LAN）對等節點（peer，如 node-a 等） | 公司 Mac 上的 Tailscale 沒啟動，或 `agents.toml` 裡的 peer URL 在公司網路無法連到。先用 `tailscale ping <peer>` 測試。 |
