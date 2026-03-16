# Clawtex Cluster Revenue Execution Plan 2026
> 目標：年利潤 NT$3,000,000（~USD $100K）
> 架構：**分散式 AI Agent 集群**，多硬體 + 多服務協同
> 起始日：2026-03-13
> 作者：深度分析自 27 個參考專案 + 21 篇賺錢文章 + Anthropic agent patterns

---

## 零、集群架構總覽 — 這不是單機，是分散式系統

### 核心理念
Clawtex 是一個**分散式 agent 集群**，不是一個跑在筆電上的 daemon。集群的價值在於：
- **並行能力**：4+ 節點同時執行不同收入任務 → 吞吐量 3-5x
- **專業化分工**：每個節點根據硬體特性承擔最適合的角色
- **容錯能力**：任一節點離線，任務自動漂移到其他節點
- **成本趨零**：本地推理為主（95%+ 任務），cloud API 只用於高價值任務

### 硬體清單

| 節點 | 角色 | 硬體 | 推理能力 | 網路 | 主要職責 |
|------|------|------|---------|------|---------|
| **Z13 Hub** | Hub + 主力推理 | Ryzen AI MAX+ 395 (16C/32T), Radeon 8060S, 64GB, NPU 50 TOPS | LM Studio: qwen3-coder-next (72 tok/s), Ollama: llama3.2:1b, NPU: Mistral-7B | Tailscale 100.x | 任務分派、agent runtime、高複雜度推理、檔案/記憶體管理 |
| **M1 Mac** | Full Worker | Apple M1, 8/16GB | Ollama: llama3/phi3 (Apple Silicon 加速) | Tailscale 100.x | 中複雜度推理、SoT 並行展開、SEO/Content 生成 |
| **AYANEO** | NPU Worker | AMD NPU (XDNA) | NPU 推理 (Mistral-7B 等小模型) | Tailscale 100.x | 低延遲分類、embedding、即時回應、Privacy-critical |
| **Acer** | Light Worker | 基本規格 | 有限推理能力 | Tailscale 100.x | 網路 I/O 密集任務：web_search、email_send、http_request |

### 雲端服務

| 服務 | 用途 | 成本控制 |
|------|------|---------|
| **Gemini** (free tier) | Vision、公開內容生成、繁體中文 | 免費額度內 |
| **Groq** (free tier) | 高速推理 backup、分類器 | 免費額度內 |
| **DeepSeek** | 複雜推理 fallback | 按量付費，設月上限 |
| **Cerebras** | 超高速推理 | 免費額度內 |
| **Together/OpenRouter** | 模型多樣性、特殊任務 | 按量付費，設月上限 |
| **ChatGPT (Codex)** | 頂級複雜任務 | Plus 訂閱 ($20/月固定成本) |

### 集群拓撲
```
                    ┌─────────────────────────────────────────┐
                    │           Telegram Bot API              │
                    └─────────────┬───────────────────────────┘
                                  │
                    ┌─────────────▼───────────────────────────┐
                    │         Z13 Hub (port 7878)             │
                    │  ┌─────────────────────────────┐        │
                    │  │ ClusterHub → ToolRouting     │        │
                    │  │ AgentRuntime (24 tools)      │        │
                    │  │ HandRunner (17 hands)        │        │
                    │  │ CronScheduler (6 jobs)       │        │
                    │  │ SmartRouter (3-tier)          │        │
                    │  │ PrivacyGuard                  │        │
                    │  │ SoT Engine (parallel gen)     │        │
                    │  │ CostTracker + RevenueTracker  │        │
                    │  │ SelfEvolve (nightly 1%)       │        │
                    │  └─────────────────────────────┘        │
                    │  LOCAL: file_*, memory_*, glob/content   │
                    │  LLM: LM Studio + Ollama + NPU          │
                    └───┬──────────┬──────────┬───────────────┘
                        │          │          │
            Tailscale VPN (100.x.x.x mesh)
                        │          │          │
              ┌─────────▼──┐  ┌───▼──────┐  ┌▼──────────┐
              │  M1 Mac     │  │ AYANEO   │  │  Acer     │
              │  Full Worker│  │NPU Worker│  │Light Worker│
              │  port 7879  │  │ port 7880│  │ port 7881 │
              │             │  │          │  │           │
              │ • SoT 展開  │  │ • 分類器 │  │ • web爬取 │
              │ • 內容生成  │  │ • embed  │  │ • email   │
              │ • code gen  │  │ • 即時應答│  │ • HTTP    │
              │ • 長文寫作  │  │ • 隱私任務│  │ • 監控    │
              └─────────────┘  └──────────┘  └───────────┘
                        │          │          │
              ┌─────────▼──────────▼──────────▼───────────┐
              │          Cloud Providers (Fallback)        │
              │  Gemini │ Groq │ DeepSeek │ ChatGPT       │
              │  (public task) │ (complex fallback)        │
              └───────────────────────────────────────────┘
```

