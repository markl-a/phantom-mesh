# Android（Termux CLI 二進位檔）— 冒煙測試清單（smoke checklist，快速驗證清單）

針對在真實 Android 裝置的 Termux 內，以叢集工作者（cluster worker，叢集中執行任務的節點）身分執行的
`phantom-aarch64-linux-android` CLI 二進位檔，提供端對端（end-to-end，從頭到尾完整流程）驗證程序。

**涵蓋範圍：** 安裝 · CLI 子命令 · 常駐程式（daemon，背景服務）· HTTP/RPC 矩陣 · 內嵌
網頁前端 · MCP（Model Context Protocol，模型上下文協議）stdio · HMAC（雜湊訊息驗證碼）強制驗證 · 真實 LLM（大型語言模型）派工 · 叢集
網狀整合（mesh integration）· ratatui TUI（終端機文字介面）· autoevolve（自動演化）· 持久化（Termux:Boot）·
壓力測試 · 失效模式 · 清理。

**時間預算：** 首次執行約 60–90 分鐘，回歸測試（regression run，重複驗證）約 15 分鐘。

> 配套文件：[INSTALL-ANDROID.md](INSTALL-ANDROID.md) 只涵蓋安裝。
> 本文件假設你已完成該文件的 §B。

---

## Phase 0 · 先決條件（10 分鐘）

```
[ ] Tailscale on phone — VPN icon in status bar, 100.x.y.z assigned
[ ] Termux from F-Droid (NOT Play Store)
[ ] Termux:Boot from F-Droid (optional; needed for Phase 12)
[ ] Mac coordinator at <mac-tailscale-ip>:7878 with phantom serve running
[ ] One usable Groq API key (gsk_…) — Phase 8 needs it
[ ] Out-of-band shell to phone — pick one:
      - adb-tcp:  adb connect 100.64.0.10:38913
      - SSH:      ssh -p 8022 u0_a187@100.64.0.10
                  (requires `pkg install openssh && sshd` in Termux)
```

在手機的 Termux 中，以下兩者都必須成功：
```bash
ping -c 1 <mac-tailscale-ip>
curl -sS http://<mac-tailscale-ip>:7878/healthz   # → ok
```

**通過條件：** 每個核取方塊都打勾，兩個 curl 都成功。

---

## Phase 1 · 透過 Termux 安裝（5 分鐘）

在手機的 Termux 中：

```bash
COORD=http://<mac-tailscale-ip>:7878
GROQ_KEY=gsk_yourkeyhere
curl -fsSL "$COORD/scripts/termux-setup.sh" | sh
```

該腳本會做的事：
- 用 `pkg install` 安裝 curl/wget/git/termux-tools
- 從 `<COORD>/dist/` 拉取最新的 `phantom-aarch64-linux-android`
- 寫入帶有 cluster_secret 與 Groq key 的 `~/.phantom-mesh/agents.toml`
- 在背景啟動 `phantom serve --port 7879`
- 印出三選一選單（TUI / 瀏覽器 / 叢集工作者）

**通過條件：**
```bash
which phantom                  # $PREFIX/bin/phantom
file $(which phantom)          # ELF 64-bit ARM aarch64
ls -lh ~/.phantom-mesh/        # bin/, data/, agents.toml
curl -sS http://127.0.0.1:7879/healthz   # ok
```

---

## Phase 2 · CLI 健全性檢查（5 分鐘）

```bash
phantom --version              # → phantom 0.4.0 (..., android-aarch64, …)
phantom -V                     # same
phantom doctor                 # 9 sections — most ✓ or ⚠

phantom autoevolve log         # "no runs yet" on first boot — fine
phantom evolve goals list      # tries to load EVOLVE-GOALS.md; "not found" is fine
```

預期會因平台限制而失敗（屬正確行為）：

```bash
phantom service status         # ✗ not yet implemented on this platform
phantom snapshot list          # ✗ macOS-only (uses tmutil)
```

**避免這些 — 會讓 shell 卡住的已知 CLI bug：**
```bash
# phantom serve --help     ← actually starts the daemon
# phantom mcp --help       ← also broken; spawns the stdio server
```

**通過條件：** doctor 顯示 8/9 為 ✓ 或 ⚠（沒有 ✗），上述兩個受平台限制的命令以
退出碼 1 結束並顯示預期的訊息。

