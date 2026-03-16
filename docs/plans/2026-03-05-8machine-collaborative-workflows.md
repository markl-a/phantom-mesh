# Clawtex 8 機協同工作流設計文件

> 日期: 2026-03-05
> 狀態: 設計完成
> 前置文件: cluster-heartbeat-design, cluster-task-scheduler-design, distributed-state-design
> 影響模組: `src/hands/mod.rs`, `src/skeleton.rs`, `src/providers/router.rs`, `src/cluster.rs`

---

## 目錄

1. [叢集拓撲與能力矩陣](#1-叢集拓撲與能力矩陣)
2. [provider_hint 與 depends_on 配置規格](#2-provider_hint-與-depends_on-配置規格)
3. [工作流 1: Freelancer Hand 8 機協同版](#3-工作流-1-freelancer-hand-8-機協同版)
4. [工作流 2: SEO Content 8 機並行版](#4-工作流-2-seo-content-8-機並行版)
5. [工作流 3: 大規模批次處理](#5-工作流-3-大規模批次處理)
6. [工作流 4: 即時對話低延遲路徑](#6-工作流-4-即時對話低延遲路徑)
7. [工作流 5: SaaS Pipeline 全流程](#7-工作流-5-saas-pipeline-全流程)
8. [負載感知路由演算法](#8-負載感知路由演算法)
9. [資料流設計](#9-資料流設計)
10. [Hand TOML 擴展格式](#10-hand-toml-擴展格式)
11. [Rust 實作要點](#11-rust-實作要點)

---

## 1. 叢集拓撲與能力矩陣

### 1.1 硬體總覽

```
                     ┌──────────────────────────────────┐
                     │          Tailscale VPN Mesh       │
                     └──┬──┬──┬──┬──┬──┬──┬──┬──────────┘
                        │  │  │  │  │  │  │  │
  ┌─────────────────────┘  │  │  │  │  │  │  └─────────────────────┐
  │          ┌─────────────┘  │  │  │  │  └─────────────────┐      │
  │          │     ┌──────────┘  │  │  └──────────┐         │      │
  │          │     │      ┌──────┘  └──────┐      │         │      │
  v          v     v      v                v      v         v      v
┌────┐   ┌────┐ ┌────┐ ┌────┐          ┌────┐ ┌────┐   ┌────┐ ┌────┐
│ #1 │   │ #2 │ │ #3 │ │ #4 │          │ #5 │ │ #6 │   │ #7 │ │ #8 │
│Z13 │   │ M1 │ │Aya │ │Acer│          │Mini│ │Mini│   │RTX │ │RTX │
│HUB │   │Mac │ │neo │ │    │          │PC1 │ │PC2 │   │3060│ │3060│
│128G│   │16G │ │16G │ │24G │          │32G │ │32G │   │12G │ │12G │
└────┘   └────┘ └────┘ └────┘          └────┘ └────┘   └────┘ └────┘
  |         |      |      |               |      |        |      |
 Hub     Worker  Worker  Worker        Worker  Worker  Worker  Worker
```

### 1.2 能力矩陣

| # | 名稱 | node_name | RAM | GPU/VRAM | 最大模型 | 推理速度 | 最佳用途 | max_concurrent |
|---|------|-----------|-----|----------|---------|---------|---------|----------------|
| 1 | Z13 | `z13` | 128GB | 8060S 96GB + NPU | 70B+ MoE | 72 t/s | Hub + 大模型 + routing | 3 |
| 2 | M1 Mac | `m1` | 16GB | Metal 16GB | 8B | 15 t/s | SoT 擴展 + 即時推理 | 2 |
| 3 | Ayaneo | `ayaneo` | 16GB | 680M 3GB | 4B | 10 t/s | 分類 + embedding | 2 |
| 4 | Acer | `acer` | 24GB + 7TB | MX350 2GB | 8B (CPU) | 5 t/s | 批次 + 儲存 + 備份 | 1 |
| 5 | Mini PC 1 | `mini1` | 32GB | 780M iGPU | 13B | 19 t/s | 中型推理 | 2 |
| 6 | Mini PC 2 | `mini2` | 32GB | 780M iGPU | 13B | 19 t/s | 中型推理 | 2 |
| 7 | RTX 3060 #1 | `gpu1` | 32GB+ | RTX 3060 12GB | 13B (CUDA) | 25 t/s | GPU 推理 | 2 |
| 8 | RTX 3060 #2 | `gpu2` | 32GB+ | RTX 3060 12GB | 13B (CUDA) | 25 t/s | GPU 推理 | 2 |

### 1.3 節點角色標籤

每個節點在 agents.toml 中宣告角色標籤，供 TaskScheduler 做親和性調度：

```toml
# Z13 的 agents.toml
[cluster]
mode = "hub"
node_name = "z13"
tags = ["hub", "large_model", "moe", "npu", "tool_calling", "coding"]

# RTX 3060 #1 的 agents.toml
[cluster]
mode = "worker"
node_name = "gpu1"
hub_address = "100.87.93.1:50051"
tags = ["gpu", "cuda", "13b", "quality_writing"]

# Ayaneo 的 agents.toml
[cluster]
mode = "worker"
node_name = "ayaneo"
hub_address = "100.87.93.1:50051"
tags = ["lightweight", "classify", "embed", "4b"]
```

### 1.4 模型部署表

| 節點 | 預載模型 | Provider | 用途 |
|------|---------|----------|------|
| z13 | qwen3.5-35b-moe (Q4_K_M) | lmstudio:1234 | 主力推理 |
| z13 | qwen3-coder-next (Q4_K_M) | lmstudio:1234 | tool calling + coding |
| z13 | nomic-embed-text | ollama:11434 | embedding 服務 |
| m1 | qwen3:8b | ollama:11434 | 即時推理 + SoT 擴展 |
| ayaneo | qwen3:4b | ollama:11434 | 快速分類 + 評分 |
| acer | qwen3:8b | ollama:11434 | 批次推理 (CPU) |
| mini1 | qwen3:8b / mistral:13b | ollama:11434 | 中型推理 |
| mini2 | qwen3:8b / mistral:13b | ollama:11434 | 中型推理 |
| gpu1 | qwen3:13b-cuda | ollama:11434 | 高品質寫作 |
| gpu2 | qwen3:13b-cuda | ollama:11434 | 高品質寫作 |
| (雲端) | gemini-2.5-flash | Gemini API | 大 context + overflow |

---

## 2. provider_hint 與 depends_on 配置規格

### 2.1 Phase 結構擴展

現有的 `Phase` struct 需要新增三個欄位來支援分散式調度：

```rust
/// src/hands/mod.rs — Phase 結構擴展
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Phase {
    pub name: String,
    pub system_prompt: String,
    #[serde(default = "default_max_rounds")]
    pub max_rounds: u32,
    #[serde(default)]
    pub condition: Option<String>,

    // ---- 新增欄位 ----

    /// 指定此 phase 應該路由到哪個節點或節點標籤
    /// 格式:
    ///   "node:gpu1"           — 指定節點
    ///   "tag:gpu"             — 任何有 gpu 標籤的節點
    ///   "tag:quality_writing" — 任何有 quality_writing 標籤的節點
    ///   "api:gemini"          — 使用雲端 API provider
    ///   "auto"                — 由 TaskScheduler 自動選擇 (預設)
    #[serde(default = "default_provider_hint")]
    pub provider_hint: String,

    /// 此 phase 依賴哪些其他 phase 完成後才能開始
    /// 格式: phase 名稱陣列
    /// 空陣列 = 依賴前一個 phase (預設串行行為)
    /// ["__none__"] = 無依賴，可立即開始
    /// ["phase_a", "phase_b"] = 等 A 和 B 都完成
    #[serde(default)]
    pub depends_on: Vec<String>,

    /// 此 phase 使用 SoT 並行生成 (僅限長文本 phase)
    /// 設為 true 時，TaskScheduler 會將此 phase 拆分為
    /// skeleton + N 個 section expansion 子任務
    #[serde(default)]
    pub use_sot: bool,
}

fn default_provider_hint() -> String { "auto".to_string() }
```

### 2.2 依賴關係圖解

```
depends_on 語義:

  depends_on = []              → 依賴前一個 phase (預設串行)
  depends_on = ["__none__"]    → 無依賴，可立即並行啟動
  depends_on = ["research"]    → 等 "research" phase 完成
  depends_on = ["a", "b"]     → 等 "a" 和 "b" 都完成 (join)

範例 — freelancer hand:
  search:    depends_on = ["__none__"]   ← 立即開始
  scoring:   depends_on = ["__none__"]   ← 立即開始（與 search 並行）
  proposals: depends_on = ["search", "scoring"]  ← 等兩者完成
  prepare:   depends_on = ["proposals"]
  review:    depends_on = ["prepare"]

DAG 圖:
  search ──────┐
               ├──> proposals ──> prepare ──> review
  scoring ─────┘
```

### 2.3 provider_hint 解析邏輯

```rust
/// TaskScheduler 中的 provider_hint 解析
fn resolve_provider_hint(
    hint: &str,
    workers: &HashMap<String, WorkerNode>,
) -> Vec<String> {
    if hint == "auto" || hint.is_empty() {
        // 回傳所有 Online 節點，按 running_tasks 升序
        return workers.values()
            .filter(|w| w.status == NodeStatus::Online)
            .sorted_by_key(|w| w.running_tasks)
            .map(|w| w.name.clone())
            .collect();
    }

    if let Some(node_name) = hint.strip_prefix("node:") {
        // 指定節點
        return vec![node_name.to_string()];
    }

    if let Some(tag) = hint.strip_prefix("tag:") {
        // 按標籤篩選
        return workers.values()
            .filter(|w| w.status == NodeStatus::Online)
            .filter(|w| w.tags.contains(&tag.to_string()))
            .sorted_by_key(|w| w.running_tasks)
            .map(|w| w.name.clone())
            .collect();
    }

    if let Some(api_name) = hint.strip_prefix("api:") {
        // 雲端 API — 在 Hub 上直接呼叫，不分配到 Worker
        return vec![format!("__api__:{}", api_name)];
    }

    vec!["auto".to_string()]
}
```

---

## 3. 工作流 1: Freelancer Hand 8 機協同版

### 3.1 執行流程圖

```
時間軸 ──────────────────────────────────────────────────────────>

Phase 0: search_jobs          Phase 2: write_proposals
┌─────────────────────┐       ┌──────────────────────────────┐
│ Z13 (Gemini Flash)  │       │ gpu1 (qwen3:13b-cuda)       │
│ 大 context 搜尋      │       │ 高品質提案撰寫               │
│ provider: api:gemini │       │ provider: tag:quality_writing│
│ ~30s                 │       │ ~120s                        │
└──────────┬──────────┘       └───────────────┬──────────────┘
           │                                  │
           │  ┌───────────────────────────┐   │
           │  │ Phase 1: score_jobs       │   │
           │  │ ayaneo (qwen3:4b)        │   │
           │  │ 快速 JSON 分類評分        │   │
           │  │ provider: node:ayaneo    │   │
           │  │ ~20s                      │   │
           │  └────────────┬──────────────┘   │
           │               │                  │
           └───────┬───────┘                  │
                   │ join                      │
                   v                           │
           (search + scoring 完成)             │
                   │                           │
                   └──────────> depends_on ────┘
                                              │
                                              v
                              Phase 3: prepare_application
                              ┌──────────────────────────┐
                              │ Z13 (Coder-Next)         │
                              │ tool calling + 檔案準備   │
                              │ provider: node:z13       │
                              │ ~60s                     │
                              └────────────┬─────────────┘
                                           │
                                           v
                              Phase 4: human_review
                              ┌──────────────────────────┐
                              │ mini1 (qwen3:8b)         │
                              │ 格式化 + 最終審核          │
                              │ provider: tag:lightweight │
                              │ ~30s                     │
                              └──────────────────────────┘

總計: ~260s (4.3 min)  vs 單機: ~24 min (5.6x 加速)
```

### 3.2 時序圖 (詳細)

```
 t=0s     t=20s    t=30s         t=150s        t=210s    t=240s
  |        |        |              |              |         |
  |<-search (Z13/Gemini, 30s)--->|              |         |
  |<-scoring (ayaneo, 20s)->|    |              |         |
  |        |        |       |    |              |         |
  |        |  join --+------+    |              |         |
  |        |        |            |              |         |
  |        |        |<--proposals (gpu1, 120s)->|         |
  |        |        |            |              |         |
  |        |        |            |    <-prepare (Z13, 60s)->|
  |        |        |            |              |         |
  |        |        |            |              | <-review (mini1, 30s)->
  |        |        |            |              |         |        |
  t=0      t=20     t=30        t=150          t=210    t=240   t=270
```

### 3.3 Hand TOML 配置

```toml
# ~/.clawtex/hands/freelancer/hand.toml

name = "freelancer"
description = "8-machine collaborative freelance job pipeline"
category = "revenue"
provider = "auto"
tools = [
    "web_search", "browser", "file_write", "file_read",
    "memory_store", "memory_recall", "http_request"
]
output_format = "markdown"

[settings]
target_platforms = "upwork,freelancer,toptal"
min_budget = "500"
max_proposals_per_run = "5"

# Phase 0: 搜尋工作 — Z13 透過 Gemini Flash API (大 context window)
[[phases]]
name = "search_jobs"
system_prompt = """You are a job search specialist. Search freelance platforms for
relevant software development jobs. Use web_search and browser tools.
Output a JSON array of jobs with: title, url, budget, skills, deadline, description."""
max_rounds = 5
provider_hint = "api:gemini"
depends_on = ["__none__"]

# Phase 1: 評分篩選 — Ayaneo (qwen3:4b, 快速分類)
[[phases]]
name = "score_jobs"
system_prompt = """You are a job scoring classifier. Given a list of jobs, score each
from 0-100 based on: budget fit, skill match, competition level, deadline feasibility.
Output ONLY a JSON array with: job_url, score, reasoning (1 sentence).
Filter to top 5 by score."""
max_rounds = 2
provider_hint = "node:ayaneo"
depends_on = ["__none__"]

# Phase 2: 撰寫提案 — RTX 3060 #1 (13B CUDA, 高品質寫作)
[[phases]]
name = "write_proposals"
system_prompt = """You are an expert freelance proposal writer. For each top-scored job,
write a compelling, personalized proposal. Include: hook, relevant experience,
proposed approach, timeline, budget justification. Use memory_recall for past work."""
max_rounds = 5
provider_hint = "tag:quality_writing"
depends_on = ["search_jobs", "score_jobs"]

# Phase 3: 準備申請 — Z13 (Coder-Next, tool calling)
[[phases]]
name = "prepare_application"
system_prompt = """You are an application preparation specialist. For each proposal:
1. Format the proposal as platform-ready text
2. Prepare any required attachments using file_write
3. Store application details in memory for dedup tracking
4. Recall past applications to avoid duplicates"""
max_rounds = 5
provider_hint = "node:z13"
depends_on = ["write_proposals"]

# Phase 4: 人工審核 — Mini PC 1 (8B, 格式化)
[[phases]]
name = "human_review"
system_prompt = """You are a quality reviewer. Review all prepared proposals:
1. Check for grammar and professionalism
2. Verify budget alignment
3. Format a summary table for human approval
4. Flag any concerns
Output the final review report with an approval recommendation per proposal."""
max_rounds = 3
provider_hint = "tag:lightweight"
depends_on = ["prepare_application"]
condition = "previous_success"
```

### 3.4 Hub 端調度偽碼

```
TaskScheduler::submit_hand(freelancer_hand, user_input):

  1. 解析 phase DAG:
     search_jobs:   deps = []  (immediate)
     score_jobs:    deps = []  (immediate)
     write_proposals: deps = [search_jobs, score_jobs]
     prepare_application: deps = [write_proposals]
     human_review: deps = [prepare_application]

  2. 建立 5 個 DistributedTask:
     task_0: search_jobs,   status=Queued, provider_hint="api:gemini"
     task_1: score_jobs,    status=Queued, provider_hint="node:ayaneo"
     task_2: write_proposals, status=Blocked, provider_hint="tag:quality_writing"
     task_3: prepare_application, status=Blocked, provider_hint="node:z13"
     task_4: human_review,  status=Blocked, provider_hint="tag:lightweight"

  3. task_0 和 task_1 立即可調度 (deps 為空)
     → task_0: resolve "api:gemini" → Hub 直接呼叫 GeminiProvider
     → task_1: resolve "node:ayaneo" → 分配到 ayaneo Worker

  4. 當 task_0 和 task_1 都完成:
     → task_2 解除 Blocked → Queued
     → resolve "tag:quality_writing" → 分配到 gpu1 (或 gpu2)

  5. 當 task_2 完成:
     → task_3 解除 Blocked → Queued → 分配到 z13

  6. 當 task_3 完成:
     → task_4 解除 Blocked → Queued → 分配到 mini1
```

---

## 4. 工作流 2: SEO Content 8 機並行版

### 4.1 執行流程圖

```
時間軸 ──────────────────────────────────────────────────────────────────>

Phase 0+1 並行:
┌─────────────────────┐
│ keyword_research    │
│ Z13 (Gemini Flash)  │
│ provider: api:gemini│
│ ~30s                │
└──────────┬──────────┘
           │
┌──────────┴──────────┐
│ competitor_analysis │
│ Z13 (Gemini Flash)  │
│ provider: api:gemini│
│ ~30s                │
└──────────┬──────────┘
           │
           │  (Phase 0 和 1 都完成)
           v

Phase 2: article_generation (SoT 8 機並行)
┌──────────────────────────────────────────────────────────────────────┐
│                                                                      │
│  Step 1: Outline (Z13)                                               │
│  ┌────────────────────────┐                                          │
│  │ Z13 → 生成 8 段大綱     │  ~15s                                   │
│  └───────────┬────────────┘                                          │
│              │                                                       │
│  Step 2: 並行擴展 (8 台機器各寫 1 段)                                  │
│  ┌───────┐┌───────┐┌───────┐┌───────┐┌───────┐┌───────┐┌───────┐┌───────┐
│  │Sec 1  ││Sec 2  ││Sec 3  ││Sec 4  ││Sec 5  ││Sec 6  ││Sec 7  ││Sec 8  │
│  │Z13    ││M1     ││Ayaneo ││Acer   ││Mini1  ││Mini2  ││GPU1   ││GPU2   │
│  │~30s   ││~40s   ││~50s   ││~90s   ││~35s   ││~35s   ││~30s   ││~30s   │
│  └───┬───┘└───┬───┘└───┬───┘└───┬───┘└───┬───┘└───┬───┘└───┬───┘└───┬───┘
│      │        │        │        │        │        │        │        │     │
│      └────────┴────────┴────────┴────┬───┴────────┴────────┴────────┘     │
│                                      │                                    │
│  Step 3: Merge (Z13)                 v                                    │
│  ┌───────────────────────────────────────┐                                │
│  │ Z13 → 合併 8 段，統一語氣風格          │  ~10s                          │
│  └───────────────────────────────────────┘                                │
│                                                                          │
│  SoT 總時間: 15 + max(30,40,50,90,35,35,30,30) + 10 = ~115s             │
│  vs 單機串行: 8 * 75s = ~600s (5.2x 加速)                                │
└──────────────────────────────────────────────────────────────────────────┘
           │
           v
Phase 3: seo_optimization
┌──────────────────────────┐
│ Mini PC 2 (qwen3:8b)    │
│ provider: tag:lightweight│
│ ~45s                     │
└──────────┬───────────────┘
           │
           v
Phase 4: publish_and_promote
┌──────────────────────────┐
│ Z13 (Coder-Next)         │
│ blog_publish + twitter   │
│ provider: node:z13       │
│ ~30s                     │
└──────────────────────────┘

總計: 30 + 115 + 45 + 30 = ~220s (3.7 min)  vs 單機: ~32 min (8.7x 加速)
```

### 4.2 SoT 8 機版時序圖

```
 t=0       t=30      t=45            t=135    t=180   t=210
  |         |         |                |        |       |
  |<-kw(Gem,30s)->|   |                |        |       |
  |<-comp(Gem,30s)->|  |                |        |       |
  |         |  join   |                |        |       |
  |         |         |                |        |       |
  |         |  outline(Z13,15s)        |        |       |
  |         |         |                |        |       |
  |         |         |<-- 8 機並行 -->|        |       |
  |         |         | Z13:   sec1 30s|        |       |
  |         |         | M1:    sec2 40s|        |       |
  |         |         | Aya:   sec3 50s|        |       |
  |         |         | Acer:  sec4 90s|<-max   |       |
  |         |         | Mini1: sec5 35s|        |       |
  |         |         | Mini2: sec6 35s|        |       |
  |         |         | GPU1:  sec7 30s|        |       |
  |         |         | GPU2:  sec8 30s|        |       |
  |         |         |                |        |       |
  |         |         |          merge(Z13,10s) |       |
  |         |         |                |        |       |
  |         |         |                | seo(mini2,45s) |
  |         |         |                |        |       |
  |         |         |                |    pub(Z13,30s)|
  |         |         |                |        |       |
  t=0      t=30      t=45           t=135    t=180    t=220
```

### 4.3 Hand TOML 配置

```toml
# ~/.clawtex/hands/seo_content/hand.toml

name = "seo_content"
description = "8-machine parallel SEO content pipeline with SoT"
category = "content"
provider = "auto"
tools = [
    "web_search", "file_write", "file_read", "memory_store",
    "memory_recall", "content_search", "blog_publish", "twitter",
    "skeleton_generate"
]
output_format = "markdown"

[settings]
target_word_count = "3000"
seo_keyword_density = "1.5-2.5%"
sot_sections = "8"

# Phase 0: 關鍵詞研究 — Gemini Flash (大 context + 免費)
[[phases]]
name = "keyword_research"
system_prompt = """Perform comprehensive keyword research for the given topic.
Use web_search to find: primary keywords, long-tail variants, search volume estimates,
keyword difficulty, related questions (People Also Ask).
Output structured JSON with keyword clusters."""
max_rounds = 4
provider_hint = "api:gemini"
depends_on = ["__none__"]

# Phase 1: 競爭分析 — Gemini Flash (並行)
[[phases]]
name = "competitor_analysis"
system_prompt = """Analyze top 10 search results for the primary keyword.
Use web_search to examine competitor content: word count, headings structure,
content gaps, backlink opportunities. Output a competitive analysis report."""
max_rounds = 4
provider_hint = "api:gemini"
depends_on = ["__none__"]

# Phase 2: 文章生成 — SoT 8 機並行
[[phases]]
name = "article_generation"
system_prompt = """Write a comprehensive, SEO-optimized article based on the keyword
research and competitor analysis. Target word count: 3000+. Include:
- Engaging introduction with primary keyword
- Detailed sections covering all keyword clusters
- Practical examples and actionable tips
- Internal and external link suggestions
- Conclusion with CTA
Use the skeleton_generate tool with 8 sections for parallel generation."""
max_rounds = 3
provider_hint = "node:z13"
depends_on = ["keyword_research", "competitor_analysis"]
use_sot = true

# Phase 3: SEO 優化 — Mini PC (輕量任務)
[[phases]]
name = "seo_optimization"
system_prompt = """Optimize the generated article for SEO:
1. Check keyword density (target 1.5-2.5%)
2. Add/fix meta title (under 60 chars) and description (under 160 chars)
3. Optimize heading hierarchy (H1 > H2 > H3)
4. Add alt text suggestions for images
5. Check internal linking opportunities via memory_recall
6. Output the final optimized article with frontmatter."""
max_rounds = 3
provider_hint = "tag:lightweight"
depends_on = ["article_generation"]

# Phase 4: 發布 + 推廣 — Z13 (需要 tool calling)
[[phases]]
name = "publish_and_promote"
system_prompt = """Publish and promote the SEO-optimized article:
1. Use blog_publish to publish to markl-ai.space
2. Use twitter to post a promotional thread (3-5 tweets)
3. Store the published URL and stats in memory for tracking
4. Output a publication report with URLs."""
max_rounds = 4
provider_hint = "node:z13"
depends_on = ["seo_optimization"]
condition = "min_length:1000"
```

### 4.4 SoT 並行擴展的節點分配

```
SkeletonConfig for 8-machine cluster:

  skeleton_provider = "node:z13"   (大綱由 Hub 生成)

  expansion_providers = [
      "node:z13",     # Section 1 — 最快
      "node:m1",      # Section 2 — Metal GPU
      "node:ayaneo",  # Section 3 — 輕量
      "node:acer",    # Section 4 — CPU (最慢，分配最短段)
      "node:mini1",   # Section 5
      "node:mini2",   # Section 6
      "node:gpu1",    # Section 7 — CUDA
      "node:gpu2",    # Section 8 — CUDA
  ]

Round-robin 分配: section.index % alive_providers.len()
但做智能分配: 最長的 section 分配給最快的節點

  sorted_sections_by_estimated_length (desc):
    longest  → z13 (72 t/s)
    2nd      → gpu1 (25 t/s)
    3rd      → gpu2 (25 t/s)
    4th      → mini1 (19 t/s)
    5th      → mini2 (19 t/s)
    6th      → m1 (15 t/s)
    7th      → ayaneo (10 t/s)
    shortest → acer (5 t/s)
```

---

## 5. 工作流 3: 大規模批次處理

### 5.1 場景: 100 封冷郵件生成

```
                    ┌──────────────────────────────┐
                    │   Hub TaskScheduler           │
                    │   100 EmailTask in queue      │
                    │   BatchJob { total: 100 }     │
                    └──────────────┬───────────────┘
                                   │
        ┌──────────────────────────┼──────────────────────────┐
        │          ┌───────────────┼───────────────┐          │
        │          │       ┌───────┼───────┐       │          │
        │          │       │       │       │       │          │
        v          v       v       v       v       v          v
   ┌─────────┐┌────────┐┌─────┐┌─────┐┌──────┐┌──────┐┌─────────┐
   │ Z13     ││GPU1    ││GPU2 ││Mini1││Mini2 ││ M1   ││ Acer    │
   │ 25 封    ││15 封   ││15封 ││15封 ││15封  ││10封  ││ 5 封    │
   │ 66 t/s  ││25 t/s  ││25t/s││19t/s││19t/s ││15t/s ││ 5 t/s   │
   │ ~3 min  ││~5 min  ││~5min││~6min││~6min ││~5min ││ ~8 min  │
   └─────────┘└────────┘└─────┘└─────┘└──────┘└──────┘└─────────┘

   全部完成: max(3, 5, 5, 6, 6, 5, 8) = ~8 min
   vs 單機 Z13: 100 * ~36s = ~60 min  (7.5x 加速)
```

### 5.2 分配演算法

```rust
/// 按照節點推理速度成比例分配批次任務
fn distribute_batch(
    total_tasks: usize,
    workers: &[WorkerNode],
) -> HashMap<String, usize> {
    // 計算總推理速度
    let total_speed: f64 = workers.iter()
        .map(|w| w.estimated_tokens_per_sec as f64)
        .sum();

    let mut allocation: HashMap<String, usize> = HashMap::new();
    let mut remaining = total_tasks;

    for (i, worker) in workers.iter().enumerate() {
        if i == workers.len() - 1 {
            // 最後一個節點分配所有剩餘
            allocation.insert(worker.name.clone(), remaining);
        } else {
            let share = ((worker.estimated_tokens_per_sec as f64
                / total_speed) * total_tasks as f64)
                .round() as usize;
            let share = share.min(remaining);
            allocation.insert(worker.name.clone(), share);
            remaining -= share;
        }
    }

    allocation
}
```

### 5.3 具體分配計算

```
節點速度:
  Z13:   66 t/s
  GPU1:  25 t/s
  GPU2:  25 t/s
  Mini1: 19 t/s
  Mini2: 19 t/s
  M1:    15 t/s
  Acer:   5 t/s
  ─────────────
  Total: 174 t/s

比例分配 100 封:
  Z13:   66/174 * 100 = 38 → 調整為 25 (Hub 要保留 capacity)
  GPU1:  25/174 * 100 = 14 → 15
  GPU2:  25/174 * 100 = 14 → 15
  Mini1: 19/174 * 100 = 11 → 15
  Mini2: 19/174 * 100 = 11 → 15
  M1:    15/174 * 100 =  9 → 10
  Acer:   5/174 * 100 =  3 →  5
                              ───
                              100

每封郵件預估 token: ~500 output tokens
每封預估時間: 500 / speed

  Z13:   25 * (500/66)  = 25 * 7.6s  = 189s  (3.2 min)
  GPU1:  15 * (500/25)  = 15 * 20s   = 300s  (5.0 min)
  GPU2:  15 * (500/25)  = 15 * 20s   = 300s  (5.0 min)
  Mini1: 15 * (500/19)  = 15 * 26s   = 395s  (6.6 min)
  Mini2: 15 * (500/19)  = 15 * 26s   = 395s  (6.6 min)
  M1:    10 * (500/15)  = 10 * 33s   = 333s  (5.6 min)
  Acer:   5 * (500/5)   =  5 * 100s  = 500s  (8.3 min)

  Max = 8.3 min (受限於 Acer)

最佳化: 不分配給 Acer → 重新分配 5 封到 Z13 (Z13 總共 30 封)
  Z13:  30 * 7.6s  = 228s (3.8 min)
  Acer: 0
  Max = 6.6 min (Mini PC 成為瓶頸)
```

### 5.4 批次任務 TOML (outreach hand 擴展)

```toml
# ~/.clawtex/hands/outreach/hand.toml 中的批次 phase

[[phases]]
name = "generate_emails"
system_prompt = """Generate a personalized cold outreach email for the given prospect.
Include: subject line, greeting, value proposition, social proof, CTA.
Recall brand_voice_profile from memory for consistent tone."""
max_rounds = 2
provider_hint = "auto"
depends_on = ["prospect_scoring"]

# 新增 Hand-level 設定
[settings]
batch_mode = "parallel"
batch_distribute = "proportional"
exclude_slow_nodes = "acer"
```

---

## 6. 工作流 4: 即時對話低延遲路徑

### 6.1 路由決策樹

```
Telegram 訊息到達 Hub
          │
          v
    ┌──────────────┐
    │  訊息分類     │  (regex + 簡易規則，零 LLM 成本)
    │  < 1ms       │
    └──────┬───────┘
           │
     ┌─────┴──────┐──────────────────┐
     │            │                  │
     v            v                  v
  [簡單對話]   [工具任務]          [Hand 觸發]
     │            │                  │
     v            v                  v
  快速路徑     標準路徑            調度路徑
     │            │                  │
     v            v                  v

快速路徑 (< 1s 目標):
  ┌────────────────────────────────────────────────┐
  │                                                │
  │  1. Z13 本地 GPU 閒置?                          │
  │     YES → Z13 LMStudio Qwen3.5-35B (72 t/s)   │
  │           預估: 200 tokens / 72 = 2.8s          │
  │                                                │
  │  2. Z13 忙碌? → 檢查 Mini PC                    │
  │     Mini1 閒? → Mini1 Ollama qwen3:8b (19 t/s) │
  │     Mini2 閒? → Mini2 Ollama qwen3:8b (19 t/s) │
  │           預估: 200 tokens / 19 = 10.5s         │
  │                                                │
  │  3. Mini PC 都忙? → M1 Mac                      │
  │     M1 閒? → M1 Ollama qwen3:8b (15 t/s)       │
  │           預估: 200 tokens / 15 = 13.3s         │
  │                                                │
  │  4. 全部忙碌? → Gemini Flash API (overflow)     │
  │     Gemini Flash Lite → ~2s 回覆                │
  │     (免費額度 1000 req/day)                      │
  │                                                │
  └────────────────────────────────────────────────┘
```

### 6.2 路由實作

```rust
/// 即時對話的低延遲路由
/// 檢查順序: Z13本地 → MiniPC → M1 → GPU → Gemini overflow
async fn route_chat_low_latency(
    &self,
    prompt: &str,
    workers: &HashMap<String, WorkerNode>,
) -> (String, String) {  // (node_name, provider)

    // 優先級排序: 按 response latency 而非 throughput
    let priority_order = [
        ("z13",   "lmstudio"),  // 72 t/s, 本地
        ("mini1", "ollama"),    // 19 t/s
        ("mini2", "ollama"),    // 19 t/s
        ("m1",    "ollama"),    // 15 t/s
        ("gpu1",  "ollama"),    // 25 t/s (但通常留給長任務)
        ("gpu2",  "ollama"),    // 25 t/s
    ];

    for (node, provider) in &priority_order {
        if let Some(w) = workers.get(*node) {
            if w.status == NodeStatus::Online
                && w.running_tasks < w.max_concurrent
            {
                return (node.to_string(), provider.to_string());
            }
        }
    }

    // 全部忙碌 → overflow 到 Gemini Flash
    ("__api__".to_string(), "gemini".to_string())
}
```

### 6.3 Streaming 路徑

```
使用者 → Telegram → Hub
                     │
                     ├─ [本地] Z13 LMStudio → SSE stream → Telegram edit_message
                     │   延遲: first token ~0.2s, 完成 ~2.8s
                     │
                     ├─ [遠端] Mini1 Ollama → gRPC stream → Hub → Telegram
                     │   延遲: first token ~0.5s (含網路), 完成 ~10.5s
                     │
                     └─ [雲端] Gemini Flash → HTTP stream → Hub → Telegram
                         延遲: first token ~0.3s, 完成 ~2s

所有路徑都透過 Telegram edit_message 做 progressive update:
  [第1段] → edit → [第1段+第2段] → edit → [完整回覆]
```

---

## 7. 工作流 5: SaaS Pipeline 全流程

### 7.1 完整 Pipeline 流程圖

```
/product "AI resume optimizer"
          │
          v
    ┌─────────────────────────────────────────────────────────────┐
    │                    product_spec Hand                        │
    │                                                             │
    │  Phase 1: market_analysis                                   │
    │  ┌──────────────────────────────────┐                       │
    │  │ Z13 + Gemini Flash               │                       │
    │  │ 大 context 市場分析               │                       │
    │  │ provider: api:gemini             │  ~60s                 │
    │  └──────────────┬───────────────────┘                       │
    │                 v                                           │
    │  Phase 2: api_design                                        │
    │  ┌──────────────────────────────────┐                       │
    │  │ Z13 (Coder-Next)                 │                       │
    │  │ API endpoint 設計                 │                       │
    │  │ provider: node:z13               │  ~45s                 │
    │  └──────────────┬───────────────────┘                       │
    │                 v                                           │
    │  Phase 3: pricing_strategy                                  │
    │  ┌──────────────────────────────────┐                       │
    │  │ Mini PC 1 (8B)                    │                       │
    │  │ 定價策略                           │                       │
    │  │ provider: tag:lightweight         │  ~30s                 │
    │  └──────────────┬───────────────────┘                       │
    │                 v                                           │
    │  Phase 4: spec_output                                       │
    │  ┌──────────────────────────────────┐                       │
    │  │ Z13 (file_write 工具)             │                       │
    │  │ 輸出 spec.json                    │                       │
    │  │ provider: node:z13               │  ~20s                 │
    │  └──────────────┬───────────────────┘                       │
    │                 │                                           │
    │  chain_to = "code_gen"                                      │
    └─────────────────┼───────────────────────────────────────────┘
                      v
    ┌─────────────────────────────────────────────────────────────┐
    │                    code_gen Hand                            │
    │                                                             │
    │  Phase 1: scaffold                                          │
    │  ┌──────────────────────────────────┐                       │
    │  │ Z13 (scaffold_saas tool)          │                       │
    │  │ 生成專案骨架                       │                       │
    │  │ provider: node:z13               │  ~10s                 │
    │  └──────────────┬───────────────────┘                       │
    │                 v                                           │
    │  Phase 2: implement                                         │
    │  ┌──────────────────────────────────┐                       │
    │  │ Z13 (Claude Code subprocess)      │                       │
    │  │ 實作核心功能                       │                       │
    │  │ provider: node:z13               │  ~300s (5 min)        │
    │  └──────────────┬───────────────────┘                       │
    │                 v                                           │
    │  Phase 3: auth_and_billing                                  │
    │  ┌──────────────────────────────────┐                       │
    │  │ Z13 (Claude Code subprocess)      │                       │
    │  │ 認證 + Stripe 整合                │                       │
    │  │ provider: node:z13               │  ~180s (3 min)        │
    │  └──────────────┬───────────────────┘                       │
    │                 v                                           │
    │  Phase 4: testing (分散到多機)                                │
    │  ┌──────────────────────────────────────────────────────┐   │
    │  │ 並行測試:                                              │   │
    │  │   Mini1: unit tests          provider: node:mini1    │   │
    │  │   Mini2: integration tests   provider: node:mini2    │   │
    │  │   GPU1:  load tests          provider: node:gpu1     │   │
    │  │ depends_on = ["auth_and_billing"]                     │   │
    │  │ ~120s (受限於最慢的測試)                                │   │
    │  └──────────────┬───────────────────────────────────────┘   │
    │                 v                                           │
    │  Phase 5: package                                           │
    │  ┌──────────────────────────────────┐                       │
    │  │ Z13 (Docker build)                │                       │
    │  │ 打包 + 產生 Docker image          │                       │
    │  │ provider: node:z13               │  ~60s                 │
    │  └──────────────┬───────────────────┘                       │
    │                 │                                           │
    │  chain_to = "saas_deploy"                                   │
    └─────────────────┼───────────────────────────────────────────┘
                      v
    ┌─────────────────────────────────────────────────────────────┐
    │                    saas_deploy Hand                         │
    │                                                             │
    │  Phase 1: github_push        → Z13    (provider: node:z13)  │
    │  Phase 2: render_deploy      → Z13    (provider: node:z13)  │
    │  Phase 3: stripe_setup       → Z13    (provider: node:z13)  │
    │  Phase 4: verify             → Mini2  (provider: node:mini2)│
    │  Phase 5: announce           → Z13    (provider: node:z13)  │
    │                                                             │
    │  (deploy 全部在 Z13 因為需要 git/docker/stripe CLI)          │
    │  (verify 在 Mini2 做獨立健康檢查)                            │
    └─────────────────────────────────────────────────────────────┘

Pipeline 總時間:
  product_spec:  60 + 45 + 30 + 20            = ~155s  (2.6 min)
  code_gen:      10 + 300 + 180 + 120 + 60    = ~670s  (11.2 min)
  saas_deploy:   30 + 60 + 30 + 30 + 30       = ~180s  (3.0 min)
  ───────────────────────────────────────────────────────────────
  總計:                                         ~1005s  (16.8 min)

  vs 單機估計: ~45 min (2.7x 加速)
  (code_gen 是瓶頸，因為 Claude Code subprocess 本身就很快)
```

### 7.2 測試分散化 (Phase 4 詳細)

```
code_gen Phase 4 拆分為 3 個並行子任務:

  ┌────────────────┐  ┌────────────────┐  ┌────────────────┐
  │ Mini1           │  │ Mini2           │  │ GPU1            │
  │ unit tests      │  │ integration     │  │ load tests      │
  │                 │  │ tests           │  │                 │
  │ npm test        │  │ npm run test:   │  │ wrk/k6 壓測    │
  │ --unit          │  │ integration     │  │                 │
  │ ~40s            │  │ ~90s            │  │ ~120s           │
  └───────┬─────────┘  └───────┬─────────┘  └───────┬─────────┘
          │                    │                    │
          └────────────────────┼────────────────────┘
                               │ join (等所有測試完成)
                               v
                     (所有測試通過 → Phase 5)

依賴配置:
  [[phases]]
  name = "testing_unit"
  provider_hint = "node:mini1"
  depends_on = ["auth_and_billing"]

  [[phases]]
  name = "testing_integration"
  provider_hint = "node:mini2"
  depends_on = ["auth_and_billing"]

  [[phases]]
  name = "testing_load"
  provider_hint = "node:gpu1"
  depends_on = ["auth_and_billing"]

  [[phases]]
  name = "package"
  depends_on = ["testing_unit", "testing_integration", "testing_load"]
```

---

## 8. 負載感知路由演算法

### 8.1 架構圖

```
    每 10s heartbeat
    ┌────────────┐
    │ Worker #1  │──── { pending: 0, cpu: 23%, gpu: 45%, speed: 15 t/s }
    │ Worker #2  │──── { pending: 1, cpu: 78%, gpu: 89%, speed: 10 t/s }
    │ Worker #3  │──── { pending: 0, cpu: 12%, gpu: 0%,  speed: 5 t/s  }
    │ Worker #4  │──── { pending: 2, cpu: 95%, gpu: 92%, speed: 19 t/s }
    │ Worker #5  │──── { pending: 0, cpu: 34%, gpu: 67%, speed: 19 t/s }
    │ Worker #6  │──── { pending: 1, cpu: 56%, gpu: 78%, speed: 25 t/s }
    │ Worker #7  │──── { pending: 0, cpu: 45%, gpu: 55%, speed: 25 t/s }
    └──────┬─────┘
           │
           v
    ┌──────────────────────────────────────────────┐
    │            Hub LoadBalancer                   │
    │                                              │
    │  Node Score = w1 * (1 - pending/max)         │
    │             + w2 * (1 - gpu_util/100)        │
    │             + w3 * speed / max_speed         │
    │             + w4 * model_affinity_bonus      │
    │                                              │
    │  w1=0.35, w2=0.25, w3=0.25, w4=0.15         │
    │                                              │
    │  新任務 → 分配到 score 最高的節點              │
    │  全部 score < 0.2 → overflow 到 Gemini Flash │
    └──────────────────────────────────────────────┘
```

### 8.2 評分公式

```rust
/// 計算節點的調度評分 (0.0 ~ 1.0)
fn compute_node_score(
    worker: &WorkerNode,
    heartbeat: &HeartbeatReport,
    task: &DistributedTask,
    max_cluster_speed: f64,
) -> f64 {
    const W_LOAD: f64 = 0.35;
    const W_GPU: f64 = 0.25;
    const W_SPEED: f64 = 0.25;
    const W_AFFINITY: f64 = 0.15;

    // 負載分數: pending 越少越好
    let load_score = 1.0
        - (worker.running_tasks as f64 / worker.max_concurrent as f64);

    // GPU 使用率: 越低越好 (有 GPU 的節點)
    let gpu_score = match heartbeat.system_stats.gpu_percent {
        Some(pct) => 1.0 - (pct as f64 / 100.0),
        None => 0.5,  // 無 GPU 節點給中性分數
    };

    // 推理速度: 相對於叢集最快節點
    let speed_score = worker.estimated_tokens_per_sec as f64
        / max_cluster_speed;

    // 模型親和性: 節點已載入所需模型 → bonus
    let affinity_score = if let Some(ref model) = task.model_hint {
        if worker.loaded_models.contains(model) { 1.0 } else { 0.0 }
    } else {
        0.5  // 無模型偏好
    };

    W_LOAD * load_score
        + W_GPU * gpu_score
        + W_SPEED * speed_score
        + W_AFFINITY * affinity_score
}
```

### 8.3 Overflow 到雲端 API 的條件

```
觸發 overflow 的條件 (任一):

  1. 所有節點 score < 0.2 (全部繁忙)
  2. 所有節點 running_tasks == max_concurrent
  3. 任務是即時對話 + 所有本地節點 first-token 預估 > 3s
  4. 任務需要 > 100K context window (只有 Gemini 能處理)

Overflow 優先級:
  1. Gemini 2.5 Flash      (250 req/day, 免費)
  2. Gemini Flash Lite      (1000 req/day, 免費)
  3. Groq Llama 70B         (1000 req/day, 免費)
  4. OpenRouter free models (有額度時)

每日額度追蹤:
  Hub 維護一個 in-memory counter:
    gemini_flash_used_today: AtomicU32
    gemini_lite_used_today: AtomicU32
    groq_used_today: AtomicU32

  每天 UTC 00:00 重置
  超出額度 → 降級到下一個 provider
  全部超出 → 排隊等本地節點
```

### 8.4 Telegram 負載狀態顯示

```
/cluster 命令輸出:

  Clawtex 8-Node Cluster Status
  ─────────────────────────────────
  NODE      STATUS  TASKS  GPU%  SCORE
  z13       Online  1/3    45%   0.72
  m1        Online  0/2    --    0.85
  ayaneo    Online  1/2    12%   0.61
  acer      Online  0/1    --    0.53
  mini1     Online  2/2    67%   0.28
  mini2     Online  0/2    55%   0.64
  gpu1      Busy    2/2    92%   0.15
  gpu2      Online  1/2    78%   0.45
  ─────────────────────────────────
  Queued: 3  Running: 7  Today: 142 tasks
  Cloud overflow: Gemini 45/250, Groq 12/1000
```

---

## 9. 資料流設計

### 9.1 Hand Output 從 Worker 回傳 Hub

```
Worker 執行 Phase 完成:

  Worker 端:
    1. Agent loop 完成 → 取得 final_output (String)
    2. 掃描 /tmp/clawtex_task_{id}/ 目錄:
       - 工具產出的檔案 (file_write)
       - 生成的 PDF (pdf_export)
       - 圖片或其他二進位

  回傳路徑 A: 小資料 (< 1MB) — 內嵌在 gRPC TaskResult 中

    Worker ──[gRPC CompleteTask]──> Hub
    {
      task_id: "task_123",
      result: TaskResultPayload {
        output: "完整的 phase 輸出文字...",  // 直接內嵌
        tokens_in: 1200,
        tokens_out: 800,
        output_files: [],  // 無大檔案
        ...
      }
    }

  回傳路徑 B: 大資料 (>= 1MB) — 先串流上傳檔案，再回報完成

    Step 1: Worker 上傳檔案
    Worker ──[gRPC UploadFile stream]──> Hub
    {
      task_id: "task_123",
      filename: "report.pdf",
      chunks: [chunk1, chunk2, chunk3, ...]  // 每 chunk 64KB
    }
    Hub 接收 → 寫入 ~/.clawtex/workspace/task_123/report.pdf

    Step 2: Worker 回報完成 (帶檔案清單)
    Worker ──[gRPC CompleteTask]──> Hub
    {
      task_id: "task_123",
      result: TaskResultPayload {
        output: "報告已生成...",
        output_files: [
          { relative_path: "report.pdf", size_bytes: 2500000, content_type: "application/pdf" }
        ],
        ...
      }
    }

  Hub 後續處理:
    1. 更新 DistributedTask status = Done
    2. 記錄 CostRecord
    3. 如果是 Hand phase → 用 output 作為下一 phase 的 context
    4. 如果是 chain_to → 用 final_output 觸發下一 Hand
    5. 通知 Telegram 使用者
    6. 如果有檔案 → Telegram 附件發送
```

### 9.2 大檔案處理流程

```
場景: PDF Export (5MB) 在 GPU1 上生成

  GPU1 Worker:
    ┌──────────────────────────────┐
    │ pdf_export tool 執行          │
    │ pandoc → output.pdf (5MB)    │
    │ 存到 /tmp/clawtex_task_abc/  │
    └──────────────┬───────────────┘
                   │
                   v
    ┌──────────────────────────────┐
    │ 檔案串流上傳 (gRPC)           │
    │                              │
    │ FileChunk {                  │
    │   task_id: "abc",            │
    │   filename: "output.pdf",    │
    │   chunk_index: 0,            │
    │   total_chunks: 79,          │  (5MB / 64KB = 79 chunks)
    │   data: [64KB bytes],        │
    │ }                            │
    │                              │
    │ 頻寬: Tailscale ~100Mbps     │
    │ 傳輸時間: 5MB / 100Mbps      │
    │          = ~0.4s             │
    └──────────────┬───────────────┘
                   │
                   v
    ┌──────────────────────────────┐
    │ Hub 接收                      │
    │ → ~/.clawtex/workspace/      │
    │   └── abc/                   │
    │       └── output.pdf (5MB)   │
    └──────────────────────────────┘

備份策略 (Acer 7TB):
  Hub 每日 cron → rsync workspace/ 到 Acer
  Acer 保留 30 天歷史
  gRPC UploadFile 也可直接寫入 Acer (雙寫)
```

### 9.3 狀態同步機制

```
                    ┌────────────────────────┐
                    │     Hub (Z13)          │
                    │                        │
                    │  ┌──────────────────┐  │
                    │  │ StateManager     │  │
                    │  │                  │  │
                    │  │ ┌──────────────┐ │  │
                    │  │ │ TaskScheduler│ │  │  ← 任務佇列 (SQLite WAL)
                    │  │ │ (SQLite)     │ │  │
                    │  │ └──────────────┘ │  │
                    │  │ ┌──────────────┐ │  │
                    │  │ │ MemoryStore  │ │  │  ← 語義記憶 (PostgreSQL+pgvector)
                    │  │ │ (PG+pgvec)  │ │  │
                    │  │ └──────────────┘ │  │
                    │  │ ┌──────────────┐ │  │
                    │  │ │ CostTracker  │ │  │  ← 成本追蹤 (SQLite)
                    │  │ │ (SQLite)     │ │  │
                    │  │ └──────────────┘ │  │
                    │  │ ┌──────────────┐ │  │
                    │  │ │ WorkerCache  │ │  │  ← 節點狀態 (in-memory)
                    │  │ │ (HashMap)    │ │  │
                    │  │ └──────────────┘ │  │
                    │  └──────────────────┘  │
                    └───────────┬────────────┘
                                │
            ┌───────────────────┼───────────────────┐
            │                   │                   │
            v                   v                   v
    ┌──────────────┐   ┌──────────────┐    ┌──────────────┐
    │ Worker 1     │   │ Worker 2     │    │ Worker N     │
    │ (無狀態)      │   │ (無狀態)      │    │ (無狀態)      │
    │              │   │              │    │              │
    │ 只做:         │   │ 只做:         │    │ 只做:         │
    │ 1. LLM 推理  │   │ 1. LLM 推理  │    │ 1. LLM 推理  │
    │ 2. 工具執行   │   │ 2. 工具執行   │    │ 2. 工具執行   │
    │ 3. 結果回報   │   │ 3. 結果回報   │    │ 3. 結果回報   │
    └──────────────┘   └──────────────┘    └──────────────┘

同步機制:

  1. 任務狀態:  Hub 是唯一真相來源
     Worker poll → Hub 回應 → Worker 執行 → Worker 回報 → Hub 更新
     無需同步 (Worker 不保存任務狀態)

  2. Memory:    Hub 是唯一真相來源
     Worker 需要 recall → gRPC 查 Hub → Hub 查 PG → 回傳結果
     Worker 需要 store  → gRPC 寫 Hub → Hub 寫 PG → 確認
     無需同步 (Worker 不保存 memory)

  3. E-Stop:    Hub push → Worker subscribe
     Hub 狀態變更 → gRPC server streaming → 所有 Worker
     延遲 < 100ms (Tailscale 低延遲)

  4. Heartbeat: Worker push → Hub collect
     Worker 每 10s → gRPC → Hub 更新 WorkerCache
     Hub 30s 無 heartbeat → 標記 Worker 為 Offline

  5. 檔案:      Worker push → Hub store
     任務完成 → Worker 上傳 → Hub 存 workspace/
     無需雙向同步
```

### 9.4 一致性保證摘要

```
┌─────────────────┬───────────────┬──────────────────────────────────┐
│ 資料類型         │ 一致性模型     │ 機制                              │
├─────────────────┼───────────────┼──────────────────────────────────┤
│ Task 狀態        │ 強一致         │ Hub SQLite WAL, Worker poll      │
│ Memory 讀寫      │ 強一致         │ Hub PostgreSQL, Worker proxy     │
│ Cost 記錄        │ 最終一致 (~10s)│ Worker heartbeat 上報, Hub 寫入   │
│ E-Stop           │ 強一致 (<100ms)│ Hub gRPC push, AtomicBool       │
│ 節點狀態          │ 最終一致 (~10s)│ Heartbeat 週期更新               │
│ 檔案             │ 最終一致       │ 任務完成時上傳                    │
│ Revenue 記錄     │ 強一致         │ Hub SQLite, 只有 Hub 寫入        │
└─────────────────┴───────────────┴──────────────────────────────────┘
```

---

## 10. Hand TOML 擴展格式

### 10.1 完整的 Phase 擴展欄位

```toml
# 新增的 Phase 欄位（向後相容，所有新欄位都有預設值）

[[phases]]
name = "phase_name"
system_prompt = "..."
max_rounds = 5

# 現有欄位
condition = "previous_success"

# 新增: 分散式調度
provider_hint = "auto"          # 預設: TaskScheduler 自動選擇
depends_on = []                 # 預設: 依賴前一個 phase (串行)
use_sot = false                 # 預設: 不使用 SoT 並行

# provider_hint 語法:
#   "auto"                → TaskScheduler 負載均衡
#   "node:z13"            → 指定節點
#   "tag:gpu"             → 任何 GPU 節點
#   "tag:quality_writing" → 任何高品質寫作節點
#   "api:gemini"          → Gemini Flash API
#   "api:groq"            → Groq API
#   "local"               → 強制 Hub 本地執行 (不分發)

# depends_on 語法:
#   []                    → 依賴前一個 phase (預設)
#   ["__none__"]          → 無依賴，可立即並行
#   ["phase_a"]           → 等 phase_a 完成
#   ["phase_a", "phase_b"]→ 等 a 和 b 都完成 (join)
```

### 10.2 Hand-level 新增設定

```toml
# Hand-level 新增欄位

name = "example_hand"
description = "..."
category = "..."
provider = "auto"
tools = [...]
output_format = "markdown"

# 新增: 批次模式設定
[settings]
batch_mode = "parallel"         # "sequential" (預設) | "parallel"
batch_distribute = "proportional" # "proportional" | "round_robin" | "fastest_first"
exclude_slow_nodes = ""         # 逗號分隔的節點名，不參與此 hand
priority = "5"                  # 0=low, 5=normal, 10=urgent

# 新增: SoT 設定 (hand-level override)
sot_sections = "8"              # SoT 段落數
sot_skeleton_node = "z13"       # 大綱生成節點
```

---

## 11. Rust 實作要點

### 11.1 Phase 依賴解析器

```rust
/// 將 Hand 的 phases 解析為 DAG，確定哪些可以並行
pub fn build_phase_dag(hand: &Hand) -> Vec<Vec<usize>> {
    // 回傳: 每一層的 phase 索引
    // 同一層內的 phases 可以並行執行
    // 層之間必須串行

    let mut phase_map: HashMap<&str, usize> = HashMap::new();
    for (i, phase) in hand.phases.iter().enumerate() {
        phase_map.insert(&phase.name, i);
    }

    // 建立依賴圖
    let mut deps: Vec<Vec<usize>> = vec![Vec::new(); hand.phases.len()];

    for (i, phase) in hand.phases.iter().enumerate() {
        if phase.depends_on.is_empty() {
            // 預設: 依賴前一個 phase
            if i > 0 {
                deps[i].push(i - 1);
            }
        } else if phase.depends_on == vec!["__none__"] {
            // 無依賴
            deps[i] = Vec::new();
        } else {
            // 明確依賴
            for dep_name in &phase.depends_on {
                if let Some(&dep_idx) = phase_map.get(dep_name.as_str()) {
                    deps[i].push(dep_idx);
                }
            }
        }
    }

    // 拓撲排序分層 (Kahn's algorithm)
    let n = hand.phases.len();
    let mut in_degree = vec![0usize; n];
    for d in &deps {
        // in_degree 是被依賴的數量
    }
    // ... (標準拓撲排序)

    // 結果範例: [[0, 1], [2], [3], [4]]
    // 第0層: phase 0 和 1 可並行
    // 第1層: phase 2 (等 0,1 完成)
    // 第2層: phase 3
    // 第3層: phase 4
    todo!()
}
```

### 11.2 TaskScheduler 擴展

```rust
/// TaskScheduler::submit_hand 的新實作 (支援並行 phase)
pub async fn submit_hand_distributed(
    &self,
    hand: &Hand,
    user_input: &str,
    initiator: &str,
) -> Result<String> {
    let root_id = uuid::Uuid::new_v4().to_string();
    let context = HandRunner::prepare_context(hand, user_input);

    // 建立所有 phase 的 DistributedTask
    for (i, phase) in hand.phases.iter().enumerate() {
        let status = if phase.depends_on == vec!["__none__"]
            || (phase.depends_on.is_empty() && i == 0)
        {
            DistributedTaskStatus::Queued  // 立即可調度
        } else {
            DistributedTaskStatus::Blocked  // 等依賴完成
        };

        let task = DistributedTask {
            task_id: format!("{}_phase_{}", root_id, i),
            parent_id: Some(root_id.clone()),
            hand_name: Some(hand.name.clone()),
            phase_index: Some(i as u32),
            total_phases: Some(hand.phases.len() as u32),
            prompt: if i == 0 { context.clone() } else { String::new() },
            system_prompt: Some(phase.system_prompt.clone()),
            tools: hand.tools.clone(),
            context: None,  // 由前一 phase 完成時填入
            status,
            priority: hand.settings.get("priority")
                .and_then(|s| s.parse().ok())
                .unwrap_or(5),
            assigned_node: None,
            provider_hint: Some(phase.provider_hint.clone()),
            max_rounds: phase.max_rounds,
            // ...
        };

        self.db.insert_task(&task)?;
    }

    Ok(root_id)
}

/// 當 phase 完成時，檢查下游依賴是否可以解除 Blocked
pub async fn on_phase_complete(
    &self,
    completed_task: &DistributedTask,
) -> Result<Vec<String>> {
    let hand_name = completed_task.hand_name.as_deref()
        .ok_or_else(|| anyhow!("Not a hand task"))?;
    let parent_id = completed_task.parent_id.as_deref()
        .ok_or_else(|| anyhow!("No parent"))?;

    // 取得同一 Hand 的所有 phase tasks
    let siblings = self.db.get_tasks_by_parent(parent_id)?;

    // 載入 Hand 定義取得依賴資訊
    let hand = self.hand_registry.get(hand_name)
        .ok_or_else(|| anyhow!("Hand not found: {}", hand_name))?;

    let mut unblocked = Vec::new();

    for sibling in &siblings {
        if sibling.status != DistributedTaskStatus::Blocked {
            continue;
        }

        let phase_idx = sibling.phase_index.unwrap_or(0) as usize;
        let phase = &hand.phases[phase_idx];

        // 檢查此 phase 的所有依賴是否已完成
        let all_deps_done = if phase.depends_on.is_empty() {
            // 預設依賴前一 phase
            phase_idx == 0 || siblings.iter().any(|s|
                s.phase_index == Some((phase_idx - 1) as u32)
                && s.status == DistributedTaskStatus::Done
            )
        } else if phase.depends_on == vec!["__none__"] {
            true
        } else {
            phase.depends_on.iter().all(|dep_name| {
                let dep_idx = hand.phases.iter()
                    .position(|p| p.name == *dep_name);
                dep_idx.map(|idx| {
                    siblings.iter().any(|s|
                        s.phase_index == Some(idx as u32)
                        && s.status == DistributedTaskStatus::Done
                    )
                }).unwrap_or(false)
            })
        };

        if all_deps_done {
            // 解除 Blocked，注入前一 phase 的 output 作為 context
            let context = completed_task.result.as_ref()
                .map(|r| r.output.clone())
                .unwrap_or_default();

            self.db.update_task_status(
                &sibling.task_id,
                DistributedTaskStatus::Queued,
            )?;
            self.db.update_task_context(
                &sibling.task_id,
                &context,
            )?;

            unblocked.push(sibling.task_id.clone());
        }
    }

    Ok(unblocked)
}
```

### 11.3 SoT 8 機版擴展

```rust
/// SkeletonRunner 擴展: 支援指定節點的 expansion
impl SkeletonRunner {
    /// 8-machine parallel expansion
    /// 按 section 預估長度 × 節點速度做智能分配
    pub async fn expand_distributed(
        &self,
        topic: &str,
        sections: &[SkeletonSection],
        workers: &[WorkerNode],
    ) -> Vec<SectionResult> {
        // 按速度降序排列節點
        let mut sorted_workers: Vec<&WorkerNode> = workers.iter()
            .filter(|w| w.status == NodeStatus::Online)
            .collect();
        sorted_workers.sort_by(|a, b|
            b.estimated_tokens_per_sec.cmp(&a.estimated_tokens_per_sec)
        );

        // 按預估長度降序排列 sections
        let mut section_indices: Vec<usize> = (0..sections.len()).collect();
        section_indices.sort_by(|a, b|
            sections[*b].description.len().cmp(&sections[*a].description.len())
        );

        // 最長 section → 最快節點
        let mut assignments: Vec<(usize, String)> = Vec::new();
        for (i, &sec_idx) in section_indices.iter().enumerate() {
            let worker = &sorted_workers[i % sorted_workers.len()];
            assignments.push((sec_idx, worker.name.clone()));
        }

        // 並行執行
        let mut handles = Vec::new();
        for (sec_idx, node_name) in assignments {
            let section = sections[sec_idx].clone();
            let topic = topic.to_string();
            let node = node_name.clone();
            // 透過 Hub 的 TaskScheduler 分發到指定節點
            let handle = tokio::spawn(async move {
                // Hub → gRPC → Worker → LLM → result
                todo!()
            });
            handles.push(handle);
        }

        // 收集結果
        let mut results = Vec::new();
        for handle in handles {
            if let Ok(result) = handle.await {
                results.push(result);
            }
        }
        results
    }
}
```

### 11.4 agents.toml 完整配置範例

```toml
# Z13 Hub 的 ~/.clawtex/agents.toml

[cluster]
mode = "hub"
node_name = "z13"
grpc_port = 50051
heartbeat_interval_secs = 10
task_timeout_secs = 600
tags = ["hub", "large_model", "moe", "npu", "tool_calling", "coding"]

# 本地 providers
[providers.lmstudio]
type = "openai_compat"
url = "http://localhost:1234"
default_model = "qwen3.5-35b-moe"

[providers.ollama]
type = "ollama"
url = "http://localhost:11434"
default_model = "qwen3:8b"

[providers.gemini]
type = "gemini"
api_key = "enc2:xxxxx"
default_model = "gemini-2.5-flash"

[providers.groq]
type = "groq"
api_key = "enc2:xxxxx"
default_model = "llama-3.3-70b-versatile"

# 遠端 Worker providers (透過 Tailscale IP)
[providers.m1]
type = "ollama"
url = "http://100.87.93.58:11434"
default_model = "qwen3:8b"

[providers.ayaneo]
type = "ollama"
url = "http://192.168.1.117:11434"
default_model = "qwen3:4b"

[providers.acer]
type = "ollama"
url = "http://192.168.1.108:11434"
default_model = "qwen3:8b"

# Mini PC 和 RTX 3060 的 IP 待部署後填入
# [providers.mini1]
# [providers.mini2]
# [providers.gpu1]
# [providers.gpu2]

# 路由 hints
[[routing]]
hint = "fast_chat"
provider = "lmstudio"
model = "qwen3.5-35b-moe"

[[routing]]
hint = "coding"
provider = "lmstudio"
model = "qwen3-coder-next"

[[routing]]
hint = "classify"
provider = "ayaneo"
model = "qwen3:4b"

[[routing]]
hint = "quality_writing"
provider = "gpu1"
model = "qwen3:13b"

[[routing]]
hint = "batch"
provider = "auto"

[[routing]]
hint = "overflow"
provider = "gemini"
model = "gemini-2.5-flash"

# SoT 配置
[skeleton]
skeleton_provider = "lmstudio"
expansion_providers = ["lmstudio", "m1", "ayaneo", "acer", "mini1", "mini2", "gpu1", "gpu2"]
max_sections = 8
section_max_tokens = 800
section_timeout_secs = 180

# Memory
[memory]
backend = "pgvector"
pg_url = "host=localhost user=clawtex dbname=clawtex"
embed_url = "http://localhost:11434"
embed_model = "nomic-embed-text"
dimensions = 768
```

---

## 附錄 A: 效能對比摘要

```
┌─────────────────────┬───────────┬───────────┬──────────┐
│ 工作流               │ 單機 (Z13) │ 8 機協同   │ 加速倍率  │
├─────────────────────┼───────────┼───────────┼──────────┤
│ Freelancer Hand     │ 24 min    │ 4.3 min   │ 5.6x     │
│ SEO Content Hand    │ 32 min    │ 3.7 min   │ 8.7x     │
│ 100 封冷郵件         │ 60 min    │ 8 min     │ 7.5x     │
│ 即時對話 (first tok) │ 3-5s      │ 0.2-0.5s  │ 10x      │
│ SaaS Pipeline       │ 45 min    │ 16.8 min  │ 2.7x     │
│ SoT 文章生成        │ 10 min    │ 1.9 min   │ 5.3x     │
└─────────────────────┴───────────┴───────────┴──────────┘

理論最大吞吐量:
  單機:  72 t/s (Z13 alone)
  8 機:  72 + 15 + 10 + 5 + 19 + 19 + 25 + 25 = 190 t/s (2.6x)
  + Gemini: 190 + ~100 t/s (雲端) = 290 t/s (4x)

  吞吐量加速 < 延遲加速，因為:
  1. 並行 phase 消除了串行等待
  2. SoT 8 路並行消除了段落串行
  3. 分類/評分用小模型更快
  4. Gemini Flash API 幾乎零延遲
```

## 附錄 B: 實施優先級

```
Phase 1 (立即可做 — 零硬體投入):
  [x] 現有 4 機 (Z13, M1, Ayaneo, Acer) 的 provider 配置
  [ ] Phase depends_on 欄位實作
  [ ] provider_hint 解析 + TaskScheduler 整合
  [ ] SoT expansion_providers 擴展到 4 機
  [ ] 即時對話的 fast path routing

Phase 2 (買 Mini PC 後):
  [ ] Mini PC 1+2 部署 Worker
  [ ] 6 機 SoT 並行
  [ ] 批次任務分配

Phase 3 (買 RTX 3060 後):
  [ ] GPU1+2 部署 Worker (CUDA Ollama)
  [ ] 8 機完整 SoT
  [ ] 高品質寫作路由到 GPU 節點
  [ ] 完整效能測試
```
