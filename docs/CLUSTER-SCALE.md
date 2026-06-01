# Cluster Scale — 加 5-N 台機器到 mesh 的 SOP

> Designed for the 4→8 node case. Same pattern works for 8→16, 16→32 etc.;
> only thing that breaks at very large N is the dashboard UI getting cluttered.

---

## TL;DR — 加一台新機器

**在新機器上**：
```powershell
# Windows
iwr -useb https://phantommesh.io/install.ps1 | iex
```
```bash
# macOS / Linux
curl -fsSL https://phantommesh.io/install.sh | sh
```

新機器自動：
1. 下載 phantom binary
2. `phantom login`（如果 broker_token 還新就短路）
3. 拉 8 個 LLM key + cluster_secret + peers list
4. 偵測 Tailscale IP + hostname → POST `/api/me/cluster-peers/upsert` → 進 vault
5. 重新拉 peers（含自己） → `cluster join` → 寫 [cluster] block

**在所有舊機器上**（讓他們知道有新成員）：
```powershell
phantom cluster sync
```

完成。任一台跑 `phantom cluster status` 都會看到 N+1 個 peers 全部 ✓。

---

## 詳細流程

### 1. 新機器準備

需要 Tailscale 已裝好 + 已 auth 加進你的 tailnet：
```bash
tailscale ip -4   # 應該回 100.x.y.z 一行
```

如果 hostname 不是你想要的 cluster 名字（例如 hostname = `DESKTOP-ABC123`，你想叫它 `homeserver`）：
```powershell
# Windows
[Environment]::SetEnvironmentVariable('PHANTOM_NODE_NAME', 'homeserver', 'User')
# 開新 PowerShell（要拿到 User scope 的新 env）
```
```bash
# macOS / Linux
echo "export PHANTOM_NODE_NAME=homeserver" >> ~/.zshrc
source ~/.zshrc
```

### 2. 安裝 + 自動上線

```powershell
iwr -useb https://phantommesh.io/install.ps1 | iex
```

預期輸出（重點看末尾）：
```
✓ pulled 9 keys from https://phantommesh.io
  cluster peers: 4 synced → ...peers.json

◆ auto-registering this machine on cluster…
  ✓ registered as 'homeserver' → http://100.x.y.z:7878
✓ joined cluster as 'homeserver'
  agents.toml: ...
  peers (4):
    "http://<mac-tailscale-ip>:7878",       # mac-coordinator
    "http://100.64.0.10:7879",       # node-a
    "http://100.64.0.11:7878",     # node-b
    "http://100.64.0.12:7878",    # node-c
```

### 3. 通知舊機器

新機器加完之後，**舊機器的 peers.json 還沒更新**。每台舊機器跑：

```powershell
phantom cluster sync
```

這條會：
- `phantom config pull` 拉最新 peers list（含新機器）
- `phantom cluster join <自己的名字>` 重寫 [cluster] block

或者每台老機器跑 `phantom login`（短路 OAuth → refresh keys → auto-register self → cluster join），效果一樣。

### 4. 驗證

任一台跑：
```powershell
phantom cluster status
```

應該看到 **N+1 個 peer**（含 mac coordinator + 包含新機器），全部 ✓。

或在 dashboard：開 https://phantommesh.io/account → **Cluster peers** 區塊 → 看到所有 peer + last-seen badge：
- 🟢 `alive` (< 5min)
- 🟡 `stale` (5min - 1hr)
- 🔴 `offline` (> 1hr)
- ⚪ `never` (註冊過但沒 ping 過 broker)

---

## 8 台 Topology 範例

| name | OS | role | URL |
|---|---|---|---|
| mac-coordinator | macOS | reference / Mac dev | http://<mac-tailscale-ip>:7878 |
| node-a | Win11 | desktop dev | http://100.64.0.10:7879 |
| node-b | Win11 | handheld | http://100.64.0.11:7878 |
| node-c | Win11 | mobile dev | http://100.64.0.12:7878 |
| **homeserver** | Linux | 24/7 background tasks | http://100.x.x.x:7878 |
| **node-c** | macOS | mac build farm | http://100.x.x.x:7878 |
| **node-d** | Android (Termux) | mobile worker | http://100.64.0.13:7879 |
| **vm-cloud** | Linux | EC2 / GCP burstable | http://100.x.x.x:7878 |

