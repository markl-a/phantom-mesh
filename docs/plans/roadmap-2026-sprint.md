# Phantom Mesh Sprint Roadmap: 3/16 → 3/31

> 時區：台灣時間 (UTC+8)
> Deadline: 3/27 全功能完成，3/31 商品化上架
> 並行開發：7 Sessions × 3 台機器

---

## 一、並行架構與衝突分析

### 7 Session 分工

| Session | 機器 | 角色 | 建 branch 名 |
|---------|------|------|-------------|
| **A (Main)** | Z13 | 整合 + 收入路線 + merge 統籌 | `main` (直接在 main 上) |
| **B (Tools)** | Z13 | 新 tool .rs 開發 | `feat/new-tools` |
| **C (Cluster)** | Z13 | 集群/排程/dispatch | `feat/cluster-enhance` |
| **D (Infra)** | Acer | 成本/監控/報告 | `feat/infra-metrics` |
| **E (Knowledge)** | Z13 | 知識/記憶/上下文 | `feat/knowledge-system` |
| **F (Security)** | AYANEO | 安全/治理/企業 | `feat/security-enterprise` |
| **G (Hands)** | Z13 | Hand TOML 開發 | `feat/new-hands` (或不需要 branch) |

### 檔案衝突分析

| 共用檔案 | 誰碰 | 解法 |
|---------|------|------|
| `src/tools/mod.rs` | A（註冊）+ B（新模組宣告） | **B 只建 .rs 檔，A 統一加 `pub mod`** |
| `src/main.rs` | A（tool 註冊 + API 路由） | **只有 A 碰 main.rs** |
| `src/lib.rs` | A（pub use 匯出） | **只有 A 碰 lib.rs** |
| `src/agent_runtime.rs` | E（context inject）+ F（validation） | **E 加 pre-execute hook，F 加 post-execute hook — 不同函數** |
| `src/cluster_hub.rs` | C（主要）| **只有 C 碰** |
| `src/approval.rs` | F（tiered approval） | **只有 F 碰** |
| `Cargo.toml` | B/D/E 可能加依賴 | **各 session 加在不同行，merge 時手動解** |
| `~/.phantom-mesh/hands/*.toml` | G（主要）+ A（微調） | **G 建新 hand，A 只調現有** |

### 完全不衝突的組合
- **B ↔ G**：B 寫 Rust tool，G 寫 TOML hand — 零交集
- **C ↔ D**：C 改 cluster，D 改 metrics — 不同檔案
- **E ↔ F**：E 改 knowledge，F 改 security — 不同檔案
- **G ↔ 所有**：G 完全不碰 Rust 代碼

### Merge 協議
```
每天 22:00（Session A 執行）：
  1. git fetch --all
  2. git merge feat/new-tools       # B 的新 tools
  3. git merge feat/cluster-enhance  # C 的集群改進
  4. git merge feat/infra-metrics    # D 的監控/成本
  5. git merge feat/knowledge-system # E 的知識系統
  6. git merge feat/security-enterprise # F 的安全/企業
  7. 手動解衝突（主要在 Cargo.toml）
  8. 在 src/tools/mod.rs 加 pub mod xxx
  9. 在 src/main.rs 加 registry.register(...)
  10. cargo test
  11. git push main
  12. 重啟 daemon
```

---

## 二、Session B — 新 Tool 實作計畫

### 每個 tool 的實作模式（統一 pattern）

所有新 tool 都參考 `ai_code.rs` 的模式：
- struct XXXTool + Tool trait impl
- `tokio::process::Command` subprocess（外部工具）或 `reqwest` HTTP（API 服務）
- 60-120s timeout
- 輸出截斷 8000 bytes
- Unit tests: 缺參數、未安裝、命令構建、輸出截斷

### D0: `image_generate` tool — `src/tools/image_generate.rs`

```
用途：AI 圖片生成（DALL-E 3, Stable Diffusion, Gemini Imagen）
模式：HTTP API 呼叫

參數：
  - action: "generate"
  - prompt: 圖片描述
  - provider: "dalle3" | "sd" | "gemini" (default: auto)
  - size: "1024x1024" | "1792x1024" | etc.
  - output_path: 儲存路徑（workspace 內）

實作：
  - DALL-E 3: POST https://api.openai.com/v1/images/generations
    Headers: Authorization: Bearer $OPENAI_API_KEY
    Body: { model: "dall-e-3", prompt, size, n: 1 }
    Response: data[0].url → reqwest download → save to output_path
  - Stable Diffusion (本地): POST http://127.0.0.1:7860/sdapi/v1/txt2img
    Body: { prompt, width, height, steps: 20 }
    Response: images[0] (base64) → decode → save
  - Gemini Imagen: POST https://generativelanguage.googleapis.com/v1/models/imagen-3.0-generate-001
    Headers: x-goog-api-key: $GEMINI_API_KEY
    Body: { instances: [{ prompt }], parameters: { sampleCount: 1 } }

依賴：reqwest (已有), base64 (需加 Cargo.toml)
測試：test_missing_prompt, test_invalid_provider, test_output_path_in_workspace
```