### 任務路由矩陣

| Tool | Routing | 最佳節點 | 原因 |
|------|---------|---------|------|
| file_read/write/edit | LOCAL_ONLY | Z13 Hub | 檔案系統在 hub |
| memory_store/recall | LOCAL_ONLY | Z13 Hub | SQLite 在 hub |
| glob/content_search | LOCAL_ONLY | Z13 Hub | workspace 在 hub |
| shell | FullWorkerOnly | Z13/M1 | 需要完整工具鏈 |
| ai_code | FullWorkerOnly | Z13/M1 | 需要 Claude/Codex CLI |
| browser | FullWorkerOnly | Z13/M1 | 需要 Playwright |
| web_search | AnyWorker | **Acer** (優先) | 網路 I/O，釋放 GPU 節點 |
| http_request | AnyWorker | **Acer** (優先) | 同上 |
| email_send | AnyWorker | **Acer** (優先) | SMTP 是 I/O bound |
| skeleton_generate | 特殊 | **全集群** | SoT 跨節點並行展開 |
| delegate_to_provider | 按 privacy | 按敏感度 | critical→Z13, public→cloud |

---

## 一、現實檢視：為什麼 95% 的 AI Agent 項目不賺錢

來自 MIT 研究和 21 篇文章的共同結論：

1. **技術 ≠ 收入**：你可以建出最強的 agent 系統，但如果沒有 paying customer，就是零
2. **Agent 是乘數器不是印鈔機**：它放大已存在的商業邏輯，不能憑空創造需求
3. **你賣的是結果**：客戶不在乎你用什麼技術，他們要報告、文章、自動化流程、解決方案
4. **品質 > 數量**：一篇精心編輯的文章勝過 100 篇 AI 垃圾文

### Clawtex **集群**的實際優勢（vs 單機）
- **並行吞吐**：4 節點同時跑不同 hand → 同時處理 4+ 收入任務
- **本地推理成本近零**：Qwen3 30B @ 72 tok/s (Z13) + Apple Silicon (M1) + NPU (AYANEO) → 邊際成本極低
- **24/7 自動化**：cron + hands + 集群容錯 → 任一節點掛掉其他照跑
- **多 provider fallback + 集群 fallback**：provider 掛 → 換 provider；節點掛 → 換節點
- **SoT 跨節點加速**：一篇 10 節文章，Z13 寫大綱，M1+AYANEO+Z13 各展開 3-4 節 → 3x 速度

### Clawtex 集群的實際弱點（必須誠實面對）
- **輸出品質不穩定**：沒有 quality gate，30% 的輸出是垃圾
- **Memory 太原始**：key-value SQLite 無法追蹤複雜客戶關係
- **沒有人類反饋迴路**：agent 不知道什麼做得好什麼做得爛
- **集群還沒完全上線**：M1/AYANEO/Acer worker 尚未部署
- **沒有真實收入驗證**：所有 route 都是理論，需要 validate

---

## 二、集群級收入路線 — 每個節點的角色

### 收入路線 × 節點分工矩陣

