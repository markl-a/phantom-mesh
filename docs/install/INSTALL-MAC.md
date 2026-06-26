# macOS 上的 phantom — 安裝指南

搭配 `INSTALL-ANDROID.md` 與 `INSTALL-IOS.md` 一起閱讀。Mac 是
phantom mesh（網狀網路）的推薦 **coordinator（協調者）** — 它內建原生
launchd 開機自動啟動、APFS 快照回滾、MLX（Apple 機器學習框架）本地大型語言模型，
以及最深入的 doctor 診斷功能。

---

## TL;DR（懶人包）— 90 秒

```bash
# 1. One-shot install (no sudo needed; ~/.cargo/bin goes on PATH automatically)
git clone https://github.com/markl-a/phantom-mesh && \
  cd phantom-mesh/core && cargo install --path .

# 2. First-time setup — wizard writes agents.toml, runs doctor, prints next steps
phantom onboarding

# 3. Auto-start at every login (launchd LaunchAgent)
phantom service install

# 4. (Optional) hourly self-improvement loop
phantom autoevolve schedule install

# 5. Verify — expect 11 sections, all ✓ or ⚠ (⚠ is fine for opt-in features)
phantom doctor
```

完成步驟 4 之後，每次登入都會執行 phantom serve，代理程式（agent）每小時
修復失敗的測試，而且你隨時都能用一行指令做診斷，懷疑哪裡出問題時
立刻就能查。

---

## 前置需求（Prereqs）

- macOS 13（Ventura）或更新版本；建議使用 macOS 26（Sequoia/Tahoe）
- 強烈建議使用 Apple Silicon（Apple Silicon 或更新）— 選用的 MLX 裝置端
  大型語言模型必須有它才能跑
- Rust toolchain（工具鏈，`rustup`）以便從原始碼建置
- 至少一家供應商（Anthropic / OpenAI / Groq / Gemini）的有效 API 金鑰
  — phantom 採 BYOK（Bring Your Own Key，自帶金鑰）；我們絕不附帶自己的金鑰

選用但很有用：

- Tailscale 帳號（用於跨裝置的 cluster（叢集））
- Xcode 命令列工具（`xcode-select --install`）— 解鎖
  `xcode_simctl` 工具
- `pip install mlx-lm` — 解鎖裝置端大型語言模型（`phantom mlx`）

---

## 各項目安裝到哪裡

| 路徑 | 用途 |
|---|---|
| `~/.cargo/bin/phantom` | 指向 `core/target/release/phantom` 的符號連結（cargo install） |
| `~/Library/Application Support/phantom-mesh/bin/phantom` | launchd 使用的 TCC 安全副本（在 `phantom service install` 時建立） |
| `~/Library/Application Support/phantom-mesh/{dist,scripts}/` | 鏡像的 repo `dist/` + `scripts/`，供 `/dist/*` 與 `/scripts/*` HTTP 路由使用 |
| `~/Library/LaunchAgents/ai.phantommesh.serve.plist` | `phantom serve` 的 LaunchAgent |
| `~/Library/LaunchAgents/ai.phantommesh.autoevolve.plist` | 每小時 `autoevolve --once` 的 LaunchAgent |
| `~/Library/Logs/phantom-serve.log` | LaunchAgent serve 的標準輸出/標準錯誤 |
| `~/Library/Logs/phantom-autoevolve.log` | LaunchAgent autoevolve 的標準輸出/標準錯誤 |
| `~/.phantom-mesh/agents.toml` | 供應商金鑰、代理程式定義、cluster |
| `~/.phantom-mesh/env` | 可由 shell 載入的密鑰（選用，BYOK 載入器） |
| `~/.phantom-mesh/autoevolve.log` | 每次 autoevolve 迭代的 JSONL 紀錄 |
| `~/.phantom-mesh/costs.json` | 跨多次執行持久保存的 LLM 累計花費 |
| `~/.phantom-mesh/conversations/<id>.jsonl` | 每個 session（工作階段）的持久逐字稿 |

除了二進位檔以外，所有東西都放在 `$HOME` 底下 — 易於備份，
也易於清除。

---

## 詳細安裝

### 1. 建置二進位檔

```bash
git clone https://github.com/markl-a/phantom-mesh
cd phantom-mesh/core
cargo install --path .
phantom --version
# → phantom 0.1.0 (<git-hash>+, macos-aarch64, built YYYY-MM-DD)
```

`cargo install --path .` 會把 `phantom` 放到你的 PATH 上，位置在
`~/.cargo/bin/phantom`。在 Apple Silicon 上第一次建置大約需要 2 分鐘。