### D1: `docx_export` tool — `src/tools/docx_export.rs`

```
用途：Markdown → Word 文件
模式：subprocess (pandoc)

參數：
  - content: Markdown 文字（或 input_path）
  - output_path: .docx 儲存路徑
  - title: 文件標題
  - template: 可選 Word 模板路徑

實作：
  1. 檢查 pandoc 是否安裝（where/which pandoc）
  2. 將 content 寫入暫存 .md 檔
  3. pandoc input.md -o output.docx --metadata title="..."
  4. 如有 template: --reference-doc=template.docx
  5. 回傳成功 + 檔案路徑

依賴：pandoc (外部, pip install pandoc 或系統安裝)
測試：test_pandoc_not_installed, test_missing_content, test_command_construction
```

### D1: `xlsx_export` tool — `src/tools/xlsx_export.rs`

```
用途：結構化資料 → Excel
模式：subprocess (python -c "import openpyxl...")

參數：
  - data: JSON array of objects [{"col1": "val", "col2": 123}, ...]
  - output_path: .xlsx 儲存路徑
  - sheet_name: 工作表名稱 (default: "Sheet1")
  - headers: 可選自訂欄位順序

實作：
  1. 將 data JSON 寫入暫存 .json
  2. 執行 Python script:
     python -c "
     import json, openpyxl
     data = json.load(open('temp.json'))
     wb = openpyxl.Workbook()
     ws = wb.active
     ws.title = 'Sheet1'
     headers = list(data[0].keys())
     ws.append(headers)
     for row in data: ws.append([row.get(h) for h in headers])
     wb.save('output.xlsx')
     "
  3. 回傳成功 + 檔案路徑

依賴：python + openpyxl (pip install openpyxl)
測試：test_python_not_installed, test_missing_data, test_empty_data
```

### D2: `tts` tool — `src/tools/tts.rs`

```
用途：文字轉語音
模式：subprocess (edge-tts) + HTTP API (ElevenLabs)

參數：
  - text: 要轉換的文字
  - provider: "edge" | "elevenlabs" (default: "edge")
  - voice: 語音名稱 (default: "zh-TW-HsiaoChenNeural")
  - output_path: .mp3 儲存路徑

實作：
  - Edge TTS (免費):
    edge-tts --text "..." --voice "zh-TW-HsiaoChenNeural" --write-media output.mp3
  - ElevenLabs:
    POST https://api.elevenlabs.io/v1/text-to-speech/{voice_id}
    Headers: xi-api-key: $ELEVENLABS_API_KEY
    Body: { text, model_id: "eleven_multilingual_v2" }
    Response: audio stream → save to file

依賴：edge-tts (pip install edge-tts), elevenlabs API key 可選
測試：test_edge_tts_not_installed, test_missing_text, test_command_construction
```

### D2: `email_receive` tool — `src/tools/email_receive.rs`

```
用途：IMAP 收信 + 回信追蹤
模式：直接 Rust IMAP client

參數：
  - action: "check" | "read" | "search"
  - folder: "INBOX" (default)
  - query: 搜尋條件（for action=search）
  - message_id: 訊息 ID（for action=read）
  - limit: 最多回傳幾封 (default: 10)

實作：
  1. 從 config 讀取 IMAP 設定 (host, port, username, password)
  2. 用 imap crate 連線
  3. check: LIST folders + UNSEEN count
  4. read: FETCH message by UID → parse headers + body
  5. search: SEARCH SUBJECT/FROM/SINCE → return matching UIDs

依賴：imap = "3", native-tls 或 rustls (Cargo.toml)
測試：test_missing_config, test_invalid_action, test_search_construction
```

### D3: `video_compose` tool — `src/tools/video_compose.rs`