| 路線 | Z13 Hub | M1 Worker | AYANEO NPU | Acer Light | Cloud |
|------|---------|-----------|------------|------------|-------|
| **A: Freelancing** | AgentRuntime + ai_code | code_gen hand | 分類 proposal | web_search jobs | ChatGPT 複雜案 |
| **B: Cold Email** | HandRunner + CRM | outreach content gen | lead 分類/評分 | email_send batch | Gemini personalize |
| **C: B2B Sub** | auto_report + customer_service | 報告生成 | 即時客服回應 | 監控 webhook | — |
| **D: SEO/Affiliate** | HandRunner orchestration | seo_content gen (主力) | 關鍵字分析 | web_search 競爭分析 | Gemini 長文 |
| **E: Personal Brand** | content hand + blog_publish | twitter content gen | — | 發布 API calls | — |
| **F: Agent Pack** | 開發 + 測試 | 測試不同模型 | NPU 兼容測試 | — | CI/CD |

### Tier 1：本月開始產生收入（3-4 週內）

#### 路線 A：AI-Enhanced Freelancing — 目標 $3-6K/月
**集群優勢**：Z13 跑 ai_code (Claude/Codex)，M1 同時跑 code_gen hand，AYANEO 即時分類工作匹配度

**具體行動**：
```
Week 1: 建立 Upwork 帳號，完善 profile
        部署 M1 + Acer worker（Tailscale 連線）
Week 2: 每天用 Freelancer Hand 找 5-10 個工作（Acer: web_search, Z13: 分析+排名）
        手動投標 3-5 個（Z13 生成 proposal, AYANEO 即時品質評分）
Week 3: 完成第一個付費專案（$50-200，低價換評價）
        Z13: ai_code 寫程式碼，M1: 同時處理其他案子的 research
Week 4: 用 AI 加速交付，開始接 $300+ 的案子
        並行處理 2-3 個案子（每個案子分配到不同節點）
```

**集群並行場景**（Week 4+ 典型工作日）：
```
09:00  Z13: freelancer hand 掃描新工作 (cron)
       M1: 繼續昨天的 code_gen 案子
       Acer: web_search 競爭對手方案
09:30  Z13: 生成 3 份 proposal (LLM-as-Judge 品質篩選)
       AYANEO: 即時分類工作 → 推薦 top 5
10:00  Z13: ai_code 開始案子 A (Claude)
       M1: ai_code 開始案子 B (local Qwen)
       Acer: 監控 email 回覆 (IMAP)
14:00  Z13: 案子 A 交付，開始案子 C
       M1: 案子 B 繼續 + SoT 寫文件
       AYANEO: 客戶訊息即時回應
```

#### 路線 B：B2B Cold Email — 目標 $1-4K/月
**集群優勢**：Acer 批量發 email 不佔 GPU，M1 同時生成 personalized content

**具體行動**：
```
Week 1: 選定 3 個目標行業（台灣中小企業自動化、跨境電商、SaaS startup）
Week 2: Lead Hand: Acer 爬取 prospect 資料 → Z13 分析 → AYANEO 評分 → M1 生成 personalized email
Week 3: Outreach Hand: Acer 排程發送 email（每天 5 封，錯開時間）
        Z13 追蹤 pipeline (CRM in memory_store)
Week 4: 追蹤回覆，手動跟進 warm leads
        M1 生成客製化方案書 (SoT 並行)
```

### Tier 2：1-3 個月建立被動收入管道

#### 路線 D：SEO + Affiliate — 目標 $1-5K/月（6 個月後）
**集群優勢**：SoT 跨 3 節點並行 → 一篇 3000 字文章從 30 分鐘降到 10 分鐘

**文章生產流水線**（完全並行）：
```
M1:     skeleton_generate(topic) → 大綱 → 並行展開 section 1, 4, 7
Z13:    並行展開 section 2, 5, 8 → merge → quality gate (LLM-as-Judge)
AYANEO: 並行展開 section 3, 6, 9 (NPU 小模型，短段落)
Acer:   web_search 關鍵字數據 + 競品分析

產能：每天 2-4 篇高品質文章（vs 單機 1 篇）
每月：60-120 篇 → 6 個月後大量 organic traffic
```

#### 路線 E：Personal Brand + Newsletter — 目標 $2-5K/月
**集群優勢**：content hand 在 M1 跑，不佔 Z13 的 ai_code 資源

### Tier 3：3-6 個月建立訂閱收入

