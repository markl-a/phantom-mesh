# 技術 Backlog（Phantom-Mesh 可實作項目）

> 來源：R001-R100 研究文件 + OpenClaws 總計畫技術底座選型 + Master Plan
> 最後更新：2026-03-14（已與現有 code 比對，標記實際狀態）
> 目的：從研究文件中萃取出**可直接應用於 phantom-mesh (Rust daemon)** 的技術模式與實作建議

---

## 使用說明

- 每項標記 **優先級**（🔴 High / 🟡 Medium / 🟢 Low）
- 每項標記 **目標模組**（phantom-mesh 中對應的 `src/` 路徑）
- 已實作的功能不再列出（如 L1 guardrail、cost_tracker 記錄、基本 cluster dispatch）
- 可作為開發 backlog 直接取用

---

## 一、集群調度與分配（Cluster Dispatch）

### 1.1 ✅ Node Health Score 複合健康分數 — **已實作（基礎版）**

| 項目 | 內容 |
|------|------|
| 描述 | 每 5 分鐘計算各節點複合分數：`Score = 0.15×CPU + 0.20×RAM + 0.30×GPU + 0.20×Disk + 0.15×Latency`（各項 0-100） |
| 門檻 | <40 全部封鎖、40-59 僅 P3、60-79 P1/P2、80+ 全開放 |
| **現況** | `cluster_worker.rs` 已用 `sysinfo` crate + 背景執行緒每 15s 採集 CPU load，存入 `AtomicU32`。**基礎 CPU 健康已解決**，但尚無 RAM/GPU/Disk 的複合計算 |
| 模組 | `src/cluster_worker.rs`（採集）、`src/cluster_hub.rs`（計算+門檻） |
| 來源 | R011-R020 |
| **狀態** | 🟡 部分完成 — 考慮後續加 RAM/Disk 維度 |

### 1.2 🔴 Task Taxonomy 任務分類調度

| 項目 | 內容 |
|------|------|
| 描述 | 結構化任務分類：`code`、`think`、`research`、`batch`、`local`、`ops`，每類定義 primary/fallback 節點、GPU/NPU 需求、延遲目標、成本上限 |
| 好處 | 從「誰有空」變成「能力匹配」的智慧調度 |
| 模組 | `src/cluster_hub.rs`、`src/cluster.rs` — 擴充 `ClusterRegistry` |
| 來源 | R001-R010 |

### 1.3 🔴 SLA 分級優先佇列

| 項目 | 內容 |
|------|------|
| 描述 | 四級 SLA：P0（即時 <30s）、P1（當日 <4h）、P2（48h）、P3（定期），各級有不同重試策略和升級規則 |
| 現況 | `TaskQueue` 僅有 pending/running/done/failed，無優先級 |
| 模組 | `src/task_queue.rs` — 增加 `priority` 欄位，依優先級 dequeue |
| 來源 | R001-R010 |

### 1.4 ⏭️ GPU Mutex 資源鎖 — **跳過**

| 項目 | 內容 |
|------|------|
| 描述 | GPU 排他/共用鎖：Coder 和 Thinker 不可同時持有 GPU（EXCLUSIVE） |
| **決策** | 跳過 — GPU 可同時載入多個小模型，不需要排他鎖 |
| 來源 | R021-R030 |

### 1.5 🟡 Concurrency Watermark 併發水位線

| 項目 | 內容 |
|------|------|
| 描述 | 每節點併發上限（Z13: 4 任務 / Acer: 3 / M1: 2 / Cloud: 5），CPU/RAM > 75% 自動暫停 P3 |
| 模組 | `src/cluster_hub.rs` — 新增 `ConcurrencyManager` |
| 來源 | R021-R030 |

### 1.6 🟡 Task Preemption 優先搶占