```
用途：ffmpeg 影片合成
模式：subprocess (ffmpeg)

參數：
  - action: "compose" | "convert" | "merge" | "add_audio"
  - inputs: 輸入檔案列表 ["img1.png", "img2.png", ...]
  - audio: 音軌路徑（for add_audio）
  - output_path: 輸出影片路徑
  - fps: 幀率 (default: 24)
  - duration: 每張圖持續秒數 (for compose)
  - codec: 編碼器 (default: "libx264")

實作：
  - compose (圖片→影片):
    ffmpeg -framerate 1/{duration} -i img%d.png -c:v libx264 -pix_fmt yuv420p output.mp4
  - convert:
    ffmpeg -i input.mp4 -c:v {codec} output.{ext}
  - merge:
    ffmpeg -i "concat:part1.mp4|part2.mp4" -c copy output.mp4
  - add_audio:
    ffmpeg -i video.mp4 -i audio.mp3 -c:v copy -c:a aac output.mp4

依賴：ffmpeg (外部, 系統安裝)
測試：test_ffmpeg_not_installed, test_missing_inputs, test_compose_command, test_convert_command
```

### D3: `youtube_upload` tool — `src/tools/youtube_upload.rs`

```
用途：YouTube 影片上傳
模式：subprocess (Python youtube-upload 或 Google API client)

參數：
  - video_path: 影片檔路徑
  - title: 影片標題
  - description: 影片描述
  - tags: 標籤列表
  - category: YouTube 分類 ID (default: "22" = People & Blogs)
  - privacy: "public" | "unlisted" | "private" (default: "private")

實作：
  1. 檢查 OAuth 認證（~/.phantom-mesh/youtube_oauth.json）
  2. 使用 Python google-api-python-client:
     python -c "
     from googleapiclient.discovery import build
     from google.oauth2.credentials import Credentials
     creds = Credentials.from_authorized_user_file('oauth.json')
     youtube = build('youtube', 'v3', credentials=creds)
     request = youtube.videos().insert(
       part='snippet,status',
       body={snippet: {title, description, tags, categoryId}, status: {privacyStatus}},
       media_body=MediaFileUpload(video_path)
     )
     response = request.execute()
     print(json.dumps(response))
     "

依賴：google-api-python-client, google-auth (pip install)
測試：test_missing_oauth, test_missing_video, test_command_construction
```

### D4: `music_generate` tool — `src/tools/music_generate.rs`

```
用途：AI 音樂生成
模式：HTTP API (Suno/Udio)

參數：
  - prompt: 音樂描述/歌詞
  - provider: "suno" | "udio" (default: "suno")
  - style: 音樂風格
  - duration: 秒數
  - output_path: 儲存路徑

實作：
  - Suno API:
    POST https://api.suno.ai/v1/generate
    Headers: Authorization: Bearer $SUNO_API_KEY
    Body: { prompt, style, duration }
    → poll status → download audio → save
  - Udio API:
    類似流程

依賴：reqwest (已有)
測試：test_missing_prompt, test_invalid_provider, test_no_api_key
```

### D4: Knowledge Base Import — `src/tools/knowledge_import.rs`

```
用途：批次匯入知識到 memory.db
模式：直接 Rust (SQLite)

參數：
  - action: "import_file" | "import_json" | "import_csv"
  - source: 檔案路徑或 JSON array
  - category: 記憶分類
  - session_id: 可選 session 標記

實作：
  1. 讀取來源（.json / .csv / .txt）
  2. 每筆資料 → MemoryStore::store()
  3. 回傳匯入筆數 + 失敗筆數

依賴：csv crate (Cargo.toml)
測試：test_missing_source, test_invalid_format, test_import_count
```

### D5: LinkedIn + Search Console + Engagement Tracking

```
linkedin.rs — HTTP API (LinkedIn Marketing API)
  POST https://api.linkedin.com/v2/ugcPosts
  需要 OAuth2 token

search_console.rs — HTTP API (Google Search Console API)
  GET https://www.googleapis.com/webmasters/v3/sites/.../searchAnalytics/query
  需要 OAuth2 token

engagement_tracking.rs — HTTP API (Twitter API v2)
  GET https://api.twitter.com/2/tweets/:id?tweet.fields=public_metrics
  需要 Bearer token
```

---

## 三、Session C — 集群改進實作計畫

### 所有改動集中在 `src/cluster_hub.rs` + 新檔案

### D0: SLA Priority Queue

```
檔案：src/cluster_hub.rs (修改 PendingTask + dispatch 邏輯)

實作：
  1. PendingTask 加 priority: Priority 欄位
     enum Priority { P0, P1, P2, P3 }
  2. pending_tasks 改用 BinaryHeap<PriorityTask>（高優先先出）
  3. dispatch_tool() 先檢查 priority → P0 直接 dispatch，P3 可被搶佔
  4. Hand 設定：hand.toml 加 priority = "P1"

測試：test_priority_ordering, test_p0_preempts_p3
```