#### 路線 C：B2B Automation Subscriptions — 目標 $5-15K/月
**集群優勢**：每個客戶的自動化任務分配到不同 worker → 並行服務多客戶

```
客戶 A 的 auto_report: M1 每日生成 → Acer email 發送
客戶 B 的 customer_service: AYANEO 即時回應 → Z13 複雜升級
客戶 C 的 market_intel: Z13 深度分析 → M1 生成報告
= 3 個客戶同時服務，零邊際成本增加
```

#### 路線 F：Open Source → Agent Pack Sales
**集群驗證**：clawtex-core 本身就是集群系統，開源後的賣點就是「多節點並行 agent 系統」

---

## 三、必要的技術改進（按 ROI 排序 — 集群版）

### P0：直接影響收入品質 + 集群上線（本週完成）

| 改進 | 參考專案 | 工時 | 影響 | 集群相關 |
|------|---------|------|------|---------|
| **Worker 部署腳本** | tsk (WorkerPool) | 4h | 集群上線 | M1 + AYANEO + Acer 部署 |
| StreamingThinkFilter | OpenFang | 2h | Qwen 輸出品質 | 全節點受益 |
| Quality Gate（LLM-as-Judge） | Swarm, CrewAI | 4h | Hand 輸出品質 | Judge 在 Z13，被判斷物可來自任何節點 |
| Subprocess env stripping | OpenFang | 1h | chatgpt_backend 安全 | Z13 only |
| **真實 CPU load 報告** | — | 1h | 集群排程準確度 | 目前 get_cpu_load() 寫死 0.1 |

### P1：直接影響集群效率（本月完成）

| 改進 | 參考專案 | 工時 | 影響 | 集群相關 |
|------|---------|------|------|---------|
| **Worker capability-based routing 增強** | IronClaw (13-dim scorer) | 6h | 任務分派智慧度 | 根據 device_type + 能力自動選節點 |
| **SoT 集群展開模式** | — | 4h | 跨節點 SoT | 目前 SoT 只在 hub 內 round-robin providers，應跨 workers |
| 結構化 CRM（Lead pipeline tracking） | LangGraph checkpoints | 8h | Route B 轉換率 | Hub 上 |
| Validation Gate（防 LLM 偷懶） | Claude Octopus | 4h | Hand 完成率 | 全節點 |
| UnsupportedParam 過濾 | IronClaw | 3h | 防 provider 400 errors | 全節點 |
| Per-agent tool whitelist | OWL | 3h | 降低 tool call 錯誤率 | worker 上限制工具 |

### P2：提升集群智慧（下月完成）

| 改進 | 參考專案 | 工時 | 影響 | 集群相關 |
|------|---------|------|------|---------|
| Observational Memory | Mastra | 40h | 長任務品質 3-40x | Hub 上 centralized |
| Condenser Pipeline | OpenHands | 16h | context 管理 | 所有需 LLM 的節點 |
| **Hand 跨節點調度** | LangGraph, Swarm DAG | 16h | Hand phase 可分派到不同節點 | Phase A→Z13, B→M1, C→AYANEO |
| **集群健康儀表板** | OpenFang (metrics) | 8h | 運維可視化 | Telegram /cluster 指令 |
| Agent role/goal/backstory | CrewAI | 4h | agent 專業度 | 每個 worker 有自己的角色 |

### P3：開源 + 生態系統（2-3 月後）

| 改進 | 參考專案 | 工時 | 影響 | 集群相關 |
|------|---------|------|------|---------|
| Skills System（SKILL.md） | OpenClaw | 16h | Route F | 可在任何 worker 執行 |
| .claw Agent File format | Letta | 16h | 可攜式 agent | 跨集群分享 agent |
| MCP Server mode | All Agents MCP | 8h | 被外部呼叫 | Hub 對外暴露 MCP |
| HostProvider DI（多 channel） | Cline | 16h | Slack/Discord/Web | Hub 統一管理 |
| Worker 自動部署 (SSH) | CLI Agent Orchestrator | 8h | 一鍵擴展集群 | 新機器自動加入 |

---

## 四、集群部署計畫（本週 3/13-3/16）

