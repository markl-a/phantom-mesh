# Workflow Optimization & Cluster Integration Design

**Date**: 2026-03-05
**Status**: Approved
**Goal**: 全面提速提質，零成本，利用免費 API + 4 機集群 + AI 工具鏈

---

## 1. 問題診斷

### 1.1 目前瓶頸

phantom-mesh 單機執行一個 Hand（如 freelancer）：
- 5 phase × 最多 5 round = **25 次串行 LLM 呼叫**
- 全部走同一個 LM Studio 模型（Qwen3-Coder-Next Q4_K_M）
- market_intel 花 32 分鐘，平均每次 LLM 呼叫 ~75 秒

```
目前流程：
[Phase1] ──LLM──LLM──LLM── [Phase2] ──LLM──LLM── [Phase3] ──LLM── [Phase4] ──LLM──
^^^^^^^^ 全部串行，同一個模型，排隊等待 ^^^^^^^^
```

### 1.2 瓶頸分析

| 因素 | 影響 | 說明 |
|------|------|------|
| Phase 串行 | 極高 | 4-5 phase 必須依序完成，無並行 |
| 單模型排隊 | 極高 | 所有請求進同一個 LM Studio，排隊處理 |
| 輪次串行 | 高 | 每輪 1 次 LLM + N 個 tool，輪之間串行 |
| Context compaction | 中 | 超過 80% window 時額外一次 LLM 呼叫 |
| is_alive 探測 | 低-中 | auto routing 每輪都做 HTTP 探測 |
| Tool 執行 | 低 | 同輪內已並行（join_all） |

### 1.3 目前硬體

| 機器 | IP | RAM | GPU | Ollama 狀態 |
|------|-----|-----|-----|-------------|
| **Z13 (Hub)** | localhost / 192.168.1.100 | 128GB (GPU 96GB + CPU 32GB) | Radeon 8060S + NPU | ✅ LM Studio port 1234 |
| **M1 Mac** | 10.0.2.1 (Tailscale) | 16GB | Apple M1 | ✅ Ollama port 11434 |
| **Ayaneo 2** | 10.0.1.4 (LAN) | 16GB | Radeon 680M 3GB | ✅ Ollama port 11434 |
| **Acer Aspire** | 10.0.1.3 (LAN) | 24GB | MX350 2GB (CPU 為主) | ✅ Ollama port 11434 |

### 1.4 目前已有的免費資源

| 資源 | 類型 | 狀態 |
|------|------|------|
| Gemini API (Flash/Lite) | 雲端 LLM | ✅ 已有 key |
| Claude Code | AI 開發工具 | ✅ 已安裝 |
| Gemini CLI (MCP) | AI 開發工具 | ✅ 已安裝 |
| Codex CLI | AI 開發工具 | ✅ 可用 |
| OpenRouter | 免費模型 API | 需申請 key |
| Groq | 快速推理 API | 需申請 key |
| Cerebras | 快速推理 API | 需申請 key |

---

## 2. 設計目標

| 指標 | 目前 | 目標 |
|------|------|------|
| freelancer hand | 24 min | < 8 min |
| content hand | 10 min | < 3 min |
| market_intel hand | 32 min | < 10 min |
| Telegram 對話回覆 | 3-5 秒 | < 1 秒 |
| 內容品質 | 本地 30B 水準 | 雲端 480B 級 |
| 每日成本 | $0 | $0 |

---

## 3. 架構設計

### 3.1 全景圖

```
                          ┌─────────────────────────────────┐
                          │        Z13 Hub (phantom-mesh)   │
                          │                                 │
  Telegram ──────────────►│  TaskRouter                     │
                          │    │                            │
                          │    ├─ tier0: 本地快速            │
                          │    │   ├─ LMStudio Qwen3.5-35B  │◄── 72 t/s
                          │    │   └─ LMStudio Coder-Next   │◄── 66 t/s
                          │    │                            │
                          │    ├─ tier1: 免費雲端            │
                          │    │   ├─ Gemini 2.5 Flash      │◄── 秒回, 250/天
                          │    │   ├─ Gemini Flash-Lite     │◄── 秒回, 1000/天
                          │    │   ├─ OpenRouter free       │◄── Qwen3-Coder 480B
                          │    │   ├─ Groq                  │◄── Llama 70B, 1000/天
                          │    │   └─ Cerebras              │◄── 1M tok/天
                          │    │                            │
                          │    ├─ tier2: 集群節點            │
                          │    │   ├─ M1 Ollama             │◄── 15 t/s
                          │    │   ├─ Ayaneo Ollama         │◄── 8-10 t/s
                          │    │   └─ Acer Ollama           │◄── 3-5 t/s
                          │    │                            │
                          │    └─ tier3: AI 工具鏈           │
                          │        ├─ Claude Code subprocess│◄── coding 最強
                          │        ├─ Gemini CLI subprocess │◄── 1M context
                          │        └─ Codex CLI subprocess  │◄── OpenAI coding
                          │                                 │
                          └─────────────────────────────────┘
```