---

## Phase 3 · 常駐程式（serve）驗證

`termux-setup.sh` 已經啟動了 serve。確認它運作正常：

```bash
PID=$(pgrep -f "phantom serve" | head -1)
echo "PID=$PID"
ss -ltn | grep 7879            # LISTEN 0.0.0.0:7879
grep -E 'VmRSS|Threads' /proc/$PID/status

tail -20 ~/.phantom-mesh/data/phantom-serve.log
```

**通過條件：** PID 存在 · 連接埠正在監聽 · log 中有啟動橫幅（banner）· RSS（常駐記憶體）< 50 MB · 0
條 error/panic/fatal 訊息。

---

## Phase 4 · HTTP / RPC 端點矩陣（10 分鐘）

```bash
PORT=7879
for path in /healthz /rpc/ping /rpc/peers /api/sessions /api/cost /api/todos /api/nodes / /m /static/app.css /static/xterm.css; do
  CODE=$(curl -sSo /dev/null -w '%{http_code}' http://127.0.0.1:$PORT$path)
  printf '  %-30s  %s\n' "$path" "$CODE"
done
```

預期 — 全部為 `200`：

| 路徑 | 用途 |
|---|---|
| `/healthz` | 健康探測（`ok`） |
| `/rpc/ping` | 節點身分 JSON |
| `/rpc/peers` | 對等節點清單 JSON |
| `/api/sessions` | 工作階段清單（很可能是 `[]`） |
| `/api/cost` | 成本摘要 |
| `/api/todos` | 待辦事項 |
| `/api/nodes` | 即時對等節點 ping |
| `/` | 桌面網頁前端（HTML） |
| `/m` | 行動裝置聊天介面（HTML） |
| `/static/app.css` | 內嵌樣式表 |
| `/static/xterm.css` | xterm.js 樣式表 |

預期為 `404`（僅協調者才有 — 工作者不會註冊這些）：
`/dist/<file>`、`/scripts/<file>`、`/api/onboarding/{token,config}`、
`/api/health`、`/api/peers`、`/api/tools`。

**通過條件：** 上述 11 個端點全部回傳 200；那些 404 端點確實回傳 404。

---

## Phase 5 · 在瀏覽器中檢視網頁前端（5 分鐘）

在手機上開啟 Chrome（或任何瀏覽器），前往：

```
http://127.0.0.1:7879/
```

**預期：**
- 標題列顯示 `phantom · mesh`
- 奶油色 / 深色主題的標頭
- xterm.js 終端機面板有正確渲染（深色）
- 可看到 Info 分頁，內含子分頁 Sessions / Cost / Todo / Tools
- 瀏覽器主控台（console）沒有紅色錯誤

```
http://127.0.0.1:7879/m
```

**預期：** 行動裝置聊天介面，底部有導覽列，底部有輸入框。

**通過條件：** 兩個頁面都正確渲染，沒有空白畫面，沒有主控台錯誤。

> 提示：Chrome → ⋮ →「加到主畫面」可取得 PWA（漸進式網頁應用程式）風格的圖示。

---

## Phase 6 · MCP stdio JSON-RPC（10 分鐘）

在 Termux 中：

```bash
echo '{"jsonrpc":"2.0","method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{}},"id":1}' \
  | phantom mcp 2>/dev/null
```

**預期：**
```json
{"id":1,"jsonrpc":"2.0","result":{"capabilities":{"tools":{"listChanged":false}},"protocolVersion":"2024-11-05","serverInfo":{"name":"phantom-mesh","version":"0.4.0"}}}
```

```bash
echo '{"jsonrpc":"2.0","method":"tools/list","id":2}' \
  | phantom mcp 2>/dev/null | head -c 1000
```

**預期：** 工具陣列，開頭為 shell / file_read / file_write / web_fetch / …

**通過條件：** 兩者都回傳形狀正確的有效 JSON-RPC 回應。

**進階：** 把它接進 Mac/Win 上的 Claude Code：

```jsonc
// ~/.claude.json
"mcpServers": {
  "phantom-android": {
    "command": "ssh",
    "args": ["-p", "8022", "u0_a187@100.64.0.10", "phantom mcp"]
  }
}
```

重新啟動 Claude Code 後，`mcp__phantom-android__*` 工具應會出現在
ToolSearch 中。

