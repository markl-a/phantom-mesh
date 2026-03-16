# Clawtex 8 機 AI 集群 — 監控與可觀測性系統設計

**日期**: 2026-03-05
**狀態**: 設計完成
**範圍**: 8 節點 AI 推理集群的 metrics / alerting / logging / dashboard

---

## 目錄

1. [架構總覽](#1-架構總覽)
2. [Metrics 指標設計](#2-metrics-指標設計)
3. [技術方案選擇 — 推薦方案 C](#3-技術方案選擇)
4. [Telegram Dashboard 設計](#4-telegram-dashboard-設計)
5. [告警規則](#5-告警規則)
6. [日誌管理](#6-日誌管理)
7. [Rust 實作建議](#7-rust-實作建議)
8. [Grafana Dashboard JSON](#8-grafana-dashboard-json)
9. [實作優先級與時間表](#9-實作優先級與時間表)

---

## 1. 架構總覽

```
┌─────────────────────────────────────────────────────────────┐
│                    Hub (Z13 或 Server)                       │
│                                                             │
│  clawtex-core daemon                                        │
│  ├── MetricsRegistry (in-memory, lock-free atomics)        │
│  ├── NodeCollector (pull /health from workers, 30s)        │
│  ├── AlertEngine (evaluate rules, 60s)                     │
│  ├── GET /metrics (Prometheus text format)                  │
│  ├── GET /api/cluster/status (JSON)                        │
│  ├── Telegram AlertBot (push CRITICAL/WARNING/INFO)        │
│  └── SQLite metrics_history (5min rollups, 90 day retain)  │
│                                                             │
│  [Optional] Prometheus ←─ scrape /metrics (15s)            │
│  [Optional] Grafana ←─── query Prometheus                  │
└───────────┬─────────────────────────────────────────────────┘
            │ HTTP pull (every 30s)
     ┌──────┼──────┬──────┬──────┬──────┬──────┬──────┐
     │      │      │      │      │      │      │      │
   Node1  Node2  Node3  Node4  Node5  Node6  Node7  Node8
   (Worker) each exposes:
   GET /health → { cpu, gpu, ram, temp, disk, models, tok_s, queue }
```

### 核心原則

1. **零外部依賴可用**: 沒有 Prometheus/Grafana 也能完整運作（SQLite + Telegram）
2. **可選升級**: 任何時候加裝 Prometheus scraper 即可接入 Grafana
3. **Hub 主動 Pull**: 不要求 Worker 推送 — Hub 每 30 秒拉 `/health`
4. **告警零延遲**: CRITICAL 事件直接 Telegram 推送，不等 Grafana

---

## 2. Metrics 指標設計

### 2.1 節點層級指標 (per-node)

每台 Worker 的 `/health` endpoint 回傳：

```json
{
  "node": "z13",
  "timestamp": "2026-03-05T14:30:00Z",
  "system": {
    "cpu_percent": 45.2,
    "gpu_percent": 78.5,
    "gpu_vram_used_mb": 24576,
    "gpu_vram_total_mb": 32768,
    "ram_used_mb": 48128,
    "ram_total_mb": 65536,
    "cpu_temp_c": 72.0,
    "gpu_temp_c": 68.0,
    "disk_used_gb": 450,
    "disk_total_gb": 1000,
    "npu_utilization_percent": 35.0,
    "uptime_secs": 86400,
    "load_avg_1m": 4.2
  },
  "inference": {
    "loaded_models": ["qwen3-coder:32b", "llama3.3:70b-q4"],
    "unloaded_models": ["deepseek-v3:236b"],
    "total_requests": 1523,
    "active_requests": 3,
    "queue_depth": 2,
    "avg_tok_s": 42.5,
    "p50_latency_ms": 230,
    "p95_latency_ms": 890,
    "p99_latency_ms": 2100,
    "errors_total": 12,
    "oom_kills": 0
  }
}
```

**Prometheus metrics 名稱規範**:

| Metric Name | Type | Labels | 說明 |
|---|---|---|---|
| `clawtex_node_cpu_percent` | gauge | `node` | CPU 使用率 |
| `clawtex_node_gpu_percent` | gauge | `node` | GPU 使用率 |
| `clawtex_node_gpu_vram_used_bytes` | gauge | `node` | GPU VRAM 已用 |
| `clawtex_node_gpu_vram_total_bytes` | gauge | `node` | GPU VRAM 總量 |
| `clawtex_node_ram_used_bytes` | gauge | `node` | RAM 已用 |
| `clawtex_node_ram_total_bytes` | gauge | `node` | RAM 總量 |
| `clawtex_node_cpu_temp_celsius` | gauge | `node` | CPU 溫度 |
| `clawtex_node_gpu_temp_celsius` | gauge | `node` | GPU 溫度 |
| `clawtex_node_disk_used_bytes` | gauge | `node` | 磁碟已用 |
| `clawtex_node_disk_total_bytes` | gauge | `node` | 磁碟總量 |
| `clawtex_node_npu_percent` | gauge | `node` | NPU 使用率 |
| `clawtex_node_uptime_seconds` | gauge | `node` | 節點 uptime |
| `clawtex_node_status` | gauge | `node` | 1=online, 0=offline |

### 2.2 模型層級指標 (per-model)

| Metric Name | Type | Labels | 說明 |
|---|---|---|---|
| `clawtex_model_loaded` | gauge | `node`, `model` | 1=loaded, 0=unloaded |
| `clawtex_model_requests_total` | counter | `node`, `model` | 模型請求總數 |
| `clawtex_model_tokens_per_second` | gauge | `node`, `model` | 即時 tok/s |
| `clawtex_model_queue_depth` | gauge | `node`, `model` | 等待佇列深度 |
| `clawtex_model_latency_ms` | histogram | `node`, `model` | 請求延遲分佈 |
| `clawtex_model_errors_total` | counter | `node`, `model` | 模型錯誤數 |

### 2.3 Hand 執行指標 (per-hand)

| Metric Name | Type | Labels | 說明 |
|---|---|---|---|
| `clawtex_hand_executions_total` | counter | `hand`, `status` | 執行次數 (success/fail) |
| `clawtex_hand_duration_seconds` | histogram | `hand` | 執行耗時 |
| `clawtex_hand_phase_duration_seconds` | histogram | `hand`, `phase` | 各階段耗時 |
| `clawtex_hand_quality_score` | gauge | `hand` | 最近品質分 (0-100) |
| `clawtex_hand_active` | gauge | `hand` | 當前執行中數量 |
| `clawtex_hand_chain_completions_total` | counter | `hand`, `chained_to` | 鏈式完成數 |

### 2.4 Provider 指標 (per-provider)

| Metric Name | Type | Labels | 說明 |
|---|---|---|---|
| `clawtex_provider_requests_total` | counter | `provider`, `model`, `status` | 請求數 |
| `clawtex_provider_latency_ms` | histogram | `provider`, `model` | 延遲分佈 |
| `clawtex_provider_errors_total` | counter | `provider`, `model`, `error_type` | 錯誤數 (按類型) |
| `clawtex_provider_tokens_in_total` | counter | `provider`, `model` | 輸入 token 數 |
| `clawtex_provider_tokens_out_total` | counter | `provider`, `model` | 輸出 token 數 |
| `clawtex_provider_cost_usd_total` | counter | `provider`, `model` | 累計花費 USD |
| `clawtex_provider_circuit_open` | gauge | `provider` | 斷路器狀態 (1=open) |
| `clawtex_provider_quota_remaining` | gauge | `provider` | 配額剩餘 (如 Groq RPM) |

### 2.5 營收指標

| Metric Name | Type | Labels | 說明 |
|---|---|---|---|
| `clawtex_revenue_daily_usd` | gauge | | 今日營收 |
| `clawtex_revenue_total_usd` | counter | `route`, `source` | 累計營收 |
| `clawtex_cost_daily_usd` | gauge | | 今日成本 |
| `clawtex_profit_margin_percent` | gauge | | 利潤率 |
| `clawtex_revenue_pending_usd` | gauge | | 待確認營收 |

### 2.6 集群整體指標

| Metric Name | Type | Labels | 說明 |
|---|---|---|---|
| `clawtex_cluster_nodes_total` | gauge | `status` | 節點數 (online/offline) |
| `clawtex_cluster_throughput_tok_s` | gauge | | 集群總吞吐 |
| `clawtex_cluster_active_requests` | gauge | | 全集群活躍請求 |
| `clawtex_cluster_total_vram_gb` | gauge | | 全集群 VRAM 總量 |
| `clawtex_cluster_used_vram_gb` | gauge | | 全集群 VRAM 已用 |
| `clawtex_sot_parallel_degree` | gauge | | SoT 當前並行度 |
| `clawtex_sot_executions_total` | counter | `status` | SoT 執行數 |
| `clawtex_sot_speedup_ratio` | gauge | | SoT 最近加速倍率 |
| `clawtex_estop_active` | gauge | | E-Stop 狀態 |

---

## 3. 技術方案選擇

### 方案比較

| | 方案 A: Prometheus + Grafana | 方案 B: SQLite + Telegram | 方案 C: 混合 (推薦) |
|---|---|---|---|
| **外部依賴** | Prometheus + Grafana (需 Docker) | 無 | 可選 |
| **記憶體用量** | ~300MB (Prom) + ~200MB (Grafana) | ~10MB (SQLite) | ~10MB 基礎 |
| **視覺化能力** | 極佳 (Grafana panels) | 文字 Telegram only | 文字 + 可選 Grafana |
| **告警能力** | Alertmanager + Grafana alerts | 自建 Telegram alerts | 自建 (足夠) |
| **歷史數據** | Prometheus TSDB (2 weeks default) | SQLite rollups (90 days) | SQLite + 可選 Prom |
| **設定複雜度** | 高 (3 個服務) | 低 (零設定) | 低 (核心) / 中 (加裝) |
| **需要額外機器?** | 最好有 (至少 2GB RAM) | 不需要 | 不需要 |
| **佈署時間** | 2-4 小時 | 30 分鐘 | 30 分鐘核心 + 按需加裝 |

### 推薦: 方案 C — 混合方案

**理由**:

1. **核心**（必須建構）: MetricsRegistry 已經存在於 `src/metrics.rs`，已有 `render_prometheus()` — 只需擴展
2. **SQLite rollup 層**: 每 5 分鐘把 in-memory metrics snapshot 寫到 SQLite，自動刪除 90 天前的資料
3. **Telegram dashboard**: 文字格式推送，日常使用完全足夠
4. **可選 Prometheus**: 隨時 `docker run prom/prometheus` 指向 Hub `/metrics`，零程式碼修改
5. **可選 Grafana**: Prometheus 加好後 `docker run grafana/grafana`，匯入本文件的 JSON

**實際效果**: 開發 1 天即可上線核心功能。之後任何時候想要漂亮圖表，花 30 分鐘加裝 Docker 即可。

### 方案 C 架構圖

```
clawtex-core (Hub)
├── MetricsRegistry           ← 已有，需擴展 label 支援
│   ├── counters (AtomicU64)
│   ├── gauges (AtomicU64)
│   └── histograms (lock-free)
├── NodeCollector             ← 新增：每 30s pull workers /health
│   └── 更新 MetricsRegistry gauges
├── MetricsRollup             ← 新增：每 5min snapshot → SQLite
│   └── metrics_snapshots table (保留 90 天)
├── AlertEngine               ← 新增：每 60s 評估規則
│   └── → TelegramChannel.send_message()
├── GET /metrics              ← 已有：Prometheus text format
├── GET /api/cluster/status   ← 新增：JSON API
├── Telegram Commands         ← 新增：/status /metrics /health
└── SQLite                    ← 已有 cost_records + revenue_records
    ├── metrics_snapshots     ← 新增
    └── alert_history         ← 新增
```

---

## 4. Telegram Dashboard 設計

### 4.1 `/status` — 集群狀態概覽

```
📊 Clawtex Cluster Status
━━━━━━━━━━━━━━━━━━━━━━
⏱ Uptime: 3d 14h 22m
🔧 E-Stop: INACTIVE

📡 Nodes: 7/8 online
  ✅ z13 (hub)    CPU:45% GPU:78% RAM:73% 🌡72°C
  ✅ m1-desktop   CPU:32% GPU:65% RAM:58% 🌡65°C
  ✅ m2-server    CPU:28% GPU:82% RAM:61% 🌡70°C
  ✅ m3-nuc       CPU:55% GPU: -- RAM:45% 🌡58°C
  ✅ m4-laptop    CPU:38% GPU:71% RAM:52% 🌡63°C
  ✅ m5-rack      CPU:22% GPU:90% RAM:70% 🌡74°C
  ✅ m6-mini      CPU:41% GPU:60% RAM:48% 🌡55°C
  ❌ m7-backup    OFFLINE (last seen: 2h ago)

🧠 Models Loaded: 12
  qwen3-coder:32b ×3  |  llama3.3:70b-q4 ×2
  deepseek-v3:236b ×1 |  phi-4:14b ×4
  gemma-3:27b ×2

⚡ Throughput: 285 tok/s (cluster)
📋 Queue: 5 active, 2 pending
🔄 SoT: 3-way parallel active
```

### 4.2 `/metrics` — 今日效能摘要

```
📈 Today's Performance (2026-03-05)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
🔢 Total Requests: 1,523
⚡ Avg Throughput: 42.5 tok/s
⏱ Avg Latency: 230ms (p50) / 890ms (p95)
❌ Error Rate: 0.8% (12/1523)

📊 By Provider:
  lmstudio   892 reqs  avg 180ms  err 0.2%
  ollama     423 reqs  avg 350ms  err 1.4%
  gemini     156 reqs  avg 420ms  err 0.6%
  anthropic   52 reqs  avg 1.2s   err 0.0%

🤖 By Hand:
  content      8 runs  avg 12m  ✅100%
  freelancer   5 runs  avg 25m  ✅ 80%
  seo_content  3 runs  avg 18m  ✅100%
  outreach     6 runs  avg 15m  ✅ 83%

💰 Token Usage: 2.4M tokens
💵 Estimated Cost: $0.42
```

### 4.3 `/revenue` — 營收儀表板

```
💰 Revenue Dashboard
━━━━━━━━━━━━━━━━━━
📅 Today: $125.00 (3 transactions)
📅 This Week: $830.50 (18 transactions)
📅 This Month: $3,245.00 (67 transactions)

📊 By Route (this month):
  A:freelance_dev      $1,800.00  (55%)
  B:saas_products        $520.00  (16%)
  C:content_monetization $380.00  (12%)
  E:api_services         $295.00   (9%)
  H:automation_services  $250.00   (8%)

💵 Cost vs Revenue (today):
  Revenue:  $125.00
  Cost:     $  0.42  (API calls)
  Infra:    $  2.80  (electricity est.)
  Profit:   $121.78  (97.4% margin)

📈 Trend: ▲ +15% vs last week

⏳ Pending: $450.00 (2 invoices)
```

### 4.4 `/health` — 各節點健康

```
🏥 Node Health Detail
━━━━━━━━━━━━━━━━━━
┌─ z13 (hub) ──────────────────────┐
│ CPU: ████████░░ 78%   Temp: 72°C │
│ GPU: ████████░░ 82%   Temp: 68°C │
│ RAM: ███████░░░ 73%   NPU:  35%  │
│ Disk: ████░░░░░░ 45%              │
│ Models: qwen3-coder:32b (42 t/s) │
│         phi-4:14b (95 t/s)        │
│ Queue: 2 active, 1 pending        │
│ Uptime: 3d 14h                    │
└──────────────────────────────────┘

┌─ m1-desktop ─────────────────────┐
│ CPU: ██████░░░░ 58%   Temp: 65°C │
│ GPU: ██████████ 95%   Temp: 78°C │  ⚠️ GPU HIGH
│ RAM: █████░░░░░ 52%              │
│ Disk: ██░░░░░░░░ 22%              │
│ Models: llama3.3:70b-q4 (18 t/s) │
│ Queue: 1 active, 0 pending        │
│ Uptime: 12d 5h                    │
└──────────────────────────────────┘
...
```

### 4.5 每小時 Heartbeat（自動推送）

```
💓 Hourly Heartbeat [14:00]
Nodes: 7/8 🟢  |  Requests: 234  |  Errors: 1
Throughput: 290 tok/s  |  Queue: 0
Revenue: $45.00  |  Cost: $0.08
```

### 4.6 Telegram Command 路由表

| Command | 說明 | 回覆格式 |
|---|---|---|
| `/status` | 集群整體概覽 | 見 4.1 |
| `/metrics` | 今日效能摘要 | 見 4.2 |
| `/metrics 7d` | 最近 7 天效能 | 表格格式 |
| `/revenue` | 營收儀表板 | 見 4.3 |
| `/revenue 30d` | 30 天營收明細 | 表格格式 |
| `/health` | 所有節點健康 | 見 4.4 |
| `/health z13` | 單節點健康 | 單節點詳細 |
| `/alerts` | 最近告警歷史 | 時間線格式 |
| `/costs` | 今日成本 (已有) | 已實作 |
| `/estop` | 緊急停止 (已有) | 已實作 |

---

## 5. 告警規則

### 5.1 告警定義結構

```rust
pub struct AlertRule {
    pub name: String,
    pub severity: AlertSeverity,  // Critical, Warning, Info
    pub condition: AlertCondition,
    pub cooldown_secs: u64,       // 避免重複告警
    pub message_template: String,
}

pub enum AlertSeverity {
    Critical,  // 🔴 立即推送 + 重複推送直到解除
    Warning,   // 🟡 推送一次，cooldown 後再檢查
    Info,      // 🔵 推送一次
}

pub enum AlertCondition {
    NodeOffline { node: String, timeout_secs: u64 },
    AllNodesOffline,
    HubOffline,
    MetricAbove { metric: String, threshold: f64 },
    MetricBelow { metric: String, threshold: f64 },
    ErrorRateAbove { provider: String, threshold_percent: f64 },
    HandFailRate { hand: String, threshold_percent: f64, window_hours: u32 },
    Custom(String),  // 未來擴充用
}
```

### 5.2 預設告警規則

#### CRITICAL (🔴)

| 規則名稱 | 條件 | Cooldown | 說明 |
|---|---|---|---|
| `hub_offline` | Hub 自身 health check 失敗 | 0s (持續) | Hub 離線 = 整個系統離線 |
| `all_inference_offline` | 所有推理節點離線 | 60s | 無法處理任何請求 |
| `disk_critical` | 任何節點磁碟 > 95% | 300s | 可能導致 OOM/crash |
| `gpu_thermal_critical` | GPU 溫度 > 90°C | 60s | 硬體損壞風險 |
| `estop_triggered` | E-Stop 被觸發 | 0s | 人工緊急停止 |
| `oom_detected` | 任何節點 OOM kill > 0 | 60s | 模型載入失敗 |

#### WARNING (🟡)

| 規則名稱 | 條件 | Cooldown | 說明 |
|---|---|---|---|
| `node_offline` | 單個節點離線 > 2 分鐘 | 600s | 集群降級 |
| `throughput_drop` | 吞吐量下降 > 50% (vs 1h avg) | 600s | 效能異常 |
| `disk_high` | 任何節點磁碟 > 90% | 1800s | 即將滿 |
| `gpu_temp_high` | GPU 溫度 > 80°C | 600s | 需要注意散熱 |
| `error_rate_high` | Provider 錯誤率 > 10% (5min) | 300s | 服務品質下降 |
| `hand_fail_rate` | Hand 失敗率 > 30% (24h) | 3600s | 工作流異常 |
| `circuit_open` | 任何 Provider 斷路器打開 | 300s | Provider 不穩定 |
| `queue_depth_high` | 推理佇列 > 20 | 300s | 過載 |
| `budget_warning` | 日成本接近預算 80% | 3600s | 成本控制 |

#### INFO (🔵)

| 規則名稱 | 條件 | Cooldown | 說明 |
|---|---|---|---|
| `node_joined` | 新節點上線 | 0s | 集群擴容 |
| `node_recovered` | 離線節點恢復 | 0s | 問題解決 |
| `model_loaded` | 新模型載入 | 0s | 模型部署 |
| `revenue_milestone` | 日營收突破 $100/$500/$1000 | 86400s | 營收目標 |
| `hand_completed` | Hand 鏈式完成 | 0s | 工作流結束 |
| `daily_summary` | 每日 23:00 | 86400s | 日報 |

### 5.3 告警訊息範例

```
🔴 CRITICAL: All Inference Nodes Offline
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
時間: 2026-03-05 14:32:15 UTC
狀態: 8/8 節點離線
最後上線: m2-server (2m 前)
影響: 所有推理請求將失敗
動作: 請檢查網路 / 電源 / VPN

/resume 解除 E-Stop
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━


🟡 WARNING: GPU Temperature High
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
節點: m1-desktop
GPU: Radeon RX 7900 XTX
溫度: 84°C (閾值: 80°C)
負載: GPU 95%, 模型 llama3.3:70b-q4
建議: 降低推理負載或改善散熱


🔵 INFO: Revenue Milestone
━━━━━━━━━━━━━━━━━━━━━━
🎉 今日營收突破 $100!
當前: $125.00 (3 筆交易)
路線: A:freelance_dev ($95), B:saas ($30)
```

### 5.4 告警歷史表 (SQLite)

```sql
CREATE TABLE IF NOT EXISTS alert_history (
    id          TEXT PRIMARY KEY,
    timestamp   TEXT NOT NULL,
    rule_name   TEXT NOT NULL,
    severity    TEXT NOT NULL,  -- 'critical', 'warning', 'info'
    message     TEXT NOT NULL,
    resolved    INTEGER NOT NULL DEFAULT 0,
    resolved_at TEXT,
    date_key    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_alert_date ON alert_history(date_key);
CREATE INDEX IF NOT EXISTS idx_alert_severity ON alert_history(severity);
```

---

## 6. 日誌管理

### 6.1 結構化日誌格式 (JSON)

使用 `tracing` + `tracing-subscriber` 的 JSON 層:

```json
{
  "timestamp": "2026-03-05T14:30:00.123Z",
  "level": "INFO",
  "target": "clawtex_core::agent_runtime",
  "span": {
    "agent": "master",
    "request_id": "req-abc123"
  },
  "fields": {
    "message": "Agent round completed",
    "tokens_in": 1500,
    "tokens_out": 350,
    "duration_ms": 2300,
    "provider": "lmstudio",
    "model": "qwen3-coder:32b",
    "tool_calls": 3
  }
}
```

### 6.2 日誌分級策略

| Level | 用途 | 範例 | 預設啟用 |
|---|---|---|---|
| `ERROR` | 不可恢復的失敗 | Provider 全部失敗、DB 損壞 | 全部環境 |
| `WARN` | 可恢復的異常 | 單 Provider 失敗 fallback、circuit break | 全部環境 |
| `INFO` | 業務事件 | Hand 開始/完成、模型載入、節點上線 | Production |
| `DEBUG` | 開發除錯 | Tool 執行參數、API 請求/回應摘要 | 開發環境 |
| `TRACE` | 極細粒度 | 完整 API payload、token 逐個處理 | 手動啟用 |

**環境變數控制**:
```bash
# Production (預設)
RUST_LOG=clawtex_core=info,tower_http=warn

# Debug 模式
RUST_LOG=clawtex_core=debug,tower_http=info

# 單模組 trace
RUST_LOG=clawtex_core::providers=trace,clawtex_core=info
```

### 6.3 日誌 Rotation

使用 `tracing-appender` 的 `RollingFileAppender`:

```rust
// 日誌 rotation 設定
let file_appender = tracing_appender::rolling::Builder::new()
    .rotation(tracing_appender::rolling::Rotation::DAILY)
    .filename_prefix("clawtex")
    .filename_suffix("log")
    .max_log_files(30)  // 保留 30 天
    .build("~/.clawtex/logs/")
    .expect("Failed to create log file appender");
```

**Rotation 策略**:
- **Hub**: 每日 rotate，保留 30 天（約 500MB/月 at INFO level）
- **Worker**: 每日 rotate，保留 7 天（Worker 日誌量較小）
- **壓縮**: 超過 3 天的 log 自動 gzip（由外部 cron 處理）

### 6.4 集中日誌 (Worker → Hub)

**方案**: Worker 每 60 秒批量 POST 日誌到 Hub

```
Worker                          Hub
  │                              │
  │ 本地 ring buffer (10000行)   │ 接收 + 寫入 SQLite
  │ ──── POST /api/logs ────────>│  (按 node 分 table)
  │     (batch 100-500 lines)    │
  │                              │
  │ 如果 Hub 不可達:             │
  │   寫入本地檔案 fallback      │
  │   Hub 恢復後回補             │
```

**集中日誌 SQLite 表**:

```sql
CREATE TABLE IF NOT EXISTS cluster_logs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp   TEXT NOT NULL,
    node        TEXT NOT NULL,
    level       TEXT NOT NULL,
    target      TEXT NOT NULL,
    message     TEXT NOT NULL,
    fields_json TEXT,
    date_key    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_logs_node_date ON cluster_logs(node, date_key);
CREATE INDEX IF NOT EXISTS idx_logs_level ON cluster_logs(level);

-- 自動清理 (保留 14 天)
-- 在 MetricsRollup 任務中一起處理
```

### 6.5 日誌搜尋 Telegram Command

```
/logs z13 error 1h     → 搜尋 z13 最近 1 小時的 error 日誌
/logs all warn 24h     → 所有節點最近 24h 的 warning
/logs m1 debug 10m     → m1 最近 10 分鐘 debug (量可能很大)
```

---

## 7. Rust 實作建議

### 7.1 擴展現有 MetricsRegistry

現有 `src/metrics.rs` 需要加入 **label 支援**。目前 counter/gauge/histogram 都是純字串 key。
為了支援 Prometheus label (e.g. `clawtex_node_cpu_percent{node="z13"}`)，需要修改：

```rust
/// Label-aware metric key: "metric_name{label1=val1,label2=val2}"
/// 用字串組合而非複雜 struct 來保持簡單
pub fn labeled_key(name: &str, labels: &[(&str, &str)]) -> String {
    if labels.is_empty() {
        return name.to_string();
    }
    let pairs: Vec<String> = labels.iter()
        .map(|(k, v)| format!("{}=\"{}\"", k, v))
        .collect();
    format!("{}{{{}}}", name, pairs.join(","))
}

impl MetricsRegistry {
    /// Set a gauge with labels
    pub fn gauge_set_labeled(&self, name: &str, labels: &[(&str, &str)], value: u64) {
        let key = labeled_key(name, labels);
        self.gauge_set(&key, value);
    }

    /// Increment a counter with labels
    pub fn inc_labeled(&self, name: &str, labels: &[(&str, &str)]) {
        let key = labeled_key(name, labels);
        self.inc(&key);
    }

    /// Observe a histogram with labels
    pub fn observe_labeled(&self, name: &str, labels: &[(&str, &str)], value_ms: f64) {
        let key = labeled_key(name, labels);
        // Auto-register if not exists
        {
            let histograms = self.histograms.read().unwrap();
            if !histograms.contains_key(&key) {
                drop(histograms);
                self.register_histogram(&key);
            }
        }
        self.observe(&key, value_ms);
    }

    /// Render Prometheus format with proper label parsing
    /// 已有的 render_prometheus() 自然支援 — key 裡面已含 {labels}
    /// 但需修改 TYPE 行: 從 key 中提取 base name
    pub fn render_prometheus_v2(&self) -> String {
        // ... 見完整實作
    }
}
```

### 7.2 NodeCollector — 節點健康採集器

```rust
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tracing::{info, warn, error};

use crate::cluster::ClusterRegistry;
use crate::metrics::MetricsRegistry;

/// Periodically polls worker /health endpoints and updates metrics
pub struct NodeCollector {
    cluster: Arc<ClusterRegistry>,
    metrics: Arc<MetricsRegistry>,
    client: reqwest::Client,
    poll_interval: Duration,
}

impl NodeCollector {
    pub fn new(
        cluster: Arc<ClusterRegistry>,
        metrics: Arc<MetricsRegistry>,
        poll_interval_secs: u64,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("HTTP client");
        Self {
            cluster,
            metrics,
            client,
            poll_interval: Duration::from_secs(poll_interval_secs),
        }
    }

    /// Start the collection loop (run as tokio::spawn)
    pub async fn run(&self) {
        let mut interval = time::interval(self.poll_interval);
        loop {
            interval.tick().await;
            self.collect_all().await;
        }
    }

    async fn collect_all(&self) {
        let nodes = self.cluster.status().await;
        let mut online_count = 0u64;
        let mut offline_count = 0u64;
        let mut total_tok_s = 0.0f64;
        let mut total_vram_used = 0u64;
        let mut total_vram_total = 0u64;
        let mut total_active = 0u64;

        for node in &nodes {
            let url = format!("http://{}:{}/health", node.host, node.port);
            match self.client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    match resp.json::<serde_json::Value>().await {
                        Ok(health) => {
                            online_count += 1;
                            self.update_node_metrics(&node.name, &health);

                            // Accumulate cluster totals
                            if let Some(sys) = health.get("system") {
                                total_vram_used += sys.get("gpu_vram_used_mb")
                                    .and_then(|v| v.as_u64()).unwrap_or(0);
                                total_vram_total += sys.get("gpu_vram_total_mb")
                                    .and_then(|v| v.as_u64()).unwrap_or(0);
                            }
                            if let Some(inf) = health.get("inference") {
                                total_tok_s += inf.get("avg_tok_s")
                                    .and_then(|v| v.as_f64()).unwrap_or(0.0);
                                total_active += inf.get("active_requests")
                                    .and_then(|v| v.as_u64()).unwrap_or(0);
                            }
                        }
                        Err(e) => {
                            warn!("Failed to parse /health from {}: {}", node.name, e);
                            offline_count += 1;
                            self.mark_node_offline(&node.name);
                        }
                    }
                }
                _ => {
                    offline_count += 1;
                    self.mark_node_offline(&node.name);
                }
            }
        }

        // Update cluster-level metrics
        self.metrics.gauge_set_labeled(
            "clawtex_cluster_nodes_total", &[("status", "online")], online_count);
        self.metrics.gauge_set_labeled(
            "clawtex_cluster_nodes_total", &[("status", "offline")], offline_count);
        self.metrics.gauge_set(
            "clawtex_cluster_throughput_tok_s", (total_tok_s * 100.0) as u64); // x100 for precision
        self.metrics.gauge_set("clawtex_cluster_active_requests", total_active);
        self.metrics.gauge_set("clawtex_cluster_used_vram_gb", total_vram_used / 1024);
        self.metrics.gauge_set("clawtex_cluster_total_vram_gb", total_vram_total / 1024);
    }

    fn update_node_metrics(&self, node: &str, health: &serde_json::Value) {
        let labels = &[("node", node)];
        self.metrics.gauge_set_labeled("clawtex_node_status", labels, 1);

        if let Some(sys) = health.get("system") {
            if let Some(v) = sys.get("cpu_percent").and_then(|v| v.as_f64()) {
                self.metrics.gauge_set_labeled(
                    "clawtex_node_cpu_percent", labels, (v * 100.0) as u64); // x100
            }
            if let Some(v) = sys.get("gpu_percent").and_then(|v| v.as_f64()) {
                self.metrics.gauge_set_labeled(
                    "clawtex_node_gpu_percent", labels, (v * 100.0) as u64);
            }
            if let Some(v) = sys.get("cpu_temp_c").and_then(|v| v.as_f64()) {
                self.metrics.gauge_set_labeled(
                    "clawtex_node_cpu_temp_celsius", labels, (v * 10.0) as u64); // x10
            }
            if let Some(v) = sys.get("gpu_temp_c").and_then(|v| v.as_f64()) {
                self.metrics.gauge_set_labeled(
                    "clawtex_node_gpu_temp_celsius", labels, (v * 10.0) as u64);
            }
            // ... ram, disk, npu similar
        }

        if let Some(inf) = health.get("inference") {
            if let Some(v) = inf.get("avg_tok_s").and_then(|v| v.as_f64()) {
                self.metrics.gauge_set_labeled(
                    "clawtex_model_tokens_per_second", labels, (v * 100.0) as u64);
            }
            if let Some(v) = inf.get("queue_depth").and_then(|v| v.as_u64()) {
                self.metrics.gauge_set_labeled(
                    "clawtex_model_queue_depth", labels, v);
            }
        }
    }

    fn mark_node_offline(&self, node: &str) {
        self.metrics.gauge_set_labeled(
            "clawtex_node_status", &[("node", node)], 0);
    }
}
```

### 7.3 AlertEngine — 告警引擎

```rust
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time;
use tracing::{info, warn};

pub struct AlertEngine {
    metrics: Arc<MetricsRegistry>,
    rules: Vec<AlertRule>,
    cooldowns: HashMap<String, Instant>,  // rule_name → last_fired
    telegram_token: String,
    telegram_chat_id: String,
    client: reqwest::Client,
}

impl AlertEngine {
    pub fn new(
        metrics: Arc<MetricsRegistry>,
        telegram_token: String,
        telegram_chat_id: String,
    ) -> Self {
        Self {
            metrics,
            rules: Self::default_rules(),
            cooldowns: HashMap::new(),
            telegram_token,
            telegram_chat_id,
            client: reqwest::Client::new(),
        }
    }

    /// Run the alert evaluation loop (every 60s)
    pub async fn run(&mut self) {
        let mut interval = time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            self.evaluate_all().await;
        }
    }

    async fn evaluate_all(&mut self) {
        let now = Instant::now();
        for rule in &self.rules {
            // Check cooldown
            if let Some(last) = self.cooldowns.get(&rule.name) {
                let elapsed = now.duration_since(*last);
                if elapsed < Duration::from_secs(rule.cooldown_secs) {
                    continue;
                }
            }

            if self.check_condition(&rule.condition) {
                let message = self.format_alert(rule);
                self.send_telegram_alert(&message).await;
                self.cooldowns.insert(rule.name.clone(), now);
                info!("Alert fired: {} [{}]", rule.name, rule.severity.as_str());
            }
        }
    }

    fn check_condition(&self, condition: &AlertCondition) -> bool {
        match condition {
            AlertCondition::AllNodesOffline => {
                let online = self.metrics.gauge("clawtex_cluster_nodes_total{status=\"online\"}");
                online == 0
            }
            AlertCondition::NodeOffline { node, .. } => {
                let key = format!("clawtex_node_status{{node=\"{}\"}}", node);
                self.metrics.gauge(&key) == 0
            }
            AlertCondition::MetricAbove { metric, threshold } => {
                let val = self.metrics.gauge(metric) as f64;
                val > *threshold
            }
            AlertCondition::MetricBelow { metric, threshold } => {
                let val = self.metrics.gauge(metric) as f64;
                val < *threshold
            }
            _ => false,
        }
    }

    fn default_rules() -> Vec<AlertRule> {
        vec![
            // CRITICAL
            AlertRule {
                name: "all_inference_offline".into(),
                severity: AlertSeverity::Critical,
                condition: AlertCondition::AllNodesOffline,
                cooldown_secs: 60,
                message_template: "All inference nodes are offline!".into(),
            },
            // WARNING: disk > 90%
            AlertRule {
                name: "disk_high_any".into(),
                severity: AlertSeverity::Warning,
                condition: AlertCondition::MetricAbove {
                    metric: "clawtex_node_disk_percent_max".into(),
                    threshold: 90.0,
                },
                cooldown_secs: 1800,
                message_template: "Disk usage above 90% on one or more nodes".into(),
            },
            // ... more rules
        ]
    }

    async fn send_telegram_alert(&self, message: &str) {
        let url = format!(
            "https://api.telegram.org/bot{}/sendMessage",
            self.telegram_token
        );
        let _ = self.client.post(&url)
            .json(&serde_json::json!({
                "chat_id": self.telegram_chat_id,
                "text": message,
                "parse_mode": "HTML",
            }))
            .send()
            .await;
    }

    fn format_alert(&self, rule: &AlertRule) -> String {
        let icon = match rule.severity {
            AlertSeverity::Critical => "🔴 CRITICAL",
            AlertSeverity::Warning  => "🟡 WARNING",
            AlertSeverity::Info     => "🔵 INFO",
        };
        format!("{}: {}\n━━━━━━━━━━━━━━━━━━\n{}",
            icon, rule.name, rule.message_template)
    }
}
```

### 7.4 MetricsRollup — 歷史資料彙整

```rust
pub struct MetricsRollup {
    metrics: Arc<MetricsRegistry>,
    db_path: String,
}

impl MetricsRollup {
    pub fn new(metrics: Arc<MetricsRegistry>, db_path: &str) -> Result<Self> {
        let conn = rusqlite::Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS metrics_snapshots (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp   TEXT NOT NULL,
                metric_name TEXT NOT NULL,
                value       REAL NOT NULL,
                date_key    TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_snap_date ON metrics_snapshots(date_key);
            CREATE INDEX IF NOT EXISTS idx_snap_name ON metrics_snapshots(metric_name);"
        )?;
        Ok(Self { metrics, db_path: db_path.to_string() })
    }

    /// Run every 5 minutes: snapshot key metrics → SQLite
    pub async fn run(&self) {
        let mut interval = time::interval(Duration::from_secs(300));
        loop {
            interval.tick().await;
            if let Err(e) = self.snapshot().await {
                error!("MetricsRollup snapshot failed: {}", e);
            }
            if let Err(e) = self.cleanup().await {
                error!("MetricsRollup cleanup failed: {}", e);
            }
        }
    }

    async fn snapshot(&self) -> Result<()> {
        let now = Utc::now();
        let date_key = now.format("%Y-%m-%d").to_string();
        let ts = now.to_rfc3339();

        let conn = rusqlite::Connection::open(&self.db_path)?;

        // Snapshot key gauges
        let key_metrics = [
            "clawtex_cluster_nodes_total{status=\"online\"}",
            "clawtex_cluster_throughput_tok_s",
            "clawtex_cluster_active_requests",
            "clawtex_cluster_used_vram_gb",
        ];

        for metric in &key_metrics {
            let value = self.metrics.gauge(metric) as f64;
            conn.execute(
                "INSERT INTO metrics_snapshots (timestamp, metric_name, value, date_key)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![ts, metric, value, date_key],
            )?;
        }
        Ok(())
    }

    async fn cleanup(&self) -> Result<()> {
        let cutoff = (Utc::now() - chrono::Duration::days(90))
            .format("%Y-%m-%d").to_string();
        let conn = rusqlite::Connection::open(&self.db_path)?;
        conn.execute(
            "DELETE FROM metrics_snapshots WHERE date_key < ?1",
            [&cutoff],
        )?;
        // 也清理集中日誌
        conn.execute(
            "DELETE FROM cluster_logs WHERE date_key < ?1",
            [&(Utc::now() - chrono::Duration::days(14)).format("%Y-%m-%d").to_string()],
        )?;
        Ok(())
    }
}
```

### 7.5 Worker 端 /health Endpoint

每台 Worker 需要運行一個輕量 HTTP server（可用 axum 或 tiny_http）:

```rust
// worker_health.rs — 在每台 Worker 上運行

use axum::{Router, routing::get, Json};
use serde_json::{json, Value};
use sysinfo::{System, SystemExt, CpuExt, DiskExt};

async fn health_handler() -> Json<Value> {
    let mut sys = System::new_all();
    sys.refresh_all();

    let cpu_percent = sys.global_cpu_info().cpu_usage();
    let ram_used = sys.used_memory() / 1024 / 1024;  // MB
    let ram_total = sys.total_memory() / 1024 / 1024;

    // GPU info (ROCm for AMD / nvidia-smi for NVIDIA)
    let (gpu_percent, gpu_temp, vram_used, vram_total) = get_gpu_info();

    // Disk
    let (disk_used, disk_total) = sys.disks().iter()
        .map(|d| (d.total_space() - d.available_space(), d.total_space()))
        .fold((0, 0), |(au, at), (u, t)| (au + u, at + t));

    // Inference stats from Ollama API
    let inference = get_ollama_stats().await;

    Json(json!({
        "node": hostname::get().unwrap().to_string_lossy(),
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "system": {
            "cpu_percent": cpu_percent,
            "gpu_percent": gpu_percent,
            "gpu_vram_used_mb": vram_used,
            "gpu_vram_total_mb": vram_total,
            "ram_used_mb": ram_used,
            "ram_total_mb": ram_total,
            "cpu_temp_c": get_cpu_temp(&sys),
            "gpu_temp_c": gpu_temp,
            "disk_used_gb": disk_used / 1_073_741_824,
            "disk_total_gb": disk_total / 1_073_741_824,
            "uptime_secs": sys.uptime(),
            "load_avg_1m": sys.load_average().one,
        },
        "inference": inference,
    }))
}

fn get_gpu_info() -> (f64, f64, u64, u64) {
    // AMD ROCm: rocm-smi --showuse --showtemp --showmeminfo vram --json
    // NVIDIA: nvidia-smi --query-gpu=utilization.gpu,temperature.gpu,memory.used,memory.total --format=csv,nounits
    // 回傳 (percent, temp_c, vram_used_mb, vram_total_mb)
    // 省略實作細節...
    (0.0, 0.0, 0, 0)
}

async fn get_ollama_stats() -> Value {
    // GET http://localhost:11434/api/ps → running models
    // GET http://localhost:11434/api/tags → all available models
    let client = reqwest::Client::new();

    let running = client.get("http://localhost:11434/api/ps")
        .send().await.ok()
        .and_then(|r| futures::executor::block_on(r.json::<Value>()).ok());

    json!({
        "loaded_models": running.as_ref()
            .and_then(|v| v.get("models"))
            .cloned()
            .unwrap_or(json!([])),
        "total_requests": 0,    // 從 Ollama 或自行計數
        "active_requests": 0,
        "queue_depth": 0,
        "avg_tok_s": 0.0,
    })
}
```

**Worker crate 依賴** (`sysinfo` 等):
```toml
[dependencies]
sysinfo = "0.30"
axum = "0.7"
tokio = { version = "1", features = ["full"] }
serde_json = "1"
reqwest = { version = "0.12", features = ["json"] }
chrono = "0.4"
hostname = "0.3"
```

### 7.6 Prometheus Exporter 增強

現有的 `render_prometheus()` 已經能輸出 Prometheus text format。需要：

1. **正確解析 labeled keys** — 現在 key 裡含 `{label="val"}` 直接輸出即可
2. **聚合 TYPE 行** — 同 base name 的 metrics 只輸出一次 TYPE
3. **HELP 行** — 加入 `# HELP` 描述

```rust
/// Enhanced Prometheus renderer that groups TYPE/HELP correctly
pub fn render_prometheus_v2(&self) -> String {
    let mut output = String::with_capacity(8192);
    let mut seen_types: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Helper: extract base name from "metric_name{labels}" or "metric_name"
    fn base_name(key: &str) -> &str {
        key.split('{').next().unwrap_or(key)
    }

    // Counters
    let counters = self.counters.read().unwrap();
    let mut counter_keys: Vec<_> = counters.keys().collect();
    counter_keys.sort();
    for key in counter_keys {
        let base = base_name(key);
        if seen_types.insert(format!("counter:{}", base)) {
            output.push_str(&format!("# TYPE {} counter\n", base));
        }
        let value = counters[key].load(Ordering::Relaxed);
        output.push_str(&format!("{} {}\n", key, value));
    }
    drop(counters);

    // Gauges
    let gauges = self.gauges.read().unwrap();
    let mut gauge_keys: Vec<_> = gauges.keys().collect();
    gauge_keys.sort();
    for key in gauge_keys {
        let base = base_name(key);
        if seen_types.insert(format!("gauge:{}", base)) {
            output.push_str(&format!("# TYPE {} gauge\n", base));
        }
        let value = gauges[key].load(Ordering::Relaxed);
        output.push_str(&format!("{} {}\n", key, value));
    }
    drop(gauges);

    // Histograms (same as existing)
    let histograms = self.histograms.read().unwrap();
    let mut hist_keys: Vec<_> = histograms.keys().collect();
    hist_keys.sort();
    for key in hist_keys {
        let base = base_name(key);
        if seen_types.insert(format!("histogram:{}", base)) {
            output.push_str(&format!("# TYPE {} histogram\n", base));
        }
        let hist = &histograms[key];
        for (i, bound) in hist.buckets.iter().enumerate() {
            let count = hist.bucket_counts[i].load(Ordering::Relaxed);
            output.push_str(&format!("{}_bucket{{le=\"{}\"}} {}\n", key, bound, count));
        }
        output.push_str(&format!(
            "{}_bucket{{le=\"+Inf\"}} {}\n",
            key, hist.count.load(Ordering::Relaxed)
        ));
        let sum_us = hist.sum.load(Ordering::Relaxed);
        output.push_str(&format!("{}_sum {:.3}\n", key, sum_us as f64 / 1000.0));
        output.push_str(&format!("{}_count {}\n", key, hist.count.load(Ordering::Relaxed)));
    }

    output
}
```

### 7.7 Telegram Report 格式化函數

```rust
/// Format cluster status for Telegram
pub fn format_cluster_status(
    nodes: &[NodeHealth],
    metrics: &MetricsRegistry,
    cost_tracker: &CostTracker,
    revenue_tracker: &RevenueTracker,
) -> String {
    let online = nodes.iter().filter(|n| n.online).count();
    let total = nodes.len();

    let mut msg = String::new();

    // Header
    msg.push_str("📊 <b>Clawtex Cluster Status</b>\n");
    msg.push_str("━━━━━━━━━━━━━━━━━━━━━━\n");

    // Uptime & E-Stop
    let uptime = metrics.gauge("clawtex_hub_uptime_seconds");
    msg.push_str(&format!("⏱ Uptime: {}\n", format_duration(uptime)));

    // Nodes
    msg.push_str(&format!("\n📡 Nodes: {}/{} online\n", online, total));
    for node in nodes {
        let icon = if node.online { "✅" } else { "❌" };
        if node.online {
            msg.push_str(&format!(
                "  {} {} CPU:{}% GPU:{}% RAM:{}% 🌡{}°C\n",
                icon, node.name,
                node.cpu_percent as u32,
                node.gpu_percent as u32,
                node.ram_percent as u32,
                node.gpu_temp as u32,
            ));
        } else {
            msg.push_str(&format!(
                "  {} {} OFFLINE (last: {})\n",
                icon, node.name, node.last_seen
            ));
        }
    }

    // Throughput
    let throughput = metrics.gauge("clawtex_cluster_throughput_tok_s") as f64 / 100.0;
    let active = metrics.gauge("clawtex_cluster_active_requests");
    msg.push_str(&format!("\n⚡ Throughput: {:.0} tok/s\n", throughput));
    msg.push_str(&format!("📋 Active requests: {}\n", active));

    // Revenue (today)
    if let Ok(rev) = revenue_tracker.today_total() {
        if let Ok(cost) = cost_tracker.today_total() {
            msg.push_str(&format!(
                "\n💰 Today: ${:.2} revenue / ${:.4} cost\n",
                rev.total_usd, cost.total_cost_usd
            ));
        }
    }

    msg
}

pub fn format_hourly_heartbeat(
    nodes: &[NodeHealth],
    metrics: &MetricsRegistry,
    cost_tracker: &CostTracker,
    revenue_tracker: &RevenueTracker,
) -> String {
    let online = nodes.iter().filter(|n| n.online).count();
    let total = nodes.len();
    let hour = chrono::Utc::now().format("%H:%M");
    let throughput = metrics.gauge("clawtex_cluster_throughput_tok_s") as f64 / 100.0;
    let errors = metrics.counter("clawtex_provider_errors_total");
    let requests = metrics.counter("clawtex_provider_requests_total");

    let rev = revenue_tracker.today_total().map(|r| r.total_usd).unwrap_or(0.0);
    let cost = cost_tracker.today_total().map(|c| c.total_cost_usd).unwrap_or(0.0);

    format!(
        "💓 Heartbeat [{}]\nNodes: {}/{} 🟢  |  Reqs: {}  |  Errs: {}\nThroughput: {:.0} tok/s  |  Queue: {}\nRevenue: ${:.2}  |  Cost: ${:.4}",
        hour, online, total, requests, errors,
        throughput,
        metrics.gauge("clawtex_cluster_active_requests"),
        rev, cost,
    )
}

fn format_duration(secs: u64) -> String {
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    if d > 0 { format!("{}d {}h {}m", d, h, m) }
    else if h > 0 { format!("{}h {}m", h, m) }
    else { format!("{}m", m) }
}
```

### 7.8 Crate 依賴建議

```toml
# 已有 — 不需要新增
# tracing, tracing-subscriber, tracing-appender, rusqlite, reqwest, chrono, serde_json

# Worker 端新增 (如果 Worker 也用 Rust)
sysinfo = "0.30"     # 系統資訊 (CPU/RAM/Disk)
hostname = "0.3"     # 取得 hostname

# 不需要第三方 metrics crate
# 現有 MetricsRegistry 已經足夠，Prometheus text format 手動輸出
# metrics / metrics-exporter-prometheus 等 crate 功能重疊，且會增加編譯時間
```

**為什麼不用 `metrics` crate?**

1. 現有 `MetricsRegistry` 已是 lock-free atomic 實作，效能足夠
2. `metrics` crate 的 recorder 系統對小型專案過度複雜
3. 已有 `render_prometheus()` 輸出，加 label 支援只需 ~50 行
4. 減少依賴 = 更快編譯 + 更少安全更新

---

## 8. Grafana Dashboard JSON

以下是可直接匯入 Grafana 的 Dashboard JSON（假設 Prometheus 已設定 `job: clawtex`）：

### 8.1 Prometheus 設定 (`prometheus.yml`)

```yaml
global:
  scrape_interval: 15s
  evaluation_interval: 15s

scrape_configs:
  - job_name: 'clawtex'
    static_configs:
      - targets: ['localhost:7878']
    metrics_path: '/metrics'
    scrape_interval: 15s
    scrape_timeout: 10s
```

### 8.2 Grafana Dashboard JSON

```json
{
  "annotations": { "list": [] },
  "description": "Clawtex 8-Node AI Cluster Monitoring",
  "editable": true,
  "fiscalYearStartMonth": 0,
  "graphTooltip": 1,
  "id": null,
  "links": [],
  "liveNow": false,
  "panels": [
    {
      "title": "Cluster Overview",
      "type": "row",
      "gridPos": { "h": 1, "w": 24, "x": 0, "y": 0 },
      "collapsed": false
    },
    {
      "title": "Online Nodes",
      "type": "stat",
      "gridPos": { "h": 4, "w": 4, "x": 0, "y": 1 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "clawtex_cluster_nodes_total{status=\"online\"}",
          "legendFormat": "Online",
          "refId": "A"
        }
      ],
      "fieldConfig": {
        "defaults": {
          "thresholds": {
            "mode": "absolute",
            "steps": [
              { "color": "red", "value": null },
              { "color": "yellow", "value": 4 },
              { "color": "green", "value": 7 }
            ]
          },
          "unit": "none",
          "max": 8
        }
      },
      "options": {
        "reduceOptions": { "calcs": ["lastNotNull"] },
        "colorMode": "value",
        "graphMode": "none",
        "textMode": "value_and_name"
      }
    },
    {
      "title": "Cluster Throughput (tok/s)",
      "type": "timeseries",
      "gridPos": { "h": 8, "w": 12, "x": 4, "y": 1 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "clawtex_cluster_throughput_tok_s / 100",
          "legendFormat": "Total Throughput",
          "refId": "A"
        }
      ],
      "fieldConfig": {
        "defaults": {
          "unit": "tok/s",
          "custom": {
            "drawStyle": "line",
            "lineInterpolation": "smooth",
            "fillOpacity": 20,
            "gradientMode": "scheme"
          },
          "color": { "mode": "palette-classic" }
        }
      }
    },
    {
      "title": "Active Requests",
      "type": "stat",
      "gridPos": { "h": 4, "w": 4, "x": 16, "y": 1 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "clawtex_cluster_active_requests",
          "legendFormat": "Active",
          "refId": "A"
        }
      ],
      "fieldConfig": {
        "defaults": {
          "thresholds": {
            "mode": "absolute",
            "steps": [
              { "color": "green", "value": null },
              { "color": "yellow", "value": 20 },
              { "color": "red", "value": 50 }
            ]
          }
        }
      }
    },
    {
      "title": "E-Stop Status",
      "type": "stat",
      "gridPos": { "h": 4, "w": 4, "x": 20, "y": 1 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "clawtex_estop_active",
          "legendFormat": "E-Stop",
          "refId": "A"
        }
      ],
      "fieldConfig": {
        "defaults": {
          "mappings": [
            { "type": "value", "options": { "0": { "text": "INACTIVE", "color": "green" } } },
            { "type": "value", "options": { "1": { "text": "ACTIVE", "color": "red" } } }
          ]
        }
      }
    },
    {
      "title": "Node Resources",
      "type": "row",
      "gridPos": { "h": 1, "w": 24, "x": 0, "y": 9 },
      "collapsed": false
    },
    {
      "title": "CPU Usage by Node",
      "type": "timeseries",
      "gridPos": { "h": 8, "w": 8, "x": 0, "y": 10 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "clawtex_node_cpu_percent / 100",
          "legendFormat": "{{node}}",
          "refId": "A"
        }
      ],
      "fieldConfig": {
        "defaults": {
          "unit": "percent",
          "max": 100,
          "custom": {
            "drawStyle": "line",
            "lineInterpolation": "smooth",
            "fillOpacity": 10
          }
        }
      }
    },
    {
      "title": "GPU Usage by Node",
      "type": "timeseries",
      "gridPos": { "h": 8, "w": 8, "x": 8, "y": 10 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "clawtex_node_gpu_percent / 100",
          "legendFormat": "{{node}}",
          "refId": "A"
        }
      ],
      "fieldConfig": {
        "defaults": {
          "unit": "percent",
          "max": 100,
          "custom": {
            "drawStyle": "line",
            "lineInterpolation": "smooth",
            "fillOpacity": 10
          }
        }
      }
    },
    {
      "title": "GPU Temperature by Node",
      "type": "timeseries",
      "gridPos": { "h": 8, "w": 8, "x": 16, "y": 10 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "clawtex_node_gpu_temp_celsius / 10",
          "legendFormat": "{{node}}",
          "refId": "A"
        }
      ],
      "fieldConfig": {
        "defaults": {
          "unit": "celsius",
          "thresholds": {
            "mode": "absolute",
            "steps": [
              { "color": "green", "value": null },
              { "color": "yellow", "value": 75 },
              { "color": "red", "value": 85 }
            ]
          },
          "custom": {
            "drawStyle": "line",
            "lineInterpolation": "smooth",
            "thresholdsStyle": { "mode": "line+area" }
          }
        }
      }
    },
    {
      "title": "VRAM Usage by Node",
      "type": "bargauge",
      "gridPos": { "h": 6, "w": 12, "x": 0, "y": 18 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "clawtex_node_gpu_vram_used_bytes / 1073741824",
          "legendFormat": "{{node}} used",
          "refId": "A"
        }
      ],
      "fieldConfig": {
        "defaults": {
          "unit": "decgbytes",
          "thresholds": {
            "mode": "percentage",
            "steps": [
              { "color": "green", "value": null },
              { "color": "yellow", "value": 70 },
              { "color": "red", "value": 90 }
            ]
          }
        }
      }
    },
    {
      "title": "RAM Usage by Node",
      "type": "bargauge",
      "gridPos": { "h": 6, "w": 12, "x": 12, "y": 18 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "clawtex_node_ram_used_bytes / 1073741824",
          "legendFormat": "{{node}}",
          "refId": "A"
        }
      ],
      "fieldConfig": {
        "defaults": { "unit": "decgbytes" }
      }
    },
    {
      "title": "Provider Performance",
      "type": "row",
      "gridPos": { "h": 1, "w": 24, "x": 0, "y": 24 },
      "collapsed": false
    },
    {
      "title": "Provider Latency (p50 / p95)",
      "type": "timeseries",
      "gridPos": { "h": 8, "w": 12, "x": 0, "y": 25 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "histogram_quantile(0.5, rate(clawtex_provider_latency_ms_bucket[5m]))",
          "legendFormat": "p50 {{provider}}",
          "refId": "A"
        },
        {
          "expr": "histogram_quantile(0.95, rate(clawtex_provider_latency_ms_bucket[5m]))",
          "legendFormat": "p95 {{provider}}",
          "refId": "B"
        }
      ],
      "fieldConfig": {
        "defaults": {
          "unit": "ms",
          "custom": { "drawStyle": "line", "lineInterpolation": "smooth" }
        }
      }
    },
    {
      "title": "Provider Error Rate",
      "type": "timeseries",
      "gridPos": { "h": 8, "w": 12, "x": 12, "y": 25 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "rate(clawtex_provider_errors_total[5m]) / rate(clawtex_provider_requests_total[5m]) * 100",
          "legendFormat": "{{provider}} err%",
          "refId": "A"
        }
      ],
      "fieldConfig": {
        "defaults": {
          "unit": "percent",
          "custom": {
            "drawStyle": "line",
            "thresholdsStyle": { "mode": "line" }
          },
          "thresholds": {
            "mode": "absolute",
            "steps": [
              { "color": "green", "value": null },
              { "color": "yellow", "value": 5 },
              { "color": "red", "value": 10 }
            ]
          }
        }
      }
    },
    {
      "title": "Revenue & Cost",
      "type": "row",
      "gridPos": { "h": 1, "w": 24, "x": 0, "y": 33 },
      "collapsed": false
    },
    {
      "title": "Daily Revenue vs Cost",
      "type": "timeseries",
      "gridPos": { "h": 8, "w": 12, "x": 0, "y": 34 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "clawtex_revenue_daily_usd",
          "legendFormat": "Revenue",
          "refId": "A"
        },
        {
          "expr": "clawtex_cost_daily_usd",
          "legendFormat": "Cost",
          "refId": "B"
        }
      ],
      "fieldConfig": {
        "defaults": {
          "unit": "currencyUSD",
          "custom": {
            "drawStyle": "bars",
            "fillOpacity": 50
          }
        }
      }
    },
    {
      "title": "Profit Margin",
      "type": "gauge",
      "gridPos": { "h": 8, "w": 6, "x": 12, "y": 34 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "clawtex_profit_margin_percent",
          "legendFormat": "Margin",
          "refId": "A"
        }
      ],
      "fieldConfig": {
        "defaults": {
          "unit": "percent",
          "min": 0,
          "max": 100,
          "thresholds": {
            "mode": "absolute",
            "steps": [
              { "color": "red", "value": null },
              { "color": "yellow", "value": 50 },
              { "color": "green", "value": 80 }
            ]
          }
        }
      }
    },
    {
      "title": "Revenue by Route",
      "type": "piechart",
      "gridPos": { "h": 8, "w": 6, "x": 18, "y": 34 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "clawtex_revenue_total_usd",
          "legendFormat": "{{route}}",
          "refId": "A"
        }
      ]
    },
    {
      "title": "Hand Execution",
      "type": "row",
      "gridPos": { "h": 1, "w": 24, "x": 0, "y": 42 },
      "collapsed": false
    },
    {
      "title": "Hand Execution Duration",
      "type": "timeseries",
      "gridPos": { "h": 8, "w": 12, "x": 0, "y": 43 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "histogram_quantile(0.5, rate(clawtex_hand_duration_seconds_bucket[1h]))",
          "legendFormat": "p50 {{hand}}",
          "refId": "A"
        },
        {
          "expr": "histogram_quantile(0.95, rate(clawtex_hand_duration_seconds_bucket[1h]))",
          "legendFormat": "p95 {{hand}}",
          "refId": "B"
        }
      ],
      "fieldConfig": {
        "defaults": { "unit": "s" }
      }
    },
    {
      "title": "Hand Success Rate (24h)",
      "type": "bargauge",
      "gridPos": { "h": 8, "w": 12, "x": 12, "y": 43 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "sum by (hand)(increase(clawtex_hand_executions_total{status=\"success\"}[24h])) / sum by (hand)(increase(clawtex_hand_executions_total[24h])) * 100",
          "legendFormat": "{{hand}}",
          "refId": "A"
        }
      ],
      "fieldConfig": {
        "defaults": {
          "unit": "percent",
          "max": 100,
          "thresholds": {
            "mode": "absolute",
            "steps": [
              { "color": "red", "value": null },
              { "color": "yellow", "value": 70 },
              { "color": "green", "value": 90 }
            ]
          }
        }
      }
    },
    {
      "title": "SoT Engine",
      "type": "row",
      "gridPos": { "h": 1, "w": 24, "x": 0, "y": 51 },
      "collapsed": false
    },
    {
      "title": "SoT Parallel Degree",
      "type": "stat",
      "gridPos": { "h": 4, "w": 6, "x": 0, "y": 52 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "clawtex_sot_parallel_degree",
          "legendFormat": "Parallel",
          "refId": "A"
        }
      ]
    },
    {
      "title": "SoT Speedup Ratio",
      "type": "gauge",
      "gridPos": { "h": 4, "w": 6, "x": 6, "y": 52 },
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "targets": [
        {
          "expr": "clawtex_sot_speedup_ratio",
          "legendFormat": "Speedup",
          "refId": "A"
        }
      ],
      "fieldConfig": {
        "defaults": {
          "unit": "x",
          "min": 1,
          "max": 8,
          "thresholds": {
            "mode": "absolute",
            "steps": [
              { "color": "yellow", "value": null },
              { "color": "green", "value": 2 },
              { "color": "super-light-green", "value": 4 }
            ]
          }
        }
      }
    }
  ],
  "schemaVersion": 39,
  "tags": ["clawtex", "ai-cluster", "llm"],
  "templating": {
    "list": [
      {
        "name": "DS_PROMETHEUS",
        "type": "datasource",
        "query": "prometheus"
      }
    ]
  },
  "time": { "from": "now-3h", "to": "now" },
  "timepicker": {},
  "timezone": "browser",
  "title": "Clawtex AI Cluster",
  "uid": "clawtex-cluster-v1",
  "version": 1
}
```

### 8.3 Docker Compose (可選安裝)

```yaml
# docker-compose.monitoring.yml
# 放在 clawtex-core/ 目錄，需要時 docker compose -f docker-compose.monitoring.yml up -d

version: '3.8'

services:
  prometheus:
    image: prom/prometheus:v2.51.0
    container_name: clawtex-prometheus
    restart: unless-stopped
    ports:
      - "9090:9090"
    volumes:
      - ./monitoring/prometheus.yml:/etc/prometheus/prometheus.yml:ro
      - prometheus-data:/prometheus
    command:
      - '--config.file=/etc/prometheus/prometheus.yml'
      - '--storage.tsdb.retention.time=30d'
      - '--storage.tsdb.retention.size=5GB'
    network_mode: host  # 直接存取 localhost:7878

  grafana:
    image: grafana/grafana:10.4.0
    container_name: clawtex-grafana
    restart: unless-stopped
    ports:
      - "3000:3000"
    volumes:
      - grafana-data:/var/lib/grafana
      - ./monitoring/grafana/provisioning:/etc/grafana/provisioning:ro
      - ./monitoring/grafana/dashboards:/var/lib/grafana/dashboards:ro
    environment:
      - GF_SECURITY_ADMIN_PASSWORD=clawtex2026
      - GF_USERS_ALLOW_SIGN_UP=false
      - GF_INSTALL_PLUGINS=grafana-piechart-panel
    depends_on:
      - prometheus

volumes:
  prometheus-data:
  grafana-data:
```

---

## 9. 實作優先級與時間表

### Phase 1: 核心可觀測性 (1 天)

| 任務 | 預估時間 | 複雜度 |
|---|---|---|
| 擴展 MetricsRegistry 加入 label 支援 | 2h | 低 |
| 增強 render_prometheus_v2() | 1h | 低 |
| 在 agent_runtime / providers 中插入 metrics 記錄點 | 2h | 中 |
| 在 HandRunner 中插入 hand 執行 metrics | 1h | 中 |
| Telegram `/status` `/metrics` `/health` commands | 3h | 中 |
| 測試 | 2h | 低 |

### Phase 2: 節點採集 + 告警 (1 天)

| 任務 | 預估時間 | 複雜度 |
|---|---|---|
| Worker /health endpoint (sysinfo + GPU) | 3h | 中 |
| NodeCollector (Hub pull 30s) | 2h | 低 |
| AlertEngine + 預設規則 | 3h | 中 |
| alert_history SQLite table | 1h | 低 |
| Telegram 告警推送 + `/alerts` command | 1h | 低 |

### Phase 3: 歷史 + 日誌 (1 天)

| 任務 | 預估時間 | 複雜度 |
|---|---|---|
| MetricsRollup (5min → SQLite, 90d retain) | 2h | 低 |
| JSON 結構化日誌層 | 1h | 低 |
| 日誌 rotation (tracing-appender) | 1h | 低 |
| 集中日誌 (Worker POST → Hub) | 3h | 中 |
| 每小時 Heartbeat 自動推送 | 1h | 低 |

### Phase 4: Grafana (可選, 半天)

| 任務 | 預估時間 | 複雜度 |
|---|---|---|
| prometheus.yml + docker-compose | 30min | 低 |
| 匯入 Dashboard JSON | 30min | 低 |
| 驗證所有 panel 有資料 | 1h | 低 |
| Grafana alerts (mirror Telegram alerts) | 1h | 中 |

### 總計

- **核心功能 (Phase 1-3)**: 3 個工作天
- **Grafana 加裝 (Phase 4)**: 0.5 工作天（可隨時做）
- **總計**: 3-3.5 工作天

### 檔案結構 (新增)

```
clawtex-core/
├── src/
│   ├── metrics.rs              ← 修改：加入 label 支援 + render_v2
│   ├── monitoring/
│   │   ├── mod.rs              ← 新增：pub mod
│   │   ├── node_collector.rs   ← 新增：NodeCollector
│   │   ├── alert_engine.rs     ← 新增：AlertEngine
│   │   ├── metrics_rollup.rs   ← 新增：MetricsRollup
│   │   └── telegram_reports.rs ← 新增：format_*() 函數
│   ├── main.rs                 ← 修改：啟動 monitoring tasks
│   └── telegram.rs             ← 修改：新增 commands
├── monitoring/                  ← 新增：Docker 監控設定
│   ├── prometheus.yml
│   ├── docker-compose.monitoring.yml
│   └── grafana/
│       ├── provisioning/
│       │   └── datasources/
│       │       └── prometheus.yml
│       └── dashboards/
│           └── clawtex-cluster.json
└── worker/                      ← 新增 (可選獨立 crate)
    └── src/
        └── health.rs           ← Worker /health endpoint
```

---

## 附錄 A: 每日自動報告範例

```
📊 Daily Report — 2026-03-05
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

🖥 Cluster:
  Uptime: 99.7% (7.12/7.15 node-hours offline)
  Peak Throughput: 420 tok/s @ 15:32
  Avg Throughput: 285 tok/s
  Total Requests: 4,523
  Error Rate: 0.6%

🤖 Hands:
  Total Runs: 32
  Success Rate: 91%
  Fastest: content (avg 10m)
  Slowest: market_intel (avg 32m)

  ╔═════════════╦══════╦═══════╦═══════════╗
  ║ Hand        ║ Runs ║ Pass% ║ Avg Time  ║
  ╠═════════════╬══════╬═══════╬═══════════╣
  ║ content     ║    8 ║  100% ║ 10m 15s   ║
  ║ freelancer  ║    5 ║   80% ║ 25m 30s   ║
  ║ seo_content ║    6 ║  100% ║ 18m 45s   ║
  ║ outreach    ║    7 ║   86% ║ 15m 20s   ║
  ║ market_intel║    4 ║  100% ║ 32m 10s   ║
  ║ trading     ║    2 ║  100% ║ 22m 00s   ║
  ╚═════════════╩══════╩═══════╩═══════════╝

💰 Revenue:
  Today:      $245.00
  This Week:  $1,230.50
  This Month: $3,780.00

  Top Route: A:freelance_dev ($185.00)
  New Clients: 2

💵 Cost:
  API Costs:  $0.82
  Infra Est:  $2.80
  Net Profit: $241.38 (98.5% margin)

🔔 Alerts: 2 warnings, 0 critical
  [11:32] GPU temp high on m1-desktop (84°C)
  [15:45] Queue depth > 20 (peak: 24)

📝 Tomorrow's Schedule:
  09:00 freelancer (cron)
  10:00 outreach → lead chain (cron)
  11:00 seo_content (cron)
```

---

## 附錄 B: Worker /health 端口配置

| Node | Role | IP (Tailscale) | Port | GPU |
|---|---|---|---|---|
| z13 | Hub + Inference | 100.x.x.1 | 7878 | Radeon 8060S |
| m1 | Inference | 100.x.x.2 | 7879 | RTX 4090 (example) |
| m2 | Inference | 100.x.x.3 | 7879 | RX 7900 XTX |
| m3 | Inference (CPU/NPU) | 100.x.x.4 | 7879 | (NPU only) |
| m4 | Inference | 100.x.x.5 | 7879 | RTX 3080 |
| m5 | Inference | 100.x.x.6 | 7879 | RTX 4080 |
| m6 | Inference (light) | 100.x.x.7 | 7879 | iGPU |
| m7 | Backup/Overflow | 100.x.x.8 | 7879 | RTX 3060 |

Worker 使用 7879 port (避免與 Hub 7878 衝突)。Hub 自身也對自己的 7878/health 做 self-check。

---

## 附錄 C: agents.toml 監控設定區塊

```toml
[monitoring]
# NodeCollector 採集間隔 (秒)
poll_interval_secs = 30

# AlertEngine 評估間隔 (秒)
alert_interval_secs = 60

# MetricsRollup 快照間隔 (秒)
rollup_interval_secs = 300

# 歷史資料保留天數
metrics_retain_days = 90
logs_retain_days = 14

# 每小時 heartbeat 推送
heartbeat_enabled = true

# 每日報告推送時間 (UTC)
daily_report_hour = 15  # = 23:00 TW time

# 告警推送的 Telegram chat_id (預設用 bot 的管理員 chat)
alert_chat_id = ""

# GPU 溫度告警閾值
gpu_temp_warning_c = 80
gpu_temp_critical_c = 90

# 磁碟告警閾值
disk_warning_percent = 90
disk_critical_percent = 95

# 預算告警 (每日 USD)
daily_budget_warning_usd = 5.0
```