### 3.2 Provider 分層路由

```rust
/// 任務類型到 Provider 的映射
enum TaskTier {
    /// Tier 0: 本地快速 — 對話、格式化、tool calling
    /// 目標: < 1 秒回覆
    LocalFast,

    /// Tier 1: 免費雲端 — 推理、規劃、長文分析
    /// 目標: 2-5 秒回覆，品質最高
    CloudFree,

    /// Tier 2: 集群節點 — SoT 並行擴展、背景批次
    /// 目標: 並行加速，不阻塞主節點
    ClusterNode,

    /// Tier 3: AI 工具鏈 — 複雜 coding、大檔案分析
    /// 目標: 最高品質 coding output
    AiTool,
}
```

#### 路由規則

| 場景 | Tier | Provider | 模型 | 原因 |
|------|------|----------|------|------|
| Telegram 快速對話 | 0 | lmstudio | Qwen3.5-35B-A3B | 72t/s，秒回 |
| Tool calling (agent loop) | 0 | lmstudio | Coder-Next | tool call 格式最穩 |
| Hand Phase: 研究/搜尋 | 1 | gemini | Flash | 1M context 讀大量 web 結果 |
| Hand Phase: 分析/推理 | 1 | gemini | Flash | 複雜推理雲端品質好 |
| Hand Phase: 內容撰寫 | 1 | openrouter | Qwen3-Coder 480B free | 最強免費寫作模型 |
| Hand Phase: 程式碼生成 | 3 | ai_code | Claude Code | coding 品質最高 |
| SoT section 擴展 | 2 | m1/ayaneo/acer | qwen3:8b 等 | 並行不阻塞 Z13 |
| 背景批次任務 | 2 | acer_ollama | qwen3:8b | CPU 慢但免費無限 |
| Context compaction | 0 | lmstudio | Qwen3.5-35B | 快速摘要，不浪費雲端額度 |
| Embedding | 0 | ollama(Z13) | nomic-embed-text | 本地最快 |

### 3.3 免費額度管理

```rust
struct QuotaTracker {
    /// 每個雲端 provider 的每日使用量
    daily_usage: HashMap<String, QuotaUsage>,
}

struct QuotaUsage {
    requests_used: u32,
    requests_limit: u32,     // e.g. Gemini Flash = 250/天
    tokens_used: u64,
    tokens_limit: u64,       // e.g. Cerebras = 1M/天
    reset_at: DateTime<Utc>, // 每日重置時間
}
```

**降級策略**：

```
Gemini Flash (250/天) 用完
  → 降級到 Gemini Flash-Lite (1000/天)
    → 降級到 OpenRouter free
      → 降級到 Groq
        → 降級到本地 LM Studio
          → 降級到集群節點 (M1 > Ayaneo > Acer)
```

### 3.4 is_alive 快取

```rust
struct AliveCache {
    /// provider_name → (is_alive, last_checked)
    cache: HashMap<String, (bool, Instant)>,
    /// 快取有效期: 60 秒
    ttl: Duration,
}

impl AliveCache {
    fn is_alive(&mut self, provider: &str) -> Option<bool> {
        if let Some((alive, checked)) = self.cache.get(provider) {
            if checked.elapsed() < self.ttl {
                return Some(*alive); // 用快取，不探測
            }
        }
        None // 過期，需重新探測
    }
}
```

目前每輪 LLM 呼叫前都做 HTTP 探測（10 輪 = 10 次探測）。改為 60 秒快取後，大部分輪次直接跳過探測。

---

