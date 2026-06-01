# Phantom Mesh 的 Tailscale 設定

本指南教你把多台執行 phantom-mesh daemon（背景服務程式）的裝置，透過 Tailscale 連成單一的運算 mesh（網狀叢集）。

---

## 概觀

Tailscale 會在你的各裝置之間建立一個以 WireGuard 為基礎的 VPN（虛擬私人網路）mesh。每台裝置都會取得一個穩定的 `100.x.x.x` IP，無論 NAT（網路位址轉換）、防火牆或網路變動如何都能運作。Phantom Mesh 使用這些 IP，讓 daemon 節點能直接呼叫彼此的 HTTP API。

**為什麼用 Tailscale，而不是普通 VPN 或連接埠轉發（port forwarding）？**

- 不需要在家用路由器或雲端供應商上設定連接埠轉發
- IP 在網路變動時依然穩定（筆電在不同 Wi-Fi 網路間移動、手機切換到 LTE）
- 單一帳號可免費連接最多 100 台裝置
- 在網路層提供加密傳輸 —— phantom-mesh 的 HMAC（雜湊訊息驗證碼）驗證再加上一道應用層的第二因素

---

## 拓樸選項

選擇符合你硬體的設定方式：

| 拓樸 | 裝置 | 你能得到什麼 |
|----------|---------|--------------|
| **最小化** | Mac + GCP/Oracle Linux VM | 開發機 + 雲端節點；透過雲端達成 24/7 不間斷運行 |
| **行動存取** | Mac + 雲端 VM + iPhone | Telegram bot 跑在雲端；用手機控制 mesh |
| **完整 mesh** | 所有裝置 | 把任務分派到任一節點；完整的容錯能力 |

以下步驟涵蓋最常見的情境：一台 Mac 作為 coordinator（協調者），一台 Linux 雲端 VM 作為永遠開機的節點，並可選擇透過 Telegram 加入 iPhone。

---

## 步驟 1：在所有裝置上安裝 Tailscale

**macOS：**
```bash
brew install tailscale
# or download the app from https://tailscale.com/download
sudo tailscaled &   # if installed via brew
tailscale up
```

**Linux（GCP / Oracle Cloud / 任何以 Debian 為基礎的發行版）：**
```bash
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up
```

**iOS / Android：**
從 App Store 或 Google Play 安裝 Tailscale app，並用同一個帳號登入。

在每台裝置上登入同一個 Tailscale 帳號。同一帳號下的所有裝置會組成單一網路。

---

## 步驟 2：取得 Tailscale IP

在每台裝置執行完 `tailscale up` 之後：

```bash
tailscale ip -4
```

或列出你網路中的所有裝置：

```bash
tailscale status
```

記下每台機器的 `100.x.x.x` 位址 —— 你之後會把這些填進 `agents.toml`。

範例輸出：
```
100.64.0.10   mac-coordinator  you@  macOS   -
100.64.0.11   gcp-worker       you@  linux   -
```

---

## 步驟 3：在每個節點上設定 agents.toml

每個節點都需要一個 `[cluster]` 區段，列出其 peer（對等節點）的 Tailscale IP，以及一個用於驗證的共享密鑰（shared secret）。

首先，產生一個高強度的共享密鑰（只跑一次，所有節點都用同一個值）：

```bash
openssl rand -hex 32
```

**Mac coordinator**（macOS 上為 `~/Library/Application Support/ai.phantommesh.app/agents.toml`，Linux 上為 `~/.config/phantom-mesh/agents.toml`）：