### Day 1: Z13 Hub 穩定化（3/13）
```bash
# 確認 Hub daemon 穩定
taskkill //F //IM clawtex-core.exe
cargo run -- daemon
# 驗證: /cluster 回傳 local node online
```

### Day 2: M1 Worker 部署（3/14）
```bash
# 在 M1 上:
# 1. 安裝 Rust + 編譯 clawtex-core
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
git clone <repo> && cd clawtex-core
cargo build --release

# 2. 確認 Tailscale 連線
tailscale ping <Z13-tailscale-ip>

# 3. 啟動 worker
./target/release/clawtex-core worker \
  --hub http://<Z13-tailscale-ip>:7878 \
  --name m1 \
  --port 7879

# 4. 在 Z13 Hub 驗證
curl http://localhost:7878/cluster/status
# 應看到: m1 online, capabilities: ["tools"]
```

### Day 3: AYANEO NPU Worker 部署（3/15）
```bash
# AYANEO 上:
# 如果 AYANEO 跑不了完整 Rust，用 Python lightweight worker
python3 lightweight_worker.py \
  --hub http://<Z13-tailscale-ip>:7878 \
  --name ayaneo \
  --port 7880 \
  --capabilities web_search,http_request,email_send \
  --device-type light
```

### Day 4: Acer Light Worker + 全集群驗證（3/16）
```bash
# Acer 上: 同 AYANEO 方式
# 全集群驗證:
# 1. /cluster → 4 nodes online
# 2. 手動觸發 freelancer hand → 確認 web_search 路由到 Acer
# 3. 手動觸發 seo_content hand → 確認 SoT 跨 Z13+M1 展開
# 4. 手動觸發 outreach hand → 確認 email_send 路由到 Acer
```

---

## 五、月度里程碑和收入目標

### Month 1（3/13 - 4/13）：集群上線 + 第一筆收入

**技術任務**：
- [ ] **集群部署**：M1 + AYANEO + Acer workers 上線
- [ ] P0: ThinkFilter + Quality Gate + Worker 部署腳本（12h）
- [ ] P0: 真實 CPU load + env stripping（2h）
- [ ] P1: Worker capability routing 增強（6h）
- [ ] Daemon 穩定運行 24/7（全集群）

**收入任務**：
- [ ] Upwork 帳號建立，profile 完善
- [ ] 第一個 freelance 案子完成（Z13 主力）
- [ ] 第一批 cold email 發送（10封，Acer 發送）
- [ ] Blog 建立 + 4 篇 SEO 文章（M1 + Z13 SoT）
- [ ] Twitter 開始每日發文（M1 content hand）

**收入目標**：NT$0-30,000（$0-1,000）
**驗證指標**：
- 集群 4 節點持續在線 >95% uptime？
- 有人為你的輸出付錢了嗎？
- SoT 跨節點比單機快多少？

### Month 2（4/13 - 5/13）：建立收入基礎

**技術任務**：
- [ ] P1: CRM + Validation Gate（12h）
- [ ] P2: Observational Memory MVP（40h）
- [ ] P2: Hand 跨節點調度 基礎（16h）
- [ ] P2: 集群健康儀表板 (/cluster Telegram)（8h）
- [ ] clawtex-core 準備開源

**收入任務**：
- [ ] Upwork 累積 3-5 個評價，開始接 $300+ 案子
- [ ] B2B outreach 第二輪（30 封 email）
- [ ] Blog 累積 12 篇文章（SoT 集群加速 → 每週 3 篇）
- [ ] Newsletter 啟動，目標 100 subscribers
- [ ] GitHub 開源發布

**收入目標**：NT$30,000-90,000（$1K-3K）
**驗證指標**：有重複客戶嗎？organic inbound 有嗎？集群自動化率 >70%？

### Month 3（5/13 - 6/13）：規模化

**技術任務**：
- [ ] P2: Condenser Pipeline（16h）
- [ ] P3: Skills System（16h）
- [ ] P3: Worker 自動部署 SSH（8h）
- [ ] 考慮加入更多 worker（VPS/新硬體）