## 4. 集群整合設計

### 4.1 零部署方案

4 台機器都已跑 Ollama + SSH 確認連通。**不需要部署 phantom-mesh worker**，只需要：

1. `agents.toml` 加 3 個 `openai_compat` provider（指向遠端 Ollama）
2. `ProviderRouter` 支援遠端健康檢查
3. `skeleton.rs` 的 expansion_providers 包含遠端節點

```toml
# agents.toml 新增

[providers.m1_ollama]
type = "openai_compat"
url = "http://10.0.2.1:11434"
default_model = "qwen2.5:14b"

[providers.ayaneo_ollama]
type = "openai_compat"
url = "http://10.0.1.4:11434"
default_model = "qwen2.5-coder:7b"

[providers.acer_ollama]
type = "openai_compat"
url = "http://10.0.1.3:11434"
default_model = "qwen3:8b"
```

### 4.2 各節點模型更新建議

| 節點 | 目前最佳模型 | 建議更新 | 原因 |
|------|-------------|---------|------|
| Z13 LMStudio | Coder-Next Q4_K_M (46GB) | + Qwen3.5-35B-A3B Q8_0 (20GB) | 雙模型並行 |
| M1 Mac | qwen2.5:14b (14B) | 保持 | 16GB 上限，14B 已最佳 |
| Ayaneo | qwen2.5-coder:7b | 保持 | 16GB + 680M，7B 合適 |
| Acer | qwen3:8b | 保持 | CPU 推理，8B 是上限 |

### 4.3 SoT 集群並行

```
Outline 生成 (Z13 LMStudio, 快)
         │
         ▼
    ┌────┴────┬──────────┬──────────┐
    ▼         ▼          ▼          ▼
 Section 1  Section 2  Section 3  Section 4
 Z13 LMS    M1 14B     Ayaneo 7B  Acer 8B
 (66 t/s)   (15 t/s)   (10 t/s)   (5 t/s)
    │         │          │          │
    ▼         ▼          ▼          ▼
    └────┬────┴──────────┴──────────┘
         │
    Merge (按 index 排序合併)
```

**預估效果**：4 section 並行，瓶頸是最慢的 Acer (~90 秒)，但 Z13 不被阻塞。
vs 目前全部 Z13 串行 (~4 分鐘)。

### 4.4 集群健康監控

```rust
struct ClusterStatus {
    nodes: Vec<NodeStatus>,
}

struct NodeStatus {
    name: String,           // "m1", "ayaneo", "acer"
    provider_name: String,  // agents.toml 中的 key
    url: String,
    is_alive: bool,
    last_ping_ms: u64,
    loaded_models: Vec<String>,
    pending_requests: u32,
}
```

`/cluster` Telegram 指令顯示：

```
🖥 Cluster Status
┌─────────┬────────┬────────┬──────────┐
│ Node    │ Status │ Ping   │ Models   │
├─────────┼────────┼────────┼──────────┤
│ Z13     │ 🟢 ON  │ <1ms   │ 2 loaded │
│ M1 Mac  │ 🟢 ON  │ 52ms   │ qwen14b  │
│ Ayaneo  │ 🟢 ON  │ <1ms   │ coder7b  │
│ Acer    │ 🟡 SLOW│ <1ms   │ qwen8b   │
└─────────┴────────┴────────┴──────────┘
```

---

## 5. AI 工具鏈整合

### 5.1 工具清單

| 工具 | 呼叫方式 | 合規使用場景 | 限制 |
|------|---------|-------------|------|
| **Claude Code** | subprocess `claude -p "prompt"` | 自有專案 coding | 合規：開發自有專案 |
| **Gemini CLI** | MCP server (已有) | 大檔案分析、1M context | 合規：Google 免費工具 |
| **Codex CLI** | subprocess `codex "prompt"` | coding 輔助 | 合規：OpenAI 免費工具 |

### 5.2 ai_code tool 升級

目前 `ai_code` tool 支援 `claude` 後端。擴展為：

```rust
enum AiCodeBackend {
    Claude,     // claude -p "prompt" --output-format stream-json
    GeminiCli,  // gemini "prompt" (via MCP or subprocess)
    Codex,      // codex "prompt"
}
```

**路由邏輯**：