```toml
[core]
host = "0.0.0.0"      # listen on Tailscale interface so peers can reach it
port = 7878
node_name = "mac-coordinator"

[providers.anthropic]
type = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"
default_model = "claude-sonnet-4-6"

[cluster]
node_name = "mac-coordinator"
cluster_secret = "your-shared-secret"   # output from openssl rand -hex 32
peers = ["http://100.x.y.z:7878"]       # GCP node Tailscale IP

[agent.master]
provider = "anthropic"
model = "claude-sonnet-4-6"
tools = ["shell", "file_read", "file_write", "file_edit", "content_search",
         "glob_search", "web_search", "memory_store", "memory_recall",
         "git_status", "git_diff", "git_commit", "git_log"]
instructions = "You are a senior software engineer AI assistant. ALWAYS use tools to accomplish tasks."
```

**GCP / 雲端 worker**（`~/.config/phantom-mesh/agents.toml`）：

```toml
[core]
host = "0.0.0.0"      # must listen on 0.0.0.0 to accept cluster traffic
port = 7878
node_name = "gcp-worker"

[providers.anthropic]
type = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"
default_model = "claude-haiku-4-5"

[cluster]
node_name = "gcp-worker"
cluster_secret = "your-shared-secret"   # same value as coordinator
peers = ["http://100.a.b.c:7878"]       # Mac Tailscale IP

[telegram]
bot_token_env = "TELEGRAM_BOT_TOKEN"
allowed_users = [123456789]             # your Telegram user ID
agent = "master"

[agent.master]
provider = "anthropic"
model = "claude-haiku-4-5"
tools = ["shell", "file_read", "file_write", "web_search", "memory_store", "memory_recall"]
instructions = "You are an AI assistant on a 24/7 cloud node. Respond concisely for mobile users."
```

一份可直接使用的 coordinator 範本在 `configs/agents.coordinator.toml`，雲端範本則在 `configs/agents.cloud.toml`。

**重要：** 在任何需要接受來自 peer 之傳入連線的節點上，都要設定 `host = "0.0.0.0"`。預設的 `127.0.0.1` 只接受本機連線。

---

## 步驟 4：啟動兩個 daemon

**在 Mac 上：**
```bash
export ANTHROPIC_API_KEY=sk-ant-...
phantom-mesh daemon
```

**在 GCP VM 上：**
```bash
export ANTHROPIC_API_KEY=sk-ant-...
export TELEGRAM_BOT_TOKEN=123456789:ABCdef...
phantom-mesh daemon
```

在每個節點上驗證健康狀態：

```bash
# from your Mac, check the GCP node
curl http://<GCP_TAILSCALE_IP>:7878/health
# → {"status":"ok"}

# from the GCP VM, check the Mac
curl http://<MAC_TAILSCALE_IP>:7878/health
# → {"status":"ok"}
```

---

## 步驟 5：測試跨節點任務

Phantom Mesh 使用 SHA-256 HMAC 來驗證節點之間的 RPC（遠端程序呼叫）。`X-Cluster-Auth` 標頭攜帶驗證 token（權杖）。

提交一個任務給 Mac coordinator，並讓它在 GCP worker 上執行：

```bash
# Compute the HMAC token
BODY='{"agent":"master","prompt":"What is the hostname of this machine?"}'
SECRET="your-shared-secret"
TOKEN=$(echo -n "${BODY}" | openssl dgst -sha256 -hmac "${SECRET}" | awk '{print $2}')

# Submit to the GCP worker directly
curl -X POST http://<GCP_TAILSCALE_IP>:7878/rpc/task/assign \
  -H "Content-Type: application/json" \
  -H "X-Cluster-Auth: ${TOKEN}" \
  -d "${BODY}"
# → {"job_id":"..."}

# Poll for result
curl http://<GCP_TAILSCALE_IP>:7878/rpc/task/status/<job_id>
```

或者提交給 coordinator，讓它自行委派：

```bash
curl -X POST http://localhost:7878/agent/master/run \
  -H "Content-Type: application/json" \
  -d '{"prompt": "Ask the gcp-worker node what its hostname is"}'
```

---

## 步驟 6：加入 Telegram 以支援行動存取

Telegram bot 讓你不必打開筆電，就能用任何手機控制 mesh。