### D0: Idempotency Key

```
檔案：新建 src/idempotency.rs

實作：
  1. idempotency_log 表（key TEXT UNIQUE, result TEXT, created_at, expires_at）
  2. key = SHA256(tool_name + args JSON + timestamp 去秒)
  3. dispatch 前查 key → 已存在直接回傳快取結果
  4. 執行後存 key + result
  5. 7 天 TTL，定期清理

測試：test_dedup_same_task, test_different_args_not_dedup, test_expiry
```

### D0: Task Taxonomy

```
檔案：新建 src/task_taxonomy.rs

實作：
  enum TaskType { Code, Think, Research, Batch, Local, Ops }
  fn classify(tool_name: &str, args: &Value) -> TaskType
  — shell/ai_code → Code
  — delegate → Think
  — web_search/http_request → Research
  — 多個同時 → Batch
  — file_*/memory_* → Local
  — cron/admin → Ops

測試：test_classify_shell, test_classify_web_search
```

### D1: Multi-Dimensional Worker Scorer

```
檔案：src/cluster_hub.rs (替換 best_worker_for)

實作：
  score = w1*cpu_load + w2*capability_match + w3*success_rate + w4*latency + w5*device_affinity
  weights: cpu=0.25, capability=0.30, success=0.20, latency=0.15, affinity=0.10

  struct WorkerScore { node_id, score: f64, breakdown: HashMap<String, f64> }
  fn score_worker(node: &ClusterNode, task: &TaskType) -> WorkerScore

測試：test_scoring_weights, test_high_load_low_score, test_capability_boost
```

### D1-D2: TaskQueue + WorkerPool + Proactive Scheduling

```
檔案：新建 src/task_scheduler.rs

實作：
  struct TaskQueue { queue: BinaryHeap<PriorityTask>, active: HashMap<String, ActiveTask> }
  struct WorkerPool { semaphores: HashMap<String, Semaphore> }
  — full worker: semaphore(3), npu: semaphore(2), light: semaphore(5)

  Scheduler loop (每 5 秒):
    1. 找 idle worker (semaphore available)
    2. 從 queue dequeue 最高優先 + 能力匹配的任務
    3. dispatch

測試：test_semaphore_limits, test_priority_dequeue, test_idle_detection
```

### D2-D3: 其餘集群項目

```
Task Preemption: P0 task 可中斷 P2/P3 → checkpoint 進行中任務 → re-queue
Node Capability Score: 4 維加權 (stability 30%, speed 25%, cost 25%, quality 20%)
SoT Cross-Node: skeleton.rs 的 expand 改為 dispatch_batch 跨 worker
Hand Phase Node Affinity: hand.toml phase 加 node_affinity = "light" | "full" | "hub"
Schedule Windows: 08-22 interactive priority, 22-08 batch priority
Starvation Prevention: P3 等待 >2h → auto-upgrade to P2
Node Onboarding: register → configure → health check → smoke test (4 步)
Worker Auto-Deploy: SSH + SCP 自動部署 Python worker
Hand Cross-Node Scheduling: Phase 可指定在不同 node 執行
Worker Quality Tracking: per-worker last 100 scores → avg → auto demote/promote
```

---

## 四、Session D — 基礎設施實作計畫

### 所有改動以新檔案為主，不碰核心邏輯

### D0: Pipeline Health Metrics — `src/pipeline_metrics.rs`

```
實作：
  1. pipeline_runs 表 (hand_name, started_at, ended_at, status, phases_completed, error)
  2. HandRunner 執行完後自動寫入
  3. API: GET /pipeline/health → 24h/7d/30d success rate
  4. RL-3 alert: success_rate < 90% → Telegram 告警

依賴：rusqlite (已有)
```

### D0: Structured Error Codes — `src/error_codes.rs`

```
實作：
  enum ErrorCode {
    PathMissing(E001), GpuOom(E002), NetworkTimeout(E003),
    ProviderRateLimit(E004), ToolNotInstalled(E005), ...
  }
  struct StructuredError { code: ErrorCode, severity: Severity, auto_recoverable: bool, fix_template: String }

  每個 tool/provider 的 error 映射到統一 ErrorCode
```

### D1: Cost Budget Circuit Breaker — `src/cost_budget.rs`

