# Clawtex 分散式狀態與記憶體管理設計

> 日期: 2026-03-05
> 範圍: 8 機集群 (1 Hub Z13 + 7 Worker)
> 作者: 分散式系統架構設計

---

## 目錄

1. [架構總覽](#1-架構總覽)
2. [設計決策：集中式 Hub + 無狀態 Worker](#2-設計決策集中式-hub--無狀態-worker)
3. [資料流圖](#3-資料流圖)
4. [任務狀態追蹤](#4-任務狀態追蹤)
5. [檔案共享與傳輸](#5-檔案共享與傳輸)
6. [成本與營收追蹤](#6-成本與營收追蹤)
7. [分散式語義記憶](#7-分散式語義記憶)
8. [一致性模型與 CAP 分析](#8-一致性模型與-cap-分析)
9. [Rust Struct 定義](#9-rust-struct-定義)
10. [通訊協議](#10-通訊協議)
11. [故障處理與恢復](#11-故障處理與恢復)
12. [實施路線圖](#12-實施路線圖)

---

## 1. 架構總覽

### 現有狀態存儲分析

目前 clawtex-core 的狀態全部集中在單機：

| 存儲類型 | 位置 | 資料 | 存取模式 |
|---------|------|------|---------|
| SQLite (`core.db`) | `~/.clawtex/core.db` | memories, cost_records, revenue_records, tasks, cluster_nodes | 讀寫密集 |
| 檔案系統 | `~/.clawtex/workspace/` | Hand 輸出檔案、工具產物 | 寫後讀 |
| 記憶體 (in-process) | HashMap/Arc | ToolRegistry, AgentRuntime, HandRegistry, EStop, Heartbeat | 高頻讀取 |
| Hands 輸出 | in-process Vec | 每個 Phase 的 PhaseOutput | 短生命週期 |

### 目標架構

```
                        ┌─────────────────────────────────────┐
                        │           Z13 Hub (主控)             │
                        │                                     │
                        │  ┌──────────┐  ┌──────────────────┐ │
  Telegram ────────────>│  │ Telegram  │  │  HTTP API / SSE  │ │
                        │  │ Handler   │  │  Gateway (:7878) │ │
                        │  └────┬──────┘  └────────┬─────────┘ │
                        │       │                  │           │
                        │  ┌────▼──────────────────▼─────────┐ │
                        │  │     TaskScheduler (集中調度)      │ │
                        │  │  ┌─────────────────────────┐    │ │
                        │  │  │  DistributedTaskQueue    │    │ │
                        │  │  │  (SQLite + WAL)          │    │ │
                        │  │  └─────────────────────────┘    │ │
                        │  └─────────────┬───────────────────┘ │
                        │                │                     │
                        │  ┌─────────────▼───────────────────┐ │
                        │  │     StateManager (集中狀態)       │ │
                        │  │  ┌──────┐ ┌──────┐ ┌──────────┐ │ │
                        │  │  │Memory│ │ Cost │ │ Revenue  │ │ │
                        │  │  │Store │ │Track │ │ Tracker  │ │ │
                        │  │  │(PG)  │ │(SQL) │ │ (SQLite) │ │ │
                        │  │  └──────┘ └──────┘ └──────────┘ │ │
                        │  └─────────────────────────────────┘ │
                        │                │                     │
                        │  ┌─────────────▼───────────────────┐ │
                        │  │   WorkerManager (連線管理)        │ │
                        │  └──┬──────┬──────┬──────┬─────────┘ │
                        └─────┼──────┼──────┼──────┼───────────┘
                              │      │      │      │
               ┌──────────────┘      │      │      └──────────────┐
               │              ┌──────┘      └──────┐              │
        ┌──────▼──────┐┌──────▼──────┐┌──────▼──────┐┌──────▼──────┐
        │  Worker 1   ││  Worker 2   ││  Worker 3   ││  Worker 4-7 │
        │  (無狀態)    ││  (無狀態)    ││  (無狀態)    ││  (無狀態)    │
        │             ││             ││             ││             │
        │ ┌─────────┐ ││ ┌─────────┐ ││ ┌─────────┐ ││ ┌─────────┐ │
        │ │Ollama/  │ ││ │Ollama/  │ ││ │Ollama/  │ ││ │Ollama/  │ │
        │ │LMStudio │ ││ │LMStudio │ ││ │LMStudio │ ││ │LMStudio │ │
        │ └─────────┘ ││ └─────────┘ ││ └─────────┘ ││ └─────────┘ │
        │ ┌─────────┐ ││ ┌─────────┐ ││ ┌─────────┐ ││ ┌─────────┐ │
        │ │ToolExec │ ││ │ToolExec │ ││ │ToolExec │ ││ │ToolExec │ │
        │ │ Sandbox  │ ││ │ Sandbox  │ ││ │ Sandbox  │ ││ │ Sandbox  │ │
        │ └─────────┘ ││ └─────────┘ ││ └─────────┘ ││ └─────────┘ │
        └─────────────┘└─────────────┘└─────────────┘└─────────────┘
```

---

## 2. 設計決策：集中式 Hub + 無狀態 Worker

### 為什麼不用完全分散式？

| 考量因素 | 完全分散式 | 集中式 Hub | 我們的選擇 |
|---------|----------|----------|-----------|
| 複雜度 | 極高 (Raft/Paxos) | 低 | **集中式** |
| 8 機規模 | 過度工程 | 完美匹配 | **集中式** |
| 一致性 | 需要共識協議 | 天然強一致 | **集中式** |
| 單點故障 | 無 | Hub 是 SPOF | 可接受 (見故障處理) |
| 開發速度 | 數月 | 數週 | **集中式** |
| SQLite 跨網路 | 不支援 | 不需要 | **集中式** |

### 核心原則

1. **Hub 擁有所有持久狀態** — SQLite/PostgreSQL 只在 Z13 上運行
2. **Worker 完全無狀態** — 每次任務透過 Hub 取得輸入，完成後回報結果
3. **Worker 只做兩件事** — LLM 推理 + 工具沙箱執行
4. **所有 Telegram/HTTP 互動都在 Hub** — 使用者只接觸 Hub
5. **Hub 可降級為單機模式** — Worker 全掛時自動回退

---

## 3. 資料流圖

### 任務執行完整流程

```
使用者 ──[Telegram/HTTP]──> Hub

Hub:
  1. 接收請求 (prompt / hand / cron trigger)
  2. 查詢 TaskScheduler → 選擇最佳 Worker
  3. 若為 Hand → 拆分為 Phase 任務序列
  4. 記錄 TaskRecord (status=Queued, node=selected_worker)

Hub ──[gRPC TaskAssignment]──> Worker:
  5. Worker 收到 TaskAssignment { task_id, prompt, system_prompt, tools, context }
  6. Worker 載入本地 LLM 模型
  7. Worker 執行 agent loop (多輪工具呼叫)
  8. 每輪 → Worker ──[gRPC HeartbeatReport]──> Hub (進度更新)
  9. 工具產生檔案 → Worker ──[gRPC FileUpload]──> Hub

Worker ──[gRPC TaskResult]──> Hub:
  10. Hub 更新 TaskRecord (status=Done, result=output)
  11. Hub 記錄 CostRecord (tokens, duration, provider)
  12. Hub 儲存 memories (若工具呼叫了 memory_store)
  13. Hub 通知 Telegram 使用者
  14. 若為 Hand phase → 觸發下一 phase
  15. 若為 chain_to → 觸發下一 Hand
```

### 記憶體查詢流程 (Worker 需要 recall)

```
Worker 執行 agent loop
  → 工具呼叫 memory_recall("keyword")
  → Worker ──[gRPC MemoryQuery { query, limit }]──> Hub
  → Hub 查詢 MemoryStore (SQLite or PG)
  → Hub ──[gRPC MemoryResult { entries }]──> Worker
  → Worker 將結果注入 agent context
```

---

## 4. 任務狀態追蹤

### 現有 TaskQueue 的問題

現有 `task_queue.rs` 中的 `TaskQueue` 缺少關鍵欄位：
- 沒有 `assigned_node` — 不知道任務在哪台機器
- 沒有 `phase_index` — Hand 的 phase 進度無法追蹤
- 沒有 `hand_name` — 無法關聯 Hand 工作流
- 沒有 `priority` — 無法實現優先級調度
- 沒有 heartbeat 機制 — 無法偵測 Worker 死亡

### 新的分散式任務模型

```
┌──────────────────────────────────────────────────────┐
│                DistributedTaskQueue                    │
│                                                      │
│  SQLite Table: distributed_tasks                     │
│  ┌────────────────────────────────────────────────┐  │
│  │ task_id       TEXT PK                          │  │
│  │ parent_id     TEXT (Hand chain)                │  │
│  │ hand_name     TEXT                             │  │
│  │ phase_index   INT                              │  │
│  │ total_phases  INT                              │  │
│  │ prompt        TEXT                             │  │
│  │ system_prompt TEXT                             │  │
│  │ tools         TEXT (JSON array)                │  │
│  │ context       TEXT (前一 phase 輸出)            │  │
│  │ status        TEXT (queued/assigned/running/    │  │
│  │               done/failed/cancelled)           │  │
│  │ priority      INT (0=low, 5=normal, 10=urgent) │  │
│  │ assigned_node TEXT                             │  │
│  │ result        TEXT                             │  │
│  │ tokens_used   INT                              │  │
│  │ cost_usd      REAL                             │  │
│  │ error         TEXT                             │  │
│  │ created_at    TEXT                             │  │
│  │ started_at    TEXT                             │  │
│  │ completed_at  TEXT                             │  │
│  │ last_heartbeat TEXT                            │  │
│  └────────────────────────────────────────────────┘  │
│                                                      │
│  調度策略:                                            │
│  1. FIFO within priority level                       │
│  2. 工作竊取: idle Worker 主動 pull                    │
│  3. 模型親和性: 優先分配到已載入所需模型的 Worker        │
│  4. 負載均衡: 優先分配到 running_tasks 最少的 Worker    │
└──────────────────────────────────────────────────────┘
```

### Telegram 跨節點進度顯示

```
Hub 透過 TaskRecord 的即時狀態 + Worker heartbeat 產生進度訊息：

/hand seo_content "AI tools"
  Phase 1/5: keyword_research [Worker-2] ⏳ Running (45s)
  Phase 2/5: competitor_analysis [Worker-5] ⏳ Running (20s)
  Phase 3/5: article_generation [Queued]
  Phase 4/5: seo_optimization [Queued]
  Phase 5/5: publish_and_promote [Queued]

更新機制:
  - Worker 每 10s 發送 HeartbeatReport
  - HeartbeatReport 包含: task_id, round_number, current_tool, tokens_so_far
  - Hub 收到後更新 Telegram 訊息 (edit_message)
```

---

## 5. 檔案共享與傳輸

### 方案比較

| 方案 | 延遲 | 複雜度 | 安全性 | 適用場景 |
|------|------|--------|--------|---------|
| NFS | 低 | 中 | 低 | 持續大量讀寫 |
| SCP/SFTP | 中 | 低 | 高 | 低頻大檔案 |
| HTTP Upload | 中 | 低 | 高 | 結構化傳輸 |
| gRPC Stream | 最低 | 中 | 高 | 已有連線 |

### 選擇：gRPC 串流上傳（結合在現有連線中）

理由：
- 已經需要 gRPC 做任務調度，不需額外協議
- 串流上傳支援大檔案（無需一次載入記憶體）
- Tailscale 已提供端對端加密
- 比 NFS 更安全（不暴露整個檔案系統）

```
檔案傳輸流程:

Worker 工具執行 file_write("/workspace/output.md", content)
  → 寫入 Worker 本地 /tmp/clawtex_task_{id}/output.md
  → 任務完成時，掃描 /tmp/clawtex_task_{id}/ 下所有檔案
  → 透過 gRPC FileUpload 串流上傳至 Hub
  → Hub 存入 ~/.clawtex/workspace/{task_id}/output.md

Hub 端目錄結構:
  ~/.clawtex/workspace/
    ├── {task_id_1}/
    │   ├── output.md
    │   └── chart.png
    ├── {task_id_2}/
    │   └── report.pdf
    └── latest/          ← 符號連結到最新完成的任務
```

### workspace/ 同步策略

**不需要跨節點同步 workspace/**。原因：
- Worker 執行任務時不需要讀取其他任務的產出
- 若 Phase 2 需要 Phase 1 的檔案，透過 `context` 傳遞文字內容
- 若確實需要前一 phase 的二進位檔案，Hub 在 TaskAssignment 中附帶

---

## 6. 成本與營收追蹤

### 成本回報機制

```
Worker 執行 LLM 呼叫:
  → 取得 TokenUsage { prompt_tokens, completion_tokens, total_tokens }
  → 計算 estimated_cost_usd = estimate_cost(provider, model, tokens_in, tokens_out)
  → 附加到 TaskResult 中

Hub 收到 TaskResult:
  → 寫入 CostRecord {
      agent: task.agent,
      provider: task.provider,
      model: task.model,
      tokens_in, tokens_out, total_tokens,
      estimated_cost_usd,
      duration_secs: completed_at - started_at,
      context: format!("node:{},hand:{},phase:{}", node, hand_name, phase_index),
    }
```

### Worker 即時成本上報 (Heartbeat 內嵌)

```
HeartbeatReport {
    task_id: String,
    node_name: String,
    round_number: u32,
    current_tool: Option<String>,
    tokens_so_far: u32,         // 累計 token 數
    cost_so_far_usd: f64,       // 累計成本
    memory_mb: u32,             // Worker 記憶體使用量
    gpu_utilization: f32,       // GPU 使用率 (0-100)
}
```

這讓 Hub 可以：
1. 即時監控全集群 token 消耗
2. 在成本超過預算時提前取消任務
3. `/costs` 命令顯示 per-node 細分

### 新增查詢維度

現有 `CostTracker` 增加 `by_node()` 方法：

```sql
SELECT context, SUM(total_tokens), SUM(estimated_cost_usd), COUNT(*)
FROM cost_records
WHERE date_key >= ?1
  AND context LIKE 'node:%'
GROUP BY substr(context, 1, instr(context, ',') - 1)
ORDER BY SUM(estimated_cost_usd) DESC
```

---

## 7. 分散式語義記憶

### 方案比較

| 方案 | 一致性 | 延遲 | 運維成本 | 8 機適合度 |
|------|--------|------|---------|-----------|
| A: Hub SQLite 集中 (Worker HTTP 查詢) | 強 | 中 (網路往返) | 零 | 足夠 |
| B: 每台本地 SQLite (最終一致性) | 弱 | 低 | 高 (同步邏輯) | 過度工程 |
| C: PostgreSQL + pgvector (集中式) | 強 | 低-中 | 中 (PG 部署) | **最佳** |

### 推薦方案：C — PostgreSQL + pgvector（在 Z13 Hub 上）

理由：

1. **已有 pgvector 後端** — `src/memory/pgvector.rs` 已完整實作
2. **真正的向量索引** — HNSW 索引，比 SQLite 暴力掃描快 100x+
3. **並發安全** — PostgreSQL 天然支援多連線，不像 SQLite 需要 Mutex
4. **未來可擴展** — 如果超過 8 機，PG 可以設 read replica
5. **8 機規模剛好** — 不需要分散式資料庫（Cockroach/Spanner），單台 PG 足矣

### 記憶體存取路徑

```
Worker 需要 memory_recall:
  → Worker gRPC call: MemoryQuery { query: "brand_voice_profile", limit: 5 }
  → Hub: MemoryStore.recall("brand_voice_profile", 5, None)
  → Hub: PgVectorMemory → HNSW vector search + keyword search → RRF merge
  → Hub gRPC response: MemoryResult { entries: [...] }
  → Worker 注入 agent context

Worker 需要 memory_store:
  → Worker gRPC call: MemoryStore { key, content, category }
  → Hub: MemoryStore.store(key, content, category, None)
  → Hub: PgVectorMemory → INSERT INTO memories + Ollama embedding
  → Hub gRPC response: MemoryStoreResult { id }

注意: embedding 生成在 Hub 上（Hub 連 Ollama nomic-embed-text），
Worker 不需要跑 embedding 模型
```

### Embedding 服務架構

```
Worker → Hub (gRPC) → MemoryStore → Ollama/embed (Hub 本地)
                                  → PgVectorMemory (Hub 本地 PG)

Hub 上的 Ollama 同時提供:
  1. LLM 推理 (qwen3-coder-next 等)
  2. Embedding 生成 (nomic-embed-text)

若 Hub 的 Ollama 負載過高，可將 embedding 服務拆到獨立容器
```

---

## 8. 一致性模型與 CAP 分析

### CAP Theorem 分析

在 8 機 Clawtex 集群中：

```
    Consistency ─────── Availability
         \                /
          \    Clawtex   /
           \    ★      /
            \        /
             \     /
              \  /
         Partition Tolerance
```

**我們選擇 CP (一致性 + 分區容忍)**：

- **Consistency (C)**: 所有狀態在 Hub，天然強一致
- **Partition Tolerance (P)**: Worker 斷線時任務 fail-over，系統繼續運作
- **Availability (A)**: Hub 是單點故障，但可接受（見故障處理）

### 為什麼不需要最終一致性？

1. **8 台機器 = 小規模** — 不是 100 台的超大規模系統
2. **任務是序列依賴的** — Hand Phase 2 必須等 Phase 1 完成，天然需要強一致
3. **記憶體寫入不頻繁** — 每次 Hand 執行約寫 5-10 條 memory，不是秒級萬寫
4. **成本追蹤容忍延遲** — 即使 heartbeat 延遲幾秒也不影響功能
5. **避免複雜性** — 最終一致性需要 conflict resolution，8 機不值得

### 具體一致性保證

| 資料類型 | 一致性等級 | 理由 |
|---------|----------|------|
| Task 狀態 | 強一致 (Hub SQLite WAL) | 調度正確性要求 |
| Memory 讀寫 | 強一致 (Hub PostgreSQL) | 避免幻讀 |
| Cost 記錄 | 最終一致 (容忍秒級延遲) | 統計報表不需即時 |
| Revenue 記錄 | 強一致 (Hub SQLite) | 財務數據不可丟失 |
| E-Stop | 強一致 (gRPC push) | 安全關鍵 |
| 檔案傳輸 | 最終一致 (完成後上傳) | 非即時需求 |

---

## 9. Rust Struct 定義

### 9.1 節點與連線管理

```rust
/// Worker 節點的完整狀態（Hub 端維護）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerNode {
    /// 節點名稱 (e.g., "m1", "worker-gpu-1")
    pub name: String,
    /// Tailscale IP 或主機名
    pub host: String,
    /// gRPC 埠號
    pub port: u16,
    /// 節點狀態
    pub status: NodeStatus,
    /// 該節點已載入的 LLM 模型列表
    pub loaded_models: Vec<String>,
    /// 硬體能力
    pub capabilities: NodeCapabilities,
    /// 目前在執行的任務數
    pub running_tasks: u32,
    /// 最大並行任務數
    pub max_concurrent: u32,
    /// 最後心跳時間
    pub last_heartbeat: DateTime<Utc>,
    /// 註冊時間
    pub registered_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NodeStatus {
    /// 線上且可接受任務
    Online,
    /// 線上但所有 slot 已滿
    Busy,
    /// 正在排空任務（即將下線）
    Draining,
    /// 離線
    Offline,
    /// 未知（新註冊/心跳過期）
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCapabilities {
    /// GPU 型號 (e.g., "RTX 4070", "RX 7600")
    pub gpu: Option<String>,
    /// 可分配給 LLM 的 VRAM (MB)
    pub vram_mb: u32,
    /// 系統 RAM (MB)
    pub ram_mb: u32,
    /// CPU 核心數
    pub cpu_cores: u32,
    /// 是否有 NPU
    pub has_npu: bool,
    /// 支援的 Provider 列表 (e.g., ["ollama", "lmstudio"])
    pub providers: Vec<String>,
}
```

### 9.2 分散式任務

```rust
/// 分散式任務記錄（取代現有 Task struct）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistributedTask {
    /// 唯一任務 ID
    pub task_id: String,
    /// 父任務 ID (Hand chain 或 phase 關聯)
    pub parent_id: Option<String>,
    /// 關聯的 Hand 名稱
    pub hand_name: Option<String>,
    /// Phase 索引 (0-based)
    pub phase_index: Option<u32>,
    /// 總 phase 數
    pub total_phases: Option<u32>,
    /// 使用者原始 prompt
    pub prompt: String,
    /// System prompt (from phase or agent config)
    pub system_prompt: Option<String>,
    /// 允許使用的工具列表
    pub tools: Vec<String>,
    /// 上下文（前一 phase 的輸出，或 Hand settings）
    pub context: Option<String>,
    /// 任務狀態
    pub status: DistributedTaskStatus,
    /// 優先級 (0=低, 5=正常, 10=緊急)
    pub priority: u32,
    /// 分配到的 Worker 節點名
    pub assigned_node: Option<String>,
    /// 請求使用的 provider (e.g., "ollama", "auto")
    pub provider_hint: Option<String>,
    /// 請求使用的 model
    pub model_hint: Option<String>,
    /// 最大工具呼叫輪數
    pub max_rounds: u32,
    /// 執行結果
    pub result: Option<TaskResultPayload>,
    /// 錯誤訊息 (status=Failed 時)
    pub error: Option<String>,
    /// 建立時間
    pub created_at: DateTime<Utc>,
    /// 開始執行時間
    pub started_at: Option<DateTime<Utc>>,
    /// 完成時間
    pub completed_at: Option<DateTime<Utc>>,
    /// 最後心跳
    pub last_heartbeat: Option<DateTime<Utc>>,
    /// 發起者 (Telegram user_id, "cron", "chain:hand_name")
    pub initiator: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DistributedTaskStatus {
    /// 等待調度
    Queued,
    /// 已分配到 Worker，等待 Worker 確認
    Assigned,
    /// Worker 正在執行
    Running,
    /// 成功完成
    Done,
    /// 執行失敗
    Failed,
    /// 被使用者或系統取消
    Cancelled,
    /// Worker 超時未回應，待重新調度
    TimedOut,
}

/// 任務執行結果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResultPayload {
    /// LLM 最終輸出文字
    pub output: String,
    /// 使用的 token 數
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub total_tokens: u32,
    /// 預估成本
    pub estimated_cost_usd: f64,
    /// 實際使用的 provider
    pub provider_used: String,
    /// 實際使用的 model
    pub model_used: String,
    /// 工具呼叫次數
    pub tool_calls_made: u32,
    /// 產出的檔案列表
    pub output_files: Vec<OutputFile>,
    /// 執行時間 (秒)
    pub duration_secs: f64,
    /// Memory 操作紀錄 (store/recall/forget)
    pub memory_ops: Vec<MemoryOperation>,
}

/// 任務產出的檔案
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputFile {
    /// Worker 端相對路徑
    pub relative_path: String,
    /// 檔案大小 (bytes)
    pub size_bytes: u64,
    /// MIME type
    pub content_type: String,
}

/// Worker 回報的 Memory 操作（Hub 代為執行）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemoryOperation {
    Store {
        key: String,
        content: String,
        category: String,
    },
    Recall {
        query: String,
        limit: usize,
        /// Hub 回傳的結果 (在任務完成時已填入)
        results: Vec<MemoryEntry>,
    },
    Forget {
        key: String,
    },
}
```

### 9.3 Hub 端集中調度器

```rust
/// Hub 端的任務調度器
pub struct TaskScheduler {
    /// 任務持久化 (SQLite WAL)
    db: Connection,
    /// 線上 Worker 列表 (in-memory cache, 由 heartbeat 更新)
    workers: Arc<RwLock<HashMap<String, WorkerNode>>>,
    /// 任務佇列通知 channel
    task_notify: tokio::sync::Notify,
    /// E-Stop 旗標
    estop: Arc<EStop>,
    /// 模型親和性快取: model_name -> Vec<node_name>
    model_affinity: Arc<RwLock<HashMap<String, Vec<String>>>>,
}

impl TaskScheduler {
    /// 提交新任務到佇列
    pub async fn submit(&self, task: DistributedTask) -> Result<String>;

    /// 為指定 Worker 取得下一個可執行的任務
    /// 考慮: 優先級 > 模型親和性 > 負載均衡
    pub async fn poll_task(&self, node_name: &str) -> Result<Option<DistributedTask>>;

    /// Worker 回報任務完成
    pub async fn complete_task(&self, task_id: &str, result: TaskResultPayload) -> Result<()>;

    /// Worker 回報任務失敗
    pub async fn fail_task(&self, task_id: &str, error: &str) -> Result<()>;

    /// 處理心跳，更新 Worker 狀態
    pub async fn heartbeat(&self, report: HeartbeatReport) -> Result<()>;

    /// 偵測超時任務並重新調度
    pub async fn reap_timed_out(&self, timeout: Duration) -> Result<Vec<String>>;

    /// 將 Hand 拆分為一系列 Phase 任務
    pub async fn submit_hand(
        &self,
        hand: &Hand,
        user_input: &str,
        initiator: &str,
    ) -> Result<String>;

    /// 查詢任務狀態 (Telegram 進度顯示用)
    pub async fn get_task_tree(&self, root_task_id: &str) -> Result<Vec<DistributedTask>>;
}
```

### 9.4 Worker 端執行器

```rust
/// Worker 端的任務執行器 (每台 Worker 機器上運行)
pub struct WorkerExecutor {
    /// Worker 名稱
    node_name: String,
    /// Hub 的 gRPC 地址
    hub_addr: String,
    /// 本地 LLM provider (Ollama/LMStudio)
    local_provider: Box<dyn Provider>,
    /// 本地工具執行環境
    tool_sandbox: ToolSandbox,
    /// 最大並行任務數
    max_concurrent: u32,
    /// 正在執行的任務
    running: Arc<RwLock<HashMap<String, JoinHandle<()>>>>,
    /// E-Stop (從 Hub 推送更新)
    estop: Arc<EStop>,
}

impl WorkerExecutor {
    /// 啟動 Worker：連接 Hub，開始 polling 任務
    pub async fn start(&self) -> Result<()>;

    /// 執行單個任務
    async fn execute_task(&self, task: DistributedTask) -> Result<TaskResultPayload>;

    /// 代理 memory 操作 — 轉發到 Hub
    async fn proxy_memory_store(&self, key: &str, content: &str, category: &str) -> Result<String>;
    async fn proxy_memory_recall(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>>;
    async fn proxy_memory_forget(&self, key: &str) -> Result<bool>;

    /// 上傳任務產出檔案到 Hub
    async fn upload_files(&self, task_id: &str, dir: &Path) -> Result<Vec<OutputFile>>;

    /// 向 Hub 發送心跳
    async fn send_heartbeat(&self, report: HeartbeatReport) -> Result<()>;
}
```

### 9.5 gRPC 服務定義

```rust
/// Hub 端 gRPC 心跳上報
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatReport {
    /// Worker 節點名
    pub node_name: String,
    /// 正在執行的任務 ID (None = idle)
    pub task_id: Option<String>,
    /// 當前 agent 輪數
    pub round_number: u32,
    /// 當前正在執行的工具 (None = LLM 推理中)
    pub current_tool: Option<String>,
    /// 累計 token 使用量
    pub tokens_so_far: u32,
    /// 累計預估成本
    pub cost_so_far_usd: f64,
    /// Worker 系統資源使用
    pub system_stats: SystemStats,
    /// 時間戳
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemStats {
    /// CPU 使用率 (0-100)
    pub cpu_percent: f32,
    /// 記憶體使用 (MB)
    pub memory_used_mb: u32,
    /// GPU 使用率 (0-100, None if no GPU)
    pub gpu_percent: Option<f32>,
    /// GPU VRAM 使用 (MB, None if no GPU)
    pub gpu_vram_used_mb: Option<u32>,
}

/// Hub → Worker: E-Stop 推送
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EStopBroadcast {
    pub active: bool,
    pub reason: Option<String>,
    pub timestamp: DateTime<Utc>,
}

/// Worker → Hub: Memory 代理請求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemoryProxyRequest {
    Store {
        key: String,
        content: String,
        category: String,
        session_id: Option<String>,
    },
    Recall {
        query: String,
        limit: usize,
        session_id: Option<String>,
    },
    Get {
        key: String,
    },
    Forget {
        key: String,
    },
}

/// Hub → Worker: Memory 代理回應
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemoryProxyResponse {
    StoreOk { id: String },
    RecallResult { entries: Vec<MemoryEntry> },
    GetResult { entry: Option<MemoryEntry> },
    ForgetResult { deleted: bool },
    Error { message: String },
}
```

### 9.6 Hub 端 StateManager

```rust
/// Hub 端集中狀態管理器 — 統一管理所有持久狀態
pub struct StateManager {
    /// 語義記憶 (PostgreSQL + pgvector)
    pub memory: Arc<MemoryStore>,
    /// 成本追蹤 (SQLite)
    pub costs: Arc<CostTracker>,
    /// 營收追蹤 (SQLite)
    pub revenue: Arc<RevenueTracker>,
    /// 分散式任務佇列 (SQLite WAL)
    pub tasks: Arc<TaskScheduler>,
    /// 對話存儲
    pub conversations: Arc<ConversationStore>,
    /// Worker 節點註冊表
    pub cluster: Arc<ClusterRegistry>,
    /// E-Stop (全集群)
    pub estop: Arc<EStop>,
    /// 心跳追蹤
    pub heartbeat: Arc<Heartbeat>,
}

impl StateManager {
    /// 初始化所有狀態存儲
    pub async fn new(config: &AppConfig) -> Result<Self>;

    /// 健康檢查 — 確認所有後端可用
    pub async fn health_check(&self) -> HashMap<String, bool>;

    /// 全集群 E-Stop — 停止 Hub + 推送到所有 Worker
    pub async fn emergency_stop(&self, reason: &str) -> Result<()>;

    /// 全集群恢復
    pub async fn resume(&self) -> Result<()>;

    /// 產生集群狀態摘要 (for /status command)
    pub async fn cluster_summary(&self) -> ClusterSummary;
}

#[derive(Debug, Serialize)]
pub struct ClusterSummary {
    pub total_nodes: usize,
    pub online_nodes: usize,
    pub running_tasks: usize,
    pub queued_tasks: usize,
    pub today_tokens: u64,
    pub today_cost_usd: f64,
    pub today_revenue_usd: f64,
    pub memory_entries: usize,
    pub estop_active: bool,
}
```

---

## 10. 通訊協議

### 為什麼選 gRPC (tonic)

| 方案 | 序列化效率 | 串流支援 | Rust 生態 | 程式碼生成 |
|------|----------|---------|----------|----------|
| HTTP/JSON | 低 | 有限 | axum | 手寫 |
| gRPC/protobuf | 高 | 原生 | tonic | 自動 |
| WebSocket | 中 | 半雙工 | tokio-tungstenite | 手寫 |
| NATS | 高 | 原生 | nats.rs | 手寫 |

**選擇 gRPC (tonic)**：
- protobuf 序列化比 JSON 小 3-5x
- 原生雙向串流（heartbeat、file upload）
- `tonic` crate 是 Rust gRPC 的事實標準
- 自動產生 client/server code

### Proto 定義 (精簡版)

```protobuf
syntax = "proto3";
package clawtex.cluster;

service ClusterHub {
    // Worker 註冊
    rpc Register(RegisterRequest) returns (RegisterResponse);

    // Worker 拉取任務 (long polling)
    rpc PollTask(PollTaskRequest) returns (PollTaskResponse);

    // Worker 回報任務完成
    rpc CompleteTask(CompleteTaskRequest) returns (CompleteTaskResponse);

    // Worker 回報任務失敗
    rpc FailTask(FailTaskRequest) returns (FailTaskResponse);

    // Worker 心跳 (雙向串流)
    rpc Heartbeat(stream HeartbeatReport) returns (stream HubDirective);

    // Memory 代理
    rpc MemoryProxy(MemoryProxyRequest) returns (MemoryProxyResponse);

    // 檔案上傳 (client streaming)
    rpc UploadFile(stream FileChunk) returns (UploadResponse);

    // E-Stop 訂閱 (server streaming)
    rpc SubscribeEStop(EStopSubscription) returns (stream EStopBroadcast);
}

message HubDirective {
    oneof directive {
        // Hub 可主動取消任務
        CancelTask cancel_task = 1;
        // Hub 可主動推送設定更新
        ConfigUpdate config_update = 2;
    }
}
```

### 連線與安全

```
所有 Worker ↔ Hub 通訊透過 Tailscale (WireGuard VPN):
  - 端對端加密 (不需額外 TLS)
  - 固定 IP (100.x.y.z)
  - mTLS optional (如果需要額外認證)

gRPC 設定:
  - keepalive_interval: 10s
  - keepalive_timeout: 5s
  - max_message_size: 64MB (檔案上傳)
  - connection_timeout: 5s
```

---

## 11. 故障處理與恢復

### Worker 故障

```
情境 1: Worker 網路斷線
  → Hub 停止收到 heartbeat
  → 30s 後 HeartbeatReport timeout
  → Hub: 將該 Worker 的 running tasks 標記為 TimedOut
  → Hub: 重新調度 TimedOut 任務到其他 Worker
  → Hub: 標記該 Worker 為 Offline
  → 通知 Telegram: "Worker-3 offline, 2 tasks rescheduled"

情境 2: Worker 進程崩潰
  → 同上（Hub 透過 heartbeat 失敗偵測）
  → Worker 重啟後自動重新 Register
  → Hub: 標記為 Online，開始分配新任務

情境 3: Worker GPU 記憶體不足 (OOM)
  → Worker 捕捉到 LLM provider 錯誤
  → Worker: FailTask { error: "GPU OOM" }
  → Hub: 任務標記為 Failed，排程到 VRAM 更大的 Worker
  → 若無更大 Worker，降級為小模型重試
```

### Hub 故障

```
情境: Z13 Hub 崩潰
  → 所有 Worker 停止收到 Heartbeat response
  → Worker 進入離線緩衝模式 (buffer mode)
  → Worker 完成手上正在執行的任務，將結果暫存本地
  → Worker 每 5s 嘗試重連 Hub

Hub 恢復後:
  → Worker 自動重連 + Register
  → Worker 上傳緩衝的 TaskResult
  → Hub 掃描 distributed_tasks 表，重新調度所有 TimedOut 任務
  → Hub 恢復 Telegram 連線

資料安全:
  → SQLite WAL 模式 → 崩潰恢復不丟失已提交的交易
  → PostgreSQL → MVCC + WAL → 崩潰恢復完整
  → Worker 本地緩衝 → 確保已完成的工作不丟失
```

### E-Stop 故障安全

```
E-Stop 傳播路徑:
  使用者 → Telegram /estop → Hub EStop.stop()
  Hub → gRPC EStopBroadcast → 所有 Worker
  Worker → EStop.stop() → 停止 agent loop + 工具執行

安全保證:
  - Hub E-Stop 是 AtomicBool → 即時生效
  - Worker 每輪 agent loop 和每次工具執行前都 check()
  - 即使 gRPC 延遲，最差情況 = 當前工具執行完後停止
  - Worker 若失聯 → 任務 timeout → 自然停止

恢復:
  使用者 → /resume → Hub EStop.reset()
  Hub → gRPC EStopBroadcast(active=false) → 所有 Worker
```

---

## 12. 實施路線圖

### Phase 1: 基礎通訊 (1-2 週)

```
目標: Hub ↔ Worker 基本通訊

工作項目:
  [ ] 定義 .proto 檔 (cluster.proto)
  [ ] tonic server (Hub 端) — Register, PollTask, CompleteTask
  [ ] tonic client (Worker 端) — WorkerExecutor 基本框架
  [ ] Worker 二進位 (clawtex-worker) — 獨立 binary crate
  [ ] Heartbeat 雙向串流
  [ ] 整合 Tailscale 網路 (手動 IP 配置)

驗證:
  - Worker 註冊到 Hub
  - Hub 分配簡單任務 → Worker 執行 → 回報結果
  - Heartbeat 正常上報
```

### Phase 2: 任務調度 (1 週)

```
目標: Hand workflow 分散執行

工作項目:
  [ ] DistributedTask SQLite schema + CRUD
  [ ] TaskScheduler 調度邏輯 (priority + affinity + load balancing)
  [ ] Hand → Phase 任務拆分
  [ ] Phase 依賴鏈 (Phase 2 等 Phase 1 完成)
  [ ] chain_to 跨 Hand 觸發
  [ ] Telegram 跨節點進度顯示

驗證:
  - seo_content Hand 5 個 phase 分散在不同 Worker 上依序執行
  - lead → outreach chain 跨 Worker 正確觸發
  - Telegram 顯示每個 phase 在哪台機器
```

### Phase 3: 狀態集中化 (1 週)

```
目標: Memory + Cost + Revenue 集中到 Hub

工作項目:
  [ ] PostgreSQL + pgvector 部署 (Z13 Hub 上)
  [ ] MemoryProxy gRPC 服務
  [ ] Worker memory_store/recall/forget 工具改為 proxy 模式
  [ ] CostTracker 增加 node 欄位 + by_node() 查詢
  [ ] Worker 成本即時上報 (heartbeat 內嵌)
  [ ] /costs 命令增加 per-node 細分

驗證:
  - Worker 執行 memory_store → Hub PG 可查到
  - Worker 執行 memory_recall → 拿到 Hub PG 的結果
  - /costs 顯示每台 Worker 的 token 和成本
```

### Phase 4: 檔案傳輸 + E-Stop (1 週)

```
目標: 完整的檔案回傳 + 全集群 E-Stop

工作項目:
  [ ] gRPC FileUpload 串流服務
  [ ] Worker 任務完成後自動掃描 + 上傳產出檔案
  [ ] Hub workspace/ 按 task_id 組織
  [ ] GET /workspace/files 增加 node + task_id 過濾
  [ ] E-Stop gRPC 廣播
  [ ] Worker E-Stop 訂閱 + 即時響應
  [ ] Worker 離線緩衝 (buffer mode)

驗證:
  - content Hand 在 Worker 上產生 output.md → Hub workspace/ 可下載
  - /estop → 所有 Worker 在 2s 內停止
  - Worker 斷線 → 任務自動重新調度
```

### Phase 5: 監控 + 優化 (1 週)

```
目標: 生產就緒

工作項目:
  [ ] /cluster 命令: 顯示所有節點狀態、負載、模型
  [ ] /tasks 命令: 顯示分散式任務佇列
  [ ] Dashboard 頁面增加集群視圖
  [ ] 模型親和性自動學習 (記錄載入時間，避免頻繁切換)
  [ ] 任務 retry 策略 (exponential backoff)
  [ ] 資源感知調度 (GPU utilization 過高時不分配)
  [ ] E2E 測試: 8 機集群完整工作流

驗證:
  - 8 台機器同時執行不同 Hand
  - 1 台 Worker 故障 → 任務自動遷移
  - /cluster 顯示完整集群狀態
  - 總吞吐量達到單機的 5x+
```

---

## 附錄 A: 設定檔變更

`~/.clawtex/agents.toml` 新增：

```toml
[cluster]
# Hub 模式 (預設) 或 Worker 模式
mode = "hub"  # "hub" | "worker"
# Hub 的 gRPC 地址 (Worker 填寫)
hub_address = "100.87.93.1:50051"
# gRPC 監聽埠 (Hub)
grpc_port = 50051
# Worker 節點名 (自動偵測或手動指定)
node_name = "z13"
# 最大並行任務數 (Worker)
max_concurrent_tasks = 2
# 心跳間隔 (秒)
heartbeat_interval_secs = 10
# 任務超時 (秒, Hub 端)
task_timeout_secs = 600

[memory]
backend = "pgvector"
pg_url = "host=localhost user=clawtex dbname=clawtex"
embed_url = "http://localhost:11434"
embed_model = "nomic-embed-text"
dimensions = 768
```

## 附錄 B: 新增 Cargo 依賴

```toml
[dependencies]
# gRPC
tonic = "0.12"
prost = "0.13"
# protobuf 編譯
[build-dependencies]
tonic-build = "0.12"
```

## 附錄 C: Worker 部署腳本 (每台機器)

```bash
#!/bin/bash
# deploy-worker.sh — 在新 Worker 上部署 clawtex-worker

# 1. 安裝 Tailscale (如果沒有)
curl -fsSL https://tailscale.com/install.sh | sh
tailscale up --authkey $TAILSCALE_KEY

# 2. 安裝 Ollama
curl -fsSL https://ollama.com/install.sh | sh
ollama pull qwen3:8b

# 3. 部署 clawtex-worker 二進位
scp z13:~/clawtex-core/target/release/clawtex-worker /usr/local/bin/

# 4. 建立設定檔
mkdir -p ~/.clawtex
cat > ~/.clawtex/agents.toml << 'EOF'
[cluster]
mode = "worker"
hub_address = "100.87.93.1:50051"
node_name = "worker-$(hostname)"
max_concurrent_tasks = 2
heartbeat_interval_secs = 10
EOF

# 5. 啟動 Worker (systemd service)
cat > /etc/systemd/system/clawtex-worker.service << 'EOF'
[Unit]
Description=Clawtex Worker
After=network-online.target ollama.service
Wants=network-online.target

[Service]
ExecStart=/usr/local/bin/clawtex-worker
Restart=always
RestartSec=5
User=clawtex
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
EOF

systemctl enable --now clawtex-worker
```

---

## 附錄 D: 遷移策略 (零停機)

```
Step 1: 部署 PostgreSQL + pgvector (Hub 上)
  → 安裝 PG 15 + pgvector extension
  → 建立 clawtex database + memories table
  → 測試連線

Step 2: 遷移 SQLite memories → PostgreSQL
  → 寫遷移腳本: 讀 SQLite → 批量 INSERT PG
  → 包含 embedding blobs 的轉換

Step 3: 切換 memory backend
  → agents.toml: backend = "pgvector"
  → 重啟 clawtex-core
  → 驗證 memory_recall 正常

Step 4: 部署 Hub gRPC server
  → 在 clawtex-core binary 中新增 gRPC listener
  → 監聽 :50051
  → 與現有 HTTP :7878 共存

Step 5: 逐台部署 Worker
  → 從第 1 台開始，確認正常後部署下一台
  → 每台 Worker 上線後自動 Register
  → Telegram /cluster 確認節點狀態

Step 6: 切換任務調度
  → 新的 /hand 命令使用 TaskScheduler
  → 舊的本地 HandRunner 作為 fallback
  → 若所有 Worker 離線 → 自動回退到本地執行
```
