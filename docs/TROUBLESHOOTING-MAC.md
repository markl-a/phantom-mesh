# spectyn 在 macOS 上的疑難排解（Troubleshooting）

如果有什麼出錯，**先從 `spectyn doctor` 開始**——它會在同一個畫面上揭露
二進位檔來源（binary provenance）、設定檔位置、provider 金鑰、healthz、launchd
狀態、Tailscale、工具數量、tmutil、Spotlight，以及 Xcode CLT（命令列工具）。

```bash
spectyn doctor
```

以下內容都是針對「我跑了 doctor，它告訴我 X——接下來怎麼辦？」的情境。

---

## healthz 在 :7878 上無法連線

**doctor 列**：`⚠ healthz: unreachable on :7878`

1. 檢查 daemon（常駐程式）：`spectyn service status`
2. 如果顯示 `registered : no`，安裝它：`spectyn service install`
3. 如果顯示 `registered : yes` 但 `healthz : unreachable`，表示 launchd
   程序還活著但沒有在監聽。這幾乎一定是下面兩個 TCC（透明度、同意與控制機制）
   陷阱之一。

### 陷阱 #1 — 二進位檔位於 ~/Documents 底下（TCC 封鎖 dyld）

**症狀**：`spectyn service status` 顯示 `registered: yes, pid: N`，
但 `lsof -nP -iTCP:7878` 沒有顯示任何 LISTEN，且日誌只顯示
橫幅（banner）到「Registering Service ...」之後就沒有後續了。

**原因**：macOS 26（Sequoia / Tahoe）的 TCC 會封鎖 launchd 衍生的
程序去載入位於 `~/Documents`、`~/Downloads`、`~/Desktop` 底下的二進位檔。
dyld 動態連結器（dynamic linker）會卡在
`__open`，在到達 `main()` 之前就停住。`launchctl print` 仍然回報
`state = running`，因為程序確實有衍生——只是卡住了。

**修復**：自 65338ab 起，`spectyn service install` 會把二進位檔複製
到 `~/Library/Application Support/spectyn-mesh/bin/spectyn`（不受 TCC
限制），並將 plist 指向那裡。重新執行 `install` 以採用
該路徑：

```bash
spectyn service uninstall
spectyn service install
```

### 陷阱 #2 — 工作目錄（cwd）位於 ~/Documents 底下（TCC 封鎖 getcwd）

**症狀**：與陷阱 #1 相同的「橫幅出現後就停住」模式，但 `lsof
-p <pid>` 顯示 `cwd /Users/.../Documents/...`。`sample <pid>` 顯示
程序卡在 `find_config()` 內部呼叫的
`std::env::current_dir`。

**原因**：同一個 TCC 子系統也會在受保護的路徑上封鎖 `getcwd()`。

**修復**：自 65338ab 起，相同的 `spectyn service install` 也會在 install
是從 `~/Documents`、`~/Downloads` 或 `~/Desktop` 內部執行時，
覆寫 plist 的 `WorkingDirectory` 為 `~/Library/Application Support/
spectyn-mesh`。

### 陷阱 #3 — 連接埠已被佔用

**症狀**：launchd 衍生的 daemon 沒有監聽，但從終端機手動
執行 `spectyn serve` 卻可以。

**原因**：另一個 spectyn serve 已經佔用了 :7878（通常是
先前互動式執行所留下的殘留程序）。`axum::serve` 在綁定（bind）失敗時不會
panic（崩潰）——它只會記錄日誌並繼續，使得監聽器（listener）
未被附掛。

**修復**：
```bash
pkill -f "spectyn serve"
launchctl kickstart -k gui/$(id -u)/ai.spectynmesh.serve
```

---

## launchd 在重開機後不會自動啟動

**doctor 列**：`⚠ launchd: not installed`

```bash
spectyn service install
```

如果 install 成功，但重開機後仍然沒有啟動：

1. `tail -50 ~/Library/Logs/spectyn-serve.log`——尋找橫幅
2. 如果沒有橫幅，表示二進位檔本身載入失敗——見陷阱 #1
3. 如果橫幅出現後就停住，見陷阱 #2
4. 如果二進位檔路徑不再存在（例如你對 target/release/ 執行了
   `cargo clean`），重新安裝服務：`spectyn service install`
   會重新整理被複製的二進位檔。

---

## ROG / Android worker 無法連線

**在 ROG 上執行 doctor**（在 Termux 中）：

```bash
~/.spectyn-mesh/bin/spectyn doctor
```

常見問題：

1. **手機上的 Tailscale 未連線**——在裝置上開啟 Tailscale app，
   登入到同一個 tailnet
2. **Termux 程序在背景被殺掉**——Android 的電池
   最佳化會殺掉 spectyn serve；在「設定 → 應用程式 → Termux」中，
   把電池設定為「不受限制」（Unrestricted）
3. **agents.toml 中的 Coordinator URL 錯誤**——安裝腳本
   預設為 `http://<mac-tailscale-ip>:7878`（原始的 Mac
   coordinator，協調者）。如果你的 Mac TS IP 變更了，編輯
   `~/.spectyn-mesh/agents.toml` 並重新啟動。