| 項目 | 內容 |
|------|------|
| 描述 | P0 任務可搶占 P2/P3 任務（被搶占者 checkpoint 後重入佇列） |
| 前置 | 需要 1.3 SLA 佇列 + 1.4 GPU Mutex |
| 模組 | `src/task_queue.rs`、`src/cluster_hub.rs` |
| 來源 | R021-R030 |

### 1.7 🔴 Node Capability Score 節點能力評分

| 項目 | 內容 |
|------|------|
| 描述 | 四維加權評分：穩定性 30%（30 天成功率）、速度 25%（實際 vs 基準）、成本 25%（實際 vs 基準）、品質 20%（NPS + 一致性）。A(90+) 接商業、B(75-89) 一般、C(60-74) 內部、D(<60) 停用 |
| 模組 | `src/cluster_hub.rs`、新表 `node_scores` in `core.db` |
| 來源 | R081-R090 |

### 1.8 🔴 Automated Node Onboarding 自動上線

| 項目 | 內容 |
|------|------|
| 描述 | 四步自動化：register (5min) → configure (15min) → health check (3min) → smoke test (15min)，全通過才接收任務 |
| 模組 | `src/cluster_hub.rs`（`POST /cluster/register`）、新腳本 `scripts/onboard-worker.sh` |
| 來源 | R081-R090 |

### 1.9 🟢 Schedule Windows 日夜排程窗口

| 項目 | 內容 |
|------|------|
| 描述 | 日間窗口（08-22 點，互動任務優先 Z13）vs 夜間批次窗口（22-08 點，batch 任務送 Acer/Ayaneo） |
| 模組 | `src/cron.rs`、`src/cluster_hub.rs` |
| 來源 | R001-R010 |

### 1.10 🟢 Starvation Prevention 飢餓防護

| 項目 | 內容 |
|------|------|
| 描述 | P3 任務等待超過 2 小時自動升級至 P2，防止低優先任務永遠被卡 |
| 模組 | `src/task_queue.rs` |
| 來源 | R021-R030 |

---

## 二、成本控制與財務安全（Cost Control）

### 2.1 🔴 Cost Budget Circuit Breaker 分層熔斷

| 項目 | 內容 |
|------|------|
| 描述 | 四階段：Normal(<$12/day) → Warning($12-15, 通知) → Circuit Break(>$15, 自動切 Ollama) → Emergency(>$20, 全停需手動恢復) |
| 任務級 | per-task max ($2 default, $5 for P0)、daily ($5)、weekly ($30)、monthly ($100) |
| 80% 預警 | 到達每日預算 80% 就發警告 |
| 現況 | `cost_tracker` 只記錄不執行 |
| 模組 | `src/cost_tracker.rs` — 新增 `BudgetGuard`，每次 API 呼叫前檢查 |
| 來源 | R021-R030, R071-R080 |

### 2.2 🔴 Red-Line Circuit Breaker 紅線熔斷

| 項目 | 內容 |
|------|------|
| 規則 | RL-1: 月 API 成本 > 月營收 25% → 全降 L1 local |
|       | RL-2: 2 週毛利 < 35% → 暫停接新案 |
|       | RL-3: 3 日 pipeline 成功率 < 90% → 暫停商業交付 |
|       | RL-4: 連續 2 筆負毛利 → 停用該 hand |
| 模組 | 新 `src/circuit_breaker.rs`，接 `costs.db` + `revenue.db` |
| 來源 | 技術底座選型, R071-R080 |

### 2.3 🟡 Per-Task Cost Ceiling 任務成本天花板

| 項目 | 內容 |
|------|------|
| 描述 | 調度前檢查 `cost_ceiling`（code $0.50 / think $1.00 / research $0.30），超過則降級模型或拒絕 |
| 區別 | 與 2.1 的每日預算不同，這是 per-task 前置檢查 |
| 模組 | `src/providers/router.rs`（路由前估算）、`src/cost_tracker.rs`（記錄） |
| 來源 | R041-R050 |