```
實作：
  enum BudgetState { Normal, Warning, CircuitBreak, Emergency }
  — Normal: cost < 80% daily budget
  — Warning: 80-100% → Telegram 通知
  — CircuitBreak: 100-120% → 只允許 P0 任務
  — Emergency: >120% → 全停

  struct CostBudget { daily_limit: f64, weekly_limit: f64, monthly_limit: f64 }
  fn check_budget(current_cost: f64, budget: &CostBudget) -> BudgetState
```

### D1-D4: 其餘基礎設施

```
Per-Task Cost Ceiling: 每種 task type 預估成本上限，dispatch 前檢查
Unit Economics Tracker: per-case revenue/cost/margin 追蹤
Auto Operations Report: 每天 08:00 Telegram 發送 (node status, costs, revenue, tasks)
Real-Time Metrics API: /metrics/health, /tasks, /costs, /revenue, /pipeline
Dispatch Audit Log: 每次 dispatch 記錄 alternatives_considered, cost_estimate, decision_reason
Financial Red-Line Monitor: 7 指標 4 級告警
Red-Line Circuit Breaker: RL-1~RL-4 財務紅線
SLO + Error Budget: 月可用率 >=99%, P0 延遲 <5min
L1/L2/L3 Budget Downgrade: 超預算 → 降級模型 → 降級本地 → 停止
Model Tier Selection Rules: 4 維規則引擎 (task_type + priority + cost + sensitivity)
Lightweight Preflight: 每個 tool 執行前輕量檢查
Canary / Gradual Rollout: 4 階段發布 (5%/20%/50%/100%)
Load Testing Framework: 6 種壓測模式
Pre-Launch Issue Gate: P0=0, P1=0, P2 有修復時間表
GitHub Actions CI/CD: 自動測試 + 部署
Langfuse observability: 整合可觀測性
災難恢復: SQLite 自動備份
```

---

## 五、Session E — 知識系統實作計畫

### D0: Post-Task Knowledge Capture — `src/knowledge_capture.rs`

```
實作：
  struct KnowledgeNode { problem, decision, result, lesson, confidence, tags }

  Hand 完成後自動呼叫：
  1. 從 Hand output 提取 KnowledgeNode（用 LLM 或 regex）
  2. 存入 knowledge_nodes 表
  3. 下次同類任務自動 recall 注入 system prompt
```

### D0: Context Pack Generator — `src/context_pack.rs`

```
實作：
  fn generate_context_pack(task: &str, memory: &MemoryStore) -> String
  1. Semantic search top-10 相關記憶
  2. Error history（同類 tool 最近 5 次錯誤）
  3. Template matching（有無匹配的 hand template）
  4. 組合成 context string → 注入 system prompt
  5. 7 天快取
```

### D1: Knowledge Graph — `src/knowledge_graph.rs`

```
實作：
  knowledge_edges 表 (from_id, to_id, edge_type, weight, created_at)
  edge_type: causes, solves, alternative_to, depends_on, supersedes

  fn add_edge(from: &str, to: &str, edge_type: EdgeType)
  fn find_related(node_id: &str, depth: usize) -> Vec<KnowledgeNode>
  fn find_solution(problem: &str) -> Option<KnowledgeNode>

依賴：rusqlite (已有)
```

### D2: Observational Memory MVP — `src/observational_memory.rs`

```
模式：Mastra pattern — 3-40x token 壓縮

實作：
  1. ConversationObserver: 監聽 agent_runtime 的每輪對話
  2. 每 N 輪（或 token count > threshold）觸發 observation
  3. Observation = LLM summary of recent conversation (壓縮 3-40x)
  4. 存入 observations 表
  5. 後續對話的 system prompt 注入最近 K 個 observations

依賴：LLM call (用現有 llm_router)
```

### D3: Condenser Pipeline — `src/condenser.rs`

```
模式：OpenHands pattern — 上下文管道壓縮

實作：
  trait Condenser { fn condense(messages: &[ChatMessage]) -> Vec<ChatMessage> }

  struct SlidingWindowCondenser { window_size: usize }  // 保留最近 N 條
  struct SummaryCondenser { threshold: usize }  // 超過 threshold 條 → LLM summary
  struct HierarchicalCondenser { levels: Vec<Box<dyn Condenser>> }  // 分層壓縮

  整合到 agent_runtime: 每次 LLM 呼叫前自動 condense
```

### D4-D7: 其餘知識系統

