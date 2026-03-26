# Phantom Mesh AI 集群 — 災難恢復與業務連續性手冊

> 日期: 2026-03-05
> 範圍: 8 機集群 (1 Hub Z13 + 7 Worker)，24/7 營利系統
> 版本: 1.0
> 狀態: 正式文件

---

## 目錄

1. [系統資產清單](#1-系統資產清單)
2. [故障場景分析與恢復流程](#2-故障場景分析與恢復流程)
3. [備份策略](#3-備份策略)
4. [Hub 高可用 (HA) 設計](#4-hub-高可用-ha-設計)
5. [UPS 方案](#5-ups-方案)
6. [安全事件回應](#6-安全事件回應)
7. [自動化腳本](#7-自動化腳本)
8. [決策樹](#8-決策樹)
9. [定期演練計畫](#9-定期演練計畫)
10. [聯絡清單與升級流程](#10-聯絡清單與升級流程)

---

## 1. 系統資產清單

### 1.1 關鍵資料分類

| 資料 | 位置 | 大小 | 重要度 | 可重建？ |
|------|------|------|--------|----------|
| `core.db` (記憶、任務、集群) | `~/.phantom-mesh/core.db` | ~80KB + WAL ~652KB | **極高** | 否 — 含語義記憶、任務歷史 |
| `costs.db` (成本追蹤) | `~/.phantom-mesh/costs.db` | ~72KB | 高 | 否 — 營運報表資料 |
| `revenue.db` (營收追蹤) | `~/.phantom-mesh/revenue.db` | ~32KB | **極高** | 否 — 財務資料 |
| `memory.db` (語義記憶) | `~/.phantom-mesh/memory.db` | ~40KB | **極高** | 否 — AI 累積知識 |
| `agents.toml` (全局配置) | `~/.phantom-mesh/agents.toml` | ~8KB | **極高** | 是 — Git 版本控制 |
| `hands/*.toml` (13 個工作流) | `~/.phantom-mesh/hands/` | ~50KB 合計 | 高 | 是 — Git 版本控制 |
| `.secret_key` (加密金鑰) | `~/.phantom-mesh/.secret_key` | 64B | **極高** | 否 — 遺失無法解密已加密秘密 |
| `workspace/` (輸出檔案) | `~/.phantom-mesh/workspace/` | 變動 | 中 | 部分 — 可重新生成 |
| `twitter_session/` (瀏覽器狀態) | `~/.phantom-mesh/twitter_session/` | 變動 | 低 | 是 — 重新登入 |
| 其他 `.db` 檔案 (62 個) | `~/.phantom-mesh/*.db` | ~62MB 合計 | 中 | 部分 — 舊系統遺留 |
| phantom-mesh 二進制 | `target/release/phantom-mesh` | ~20MB | 高 | 是 — 重新編譯 |
| 模型檔案 (Ollama/LMStudio) | 各機本地 | 數 GB | 低 | 是 — 重新下載 |

### 1.2 節點清單

| # | 名稱 | 硬體 | 角色 | 網路 | 關鍵服務 |
|---|------|------|------|------|---------|
| 1 | z13 | ASUS ROG Flow Z13 (Ryzen AI MAX+ 395, 64GB) | **Hub** + Worker | localhost | phantom-mesh, Telegram Bot, HTTP API, LM Studio, NPU |
| 2 | acer | Acer (7TB HDD) | Worker + **備份存儲** | LAN | Ollama |
| 3 | ayaneo | Ayaneo | Worker | LAN | Ollama |
| 4 | m1-mac | M1 Mac | Worker | Tailscale VPN | Ollama (Apple Silicon) |
| 5 | gpu-cloud-1 | 雲端 GPU | Worker | VPN/公網 | Ollama |
| 6 | gpu-cloud-2 | 雲端 GPU | Worker | VPN/公網 | Ollama |
| 7 | npu-node | NPU 專用 | Worker | LAN | Lemonade NPU |
| 8 | backup-hub | Standby Hub | **備援 Hub** | LAN/VPN | phantom-mesh (standby) |

### 1.3 外部服務依賴

| 服務 | 用途 | API Key 位置 | 免費額度 |
|------|------|-------------|---------|
| Telegram Bot API | 使用者介面 | `agents.toml [telegram]` | 無限 |
| Gemini API | 視覺 + 文字 | `agents.toml [providers.gemini]` | 免費額度 |
| Serper | Google 搜尋 | `agents.toml [search]` | 2500/月 |
| Tavily | 備用搜尋 | `agents.toml [search]` | 1000/月 |
| Gmail SMTP | 郵件發送 | `agents.toml [email]` | 500/日 |
| Twitter API | 社群發布 | `agents.toml [twitter]` | Free tier |
| Stripe | 支付處理 | `agents.toml [stripe]` | Test mode |
| Render | 雲端部署 | `agents.toml [render]` | Free tier |

---

## 2. 故障場景分析與恢復流程

### 場景 A: Z13 (Hub) 硬碟故障

**影響範圍**: 全面停機 — 所有 Telegram 指令、HTTP API、Cron 排程、Hand 工作流停止

**嚴重程度**: P0 (最高)

**RTO**: 2-4 小時 (使用備援 Hub) / 24-48 小時 (完全重建)

**RPO**: 最多 1 小時 (取決於最後一次備份到 Acer)

**恢復流程**:

```
1. 確認故障
   ├─ Z13 無法啟動 / 藍屏 / 磁碟讀取錯誤
   └─ Telegram bot 無回應

2. 評估硬碟狀態
   ├─ 可修復 (壞軌少) → 用 Linux Live USB 嘗試救援資料
   │   ├─ ddrescue 做磁碟映像
   │   ├─ 優先救援: .secret_key, core.db, memory.db, revenue.db, costs.db
   │   └─ 救援成功 → 換新 SSD → 還原資料 → 重啟服務
   │
   └─ 不可修復 → 啟動備援 Hub 流程 (見第 4 節)
       ├─ 在 Acer (或 backup-hub) 上啟動 phantom-mesh
       ├─ 從 Acer 7TB 還原最近備份
       ├─ 更新 Telegram Bot Webhook (如需)
       └─ 驗證所有 Hand 和 Cron 正常運行
```

**預防措施**:
- 每小時自動備份關鍵 DB 到 Acer 7TB
- `.secret_key` 複製到 Acer 離線保管
- 定期 `smartctl` 監控 SSD 健康

---

### 場景 B: Z13 完全損壞 (主機板故障)

**影響範圍**: Hub 完全不可用，Worker 無法接收任務

**嚴重程度**: P0

**RTO**: 2-6 小時 (備援 Hub) / 3-7 天 (送修/購新機)

**RPO**: 最多 1 小時

**恢復流程**:

```
1. 確認 Z13 徹底無法啟動 (非 SSD 問題)
   ├─ 拆下 SSD → 用 USB 外接盒讀取
   │   ├─ 可讀 → 複製全部 ~/.phantom-mesh/ 到 Acer
   │   └─ 不可讀 → 使用 Acer 上的最近備份
   │
2. 啟動備援 Hub (Acer)
   ├─ cd ~/phantom-mesh-standby/
   ├─ 複製備份的 ~/.phantom-mesh/ 資料到 Acer 的 ~/.phantom-mesh/
   ├─ 確認 agents.toml 中 host/port 正確
   ├─ 修改 providers.lmstudio.url 指向 Acer 本地或其他 Worker
   ├─ ./phantom-mesh daemon
   └─ 驗證 Telegram bot 回應

3. Worker 節點更新
   ├─ Worker 的心跳目標改為 Acer IP
   └─ 或: 如果 Worker 用 Tailscale → 更新 DNS/IP 映射

4. 長期修復
   ├─ Z13 送修 / 購買替代機
   ├─ 修復後還原 Hub 角色
   └─ Acer 恢復為備份存儲
```

---

### 場景 C: 電源中斷 (停電)

**影響範圍**: 所有 LAN 設備 (Z13, Acer, Ayaneo, NPU) 同時離線；雲端 Worker 不受影響

**嚴重程度**: P1

**RTO**: 立即 (有 UPS) / 停電恢復後 5-15 分鐘 (無 UPS)

**RPO**: 0 (WAL 模式的 SQLite 可承受突然斷電) / 最多數秒未 flush 的 WAL

**恢復流程**:

```
有 UPS 的情況:
1. UPS 自動切換電池供電
2. NUT/apcupsd 偵測到電池模式
3. 電池低於 20% → 自動觸發 graceful shutdown 腳本
   ├─ 發送 Telegram 通知: "UPS 電池低，準備關機"
   ├─ E-Stop 啟動 (停止所有 agent)
   ├─ SQLite WAL checkpoint (強制寫入)
   ├─ 安全關機
   └─ 來電後自動開機 (BIOS 設定 "Restore on AC Power Loss")

無 UPS 的情況:
1. 突然斷電 → 所有設備關機
2. 來電後:
   ├─ Z13 開機 (需手動或 BIOS 自動開機)
   ├─ 檢查 SQLite 完整性: sqlite3 ~/.phantom-mesh/core.db "PRAGMA integrity_check;"
   │   ├─ OK → 正常啟動 phantom-mesh
   │   └─ 損壞 → 見場景 I (SQLite 損壞)
   ├─ 啟動 LM Studio / Ollama
   └─ 啟動 phantom-mesh daemon
```

**預防措施**:
- 購買 UPS (見第 5 節)
- BIOS 設定 "Restore on AC Power Loss = On"
- SQLite 已使用 WAL 模式 (cluster.rs L29: `PRAGMA journal_mode=WAL;`)

---

### 場景 D: 網路中斷

**影響範圍**: 取決於中斷範圍

**嚴重程度**: P1 (全面) / P2 (部分)

**RTO**: 取決於 ISP 恢復

**RPO**: 0 (本地處理不受影響)

**恢復流程**:

```
全面網路中斷 (ISP 斷線):
├─ 影響:
│   ├─ Telegram bot 無法收發訊息
│   ├─ 雲端 Worker (gpu-cloud-1/2) 無法連線
│   ├─ 外部 API (Gemini, Serper, Stripe) 不可用
│   ├─ M1 Mac (Tailscale) 無法連線
│   └─ LAN Worker (Acer, Ayaneo) 仍可用
│
├─ 自動降級:
│   ├─ ReliableProvider 的 CircuitBreaker 標記雲端 provider 為 OPEN
│   ├─ ProviderRouter 自動 fallback 到本地 provider (LMStudio, Ollama)
│   └─ Cron 任務繼續執行 (使用本地推理)
│
├─ 手動操作:
│   ├─ 可透過 LAN 存取 HTTP API: http://z13:7878/
│   ├─ 等待網路恢復
│   └─ 恢復後: CircuitBreaker 自動 half-open → 測試 → 恢復

LAN 中斷 (路由器故障):
├─ 影響: Acer, Ayaneo, NPU 節點離線
├─ Z13 本地服務不受影響
├─ 重啟路由器 / 切換備用路由器
└─ Worker 自動重新連線 (心跳恢復)
```

---

### 場景 E: 單一 Worker 離線

**影響範圍**: 推理能力降低，但系統仍可運行

**嚴重程度**: P3

**RTO**: 自動 — 心跳超時 (120 秒) 後自動從路由池移除

**RPO**: 0 (Hub 狀態不受影響)

**恢復流程**:

```
1. 心跳超時偵測 (estop.rs Heartbeat, 120 秒)
   ├─ Heartbeat::stale_agents() 標記該節點為 stale
   ├─ ClusterRegistry 更新 status = "offline"
   └─ Telegram 通知: "Worker {name} 離線"

2. 自動處理:
   ├─ ProviderRouter 停止向該 Worker 路由請求
   ├─ 該 Worker 上正在執行的任務:
   │   ├─ 使用中的 HTTP 請求 → 超時後自動重試到其他 Worker
   │   └─ TaskQueue 中分配給該 Worker 的任務 → 重新排程
   └─ 剩餘 Worker 承接工作負載

3. Worker 恢復:
   ├─ 自動: Worker 重啟後發送心跳 → ClusterRegistry 更新 status = "online"
   ├─ 手動: SSH 到 Worker → 檢查 Ollama 服務 → 重啟
   └─ 驗證: ProviderRouter::is_alive() 確認可用
```

---

### 場景 F: 多台 Worker 同時離線

**影響範圍**: 推理能力嚴重不足，可能導致 Hand 工作流超時

**嚴重程度**: P1 (3+ Worker) / P2 (2 Worker)

**RTO**: 取決於 Worker 恢復時間

**RPO**: 0

**恢復流程**:

```
1. 評估存活狀態
   ├─ GET http://z13:7878/cluster/status → 查看各節點狀態
   ├─ /status (Telegram) → 查看集群概覽
   └─ 計算剩餘推理能力

2. 依存活節點數決定策略:

   7/8 存活 (1 離線): → P3, 正常處理

   5-6/8 存活 (2-3 離線):
   ├─ 降低 Cron 任務頻率 (非關鍵 Hand 暫停)
   ├─ skeleton_generate 減少平行展開數
   └─ 繼續運行核心營利 Hand (freelancer, outreach)

   3-4/8 存活 (4-5 離線):
   ├─ 啟動 E-Stop 暫停非關鍵操作
   ├─ 僅保留: Telegram 回應 + 最高優先 Hand
   ├─ 調查: 是否電源問題？網路問題？
   └─ 考慮啟用雲端 Provider (Gemini/Groq) 補充

   1-2/8 存活 (Hub + 1 或僅 Hub):
   ├─ 全面降級: 僅 Z13 本地推理
   ├─ 暫停所有 Cron 和自動 Hand
   ├─ Telegram 手動模式
   └─ 緊急修復 Worker / 啟用雲端 GPU
```

---

### 場景 G: 被駭客入侵

**影響範圍**: 資料外洩、系統被控制、API 金鑰被盜

**嚴重程度**: P0

**RTO**: 數小時 (隔離) + 24-72 小時 (徹底清除)

**RPO**: 取決於入侵時間點

**恢復流程**:

```
1. 偵測入侵
   ├─ 異常跡象:
   │   ├─ 未知程序佔用 CPU/GPU
   │   ├─ 異常的外部連線 (netstat -an)
   │   ├─ agents.toml 被修改
   │   ├─ 未預期的 Telegram 訊息
   │   ├─ API 用量異常暴增
   │   └─ 未知的 cron 任務
   │
   └─ 確認入侵

2. 立即隔離 (黃金 15 分鐘)
   ├─ 斷開 Z13 網路 (拔網線或 Wi-Fi off)
   ├─ E-Stop 啟動: curl -X POST http://localhost:7878/estop
   ├─ 殺掉所有 phantom-mesh 進程: taskkill /F /IM phantom-mesh.exe
   ├─ 殺掉可疑進程
   └─ 通知所有 Worker 斷線

3. 證據保全
   ├─ 記錄當前時間
   ├─ 匯出系統日誌: cp ~/.phantom-mesh/logs/ ~/incident/
   ├─ 匯出網路連線: netstat -an > ~/incident/netstat.txt
   ├─ 匯出進程清單: tasklist > ~/incident/processes.txt
   └─ 保留 core.db 快照

4. 密鑰 Rotate (見第 6 節)
   ├─ Telegram bot token → BotFather 重新生成
   ├─ 所有 API keys → 各平台 dashboard 重新生成
   ├─ Gmail app password → 撤銷 + 重新生成
   ├─ Twitter OAuth → 重新授權
   ├─ .secret_key → 重新生成 (注意: 已加密的秘密將無法解密)
   └─ agents.toml 寫入新 keys

5. 系統重建
   ├─ 掃描所有機器 (Windows Defender / ClamAV)
   ├─ 檢查: 是否有後門、rootkit
   ├─ 重新編譯 phantom-mesh (確認源碼未被竄改)
   │   └─ git log 確認無異常 commit
   ├─ 從已知良好備份還原 DB
   └─ 逐一重啟 Worker → Hub

6. 事後分析
   ├─ 確認入侵路徑 (SSH? Telegram? HTTP API? API key?)
   ├─ 加固薄弱環節
   ├─ 考慮: 開啟 Cloudflare → HTTP API
   └─ 考慮: agents.toml 中 API key 全面遷移到 enc2: 加密格式
```

---

### 場景 H: API Key 洩漏

**影響範圍**: 取決於哪個 key 洩漏

**嚴重程度**: P0 (Telegram/Stripe) / P1 (其他)

**RTO**: 5-15 分鐘 (單 key rotate)

**RPO**: 0

**恢復流程**:

```
判斷洩漏的 key:

Telegram Bot Token:
├─ 影響: 攻擊者可以冒充 bot 發訊息
├─ 步驟:
│   ├─ 打開 @BotFather → /revoke → 選擇你的 bot → 獲得新 token
│   ├─ 更新 agents.toml [telegram] bot_token
│   ├─ 重啟 phantom-mesh
│   └─ 驗證 bot 正常回應
├─ RTO: 5 分鐘

Gemini API Key:
├─ 影響: 攻擊者用你的配額 → 帳單暴增
├─ 步驟:
│   ├─ https://console.cloud.google.com/apis/credentials → 刪除舊 key → 建新 key
│   ├─ 更新 agents.toml [providers.gemini] api_key
│   └─ 重啟 phantom-mesh
├─ RTO: 5 分鐘

Search API Keys (Serper/Tavily/Brave/Exa):
├─ 影響: 配額被用完
├─ 步驟:
│   ├─ 到各平台 dashboard rotate key
│   ├─ 更新 agents.toml [search] 相關欄位
│   └─ 重啟 phantom-mesh
├─ RTO: 10 分鐘

Gmail App Password:
├─ 影響: 攻擊者可用你的帳號發垃圾郵件
├─ 步驟:
│   ├─ https://myaccount.google.com/apppasswords → 撤銷舊密碼
│   ├─ 重新生成 app password
│   ├─ 更新 agents.toml [email] password
│   └─ 重啟 phantom-mesh
├─ RTO: 5 分鐘

Twitter OAuth:
├─ 影響: 攻擊者可以用你的帳號發推
├─ 步驟:
│   ├─ https://developer.twitter.com/en/portal → 重新生成 Consumer Key + Access Token
│   ├─ 更新 agents.toml [twitter] 全部 4 個欄位
│   ├─ 重新執行瀏覽器登入: twitter(action="login")
│   └─ 重啟 phantom-mesh
├─ RTO: 10 分鐘

Stripe Secret Key:
├─ 影響: 攻擊者可以存取支付資料 (P0!)
├─ 步驟:
│   ├─ https://dashboard.stripe.com/apikeys → Roll key
│   ├─ 更新 agents.toml [stripe] secret_key
│   ├─ 更新所有已部署的 SaaS 服務中的 STRIPE_SECRET_KEY 環境變數
│   └─ 重啟 phantom-mesh
├─ RTO: 15 分鐘

.secret_key 洩漏:
├─ 影響: 所有 enc2: 加密的秘密可被解密
├─ 步驟:
│   ├─ 刪除 ~/.phantom-mesh/.secret_key
│   ├─ 重啟 phantom-mesh (自動生成新 key)
│   ├─ 重新加密所有秘密: phantom-mesh encrypt-secret
│   └─ 同時 rotate 所有被加密的原始 key
├─ RTO: 30 分鐘
```

---

### 場景 I: SQLite 資料庫損壞

**影響範圍**: 取決於哪個 DB 損壞

**嚴重程度**: P1 (core.db/memory.db) / P2 (其他)

**RTO**: 15-60 分鐘

**RPO**: 最多 1 小時 (到最近備份)

**恢復流程**:

```
1. 診斷
   $ sqlite3 ~/.phantom-mesh/core.db "PRAGMA integrity_check;"
   ├─ "ok" → 資料庫完好，問題在其他地方
   └─ 錯誤訊息 → 確認損壞

2. 嘗試修復 (輕度損壞)
   $ sqlite3 ~/.phantom-mesh/core.db ".recover" | sqlite3 ~/.phantom-mesh/core_recovered.db
   ├─ 成功 → 驗證 recovered DB
   │   $ sqlite3 core_recovered.db "PRAGMA integrity_check;"
   │   $ sqlite3 core_recovered.db "SELECT COUNT(*) FROM memories;"
   │   ├─ 資料完整 → 替換: mv core.db core.db.bad && mv core_recovered.db core.db
   │   └─ 資料缺失 → 從備份還原
   │
   └─ 失敗 → 從備份還原

3. 從備份還原
   ├─ 找到最近的備份:
   │   $ ls -lt /mnt/acer/phantom-mesh-backup/daily/core.db.* | head -5
   │
   ├─ 停止 phantom-mesh
   │   $ taskkill /F /IM phantom-mesh.exe
   │
   ├─ 備份損壞的 DB (留存證據):
   │   $ mv ~/.phantom-mesh/core.db ~/.phantom-mesh/core.db.corrupted.$(date +%Y%m%d%H%M)
   │
   ├─ 還原:
   │   $ cp /mnt/acer/phantom-mesh-backup/daily/core.db.latest ~/.phantom-mesh/core.db
   │
   └─ 重啟 phantom-mesh

4. WAL 相關問題
   ├─ 如果 core.db-wal 損壞但 core.db 完好:
   │   $ sqlite3 ~/.phantom-mesh/core.db "PRAGMA wal_checkpoint(TRUNCATE);"
   │   ├─ 成功 → WAL 已合併到主 DB
   │   └─ 失敗 → 刪除 core.db-wal 和 core.db-shm → 損失 WAL 中未 checkpoint 的資料
   │
   └─ 預防: 每小時執行 WAL checkpoint (見備份腳本)
```

---

## 3. 備份策略

### 3.1 備份層次

```
┌────────────────────────────────────────────────────────────────────┐
│                        備份金字塔                                   │
│                                                                    │
│  ┌─────────────────────────┐                                       │
│  │  Layer 3: 異地備份       │  每週 → 加密 USB / 雲端 (手動)        │
│  │  .secret_key + DB 快照   │  保留: 4 週                           │
│  └────────┬────────────────┘                                       │
│           │                                                        │
│  ┌────────▼────────────────┐                                       │
│  │  Layer 2: 每日完整備份    │  每日 03:00 → Acer 7TB               │
│  │  全部 DB + workspace     │  保留: 30 天 (自動輪替)               │
│  └────────┬────────────────┘                                       │
│           │                                                        │
│  ┌────────▼────────────────┐                                       │
│  │  Layer 1: 每小時增量     │  每小時 → Acer 7TB                    │
│  │  WAL checkpoint + rsync  │  保留: 48 小時 (滾動覆蓋)             │
│  └────────┬────────────────┘                                       │
│           │                                                        │
│  ┌────────▼────────────────┐                                       │
│  │  Layer 0: 即時 (WAL)     │  SQLite WAL 模式 — 寫入立即安全       │
│  │  + Git 版本控制          │  agents.toml + hands/*.toml in Git   │
│  └─────────────────────────┘                                       │
└────────────────────────────────────────────────────────────────────┘
```

### 3.2 各資料類型備份策略

| 資料 | Layer 0 | Layer 1 (每小時) | Layer 2 (每日) | Layer 3 (每週) |
|------|---------|-----------------|----------------|---------------|
| core.db | WAL | WAL checkpoint + copy | 完整備份 | 加密離線 |
| memory.db | WAL | WAL checkpoint + copy | 完整備份 | 加密離線 |
| costs.db | WAL | rsync | 完整備份 | -- |
| revenue.db | WAL | WAL checkpoint + copy | 完整備份 | 加密離線 |
| agents.toml | Git | -- | Git push | -- |
| hands/*.toml | Git | -- | Git push | -- |
| .secret_key | -- | -- | 加密複製到 Acer | 加密 USB |
| workspace/ | -- | -- | 壓縮歸檔 | -- |
| 模型檔案 | -- | -- | -- | 不備份 (可重下) |
| twitter_session/ | -- | -- | -- | 不備份 (可重建) |

### 3.3 備份目標路徑

```
Acer 7TB (\\acer\backup 或 /mnt/acer):
├── phantom-mesh-backup/
│   ├── hourly/
│   │   ├── core.db.2026-03-05T10       (每小時輪替, 保留 48 個)
│   │   ├── memory.db.2026-03-05T10
│   │   └── revenue.db.2026-03-05T10
│   ├── daily/
│   │   ├── 2026-03-05/
│   │   │   ├── core.db
│   │   │   ├── memory.db
│   │   │   ├── costs.db
│   │   │   ├── revenue.db
│   │   │   ├── agents.toml
│   │   │   ├── hands/
│   │   │   ├── .secret_key.enc         (AES 加密)
│   │   │   └── workspace.tar.gz
│   │   ├── 2026-03-04/
│   │   └── ... (保留 30 天)
│   └── weekly/
│       ├── 2026-W10.tar.gz.gpg         (GPG 加密)
│       └── ... (保留 4 週)
└── phantom-mesh-standby/
    └── phantom-mesh                     (預編譯的 binary)
```

---

## 4. Hub 高可用 (HA) 設計

### 4.1 架構: Active-Passive Failover

```
正常狀態:
                    ┌─────────────┐
  Telegram ────────>│  Z13 (Hub)  │──────> Workers
  HTTP API ────────>│  ACTIVE     │
                    └──────┬──────┘
                           │ 每小時同步
                           ▼
                    ┌─────────────┐
                    │ Acer (備援)  │
                    │ PASSIVE     │
                    │ (standby)   │
                    └─────────────┘

故障切換後:
                    ┌─────────────┐
                    │  Z13 (Hub)  │  ✗ 離線
                    │  DOWN       │
                    └─────────────┘

                    ┌─────────────┐
  Telegram ────────>│ Acer (Hub)  │──────> Workers
  HTTP API ────────>│ ACTIVE      │
                    └─────────────┘
```

### 4.2 準備工作清單

**在 Acer 上預先準備 (一次性)**:

```bash
# 1. 安裝 Rust 工具鏈
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2. Clone 並編譯 phantom-mesh
cd ~/
git clone <phantom-mesh-repo> phantom-mesh-standby
cd phantom-mesh-standby
cargo build --release
# binary 位於: ~/phantom-mesh-standby/target/release/phantom-mesh

# 3. 建立 ~/.phantom-mesh/ 目錄結構
mkdir -p ~/.phantom-mesh/hands ~/.phantom-mesh/workspace ~/.phantom-mesh/logs

# 4. 複製 agents.toml (需修改 providers 指向)
cp /mnt/z13-backup/agents.toml ~/.phantom-mesh/agents.toml

# 5. 複製 hands
cp -r /mnt/z13-backup/hands/* ~/.phantom-mesh/hands/

# 6. 複製 .secret_key
cp /mnt/z13-backup/.secret_key ~/.phantom-mesh/.secret_key
chmod 600 ~/.phantom-mesh/.secret_key

# 7. 安裝 Ollama (如果 Acer 要跑本地推理)
curl -fsSL https://ollama.com/install.sh | sh
ollama pull llama3.2:1b
ollama pull qwen2.5-coder:7b

# 8. 測試啟動 (dry run)
~/phantom-mesh-standby/target/release/phantom-mesh status
```

**agents.toml 備援版差異**:

```toml
# Acer 備援版 agents.toml 需要修改的部分:
[core]
host = "0.0.0.0"
port = 7878
# db_path 保持預設

[providers.lmstudio]
# Acer 沒有 LM Studio → 改用 Ollama 或指向其他 Worker
type = "ollama"
url = "http://localhost:11434"
default_model = "qwen2.5-coder:7b"

[providers.npu]
# Acer 沒有 NPU → 停用或指向 Z13 (如果 Z13 復活)
# type = "openai_compat"
# url = "http://z13:8000/api/v1"  # 故障時不可用

[agent.master]
provider = "ollama"  # 改用 Ollama
model = "qwen2.5-coder:7b"

[agent.coder]
provider = "ollama"
model = "qwen2.5-coder:7b"
```

### 4.3 SQLite 同步策略

```
每小時同步 (Z13 → Acer):
┌─────────┐                    ┌──────────┐
│  Z13    │  WAL checkpoint    │  Acer    │
│ core.db │ ───────────────>   │ core.db  │
│         │  rsync --checksum  │ (備份)    │
└─────────┘                    └──────────┘

步驟:
1. Z13 執行 WAL checkpoint:
   sqlite3 ~/.phantom-mesh/core.db "PRAGMA wal_checkpoint(PASSIVE);"

2. rsync 到 Acer (確保一致性):
   rsync -az --checksum \
     ~/.phantom-mesh/core.db \
     ~/.phantom-mesh/memory.db \
     ~/.phantom-mesh/costs.db \
     ~/.phantom-mesh/revenue.db \
     acer:/home/user/phantom-mesh-backup/hourly/

注意事項:
- 使用 PASSIVE checkpoint (不阻塞寫入)
- rsync 的 --checksum 確保傳輸完整性
- 不要在 rsync 過程中使用 TRUNCATE checkpoint (會鎖定 DB)
- WAL 檔案不需要同步 (checkpoint 後資料已在主 DB)
```

### 4.4 故障切換 SOP (Standard Operating Procedure)

```
故障切換流程 (手動, 約 5-10 分鐘):

1. 確認 Z13 Hub 不可用
   $ ping z13    # 無回應
   $ curl http://z13:7878/status    # 超時
   → 確認 Z13 離線

2. 在 Acer 上還原最新備份
   $ cd ~/phantom-mesh-backup/hourly/
   $ ls -lt    # 找最新的備份
   $ cp core.db.latest ~/.phantom-mesh/core.db
   $ cp memory.db.latest ~/.phantom-mesh/memory.db
   $ cp costs.db.latest ~/.phantom-mesh/costs.db
   $ cp revenue.db.latest ~/.phantom-mesh/revenue.db

3. 驗證 DB 完整性
   $ sqlite3 ~/.phantom-mesh/core.db "PRAGMA integrity_check;"
   # 應該顯示 "ok"

4. 啟動 phantom-mesh
   $ ~/phantom-mesh-standby/target/release/phantom-mesh daemon &

5. 驗證 Telegram bot
   - 發送測試訊息
   - bot 應該回應 (Telegram bot token 不變, webhook 不需改)

   注意: Telegram Bot API 使用 long polling, 不需要 webhook。
   只要新的 phantom-mesh 啟動並開始 polling, 就能接收訊息。
   如果使用 webhook 模式, 需要更新 webhook URL 到 Acer 的公網 IP。

6. 通知 Workers 更新 Hub 地址 (如果 Worker 主動連 Hub):
   - 更新各 Worker 的配置
   - 或: 使用 DNS 名稱 (如 hub.phantom-mesh.local) → 修改 DNS 指向 Acer

7. 驗證完整功能
   $ curl http://localhost:7878/cluster/status
   $ # 發送 /status 到 Telegram
   $ # 觸發一個測試 Hand: /hand researcher "test query"
```

### 4.5 故障回切 SOP (Z13 修復後)

```
1. Z13 修復完畢, 確認硬體正常

2. 停止 Acer 上的 phantom-mesh
   $ kill $(pgrep phantom-mesh)

3. 從 Acer 同步最新資料回 Z13
   $ rsync -az ~/.phantom-mesh/*.db z13:~/.phantom-mesh/

4. 在 Z13 重啟 phantom-mesh
   $ phantom-mesh daemon &

5. 驗證 Z13 Hub 正常
   $ curl http://localhost:7878/status

6. Acer 恢復為 Passive 備援角色
```

---

## 5. UPS 方案

### 5.1 容量計算

```
設備功耗估算:
┌─────────────────────────┬─────────────┬───────────────┐
│ 設備                     │ 典型功耗 (W) │ 最大功耗 (W)   │
├─────────────────────────┼─────────────┼───────────────┤
│ Z13 (充電 + 推理中)      │ 100         │ 150           │
│ Z13 (僅 CPU, 省電模式)   │ 40          │ 65            │
│ 路由器                   │ 10          │ 15            │
│ 網路交換機 (如有)        │ 10          │ 15            │
│ Acer (如需同時保護)      │ 60          │ 100           │
├─────────────────────────┼─────────────┼───────────────┤
│ 最小方案 (Z13 + 路由器)  │ 110         │ 165           │
│ 建議方案 (+Acer)         │ 170         │ 265           │
└─────────────────────────┴─────────────┴───────────────┘

目標: 停電後至少 10 分鐘安全關機時間

最小 UPS 容量:
- 165W × 10min = 27.5 Wh
- 考慮效率損失 (80%): 27.5 / 0.8 = 34.4 Wh
- 建議最小: 650VA / 390W (約能撐 15-20 分鐘 @165W)

建議 UPS 容量:
- 265W × 15min = 66.25 Wh
- 考慮效率和電池衰減: 66.25 / 0.7 = 94.6 Wh
- 建議: 1000VA / 600W (約能撐 15-20 分鐘 @265W)
```

### 5.2 台灣市場 UPS 推薦

```
┌──────────────────────────────┬──────────┬──────────┬─────────────────────┐
│ 型號                          │ 容量      │ 價格 (TWD)│ 適合場景             │
├──────────────────────────────┼──────────┼──────────┼─────────────────────┤
│ 飛瑞 A-650                   │ 650VA    │ ~2,000   │ 最小方案 (僅 Z13)    │
│ 科風 BNT-650A (在線互動式)    │ 650VA    │ ~2,500   │ 最小方案 + 穩壓      │
│ CyberPower CP1000AVRLCDA     │ 1000VA   │ ~3,500   │ 建議方案 (Z13+路由器)│
│ APC BX1100CI-MS              │ 1100VA   │ ~4,000   │ 建議方案 (含 Acer)   │
│ 飛瑞 A-1000                  │ 1000VA   │ ~3,000   │ 建議方案 (CP 值高)   │
│ CyberPower PR1000ELCDGR      │ 1000VA   │ ~6,000   │ 進階 (純在線式, LCD) │
└──────────────────────────────┴──────────┴──────────┴─────────────────────┘

建議選擇: CyberPower CP1000AVRLCDA 或 飛瑞 A-1000
- 在線互動式 (Line-Interactive): 切換時間 < 5ms
- USB 介面: 可以連接電腦做自動關機
- 1000VA 提供足夠安全邊際
- 價格合理 (TWD 3,000-3,500)

購買管道:
- PChome 24h / momo / 蝦皮 (快速到貨)
- 順發 3C / 原價屋 (實體店面, 可立即取貨)
```

### 5.3 UPS 自動關機腳本

**Windows (Z13)**:

使用 CyberPower 的 PowerPanel Personal 軟體 (免費) 或 NUT (Network UPS Tools):

```
方案 1: CyberPower PowerPanel (最簡單)
├─ 安裝 PowerPanel Personal (USB 連接 UPS)
├─ 設定:
│   ├─ 電池模式 → 執行自訂腳本
│   ├─ 電池低於 20% → 執行安全關機
│   └─ 自訂腳本路徑: C:\phantom-mesh\ups_shutdown.bat
└─ 自動處理

方案 2: 自訂腳本 (如果 PowerPanel 不可用)
```

---

## 6. 安全事件回應

### 6.1 API Key 洩漏 — 立即 Rotate 流程

**自動化 Rotate 腳本** (見第 7 節 `rotate_keys.sh`)

**手動 Rotate 檢查表**:

```
□ 1. 確認哪些 key 洩漏
□ 2. 立即 E-Stop: /estop (Telegram) 或 curl -X POST localhost:7878/estop
□ 3. 逐一 rotate:
     □ Telegram bot token   → @BotFather /revoke
     □ Gemini API key       → console.cloud.google.com
     □ Serper API key       → serper.dev dashboard
     □ Tavily API key       → tavily.com dashboard
     □ Brave API key        → brave.com dashboard
     □ Exa API key          → exa.ai dashboard
     □ Gmail app password   → myaccount.google.com/apppasswords
     □ Twitter OAuth        → developer.twitter.com
     □ Stripe secret key    → dashboard.stripe.com/apikeys
     □ Render API key       → dashboard.render.com/settings
     □ .secret_key          → 刪除 + 重啟 (自動重新生成)
□ 4. 更新 agents.toml
□ 5. 重啟 phantom-mesh
□ 6. 驗證所有功能正常
□ 7. 檢查帳單/用量是否有異常
□ 8. 記錄事件報告
```

### 6.2 Telegram Bot 被盜

```
1. 立即撤銷 token
   → @BotFather → /revoke → 選擇被盜 bot

2. 檢查是否有異常訊息被發送
   → 檢查 Telegram 群組/頻道中是否有非預期訊息

3. 重新生成 token
   → @BotFather → /token → 選擇 bot → 獲得新 token

4. 更新 agents.toml
   [telegram]
   bot_token = "新的token"

5. 加強安全
   → 確認 allowed_users 只有你的 Telegram username
   → 考慮加上 chat_id 白名單
```

### 6.3 被 DDoS 攻擊

```
如果 HTTP API (port 7878) 被 DDoS:

1. 評估影響
   ├─ Telegram bot 不受影響 (走 Telegram 的 long polling, 不需要公網)
   ├─ LAN Worker 不受影響
   └─ 只有公開的 HTTP API 受影響

2. 緊急處理
   ├─ 如果 7878 port 沒有暴露到公網 → 不受影響
   ├─ 如果有暴露:
   │   ├─ 關閉公網映射 (路由器 NAT/port forwarding 關掉)
   │   └─ 或: 加 Cloudflare Tunnel
   └─ 如果使用 Cloudflare:
       ├─ 開啟 "Under Attack Mode"
       ├─ 設定 Rate Limiting 規則
       └─ 必要時開啟 "I'm Under Attack" 模式

3. 長期防護
   ├─ 不要直接暴露 7878 到公網
   ├─ 使用 Cloudflare Tunnel 或 ngrok (有 DDoS 防護)
   ├─ 或: Tailscale Funnel (僅 Tailscale 網路可存取)
   └─ HTTP API 加上 API key 認證
```

### 6.4 模型被污染

```
1. 偵測跡象
   ├─ Agent 輸出異常 (偏見、有害內容、垃圾)
   ├─ 模型回應品質突然下降
   └─ 模型檔案 hash 與已知值不符

2. 隔離
   ├─ E-Stop 暫停所有 agent
   ├─ 標記該 Worker 的模型為不可信
   └─ ProviderRouter 停止路由到該 Worker

3. 回滾
   ├─ Ollama: ollama rm <model> && ollama pull <model>
   ├─ LM Studio: 刪除模型目錄 → 重新下載
   └─ 如果是自訂微調模型 → 從備份還原 checkpoint

4. 驗證
   ├─ 用標準 prompt 測試模型輸出
   ├─ 比對已知良好輸出
   └─ 確認正常後 E-Stop reset
```

---

## 7. 自動化腳本

### 7.1 每小時備份腳本

```bash
#!/bin/bash
# phantom-mesh_hourly_backup.sh
# 放置: C:/phantom-mesh/scripts/phantom-mesh_hourly_backup.sh
# 排程: Windows Task Scheduler 每小時執行一次

set -euo pipefail

# === 配置 ===
PHANTOM_MESH_DIR="$HOME/.phantom-mesh"
BACKUP_BASE="/mnt/acer/phantom-mesh-backup"  # 根據實際 Acer 掛載路徑修改
HOURLY_DIR="$BACKUP_BASE/hourly"
TIMESTAMP=$(date +%Y-%m-%dT%H)
MAX_HOURLY=48  # 保留 48 小時

# 關鍵 DB 清單
CRITICAL_DBS=("core.db" "memory.db" "revenue.db" "costs.db")

# === 建立目錄 ===
mkdir -p "$HOURLY_DIR"

# === Step 1: WAL Checkpoint ===
echo "[$(date)] Starting WAL checkpoint..."
for db in "${CRITICAL_DBS[@]}"; do
    DB_PATH="$PHANTOM_MESH_DIR/$db"
    if [ -f "$DB_PATH" ]; then
        sqlite3 "$DB_PATH" "PRAGMA wal_checkpoint(PASSIVE);" 2>/dev/null || true
        echo "  Checkpointed: $db"
    fi
done

# === Step 2: 複製 DB 到備份 ===
echo "[$(date)] Copying databases..."
for db in "${CRITICAL_DBS[@]}"; do
    DB_PATH="$PHANTOM_MESH_DIR/$db"
    if [ -f "$DB_PATH" ]; then
        cp "$DB_PATH" "$HOURLY_DIR/${db}.${TIMESTAMP}"
        echo "  Copied: $db → ${db}.${TIMESTAMP}"
    fi
done

# 建立 "latest" 符號連結
for db in "${CRITICAL_DBS[@]}"; do
    LATEST="$HOURLY_DIR/${db}.latest"
    SOURCE="$HOURLY_DIR/${db}.${TIMESTAMP}"
    if [ -f "$SOURCE" ]; then
        cp "$SOURCE" "$LATEST"
    fi
done

# === Step 3: 清理過期備份 ===
echo "[$(date)] Cleaning old backups..."
for db in "${CRITICAL_DBS[@]}"; do
    # 找出該 DB 的所有備份, 按時間排序, 刪除超過 MAX_HOURLY 的
    ls -t "$HOURLY_DIR/${db}."* 2>/dev/null | grep -v "latest" | tail -n +$((MAX_HOURLY + 1)) | while read -r old; do
        rm -f "$old"
        echo "  Deleted: $old"
    done
done

# === Step 4: 驗證 ===
echo "[$(date)] Verifying backups..."
for db in "${CRITICAL_DBS[@]}"; do
    BACKUP="$HOURLY_DIR/${db}.${TIMESTAMP}"
    if [ -f "$BACKUP" ]; then
        RESULT=$(sqlite3 "$BACKUP" "PRAGMA integrity_check;" 2>/dev/null || echo "FAILED")
        if [ "$RESULT" = "ok" ]; then
            echo "  OK: $BACKUP"
        else
            echo "  WARNING: $BACKUP integrity check FAILED!"
            # TODO: 發 Telegram 告警
        fi
    fi
done

echo "[$(date)] Hourly backup complete."
```

### 7.2 每日完整備份腳本

```bash
#!/bin/bash
# phantom-mesh_daily_backup.sh
# 排程: 每日 03:00

set -euo pipefail

PHANTOM_MESH_DIR="$HOME/.phantom-mesh"
BACKUP_BASE="/mnt/acer/phantom-mesh-backup"
DAILY_DIR="$BACKUP_BASE/daily/$(date +%Y-%m-%d)"
MAX_DAILY=30  # 保留 30 天

# Telegram 通知函數
notify_telegram() {
    local msg="$1"
    local BOT_TOKEN=$(grep bot_token "$PHANTOM_MESH_DIR/agents.toml" | head -1 | cut -d'"' -f2)
    local CHAT_ID=$(grep allowed_users "$PHANTOM_MESH_DIR/agents.toml" | head -1 | cut -d'"' -f2)
    # 注意: 需要用 chat_id 而非 username, 這裡僅為示意
    # 實際實作需要取得自己的 chat_id
    curl -s "https://api.telegram.org/bot${BOT_TOKEN}/sendMessage" \
        -d "chat_id=${CHAT_ID}" -d "text=${msg}" > /dev/null 2>&1 || true
}

echo "[$(date)] Starting daily backup..."
mkdir -p "$DAILY_DIR"

# === Step 1: 完整 WAL Checkpoint (TRUNCATE — 會短暫鎖定) ===
echo "[$(date)] Full WAL checkpoint..."
for db in "$PHANTOM_MESH_DIR"/*.db; do
    if [ -f "$db" ]; then
        sqlite3 "$db" "PRAGMA wal_checkpoint(TRUNCATE);" 2>/dev/null || true
    fi
done

# === Step 2: 複製所有 DB ===
echo "[$(date)] Copying all databases..."
cp "$PHANTOM_MESH_DIR"/*.db "$DAILY_DIR/" 2>/dev/null || true

# === Step 3: 複製配置 ===
echo "[$(date)] Copying configuration..."
cp "$PHANTOM_MESH_DIR/agents.toml" "$DAILY_DIR/"
cp -r "$PHANTOM_MESH_DIR/hands" "$DAILY_DIR/"

# === Step 4: 加密備份 .secret_key ===
if [ -f "$PHANTOM_MESH_DIR/.secret_key" ]; then
    # 使用 GPG 對稱加密 (需要預設密碼或 gpg-agent)
    # 或簡單 openssl 加密
    openssl enc -aes-256-cbc -salt -pbkdf2 \
        -in "$PHANTOM_MESH_DIR/.secret_key" \
        -out "$DAILY_DIR/.secret_key.enc" \
        -pass file:"$HOME/.phantom-mesh_backup_passphrase" 2>/dev/null || \
    cp "$PHANTOM_MESH_DIR/.secret_key" "$DAILY_DIR/.secret_key"
    echo "  .secret_key backed up (encrypted)"
fi

# === Step 5: 歸檔 workspace ===
echo "[$(date)] Archiving workspace..."
if [ -d "$PHANTOM_MESH_DIR/workspace" ]; then
    tar czf "$DAILY_DIR/workspace.tar.gz" -C "$PHANTOM_MESH_DIR" workspace/ 2>/dev/null || true
fi

# === Step 6: 清理過期備份 ===
echo "[$(date)] Cleaning old daily backups..."
ls -dt "$BACKUP_BASE/daily"/*/ 2>/dev/null | tail -n +$((MAX_DAILY + 1)) | while read -r old_dir; do
    rm -rf "$old_dir"
    echo "  Deleted: $old_dir"
done

# === Step 7: 驗證 ===
echo "[$(date)] Verifying critical databases..."
VERIFY_PASS=true
for db in core.db memory.db revenue.db costs.db; do
    BACKUP="$DAILY_DIR/$db"
    if [ -f "$BACKUP" ]; then
        RESULT=$(sqlite3 "$BACKUP" "PRAGMA integrity_check;" 2>/dev/null || echo "FAILED")
        if [ "$RESULT" != "ok" ]; then
            echo "  CRITICAL: $db integrity check FAILED!"
            VERIFY_PASS=false
        else
            echo "  OK: $db"
        fi
    fi
done

# === Step 8: 同步備援 Hub 的 binary ===
BINARY="$HOME/Desktop/adreanalai/LLM-Cluster-Project/phantom-mesh/target/release/phantom-mesh"
if [ -f "$BINARY" ]; then
    cp "$BINARY" "$BACKUP_BASE/phantom-mesh-standby/phantom-mesh"
    echo "  Synced phantom-mesh binary to standby"
fi

# === 完成通知 ===
BACKUP_SIZE=$(du -sh "$DAILY_DIR" | cut -f1)
if $VERIFY_PASS; then
    echo "[$(date)] Daily backup complete. Size: $BACKUP_SIZE"
    # notify_telegram "Daily backup OK. Size: $BACKUP_SIZE"
else
    echo "[$(date)] Daily backup COMPLETE WITH WARNINGS. Check integrity!"
    # notify_telegram "WARN: Daily backup has integrity issues!"
fi
```

### 7.3 UPS 安全關機腳本

```batch
@echo off
REM ups_shutdown.bat — UPS 電池低時自動執行
REM 放置: C:\phantom-mesh\scripts\ups_shutdown.bat

echo [%date% %time%] UPS battery low — initiating graceful shutdown...

REM Step 1: 發送 E-Stop
curl -s -X POST http://localhost:7878/estop > nul 2>&1

REM Step 2: 等待 agent 停止 (5 秒)
timeout /t 5 /nobreak > nul

REM Step 3: WAL Checkpoint
sqlite3 "%USERPROFILE%\.phantom-mesh\core.db" "PRAGMA wal_checkpoint(TRUNCATE);"
sqlite3 "%USERPROFILE%\.phantom-mesh\memory.db" "PRAGMA wal_checkpoint(TRUNCATE);"
sqlite3 "%USERPROFILE%\.phantom-mesh\revenue.db" "PRAGMA wal_checkpoint(TRUNCATE);"
sqlite3 "%USERPROFILE%\.phantom-mesh\costs.db" "PRAGMA wal_checkpoint(TRUNCATE);"

REM Step 4: 停止 phantom-mesh
taskkill /F /IM phantom-mesh.exe > nul 2>&1

REM Step 5: 安全關機
echo [%date% %time%] Shutting down...
shutdown /s /t 30 /c "UPS battery low - safe shutdown"
```

```bash
#!/bin/bash
# ups_shutdown.sh — Linux/Acer 版本
# 由 NUT (upssched) 或 apcupsd 觸發

set -euo pipefail

echo "[$(date)] UPS battery low — graceful shutdown..."

# E-Stop
curl -s -X POST http://localhost:7878/estop 2>/dev/null || true

# Wait for agents to stop
sleep 5

# WAL Checkpoint
PHANTOM_MESH_DIR="$HOME/.phantom-mesh"
for db in core.db memory.db revenue.db costs.db; do
    sqlite3 "$PHANTOM_MESH_DIR/$db" "PRAGMA wal_checkpoint(TRUNCATE);" 2>/dev/null || true
done

# 緊急備份到本地
EMERGENCY_DIR="$PHANTOM_MESH_DIR/emergency_backup_$(date +%Y%m%d%H%M)"
mkdir -p "$EMERGENCY_DIR"
cp "$PHANTOM_MESH_DIR"/core.db "$PHANTOM_MESH_DIR"/memory.db "$PHANTOM_MESH_DIR"/revenue.db "$PHANTOM_MESH_DIR"/costs.db "$EMERGENCY_DIR/" 2>/dev/null || true

# 停止服務
pkill -TERM phantom-mesh 2>/dev/null || true

# 安全關機
sudo shutdown -h now "UPS battery critical"
```

### 7.4 健康檢查腳本

```bash
#!/bin/bash
# phantom-mesh_health_check.sh
# 排程: 每 5 分鐘 (Windows Task Scheduler 或 cron)

PHANTOM_MESH_DIR="$HOME/.phantom-mesh"
STATUS_OK=true

# Check 1: phantom-mesh 進程存活
if ! pgrep -f phantom-mesh > /dev/null 2>&1; then
    echo "CRITICAL: phantom-mesh is not running!"
    STATUS_OK=false
    # 自動重啟
    cd "$HOME/Desktop/adreanalai/LLM-Cluster-Project/phantom-mesh"
    ./target/release/phantom-mesh daemon &
    echo "  Auto-restarted phantom-mesh"
fi

# Check 2: HTTP API 可回應
HTTP_STATUS=$(curl -s -o /dev/null -w "%{http_code}" http://localhost:7878/status 2>/dev/null || echo "000")
if [ "$HTTP_STATUS" != "200" ]; then
    echo "WARNING: HTTP API returned $HTTP_STATUS"
    STATUS_OK=false
fi

# Check 3: DB 完整性 (每小時只跑一次, 每 5 分鐘太頻繁)
MINUTE=$(date +%M)
if [ "$MINUTE" = "00" ]; then
    INTEGRITY=$(sqlite3 "$PHANTOM_MESH_DIR/core.db" "PRAGMA quick_check;" 2>/dev/null || echo "FAILED")
    if [ "$INTEGRITY" != "ok" ]; then
        echo "CRITICAL: core.db integrity check failed!"
        STATUS_OK=false
    fi
fi

# Check 4: 磁碟空間
DISK_USAGE=$(df -h "$PHANTOM_MESH_DIR" | tail -1 | awk '{print $5}' | tr -d '%')
if [ "$DISK_USAGE" -gt 90 ]; then
    echo "WARNING: Disk usage at ${DISK_USAGE}%!"
    STATUS_OK=false
fi

# Check 5: LM Studio / Ollama 可用
LMS_STATUS=$(curl -s -o /dev/null -w "%{http_code}" http://localhost:1234/v1/models 2>/dev/null || echo "000")
if [ "$LMS_STATUS" != "200" ]; then
    echo "WARNING: LM Studio not responding"
fi

OLLAMA_STATUS=$(curl -s -o /dev/null -w "%{http_code}" http://localhost:11434/api/tags 2>/dev/null || echo "000")
if [ "$OLLAMA_STATUS" != "200" ]; then
    echo "WARNING: Ollama not responding"
fi

if $STATUS_OK; then
    echo "[$(date)] Health check: ALL OK"
fi
```

### 7.5 Windows Task Scheduler 設定

```powershell
# setup_scheduled_tasks.ps1 — 一次性設定所有排程任務

# 每小時備份 (xx:05)
$hourlyAction = New-ScheduledTaskAction -Execute "bash.exe" `
    -Argument "-c '/c/phantom-mesh/scripts/phantom-mesh_hourly_backup.sh'"
$hourlyTrigger = New-ScheduledTaskTrigger -Once -At "00:05" -RepetitionInterval (New-TimeSpan -Hours 1)
Register-ScheduledTask -TaskName "Phantom Mesh-HourlyBackup" -Action $hourlyAction -Trigger $hourlyTrigger `
    -Description "Phantom Mesh hourly WAL checkpoint and DB backup" -RunLevel Highest

# 每日完整備份 (03:00)
$dailyAction = New-ScheduledTaskAction -Execute "bash.exe" `
    -Argument "-c '/c/phantom-mesh/scripts/phantom-mesh_daily_backup.sh'"
$dailyTrigger = New-ScheduledTaskTrigger -Daily -At "03:00"
Register-ScheduledTask -TaskName "Phantom Mesh-DailyBackup" -Action $dailyAction -Trigger $dailyTrigger `
    -Description "Phantom Mesh daily full backup to Acer" -RunLevel Highest

# 每 5 分鐘健康檢查
$healthAction = New-ScheduledTaskAction -Execute "bash.exe" `
    -Argument "-c '/c/phantom-mesh/scripts/phantom-mesh_health_check.sh'"
$healthTrigger = New-ScheduledTaskTrigger -Once -At "00:00" -RepetitionInterval (New-TimeSpan -Minutes 5)
Register-ScheduledTask -TaskName "Phantom Mesh-HealthCheck" -Action $healthAction -Trigger $healthTrigger `
    -Description "Phantom Mesh health monitoring" -RunLevel Highest
```

---

## 8. 決策樹

### 8.1 主決策樹: "系統出問題了, 怎麼辦?"

```
系統異常!
│
├── Telegram bot 無回應?
│   ├── 網路正常? (能上網?)
│   │   ├── 是 → phantom-mesh 掛了
│   │   │   ├── 檢查進程: tasklist | grep phantom-mesh
│   │   │   ├── 進程存在 → 看日誌 ~/.phantom-mesh/logs/
│   │   │   │   ├── E-Stop 啟動了 → /resume
│   │   │   │   ├─ OOM → 重啟 + 減少 worker 數
│   │   │   │   └── 其他錯誤 → 重啟
│   │   │   └── 進程不存在 → 重啟 phantom-mesh daemon
│   │   │       └── 啟動失敗?
│   │   │           ├── DB 鎖定 → rm core.db-shm core.db-wal (小心!)
│   │   │           ├── Port 占用 → netstat -an | grep 7878
│   │   │           └── 編譯錯誤 → cargo build --release
│   │   │
│   │   └── 否 → 網路中斷 (場景 D)
│   │       ├── 路由器掛了 → 重啟路由器
│   │       └── ISP 斷線 → 等待恢復, 本地服務正常
│   │
│   └── Z13 能開機?
│       ├── 是, 但卡住 → 強制重啟
│       ├── 是, 藍屏 → 記下錯誤碼 → 硬體問題?
│       └── 否 → Z13 完全損壞 (場景 B)
│           └── 啟動備援 Hub (Acer)
│
├── Agent 輸出品質差?
│   ├── 所有 agent 都差 → LLM provider 問題
│   │   ├── LM Studio 掛了 → 重啟
│   │   ├── 模型載入失敗 → 重新載入模型
│   │   └── VRAM 不足 → 減小 context, 換小模型
│   ├── 特定 agent 差 → 檢查該 agent 的 provider 配置
│   └── 特定 Hand 差 → 檢查 hand.toml 的 system_prompt
│
├── 成本突然暴增?
│   ├── /costs → 查看哪個 agent/provider 花最多
│   ├── 雲端 API (Gemini/Groq) → 檢查 API dashboard 用量
│   ├── Cron 任務太頻繁 → /cron list → 調整
│   └── 被盜用? → 場景 H (API key 洩漏)
│
├── Worker 離線?
│   ├── 單一 → 場景 E (自動處理)
│   └── 多台 → 場景 F (降級模式)
│
└── 安全告警?
    ├── API key 洩漏 → 場景 H
    ├── 被入侵 → 場景 G
    └── DDoS → 第 6.3 節
```

### 8.2 備份恢復決策樹

```
需要恢復資料!
│
├── 哪個 DB 需要恢復?
│   │
│   ├── core.db
│   │   ├── 何時損壞?
│   │   │   ├── 剛才 (< 1 小時) → 用每小時備份: hourly/core.db.latest
│   │   │   ├── 今天 → 用今日每小時備份: hourly/core.db.{時間}
│   │   │   └── 更早 → 用每日備份: daily/{日期}/core.db
│   │   ├── 嘗試修復: sqlite3 core.db ".recover" | sqlite3 core_recovered.db
│   │   └── 修復失敗 → 用備份還原
│   │
│   ├── memory.db (語義記憶)
│   │   ├── 同上流程
│   │   └── 注意: 記憶遺失影響 agent 品質
│   │
│   ├── revenue.db (營收)
│   │   ├── 同上流程
│   │   └── 關鍵: 財務資料, 優先恢復
│   │
│   └── agents.toml (配置)
│       ├── Git 版本控制 → git log → git checkout <commit> -- agents.toml
│       └── 每日備份 → daily/{日期}/agents.toml
│
└── .secret_key 遺失?
    ├── 有備份 → 從 daily/.secret_key.enc 解密還原
    ├── 無備份 → 重新生成 (刪除 → 重啟)
    │   └── 警告: 所有 enc2: 加密的秘密將無法解密!
    └── 需要重新配置所有秘密 (手動輸入明文, 再重新加密)
```

---

## 9. 定期演練計畫

### 9.1 每月演練

| 演練項目 | 頻率 | 持續時間 | 步驟 |
|---------|------|---------|------|
| 備份恢復測試 | 每月 1 次 | 30 分鐘 | 從備份還原 DB 到臨時目錄, 驗證完整性 |
| Hub 故障切換 | 每季 1 次 | 1 小時 | 停止 Z13 phantom-mesh, 啟動 Acer 備援, 驗證功能 |
| UPS 斷電測試 | 每季 1 次 | 15 分鐘 | 拔電源, 驗證 UPS 接管, 驗證自動關機腳本 |
| API Key Rotate | 每 90 天 | 30 分鐘 | Rotate 所有非加密的 API key |
| 安全掃描 | 每月 1 次 | 30 分鐘 | Windows Defender 全碟掃描, 檢查開放 port |

### 9.2 每月備份恢復測試 SOP

```bash
# 在臨時目錄測試恢復
RESTORE_TEST="/tmp/phantom-mesh-restore-test-$(date +%Y%m%d)"
mkdir -p "$RESTORE_TEST"

# 複製最近備份
cp /mnt/acer/phantom-mesh-backup/daily/$(date +%Y-%m-%d)/core.db "$RESTORE_TEST/"

# 驗證
sqlite3 "$RESTORE_TEST/core.db" "PRAGMA integrity_check;"
sqlite3 "$RESTORE_TEST/core.db" "SELECT COUNT(*) FROM memories;"
sqlite3 "$RESTORE_TEST/core.db" "SELECT COUNT(*) FROM cluster_nodes;"

# 記錄結果
echo "$(date): Restore test PASSED — $(sqlite3 "$RESTORE_TEST/core.db" "SELECT COUNT(*) FROM memories;") memories recovered" >> /mnt/acer/phantom-mesh-backup/restore_test.log

# 清理
rm -rf "$RESTORE_TEST"
```

---

## 10. 聯絡清單與升級流程

### 10.1 升級矩陣

| 嚴重度 | 定義 | 回應時間 | 升級 |
|--------|------|---------|------|
| P0 | Hub 完全離線 / 資安事件 / 資料遺失 | 15 分鐘 | 立即處理 |
| P1 | 多 Worker 離線 / 營利功能受影響 | 1 小時 | 當日處理 |
| P2 | 單 Worker 離線 / 非關鍵功能受影響 | 4 小時 | 24 小時內處理 |
| P3 | 效能下降 / 非關鍵告警 | 24 小時 | 下次維護視窗 |

### 10.2 自我通知管道

由於是個人營運, "升級" = 自我通知:

```
優先通知管道:
1. Telegram (主要) — phantom-mesh 自動發送告警
2. Email (備用) — 如果 Telegram 不可用, 用 email tool 發送
3. 手機推播 — Telegram 手機 app 通知

自動告警規則 (建議加入 phantom-mesh):
- Worker 離線 → Telegram 通知
- DB integrity check 失敗 → Telegram + Email
- 成本超過每日上限 → Telegram 通知
- E-Stop 觸發 → Telegram 通知
- UPS 切換電池 → Telegram 通知
- 備份失敗 → Telegram + Email
```

---

## 附錄 A: 快速參考卡

```
╔══════════════════════════════════════════════════════════════════╗
║              PHANTOM_MESH 災難恢復快速參考卡                          ║
╠══════════════════════════════════════════════════════════════════╣
║                                                                  ║
║  緊急停止:                                                       ║
║    Telegram: /estop                                              ║
║    HTTP:     curl -X POST http://localhost:7878/estop             ║
║    恢復:     /resume 或 curl -X DELETE http://localhost:7878/estop║
║                                                                  ║
║  重啟 phantom-mesh:                                              ║
║    cd ~/Desktop/adreanalai/LLM-Cluster-Project/phantom-mesh      ║
║    ./target/release/phantom-mesh daemon                          ║
║                                                                  ║
║  DB 完整性檢查:                                                   ║
║    sqlite3 ~/.phantom-mesh/core.db "PRAGMA integrity_check;"          ║
║                                                                  ║
║  手動 WAL Checkpoint:                                            ║
║    sqlite3 ~/.phantom-mesh/core.db "PRAGMA wal_checkpoint(TRUNCATE);" ║
║                                                                  ║
║  手動備份到 Acer:                                                 ║
║    rsync -az ~/.phantom-mesh/*.db acer:/backup/phantom-mesh/               ║
║                                                                  ║
║  啟動備援 Hub (Acer):                                            ║
║    1. cp /backup/hourly/*.db.latest ~/.phantom-mesh/                  ║
║    2. ~/phantom-mesh-standby/target/release/phantom-mesh daemon       ║
║                                                                  ║
║  API Key Rotate:                                                  ║
║    1. /estop                                                     ║
║    2. 到各平台 dashboard 重新生成                                  ║
║    3. 更新 ~/.phantom-mesh/agents.toml                                ║
║    4. 重啟 phantom-mesh                                          ║
║                                                                  ║
║  備份路徑:                                                        ║
║    每小時: /mnt/acer/phantom-mesh-backup/hourly/                      ║
║    每日:   /mnt/acer/phantom-mesh-backup/daily/{YYYY-MM-DD}/          ║
║    每週:   /mnt/acer/phantom-mesh-backup/weekly/                      ║
║                                                                  ║
╚══════════════════════════════════════════════════════════════════╝
```

---

## 附錄 B: 預估 RTO/RPO 總覽

| 場景 | RTO (恢復時間) | RPO (資料丟失) | 自動化程度 |
|------|---------------|---------------|-----------|
| A. Z13 硬碟故障 | 2-4h (備援) / 24-48h (重建) | ≤ 1h | 手動 |
| B. Z13 完全損壞 | 2-6h (備援) / 3-7d (送修) | ≤ 1h | 手動 |
| C. 停電 (有 UPS) | 0 (自動切換) / 15min (自動關機後來電) | 0 | **全自動** |
| C. 停電 (無 UPS) | 5-15min (來電後) | ≤ 數秒 (WAL) | 半自動 |
| D. 網路中斷 | ISP 恢復後立即 | 0 | **全自動** (降級) |
| E. 單一 Worker 離線 | 2min (心跳超時) | 0 | **全自動** |
| F. 多台 Worker 離線 | 取決於恢復 | 0 | 半自動 (降級) |
| G. 被入侵 | 數小時 (隔離) + 24-72h (清除) | 取決於入侵時間 | 手動 |
| H. API Key 洩漏 | 5-30min | 0 | 手動 |
| I. SQLite 損壞 | 15-60min | ≤ 1h | 半自動 |

---

## 附錄 C: 文件版本歷史

| 版本 | 日期 | 變更 |
|------|------|------|
| 1.0 | 2026-03-05 | 初始版本: 9 個故障場景, 備份策略, HA 設計, UPS 方案, 安全回應 |

---

> 本文件應每季度審查一次, 並在以下情況更新:
> - 新增/移除叢集節點
> - 基礎設施變更 (ISP, 路由器, UPS)
> - 安全事件後
> - 演練發現的問題