### 2.4 🔴 Financial Red-Line Monitor 財務紅線監控

| 項目 | 內容 |
|------|------|
| 指標 | 7 項：現金餘額（紅:<2 月）、月燒錢率（紅:>110% 預算）、API/營收比（紅:>25%）、應收帳齡（紅:>30 天）、營收達成率（紅:<70%）、專案毛利（紅:<30%）、客戶集中度（紅:>50%） |
| 告警 | 4 級：INFO → WARN → CRIT → EMERG |
| 模組 | 新 `src/financial_monitor.rs`，接 `costs.db` + `revenue.db` |
| 來源 | R071-R080 |

### 2.5 🟢 Unit Economics Tracker 單案經濟追蹤

| 項目 | 內容 |
|------|------|
| 描述 | 追蹤每案 revenue/cost/margin：`{case_id, hand_name, revenue, api_cost, compute_cost, margin_rate}`，連續 2 筆負毛利自動告警 |
| 模組 | `src/revenue_tracker.rs` + `src/cost_tracker.rs` join 查詢 |
| 來源 | R041-R050 |

---

## 三、可靠性與容錯（Reliability）

### 3.1 🔴 Idempotency Key 冪等鍵

| 項目 | 內容 |
|------|------|
| 描述 | 每次任務執行計算 `hash(task_id + date + hand_name + input_hash)`，存入 `idempotency_log` 表（7 天 TTL），執行前檢查防重複 |
| 現況 | 重試可能導致重複執行 |
| 模組 | `src/task_queue.rs` — 新增 `idempotency_key` 欄位 |
| 來源 | R011-R020 |

### 3.2 🔴 Structured Error Codes 結構化錯誤碼

| 項目 | 內容 |
|------|------|
| 描述 | 錯誤分類 enum：PATH_MISSING、ENV_MISMATCH、GPU_OOM、NETWORK_TIMEOUT、PERMISSION_DENIED、DISK_FULL、API_RATE_LIMIT、SCHEDULE_CONFLICT、UNKNOWN。每類帶 severity、auto_recoverable flag、root_cause_candidates、fix_template |
| 好處 | 下游所有功能（fallback、anti-repeat、alerting）都依賴結構化錯誤 |
| 模組 | 新 `src/error_codes.rs` |
| 來源 | R001-R010 |

### 3.3 🔴 Three-Level Fallback 三層故障轉移

| 項目 | 內容 |
|------|------|
| 描述 | L1: 本地重試 exponential backoff → L2: failover 至 backup node → L3: 人工確認後降級執行 |
| 前置 | 需要 3.2 錯誤碼 + 1.1 健康分數 |
| 模組 | `src/cluster_hub.rs` — 新增 `FallbackChain` |
| 來源 | R001-R010, 技術底座選型 |

### 3.4 🟡 Automated Failover 自動故障轉移

| 項目 | 內容 |
|------|------|
| 描述 | 主節點連續 3 次心跳失敗（30s 間隔）→ 自動選備用節點 → 同步配置 → preflight → 啟動任務 → Telegram 通知 |
| 鏈路 | Z13 → Acer → M1 → cloud-only degraded mode |
| 模組 | `src/cluster_hub.rs` — 新增 `FailoverOrchestrator` |
| 來源 | R011-R020, 技術底座選型 |

### 3.5 🔴 Incident Response 事件回應狀態機

| 項目 | 內容 |
|------|------|
| 描述 | P0-P3 事件分級，SLA 驅動回應：P0(5min 回應, 15min MTTR)、P1(15min, 30min)、P2(1h, 4h)、P3(24h, 3d) |
| 狀態流 | Detect → Classify → Notify → Respond → RCA → Fix → Verify → Recovery → Postmortem(48h) |
| 模組 | 新 `src/incident.rs`，接 Telegram 通知 |
| 來源 | R071-R080 |

### 3.6 🟡 Automated Rollback 自動回滾

