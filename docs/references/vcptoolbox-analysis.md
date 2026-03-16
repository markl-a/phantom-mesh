# VCPToolBox 分析 — AI 中間件 + 進階 RAG 記憶系統

> Repo: https://github.com/lioensky/VCPToolBox
> Language: Node.js + Rust (N-API)
> Stars: 1,544
> 分析日期: 2026-03-14

---

## 概述

VCPToolBox 是一個 AI 中間件代理，坐在前端（SillyTavern / Open WebUI）和後端 LLM API 之間。攔截 `/v1/chat/completions` 請求，注入記憶、工具描述、Agent 人格，然後轉發給 LLM。核心價值：讓任何無狀態 LLM 變成有記憶 + 工具的智能系統，前後端都不用改。

**不是** agent framework，是 **proxy enhancement layer**。

---

## 架構

```
Frontend (SillyTavern/OpenWebUI)
  → POST /v1/chat/completions
  → server.js: auth + variable substitution
  → messageProcessor.js: inject agent prompts, tool lists, memory
  → chatCompletionHandler.js: forward to backend LLM
  → Model response: <<<[TOOL_REQUEST]>>> blocks
  → toolCallParser.js: parse 「始」value「末」 delimiters
  → toolExecutor.js: execute plugin (±approval)
  → VCP Loop: inject result back → re-query LLM (max 5 rounds)
  → Return final response
```

**Tech Stack:**
- Express HTTP server (port 6005) + SSE streaming + WebSocket
- 87+ plugins（browser/search/image gen/email/shell/diary）
- SQLite (better-sqlite3) for vector metadata + KV store
- Rust N-API module (`rust-vexus-lite`) for vector math: USearch, SVD, orthogonal projection

---

## 值得參考的技術（3 個高價值 + 3 個中價值）

### HIGH 1: TagMemo V7 — LIF 神經激發記憶傳播

**位置:** `KnowledgeBaseManager.js` → `_applyTagBoostV6`（~400 行）

普通 RAG 做 cosine similarity top-K。VCPToolBox 在此之上加了 4 層：

1. **EPA Module（Embedding Projection Analysis）**
   - 對 tag 向量做 K-Means 聚類 → Gram matrix → Power iteration PCA
   - 將 query 投影到正交語義軸，偵測查詢屬於哪個「概念世界」
   - 計算 logic depth、entropy、cross-domain resonance

2. **Residual Pyramid**
   - Gram-Schmidt 正交分解 query 向量
   - 測量多少語義能量被現有 tag 「解釋」，多少是新的/未探索的
   - 高殘差 = 新概念，觸發探索性檢索

3. **LIF Spike Propagation（核心創新）**
   - 建立 tag 共現圖（co-occurrence graph）
   - 初始 tag 命中 → 膜電位累積 → 超過閾值「激發」
   - 激發能量沿邊傳播，突觸衰減
   - **效果**：找到向量搜尋找不到、但概念拓撲相連的記憶
   - 例：查「Rust 效能」→ 激發傳播到「記憶體安全」→ 再到「unsafe 最佳實踐」

4. **Worldview Gating**
   - 語言信心補償：中文查詢時降低英文技術詞的權重
   - 防止 code-like tag name 在非技術語境中造成噪音

**Clawtex 應用場景:**
- `src/memory/` 的 `recall()` 目前只做 keyword + cosine + RRF
- 可在 RRF merge 後加一層 spike propagation
- 需要：tag 共現表（SQLite adjacency）+ 簡化版 LIF 算法
- 預估工作量：200-300 行 Rust（比 JS 版更高效）

---

### HIGH 2: Dynamic Fold Protocol — 動態工具描述注入

**位置:** `messageProcessor.js` → tool description injection

**問題:** clawtex-core 每次 agent 呼叫都把 24 個 tool 的完整描述注入 system prompt，浪費 token。llama3.2:1b 只有 2K context，24 tool 描述可能佔 30%+ 。

**VCPToolBox 做法:**
1. 將每個 tool 描述做 embedding
2. 對當前對話最後 N 則訊息做 embedding
3. 計算 cosine similarity
4. 只注入 similarity > threshold 的 tool 描述
5. 其餘省略（model 不知道有這些 tool）

**Clawtex 應用場景:**
- `src/agent_runtime.rs` → `build_tool_specs()` 目前回傳全部 tool
- 可加入 `filter_relevant_tools(conversation, all_tools)` 步驟
- 用現有的 Ollama `/api/embed` 產生 tool description embedding（一次性，cache）
- 每次 agent run 前比對對話 embedding vs tool embeddings
- 預估工作量：100-150 行 Rust

---

### HIGH 3: Model-Specific Sar Prompts — 模型專屬行為注入

**位置:** `messageProcessor.js` → `SarModel1` / `SarPrompt1` variables