---

## Phase 7 · HMAC 強制驗證（5 分鐘）

```bash
SECRET=$(grep cluster_secret ~/.phantom-mesh/agents.toml | sed 's/.*"\(.*\)"/\1/')
BODY='{"agent":"master","prompt":"reply: ok"}'
GOOD=$(printf '%s' "$BODY" | openssl dgst -sha256 -hmac "$SECRET" -hex | awk '{print $2}')
PORT=7879

# bad → 401
curl -sS -w 'HTTP %{http_code}\n' -X POST http://127.0.0.1:$PORT/rpc/task/assign \
  -H "X-Cluster-Auth: $(printf '0%.0s' {1..64})" \
  -H 'Content-Type: application/json' -d "$BODY"

# good → {job_id}
curl -sS -X POST http://127.0.0.1:$PORT/rpc/task/assign \
  -H "X-Cluster-Auth: $GOOD" -H 'Content-Type: application/json' -d "$BODY"
```

**通過條件：** bad → `HTTP 401`；good → `{"job_id":"…"}`。

> 當 `cluster_secret` **未**設定（沒有 agents.toml）時，常駐程式會以
> 開發 / 免驗證（no-auth）模式執行，接受任何請求。對於非 localhost 的部署，
> 請務必設定 `[cluster].cluster_secret`。

---

## Phase 8 · 真實 LLM 派工（5 分鐘）

確認 `~/.phantom-mesh/agents.toml` 中有 `[providers.groq].api_key = "gsk_…"`
（真實的 key，而非預留佔位字串）。若仍是佔位字串：

```bash
nano ~/.phantom-mesh/agents.toml   # set api_key
pkill phantom
nohup phantom serve > ~/.phantom-mesh/data/phantom-serve.log 2>&1 &
sleep 4
```

沿用 Phase 7 的 `SECRET` 與 `GOOD`：

```bash
RESP=$(curl -sS -X POST http://127.0.0.1:7879/rpc/task/assign \
  -H "X-Cluster-Auth: $GOOD" -H 'Content-Type: application/json' -d "$BODY")
JOB=$(echo "$RESP" | sed -n 's/.*"job_id":"\([^"]*\)".*/\1/p')
sleep 8
curl -sS http://127.0.0.1:7879/rpc/task/status/$JOB
```

**預期：**
```json
{"status":"done","output":"<llama text>","error":null,"job_id":"…"}
```

**通過條件：** `status=done`、`output` 非空、`error=null`。

> Llama 3.3 70B 的工具使用（tool-use）格式器有時會對 Groq 回傳 400。最乾淨的
> 單次（single-shot）提示為：`reply with at most 8 words`。若要完全消除工具使用，
> 可設定 `[agent.master].tools = []`。

---

## Phase 9 · 叢集網狀整合（10 分鐘）

在手機上：

```bash
# Add Mac as peer
nano ~/.phantom-mesh/agents.toml
# under [cluster] add:
#   peers = ["http://<mac-tailscale-ip>:7878"]
pkill phantom
nohup phantom serve > ~/.phantom-mesh/data/phantom-serve.log 2>&1 &
sleep 4

curl -sS http://127.0.0.1:7879/rpc/peers      # Mac should appear
PHONE_IP=$(tailscale ip -4 | head -1)
echo "phone TS IP: $PHONE_IP"
```

從 **Mac 協調者** 的終端機：

```bash
PHONE_IP=<value from above>
curl -sS http://$PHONE_IP:7879/healthz
curl -sS http://$PHONE_IP:7879/rpc/ping

# Mac → phone HMAC dispatch
# 請設定你自己的共享密鑰（須與各節點 PHANTOM_CLUSTER_SECRET 一致）
SECRET="changeme-cluster-secret"
BODY='{"agent":"master","prompt":"reply: hi from mac"}'
AUTH=$(printf '%s' "$BODY" | openssl dgst -sha256 -hmac "$SECRET" -hex | awk '{print $2}')
RESP=$(curl -sS -X POST http://$PHONE_IP:7879/rpc/task/assign \
  -H "X-Cluster-Auth: $AUTH" -H 'Content-Type: application/json' -d "$BODY")
JOB=$(echo "$RESP" | sed -n 's/.*"job_id":"\([^"]*\)".*/\1/p')
sleep 8
curl -sS http://$PHONE_IP:7879/rpc/task/status/$JOB
```

