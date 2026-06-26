# Phantom Mesh — 部署指南

本指南帶你一步步部署 Phantom Mesh，從單一本機節點（local node），一路擴展到完整的 9 台裝置網格（mesh），含一個 24/7 全時運行的雲端節點（cloud node）與透過 Telegram 的行動裝置存取。

> **相關**：要設定簽章金鑰、`git tag` 觸發的 release CI 與 Tauri 自動更新（OTA），請見 [DEPLOY-AUTOUPDATE.md](DEPLOY-AUTOUPDATE.md)。

---

## 目錄

1. [前置需求](#前置需求)
2. [快速開始 — 單一節點](#快速開始--單一節點)
3. [網路拓樸](#網路拓樸)
4. [多節點逐步設定](#多節點逐步設定)
   - [步驟 1：協調者節點](#步驟-1協調者節點)
   - [步驟 2：Tailscale 網格 VPN](#步驟-2tailscale-網格-vpn)
   - [步驟 3：本地工作節點（node-b / node-c）](#步驟-3本地工作節點node-b--node-c)
   - [步驟 4：組成本地叢集](#步驟-4組成本地叢集)
   - [步驟 5：遠端工作節點（Mac worker）](#步驟-5遠端工作節點mac-worker)
   - [步驟 5.5：雲端節點（Oracle / GCP）](#步驟-55雲端節點oracle--gcp)
   - [步驟 6：透過 Telegram 進行行動存取](#步驟-6透過-telegram-進行行動存取)
   - [步驟 7：完整網格驗證](#步驟-7完整網格驗證)
5. [設定範本](#設定範本)
6. [Telegram 機器人設定](#telegram-機器人設定)
7. [疑難排解](#疑難排解)

---

## 前置需求

| 項目 | 取得處 | 費用 |
|------|----------------|------|
| Tailscale 帳號 | https://tailscale.com | 免費（最多 100 台裝置） |
| Anthropic API 金鑰 | https://console.anthropic.com | 按用量計費 |
| Telegram 機器人權杖（bot token） | Telegram 上的 @BotFather | 免費 |
| Oracle Cloud 帳號 | https://cloud.oracle.com | 免費方案（Always Free VM，永久免費虛擬機） |
| Rust 工具鏈（toolchain） | https://rustup.rs | 免費（用於從原始碼建置） |

---

## 快速開始 — 單一節點

5 分鐘內讓單一節點運行起來。

```bash
# 1. Build the daemon
cargo build --release -p phantom-mesh-daemon

# 2. Copy the example config
#   macOS
cp agents.toml.example ~/Library/Application\ Support/ai.phantommesh.app/agents.toml
#   Linux
cp agents.toml.example ~/.config/phantom-mesh/agents.toml

# 3. Edit the config — set at least one provider API key
$EDITOR ~/.config/phantom-mesh/agents.toml

# 4. Start the daemon
./target/release/phantom-mesh daemon

# 5. Verify it's running
curl http://localhost:7878/health
# → {"status":"ok"}
```

---

## 網路拓樸

```
                    ┌─────────── Tailscale Mesh VPN ───────────┐
                    │                                           │
  Local LAN         │   Remote                                  │   Cloud
  ┌──────────┐      │   ┌──────────┐                           │   ┌──────────────┐
  │ node-a      │◄────►│◄─►│ Mac worker   │                           │◄─►│ Oracle Cloud  │
  │ coord    │      │   │ worker   │                           │   │ cloud node   │
  ├──────────┤      │   └──────────┘                           │   │ Telegram bot │
  │ node-b     │◄────►│                                           │   └──────┬───────┘
  │ worker   │      │   ┌───────────────────────────────────┐  │          │
  ├──────────┤      │   │ Mobile (Telegram clients)         │  │          │
  │ node-c   │◄────►│   │  node-d  iPhone  iPad  MiPad      │──┼──────────┘
  │ worker   │      │   └───────────────────────────────────┘  │  Telegram API
  └──────────┘      └───────────────────────────────────────────┘
```

**節點角色：**

| 節點 | 角色 | Tailscale IP |
|------|------|-------------|
| node-a（Windows） | 協調者（Coordinator）— 主要開發樞紐 | 100.x.x.1 |
| node-b（Windows） | 工作者（Worker）— 儲存 + 備援推論 | 100.x.x.2 |
| node-c（Windows） | 工作者 — 邊緣 GPU 推論 | 100.x.x.3 |
| Mac worker（macOS） | 工作者 — 輕量推論 | 100.x.x.4 |
| Oracle Cloud VM（Linux ARM） | 雲端節點 — 24/7、Telegram 機器人 | 100.x.x.5 |
| node-d / iPhone / iPad / MiPad | 行動裝置 — 僅 Telegram 用戶端 | 100.x.x.6-9 |

---

## 多節點逐步設定

請依序執行這些步驟。每一步通過驗證後再進行下一步。

---

### 步驟 1：協調者節點

先把主協調者（node-a 或 Mac）設定為一個獨立運行的常駐程式（daemon）。

1. 在協調者機器上建置並安裝常駐程式的二進位檔（binary）。

2. 建立設定目錄並複製協調者範本：

   ```bash
   # macOS
   mkdir -p ~/Library/Application\ Support/ai.phantommesh.app
   cp configs/agents.coordinator.toml \
      ~/Library/Application\ Support/ai.phantommesh.app/agents.toml

   # Windows (PowerShell)
   New-Item -ItemType Directory -Force "$env:APPDATA\ai.phantommesh.app"
   Copy-Item configs\agents.coordinator.toml `
             "$env:APPDATA\ai.phantommesh.app\agents.toml"
   ```

3. 編輯設定檔 — 填入你的 Anthropic API 金鑰，以及（選用）Brave 搜尋金鑰。

4. 啟動常駐程式：
   ```bash
   # macOS / Linux
   ./phantom-mesh daemon

   # Windows
   .\phantom-mesh.exe daemon
   ```

**驗收標準：**
```
curl http://localhost:7878/health  →  {"status":"ok"}
```

---

### 步驟 2：Tailscale 網格 VPN

在每一台機器上安裝 Tailscale，讓它們跨網路擁有穩定、可路由（routable）的 IP。

1. 安裝 Tailscale：
   - Windows / macOS：從 https://tailscale.com/download 下載
   - Linux：`curl -fsSL https://tailscale.com/install.sh | sh`
   - Android / iOS：從應用程式商店安裝 Tailscale app

2. 在每台裝置上以相同的 Tailscale 帳號登入：
   ```bash
   tailscale up
   ```

3. 記下每台裝置的 Tailscale IP：
   ```bash
   tailscale status
   ```

4. 用真實 IP 更新每個節點 `agents.toml` 中的 `cluster.peers`。

**驗收標準：**
```bash
tailscale ping node-b     # replies with latency
tailscale ping node-c   # replies
tailscale ping mac-worker   # replies
tailscale status        # all devices show "online"
```

---

### 步驟 3：本地工作節點（node-b / node-c）

將常駐程式部署到每一台本地 Windows 工作機。

1. 將常駐程式二進位檔複製到每台機器（透過 LAN 共享，或經 Tailscale 用 SCP）：
   ```bash
   scp phantom-mesh.exe user@node-b-tailscale-ip:C:/phantom-mesh/
   ```

2. 複製工作者設定範本：
   ```bash
   scp configs/agents.worker.toml \
       user@node-b-tailscale-ip:"%APPDATA%/ai.phantommesh.app/agents.toml"
   ```

3. 在每台工作機上編輯 `agents.toml`：
   - 將 `node_name` 設為機器名稱（例如 `"node-b"` 或 `"node-c"`）
   - 用協調者的 Tailscale IP 更新 `cluster.peers`

4. （選用）安裝 Ollama 以進行本地推論：
   ```bash
   # On Windows: download from https://ollama.ai
   # Pull a model
   ollama pull qwen2.5-coder:14b
   ```

5. 在每台工作機上啟動常駐程式：
   ```batch
   .\phantom-mesh.exe daemon
   ```

**驗收標準：**
```bash
curl http://<node-b-tailscale-ip>:7878/health   # → {"status":"ok"}
curl http://<node-c-tailscale-ip>:7878/health # → {"status":"ok"}
# If Ollama installed:
curl http://<node-b-ip>:11434/api/tags          # → lists models
```

---

### 步驟 4：組成本地叢集

將 3 個本地節點（node-a + node-b + node-c）連接成一個叢集（cluster）。

1. 在每個節點上編輯 `agents.toml`，加入含所有對等節點（peer）IP 的 `[cluster]` 區段。可參考 `configs/agents.coordinator.toml` 中的範本。

2. 選擇一組強度足夠的共享密鑰（shared secret），並在所有節點上使用相同的值：
   ```bash
   # Generate a random secret
   openssl rand -hex 32
   ```

3. 在每個節點的設定中，把 `cluster.cluster_secret` 設為這個值。

4. 重新啟動全部 3 個節點上的常駐程式。

**驗收標準：**
```
node-a → node-b:   rpc.ping → pong
node-a → node-c: rpc.ping → pong
node-b → node-a:   rpc.ping → pong (bidirectional)
Cluster shows 3 nodes online
Coordinator election completed
```

---

### 步驟 5：遠端工作節點（Mac worker）

將 Mac worker 經由 Tailscale 加入為遠端工作者。

1. 在 Mac 上建置常駐程式：
   ```bash
   git clone <repo-url> phantom-mesh && cd phantom-mesh
   cargo build --release -p phantom-mesh-daemon
   ```

2. 複製工作者設定：
   ```bash
   mkdir -p ~/Library/Application\ Support/ai.phantommesh.app
   cp configs/agents.worker.toml \
      ~/Library/Application\ Support/ai.phantommesh.app/agents.toml
   ```

3. 編輯 `agents.toml`：
   - 設定 `node_name = "mac-worker"`
   - 將協調者與雲端的 IP 加入 `cluster.peers`
   - 將 `cluster.cluster_secret` 設為共享密鑰

4. 啟動常駐程式。

**驗收標準：**
```bash
curl http://<mac-tailscale-ip>:7878/health  # → {"status":"ok"}
# node-a → Mac: rpc.ping → pong (cross-network)
# Cluster shows 4 nodes online
```

---

### 步驟 5.5：雲端節點（Oracle / GCP）

設定 24/7 全時運行的雲端節點。此節點同時也運行 Telegram 機器人。

#### Oracle Cloud 免費方案設定

1. 在 https://cloud.oracle.com 註冊（需信用卡進行驗證；在 Always Free 方案下不會產生任何費用）。

2. 建立一個 **Ampere A1 ARM** 執行個體（instance）：
   - 規格（Shape）：`VM.Standard.A1.Flex`
   - OCPU：4，RAM：24 GB（Always Free 的最大配額）
   - 作業系統：Ubuntu 22.04（aarch64）
   - 加入你的 SSH 公鑰

3. 在 Oracle 安全清單中開放 7878 連接埠（Networking > VCN > Security Lists）。

#### 部署常駐程式

```bash
# On your dev machine: cross-compile for aarch64 Linux
rustup target add aarch64-unknown-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu -p phantom-mesh-daemon

# Copy binary to Oracle VM
scp target/aarch64-unknown-linux-gnu/release/phantom-mesh \
    ubuntu@<oracle-public-ip>:~/phantom-mesh
```

或者，直接在 VM 上建置：
```bash
ssh ubuntu@<oracle-public-ip>
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
git clone <repo-url> phantom-mesh && cd phantom-mesh
cargo build --release -p phantom-mesh-daemon
```

#### 設定雲端節點

```bash
ssh ubuntu@<oracle-public-ip>
mkdir -p ~/.config/phantom-mesh
cp configs/agents.cloud.toml ~/.config/phantom-mesh/agents.toml
nano ~/.config/phantom-mesh/agents.toml
# - Set cluster.peers with Tailscale IPs of all other nodes
# - Set cluster.cluster_secret
```

#### 設定環境變數

建立 `/etc/phantom-mesh.env`：
```
ANTHROPIC_API_KEY=sk-ant-YOUR_KEY
TELEGRAM_BOT_TOKEN=123456:YOUR_BOT_TOKEN
```

#### 安裝為 systemd 服務

```bash
sudo tee /etc/systemd/system/phantom-mesh.service > /dev/null <<EOF
[Unit]
Description=Phantom Mesh Daemon
After=network-online.target tailscaled.service
Wants=network-online.target

[Service]
Type=simple
User=ubuntu
EnvironmentFile=/etc/phantom-mesh.env
ExecStart=/home/ubuntu/phantom-mesh daemon
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable --now phantom-mesh
```

#### 在 Oracle VM 上安裝 Tailscale

```bash
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up
# Note the Tailscale IP — update cluster.peers on all other nodes
```

**驗收標準：**
```bash
ssh oracle-vm                                          # can SSH in
curl http://<oracle-tailscale-ip>:7878/health         # → {"status":"ok"}
# node-a → Oracle: rpc.ping → pong
# Cluster shows 5 daemon nodes online
# Turn off node-a → send Telegram message → Oracle node replies (24/7 verification)
```

---

### 步驟 6：透過 Telegram 進行行動存取

雲端節點上的 Telegram 機器人為所有裝置提供行動存取。

1. 確認雲端節點正在運行，且已設定 `[telegram]`（見下方 [Telegram 機器人設定](#telegram-機器人設定)）。

2. 在每台行動裝置上開啟 Telegram，依使用者名稱找到你的機器人。

3. 送出一則測試訊息。

**驗收標準：**
```
node-d    → "你好" → reply received
iPhone    → "你好" → reply received
iPad      → same
MiPad     → same
```

---

### 步驟 7：完整網格驗證

執行一次完整的端到端（end-to-end）測試，過程中完全不需電腦介入：

1. 從 iPhone 的 Telegram 送出：`"幫我研究 2026 年 AI agent 市場趨勢，整理成報告"`
2. Oracle 雲端節點透過 Telegram 接收這個任務
3. 任務被委派給某個 GPU 工作者（node-c 或 Mac）進行推論
4. `web_search` 工具執行該研究
5. 結果經彙整後回傳到 Telegram
6. 整個流程完成，全程不需碰任何電腦

預期的 `tailscale status`：
```
node-a        100.x.x.1   online   Windows    daemon (coordinator)
node-b       100.x.x.2   online   Windows    daemon (worker)
node-c     100.x.x.3   online   Windows    daemon (worker, GPU)
mac-worker     100.x.x.4   online   macOS      daemon (worker)
oracle-vm  100.x.x.5   online   Linux ARM  daemon (cloud, 24/7)
node-d     100.x.x.6   online   Android    Telegram client
iphone     100.x.x.7   online   iOS        Telegram client
ipad       100.x.x.8   online   iOS        Telegram client
mipad      100.x.x.9   online   Android    Telegram client
```

---

## 設定範本

`configs/` 中提供了三份即用型範本：

| 檔案 | 適用對象 | 關鍵設定 |
|------|-----|-------------|
| `configs/agents.coordinator.toml` | node-a / Mac（主開發機） | 全部 13 個工具、叢集對等節點、Anthropic 為主 |
| `configs/agents.cloud.toml` | Oracle Cloud / GCP VM | 啟用 Telegram 機器人、`host = "0.0.0.0"`、Haiku 模型 |
| `configs/agents.worker.toml` | node-b / node-c / Linux 工作者 | Ollama 為主 + Anthropic 備援、無 git 工具 |

所有範本都包含 `[cluster]` 區段。請將 `100.x.x.*` 佔位符（placeholder）替換為真實的 Tailscale IP，並在所有節點上設定一組共享的 `cluster_secret`。

---

## Telegram 機器人設定

1. 開啟 Telegram，傳訊息給 **@BotFather**。

2. 送出 `/newbot`，依提示選擇一個名稱與使用者名稱。

3. BotFather 會給你一個類似 `123456789:ABCdef...` 的權杖。

4. 在雲端節點上把該權杖設為環境變數：
   ```bash
   # Add to /etc/phantom-mesh.env
   TELEGRAM_BOT_TOKEN=123456789:ABCdef...
   ```

5. 取消雲端節點 `agents.toml` 中 `[telegram]` 區段的註解：
   ```toml
   [telegram]
   bot_token_env = "TELEGRAM_BOT_TOKEN"
   allowed_users = []         # empty = public; or list numeric user IDs for private bot
   agent = "master"
   ```

6. 重新啟動常駐程式。機器人現在已上線。

**提示：** 要找出你的 Telegram 使用者 ID，傳訊息給 @userinfobot。將你的 ID 加入 `allowed_users`，即可讓機器人變為私有：
```toml
allowed_users = [123456789]   # your numeric user ID
```

---

## 疑難排解

### 常駐程式無法啟動

```bash
# Check logs
journalctl -u phantom-mesh -f   # Linux systemd
# or run interactively:
./phantom-mesh daemon 2>&1 | tee daemon.log
```

常見原因：
- 設定中缺少或無效的 API 金鑰
- 7878 連接埠已被佔用：`lsof -i :7878`
- 找不到設定檔 — 請檢查各平台專屬的路徑

### 健康檢查失敗

```bash
curl -v http://localhost:7878/health
```

- 若連線被拒（connection refused）：常駐程式未在運行
- 若出現 401/403：檢查所有節點上的 `cluster_secret` 是否一致
- 若逾時（timeout）：防火牆封鎖了 7878 連接埠

### 節點之間無法經由 Tailscale 互相連通

```bash
tailscale status           # check all nodes show "online"
tailscale ping <node-name> # test connectivity
```

- 確認兩端節點上的 Tailscale 都在運行：`sudo systemctl status tailscaled`
- 檢查雲端供應商的防火牆／安全群組（security group）中 7878 連接埠是否已開放
- 在 Oracle Cloud 上：確認 VCN 安全清單有一條針對 TCP 7878 的入站（ingress）規則

### Telegram 機器人沒有回應

- 檢查雲端節點上的常駐程式是否在運行：`curl http://localhost:7878/health`
- 確認 `TELEGRAM_BOT_TOKEN` 已設定：`echo $TELEGRAM_BOT_TOKEN`
- 檢查常駐程式記錄中是否有 Telegram 相關錯誤
- 確認 `[telegram]` 中的 `bot_token_env` 與環境變數名稱完全一致

### Ollama 在工作節點上無法運作

```bash
# Check Ollama is running
curl http://localhost:11434/api/tags

# Pull the model if missing
ollama pull qwen2.5-coder:14b

# Check provider config matches the model name exactly
```

### Oracle 免費方案 ARM VM 記憶體不足

Ampere A1 的 Always Free 配額為 4 OCPU / 24 GB RAM — 必要時請改用較小的模型：

```toml
[providers.ollama]
default_model = "qwen2.5-coder:7b"   # lighter than 14b
```

---

## 預估設定時間

| 步驟 | 時間 |
|------|------|
| 步驟 1 — 協調者 | 約 30 分鐘 |
| 步驟 2 — Tailscale | 約 30 分鐘 |
| 步驟 3 — 本地工作者 | 約 45 分鐘 |
| 步驟 4 — 本地叢集 | 約 30 分鐘 |
| 步驟 5 — Mac worker | 約 45 分鐘 |
| 步驟 5.5 — Oracle Cloud | 約 1–2 小時（含帳號設定 + 部署） |
| 步驟 6 — Telegram 行動裝置 | 約 15 分鐘 |
| 步驟 7 — 完整驗證 | 約 15 分鐘 |
| **總計** | **約 4–5 小時** |