**收入任務**：
- [ ] Freelance 穩定每月 $2-4K（並行 2-3 案）
- [ ] 第一個 B2B subscription 客戶
- [ ] Blog 20+ 篇，申請 AdSense + affiliate
- [ ] Newsletter 500+ subscribers
- [ ] 集群服務 2+ B2B 客戶（每個客戶分配到不同 worker）

**收入目標**：NT$90,000-180,000（$3K-6K）

### Month 4-6（6-8月）：被動收入開始

**收入目標**：NT$150,000-300,000/月（$5K-10K）

**集群擴展考量**：
- 如果 B2B 客戶 >5 → 考慮加 VPS worker (Hetzner $10/月)
- 如果 SEO 文章效果好 → 集群全力產文章（每天 4-6 篇）
- 如果 freelance 案子太多 → M1 全職跑 code_gen

**預期收入來源**：
- Freelance: $3-5K（高階案子 + repeat clients，Z13 + M1 並行）
- B2B subscription: $1-3K（2-5 clients，每個 client 一個 worker 任務）
- SEO/Affiliate: $500-2K（30+ 篇文章開始見效，SoT 集群加速產出）
- Newsletter/Brand: $500-1K
- Agent Pack: $0-1K（剛起步）

### Month 7-12（9月-2027年3月）：年目標衝刺

**收入目標**：NT$250,000-400,000/月（$8-13K）

**集群成熟狀態**：
- 4+ 硬體 worker + 1-2 VPS worker = 6 節點集群
- 每日自動處理 50+ 任務（cron + 手動觸發混合）
- Self-evolve 每晚 1% 改進 → 12 個月後整體效率 +37%
- 開源社群帶來的 contributor 幫忙改進 worker 系統

---

## 六、從 27 個參考專案學到的**集群級**關鍵洞察

### 1. tsk → WorkerPool 信號量控制
tsk 的 WorkerPool 用 semaphore 控制並行度，避免過載。
**集群啟發**：ClusterHub 的 dispatch 應該加 per-worker concurrency limit，防止某節點被灌爆。

### 2. AutoAgents → Actor Model 跨節點通訊
AutoAgents 用 Ractor actor model 做 typed pub/sub。
**集群啟發**：Worker 之間可以不經 Hub 直接通訊（peer-to-peer），適合 SoT 展開時 section 之間有依賴的場景。

### 3. IronClaw → 13-dim Routing Scorer
IronClaw 的路由器考慮 13 個維度選最佳 provider。
**集群啟發**：ClusterHub 的 `best_worker_for()` 目前只看 cpu_load，應該加入：latency、memory、GPU utilization、當前任務隊列長度、歷史成功率。

### 4. Swarm → DAG Workflow 跨節點
Swarm 的 DAG 引擎用 topological sort + `{{output}}` interpolation。
**集群啟發**：Hand 的 phase 應該可以標記 `node_affinity`，讓不同 phase 在不同節點執行。例如：
```toml
[phases.research]
node_affinity = "light"  # Acer: web_search heavy

[phases.generate]
node_affinity = "full"  # M1: content generation

[phases.review]
node_affinity = "hub"   # Z13: LLM-as-Judge
```

### 5. OpenFang → 14-crate Workspace 模式
OpenFang 用 14 個 crate 組成的 workspace，每個 crate 可獨立編譯。
**集群啟發**：clawtex-core 可以拆成 `clawtex-hub` + `clawtex-worker` + `clawtex-common`，worker binary 更小，部署更快。

### 6. Goose → Recipe 系統 = 可販售的 Hand
Goose 的 Recipe YAML + 1700+ MCP server 生態 = 社區驅動的 workflow marketplace。
**集群啟發**：clawtex 的 Hands 可以包裝成 marketplace 商品。每個 Hand = 一個可販售的自動化解決方案。集群版的 Hand 更有價值（跨節點並行 = 更快交付）。

### 7. Mastra → Observational Memory 降低成本
3-40x token 壓縮 = 長任務成本降低 = 利潤率提升。
**集群啟發**：Memory 集中在 Hub，所有 worker 的 context 都經過 compaction → 整個集群的 token 消耗降低。