### 2. 首次設定精靈

```bash
phantom onboarding
```

互動式 — 詢問你的 Groq / Gemini / Anthropic / OpenAI 金鑰，
寫入 `~/.phantom-mesh/agents.toml`，然後執行 `phantom doctor` 來
確認。最後會印出一個 3 步驟的「next steps（後續步驟）」區塊，告訴你確切
該輸入什麼。

你隨時可以重新執行 `phantom onboarding` 來重新檢視／覆寫設定。

### 3. 開機自動啟動

```bash
phantom service install
```

這是支援 macOS 26 的版本：它會把二進位檔複製到
`~/Library/Application Support/phantom-mesh/bin/phantom`（這樣才符合 TCC
安全要求，因為 `~/Documents` 對 launchd 衍生的程序是被封鎖的），並透過
`launchctl bootstrap` 載入 `ai.phantommesh.serve.plist`。該
服務會立即啟動，並在每次使用者登入時重新啟動，非零結束時會
KeepAlive（保持存活，10 秒節流）。

```bash
phantom service status        # registered/pid/healthz
phantom service uninstall     # bootout + remove plist
```

### 4. 每小時自我改進（選用）

```bash
phantom autoevolve schedule install
```

安裝第二個 LaunchAgent，每小時執行 `phantom autoevolve --once`。
當 `cargo check` 變紅（失敗）時，它會啟動 `phantom evolve` 來
修復；當修復成功變綠時，它會用 `git commit` 提交。過去的迭代
會被當作「什麼有效／什麼無效」的提示，回饋進 LLM 的提示詞裡。

```bash
phantom autoevolve schedule status
phantom autoevolve log --n 10        # last 10 JSONL entries, pretty
phantom autoevolve schedule uninstall
```

### 5.（選用）裝置端大型語言模型

```bash
pip3 install mlx-lm                  # one-time
phantom mlx pull                     # default Llama 3.1 8B 4-bit (~5 GB)
phantom mlx serve                    # foreground at :8080
```

然後加入 `~/.phantom-mesh/agents.toml`：

```toml
[providers.mlx-local]
type          = "openai"
base_url      = "http://127.0.0.1:8080/v1"
api_key       = "mlx"
default_model = "mlx-community/Llama-3.1-8B-Instruct-4bit"

[agent.local]
provider = "mlx-local"
model    = "mlx-community/Llama-3.1-8B-Instruct-4bit"
```

完成後，`phantom autoevolve --once --agent local` 會把整個
自我改進迴圈跑成 **完全在裝置端、完全離線、每個 token 零成本**。
各模型大小的效能表請見 `docs/providers/MLX-PROVIDER.md`。

### 6.（選用）cluster（叢集）

如果你還有其他 Mac / Windows / Linux / Termux 節點：

1. 確認它們全都在同一個 Tailscale tailnet 裡。
2. 在每個 peer（對等節點）上安裝 phantom（正確的二進位檔位於
   `http://<this-mac-ts-ip>:7878/dist/phantom-<target>`）。
3. 在這台 Mac 上，編輯 `~/.phantom-mesh/agents.toml`：
   ```toml
   [cluster]
   node_name      = "mac-coordinator"
   cluster_secret = "<shared-secret-string>"
   peers = [
     "http://<peer-1-ts-ip>:7878",
     "http://<peer-2-ts-ip>:7879",
   ]
   ```
4. 重新啟動服務（`launchctl kickstart -k
   gui/$UID/ai.phantommesh.serve`），並執行 `phantom doctor` — 
   network（網路）那一列應該會顯示你的各個 peer。

接著跨 mesh dispatch（派發）就能運作：`mcp__phantom__subagent({node:
"100.64.0.10:7879", agent: "master", prompt: "..."})`。

---

## 驗證（Verify）

### 1. 快速健康檢查

```bash
phantom doctor
```

`phantom doctor` 會跑 11 個彩色標示的區段。在一個健康的安裝裡，
每一行都是 `✓` 綠色或 `⚠` 黃色。對於你尚未選用的功能（MLX 伺服器、
Spotlight 索引、未使用的 API 金鑰），出現 `⚠` 是正常的。紅色的 `✗`
行表示有東西需要處理。

**在設定良好的 Mac 上的預期輸出**（你的金鑰與節點名稱
會有所不同）：