| 項目 | 內容 |
|------|------|
| 觸發 | 全回滾（P0 未解 30min）、部分回滾（功能錯誤率 >10%）、流量回滾（效能降 >50%） |
| 目標 | 回滾 <10min + 健康檢查 <5min + 通知 <15min = 總計 <20min |
| 模組 | 新 `src/rollback.rs` |
| 來源 | R091-R100 |

---

## 四、Hands 工作流增強（Workflow）

### 4.1 🔴 Preflight / Post-Capture 三階段工作流

| 項目 | 內容 |
|------|------|
| 描述 | 每個 Hand 自動增加三階段：preflight（路徑存在、模型可用、API key 有效、磁碟空間）→ execution → post-capture（存結果、記指標、萃取知識） |
| 格式 | hand.toml 新增 `[preflight]` 和 `[capture]` 區段 |
| 模組 | `src/hands/mod.rs`（或新 `src/preflight.rs`） |
| 來源 | R001-R010, 技術底座選型 |

### 4.2 🔴 Pipeline Health Metrics 管線健康指標

| 項目 | 內容 |
|------|------|
| 描述 | 記錄每次 Hand 執行結果到 `pipeline_runs` 表：hand_name、start/end_time、status、error_message、total_cost、node_id。計算 24h/7d/30d 滾動成功率 |
| 紅線 | 3 日滾動成功率 <90% 觸發 RL-3 告警 |
| 模組 | `src/hands/mod.rs`（插裝）、`core.db` 新表、`GET /metrics/pipeline` |
| 來源 | 技術底座選型, R091-R100 |

### 4.3 🟢 Hand Registry 版本化註冊表

| 項目 | 內容 |
|------|------|
| 描述 | 追蹤每個 Hand 的版本、相依、排程、超時、健康門檻。連續 2 次失敗自動回滾至 `_previous` 版本 |
| 模組 | 新 `src/hands/registry.rs`，SQLite 版本表 |
| 來源 | R011-R020 |

### 4.4 🟡 Structured Agent Report 結構化報告

| 項目 | 內容 |
|------|------|
| 描述 | 每次 Agent 輸出符合 schema：`task_id`、`agent`、`duration`、`status`、`conclusion`、`evidence[]`、`risks[]`、`next_steps[]`、`cost{}`、`confidence_score`、`tags[]`。不符格式自動轉換或人工審核 |
| 模組 | `src/agent_runtime.rs` — 定義 `AgentReport` struct |
| 來源 | R021-R030 |

---

## 五、知識與記憶系統（Knowledge）

### 5.1 🟡 Knowledge Before/After Hooks 知識前後鉤子

| 項目 | 內容 |
|------|------|
| 描述 | 任務前自動 recall 相似案例和已知陷阱注入 context；任務後自動萃取知識節點（problem, decision, result, lesson） |
| 現況 | memory_recall 需手動呼叫 |
| 模組 | `src/agent_runtime.rs`（`pre_task_recall()` / `post_task_capture()`） |
| 來源 | R001-R010, 技術底座選型 |

### 5.2 🟡 Context Pack Generator 上下文包產生器

| 項目 | 內容 |
|------|------|
| 描述 | P0/P1 任務前自動產生 Context Pack：向量化任務描述 → semantic search top-10（閾值 >0.7）→ 查錯誤歷史 → 匹配模板 → 組成 Markdown 注入 system prompt。快取 7 天 |
| 模組 | 新 `src/knowledge/context_pack.rs` |
| 來源 | R031-R040 |

### 5.3 🟡 Knowledge Graph 知識圖譜

| 項目 | 內容 |
|------|------|
| 描述 | 帶型別邊的知識圖：`causes`、`solves`、`alternative_to`、`depends_on`、`conflicts_with`、`supersedes`、`similar_to`。SQLite 鄰接表，3 層遍歷 <200ms |
| 模組 | 新 `src/knowledge/graph.rs` |
| 來源 | R031-R040 |