### 8. Claude Octopus → Validation Gate + Dark Factory Mode
防止 LLM 跳過步驟 + 無人值守模式。
**集群啟發**：集群天生適合 Dark Factory — 多節點 24/7 自動運行，每個任務都有 validation gate，異常時 Telegram 通知。

---

## 七、集群經濟學 — 成本 vs 收入模型

### 固定成本（月）
| 項目 | 成本 |
|------|------|
| 電費（Z13 24/7 + 其他節點間歇） | ~NT$1,500 |
| Tailscale（免費 100 節點） | $0 |
| ChatGPT Plus（Codex CLI） | $20 = NT$640 |
| 域名 + Vercel（Blog） | ~NT$300 |
| Upwork service fee (20%→10%) | 收入的 10-20% |
| **月固定成本** | **~NT$2,500** |

### 變動成本（月）
| 項目 | 用量控制 | 預估 |
|------|---------|------|
| Cloud API (Gemini/Groq/DeepSeek) | 90% 本地推理 | NT$500-2,000 |
| Email (Gmail SMTP) | 免費 500/天 | $0 |
| Search API (Serper) | 2500 free queries | $0-500 |
| **月變動成本** | | **NT$500-2,500** |

### 損益平衡點
```
月固定 + 月變動 = NT$3,000 - 5,000
= 只需每月收入 NT$5,000+ 就不虧錢
= 約 1 個 $200 的 freelance 案子
```

### 集群 ROI 計算
```
不用集群（只有 Z13）：
  - 同時處理 1 個案子
  - SoT 單機 30 分/篇
  - 月產能: ~30 篇文章 + 5 案子

用集群（Z13 + M1 + AYANEO + Acer）：
  - 同時處理 3-4 個案子
  - SoT 集群 10 分/篇
  - 月產能: ~90 篇文章 + 15 案子
  - 額外硬體成本: 電費 ~NT$500/月
  - 產能提升: 3x
  - ROI: 3x 產能 / NT$500 = 每 NT$1 額外成本帶來 NT$600 額外產能
```

---

## 八、開源策略 → GitHub 影響力 → 間接收入

### 開源賣點：「分散式 AI Agent 集群」
clawtex-core 的最大差異化：**不是又一個 agent framework，而是一個可部署的分散式集群**。

```
README pitch:
"Deploy AI agents across your local machines.
 Z13 as brain, M1 as workhorse, Raspberry Pi as web crawler.
 Zero cloud cost. Full privacy. 24/7 autonomous."
```

### Phase 1：準備開源（Month 2）
1. 拆分 `clawtex-hub` + `clawtex-worker` crates
2. 寫 README（英文 + 繁體中文）— 重點突出集群架構
3. 建立 examples/：`single-node.toml`, `two-node-cluster.toml`, `full-cluster.toml`
4. Docker Compose for worker deployment
5. CI/CD（GitHub Actions）

### Phase 2：發布 + 社區建立（Month 3-4）
1. GitHub release v0.1.0
2. 發 Hacker News、Reddit r/rust、r/LocalLLaMA、r/selfhosted
3. Blog post：「How I Built a Multi-Node AI Agent Cluster in Rust」
4. 回覆 issues，merge PRs

### Phase 3：生態系統 + 商業化（Month 5+）
1. Hand marketplace（免費 + 付費）
2. clawtex.dev 官網
3. Managed hosting（Route G）— 幫客戶部署集群
4. Enterprise support tiers
5. GitHub Sponsors / Open Collective

### 開源 KPI
- Stars：Month 3 目標 500，Month 6 目標 2000
- Contributors：Month 6 目標 10
- Discord members：Month 6 目標 200
- **最重要**：有人在自己的硬體上部署了 clawtex cluster

---

## 九、風險管理