```
phantom doctor 0.4.0

binary
  ✓ version: phantom 0.4.0 (093b1af4c8+, macos-aarch64, built 2026-05-11)
  ✓ path: /Users/you/.cargo/bin/phantom

config
  ✓ agents.toml: /Users/you/.phantom-mesh/agents.toml
  ✓ ~/.phantom-mesh: exists

permissions
  ⚠ [permissions]: no rules → allow all (legacy default).
                    See docs/PERMISSIONS.md for the Tool(specifier) DSL.

provider keys
  ⚠ Anthropic: not in env or agents.toml
  ✓ Groq: env (gsk_L1…)
  ✓ Gemini: agents.toml
  ⚠ DeepSeek: not in env or agents.toml

phantom serve
  ✓ healthz: 200 OK on http://127.0.0.1:7878/healthz
  ✓ launchd: registered (pid 61585)

network
  ✓ Tailscale: connected (100.x.x.x  your-host  userid:…  macOS  -)

MLX local LLM
  ✓ mlx_lm: importable (`pip install mlx-lm` available)
  ⚠ server: not reachable — `phantom mlx serve`

autoevolve
  ✓ history: last run @ 2026-05-12 07:10 → green (140 total)
  ✓ schedule: registered (LaunchAgent)

identity
  ✓ logged in: you@example.com (Your Name)  via Google  device xxxxxxxx

diagnostics
  ⚠ crash logs: 7 recorded — latest: …/crash-xxxxxxx.log
               › read with: phantom debug last
  ✓ events log: …/events.jsonl (513196 bytes)

tools
  ✓ tools: 54 total (52 built-in + 2 cluster RPC)

macOS integrations
  ✓ APFS snapshots: tmutil reachable (0 snapshots — `phantom snapshot create`)
  ⚠ Spotlight: not indexing /Users/you/repos/phantom-mesh
  ✓ Xcode CLT: installed (xcode_simctl tool ready)

done.
```

**首次安裝時要留意的 ⚠ 行**（屬正常，不是錯誤）：
- `Anthropic: not in env` — 你在 onboarding 時沒選 Anthropic；
  如果你想用它，就把 `ANTHROPIC_API_KEY` 加到 env 或 `agents.toml`
- `MLX server: not reachable` — 除非你執行了 `phantom mlx serve`，否則屬正常
- `Spotlight: not indexing …` — 除非你在 `agents.toml [core].spotlight_paths`
  加入 Spotlight 路徑，否則屬正常
- `crash logs: N recorded` — 在一次糟糕的代理程式執行後可能出現；用
  `phantom debug last` 檢視

**需要修正的紅色 ✗ 行：**
- `agents.toml: not found` → 執行 `phantom onboarding`
- `launchd: not installed` → 執行 `phantom service install`
- `healthz: unreachable` → 執行 `phantom serve` 或 `phantom service install`
- `Tailscale: not in PATH or not connected` → `tailscale up`
- `systemd: no unit installed` → 執行 `phantom service install`

若需機器可讀的輸出（CI（持續整合）／監控／腳本化檢查）：

```bash
phantom doctor --json | jq '.status'       # → "ok" / "warn" / "fail"
phantom doctor --json | jq '.serve'        # port, running, status code
phantom doctor --json | jq '.autoevolve'   # queue + last run timestamp
```

### 2. 執行測試掃描

```bash
./scripts/test-mac.sh        # 51 fast checks, ~30 s
phantom selftest             # 22+ feature checks (TUI, MCP, doctor, dashboard…)
phantom selftest --p0-only   # critical checks only, ~5 s
```

`test-mac.sh` 預期結果為 PASS 51 / FAIL 0 / SKIP ≤ 1。
`phantom selftest` 預期 22+ 通過 / 0 失敗。

### 3. 打開儀表板（dashboard）

```bash
phantom serve &              # if not already running via launchd
open http://127.0.0.1:7878/projects
```

你應該會看到：
- 6 個釘選專案磚塊，每個都有 [Run Demo] 按鈕
- 一條 cluster 狀態列（單機執行時是單一節點，當有 peer
  透過 Tailscale 可連線時會出現更多藥丸狀標籤）
- 一條「Recent activity（近期活動）」帶狀區，顯示 autoevolve 的執行紀錄

點按任一個 [Run Demo] — 輸出會透過 Server-Sent Events（伺服器推送事件）即時串流。

### 4.（選用）接入 Claude Code

`phantom-mesh` 把它的 50+ 個工具以 MCP（模型上下文協議）伺服器的形式
公開出來。要從 Claude Code 使用它們：

```bash
claude mcp add phantom $(which phantom) mcp
```

或者，如果你正在 phantom-mesh repo *內部* 工作，專案內的
`.mcp.json` 會自動註冊 — 只要在第一次啟動 Claude Code session 時
信任那個提示即可。之後，每個工具都會以
`mcp__phantom__file_read`、`mcp__phantom__shell` 等形式出現。