1. 打開 Telegram 並傳訊給 `@BotFather`。傳送 `/newbot`，依照提示選擇名稱與使用者名稱。

2. BotFather 會給你一個像 `123456789:ABCdef...` 的 token。

3. 在永遠開機的雲端節點上設定它：
   ```bash
   export TELEGRAM_BOT_TOKEN=123456789:ABCdef...
   ```
   若要讓 daemon 持續運行，可把它加進 `/etc/phantom-mesh.env`，並在你的 systemd unit（系統服務單元）中引用它。

4. 把 `[telegram]` 區塊加進雲端節點的 `agents.toml`（上面步驟 3 已示範）。

5. 傳訊給 `@userinfobot` 取得你的 Telegram 使用者 ID。把它填進 `allowed_users`，讓這個 bot 變成私有。

6. 重新啟動雲端節點上的 daemon。打開 Telegram，依使用者名稱找到你的 bot，並傳訊給它。

雲端節點負責處理 Telegram 的輪詢（polling），並把任務轉發進 mesh。Telegram 要能運作，Mac 不需要保持上線。

---

## 疑難排解

### 節點無法互相連線

```bash
# Check all devices appear in your Tailscale network
tailscale status

# Test reachability directly
tailscale ping gcp-worker    # replace with device name from tailscale status
```

如果 `tailscale ping` 成功但連接埠 7878 無法連通：

- **雲端供應商防火牆：** 為連接埠 7878 新增一條 TCP 傳入（ingress）規則。
  - GCP：VPC network > Firewall > 為 TCP 7878 新增規則
  - Oracle Cloud：Networking > VCN > Security Lists > 為 TCP 7878 新增 Ingress Rule
- **Daemon 監聽在錯誤的介面上：** 在接收連線的節點上，確認 `[core]` 中設定了 `host = "0.0.0.0"`。

### Tailscale ACL 阻擋了流量

如果你的 Tailscale 帳號有自訂 ACL（存取控制清單），請新增一條規則來放行連接埠 7878 的流量：

```json
{
  "action": "accept",
  "src":    ["*"],
  "dst":    ["*:7878"]
}
```

在 [login.tailscale.com/admin/acls](https://login.tailscale.com/admin/acls) 編輯 ACL。

### 叢集驗證錯誤（401 / 403）

- 確認 `cluster_secret` 在每個節點上都完全相同 —— 把 `openssl rand -hex 32` 的輸出原封不動地複製貼上。
- 檢查密鑰值中沒有多餘的尾端空格或換行符。

### Tailscale 沒有在執行

```bash
# Linux
sudo systemctl status tailscaled
sudo systemctl start tailscaled
tailscale up

# macOS (brew install)
sudo tailscaled &
tailscale up
```

### Daemon 在雲端 VM 上無法啟動

```bash
# Run interactively to see errors
./phantom-mesh daemon 2>&1 | tee /tmp/phantom.log

# Check the log for:
# - "config file not found" → wrong path
# - "provider error" → bad or missing API key
# - "address already in use" → port 7878 taken: lsof -i :7878
```

### Telegram bot 沒有回應

```bash
# Verify the daemon is running on the cloud node
curl http://localhost:7878/health

# Check the bot token is set
echo $TELEGRAM_BOT_TOKEN

# Check daemon logs for Telegram errors
journalctl -u phantom-mesh -f   # if running under systemd
```

---

## 以 systemd 服務的方式運行（雲端節點）

要讓 daemon 在登出後與重新開機後仍持續運行：

```bash
sudo tee /etc/phantom-mesh.env > /dev/null <<EOF
ANTHROPIC_API_KEY=sk-ant-YOUR_KEY
TELEGRAM_BOT_TOKEN=123456789:YOUR_TOKEN
EOF

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

---

關於完整的多節點部署逐步說明（coordinator、本地 worker、雲端 VM），請見 [DEPLOYMENT.md](DEPLOYMENT.md)。