| 風險 | 機率 | 影響 | 對策 |
|------|------|------|------|
| Worker 節點不穩定 | 高 | 中 | heartbeat + auto-reconnect + 任務自動漂移 |
| Freelance 找不到案子 | 中 | 高 | 多平台 + 降低價格 + 集群並行投更多標 |
| Cold email 零回覆 | 高 | 中 | A/B test、改進 targeting、集群批量測試多版本 |
| SEO 文章被 Google 懲罰 | 低 | 中 | 人工編輯每篇、加入原創觀點和數據 |
| API 成本超支 | 低 | 中 | 本地模型為主（95% tasks），cost alert |
| Agent 輸出品質差 | 中 | 高 | Quality Gate + human review + feedback loop |
| 硬體故障 | 低 | 中 | 集群冗餘：任一節點掛，其他照跑；VPS backup daemon |
| Tailscale 斷線 | 低 | 高 | Hub 自動降級為單機模式，重連後 worker auto re-register |
| 開源被複製 | 高 | 低 | 速度 + 社區 + 品牌就是護城河 |
| 破產（零收入持續 3 月+）| 中 | 極高 | Month 1 就要看到第一筆收入，不然立即調整 |

### 止損點
- **Month 2 末**：如果完全零收入 → 重新評估方向
- **Month 4 末**：如果月收入 < NT$30K → 砍掉 ROI 低的 route，集中資源
- **隨時**：如果單月 API 成本 > NT$10K 且無對應收入 → 立即降級到純本地模型
- **集群止損**：如果 worker 維護成本（時間 + 電費）> 產能提升帶來的收入 → 降為單機

---

## 十、本週立即行動（3/13-3/20）

### Day 1-2（3/13-14）：技術 P0 + M1 Worker
- [ ] 實作 StreamingThinkFilter（2h）
- [ ] 實作 Quality Gate / LLM-as-Judge 在 self_evolve Hand（4h）
- [ ] Subprocess env stripping for chatgpt_backend（1h）
- [ ] **M1 Worker 部署**：編譯 clawtex-core + Tailscale 連線 + 註冊 hub
- [ ] 修復 get_cpu_load() 回傳真實值（1h）

### Day 3-4（3/15-16）：AYANEO + Acer + 驗證
- [ ] **AYANEO Worker 部署**（NPU or lightweight worker）
- [ ] **Acer Worker 部署**（lightweight worker）
- [ ] 全集群驗證：4 節點 online + tool routing 正確
- [ ] 測試 SoT 跨節點並行展開
- [ ] 建立 Upwork 帳號 + 完善 profile

### Day 5-7（3/17-20）：收入啟動
- [ ] 用 Freelancer Hand 搜索 10 個合適的工作（Acer: web_search）
- [ ] 手動投標 3-5 個（Z13: proposal gen + AYANEO: quality score）
- [ ] 選定 B2B 目標行業
- [ ] Lead Hand 收集 20 個 prospect（Acer: crawl）
- [ ] 寫第一篇 SEO 文章（Z13 + M1 SoT 並行）
- [ ] Twitter 帳號建立，發第一條

---

## 十一、成功的定義

| 時間 | 月收入 | 年化 | 集群狀態 | 狀態 |
|------|--------|------|---------|------|
| Month 1 | NT$0-30K | - | 4 節點 online | 驗證中 |
| Month 3 | NT$90-180K | NT$1.1-2.2M | 4 節點 95% uptime | 基礎建立 |
| Month 6 | NT$150-300K | NT$1.8-3.6M | 4-6 節點 + VPS | **達標區間** |
| Month 9 | NT$200-400K | NT$2.4-4.8M | 穩定集群 | 穩定成長 |
| Month 12 | NT$250-500K | **NT$3-6M** | 成熟集群 | **目標達成** |

### 最保守估計（只有 Route A + D 成功，集群加速）
- Freelance $2K/月 + SEO $1K/月 = $3K/月 = NT$90K/月 = NT$1.08M/年
- 集群加速版：Freelance $3K（並行 2 案）+ SEO $2K（3x 文章量）= $5K = NT$1.8M/年
- **比單機方案多 67%**

### 現實路徑：3 條 route 穩定（集群加速）
- Freelance $3-6K + B2B $2-5K + SEO/Content $2-4K = $7-15K/月
- **年收 NT$2.5-5.4M** ← 這是集群系統的可行目標

---

> **核心信念**：這不是一台電腦上的一個程式。
> 這是一個分散式系統，每個節點都是一個高效率的工作者。
> 集群的價值 = 並行能力 × 節點效率 × 24/7 自動化。
> 每一行代碼都必須回答：「這能讓集群多賺多少，或多快交付？」