```
coding 任務
  ├─ 檔案 < 10KB → Claude Code（品質最高）
  ├─ 檔案 > 10KB → Gemini CLI（1M context）
  └─ 快速修復     → Codex CLI（速度快）
```

### 5.3 Hand Phase 直接呼叫 AI 工具

新增 phase 執行模式：

```toml
# hand.toml 範例
[[phases]]
name = "generate_code"
system_prompt = "Generate the API endpoints..."
execution_mode = "ai_tool"      # 新欄位：跳過 agent loop
ai_tool = "claude_code"         # 直接用 Claude Code
ai_tool_args = "--output-format json"
```

當 `execution_mode = "ai_tool"` 時，HandRunner 跳過 agent runtime loop，直接呼叫對應的 subprocess，拿到 stdout 作為 phase output。省去 tool calling 的多輪往返。

---

## 6. Hand Phase 並行化

### 6.1 Phase 依賴模型

目前所有 phase 嚴格串行。改為支援依賴聲明：

```toml
# hand.toml 範例 — market_intel
[[phases]]
name = "market_overview"
phase_index = 0
depends_on = []                 # 無依賴，立即開始

[[phases]]
name = "competitor_analysis"
phase_index = 1
depends_on = []                 # 無依賴，可與 phase 0 並行

[[phases]]
name = "pricing_analysis"
phase_index = 2
depends_on = [0, 1]             # 等 phase 0+1 完成

[[phases]]
name = "opportunity_report"
phase_index = 3
depends_on = [2]                # 等 phase 2 完成
provider_hint = "gemini"        # 指定用 Gemini 做最終報告
```

### 6.2 執行引擎改動

```rust
// hands/mod.rs — 新增並行執行

async fn run_phases_parallel(
    &self,
    hand: &Hand,
    input: &str,
    // ...
) -> Result<String> {
    let mut completed: HashMap<usize, String> = HashMap::new();
    let mut pending: JoinSet<(usize, Result<String>)> = JoinSet::new();

    loop {
        // 找出所有依賴已滿足且尚未啟動的 phase
        for (i, phase) in hand.phases.iter().enumerate() {
            if completed.contains_key(&i) { continue; }
            if !is_running(&pending, i) {
                let deps_met = phase.depends_on
                    .iter()
                    .all(|d| completed.contains_key(d));
                if deps_met {
                    let context = build_context(&completed, &phase.depends_on, input);
                    pending.spawn(async move {
                        (i, run_single_phase(phase, &context).await)
                    });
                }
            }
        }

        // 等待任一 phase 完成
        if let Some(Ok((idx, result))) = pending.join_next().await {
            completed.insert(idx, result?);
        }

        if completed.len() == hand.phases.len() {
            break;
        }
    }

    // 回傳最後一個 phase 的輸出
    Ok(completed[&(hand.phases.len() - 1)].clone())
}
```

### 6.3 向下相容

- 如果所有 phase 都沒有 `depends_on` 欄位，保持原本串行行為（phase N 依賴 phase N-1）
- 只有明確聲明 `depends_on = []` 的 phase 才能並行

---

## 7. Provider 配置（完整 agents.toml 變更）

### 7.1 新增 Provider

```toml
# === 集群節點 ===

[providers.m1_ollama]
type = "openai_compat"
url = "http://10.0.2.1:11434"
default_model = "qwen2.5:14b"

[providers.ayaneo_ollama]
type = "openai_compat"
url = "http://10.0.1.4:11434"
default_model = "qwen2.5-coder:7b"

[providers.acer_ollama]
type = "openai_compat"
url = "http://10.0.1.3:11434"
default_model = "qwen3:8b"

# === 免費雲端 API ===

[providers.openrouter]
type = "openai_compat"
url = "https://openrouter.ai/api/v1"
api_key = ""  # 需申請填入
default_model = "qwen/qwen3-coder:free"

[providers.groq]
type = "groq"
api_key = ""  # 需申請填入
default_model = "llama-3.3-70b-versatile"

[providers.cerebras]
type = "openai_compat"
url = "https://api.cerebras.ai/v1"
api_key = ""  # 需申請填入
default_model = "llama-3.3-70b"
```

### 7.2 路由表