**通過條件：**
- 手機的 `/rpc/peers` 列出 Mac
- Mac 的 `curl http://$PHONE_IP:7879/healthz` 回傳 `ok`
- 從 Mac → 手機的 HMAC 派工回傳 `status: done`
- Mac 的 `/api/nodes` 在上線清單中顯示手機

---

## Phase 10 · TUI（互動式，10 分鐘）

```bash
phantom            # default = ratatui TUI
```

測試：
- `↑` / `↓` — 輸入歷史
- Tab — 斜線命令 / `@file` 自動完成
- `/help` — 列出 22 個斜線命令
- `/agents` — 來自 agents.toml 的代理
- `/tools` — 49 個工具
- `/density compact`
- 輸入一般提示（`hi`）— 看到逐 token（token-by-token）串流回應
- Ctrl-C 退出

**通過條件：** 每個按鍵綁定都有回應，提示會串流真實的 LLM 輸出，Ctrl-C
乾淨退出。

> 透過 adb shell 渲染的 TUI 有輕微的換行 / 顏色錯位，但仍可運作。測試 TUI 時，
> 請優先使用正規的 Termux 工作階段，而非 adb shell。

---

## Phase 11 · Autoevolve（10 分鐘）

> Autoevolve 假設目前的工作目錄（cwd）有一個 Cargo.toml。在尚未複製（clone）
> phantom-mesh 倉庫的全新 Termux 上，它會優雅地失敗（fail gracefully，安全地結束而不崩潰）。
> 請先 git-clone，或接受無目標（no-target）的結果。

```bash
cd ~
phantom autoevolve --once
phantom autoevolve log --n 3
```

**預期：** 一筆紀錄；若無 Cargo.toml，狀態為 `no-target` 或 `cargo-missing`。

```bash
phantom autoevolve schedule install
phantom autoevolve schedule status
```

**預期：** `not yet implemented on this platform`（Android 沒有
LaunchAgent / systemd）— 屬正確行為。

**通過條件：** autoevolve 執行一次而不崩潰（panic）；schedule install 以預期的
平台訊息失敗。

---

## Phase 12 · Termux:Boot 持久化（15 分鐘，包含手機重開機）

從 F-Droid 安裝 Termux:Boot 後：

```bash
mkdir -p ~/.termux/boot
cat > ~/.termux/boot/phantom-serve <<'EOF'
#!/data/data/com.termux/files/usr/bin/sh
~/.phantom-mesh/bin/phantom serve >> ~/.phantom-mesh/data/phantom-serve.log 2>&1 &
EOF
chmod +x ~/.termux/boot/phantom-serve
```

**重新啟動手機**。在它喚醒後（給它 30 秒）：

```bash
# in Termux
pgrep phantom
curl -sS http://127.0.0.1:7879/healthz
```

**通過條件：** phantom 已自動執行中，且 healthz 回傳 `ok`。

---

## Phase 13 · 壓力 / 耐久測試（30 分鐘）

```bash
# 100 sequential healthz hits
START=$(date +%s%N)
OK=0
for i in $(seq 1 100); do
  R=$(curl -sS http://127.0.0.1:7879/healthz)
  [ "$R" = "ok" ] && OK=$((OK+1))
done
END=$(date +%s%N)
echo "$OK/100 OK, $(( (END-START)/1000000 ))ms total"

# 10 concurrent dispatches (needs Groq key)
# 請設定你自己的共享密鑰（須與各節點 PHANTOM_CLUSTER_SECRET 一致）
SECRET="changeme-cluster-secret"
for i in $(seq 1 10); do
  BODY="{\"agent\":\"master\",\"prompt\":\"echo $i\"}"
  AUTH=$(printf '%s' "$BODY" | openssl dgst -sha256 -hmac "$SECRET" -hex | awk '{print $2}')
  curl -sS -X POST http://127.0.0.1:7879/rpc/task/assign \
    -H "X-Cluster-Auth: $AUTH" -H 'Content-Type: application/json' -d "$BODY" &
done
wait
sleep 10
curl -sS http://127.0.0.1:7879/api/sessions | head -c 400

# memory after 1 h
PID=$(pgrep -f "phantom serve")
grep VmRSS /proc/$PID/status
ls /proc/$PID/fd | wc -l
```