### 5.4 🟡 Anti-Repeat Rules 防重複規則引擎

| 項目 | 內容 |
|------|------|
| 描述 | 同類錯誤 2 次 → 建議建立 preflight 規則；3 次 → 自動草擬規則待人核准；誤報率 >10% 自動降級為 WARN |
| 模組 | 新 `src/knowledge/anti_repeat.rs` 或擴充 `src/guardrail.rs` |
| 來源 | R031-R040 |

### 5.5 🟢 Knowledge Value Scoring 知識價值評分

| 項目 | 內容 |
|------|------|
| 描述 | `ValueScore = 0.30×Reuse + 0.25×TimeSave + 0.20×Revenue + 0.15×Recency + 0.10×Quality`。前 20%（80+）優先模板化，<20 且 30+ 天清理 |
| 模組 | 新 `src/knowledge/scorer.rs` |
| 來源 | R031-R040 |

### 5.6 🟢 Data Lifecycle 資料生命週期

| 項目 | 內容 |
|------|------|
| 描述 | Hot/Warm/Cold 分層：知識 30d/180d/180d+，快取 7d/30d 刪除，日誌 14d/90d 歸檔/365d 刪除。排程清理 |
| 模組 | `src/cron.rs` + 新 `src/data_lifecycle.rs` |
| 來源 | R031-R040 |

---

## 六、安全與治理（Security & Governance）

### 6.1 🔴 Three-Layer Governance 三層治理規則

| 項目 | 內容 |
|------|------|
| 描述 | L1 硬規則（不可違反：不調度至不健康節點、無 preflight 不執行、超預算 non-P0 停止）、L2 策略規則（可調權重：GPU 親和、互斥 w=0.9）、L3 偏好規則（可覆蓋：local-first、小模型優先） |
| 模組 | 新 `src/governance.rs` 或擴充 `src/cluster_hub.rs` |
| 來源 | R021-R030 |

### 6.2 🔴 RBAC + Audit Log 角色權限 + 審計

| 項目 | 內容 |
|------|------|
| 角色 | Admin / Operator / Viewer |
| 權限矩陣 | 配置變更、用戶管理、任務排程、任務執行、報告查看、日誌查看、API Key 管理、成本查看、資料匯出、上線部署 |
| 審計 | 每筆敏感操作記 `audit_log` 表：timestamp、actor、role、action、resource、result、risk_level。保留 ≥90 天 |
| 模組 | 新 `src/rbac.rs`、`core.db` 新 `audit_log` 表 |
| 來源 | R071-R080 |

### 6.3 🟡 Data Classification Gate 資料分級閘門

| 項目 | 內容 |
|------|------|
| 分級 | L1-PUBLIC（無限制）、L2-INTERNAL（不公開）、L3-CONFIDENTIAL（加密存儲、匿名化後才送 cloud）、L4-RESTRICTED（僅本地處理、完整審計） |
| 強制 | L4 資料不可到達任何外部 provider |
| 模組 | 新 `src/data_classification.rs`，作為 provider pipeline pre-send hook |
| 來源 | R001-R010, R071-R080 |

### 6.4 🟡 Tiered Approval 分級審批

| 項目 | 內容 |
|------|------|
| 描述 | 擴充現有 `approval.rs`：外部發布(12h)、合約簽署(24h, 不可繞過)、生產部署(4h, P0 可先部署後審批)、付款 >NT$5000(12h)、L3+ 資料匯出(4h) |
| 模組 | 擴充 `src/approval.rs` |
| 來源 | R071-R080 |

### 6.5 🟡 Prompt Injection Guard 提示注入防護