```
Anti-Repeat Rules Engine: 同錯 2x → suggest preflight rule, 3x → auto-draft
Knowledge Value Scoring: 加權分數，top 20% 模板化，bottom aging 清理
Data Lifecycle Management: Hot/Warm/Cold 分層 + 定期清理
Skills System: SKILL.md 格式定義 + skills registry + 載入/執行
llama.cpp + Vulkan: 本地推理 backend
LiteLLM unified gateway: 統一 API 閘道
Agent Pack: Hand 包裝成可下載產品
Grafana Dashboard: Prometheus metrics 視覺化
分散式語義記憶: 跨節點 memory sync
```

---

## 六、Session F — 安全/企業實作計畫

### D0: Three-Layer Governance — `src/governance.rs`

```
實作：
  enum RuleLayer { L1Hard, L2Policy, L3Preference }
  struct GovernanceRule { layer, condition: String, action: RuleAction, weight: f64 }

  L1: 不可違反（如：不得刪除生產資料庫）
  L2: 加權政策（如：優先使用免費模型，權重 0.8）
  L3: 使用者偏好（如：繁體中文輸出，可覆蓋）

  fn evaluate(rules: &[GovernanceRule], context: &Value) -> GovernanceDecision
```

### D0: Prompt Injection Guard — `src/injection_guard.rs`

```
實作：
  fn check_injection(input: &str) -> Option<InjectionType>

  檢查模式：
  1. System prompt override: "ignore previous instructions", "you are now"
  2. Role switching: "act as", "pretend you are"
  3. Encoding bypass: base64 encoded malicious prompts
  4. Delimiter injection: ``` ``` blocks with hidden instructions

  整合：Telegram/HTTP/Hand params 入口全部過濾
```

### D1: RBAC + Audit Log — `src/rbac.rs` + `src/audit.rs`

```
RBAC:
  enum Role { Admin, Operator, Viewer, Customer }
  struct Permission { resource: String, action: String }
  fn check_permission(user: &User, resource: &str, action: &str) -> bool

Audit:
  audit_log 表 (timestamp, actor, role, action, resource, result, ip, details)
  fn log_audit(actor: &str, action: &str, resource: &str, result: &str)
  保留 >= 90 天
```

### D1: Tiered Approval — 修改 `src/approval.rs`

```
實作：
  enum ApprovalType { ExternalPublish(12h), ContractSign(24h), ProdDeploy(4h), Payment(12h), DataExport(4h) }

  每種類型有不同 timeout 和通知管道
  擴展現有 ApprovalGate 支援多類型
```

### D2-D7: 其餘安全/企業

```
Service Tier: Lite($29)/Pro($99)/Team($299) — 任務限額、模型存取、SLA
Customer Health Score: 0-100 加權 (efficiency 30%, quality 25%, speed 25%, satisfaction 20%)
Churn Risk Detector: 2週未用/NPS<6/3+投訴 → alert
Per-Agent Tool Whitelist: 每 worker 限制可用 tools
Agent Role/Goal/Backstory: CrewAI pattern, per-worker 專精設定
Browser Isolation Profiles: 每次 browser tool 用獨立 profile
Validation Gate: LLM 輸出 < 100 chars 或明顯偷懶 → reject + retry
UnsupportedParam Filter: 各 provider 過濾不支援的參數
Confidence Scoring: 多 agent 輸出比較，差異 <20% → 選信心分高的
.claw Agent File Format: 可攜式 agent 格式（JSON manifest + tools + prompts）
Cross-Device Consistency: 同 prompt 多節點輸出比較 >= 90% 語意相似
API Key 花費上限: 每日/每月 per-key spend cap
memory.db 備份: 定期 SQLite backup → 異地
MCP Server Mode: 暴露 Hub 為 MCP server
HostProvider DI: 支援 Slack/Discord/Web channel
Dashboard / Routing Optimizer
Market Data / Backtesting
Claude API 整合驗證
Research report 銷售通道
```

---

## 七、Session G — Hand TOML 實作計畫

### 所有 Hand 都是 ~/.phantom-mesh/hands/{name}/hand.toml，不碰 Rust

### 通用 Hand TOML 結構
```toml
[hand]
name = "xxx"
description = "..."
version = "1.0"
priority = "P1"

[settings]
output_format = "markdown"

[[phases]]
name = "phase_1"
system_prompt = "..."
tools = ["web_search", "http_request"]
node_affinity = "light"
quality_gate = true

[[phases]]
name = "phase_2"
system_prompt = "..."
tools = ["file_write"]
```

### D0: `youtube` hand

```toml
Phase 1: 研究（web_search 找熱門主題）
Phase 2: 腳本（generate script, 含 hook/intro/body/outro/CTA）
Phase 3: TTS（tts tool 語音生成）
Phase 4: 配圖（image_generate 每段配圖）
Phase 5: 合成（video_compose 圖片+音頻→影片）
Phase 6: 上傳（youtube_upload）
```