要從 coordinator 重新引導（re-bootstrap）：

```bash
COORD=http://<NEW-COORD-IP>:7878 \
  curl -fsSL "$COORD/scripts/termux-setup.sh" | sh
```

---

## spectyn MCP 從 Claude Code / Codex 無法連線

**doctor 不會直接揭露這一項**，但如果 `mcp__spectyn__*`
工具在 Claude Code 中失敗，或 `codex mcp list` 沒有顯示 spectyn：

1. `cat ~/.claude.json | grep -A5 spectyn`——必須顯示 stdio 與路徑
2. `cat ~/.codex/config.toml | grep -A2 spectyn`——必須顯示
   `[mcp_servers.spectyn]`
3. 如果缺少就重新註冊：
   ```bash
   claude mcp add spectyn $(which spectyn) mcp
   codex mcp add spectyn -- $(which spectyn) mcp
   ```
4. 重新啟動 Claude Code 工作階段——MCP 伺服器是在
   工作階段啟動時衍生的，當二進位檔變更時不會熱重載（hot-reload）
5. 執行 `./scripts/validate-mcp.sh` 以確認二進位檔本身
   是健康的

---

## Provider 金鑰未載入

**doctor 列**：`⚠ Anthropic / OpenAI / Groq / Gemini: not in env`

1. 確認 `~/.spectyn-mesh/env` 存在且包含 `KEY=value` 行
2. 為目前的 shell 載入它：
   ```bash
   set -a; source ~/.spectyn-mesh/env; set +a
   ```
3. 讓它自動載入：附加到 `~/.zshrc` /
   `~/.bashrc`：
   ```bash
   [ -f ~/.spectyn-mesh/env ] && set -a && source ~/.spectyn-mesh/env && set +a
   ```
4. 對於 launchd 衍生的 daemon，金鑰必須放在 plist 的
   `EnvironmentVariables` 區塊中。預設安裝只注入
   `PATH` 與 `HOME`——機密（secrets）刻意不被包含進去；請改以
   `~/.spectyn-mesh/agents.toml` 的 `[providers.*]` 區塊來設定
   它們。

---

## Spotlight `spotlight_search` 沒有回傳任何結果

**doctor 列**：`⚠ Spotlight: not indexing /Users/.../spectyn-mesh`

```bash
sudo mdutil -i on /Users/<you>/path/to/spectyn-mesh
sudo mdutil -E /Users/<you>/path/to/spectyn-mesh
```

等待約 30 秒讓索引重建，然後重新執行 `spotlight_search`。

`spotlight_search` 會優雅地降級（fall back）——如果沒有結果，工具
會印出一個指向 `mdutil` 的提示。

---

## Xcode 工具（`xcode_simctl`）回報缺失

**doctor 列**：`⚠ Xcode CLT: missing`

```bash
xcode-select --install
```

`xcode_simctl` 不需要完整的 Xcode——只需要提供
`xcrun simctl` 的命令列工具。安裝完成後，再次執行
`spectyn doctor`。

---

## Subagent / parallel_tasks 回傳「runtime not initialised」

**原因**：你正在執行一個舊版的 spectyn 二進位檔（早於 48bb842），其中
`spectyn mcp` 的 stdio 路徑忘了呼叫 `subagent::init_global()`。

**修復**：重新建置（rebuild）並重新安裝：

```bash
cd /path/to/spectyn-mesh/core
cargo build --release --bin spectyn
spectyn service uninstall
spectyn service install
```

然後重新啟動任何已經衍生了 MCP 伺服器的 Claude Code / Codex
工作階段——它們不會即時採用新的二進位檔。

---

## subagent 預算超支得離譜

**症狀**：你設定 `max_cost_usd: 0.10`，而任務卻花了 $0.98。

**原因**：早於 72b34f7 時，預算是在 agent 迴圈返回之後才**事後**
檢查的。形同虛設。

**修復**：從包含 72b34f7 的程式樹重新建置——現在預算
會在每一輪（round）輪詢，並在下一輪邊界處中斷退出。
預期的超支約為 10%（LLM 在檢查落地前會多完成一輪），
而不是 10 倍。

---

## 在以上任何操作之後重新驗證

```bash
spectyn doctor
./scripts/validate-mcp.sh
```

兩者都應該全綠。如果還是有什麼不對勁，相關的
日誌是 daemon 的 `~/Library/Logs/spectyn-serve.log`，並在重現問題時
以 `tail -f` 監看。