| 項目 | 內容 |
|------|------|
| 描述 | 所有外部輸入（Telegram、HTTP API、Hand 參數）在注入 LLM prompt 前通過清洗層，過濾 system prompt override、角色切換、指令分隔符 |
| 現況 | L1 guardrail 已存在但僅檢查輸出，需擴展至輸入面 |
| 模組 | `src/guardrail.rs`（擴充）、`src/telegram.rs`、HTTP handler |
| 來源 | 技術底座選型 |

---

## 七、監控與報告（Monitoring）

### 7.1 🟡 Structured Alert Payloads 結構化告警

| 項目 | 內容 |
|------|------|
| 描述 | 告警包含：`alert_id`、`severity`、`source_node`、`hand`、`error_code`、`root_cause_candidates[]`、`suggested_fix[]`、`next_action`、`escalation_deadline`、`related_runbook` |
| 現況 | Telegram 通知為純文字 |
| 模組 | `src/telegram.rs`、新 `src/alerting.rs` — 定義 `AlertPayload` struct |
| 來源 | R011-R020 |

### 7.2 🟡 SLO + Error Budget 服務水平目標

| 項目 | 內容 |
|------|------|
| SLO | 月可用性 ≥99%、P0 延遲 <5min、P1 P50 <10min、preflight 通過率 ≥95% |
| Error Budget | 1% = ~7.2h/月。<25% 凍結新功能，0% 停止所有變更 |
| 模組 | 新 `src/slo.rs` |
| 來源 | R011-R020 |

### 7.3 🟡 Dispatch Audit Log 調度審計日誌

| 項目 | 內容 |
|------|------|
| 描述 | 每次調度決策記錄：`dispatch_id`、`task_id`、`task_type`、`priority`、`selected_node`、`health_score`、`decision_reason`、`alternatives_considered[]`、`cost_estimate` |
| 模組 | `src/cluster_hub.rs` — `core.db` 新 `dispatch_log` 表 |
| 來源 | R021-R030 |

### 7.4 🔴 Auto Operations Report 自動營運報告

| 項目 | 內容 |
|------|------|
| 描述 | 每日 08:00 + 每週一 09:00 自動產生摘要送 Telegram：節點狀態、任務統計、API 成本（日 + MTD vs 預算）、營收、活躍專案、待處理 leads、告警狀態 |
| 模組 | 新 Hand `~/.phantom-mesh/hands/ops_report/hand.toml`，利用現有 cron |
| 來源 | R081-R090 |

### 7.5 🟡 Real-Time Metrics API 即時指標 API

| 項目 | 內容 |
|------|------|
| 端點 | `GET /api/metrics/health`（節點矩陣）、`/tasks`（完成率）、`/costs`（日成本）、`/revenue`（月營收）、`/customers`（NPS）、`/pipeline`（漏斗） |
| 延遲 | 即時 <1min、批次 <1h |
| 模組 | daemon HTTP handler 新增端點，聚合現有 DB |
| 來源 | R081-R090 |

---

## 八、模型路由增強（Model Routing）

### 8.1 🔴 Three-Layer Model Hierarchy L1/L2/L3

| 項目 | 內容 |
|------|------|
| 描述 | L1=本地 Ollama（預設）、L2=便宜雲端（本地超載/模型不可用時）、L3=高級雲端（高價值決策+明確審批）。超成本天花板自動降級 L3→L2→L1 |
| 現況 | router 有 provider 路由但無明確成本驅動降級 |
| 模組 | `src/providers/router.rs`、`src/providers/rotation.rs` |
| 來源 | 技術底座選型 |

### 8.2 🟡 Model Tier Selection Rules Engine

| 項目 | 內容 |
|------|------|
| 描述 | 規則引擎同時考慮 4 維度：task_type + priority + cost_ceiling + data_sensitivity → 選定模型層級 |
| 模組 | `src/providers/router.rs` — 新增 `ModelTierSelector` |
| 來源 | R001-R010 |

---

## 九、發布與部署（Release）

### 9.1 🟡 Canary / Gradual Rollout 灰度發布