### D0: `report` hand

```toml
Phase 1: 資料收集（web_search + http_request）
Phase 2: 分析（delegate 到 reasoning model）
Phase 3: 圖表資料準備（結構化 JSON）
Phase 4: 輸出（xlsx_export + docx_export + pdf_export）
```

### D1: `novel` hand（完善）

```toml
Phase 1: 世界觀設定（genre, setting, rules, timeline）
Phase 2: 角色設計（protagonist, antagonist, supporting, 5W1H）
Phase 3: 大綱（3 幕結構, 每幕章節列表）
Phase 4: 逐章生成（循環 — 讀前章摘要 → 生成本章）
Phase 5: 校對（一致性檢查、語法、風格）
Phase 6: 輸出（pdf_export + docx_export）
```

### D1: `design` hand

```toml
Phase 1: 需求分析（web_search 競品 + 使用者需求整理）
Phase 2: 多稿生成（image_generate × 3-5 個方向）
Phase 3: 自評（vision tool 分析每稿 → 打分）
Phase 4: 精修（選最高分 → image_generate 變體）
Phase 5: 輸出（file_write 最終稿 + 設計說明）
```

### D2: `comic` hand

```toml
Phase 1: 劇本（故事大綱 + 分鏡描述）
Phase 2: 角色設計（image_generate 角色 reference sheet）
Phase 3: 每格生成（image_generate 每一格）
Phase 4: 對話框（文字排版疊加）
Phase 5: 排版（page layout）
Phase 6: 輸出（pdf_export）
```

### D2: `ecommerce_ops` hand

```toml
Phase 1: 產品研究（web_search 競品分析）
Phase 2: 文案生成（產品描述、FAQ、賣點）
Phase 3: 定價策略（市場比較 → 建議價格）
Phase 4: 監控設定（排程檢查價格變動）
```

### D3: `music` hand

```toml
Phase 1: 風格研究（web_search 流行趨勢）
Phase 2: 歌詞生成（主題 → 歌詞 + 結構）
Phase 3: 音樂生成（music_generate）
Phase 4: 封面（image_generate album art）
Phase 5: 發布（file_write 成品 + metadata）
```

### D3: `game_dev` hand

```toml
Phase 1: 設計文件（遊戲類型、機制、平台）
Phase 2: 代碼生成（ai_code → 遊戲核心邏輯）
Phase 3: 資產生成（image_generate 素材）
Phase 4: 測試（shell 執行測試）
Phase 5: 打包（shell 打包命令）
```

### D4: `micro_saas` hand

```toml
Phase 1: 市場驗證（web_search + analysis）
Phase 2: 競品分析（web_search + 功能比較矩陣）
Phase 3: 定價策略（成本估算 + 市場定位）
Phase 4: 功能規格（MVP feature list）
Phase 5: 輸出（docx_export 或 file_write）
```

---

## 八、D11-D15 商品化實作計畫（3/27-3/31）

### Multi-Tenant API — `src/multi_tenant.rs`

```
實作：
  struct Tenant { id, name, api_key, tier, created_at }
  tenants 表 (SQLite)

  API middleware:
  1. 每個 request 帶 X-API-Key header
  2. 查 tenants 表 → 取得 tenant info
  3. 注入 tenant context（隔離 workspace, memory, sessions）
  4. 每個 tenant 有自己的 namespace: ~/.phantom-mesh/tenants/{id}/

  endpoints:
  POST /tenants — 建立租戶（需 admin API key）
  GET /tenants/:id — 查看租戶資訊
  POST /tenants/:id/api-keys — 生成 API key
  DELETE /tenants/:id/api-keys/:key — 撤銷
```

### 用量計量 — `src/usage_meter.rs`

```
實作：
  usage_records 表 (tenant_id, metric, quantity, timestamp)
  metrics: api_calls, tokens_in, tokens_out, tool_calls, tasks_completed

  fn record_usage(tenant_id: &str, metric: &str, qty: u64)
  fn get_usage(tenant_id: &str, period: &str) -> UsageSummary

  每次 tool call / LLM call 結束後自動記錄
```

### Stripe Subscription + Metered Billing

```
實作：
  1. Stripe 建立 3 個 Product + Price:
     - Lite: $29/mo (base) + $0.01/API call overage
     - Pro: $99/mo (base) + $0.005/API call overage
     - Team: $299/mo (base) + $0.003/API call overage
  2. 用戶註冊時 → Stripe Checkout → create subscription
  3. 每月底 → 計算超額用量 → Stripe Usage Record → 自動計費
  4. Webhook: subscription.created/updated/deleted → 更新 tenant tier
```