```toml
[routing]
# 任務類型 → (provider, model_override)
chat = { provider = "lmstudio", model = "qwen3.5-35b-a3b" }
tool_calling = { provider = "lmstudio", model = "qwen3-coder-next" }
reasoning = { provider = "gemini", model = "gemini-2.5-flash" }
content_writing = { provider = "openrouter", model = "qwen/qwen3-coder:free" }
code_generation = { provider = "lmstudio", model = "qwen3-coder-next" }
summarization = { provider = "lmstudio", model = "qwen3.5-35b-a3b" }
embedding = { provider = "ollama", model = "nomic-embed-text" }
background_batch = { provider = "acer_ollama", model = "qwen3:8b" }

# SoT 擴展節點（round-robin）
[routing.sot_expansion]
providers = ["lmstudio", "m1_ollama", "ayaneo_ollama", "acer_ollama"]

# 降級鏈
[routing.fallback_chain]
order = ["gemini", "openrouter", "groq", "cerebras", "lmstudio", "m1_ollama", "ayaneo_ollama", "acer_ollama"]
```

### 7.3 額度配置

```toml
[quotas]
[quotas.gemini]
daily_requests = 250
rpm = 10
model = "gemini-2.5-flash"

[quotas.gemini_lite]
daily_requests = 1000
rpm = 15
model = "gemini-2.5-flash-lite"

[quotas.groq]
daily_requests = 1000
rpm = 30

[quotas.cerebras]
daily_tokens = 1000000
rpm = 30

[quotas.openrouter]
daily_requests = 0  # 0 = 無硬限制（rate limit 由 server 端控制）
```

---

## 8. 實施計畫

### 階段 1：加 Provider + 智慧路由（1 天）

**目標**：不改架構，只加配置和路由邏輯，立刻提速。

| 任務 | 檔案 | 改動 |
|------|------|------|
| 1.1 agents.toml 加 6 個 provider | `~/.phantom-mesh/agents.toml` | 新增 TOML section |
| 1.2 申請免費 API key | OpenRouter, Groq, Cerebras | 網頁操作 |
| 1.3 ProviderRouter 加路由表 | `src/providers/router.rs` | 讀取 `[routing]` 表 |
| 1.4 is_alive 快取 | `src/providers/router.rs` | AliveCache 結構 |
| 1.5 QuotaTracker | `src/quota.rs` (新檔案) | 額度追蹤 + 降級 |
| 1.6 Hand phase provider_hint | `src/hands/mod.rs` | Phase TOML 讀 provider_hint |
| 1.7 /cluster 指令 | `src/telegram.rs` | 顯示所有節點狀態 |
| 1.8 測試 | `tests/` | 路由 + 額度 + 集群連通測試 |

**驗收**：
- Telegram 對話 < 1 秒回覆（走本地 Qwen3.5-35B）
- Hand phase 可指定 provider_hint = "gemini"
- /cluster 顯示 4 節點狀態
- 額度用完自動降級

### 階段 2：AI 工具鏈（2-3 天）

| 任務 | 檔案 | 改動 |
|------|------|------|
| 2.1 ai_code 加 gemini_cli 後端 | `src/tools/ai_code.rs` | GeminiCli backend |
| 2.2 ai_code 加 codex 後端 | `src/tools/ai_code.rs` | Codex backend |
| 2.3 Phase execution_mode | `src/hands/mod.rs` | ai_tool 直呼模式 |
| 2.4 更新 10 個 hand.toml | `~/.phantom-mesh/hands/*/hand.toml` | 每 phase 加 provider_hint + execution_mode |
| 2.5 測試 | `tests/` | AI 工具呼叫 + phase routing 測試 |

**驗收**：
- code_gen hand 的 Phase 2 用 Claude Code 生成程式碼
- seo_content 的 Phase 3 用 Gemini CLI 分析長文
- Hand 執行時間減半

### 階段 3：並行化 + 集群 SoT（3-5 天）

| 任務 | 檔案 | 改動 |
|------|------|------|
| 3.1 Phase depends_on 欄位 | `src/hands/mod.rs` | Phase struct + 解析 |
| 3.2 並行執行引擎 | `src/hands/mod.rs` | JoinSet 並行 + 依賴解析 |
| 3.3 SoT 集群擴展 | `src/skeleton.rs` | expansion_providers 包含遠端 |
| 3.4 Provider 負載均衡 | `src/providers/router.rs` | pending_requests 計數 |
| 3.5 更新 hand.toml 依賴 | `~/.phantom-mesh/hands/*/hand.toml` | 加 depends_on 欄位 |
| 3.6 /sot 指令增強 | `src/telegram.rs` | 顯示各節點進度 |
| 3.7 整合測試 | `tests/integration.rs` | 並行 + 集群 E2E |

