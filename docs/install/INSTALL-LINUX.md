# 在 Linux 上安裝 phantom-mesh

已在 **Ubuntu 22.04 / 24.04** 與 **Debian 12 (bookworm)** 上測試通過。其他
以 glibc（GNU C 函式庫）為基礎的發行版（Fedora、Arch、openSUSE）應該也能運作 —— 此二進位檔
採用近似靜態連結（statically-ish linked），但你會需要 `glibc 2.31+` 與 OpenSSL。

若是 Alpine / 以 musl（輕量 C 函式庫）為基礎的發行版，你會需要另一個 static-musl
（靜態 musl）建置 —— 目前尚未放在 `dist/` 中；請以
`cargo build --release --target x86_64-unknown-linux-musl` 進行交叉編譯（cross-compile）。

---

## TL;DR（懶人包）—— 60 秒

```bash
# As a normal user (sudo only for systemd install)
sudo apt-get update && sudo apt-get install -y curl ca-certificates git build-essential pkg-config libssl-dev

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

git clone https://github.com/markl-a/phantom-mesh && cd phantom-mesh/core
cargo install --path . --locked

phantom onboarding              # interactive ~90s wizard
phantom serve &                  # or set up systemd unit (below)
phantom doctor                   # verify all green
```

接著在瀏覽器開啟 `http://127.0.0.1:7878/projects` —— 你應該會
看到 6 個帶有 [Run Demo] 按鈕的方塊（tile）。

---

## 先決條件（Prereqs）

| 元件 | 用途 | 取得方式 |
|---|---|---|
| Rust toolchain ≥ 1.80 | 從原始碼建置 phantom | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| `git`                  | Clone 此 repo（程式碼倉庫）                      | `apt install git` |
| OpenSSL 開發標頭檔（dev headers）    | 部分 Rust 相依套件會針對系統 OpenSSL 編譯 | `apt install libssl-dev pkg-config` |
| `build-essential`      | 原生相依套件所需的 gcc + make          | `apt install build-essential` |
| Tailscale（選用）   | 跨機叢集（cluster）與行動裝置存取 | https://tailscale.com/download/linux |

**在 4 核心 8 GB 的 VPS（虛擬私人伺服器）上全新安裝的總時間：** 約 6 分鐘（主要花在
cargo build）。

---

## 詳細安裝

### 1. 建置二進位檔

```bash
cd ~/Documents
git clone https://github.com/markl-a/phantom-mesh
cd phantom-mesh/core
cargo install --path . --locked
phantom --version
```

`cargo install` 會把二進位檔放在 `~/.cargo/bin/phantom`。若該路徑
不在你的 `$PATH` 中，rustup 安裝程式已在 `~/.profile` 加入一行
把它加進去 —— 執行 `source ~/.profile` 或重新開啟 shell。

### 2. 首次導引設定（onboarding）

```bash
phantom onboarding
```

90 秒的互動式精靈（wizard）。會設定：
- `~/.phantom-mesh/agents.toml`，並帶一個供應商（provider，由你挑選 ——
  Anthropic / OpenAI / Groq / OpenRouter / OpenCode / 等等）
- 預設代理（agent）組合（master、coder、reviewer、researcher）
- 用於 HMAC（雜湊訊息驗證碼）對等節點驗證的叢集密鑰（cluster secret）

若你是單機作業，可跳過叢集區段；之後可再編輯 `agents.toml`
來加入對等節點（peer）。

### 3. 以 systemd 使用者單元（user unit）執行（這樣登出後仍能存活）

phantom 的 `service install` 會寫入一個 systemd `--user` 單元：

```bash
phantom service install
systemctl --user status phantom-serve     # should be active (running)
```

該單元位於 `~/.config/systemd/user/phantom-serve.service`，並
執行 `phantom serve`，失敗時會自動重啟。若要在沒有作用中
登入工作階段的情況下開機自動啟動（無頭伺服器，headless server）：

```bash
sudo loginctl enable-linger $USER
```

### 4.（選用）每小時自動演化（autoevolve）

```bash
phantom autoevolve schedule install --interval 3600
systemctl --user status phantom-autoevolve.timer
```

autoevolve 計時器每小時執行一次 `phantom autoevolve --once`：
檢查 cargo 是否為紅燈（red），若是則派出修復代理（fix agent），綠燈時提交（commit）。

日誌位於 `~/.local/state/phantom-mesh/autoevolve.log`（或
`~/.phantom-mesh/autoevolve.log`，視 XDG state 而定）。