煙霧測試（smoke-test）MCP 的線路格式：
```bash
./scripts/test-mcp-tools.sh   # 13 tool/call e2e checks
```

---

## 更新（Updating）

```bash
phantom self-update                       # pulls /dist/<target> from coord
phantom self-update --source <URL>        # explicit source
phantom self-update --dry-run             # show what would happen
```

self-update 完成後，launchd 服務會自動以
`launchctl kickstart -k` 重新啟動。如果有什麼出錯：

```bash
mv ~/Library/Application\ Support/phantom-mesh/bin/phantom.bak \
   ~/Library/Application\ Support/phantom-mesh/bin/phantom
launchctl kickstart -k gui/$UID/ai.phantommesh.serve
```

---

## 解除安裝（Uninstall）

```bash
phantom autoevolve schedule uninstall
phantom service uninstall
rm ~/.cargo/bin/phantom
rm -rf ~/.phantom-mesh
rm -rf ~/Library/Application\ Support/phantom-mesh
rm    ~/Library/Logs/phantom-{serve,autoevolve}.log
```

這樣就乾淨了。沒有系統層級的狀態、沒有 kext（核心擴充功能）、沒有常駐程式（daemon）。

---

## 疑難排解（Troubleshooting）

完整的踩雷清單（每個我們建置過程中遇到的問題都有一個段落）請見
`docs/TROUBLESHOOTING-MAC.md`。

### `phantom doctor` 快速分流

執行 `phantom doctor`，並依此順序尋找失敗點：

| `phantom doctor` 行 | 原因 | 修正 |
|---|---|---|
| `✗ agents.toml: not found` | 尚未執行 onboarding | `phantom onboarding` |
| `✗ healthz: unreachable` | serve 未執行 | `phantom serve &` 或 `phantom service install` |
| `✗ launchd: not installed` | 服務未設定 | `phantom service install` |
| `⚠ MLX server: not reachable` | 未啟動 | `phantom mlx serve`（首次執行約 5 分鐘下載） |
| `⚠ Spotlight: not indexing …` | 路徑不在設定中 | 把路徑加到 `agents.toml [core].spotlight_paths` |
| `⚠ Tailscale: not in PATH or not connected` | 未登入 | `tailscale up` |
| `⚠ autoevolve/history: no runs yet` | 從未執行過 | `phantom autoevolve --once` |
| `⚠ autoevolve/schedule: not scheduled` | 排程未安裝 | `phantom autoevolve schedule install` |
| `⚠ crash logs: N recorded` | 近期一次代理程式執行崩潰 | `phantom debug last` 讀取最新一筆 |
| `⚠ identity: local-only` | 屬正常（broker（中介伺服器）尚未部署） | 無需修正 — 這是正常的 |
| `✗ [permissions]: parse error` | `agents.toml [permissions]` 區塊有語法錯誤 | 對照 docs/PERMISSIONS.md 檢查 DSL |

最常見、且專屬於 macOS 26 的問題是 TCC 陷阱，已在
commit 65338ab 修復 — 但如果你是跨過那個邊界升級上來的，請執行：

```bash
phantom service uninstall && phantom service install
```

若要對 51 項環境檢查做完整的自動化分流：

```bash
./scripts/test-mac.sh    # tells you exactly which check is failing
```

---

## 效能基準（Apple Silicon 16 GB，macOS 26.3）

| 操作 | 時間 |
|---|---|
| `phantom --version` | < 30 ms |
| `phantom doctor` | ~ 800 ms |
| `phantom mcp` 冷啟動（stdio 握手 + tools/list） | ~ 200 ms |
| `mcp__phantom__subagent({...})` 透過 Groq Llama 3.3 70B 來回 | 2-5 s |
| `mcp__phantom__subagent` 跨 mesh 至 peer | 比本地多 + 200-1000 ms |
| `phantom mlx serve` 冷載入 8B-4bit → 第一個 token | ~ 150 s |
| `phantom mlx serve` 熱狀態 8B-4bit → 50 個 token | ~ 5 s |
| `phantom autoevolve --once`（綠樹，無工作） | ~ 110 s（cargo check 重新建置） |
| `phantom autoevolve --once`（紅樹 → 修復 → 提交） | 視代理程式而定 30-180 s |
| `cargo build --release --bin phantom`（冷） | ~ 2 min |

熱路徑（hot path）的耗時都遠低於一秒；慢的那些受限於 LLM
（網路來回時間佔主要）。