**驗收**：
- market_intel Phase 0+1 並行（從 32 min → ~12 min）
- SoT 4 機並行展開（從 4 min → ~1.5 min）
- 負載均衡自動分配到最空閒節點

---

## 9. Hand Phase Provider 分配

### 9.1 freelancer hand（目前 24 min → 目標 7 min）

| Phase | 名稱 | Provider | 原因 |
|-------|------|----------|------|
| 0 | 搜工作 | gemini | web_search 結果大，Gemini 1M context |
| 1 | 評分 | lmstudio (35B) | 快速判斷，不需雲端 |
| 2 | 寫提案 | openrouter | 480B 品質寫作 |
| 3 | 準備資料 | lmstudio (Coder) | tool calling 穩定 |
| 4 | 人工審核 | lmstudio (35B) | 格式化快速 |

### 9.2 seo_content hand（目前 15 min → 目標 5 min）

| Phase | 名稱 | Provider | 原因 |
|-------|------|----------|------|
| 0 | 關鍵詞研究 | gemini | 搜尋分析 |
| 1 | 競爭分析 | gemini | 長文分析 |
| 2 | 寫文章 | openrouter | 480B 寫作品質 |
| 3 | SEO 優化 | lmstudio (35B) | 格式化快速 |
| 4 | 發布推廣 | lmstudio (Coder) | tool calling (twitter, blog) |
| depends_on | 0∥1 並行 | — | Phase 0 和 1 無依賴可並行 |

### 9.3 market_intel hand（目前 32 min → 目標 10 min）

| Phase | 名稱 | Provider | depends_on |
|-------|------|----------|------------|
| 0 | 市場概況 | gemini | [] (獨立) |
| 1 | 競爭對手 | gemini | [] (獨立，與 0 並行) |
| 2 | 定價分析 | lmstudio (35B) | [0, 1] |
| 3 | 機會報告 | openrouter | [2] |

### 9.4 content hand（目前 10 min → 目標 3 min）

| Phase | 名稱 | Provider | 原因 |
|-------|------|----------|------|
| 0 | 主題研究 | gemini | 搜尋 |
| 1 | 撰寫內容 | openrouter | 480B 品質 |
| 2 | 編輯潤色 | lmstudio (35B) | 快速 |
| 3 | 發布推廣 | lmstudio (Coder) | tool calling |

---

## 10. 風險與緩解

| 風險 | 機率 | 緩解 |
|------|------|------|
| 免費額度不夠用 | 中 | 降級鏈自動回落本地；多個免費 provider 輪替 |
| M1 Mac 離線 | 中 | is_alive 檢測 → 自動跳過，分配給其他節點 |
| OpenRouter 免費模型品質波動 | 低-中 | 配置多個免費模型候選，自動切換 |
| 雲端 API rate limit | 中 | QuotaTracker 預防；碰到 429 自動降級 |
| Hand 並行 phase 結果不一致 | 低 | Phase 3 (合併) 明確聲明依賴 |
| AI 工具 subprocess 掛掉 | 低 | timeout + fallback 到普通 agent loop |
| 集群節點 Ollama 版本不一致 | 低 | 只用通用 /v1/chat/completions 端點 |

---

## 11. 測試策略

### 單元測試
- `QuotaTracker`: 額度計算、降級觸發、每日重置
- `AliveCache`: TTL 過期、快取命中
- `TaskRouter`: 路由規則匹配、fallback chain
- `Phase depends_on`: 依賴解析、拓撲排序

### 整合測試
- Provider 連通: 6 個新 provider 都能 chat
- 集群 SoT: 4 機並行展開 + merge
- Hand phase routing: provider_hint 正確分派
- 額度降級: mock 額度用完 → 自動降級

### E2E 測試
- freelancer hand 全流程（含 provider routing）
- content hand 全流程（含 AI tool 直呼）
- /cluster 指令顯示正確
- 額度告警 Telegram 通知