**做法:** 在 config 中定義 model name → extra instruction 的映射：
```
SarModel1=gemini-2.5-flash
SarPrompt1=Always show your reasoning step by step.
SarModel2=qwen3-coder
SarPrompt2=請一律使用繁體中文回答，不要使用簡體中文。
```

Router 偵測當前用哪個 model，自動 append 對應的 Sar prompt 到 system message。

**Clawtex 應用場景:**
- 解決已知問題：Qwen 回簡體中文
- `src/providers/router.rs` → `chat_with_tools()` 在組裝 system prompt 時檢查 model name
- 可在 `agents.toml` 加 `[model_prompts]` section
- 預估工作量：30-50 行 Rust + TOML config

---

### MEDIUM 1: Agent Dream — 離線記憶整理

**位置:** `Plugins/AgentDream/`

Agent 空閒時執行「做夢」：
1. 讀取近期日記/記憶
2. LLM 分析哪些可合併、哪些冗餘
3. 產生 JSON 操作提案（merge/delete/insight）
4. 需管理員批准才執行

**Clawtex 應用:** `self_evolve` hand 目前專注 code improvement，可加一個 `memory_consolidation` phase 或獨立 hand。

---

### MEDIUM 2: 熱重載審批配置

**位置:** `ToolApprovalManager.js`

用 `chokidar` 監聽 `toolApprovalConfig.json`，變更時自動 reload。不需重啟 daemon 即可更新哪些 tool 需要人工批准。

**Clawtex 應用:** `src/approval.rs` 的規則目前是 static。可用 `notify` crate 監聽 TOML 變更。

---

### MEDIUM 3: 閒置索引卸載

**位置:** `KnowledgeBaseManager.js` → index lifecycle

向量索引 2 小時未被查詢 → 自動從記憶體卸載。下次查詢時 lazy-load。

**Clawtex 應用:** 適合記憶體有限的 cluster worker（Acer 等），pgvector backend 不受影響但 SQLite 記憶體向量搜尋可受益。

---

## 不需要參考的部分

| 面向 | VCPToolBox | Clawtex 現況 | 結論 |
|------|-----------|-------------|------|
| Provider 架構 | 單一上游 proxy | 12+ 原生 provider | Clawtex 更好 |
| Tool 協議 | 自造文字分隔符（脆弱） | OpenAI function calling | Clawtex 更標準 |
| 安全 | Basic auth + bearer | ChaCha20 + guardrail + RBAC | Clawtex 更強 |
| 測試 | 0 個 | 816+ | Clawtex 完勝 |
| 集群 | WebSocket hub（簡易） | Tailscale + HTTP + 負載均衡 | Clawtex 更成熟 |
| 工作流 | 無 | 17 Hands + cron | Clawtex 獨有 |
| 成本追蹤 | 無 | costs.db per-agent/provider | Clawtex 獨有 |
| Streaming | Proxy SSE | 原生 SSE + ThinkFilter | Clawtex 更好 |
| Multi-agent | 單 plugin (AgentAssistant) | delegate + delegate_to_provider | Clawtex 更完整 |

---

## 實作優先順序

| 順序 | 項目 | 難度 | 影響 | 目標模組 |
|------|------|------|------|---------|
| 1 | Dynamic Fold（動態工具注入） | 低 | 省 30%+ token | `src/agent_runtime.rs` |
| 2 | Model-Specific Prompts | 低 | 解決 Qwen 簡體問題 | `src/providers/router.rs` |
| 3 | LIF 記憶傳播 | 中 | 提升記憶關聯性 | `src/memory/` |
| 4 | Agent Dream 記憶整理 | 低 | 記憶品質 | 新 hand TOML |
| 5 | 熱重載審批 | 低 | 運維便利 | `src/approval.rs` |
| 6 | 閒置索引卸載 | 低 | Worker 記憶體 | `src/memory/sqlite.rs` |

---

## 關鍵程式碼參考

| 功能 | VCPToolBox 位置 | 行數 | 說明 |
|------|----------------|------|------|
| LIF Spike | `KnowledgeBaseManager.js:607-707` | ~100 | 核心傳播算法 |
| EPA Module | `EPAModule.js` | ~200 | 語義軸投影 |
| Residual Pyramid | `ResidualPyramid.js` | ~150 | 殘差能量分析 |
| Dynamic Fold | `messageProcessor.js` (tool injection) | ~50 | 相似度過濾 |
| Sar Prompts | `messageProcessor.js` (SarModel) | ~30 | 模型偵測+注入 |
| VCP Loop | `chatCompletionHandler.js` | ~100 | 多輪工具呼叫 |
| Agent Dream | `Plugins/AgentDream/` | ~300 | 離線記憶整理 |
| Rust Vector | `rust-vexus-lite/src/lib.rs` | ~500 | USearch + SVD + projection |