**通過條件：** 100/100 OK，並行派工中 ≥ 8/10 達到 `done`（在 1 GB E2.1.Micro 等級的
帳號上，Groq 可能會限速），閒置 1 小時後 VmRSS 增長 <
5 MB，fd（檔案描述符）數量 < 50。

---

## Phase 14 · 失效模式（20 分鐘）

| # | 注入故障 | 程序 | 預期 |
|---|---|---|---|
| 1 | 錯誤的供應商 key | 編輯 toml，設 `api_key = "gsk_invalid"`，重啟，派工 | status=error，error 包含 401 |
| 2 | 網路中斷 | `tailscale down`，派工 | status=error，error 包含 timeout / connection refused |
| 3 | OOM（記憶體不足）邊界 | 在 1 GB 手機上 5 個並行提示 | ≥ 3 個 done，無崩潰 |
| 4 | `kill -9` 常駐程式 | `pkill -9 phantom`，檢查 log | log 乾淨，socket 已釋放，無崩潰訊息 |
| 5 | 連接埠衝突 | 在第 1 個尚未結束時啟動第 2 個 serve | 第 2 個以退出碼 1 結束並顯示 `Address already in use` |
| 6 | 損壞的 agents.toml | `cluster_secret = `（無值），重啟 | 在綁定（bind）前以退出碼 1 結束並顯示 toml 解析錯誤 |
| 7 | 磁碟滿（Termux home） | `dd if=/dev/zero of=~/big bs=1M count=$(df ~ \| awk 'NR==2{print $4}')` | 派工錯誤被記錄，無崩潰 |
| 8 | Tailscale IP 變更 | `tailscale down; tailscale up` | 重啟常駐程式 → Mac 仍能以新 IP 連到它 |

**通過條件：** 每個情境都*安全地*失敗 — 有錯誤訊息、無崩潰、無
殭屍程序（zombie process），常駐程式存活或乾淨退出。

---

## Phase 15 · 清理（5 分鐘）

```bash
pkill phantom
sleep 2
pgrep phantom || echo "all stopped"

# Optional full uninstall
rm -rf ~/.phantom-mesh
rm -f ~/.termux/boot/phantom-serve

ls ~/.phantom-mesh 2>&1            # No such file or directory
which phantom 2>&1                 # not found
```

**通過條件：** phantom 及其設定目錄已從裝置上消失。

---

## 結果矩陣

逐 phase 追蹤 通過 / 失敗 / 略過：

```
Phase  Title                              Result      Notes
─────  ────────────────────────────────  ──────────  ────────────────────
0      Prerequisites                      [ ]
1      Termux install                     [ ]
2      CLI sanity                         [ ]
3      Daemon serve                       [ ]
4      HTTP/RPC matrix                    [ ]
5      Web frontend (Chrome)              [ ]
6      MCP stdio                          [ ]
7      HMAC enforcement                   [ ]
8      Real LLM dispatch                  [ ]
9      Cluster mesh integration           [ ]
10     TUI                                [ ]
11     Autoevolve                         [ ]
12     Termux:Boot persistence            [ ]
13     Stress / longevity                 [ ]
14     Failure modes (8 cases)            [ ]
15     Cleanup                            [ ]
```

---

## 已知注意事項（截至 2026-05-01）

- `phantom serve --help` 與 `phantom mcp --help` 會啟動常駐程式，而非
  印出用法說明。在前景腳本中請避免使用。
- `phantom serve --port <N>` 會被靜默忽略；常駐程式使用
  `~/.phantom-mesh/agents.toml` 中 `[core].port` 的值（預設 7878）。Termux 設定
  透過該 toml 選用 7879。
- `/api/health`、`/api/peers`、`/api/tools` 在工作者上回傳 404。請改用
  對應的 `/rpc/*` 端點。
- `/dist/<…>`、`/scripts/<…>`、`/api/onboarding/*` 僅協調者才有。
- `/proc/<pid>/status` 中顯示的 `VmPeak` 看起來很驚人（約 12 GB 虛擬記憶體）— 這其實是
  tokio 為工作者堆疊（worker stack）保留的空間。`VmRSS` 才是實際的實體記憶體佔用量
  （閒置約 10 MB，忙碌時 < 50 MB）。