### 5.（選用）叢集

編輯 `~/.phantom-mesh/agents.toml`：

```toml
[cluster]
node_name      = "linux-1"
cluster_secret = "<same secret across nodes>"
peers = [
  "http://<mac-tailscale-ip>:7878",      # mac (over Tailscale)
  "http://100.64.0.10:7879",      # windows-node-a
]
```

若你尚未這麼做，請先執行 `tailscale up`。接著驗證連通性：

```bash
curl http://<peer-tailscale-ip>:7878/healthz
```

---

## 驗證

### 1. 快速健康檢查

```bash
phantom doctor
```

`phantom doctor` 在 Linux 上會執行 11 個彩色標示的區段
（binary、config、permissions、provider keys、phantom serve、
systemd、network、autoevolve、identity、diagnostics、tools）。
每一行都應該是 `✓` 綠燈或 `⚠` 黃燈。對於你尚未啟用的
功能（未使用的供應商金鑰、尚未執行的 autoevolve），出現 `⚠` 是預期內的。紅色 `✗`
的行則需要修正。

**在健康的 Linux 安裝上的預期輸出：**

```
phantom doctor 0.4.0

binary
  ✓ version: phantom 0.4.0 (093b1af4c8+, linux-x86_64, built 2026-05-11)
  ✓ path: /home/you/.cargo/bin/phantom

config
  ✓ agents.toml: /home/you/.phantom-mesh/agents.toml
  ✓ ~/.phantom-mesh: exists

permissions
  ⚠ [permissions]: no rules → allow all (legacy default).
                    See docs/PERMISSIONS.md for the Tool(specifier) DSL.

provider keys
  ⚠ Anthropic: not in env or agents.toml
  ✓ Groq: env (gsk_L1…)

phantom serve
  ✓ healthz: 200 OK on http://127.0.0.1:7878/healthz
  ✓ systemd: phantom-serve.service active

network
  ✓ Tailscale: connected (100.x.x.x  your-host  …  linux  -)

autoevolve
  ⚠ history: no runs yet — `phantom autoevolve --once`
  ⚠ schedule: not scheduled — `phantom autoevolve schedule install`

identity
  ✓ identity: local-only (broker not deployed yet — login becomes available
              once phantommesh.io/healthz returns 200)

diagnostics
  ✓ crash logs: 0 (no panics recorded)
  ✓ events log: /home/you/.phantom-mesh/events.jsonl (0 bytes)

tools
  ✓ tools: 54 total (52 built-in + 2 cluster RPC)

done.
```

**首次安裝時需留意的 ⚠ 行**（正常現象，非錯誤）：
- `Anthropic: not in env` —— 你在 onboarding 期間沒有選它；
  如有需要，可把 `ANTHROPIC_API_KEY` 加到 env 或 `agents.toml`
- `autoevolve/history: no runs yet` —— 首次執行前的預期狀況；
  以 `phantom autoevolve --once` 修正
- `autoevolve/schedule: not scheduled` —— 若你跳過了該步驟，這是正常的
- `identity: local-only (broker not deployed)` —— 預期狀況；位於
  phantommesh.io 的 broker（中介伺服器）尚未上線，因此登入功能尚未可用

**需要修正的紅色 ✗ 行：**
- `agents.toml: not found` → 執行 `phantom onboarding`
- `healthz: unreachable` → 執行 `phantom serve` 或 `systemctl --user start phantom-serve`
- `systemd: no unit installed` → 執行 `phantom service install`
- `Tailscale: not in PATH or not connected` → `sudo tailscale up`

若需要機器可讀（machine-readable）的輸出：

```bash
phantom doctor --json | jq '.status'       # "ok" / "warn" / "fail"
phantom doctor --json | jq '.serve'         # port, running, status
phantom doctor --json | jq '.autoevolve'   # queue + last run
```

### 2. 開啟儀表板（dashboard）

```bash
xdg-open http://127.0.0.1:7878/projects
```

應該會顯示 6 個專案方塊 + 叢集狀態列 + 近期活動。
每個 [Run Demo] 都會透過 SSE（伺服器發送事件，Server-Sent Events）即時串流輸出。

### 3. 功能全面測試（feature sweep）

```bash
phantom selftest                # 22+ feature checks
phantom selftest --p0-only       # critical checks only, ~3 s
./scripts/test-mac.sh           # works on Linux too (51 checks)
./scripts/test-mcp-tools.sh     # 13 MCP tool/call e2e checks
```

