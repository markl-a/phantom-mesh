# Phantom Mesh 8-Machine Cluster: Heartbeat Monitoring與容錯機制設計

> 日期: 2026-03-05
> 狀態: 設計完成，待實作
> 影響模組: `src/cluster.rs`, `src/providers/router.rs`, `src/providers/reliable.rs`, `src/estop.rs`, `src/telegram.rs`

---

## 目錄

1. [叢集拓撲與現狀分析](#1-叢集拓撲與現狀分析)
2. [心跳協議設計](#2-心跳協議設計)
3. [三層健康檢查](#3-三層健康檢查)
4. [節點狀態機](#4-節點狀態機)
5. [故障檢測與任務遷移](#5-故障檢測與任務遷移)
6. [Split Brain 處理](#6-split-brain-處理)
7. [降級模式策略](#7-降級模式策略)
8. [自動恢復機制](#8-自動恢復機制)
9. [監控告警系統](#9-監控告警系統)
10. [Rust 實作程式碼](#10-rust-實作程式碼)
11. [設定檔格式](#11-設定檔格式)
12. [測試策略](#12-測試策略)

---

## 1. 叢集拓撲與現狀分析

### 1.1 硬體拓撲

```
                        ┌─────────────────────┐
                        │   Internet / VPN     │
                        └──────────┬──────────┘
                                   │
              ┌────────────────────┼────────────────────┐
              │                    │                    │
    ┌─────────┴─────────┐  ┌─────┴──────┐  ┌─────────┴─────────┐
    │  Tailscale VPN    │  │   LAN      │  │  雲端 (未來)       │
    │  +-----------+    │  │  +-------+ │  │  +-------------+  │
    │  | M1 Mac    |    │  │  |Ayaneo | │  │  | GPU Cloud   |  │
    │  | (不穩定)   |    │  │  |       | │  │  | (按需)       |  │
    │  +-----------+    │  │  +-------+ │  │  +-------------+  │
    │                   │  │  +-------+ │  │  +-------------+  │
    │                   │  │  |Acer   | │  │  | GPU Cloud 2 |  │
    │                   │  │  |       | │  │  | (按需)       |  │
    │                   │  │  +-------+ │  │  +-------------+  │
    └───────────────────┘  └─────┬──────┘  └───────────────────┘
                                 │
                        ┌────────┴────────┐
                        │  Z13 (Hub)      │
                        │  AMD Ryzen AI   │
                        │  MAX+ 395       │
                        │  64GB LPDDR5X   │
                        │  NPU 50 TOPS    │
                        └─────────────────┘
```

### 1.2 節點清單 (8 機規劃)

| # | 名稱 | 類型 | 網路 | 穩定度 | Ollama | 角色 |
|---|------|------|------|--------|--------|------|
| 1 | z13 | Hub/Worker | localhost | 極高 | Yes | Hub + 推理 + NPU |
| 2 | m1-mac | Worker | Tailscale | 中等 | Yes | 推理 (Apple Silicon) |
| 3 | ayaneo | Worker | LAN | 高 | Yes | 推理 (輕量) |
| 4 | acer | Worker | LAN | 高 | Yes | 推理 (輕量) |
| 5 | gpu-cloud-1 | Worker | VPN/公網 | 高 | Yes | 重負載推理 |
| 6 | gpu-cloud-2 | Worker | VPN/公網 | 高 | Yes | 重負載推理 |
| 7 | npu-node | Worker | LAN | 高 | No | NPU 專用 (Lemonade) |
| 8 | backup-hub | Standby Hub | LAN/VPN | 高 | Yes | 備援 Hub |

### 1.3 現有基礎設施

已有的模組可以直接整合：

- **`cluster.rs`**: `ClusterRegistry` + `ClusterNode` — SQLite 節點註冊，但缺乏心跳和健康檢查
- **`estop.rs`**: `Heartbeat` struct — 已有 agent-level 心跳追蹤，需擴展到 node-level
- **`providers/reliable.rs`**: `ReliableProvider` + `CircuitBreaker` — 已有斷路器模式
- **`providers/router.rs`**: `ProviderRouter` — 已有 auto-routing 和 `is_alive()` 檢查
- **`task_queue.rs`**: `TaskQueue` — 已有任務排程，需增加 node affinity + migration

---

## 2. 心跳協議設計

### 2.1 心跳報告內容

```rust
/// 節點向 Hub 發送的心跳報告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHeartbeat {
    /// 節點唯一名稱
    pub node_name: String,
    /// 心跳序號 (單調遞增，用於亂序檢測)
    pub sequence: u64,
    /// 節點本地時間 (UTC)
    pub timestamp: DateTime<Utc>,

    // ── 資源使用率 ──
    pub cpu_usage_percent: f32,       // 0.0 ~ 100.0
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub gpu_usage_percent: Option<f32>,
    pub gpu_memory_used_mb: Option<u64>,
    pub gpu_memory_total_mb: Option<u64>,

    // ── Ollama 狀態 ──
    pub ollama_alive: bool,
    pub loaded_models: Vec<String>,   // 當前已載入到記憶體的模型
    pub available_models: Vec<String>,// 所有可用模型 (/api/tags)
    pub pending_requests: u32,        // 排隊中的推理請求

    // ── 推理性能 ──
    pub inference_speed: Option<InferenceMetrics>,

    // ── 網路 ──
    pub network_type: NetworkType,    // LAN | Tailscale | Cloud
    pub round_trip_ms: Option<u32>,   // 到 Hub 的 RTT (節點自測)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceMetrics {
    pub tokens_per_second: f32,       // 最近一次推理的 t/s
    pub avg_tokens_per_second: f32,   // 近 5 分鐘平均
    pub last_inference_at: DateTime<Utc>,
    pub model_name: String,           // 產生此指標的模型
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NetworkType {
    Local,      // localhost (Z13)
    Lan,        // 區域網路 (Ayaneo, Acer)
    Tailscale,  // VPN (M1 Mac)
    Cloud,      // 公網/雲端
}
```

### 2.2 心跳間隔建議

| 網路類型 | 心跳間隔 | 理由 |
|----------|----------|------|
| Local (Z13) | 5 秒 | 本地無網路開銷，快速檢測 |
| LAN (Ayaneo/Acer) | 10 秒 | 區域網路穩定，適度頻率 |
| Tailscale (M1 Mac) | 15 秒 | VPN 可能抖動，避免誤判 |
| Cloud | 15 秒 | 網路延遲較高 |

**為什麼不用更短的間隔？**
- 5 秒以下：Ollama `/api/tags` 呼叫有 I/O 開銷，頻繁檢查會拖慢推理
- 15 秒以上：故障檢測延遲過高，任務遷移不夠及時

### 2.3 心跳失敗判定

```
失敗次數閾值 = ceil(30秒 / 心跳間隔)

LAN 節點：3 次失敗 (30秒) → Suspect
           6 次失敗 (60秒) → Offline
Tailscale：3 次失敗 (45秒) → Suspect
           5 次失敗 (75秒) → Offline
Cloud：    3 次失敗 (45秒) → Suspect
           5 次失敗 (75秒) → Offline
```

**為什麼分兩階段？**
- Suspect：暫停派發新任務，但不遷移現有任務 (可能只是網路抖動)
- Offline：開始遷移任務，觸發恢復流程

---

## 3. 三層健康檢查

### 3.1 檢查層級定義

```
┌──────────────────────────────────────────────────────────┐
│                    Level 3: 推理測試                      │
│  POST /api/generate {"model":"..","prompt":"ping"}       │
│  預期: 在 30 秒內回傳任何非空內容                          │
│  頻率: 每 5 分鐘 (或 Level 2 恢復後立即執行)              │
│  用途: 驗證模型能實際工作、GPU 未卡死                      │
├──────────────────────────────────────────────────────────┤
│                    Level 2: API 服務檢查                   │
│  GET /api/tags                                           │
│  預期: HTTP 200 + 有效 JSON (models 列表)                 │
│  頻率: 隨心跳 (每 10~15 秒)                               │
│  用途: 確認 Ollama 進程存活且能回應 API                    │
├──────────────────────────────────────────────────────────┤
│                    Level 1: 網路連通                       │
│  TCP connect 到 Ollama port (11434) 或 ICMP ping         │
│  預期: 在 3 秒內建立連線                                   │
│  頻率: 隨心跳 (每 10~15 秒)                               │
│  用途: 基本網路可達性                                      │
└──────────────────────────────────────────────────────────┘
```

### 3.2 檢查頻率矩陣

| 檢查層級 | 正常狀態 | Suspect 狀態 | Recovering 狀態 |
|----------|----------|-------------|-----------------|
| L1 Ping | 每心跳 | 每 5 秒 | 每 5 秒 |
| L2 API | 每心跳 | 每 10 秒 | 每 10 秒 |
| L3 推理 | 每 5 分鐘 | 跳過 | 恢復後立即一次 |

### 3.3 檢查結果結構

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckResult {
    pub node_name: String,
    pub timestamp: DateTime<Utc>,
    pub level1_ping: CheckStatus,
    pub level2_api: CheckStatus,
    pub level3_inference: CheckStatus,
    pub overall: HealthLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CheckStatus {
    Pass { latency_ms: u32 },
    Fail { error: String },
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum HealthLevel {
    Healthy,      // L1+L2+L3 全通過
    Degraded,     // L1+L2 通過，L3 失敗 (Ollama 活著但推理異常)
    Unhealthy,    // L1 通過，L2 失敗 (機器活著但 Ollama 掛了)
    Unreachable,  // L1 失敗 (網路不通)
}
```

---

## 4. 節點狀態機

### 4.1 狀態圖 (ASCII)

```
                          ┌─────────────┐
                 啟動/註冊 │             │
              ┌──────────>│   Online     │<──────────────────────────┐
              │           │  (正常工作)   │                           │
              │           └──────┬──────┘                           │
              │                  │                                  │
              │         連續 N 次心跳失敗                            │
              │         或 L2 檢查失敗                              │
              │                  │                                  │
              │                  v                                  │
              │           ┌─────────────┐                           │
              │           │             │    心跳恢復               │
              │           │   Suspect   │────(< suspect_timeout)───┘
              │           │  (疑似離線)  │
              │           └──────┬──────┘
              │                  │
              │         超過 suspect_timeout
              │         (LAN: 30s, VPN: 45s)
              │                  │
              │                  v
              │           ┌─────────────┐
              │           │             │
              │           │   Offline   │───────────────┐
              │           │  (確認離線)  │               │
              │           └──────┬──────┘               │
              │                  │                      │
              │         自動恢復啟動                     │ 恢復失敗
              │         (SSH restart /                  │ (超過 max_recovery_attempts)
              │          Wake-on-LAN)                   │
              │                  │                      v
              │                  v               ┌─────────────┐
              │           ┌─────────────┐        │             │
              │           │             │        │    Dead     │
              └───────────│ Recovering  │        │  (需人工)    │
              L3 推理通過  │  (恢復中)    │        │             │
                          └──────┬──────┘        └─────────────┘
                                 │
                          恢復失敗 → 回到 Offline (重試計數+1)
```

### 4.2 狀態轉換規則

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeState {
    /// 節點正常運行，接受任務
    Online,
    /// 疑似離線：停止派發新任務，不遷移現有任務
    Suspect,
    /// 確認離線：遷移任務，啟動恢復流程
    Offline,
    /// 恢復中：正在嘗試重啟服務
    Recovering,
    /// 死亡：多次恢復失敗，需要人工介入
    Dead,
}

/// 狀態轉換事件
#[derive(Debug, Clone)]
pub enum NodeEvent {
    HeartbeatReceived(NodeHeartbeat),
    HeartbeatTimeout,
    SuspectTimeout,         // Suspect 狀態超時
    RecoveryStarted,
    RecoverySucceeded,
    RecoveryFailed,
    MaxRecoveryExceeded,    // 超過最大恢復嘗試次數
    ManualReset,            // 人工重置
}
```

### 4.3 狀態轉換表

| 當前狀態 | 事件 | 新狀態 | 動作 |
|---------|------|--------|------|
| Online | HeartbeatTimeout | Suspect | 暫停新任務派發，加速檢查頻率 |
| Online | HeartbeatReceived | Online | 更新指標 |
| Suspect | HeartbeatReceived | Online | 恢復正常派發 |
| Suspect | SuspectTimeout | Offline | 遷移任務，觸發恢復，發送告警 |
| Offline | RecoveryStarted | Recovering | 記錄恢復開始時間 |
| Offline | HeartbeatReceived | Online | 直接恢復 (可能是網路抖動) |
| Recovering | RecoverySucceeded + L3 Pass | Online | 重新加入叢集，發送恢復通知 |
| Recovering | RecoveryFailed | Offline | 重試計數+1，等待下次恢復 |
| Recovering | MaxRecoveryExceeded | Dead | 發送緊急告警，需人工介入 |
| Dead | ManualReset | Offline | 重置重試計數，重新嘗試恢復 |

---

## 5. 故障檢測與任務遷移

### 5.1 任務遷移流程

```
節點 A (Offline)                Hub (Z13)                    節點 B (Online)
     │                            │                             │
     │  ---- 心跳超時 ---->       │                             │
     │                            │── 1. 標記 A 為 Suspect      │
     │                            │                             │
     │  ---- suspect_timeout -->  │                             │
     │                            │── 2. 標記 A 為 Offline       │
     │                            │── 3. 查詢 A 的 pending 任務  │
     │                            │── 4. 選擇目標節點 B          │
     │                            │                             │
     │                            │── 5. 遷移任務到 B ────────> │
     │                            │                             │── 6. B 開始執行
     │                            │── 7. 更新任務 node_affinity  │
     │                            │── 8. 啟動 A 的恢復流程       │
     │                            │                             │
```

### 5.2 任務遷移策略

```rust
/// 任務遷移決策器
pub struct TaskMigrator;

impl TaskMigrator {
    /// 選擇最佳目標節點
    pub fn select_target(
        &self,
        task: &MigratableTask,
        online_nodes: &[NodeInfo],
    ) -> Option<String> {
        // 優先級:
        // 1. 已載入相同模型的節點 (避免模型載入延遲)
        // 2. 負載最低的節點 (pending_requests 最少)
        // 3. 推理速度最快的節點 (avg_tokens_per_second)
        // 4. 網路最近的節點 (LAN > Tailscale > Cloud)

        let mut candidates: Vec<(&NodeInfo, u32)> = online_nodes
            .iter()
            .map(|node| {
                let mut score = 0u32;

                // 已載入模型 +100
                if node.loaded_models.contains(&task.required_model) {
                    score += 100;
                }
                // 可用模型 +50 (需要載入但至少有)
                else if node.available_models.contains(&task.required_model) {
                    score += 50;
                }

                // 低負載 +0~40 (pending < 2 得 40, < 5 得 20, 否則 0)
                match node.pending_requests {
                    0..=1 => score += 40,
                    2..=4 => score += 20,
                    _ => {}
                }

                // 高推理速度 +0~30
                if let Some(ref metrics) = node.inference_speed {
                    if metrics.avg_tokens_per_second > 30.0 { score += 30; }
                    else if metrics.avg_tokens_per_second > 15.0 { score += 15; }
                }

                // 網路優先 +0~20
                match node.network_type {
                    NetworkType::Local => score += 20,
                    NetworkType::Lan => score += 15,
                    NetworkType::Tailscale => score += 5,
                    NetworkType::Cloud => score += 10,
                }

                (node, score)
            })
            .collect();

        candidates.sort_by(|a, b| b.1.cmp(&a.1));
        candidates.first().map(|(node, _)| node.name.clone())
    }
}
```

### 5.3 節點恢復與重新加入

```
節點 A (Recovering)             Hub (Z13)
     │                            │
     │  ---- 心跳恢復 ---->       │
     │                            │── 1. 狀態: Recovering
     │                            │── 2. 執行 L2 健康檢查
     │  <---- L2 Pass -----      │
     │                            │── 3. 執行 L3 推理測試
     │  <---- L3 Pass -----      │
     │                            │── 4. 狀態: Online
     │                            │── 5. 更新 ProviderRouter
     │                            │── 6. 節點可接受新任務
     │                            │── 7. 發送恢復通知
```

**重新加入不會回遷任務**：已經遷移走的任務不會遷回來，避免不必要的中斷。恢復後的節點只接受新任務。

---

## 6. Split Brain 處理

### 6.1 場景分析

最危險的 Split Brain 場景：Z13 和 M1 Mac 之間的 Tailscale VPN 斷裂。

```
         Tailscale 斷裂
              ╳
    ┌─────────┤
    │         │
 ┌──┴──┐  ┌──┴──┐
 │ M1  │  │ Z13 │──── Ayaneo (LAN)
 │ Mac │  │     │──── Acer   (LAN)
 └─────┘  └─────┘

分區 A: M1 Mac (孤立)
分區 B: Z13 + Ayaneo + Acer (多數)
```

### 6.2 解決方案：Hub 為唯一真相源 (Single Source of Truth)

**設計原則**: 採用非對稱架構，Z13 作為 Hub 持有唯一權威狀態。不採用分散式共識（Raft/Paxos），因為：

1. 叢集規模小 (8 台)，共識算法 overhead 不划算
2. Z13 始終在線（筆電合蓋也不關機），可作為固定 Hub
3. Worker 不做決策，只報告心跳和執行任務

```rust
/// Split Brain 檢測與處理
pub struct SplitBrainDetector {
    /// 已知的網路分區群組
    partitions: HashMap<String, Vec<String>>,  // partition_id -> node_names
    /// 每個分區的最後確認時間
    partition_last_seen: HashMap<String, DateTime<Utc>>,
}

impl SplitBrainDetector {
    /// 檢測分區: 如果一組節點同時消失，可能是網路分區
    pub fn detect_partition(&mut self, offline_nodes: &[String], all_nodes: &[NodeInfo]) -> Option<Partition> {
        // 同一網路類型的節點同時離線 = 可能分區
        let tailscale_nodes: Vec<_> = offline_nodes.iter()
            .filter(|n| {
                all_nodes.iter().any(|info| info.name == **n && info.network_type == NetworkType::Tailscale)
            })
            .cloned()
            .collect();

        if tailscale_nodes.len() > 0 {
            // 所有 Tailscale 節點同時離線 → 判定為 VPN 分區
            return Some(Partition {
                partition_type: PartitionType::VpnDisconnect,
                affected_nodes: tailscale_nodes,
                recommendation: PartitionAction::WaitAndRetry {
                    retry_interval: Duration::from_secs(60),
                    max_wait: Duration::from_secs(600), // 10 分鐘
                },
            });
        }

        None
    }
}

#[derive(Debug)]
pub struct Partition {
    pub partition_type: PartitionType,
    pub affected_nodes: Vec<String>,
    pub recommendation: PartitionAction,
}

#[derive(Debug)]
pub enum PartitionType {
    VpnDisconnect,     // Tailscale 斷裂
    LanSegment,        // 區域網路分段
    SingleNode,        // 單節點失聯
}

#[derive(Debug)]
pub enum PartitionAction {
    /// 等待並重試 (VPN 通常會自動恢復)
    WaitAndRetry { retry_interval: Duration, max_wait: Duration },
    /// 立即遷移 (LAN 節點不太會自動恢復)
    MigrateImmediately,
    /// 忽略 (單節點問題)
    HandleAsNodeFailure,
}
```

### 6.3 Hub 失敗處理

如果 Z13 (Hub) 本身離線，整個叢集將停擺。為此提供以下保障：

```
                      ┌──────────────┐
                      │  Z13 (Hub)   │
                      │  Priority: 0 │
                      └──────┬───────┘
                             │
                      自動 failover
                      (Z13 心跳消失)
                             │
                      ┌──────┴───────┐
                      │ backup-hub   │
                      │ Priority: 1  │
                      └──────────────┘
```

**Backup Hub 提升流程**:
1. backup-hub 持續從 Z13 同步叢集狀態 (SQLite replication)
2. 如果 backup-hub 連續 60 秒無法連到 Z13，啟動 Hub 提升
3. backup-hub 成為新 Hub，通知所有 Worker 更新 Hub 地址
4. Z13 恢復後，backup-hub 讓出 Hub 角色 (fencing)

```rust
/// Hub 選舉 (簡化版，非 Raft)
pub struct HubElection {
    pub hub_priority: Vec<(String, u32)>,  // (node_name, priority) 0=最高
    pub current_hub: String,
    pub hub_last_heartbeat: DateTime<Utc>,
    pub failover_timeout: Duration,        // 預設 60 秒
}

impl HubElection {
    pub fn should_promote(&self) -> Option<String> {
        let now = Utc::now();
        let elapsed = now.signed_duration_since(self.hub_last_heartbeat);
        if elapsed > chrono::Duration::from_std(self.failover_timeout).unwrap() {
            // 找到下一個優先級的節點
            let current_priority = self.hub_priority.iter()
                .find(|(name, _)| name == &self.current_hub)
                .map(|(_, p)| *p)
                .unwrap_or(0);
            self.hub_priority.iter()
                .filter(|(name, p)| name != &self.current_hub && *p > current_priority)
                .min_by_key(|(_, p)| *p)
                .map(|(name, _)| name.clone())
        } else {
            None
        }
    }
}
```

---

## 7. 降級模式策略

### 7.1 降級等級定義

```
┌─────────────────────────────────────────────────────────────┐
│ Level 0: FULL CAPACITY                                      │
│ 全部 8 台在線                                                │
│ 策略: 最大並行度，SoT 跨 4+ 節點                             │
│ 預期吞吐: ~200 tokens/s (aggregate)                          │
├─────────────────────────────────────────────────────────────┤
│ Level 1: MINOR DEGRADATION (1 台離線)                        │
│ 7/8 台在線                                                   │
│ 策略: 重新分配負載到剩餘節點                                  │
│ 動作:                                                        │
│   - 如果離線的是小節點 → 幾乎無影響                           │
│   - 如果離線的是 GPU Cloud → 降低重負載任務的並行度             │
│   - Telegram 通知 (info 級別)                                │
├─────────────────────────────────────────────────────────────┤
│ Level 2: MODERATE DEGRADATION (2-3 台離線)                   │
│ 5-6/8 台在線                                                 │
│ 策略: 降低並行度，優先處理高優先級任務                         │
│ 動作:                                                        │
│   - SoT 並行段數從 N 降到 max(2, online_count-1)             │
│   - 暫停 cron 中的低優先級任務 (seo_content, market_intel)    │
│   - 保留 freelancer, outreach 等收入相關任務                  │
│   - Telegram 通知 (warning 級別)                             │
├─────────────────────────────────────────────────────────────┤
│ Level 3: SEVERE DEGRADATION (4+ 台離線)                      │
│ 1-4/8 台在線                                                 │
│ 策略: 僅保留核心功能                                          │
│   - 停止所有 cron 任務                                        │
│   - 僅回應即時 Telegram 指令                                  │
│   - SoT 退回為串行模式                                        │
│   - Telegram 通知 (critical 級別)                            │
├─────────────────────────────────────────────────────────────┤
│ Level 4: SINGLE NODE (只剩 Z13)                              │
│ 1/8 台在線                                                   │
│ 策略: 完全退回單機模式                                        │
│   - 使用 Z13 本地 Ollama + NPU (Lemonade)                    │
│   - 所有請求串行處理                                          │
│   - 模型限制: 僅載入 1 個 <=8B 模型                           │
│   - Telegram 通知 (emergency 級別)                           │
├─────────────────────────────────────────────────────────────┤
│ Level 5: HUB DOWN (Z13 離線)                                 │
│ 0/8 由 Z13 控制                                              │
│ 策略: backup-hub 接管 或 完全停擺                             │
│   - 若有 backup-hub → 自動提升                                │
│   - 若無 → 所有 Worker 進入 idle 等待                         │
│   - Worker 本地持久化未完成任務                                │
│   - Z13 恢復後自動恢復                                        │
│   - Telegram 通知: 無法發送 (Hub 掛了)，改發 email 告警       │
└─────────────────────────────────────────────────────────────┘
```

### 7.2 降級模式實作

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DegradationLevel {
    Full,             // Level 0: 全容量
    Minor,            // Level 1: 1 台離線
    Moderate,         // Level 2: 2-3 台離線
    Severe,           // Level 3: 4+ 台離線
    SingleNode,       // Level 4: 只剩 Z13
    HubDown,          // Level 5: Z13 離線
}

impl DegradationLevel {
    pub fn from_cluster_state(total: usize, online: usize, hub_online: bool) -> Self {
        if !hub_online {
            return DegradationLevel::HubDown;
        }
        let offline = total.saturating_sub(online);
        match offline {
            0 => DegradationLevel::Full,
            1 => DegradationLevel::Minor,
            2..=3 => DegradationLevel::Moderate,
            _ if online <= 1 => DegradationLevel::SingleNode,
            _ => DegradationLevel::Severe,
        }
    }

    /// 最大 SoT 並行段數
    pub fn max_sot_parallelism(&self) -> usize {
        match self {
            Self::Full => 8,
            Self::Minor => 6,
            Self::Moderate => 3,
            Self::Severe => 2,
            Self::SingleNode => 1,
            Self::HubDown => 0,
        }
    }

    /// 允許的 cron 任務優先級閾值 (0=最高)
    pub fn min_cron_priority(&self) -> u32 {
        match self {
            Self::Full => 100,      // 所有任務
            Self::Minor => 80,      // 大部分任務
            Self::Moderate => 50,   // 只有中高優先級
            Self::Severe => 0,      // 停止所有 cron
            Self::SingleNode => 0,
            Self::HubDown => 0,
        }
    }
}
```

---

## 8. 自動恢復機制

### 8.1 恢復動作優先級

```
┌───────────────────────────────────────────────────────────┐
│ Phase 1: Ollama 服務重啟 (最輕量)                         │
│ 方式: SSH 到節點執行 `systemctl restart ollama`            │
│       或 `ollama serve &`                                 │
│ 超時: 30 秒                                               │
│ 適用: L2 失敗 (Ollama 掛了) 但 L1 通過 (機器還活著)       │
├───────────────────────────────────────────────────────────┤
│ Phase 2: 模型重新載入                                     │
│ 方式: POST /api/pull {"name":"model"} 或                  │
│       POST /api/generate {"model":"model","prompt":"hi"}  │
│ 超時: 120 秒 (大模型載入慢)                                │
│ 適用: L2 通過但 L3 失敗 (模型未載入或損壞)                 │
├───────────────────────────────────────────────────────────┤
│ Phase 3: 遠端重啟機器                                     │
│ 方式:                                                     │
│   LAN: Wake-on-LAN (ethool / wakeonlan 指令)              │
│   Tailscale: SSH -> `sudo reboot`                         │
│   Cloud: Provider API (restart instance)                   │
│ 超時: 180 秒                                              │
│ 適用: L1 失敗 (機器完全無回應)                             │
├───────────────────────────────────────────────────────────┤
│ Phase 4: 人工介入                                         │
│ 方式: 發送 Telegram 告警，等待人工處理                     │
│ 超時: 無限 (直到手動 /reset node_name)                     │
│ 適用: Phase 1-3 全部失敗                                   │
└───────────────────────────────────────────────────────────┘
```

### 8.2 恢復動作實作

```rust
/// 自動恢復執行器
pub struct RecoveryExecutor {
    ssh_configs: HashMap<String, SshConfig>,
    wol_configs: HashMap<String, WolConfig>,
    max_recovery_attempts: u32,  // 預設 3
}

#[derive(Debug, Clone)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,            // 預設 22
    pub user: String,
    pub key_path: String,     // SSH 私鑰路徑
    pub ollama_restart_cmd: String,  // 例: "systemctl restart ollama"
    pub reboot_cmd: String,   // 例: "sudo reboot"
}

#[derive(Debug, Clone)]
pub struct WolConfig {
    pub mac_address: String,  // 例: "AA:BB:CC:DD:EE:FF"
    pub broadcast_addr: String,  // 例: "192.168.1.255"
}

impl RecoveryExecutor {
    /// 執行分階段恢復
    pub async fn recover(&self, node: &NodeInfo, health: &HealthCheckResult) -> RecoveryResult {
        let node_name = &node.name;

        // Phase 1: 重啟 Ollama (如果機器可達)
        if health.level1_ping != CheckStatus::Fail { .. } {
            if let Some(ssh) = self.ssh_configs.get(node_name) {
                match self.restart_ollama(ssh).await {
                    Ok(_) => {
                        // 等待 Ollama 啟動
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        if self.check_l2(node).await {
                            return RecoveryResult::Success("Ollama restarted via SSH".into());
                        }
                    }
                    Err(e) => tracing::warn!("SSH Ollama restart failed for {}: {}", node_name, e),
                }
            }
        }

        // Phase 2: 重啟機器
        if let Some(wol) = self.wol_configs.get(node_name) {
            match self.wake_on_lan(wol).await {
                Ok(_) => {
                    // WOL 後等待機器開機
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    if self.check_l1(node).await {
                        // 再等 Ollama 啟動
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        if self.check_l2(node).await {
                            return RecoveryResult::Success("Machine woke up via WOL".into());
                        }
                    }
                }
                Err(e) => tracing::warn!("WOL failed for {}: {}", node_name, e),
            }
        }

        // 也嘗試 SSH reboot
        if let Some(ssh) = self.ssh_configs.get(node_name) {
            let _ = self.ssh_reboot(ssh).await;
            tokio::time::sleep(Duration::from_secs(120)).await;
            if self.check_l2(node).await {
                return RecoveryResult::Success("Machine rebooted via SSH".into());
            }
        }

        RecoveryResult::Failed("All recovery phases exhausted".into())
    }

    async fn restart_ollama(&self, ssh: &SshConfig) -> Result<()> {
        // 使用 tokio::process::Command 執行 SSH
        let output = tokio::process::Command::new("ssh")
            .args([
                "-i", &ssh.key_path,
                "-o", "StrictHostKeyChecking=no",
                "-o", "ConnectTimeout=10",
                &format!("{}@{}", ssh.user, ssh.host),
                &ssh.ollama_restart_cmd,
            ])
            .output()
            .await?;

        if output.status.success() {
            Ok(())
        } else {
            Err(anyhow::anyhow!("SSH command failed: {}",
                String::from_utf8_lossy(&output.stderr)))
        }
    }

    async fn wake_on_lan(&self, wol: &WolConfig) -> Result<()> {
        // 建構 magic packet (6 bytes of 0xFF + 16 repetitions of MAC address)
        let mac_bytes: Vec<u8> = wol.mac_address
            .split(':')
            .filter_map(|s| u8::from_str_radix(s, 16).ok())
            .collect();

        if mac_bytes.len() != 6 {
            return Err(anyhow::anyhow!("Invalid MAC address"));
        }

        let mut magic = vec![0xFFu8; 6];
        for _ in 0..16 {
            magic.extend_from_slice(&mac_bytes);
        }

        let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await?;
        socket.set_broadcast(true)?;
        socket.send_to(&magic, format!("{}:9", wol.broadcast_addr)).await?;
        Ok(())
    }
}

#[derive(Debug)]
pub enum RecoveryResult {
    Success(String),
    Failed(String),
}
```

---

## 9. 監控告警系統

### 9.1 告警等級

| 等級 | 觸發條件 | Telegram 格式 | 頻率限制 |
|------|---------|---------------|---------|
| INFO | 節點恢復上線 | `[INFO] Node "m1-mac" is back online` | 每事件一次 |
| WARNING | 單節點 Suspect/Offline | `[WARN] Node "ayaneo" went offline` | 每事件一次 |
| CRITICAL | 2+ 節點同時離線 | `[CRIT] 2 nodes offline: ayaneo, acer` | 每 5 分鐘提醒 |
| EMERGENCY | Hub 離線或全叢集停擺 | `[EMERG] Hub Z13 unreachable!` | 每 1 分鐘提醒 |
| ANOMALY | 推理速度驟降 >50% | `[ANOMALY] m1-mac: 25 t/s -> 8 t/s` | 每 10 分鐘一次 |

### 9.2 每日狀態報告

```
📊 Phantom Mesh Cluster Daily Report (2026-03-05)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Nodes: 7/8 online (87.5%)

| Node       | Status  | Uptime | Avg t/s | Tasks |
|------------|---------|--------|---------|-------|
| z13        | Online  | 100%   | 42.3    | 156   |
| m1-mac     | Online  | 94.2%  | 28.7    | 89    |
| ayaneo     | Online  | 99.8%  | 15.2    | 45    |
| acer       | Online  | 99.9%  | 12.8    | 38    |
| gpu-cloud1 | Online  | 100%   | 65.1    | 203   |
| gpu-cloud2 | Offline | 0%     | -       | 0     |
| npu-node   | Online  | 100%   | 18.5    | 67    |
| backup-hub | Standby | 100%   | -       | 0     |

Incidents today: 2
  09:14 gpu-cloud2 went offline (provider API error)
  09:18 Tasks migrated: 3 -> gpu-cloud1

Costs: $4.23 (compute) + $0.87 (API calls)
Revenue: $127.50 (5 tasks completed)
```

### 9.3 異常檢測

```rust
/// 推理速度異常檢測器
pub struct AnomalyDetector {
    /// 每個節點的歷史推理速度 (滑動窗口)
    history: HashMap<String, VecDeque<f32>>,
    /// 窗口大小 (幾筆資料)
    window_size: usize,
    /// 異常閾值 (偏離均值的倍數)
    threshold_stddev: f32,
}

impl AnomalyDetector {
    pub fn new() -> Self {
        Self {
            history: HashMap::new(),
            window_size: 30,       // 30 筆心跳資料 (~5 分鐘)
            threshold_stddev: 2.0, // 2 個標準差
        }
    }

    /// 記錄新的推理速度，回傳是否異常
    pub fn record(&mut self, node: &str, tokens_per_second: f32) -> Option<Anomaly> {
        let window = self.history
            .entry(node.to_string())
            .or_insert_with(|| VecDeque::with_capacity(self.window_size));

        if window.len() >= self.window_size {
            window.pop_front();
        }
        window.push_back(tokens_per_second);

        // 至少需要 10 筆資料才做判斷
        if window.len() < 10 {
            return None;
        }

        let mean: f32 = window.iter().sum::<f32>() / window.len() as f32;
        let variance: f32 = window.iter()
            .map(|x| (x - mean).powi(2))
            .sum::<f32>() / window.len() as f32;
        let stddev = variance.sqrt();

        // 當前值低於 (mean - threshold * stddev) 視為異常
        let lower_bound = mean - self.threshold_stddev * stddev;
        if tokens_per_second < lower_bound && tokens_per_second < mean * 0.5 {
            Some(Anomaly {
                node: node.to_string(),
                metric: "tokens_per_second".to_string(),
                expected: mean,
                actual: tokens_per_second,
                stddev_distance: (mean - tokens_per_second) / stddev.max(0.01),
            })
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct Anomaly {
    pub node: String,
    pub metric: String,
    pub expected: f32,
    pub actual: f32,
    pub stddev_distance: f32,
}
```

---

## 10. Rust 實作程式碼

### 10.1 核心 ClusterMonitor 結構

```rust
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use tracing::{info, warn, error};

/// 叢集監控器 — 心跳收集、健康檢查、狀態機驅動
pub struct ClusterMonitor {
    /// 節點狀態表 (node_name -> NodeInfo)
    nodes: Arc<RwLock<HashMap<String, NodeInfo>>>,
    /// 健康檢查器
    health_checker: Arc<HealthChecker>,
    /// 恢復執行器
    recovery_executor: Arc<RecoveryExecutor>,
    /// 異常檢測器
    anomaly_detector: Arc<RwLock<AnomalyDetector>>,
    /// 降級模式追蹤
    degradation: Arc<RwLock<DegradationLevel>>,
    /// Telegram 告警發送器
    alerter: Arc<ClusterAlerter>,
    /// Split brain 檢測器
    split_brain: Arc<RwLock<SplitBrainDetector>>,
    /// 心跳設定
    config: ClusterMonitorConfig,
}

#[derive(Debug, Clone)]
pub struct ClusterMonitorConfig {
    /// 各網路類型的心跳間隔
    pub heartbeat_intervals: HashMap<NetworkType, Duration>,
    /// 進入 Suspect 的心跳失敗次數
    pub suspect_threshold: u32,
    /// Suspect 到 Offline 的超時
    pub suspect_timeout: Duration,
    /// 最大恢復嘗試次數
    pub max_recovery_attempts: u32,
    /// L3 推理測試間隔
    pub inference_check_interval: Duration,
    /// 每日報告時間 (UTC 小時)
    pub daily_report_hour: u32,
}

impl Default for ClusterMonitorConfig {
    fn default() -> Self {
        let mut intervals = HashMap::new();
        intervals.insert(NetworkType::Local, Duration::from_secs(5));
        intervals.insert(NetworkType::Lan, Duration::from_secs(10));
        intervals.insert(NetworkType::Tailscale, Duration::from_secs(15));
        intervals.insert(NetworkType::Cloud, Duration::from_secs(15));

        Self {
            heartbeat_intervals: intervals,
            suspect_threshold: 3,
            suspect_timeout: Duration::from_secs(30),
            max_recovery_attempts: 3,
            inference_check_interval: Duration::from_secs(300), // 5 分鐘
            daily_report_hour: 9,  // UTC 09:00 = 台灣 17:00
        }
    }
}

/// 完整的節點資訊 (Hub 端維護)
#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub name: String,
    pub host: String,
    pub ollama_port: u16,
    pub network_type: NetworkType,
    pub state: NodeState,
    pub last_heartbeat: Option<DateTime<Utc>>,
    pub consecutive_failures: u32,
    pub recovery_attempts: u32,
    pub last_state_change: DateTime<Utc>,

    // 來自心跳的即時指標
    pub cpu_usage: f32,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub gpu_usage: Option<f32>,
    pub loaded_models: Vec<String>,
    pub available_models: Vec<String>,
    pub pending_requests: u32,
    pub inference_speed: Option<InferenceMetrics>,
}

impl NodeInfo {
    pub fn new(name: &str, host: &str, port: u16, network_type: NetworkType) -> Self {
        Self {
            name: name.to_string(),
            host: host.to_string(),
            ollama_port: port,
            network_type,
            state: NodeState::Online,
            last_heartbeat: None,
            consecutive_failures: 0,
            recovery_attempts: 0,
            last_state_change: Utc::now(),
            cpu_usage: 0.0,
            memory_used_mb: 0,
            memory_total_mb: 0,
            gpu_usage: None,
            loaded_models: vec![],
            available_models: vec![],
            pending_requests: 0,
            inference_speed: None,
        }
    }
}
```

### 10.2 心跳收集迴圈

```rust
impl ClusterMonitor {
    /// 啟動心跳監控主迴圈
    pub async fn start(self: Arc<Self>) {
        let monitor = self.clone();

        // 心跳收集 + 健康檢查迴圈
        let heartbeat_handle = tokio::spawn(async move {
            monitor.heartbeat_loop().await;
        });

        // L3 推理測試迴圈
        let monitor2 = self.clone();
        let inference_handle = tokio::spawn(async move {
            monitor2.inference_check_loop().await;
        });

        // 每日報告迴圈
        let monitor3 = self.clone();
        let report_handle = tokio::spawn(async move {
            monitor3.daily_report_loop().await;
        });

        // 恢復管理迴圈
        let monitor4 = self.clone();
        let recovery_handle = tokio::spawn(async move {
            monitor4.recovery_loop().await;
        });

        info!("ClusterMonitor started with 4 background tasks");

        // 等待所有任務 (正常情況下不會結束)
        let _ = tokio::join!(heartbeat_handle, inference_handle, report_handle, recovery_handle);
    }

    /// 主心跳迴圈: 以最短間隔 tick，對每個節點檢查是否到了它的心跳時間
    async fn heartbeat_loop(&self) {
        // 最短 tick 間隔 = 所有心跳間隔的 GCD，簡化為 5 秒
        let tick = Duration::from_secs(5);
        let mut interval = tokio::time::interval(tick);
        let mut last_check: HashMap<String, tokio::time::Instant> = HashMap::new();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("HTTP client");

        loop {
            interval.tick().await;
            let nodes = self.nodes.read().await.clone();

            for (name, node) in &nodes {
                // 跳過 Local 節點 (Z13 自己)
                if node.network_type == NetworkType::Local {
                    self.handle_local_heartbeat(name).await;
                    continue;
                }

                // 檢查是否到了該節點的心跳檢查時間
                let hb_interval = self.config.heartbeat_intervals
                    .get(&node.network_type)
                    .copied()
                    .unwrap_or(Duration::from_secs(15));

                let now = tokio::time::Instant::now();
                let should_check = match last_check.get(name) {
                    Some(last) => now.duration_since(*last) >= hb_interval,
                    None => true,
                };

                if !should_check {
                    continue;
                }
                last_check.insert(name.clone(), now);

                // 執行 L1 + L2 檢查
                let health = self.health_checker.check_l1_l2(&client, node).await;
                self.process_health_result(name, health).await;
            }
        }
    }

    /// 處理健康檢查結果，驅動狀態機
    async fn process_health_result(&self, node_name: &str, health: HealthCheckResult) {
        let mut nodes = self.nodes.write().await;
        let Some(node) = nodes.get_mut(node_name) else { return };

        let old_state = node.state;
        let event = match health.overall {
            HealthLevel::Healthy | HealthLevel::Degraded => {
                node.consecutive_failures = 0;
                NodeEvent::HeartbeatReceived(NodeHeartbeat {
                    node_name: node_name.to_string(),
                    sequence: 0, // TODO: 從心跳報告取得
                    timestamp: Utc::now(),
                    cpu_usage_percent: 0.0,
                    memory_used_mb: 0,
                    memory_total_mb: 0,
                    gpu_usage_percent: None,
                    gpu_memory_used_mb: None,
                    gpu_memory_total_mb: None,
                    ollama_alive: true,
                    loaded_models: vec![],
                    available_models: vec![],
                    pending_requests: 0,
                    inference_speed: None,
                    network_type: node.network_type.clone(),
                    round_trip_ms: match health.level1_ping {
                        CheckStatus::Pass { latency_ms } => Some(latency_ms),
                        _ => None,
                    },
                })
            }
            HealthLevel::Unhealthy | HealthLevel::Unreachable => {
                node.consecutive_failures += 1;
                if node.consecutive_failures >= self.config.suspect_threshold {
                    NodeEvent::HeartbeatTimeout
                } else {
                    return; // 還沒到閾值，不做狀態變更
                }
            }
        };

        // 狀態機轉換
        let new_state = self.transition(node, event).await;

        if old_state != new_state {
            info!(
                "Node '{}' state transition: {:?} -> {:?}",
                node_name, old_state, new_state
            );
            node.state = new_state;
            node.last_state_change = Utc::now();

            // 發送告警
            drop(nodes); // 釋放鎖，避免死鎖
            self.alert_state_change(node_name, old_state, new_state).await;

            // 更新降級等級
            self.update_degradation_level().await;
        }
    }

    /// 狀態機轉換邏輯
    async fn transition(&self, node: &NodeInfo, event: NodeEvent) -> NodeState {
        match (node.state, &event) {
            // Online -> Suspect (心跳超時)
            (NodeState::Online, NodeEvent::HeartbeatTimeout) => {
                NodeState::Suspect
            }

            // Suspect -> Online (心跳恢復)
            (NodeState::Suspect, NodeEvent::HeartbeatReceived(_)) => {
                NodeState::Online
            }

            // Suspect -> Offline (超時)
            (NodeState::Suspect, NodeEvent::SuspectTimeout) => {
                NodeState::Offline
            }

            // Offline -> Online (直接恢復)
            (NodeState::Offline, NodeEvent::HeartbeatReceived(_)) => {
                NodeState::Online
            }

            // Offline -> Recovering (開始恢復)
            (NodeState::Offline, NodeEvent::RecoveryStarted) => {
                NodeState::Recovering
            }

            // Recovering -> Online (恢復成功)
            (NodeState::Recovering, NodeEvent::RecoverySucceeded) => {
                NodeState::Online
            }

            // Recovering -> Offline (恢復失敗)
            (NodeState::Recovering, NodeEvent::RecoveryFailed) => {
                NodeState::Offline
            }

            // Offline/Recovering -> Dead (超過最大重試)
            (NodeState::Offline | NodeState::Recovering, NodeEvent::MaxRecoveryExceeded) => {
                NodeState::Dead
            }

            // Dead -> Offline (手動重置)
            (NodeState::Dead, NodeEvent::ManualReset) => {
                NodeState::Offline
            }

            // 其他: 保持不變
            _ => node.state,
        }
    }

    /// L3 推理測試迴圈
    async fn inference_check_loop(&self) {
        let mut interval = tokio::time::interval(self.config.inference_check_interval);

        loop {
            interval.tick().await;
            let nodes = self.nodes.read().await.clone();

            for (name, node) in &nodes {
                if node.state != NodeState::Online {
                    continue;
                }

                let result = self.health_checker.check_l3(node).await;
                if let CheckStatus::Fail { ref error } = result {
                    warn!("L3 inference check failed for '{}': {}", name, error);

                    // 記錄異常但不立即改變狀態
                    // (L3 失敗可能只是模型未載入，不代表節點有問題)
                    if let Some(anomaly) = self.anomaly_detector.write().await
                        .record(name, 0.0)
                    {
                        self.alerter.send_anomaly(&anomaly).await;
                    }
                }
            }
        }
    }

    /// 恢復管理迴圈: 檢查 Suspect/Offline 節點並嘗試恢復
    async fn recovery_loop(&self) {
        let mut interval = tokio::time::interval(Duration::from_secs(30));

        loop {
            interval.tick().await;
            let nodes = self.nodes.read().await.clone();

            for (name, node) in &nodes {
                match node.state {
                    NodeState::Suspect => {
                        // 檢查是否超過 suspect_timeout
                        let elapsed = Utc::now()
                            .signed_duration_since(node.last_state_change);
                        if elapsed > chrono::Duration::from_std(self.config.suspect_timeout).unwrap_or_default() {
                            // 轉為 Offline
                            let mut nodes_w = self.nodes.write().await;
                            if let Some(n) = nodes_w.get_mut(name) {
                                n.state = NodeState::Offline;
                                n.last_state_change = Utc::now();
                            }
                            drop(nodes_w);

                            self.alert_state_change(name, NodeState::Suspect, NodeState::Offline).await;
                            self.trigger_task_migration(name).await;
                            self.update_degradation_level().await;
                        }
                    }
                    NodeState::Offline => {
                        if node.recovery_attempts < self.config.max_recovery_attempts {
                            // 嘗試恢復
                            let health = self.health_checker.full_check(node).await;
                            let result = self.recovery_executor.recover(node, &health).await;

                            let mut nodes_w = self.nodes.write().await;
                            if let Some(n) = nodes_w.get_mut(name) {
                                match result {
                                    RecoveryResult::Success(msg) => {
                                        n.state = NodeState::Online;
                                        n.recovery_attempts = 0;
                                        n.consecutive_failures = 0;
                                        n.last_state_change = Utc::now();
                                        info!("Node '{}' recovered: {}", name, msg);
                                    }
                                    RecoveryResult::Failed(msg) => {
                                        n.recovery_attempts += 1;
                                        warn!("Recovery attempt {} failed for '{}': {}",
                                            n.recovery_attempts, name, msg);

                                        if n.recovery_attempts >= self.config.max_recovery_attempts {
                                            n.state = NodeState::Dead;
                                            n.last_state_change = Utc::now();
                                        }
                                    }
                                }
                            }
                            drop(nodes_w);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// 觸發任務遷移
    async fn trigger_task_migration(&self, offline_node: &str) {
        let nodes = self.nodes.read().await;
        let online_nodes: Vec<NodeInfo> = nodes.values()
            .filter(|n| n.state == NodeState::Online)
            .cloned()
            .collect();

        if online_nodes.is_empty() {
            error!("No online nodes available for task migration from '{}'", offline_node);
            return;
        }

        // TODO: 從 TaskQueue 查詢 offline_node 的 pending 任務並遷移
        // 使用 TaskMigrator::select_target() 選擇目標
        info!("Task migration triggered for offline node '{}', {} online nodes available",
            offline_node, online_nodes.len());
    }

    /// 更新降級等級
    async fn update_degradation_level(&self) {
        let nodes = self.nodes.read().await;
        let total = nodes.len();
        let online = nodes.values().filter(|n| n.state == NodeState::Online).count();
        let hub_online = nodes.values()
            .any(|n| n.network_type == NetworkType::Local && n.state == NodeState::Online);

        let new_level = DegradationLevel::from_cluster_state(total, online, hub_online);
        let mut current = self.degradation.write().await;

        if *current != new_level {
            let old = *current;
            *current = new_level;
            drop(current);
            info!("Degradation level changed: {:?} -> {:?}", old, new_level);
            self.alerter.send_degradation_change(old, new_level).await;
        }
    }
}
```

### 10.3 健康檢查器

```rust
pub struct HealthChecker {
    client: reqwest::Client,
    /// L3 推理測試用的 prompt (盡量短，消耗最少資源)
    inference_probe_prompt: String,
}

impl HealthChecker {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("HTTP client"),
            inference_probe_prompt: "Say 'ok'.".to_string(),
        }
    }

    /// L1: TCP 連通測試
    pub async fn check_l1(&self, node: &NodeInfo) -> CheckStatus {
        let addr = format!("{}:{}", node.host, node.ollama_port);
        let start = std::time::Instant::now();

        match tokio::time::timeout(
            Duration::from_secs(3),
            tokio::net::TcpStream::connect(&addr),
        ).await {
            Ok(Ok(_)) => CheckStatus::Pass {
                latency_ms: start.elapsed().as_millis() as u32,
            },
            Ok(Err(e)) => CheckStatus::Fail {
                error: format!("TCP connect failed: {}", e),
            },
            Err(_) => CheckStatus::Fail {
                error: "TCP connect timeout (3s)".to_string(),
            },
        }
    }

    /// L2: Ollama API 服務檢查
    pub async fn check_l2(&self, node: &NodeInfo) -> CheckStatus {
        let url = format!("http://{}:{}/api/tags", node.host, node.ollama_port);
        let start = std::time::Instant::now();

        match self.client.get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                CheckStatus::Pass {
                    latency_ms: start.elapsed().as_millis() as u32,
                }
            }
            Ok(resp) => CheckStatus::Fail {
                error: format!("Ollama API returned {}", resp.status()),
            },
            Err(e) => CheckStatus::Fail {
                error: format!("Ollama API unreachable: {}", e),
            },
        }
    }

    /// L3: 實際推理測試
    pub async fn check_l3(&self, node: &NodeInfo) -> CheckStatus {
        // 使用節點上已載入的第一個模型，或 fallback
        let model = node.loaded_models.first()
            .cloned()
            .unwrap_or_else(|| "qwen3:0.6b".to_string()); // 用最小模型

        let url = format!("http://{}:{}/api/generate", node.host, node.ollama_port);
        let body = serde_json::json!({
            "model": model,
            "prompt": self.inference_probe_prompt,
            "stream": false,
            "options": {
                "num_predict": 5,  // 最多生成 5 tokens
            }
        });

        let start = std::time::Instant::now();
        match tokio::time::timeout(
            Duration::from_secs(30),
            self.client.post(&url).json(&body).send(),
        ).await {
            Ok(Ok(resp)) if resp.status().is_success() => {
                CheckStatus::Pass {
                    latency_ms: start.elapsed().as_millis() as u32,
                }
            }
            Ok(Ok(resp)) => CheckStatus::Fail {
                error: format!("Inference returned {}", resp.status()),
            },
            Ok(Err(e)) => CheckStatus::Fail {
                error: format!("Inference request failed: {}", e),
            },
            Err(_) => CheckStatus::Fail {
                error: "Inference timeout (30s)".to_string(),
            },
        }
    }

    /// L1 + L2 組合檢查
    pub async fn check_l1_l2(&self, client: &reqwest::Client, node: &NodeInfo) -> HealthCheckResult {
        let l1 = self.check_l1(node).await;
        let l2 = if matches!(l1, CheckStatus::Pass { .. }) {
            self.check_l2(node).await
        } else {
            CheckStatus::Skipped
        };

        let overall = match (&l1, &l2) {
            (CheckStatus::Pass { .. }, CheckStatus::Pass { .. }) => HealthLevel::Healthy,
            (CheckStatus::Pass { .. }, CheckStatus::Fail { .. }) => HealthLevel::Unhealthy,
            (CheckStatus::Pass { .. }, CheckStatus::Skipped) => HealthLevel::Healthy,
            _ => HealthLevel::Unreachable,
        };

        HealthCheckResult {
            node_name: node.name.clone(),
            timestamp: Utc::now(),
            level1_ping: l1,
            level2_api: l2,
            level3_inference: CheckStatus::Skipped,
            overall,
        }
    }

    /// 完整三層檢查
    pub async fn full_check(&self, node: &NodeInfo) -> HealthCheckResult {
        let l1 = self.check_l1(node).await;
        let l2 = if matches!(l1, CheckStatus::Pass { .. }) {
            self.check_l2(node).await
        } else {
            CheckStatus::Skipped
        };
        let l3 = if matches!(l2, CheckStatus::Pass { .. }) {
            self.check_l3(node).await
        } else {
            CheckStatus::Skipped
        };

        let overall = match (&l1, &l2, &l3) {
            (CheckStatus::Pass {..}, CheckStatus::Pass {..}, CheckStatus::Pass {..}) => HealthLevel::Healthy,
            (CheckStatus::Pass {..}, CheckStatus::Pass {..}, _) => HealthLevel::Degraded,
            (CheckStatus::Pass {..}, _, _) => HealthLevel::Unhealthy,
            _ => HealthLevel::Unreachable,
        };

        HealthCheckResult {
            node_name: node.name.clone(),
            timestamp: Utc::now(),
            level1_ping: l1,
            level2_api: l2,
            level3_inference: l3,
            overall,
        }
    }
}
```

### 10.4 告警發送器

```rust
/// 叢集告警發送器 (整合 Telegram)
pub struct ClusterAlerter {
    telegram_bot_token: String,
    telegram_chat_id: String,
    client: reqwest::Client,
    /// 告警去重: (node_name, alert_type) -> last_sent
    last_alerts: Arc<RwLock<HashMap<String, DateTime<Utc>>>>,
    /// 告警最小間隔 (避免轟炸)
    min_alert_interval: Duration,
}

impl ClusterAlerter {
    pub async fn send_state_change(
        &self,
        node: &str,
        old: NodeState,
        new: NodeState,
    ) {
        let (level, emoji) = match new {
            NodeState::Online => ("INFO", "✅"),
            NodeState::Suspect => ("WARN", "⚠️"),
            NodeState::Offline => ("CRIT", "🔴"),
            NodeState::Recovering => ("INFO", "🔄"),
            NodeState::Dead => ("EMERG", "💀"),
        };

        let msg = format!(
            "{} [{}] Node <b>{}</b>: {:?} → {:?}",
            emoji, level, node, old, new
        );

        self.send_telegram(&msg).await;
    }

    pub async fn send_degradation_change(
        &self,
        old: DegradationLevel,
        new: DegradationLevel,
    ) {
        let msg = format!(
            "📊 Cluster degradation: <b>{:?}</b> → <b>{:?}</b>\nMax SoT parallelism: {}",
            old, new, new.max_sot_parallelism()
        );
        self.send_telegram(&msg).await;
    }

    pub async fn send_anomaly(&self, anomaly: &Anomaly) {
        // 去重: 同一節點同一指標，10 分鐘內不重複
        let key = format!("{}:{}", anomaly.node, anomaly.metric);
        {
            let alerts = self.last_alerts.read().await;
            if let Some(last) = alerts.get(&key) {
                let elapsed = Utc::now().signed_duration_since(*last);
                if elapsed < chrono::Duration::minutes(10) {
                    return;
                }
            }
        }
        self.last_alerts.write().await
            .insert(key, Utc::now());

        let msg = format!(
            "🔍 [ANOMALY] Node <b>{}</b>: {} dropped\n\
             Expected: {:.1}, Actual: {:.1} ({:.1}σ deviation)",
            anomaly.node, anomaly.metric,
            anomaly.expected, anomaly.actual, anomaly.stddev_distance
        );
        self.send_telegram(&msg).await;
    }

    pub async fn send_daily_report(&self, report: &str) {
        self.send_telegram(report).await;
    }

    async fn send_telegram(&self, text: &str) {
        let url = format!(
            "https://api.telegram.org/bot{}/sendMessage",
            self.telegram_bot_token
        );

        let _ = self.client
            .post(&url)
            .json(&serde_json::json!({
                "chat_id": self.telegram_chat_id,
                "text": text,
                "parse_mode": "HTML",
            }))
            .send()
            .await;
    }
}
```

### 10.5 與現有 ProviderRouter 的整合

```rust
/// 擴展 ProviderRouter 以感知叢集狀態
impl ProviderRouter {
    /// 根據叢集監控資訊動態調整 auto_order
    pub fn update_from_cluster(&mut self, nodes: &[NodeInfo]) {
        // 只保留 Online 節點的 provider
        let online_providers: Vec<String> = nodes.iter()
            .filter(|n| n.state == NodeState::Online)
            .filter_map(|n| {
                // 映射: node_name -> provider_name
                // 例: "m1-mac" 對應 agents.toml 中的 "ollama-m1" provider
                self.node_to_provider(&n.name)
            })
            .collect();

        // 按推理速度排序 (快的排前面)
        // 這裡簡化為保持原有順序但移除離線的
        self.auto_order.retain(|p| {
            // 本地 provider 始終保留
            if p == "ollama" || p == "lmstudio" || p == "lemonade" {
                return true;
            }
            online_providers.contains(p)
        });
    }

    fn node_to_provider(&self, node_name: &str) -> Option<String> {
        // 從設定中查找 node_name 到 provider_name 的映射
        // 這需要在 agents.toml 中加入 [cluster] 區段
        // 暫時簡單映射
        match node_name {
            "z13" => Some("ollama".to_string()),
            "m1-mac" => Some("ollama-m1".to_string()),
            "ayaneo" => Some("ollama-ayaneo".to_string()),
            "acer" => Some("ollama-acer".to_string()),
            _ => None,
        }
    }
}
```

---

## 11. 設定檔格式

### 11.1 agents.toml 新增 [cluster] 區段

```toml
# ~/.phantom-mesh/agents.toml

# ── 現有設定 (不變) ──
[providers.ollama]
type = "ollama"
url = "http://localhost:11434"
default_model = "qwen3:8b"

# ── 叢集節點設定 ──
[cluster]
enabled = true
hub_name = "z13"
backup_hub = "backup-hub"
daily_report_hour = 9           # UTC
suspect_threshold = 3
suspect_timeout_secs = 30
max_recovery_attempts = 3
inference_check_interval_secs = 300

# 節點定義
[[cluster.nodes]]
name = "z13"
host = "127.0.0.1"
ollama_port = 11434
network_type = "local"          # local | lan | tailscale | cloud
role = "hub"                    # hub | worker | standby_hub

[[cluster.nodes]]
name = "m1-mac"
host = "10.0.2.1"           # Tailscale IP
ollama_port = 11434
network_type = "tailscale"
role = "worker"
heartbeat_interval_secs = 15    # 覆寫預設值

[[cluster.nodes]]
name = "ayaneo"
host = "192.168.1.100"
ollama_port = 11434
network_type = "lan"
role = "worker"
provider_name = "ollama-ayaneo" # 映射到 [providers] 中的名稱

[[cluster.nodes]]
name = "acer"
host = "192.168.1.101"
ollama_port = 11434
network_type = "lan"
role = "worker"
provider_name = "ollama-acer"

[[cluster.nodes]]
name = "gpu-cloud-1"
host = "gpu1.example.com"
ollama_port = 11434
network_type = "cloud"
role = "worker"

[[cluster.nodes]]
name = "gpu-cloud-2"
host = "gpu2.example.com"
ollama_port = 11434
network_type = "cloud"
role = "worker"

[[cluster.nodes]]
name = "npu-node"
host = "192.168.1.102"
ollama_port = 8000              # Lemonade 用不同 port
network_type = "lan"
role = "worker"

[[cluster.nodes]]
name = "backup-hub"
host = "192.168.1.200"
ollama_port = 11434
network_type = "lan"
role = "standby_hub"

# ── SSH 恢復設定 ──
[[cluster.recovery.ssh]]
node = "m1-mac"
host = "10.0.2.1"
port = 22
user = "m4932"
key_path = "~/.ssh/id_ed25519"
ollama_restart_cmd = "launchctl kickstart -kp system/com.ollama.server"
reboot_cmd = "sudo shutdown -r now"

[[cluster.recovery.ssh]]
node = "ayaneo"
host = "192.168.1.100"
port = 22
user = "ayaneo"
key_path = "~/.ssh/id_ed25519"
ollama_restart_cmd = "systemctl restart ollama"
reboot_cmd = "sudo reboot"

[[cluster.recovery.ssh]]
node = "acer"
host = "192.168.1.101"
port = 22
user = "acer"
key_path = "~/.ssh/id_ed25519"
ollama_restart_cmd = "systemctl restart ollama"
reboot_cmd = "sudo reboot"

# ── Wake-on-LAN 設定 ──
[[cluster.recovery.wol]]
node = "ayaneo"
mac_address = "AA:BB:CC:DD:EE:01"
broadcast_addr = "192.168.1.255"

[[cluster.recovery.wol]]
node = "acer"
mac_address = "AA:BB:CC:DD:EE:02"
broadcast_addr = "192.168.1.255"
```

---

## 12. 測試策略

### 12.1 單元測試清單

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // ── 狀態機測試 ──

    #[test]
    fn test_state_online_to_suspect() {
        // 連續 3 次心跳失敗 → Suspect
    }

    #[test]
    fn test_state_suspect_to_online() {
        // Suspect 狀態收到心跳 → 回到 Online
    }

    #[test]
    fn test_state_suspect_timeout_to_offline() {
        // Suspect 超過 30 秒 → Offline
    }

    #[test]
    fn test_state_offline_direct_recovery() {
        // Offline 直接收到心跳 → Online (網路恢復)
    }

    #[test]
    fn test_state_dead_needs_manual_reset() {
        // Dead 狀態只能透過 ManualReset 回到 Offline
    }

    #[test]
    fn test_state_max_recovery_to_dead() {
        // 恢復次數超過閾值 → Dead
    }

    // ── 降級模式測試 ──

    #[test]
    fn test_degradation_full() {
        assert_eq!(
            DegradationLevel::from_cluster_state(8, 8, true),
            DegradationLevel::Full
        );
    }

    #[test]
    fn test_degradation_minor() {
        assert_eq!(
            DegradationLevel::from_cluster_state(8, 7, true),
            DegradationLevel::Minor
        );
    }

    #[test]
    fn test_degradation_moderate() {
        assert_eq!(
            DegradationLevel::from_cluster_state(8, 6, true),
            DegradationLevel::Moderate
        );
    }

    #[test]
    fn test_degradation_severe() {
        assert_eq!(
            DegradationLevel::from_cluster_state(8, 3, true),
            DegradationLevel::Severe
        );
    }

    #[test]
    fn test_degradation_single_node() {
        assert_eq!(
            DegradationLevel::from_cluster_state(8, 1, true),
            DegradationLevel::SingleNode
        );
    }

    #[test]
    fn test_degradation_hub_down() {
        assert_eq!(
            DegradationLevel::from_cluster_state(8, 7, false),
            DegradationLevel::HubDown
        );
    }

    #[test]
    fn test_sot_parallelism() {
        assert_eq!(DegradationLevel::Full.max_sot_parallelism(), 8);
        assert_eq!(DegradationLevel::SingleNode.max_sot_parallelism(), 1);
        assert_eq!(DegradationLevel::HubDown.max_sot_parallelism(), 0);
    }

    // ── 異常檢測測試 ──

    #[test]
    fn test_anomaly_not_enough_data() {
        let mut detector = AnomalyDetector::new();
        // 少於 10 筆不做判斷
        for _ in 0..9 {
            assert!(detector.record("node1", 30.0).is_none());
        }
    }

    #[test]
    fn test_anomaly_detected() {
        let mut detector = AnomalyDetector::new();
        // 建立穩定基線
        for _ in 0..20 {
            detector.record("node1", 30.0);
        }
        // 突然降到 5 t/s → 應該觸發異常
        let result = detector.record("node1", 5.0);
        assert!(result.is_some());
    }

    #[test]
    fn test_anomaly_normal_variance() {
        let mut detector = AnomalyDetector::new();
        // 正常波動不應觸發
        for i in 0..20 {
            let speed = 30.0 + (i as f32 % 5.0) - 2.5; // 27.5 ~ 32.5
            assert!(detector.record("node1", speed).is_none());
        }
    }

    // ── 任務遷移測試 ──

    #[test]
    fn test_select_target_prefers_loaded_model() {
        // 已載入模型的節點應優先
    }

    #[test]
    fn test_select_target_prefers_low_load() {
        // 負載低的節點應優先
    }

    #[test]
    fn test_select_target_none_when_empty() {
        let migrator = TaskMigrator;
        let task = MigratableTask { required_model: "qwen3:8b".into(), .. };
        assert!(migrator.select_target(&task, &[]).is_none());
    }

    // ── Split Brain 測試 ──

    #[test]
    fn test_detect_vpn_partition() {
        // 所有 Tailscale 節點同時離線 → VPN 分區
    }

    #[test]
    fn test_single_node_not_partition() {
        // 單個節點離線不是分區
    }

    // ── 健康檢查測試 (需要 mock HTTP) ──

    #[tokio::test]
    async fn test_l1_timeout() {
        // 對不存在的 host 做 L1 應該 timeout
    }

    #[tokio::test]
    async fn test_l2_requires_l1() {
        // L1 失敗時 L2 應該 Skipped
    }

    // ── WOL 測試 ──

    #[test]
    fn test_wol_magic_packet_format() {
        // 檢查 magic packet 是否符合規格
        // 6 bytes 0xFF + 16 * 6 bytes MAC = 102 bytes
    }
}
```

### 12.2 整合測試

```rust
// tests/cluster_integration.rs

#[tokio::test]
async fn test_cluster_monitor_lifecycle() {
    // 1. 建立 ClusterMonitor
    // 2. 註冊 3 個 mock 節點
    // 3. 模擬心跳收集
    // 4. 模擬一個節點離線
    // 5. 驗證狀態機轉換: Online -> Suspect -> Offline
    // 6. 驗證任務遷移被觸發
    // 7. 模擬節點恢復
    // 8. 驗證回到 Online
}

#[tokio::test]
async fn test_degradation_triggers() {
    // 測試不同數量的節點離線對應正確的降級等級
}

#[tokio::test]
async fn test_alert_dedup() {
    // 確認同一告警在間隔內不會重複發送
}
```

---

## 附錄 A: Telegram 指令擴展

新增的叢集管理指令：

| 指令 | 說明 |
|------|------|
| `/cluster` | 顯示叢集狀態總覽 |
| `/node <name>` | 顯示特定節點詳細資訊 |
| `/reset <name>` | 手動重置 Dead 節點為 Offline |
| `/drain <name>` | 優雅地將節點標記為 offline (計劃性維護) |
| `/undrain <name>` | 將已 drain 的節點恢復為 online |
| `/migrate <task_id> <target>` | 手動遷移指定任務 |

---

## 附錄 B: 效能預估

| 指標 | 數值 |
|------|------|
| 心跳 HTTP 請求大小 | ~500 bytes |
| 心跳每節點頻寬 | ~200 bytes/s (15s interval) |
| 全叢集心跳頻寬 | ~1.6 KB/s (8 nodes) |
| 故障檢測延遲 (LAN) | 30~60 秒 |
| 故障檢測延遲 (VPN) | 45~75 秒 |
| 任務遷移延遲 | ~5 秒 (不含模型載入) |
| 模型載入延遲 | 10~60 秒 (視模型大小) |
| SQLite 心跳寫入 | ~0.1ms per write |
| 記憶體使用 (監控器) | ~10 MB (含歷史資料) |

---

## 附錄 C: 實作優先序

| 優先級 | 模組 | 預估工時 |
|--------|------|---------|
| P0 | NodeState 狀態機 + ClusterMonitor 核心 | 4 小時 |
| P0 | 三層健康檢查 (HealthChecker) | 3 小時 |
| P0 | 心跳收集迴圈 | 2 小時 |
| P1 | 降級模式 + ProviderRouter 整合 | 3 小時 |
| P1 | Telegram 告警 (ClusterAlerter) | 2 小時 |
| P1 | 異常檢測 (AnomalyDetector) | 2 小時 |
| P2 | SSH 恢復 + WOL | 3 小時 |
| P2 | Split Brain 檢測 | 2 小時 |
| P2 | 任務遷移 (TaskMigrator) | 3 小時 |
| P3 | Hub Failover (HubElection) | 4 小時 |
| P3 | 每日報告生成 | 1 小時 |
| P3 | Telegram 指令 (/cluster, /node 等) | 2 小時 |
|    | **總計** | **~31 小時** |

---

## 附錄 D: 現有程式碼修改清單

| 檔案 | 修改 |
|------|------|
| `src/cluster.rs` | 大幅重寫: 加入 `ClusterMonitor`, `NodeInfo`, `NodeState`, `HealthChecker` 等 |
| `src/lib.rs` | 新增 re-export: `ClusterMonitor`, `NodeState`, `DegradationLevel` 等 |
| `src/providers/router.rs` | 新增 `update_from_cluster()` 方法 |
| `src/telegram.rs` | 新增 `/cluster`, `/node`, `/reset`, `/drain` 指令處理 |
| `src/main.rs` | 啟動 `ClusterMonitor::start()` |
| `src/agent_runtime.rs` | 查詢 `DegradationLevel` 以調整 SoT 並行度 |
| `src/cron.rs` | 查詢 `DegradationLevel.min_cron_priority()` 以過濾任務 |
| `src/skeleton.rs` | 從 `ClusterMonitor` 取得可用的 expansion providers |
| `src/task_queue.rs` | 新增 `node_affinity` 欄位和遷移 API |
| `Cargo.toml` | 新增依賴: `sysinfo` (系統資訊收集) |

---

*本文件由 Phantom Mesh 分散式系統可靠性工程設計，版本 1.0*