| 項目 | 內容 |
|------|------|
| 描述 | 四階段：Canary(5%, 3d) → Beta(20%, 7d) → Limited GA(50%, 14d) → GA(100%)。每階段有錯誤率/P95/成功率門檻，超標自動回滾 |
| 模組 | `src/providers/router.rs` 或新 `src/release/` |
| 來源 | R091-R100 |

### 9.2 🟡 Load Testing Framework 壓力測試

| 項目 | 內容 |
|------|------|
| 描述 | daemon 內建壓測模式：baseline(1x, 5 併發)、stress(2x, 10)、extreme(3x, 15)、node-failure(殺 1 節點)、API-delay(3x 延遲注入)、24h 穩定性（偵測 memory leak） |
| 模組 | 新 `src/testing/` 或 CLI subcommand |
| 來源 | R091-R100 |

### 9.3 🟡 Pre-Launch Issue Gate 上線前問題閘門

| 項目 | 內容 |
|------|------|
| 描述 | Go-Live 條件：P0=0、P1=0、P2 有修復時程 <30d、P3 進正常迭代。程式化阻擋直到條件達成 |
| 模組 | 部署流程 hook 或 Hand runner gate |
| 來源 | R091-R100 |

---

## 十、客戶與營運（Customer Ops）

### 10.1 🟡 Customer Health Score 客戶健康分數

| 項目 | 內容 |
|------|------|
| 描述 | 加權 0-100：效率提升 30% + 品質提升 25% + 速度提升 25% + 滿意度 20%。Healthy(80+) / Watch(60-79) / Risk(40-59, 升級) / Danger(<40, 立即介入) |
| 模組 | 新 `src/customer_health.rs`、`core.db` 新 `customer_health` 表 |
| 來源 | R081-R090 |

### 10.2 🟡 Churn Risk Detector 流失預警

| 項目 | 內容 |
|------|------|
| 信號 | 2 週未使用（中風險）、NPS<6（高）、3+ 投訴（高）、續約未回覆（中）、降級請求（中） |
| 動作 | 60/30/14 天續約提醒 |
| 模組 | 新 `src/customer_health.rs`、cron job |
| 來源 | R061-R070 |

### 10.3 🟡 Service Tier Enforcement 服務級別執行

| 項目 | 內容 |
|------|------|
| 描述 | Lite(10 tasks/mo, basic model, 95% SLA) / Pro(50, advanced, 99%) / Team(unlimited, best, 99.5%)。80% 用量通知，100% 阻擋 |
| 模組 | `src/providers/router.rs`（模型分級）、新 `src/billing/`（用量追蹤） |
| 來源 | R081-R090 |

---

## 十一、其他技術模式（Misc Patterns）

### 11.1 🟡 Confidence Scoring 信心評分

| 項目 | 內容 |
|------|------|
| 描述 | 多 Agent 衝突時：比對結論相似度，<50% 觸發仲裁，高信心者勝（歷史準確率 + 推理深度），差距 <20% 升級人工 |
| 模組 | 新 `src/arbitration.rs` 或擴充 `src/evaluate.rs` |
| 來源 | R021-R030 |

### 11.2 🟡 Adapter Trait Pattern 介面抽象

| 項目 | 內容 |
|------|------|
| 描述 | 每個外部依賴通過 trait 存取（如 `MemoryBackend` trait、`Scheduler` trait），可無痛切換後端。Provider trait 已遵循，記憶和排程尚未 |
| 模組 | `src/memory.rs`（`MemoryBackend` trait）、`src/cron.rs`（`Scheduler` trait） |
| 來源 | 技術底座選型 |

### 11.3 🟢 Dual-Write Migration 雙寫遷移

| 項目 | 內容 |
|------|------|
| 描述 | 後端遷移時雙寫 1-2 週，比對輸出一致性，歸零後才切換。通用 `DualWriteAdapter<T: Backend>` |
| 模組 | 通用 wrapper，適用於任何 adapter 實作 |
| 來源 | 技術底座選型 |