### Landing Page

```
技術：Astro or Next.js 靜態站
頁面：
  - Hero: Phantom Mesh — AI Agent Cluster Platform
  - Features: 25+ Tools, 20+ Hands, 8-Device Cluster
  - Pricing: Lite/Pro/Team
  - Docs: Quick Start + API Reference
  - Sign Up → Stripe Checkout

部署：Vercel (免費)
```

---

## 九、每日產出摘要

| Day | 日期 | A (Main) | B (Tools) | C (Cluster) | D (Infra) | E (Knowledge) | F (Security) | G (Hands) |
|-----|------|----------|-----------|-------------|-----------|---------------|-------------|-----------|
| D0 | 3/16 | SEO站+E2E測試 | image_generate | SLA+Idempotency+Taxonomy | Metrics+ErrorCodes+Alerts | KnowledgeCapture+ContextPack | Governance+InjectionGuard | youtube+report |
| D1 | 3/17 | Stripe+CS入口 | docx+xlsx_export | WorkerScorer+TaskQueue+Scheduler | CostBudget+CostCeiling+UnitEcon | KnowledgeGraph+AntiRepeat+ValueScore | RBAC+Audit+TieredApproval | novel+design |
| D2 | 3/18 | SaaS+Trading+Cron | tts+email_receive | Preemption+NodeCap+SoTCross+Affinity | OpsReport+MetricsAPI+AuditLog | ObservationalMemory | ServiceTier+HealthScore+ChurnRisk | comic+ecommerce |
| D3 | 3/19 | 7路線整合測試 | video_compose+youtube_upload | NodeOnboard+AutoDeploy+ScheduleWin | RedLine+FinancialMonitor+SLO | Condenser | ToolWhitelist+AgentRole+BrowserIso | music+game_dev |
| D4 | 3/20 | Stripe Live+SPF | music_generate+KB_import | CrossNodeSched+WorkerQuality | BudgetDowngrade+ModelTier+Preflight | DataLifecycle+AgentReport+HandVersion | ConfidenceScore+ValidationGate+ParamFilter | micro_saas+Email模板 |
| D5 | 3/21 | ✅驗收+投標 | cli_anything+linkedin | Retry+Failover+Fallback | Canary+LoadTest | SkillsSystem | .claw format | Hand微調+Newsletter |
| D6 | 3/22 | SEO量產+客戶 | MCP Server+HostProvider | IncidentResponse+Rollback | LaunchGate+Adapter+DualWrite | llama.cpp+LiteLLM | CrossDevice+APILimit+Backup | AdSense+Affiliate |
| D7 | 3/23 | SEO+Twitter+轉換 | SearchConsole+Engagement | ClusterDashboard+RoutingEnhance | Langfuse+災難恢復 | AgentPack | Dashboard+Backtesting | NodeAffinity+SOP |
| D8 | 3/24 | 投標+B2B+報表 | Proxy+修復 | 分散式狀態 | CI/CD | 分散式記憶+Grafana | Claude整合+Research銷售 | 壓測+CronSched |
| D9 | 3/25 | 全系統整合測試 | 修 bug | 修 bug | 修 bug | 修 bug | 修 bug | 修 bug |
| D10 | 3/26 | 回歸測試+文件 | Unit test 補完 | 穩定性監控 | CI/CD完整跑 | 知識整合測試 | 安全審計 | Hand全跑一輪 |
| D11-15 | 3/27-31 | 商品化+上架 | 商品化支援 | 商品化支援 | 商品化支援 | 商品化支援 | 商品化支援 | 商品化支援 |

---

## 十、風險與緩解

| 風險 | 機率 | 影響 | 緩解策略 |
|------|------|------|---------|
| Merge conflict 太多 | 中 | 延遲 1-2h/天 | 嚴格按模組分工 + 每天 22:00 merge |
| Acer 編譯慢 | 高 | D session 產出慢 | D session 只建新檔案，不需頻繁編譯 |
| 154 項品質不足 | 高 | Bug 多 | D9-D10 全天修 bug + 穩定化 |
| 外部 API 限制 | 中 | 某些 tool 不能完整測試 | mock 測試 + 標記為 beta |
| Stripe 審核慢 | 低 | 金流延遲 | 先用平台收款 |
| 團隊只有一人操作 7 session | 高 | 瓶頸在 context switch | 每個 session 給清楚的 prompt + 讓 Claude 自主跑 |