每台跑同一條安裝指令，差別只在：
- `PHANTOM_NODE_NAME` env var 給它友善名字
- `[core].port` 在 agents.toml 裡可調（預設 7878；衝突的話改 7879 等）

---

## 大 N 的注意事項

### Cluster status RPC 量
- 8 台 × parallel ping = 仍然 < 3s（max 是慢的那台 timeout）
- 派任務 `/rpc/task/assign` 是 1-to-1，不會因 N 大爆炸

### Vault 容量
- D1 free tier: 5GB storage, 100k writes/day, 5M reads/day
- 8 台每天 `config pull` 100 次 = 800 reads/day，0.016% utilisation。N=80 也撐得住

### Tailscale 限制
- Free plan: 100 devices, 3 users
- 你目前 9 devices（含手機平板）— 還差 91 才碰天花板
- 大 cluster 直接掛同一個 tailnet 即可

### 命名衝突
- `phantom cluster join <name>` 會把你 register 成那個 name
- 兩台用同 name 會在 broker 的 user_cluster_peers PK (user_id, name) 上衝突 → 第二台的 upsert **覆蓋** 第一台的 URL
- 所以名字要 unique。新機器跑 `iwr | iex` 之前先想好 PHANTOM_NODE_NAME

### 不同地理區域延遲
- Tailscale relay 延遲若 > 50ms，cluster status 會顯示明顯。看 RTT 分佈知道哪台網路差
- 跨洲建議自架 DERP server 降延遲

---

## 從 mesh 移除一台

該台機器自己跑：
```powershell
phantom cluster leave
Stop-Process -Name phantom -Force         # 停 serve
schtasks /Delete /F /TN PhantomMeshServe   # 移除 schtask（如果有裝）
```

從 broker 移除：
- 開 https://phantommesh.io/account → Cluster peers → 點該 peer 列右邊的 `×` → Save

其他機器：
```powershell
phantom cluster sync   # peers.json 更新，[cluster] block 拿掉那台
```

---

## 故障排除

| 症狀 | 原因 | 解法 |
|---|---|---|
| 新機器 install 完 cluster status 看不到自己 | 自己不會 ping 自己（Windows Tailscale loopback 限制） | 正常。從**別台**機器看你 |
| 某 peer 一直 offline (> 1hr) | 該機 phantom serve 掛了，或 Tailscale 斷線 | 該機重 `iwr \| iex` 或 `Start-Process phantom serve` |
| 所有 peer 同時都 offline | broker 的 peer URL 全錯 / dashboard 沒 Save | dashboard 重新填、按 Save |
| 派任務超時 | 對方 phantom serve 在跑但 LLM call hung | 對方跑 `phantom debug \| Set-Clipboard`，看 events.jsonl 最後 |
| RTT > 1s | Tailscale relay 跳了多次 | `tailscale netcheck` 看 PreferredDERP / 路徑 |
| 名字衝突 (`upsert` 覆蓋) | 兩台用同 PHANTOM_NODE_NAME | 改一台的 PHANTOM_NODE_NAME，重 login |

---

## 大 cluster 維運建議

對 5+ 台的 setup 建議：

1. **集中管理腳本**：把 install + cluster join 做成一個你 fork 的 Ansible / shell script，每台跑一次比手動可靠
2. **Heartbeat 監控**：寫個 cronjob 每 5 分鐘從 mac coordinator 跑 `phantom cluster status` 推到 Slack / email 警告
3. **Key rotation**：CLUSTER_SECRET 每 90 天 rotate 一次。dashboard 改 → 所有機器 `phantom cluster sync`
4. **角色分組**：dashboard label 欄填角色（`build-runner` / `gpu-worker` / `desktop`），之後 Phase 2 capability dispatcher 才有 metadata 可路由