### 11.4 🟡 Cross-Device Consistency 跨設備一致性

| 項目 | 內容 |
|------|------|
| 描述 | 相同 prompt+seed 送多節點比對輸出：文字生成語義相似度 ≥90%、資料處理完全匹配、延遲差異 <20%。低於 90% 限制商業任務 |
| 模組 | `src/cluster/` 或新 `src/consistency/` |
| 來源 | R081-R090 |

---

## 實施優先級摘要（2026-03-14 比對後修訂）

### 已確認要做（8 項，按實作順序）

| 順序 | 項目 | 改動位置 | 工作量 |
|------|------|---------|--------|
| 1 | **SLA Priority Queue** (1.3) | `src/task_queue.rs` 加 `priority` 欄位 + 按優先級 dequeue | 小 |
| 2 | **Idempotency Key** (3.1) | `src/task_queue.rs` 加 idempotency 查重 | 小 |
| 3 | **Cost Budget 自動執行** (2.1) | `src/agent_runtime.rs` LLM call 前自動 check_budget | 小 |
| 4 | **Structured Error Codes** (3.2) | 新 `src/error_codes.rs`，先在 cluster_hub + task_queue 使用 | 中 |
| 5 | **Pipeline Health Metrics** (4.2) | `src/hands/mod.rs` 執行結果寫 `pipeline_runs` 表 | 小 |
| 6 | **Lightweight Preflight** (4.1) | `src/hands/mod.rs` 加 provider alive + disk check | 小 |
| 7 | **L1/L2/L3 Budget Downgrade** (8.1) | `src/providers/router.rs` 預算不夠時自動切 local | 中 |
| 8 | **Post-task Knowledge Capture** (5.1) | `src/agent_runtime.rs` 執行後自動存結果到 memory（控量） | 中 |

### 已確認跳過

| 項目 | 原因 |
|------|------|
| GPU Mutex (1.4) | GPU 可同時載入多個小模型，不需排他鎖 |
| Tool Routing 設定化 (1.9 補充) | 不常加新 tool，維持 const 陣列 |

### 已經實作（研究文件資訊過時）

| 研究建議 | 現有實作 | 狀態 |
|----------|---------|------|
| Node Health Score | `sysinfo` crate + 背景 CPU 採集 | ✅ 基礎版完成 |
| RBAC 角色權限 | `security/roles.rs` — Owner/Admin/Operator/Viewer + 工具權限矩陣 | ✅ |
| 資料分級 | `security/privacy.rs` — 4 級 + 14 regex + 依級別路由 provider | ✅ |
| L1 Guardrail | `guardrail.rs` — 6 項檢查含簡體中文偵測 | ✅ |
| L2 LLM-as-Judge | `evaluate.rs` — 1-5 分 + retry | ✅ |
| Provider Fallback | `rotation.rs` — exponential backoff + cooldown + priority | ✅ |
| Smart Routing | `router.rs` — Simple/Medium/Complex 三層分類 | ✅ |
| Memory 知識系統 | `memory/` — SQLite + pgvector + 向量 + keyword + RRF 融合 | ✅ |
| 審批閘門 | `approval.rs` — Telegram 人工審批 | ✅ |
| Pre-task Knowledge Recall | `agent_runtime.rs` — 自動注入 top-5 相關 memory 到 system prompt | ✅ |
| MemoryBackend Trait | `memory/mod.rs` — `MemoryBackend` async trait + 2 實作 | ✅ |

### 後續可選（不急）

其餘 🟡 和 🟢 項目按需安排。Three-Level Fallback、Governance Rules 等列為後續候選。

---

> 本文件為技術 backlog 參考，不包含商業策略、市場分析或定價內容。
> 商業面請參見研究目錄中的 `01-戰略總覽.md`、`02-商業模型與定價.md`。