---

## 與 Claude Code 的 MCP 整合

```bash
claude mcp add phantom $(which phantom) mcp
```

完成後，Claude Code 的工具面板（tool palette）會新增 `mcp__phantom__*` 工具
（file_read、shell、content_search、git_*、task、subagent、…）。

煙霧測試（Smoke-test）：
```bash
./scripts/test-mcp-tools.sh    # 13 checks; expect all pass
```

---

## 更新

```bash
cd ~/Documents/phantom-mesh
git pull
cd core && cargo install --path . --locked
systemctl --user restart phantom-serve.service
```

---

## 解除安裝（Uninstall）

```bash
phantom autoevolve schedule uninstall
phantom service uninstall
rm -rf ~/.phantom-mesh ~/.local/state/phantom-mesh
cargo uninstall phantom-mesh
# Optional: rm -rf ~/Documents/phantom-mesh   # clone itself
```

---

## 疑難排解（Troubleshooting）

### `phantom doctor` 快速分流（triage）

執行 `phantom doctor`，並依此順序尋找故障：

| `phantom doctor` 行 | 原因 | 修正方式 |
|---|---|---|
| `✗ agents.toml: not found` | 尚未執行 onboarding | `phantom onboarding` |
| `✗ healthz: unreachable` | serve 未執行 | `phantom serve &` 或 `systemctl --user start phantom-serve` |
| `⚠ autoevolve/history: no runs yet` | 從未執行過首次執行 | `phantom autoevolve --once` |
| `⚠ autoevolve/schedule: not scheduled` | 尚未安裝排程 | `phantom autoevolve schedule install` |
| `⚠ Tailscale: not in PATH` | 尚未安裝 Tailscale | `curl -fsSL https://tailscale.com/install.sh \| sh` |
| `⚠ Tailscale: not connected` | 尚未登入 | `sudo tailscale up` |
| `⚠ crash logs: N recorded` | 近期某次代理執行當機 | `phantom debug last` 讀取最新一筆 |
| `⚠ identity: local-only` | 預期狀況（broker 尚未部署） | 無需修正 —— 這是正常的 |
| `⚠ events.jsonl: 0 bytes` | 首次執行時的預期狀況 | 無需修正 —— 這是正常的 |
| `✗ [permissions]: parse error` | `agents.toml [permissions]` 區塊中的語法問題 | 檢查 docs/PERMISSIONS.md 中的 DSL（領域特定語言） |

### 其他 shell 層級的故障

| 症狀 | 修正方式 |
|---|---|
| `cargo install` 在 `openssl-sys` 失敗 | `apt install libssl-dev pkg-config` 後重試 |
| `cargo install` 失敗並提示找不到 `link.exe` | 你在 WSL 上 —— 改用 `cargo build --target x86_64-unknown-linux-gnu` |
| 安裝後出現 `phantom: command not found` | `source ~/.cargo/env` 或把 `~/.cargo/bin` 加到 PATH |
| `systemctl --user start phantom-serve` 顯示 "Failed to connect to bus" | 未啟用 lingering —— 執行 `sudo loginctl enable-linger $USER` 後重試 |
| `phantom autoevolve --once` 因無 API 金鑰而當機 | 在 `agents.toml` 中設定 `api_key_env = "GROQ_API_KEY"`（或你的供應商），匯出該環境變數後重試 |
| 連接埠（port）7878 已被佔用 | 在 agents.toml 中設定 `[core] port = 7879` |
| `phantom doctor` 輸出亂碼（ANSI 控制碼） | 透過 `cat -v` 或 `less -R` 過濾；終端機可能不支援彩色 |

---

## 效能基準（Performance baseline，Ubuntu 24.04、AMD EPYC 4 vCPU、8 GB RAM）

| 操作 | 時間 |
|---|---|
| 全新 `cargo install --path .`（首次建置） | 約 5-6 分鐘 |
| 編輯 1 個檔案後的增量重建（incremental rebuild） | 4-8 秒 |
| `phantom doctor` 冷啟動 | 約 1 秒 |
| `phantom selftest --p0-only` | 約 3 秒 |
| `phantom autoevolve --once`（綠燈路徑） | < 5 秒 |
| `phantom mcp` 啟動 → tools/list 回應 | < 200 毫秒 |
| HTTP `/api/projects` 冷啟動 | < 50 毫秒 |
