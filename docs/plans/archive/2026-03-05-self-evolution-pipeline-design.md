# Phantom Mesh 自我進化管線設計文件

## 日期：2026-03-05

## 目標

讓 phantom-mesh AI agent 集群具備**受控的自我改進能力**：自動監控效能 → 診斷瓶頸 → 建議改進 → 人工審批 → 自動套用 → 自動驗證。所有自我修改必須經過 Telegram 人工審批，且可隨時 rollback。

---

## 整體架構

```
                         ┌─────────────────────────────┐
                         │     Telegram 審批閘道       │
                         │  /approve /deny /rollback   │
                         └──────────┬──────────────────┘
                                    │ 人工決策
    ┌───────────────────────────────┼───────────────────────────────┐
    │                               │                               │
    ▼                               ▼                               ▼
┌─────────┐  metrics   ┌──────────────────┐  diagnosis  ┌──────────────────┐
│ 效能監控 │──────────→│   診斷引擎        │───────────→│   改進生成器      │
│ Monitor  │           │   Diagnostics     │            │   Improver        │
└─────────┘           └──────────────────┘            └──────────────────┘
    ▲                                                         │
    │                                                         │ 改進方案
    │    ┌──────────────────────────────────────────────────┐ │
    │    │              安全護欄 (SafeGuard)                 │◄┘
    │    │  ・備份 ・diff 審核 ・禁區檢查 ・rollback        │
    │    └───────────────────┬──────────────────────────────┘
    │                        │ 已批准 + 已備份
    │                        ▼
    │              ┌──────────────────┐
    │              │   套用引擎       │
    │              │   Applicator     │
    └──────────────│  ・寫入 TOML     │
     效能回測       │  ・更新路由表    │
                   │  ・編譯新 tool   │
                   └──────────────────┘
```

---

## 1. 效能自動監控 (`src/evolution/monitor.rs`)

### 1.1 數據模型

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 單一 Phase 的執行指標
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseMetrics {
    pub hand_name: String,
    pub phase_name: String,
    pub phase_index: usize,
    /// 執行耗時（秒）
    pub duration_secs: f64,
    /// Token 使用量
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub total_tokens: u32,
    /// 估算成本 (USD)
    pub estimated_cost_usd: f64,
    /// Tool 調用次數
    pub tool_calls: usize,
    /// 使用的 provider 和 model
    pub provider: String,
    pub model: String,
    /// 輸出長度（字符）
    pub output_length: usize,
    /// 品質評分（0.0 - 10.0），None = 尚未評分
    pub quality_score: Option<f64>,
    /// 評分來源
    pub score_source: Option<ScoreSource>,
    /// 是否被跳過（condition gate）
    pub skipped: bool,
    /// 時間戳
    pub timestamp: DateTime<Utc>,
}

/// 完整 Hand 執行的指標
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandMetrics {
    pub run_id: String,
    pub hand_name: String,
    pub user_input_preview: String,
    /// 各 phase 指標
    pub phases: Vec<PhaseMetrics>,
    /// 整體耗時
    pub total_duration_secs: f64,
    /// 整體 token
    pub total_tokens: u32,
    /// 整體成本
    pub total_cost_usd: f64,
    /// 整體品質分（各 phase 加權平均）
    pub overall_quality: Option<f64>,
    /// 執行結果：成功/失敗/部分完成
    pub outcome: HandOutcome,
    /// 鏈到的下一個 Hand（如果有）
    pub chain_to: Option<String>,
    /// 時間戳
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HandOutcome {
    Success,
    PartialSuccess { completed_phases: usize, total_phases: usize },
    Failed { error: String },
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScoreSource {
    /// LLM 自動評分（Gemini Flash 便宜且快）
    LlmAuto { model: String },
    /// 人工 Telegram 回饋
    HumanFeedback { user_id: String },
    /// A/B 測試比較分數
    AbTest { comparison_run_id: String },
}
```

### 1.2 品質評分機制

```rust
/// 品質評估器
pub struct QualityEvaluator {
    /// 用來做自動評分的 provider（推薦 Gemini Flash — 免費）
    eval_provider: String,
    eval_model: String,
}

impl QualityEvaluator {
    /// LLM 自動評分：把 phase 的 system_prompt + output 送給評估模型
    pub async fn auto_evaluate(
        &self,
        phase: &Phase,
        output: &str,
        router: &LlmRouter,
    ) -> Result<f64> {
        let eval_prompt = format!(
            r#"你是一個嚴格的品質評審員。請評估以下 AI 輸出的品質。

## 任務描述（system_prompt）
{}

## AI 輸出
{}

## 評分標準（每項 0-2 分，共 10 分）
1. 完整性：是否完成了 system_prompt 要求的所有步驟？
2. 準確性：內容是否準確、無虛構數據？
3. 結構性：是否有良好的格式和組織？
4. 可行性：輸出是否可直接使用/執行？
5. 深度：分析是否深入而非流於表面？

只回覆一個 JSON 物件：
{{"scores": {{"completeness": N, "accuracy": N, "structure": N, "actionability": N, "depth": N}}, "total": N, "brief_reason": "一句話理由"}}"#,
            phase.system_prompt,
            &output[..output.len().min(3000)] // 截斷避免超過 context
        );
        // ... 解析 JSON 回覆取得 total 分數
        todo!()
    }

    /// 接收 Telegram 人工回饋（👍=8分, 👎=3分, 自定義=用戶輸入分數）
    pub fn record_human_feedback(
        &self,
        run_id: &str,
        phase_name: &str,
        feedback: HumanFeedback,
    ) -> Result<f64> {
        match feedback {
            HumanFeedback::ThumbsUp => Ok(8.0),
            HumanFeedback::ThumbsDown => Ok(3.0),
            HumanFeedback::Score(s) => Ok(s.clamp(0.0, 10.0)),
            HumanFeedback::Comment(text) => {
                // 存到 memory 供後續分析
                Ok(5.0) // 有評論但無明確分數時預設中間值
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum HumanFeedback {
    ThumbsUp,
    ThumbsDown,
    Score(f64),
    Comment(String),
}
```

### 1.3 SQLite 持久化

```rust
/// 效能指標資料庫（與 CostTracker 獨立，有不同的查詢模式）
pub struct MetricsStore {
    db_path: String,
}

impl MetricsStore {
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = rusqlite::Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS hand_runs (
                run_id TEXT PRIMARY KEY,
                hand_name TEXT NOT NULL,
                input_preview TEXT,
                total_duration_secs REAL,
                total_tokens INTEGER,
                total_cost_usd REAL,
                overall_quality REAL,
                outcome TEXT NOT NULL,
                chain_to TEXT,
                timestamp TEXT NOT NULL,
                date_key TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS phase_metrics (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL REFERENCES hand_runs(run_id),
                hand_name TEXT NOT NULL,
                phase_name TEXT NOT NULL,
                phase_index INTEGER NOT NULL,
                duration_secs REAL,
                tokens_in INTEGER,
                tokens_out INTEGER,
                total_tokens INTEGER,
                estimated_cost_usd REAL,
                tool_calls INTEGER,
                provider TEXT,
                model TEXT,
                output_length INTEGER,
                quality_score REAL,
                score_source TEXT,
                skipped INTEGER DEFAULT 0,
                timestamp TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_phase_hand ON phase_metrics(hand_name);
            CREATE INDEX IF NOT EXISTS idx_phase_quality ON phase_metrics(quality_score);
            CREATE INDEX IF NOT EXISTS idx_run_date ON hand_runs(date_key);
            CREATE INDEX IF NOT EXISTS idx_run_hand ON hand_runs(hand_name);"
        )?;
        Ok(Self { db_path: db_path.to_string() })
    }

    /// 記錄一次完整 Hand 執行的指標
    pub fn record_hand_run(&self, metrics: &HandMetrics) -> Result<()> { todo!() }

    /// 查詢：哪些 phase 最慢？（按平均耗時排序）
    pub fn slowest_phases(&self, days: u32, limit: usize) -> Result<Vec<PhasePerformance>> { todo!() }

    /// 查詢：哪些 phase 品質最差？（按平均品質分排序）
    pub fn lowest_quality_phases(&self, days: u32, limit: usize) -> Result<Vec<PhasePerformance>> { todo!() }

    /// 查詢：特定 Hand 的效能趨勢（按日）
    pub fn hand_trend(&self, hand_name: &str, days: u32) -> Result<Vec<DailyHandStats>> { todo!() }

    /// 查詢：各 provider 在同一個 phase 上的 A/B 比較
    pub fn provider_comparison(
        &self,
        hand_name: &str,
        phase_name: &str,
        days: u32,
    ) -> Result<Vec<ProviderPhaseStats>> { todo!() }
}

/// Phase 效能摘要（用於報告）
#[derive(Debug, Clone, Serialize)]
pub struct PhasePerformance {
    pub hand_name: String,
    pub phase_name: String,
    pub avg_duration_secs: f64,
    pub avg_quality: f64,
    pub avg_tokens: u32,
    pub avg_cost_usd: f64,
    pub run_count: u32,
}

/// 每日 Hand 統計
#[derive(Debug, Clone, Serialize)]
pub struct DailyHandStats {
    pub date: String,
    pub run_count: u32,
    pub avg_quality: f64,
    pub avg_duration_secs: f64,
    pub total_cost_usd: f64,
    pub success_rate: f64,
}

/// Provider 在特定 phase 的表現統計
#[derive(Debug, Clone, Serialize)]
pub struct ProviderPhaseStats {
    pub provider: String,
    pub model: String,
    pub avg_quality: f64,
    pub avg_duration_secs: f64,
    pub avg_tokens: u32,
    pub avg_cost_usd: f64,
    pub run_count: u32,
}
```

### 1.4 與 HandRunner 整合

修改 `hands/mod.rs` 的 `HandRunner::run()` 和 `run_single_phase()`：

```rust
// 在 HandRunner::run_single_phase() 結束時，自動記錄 PhaseMetrics
// 在 HandRunner::run() 結束時，自動記錄 HandMetrics
// HandRunner 需要新增 metrics_store: Option<Arc<MetricsStore>> 參數
// 或者透過 AgentRuntime 傳遞（因為 AgentRuntime 已有 cost_tracker pattern）

impl HandRunner {
    pub async fn run_with_metrics(
        hand: &Hand,
        user_input: &str,
        runtime: &AgentRuntime,
        router: &LlmRouter,
        tool_registry: &ToolRegistry,
        metrics_store: Option<&MetricsStore>,
        quality_evaluator: Option<&QualityEvaluator>,
    ) -> Result<HandResult> {
        let start = std::time::Instant::now();
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut outputs = Vec::new();
        let mut phase_metrics = Vec::new();
        let mut context = Self::prepare_context(hand, user_input);

        for i in 0..hand.phases.len() {
            let phase_start = std::time::Instant::now();

            let (output, new_context) = Self::run_single_phase(
                hand, i, user_input, &context, &outputs,
                runtime, router, tool_registry,
            ).await?;

            let phase_duration = phase_start.elapsed().as_secs_f64();

            // 自動品質評分（如果啟用）
            let quality_score = if let Some(evaluator) = quality_evaluator {
                if !output.skipped {
                    match evaluator.auto_evaluate(
                        &hand.phases[i], &output.output, router
                    ).await {
                        Ok(score) => Some(score),
                        Err(_) => None,
                    }
                } else {
                    None
                }
            } else {
                None
            };

            phase_metrics.push(PhaseMetrics {
                hand_name: hand.name.clone(),
                phase_name: output.phase_name.clone(),
                phase_index: i,
                duration_secs: phase_duration,
                // tokens 從 AgentResult 中取得（需要修改 run_single_phase 返回值）
                tokens_in: 0, // TODO: propagate from AgentResult
                tokens_out: 0,
                total_tokens: 0,
                estimated_cost_usd: 0.0,
                tool_calls: output.tool_calls,
                provider: hand.provider.clone(),
                model: hand.model.clone(),
                output_length: output.output.len(),
                quality_score,
                score_source: quality_score.map(|_| ScoreSource::LlmAuto {
                    model: "gemini-2.5-flash-lite".to_string(),
                }),
                skipped: output.skipped,
                timestamp: chrono::Utc::now(),
            });

            outputs.push(output);
            context = new_context;
        }

        // 記錄整體指標
        if let Some(store) = metrics_store {
            let hand_metrics = HandMetrics {
                run_id: run_id.clone(),
                hand_name: hand.name.clone(),
                user_input_preview: user_input[..user_input.len().min(200)].to_string(),
                phases: phase_metrics,
                total_duration_secs: start.elapsed().as_secs_f64(),
                total_tokens: 0, // TODO: sum
                total_cost_usd: 0.0,
                overall_quality: None, // 計算加權平均
                outcome: HandOutcome::Success,
                chain_to: hand.chain_to.clone(),
                timestamp: chrono::Utc::now(),
            };
            let _ = store.record_hand_run(&hand_metrics);
        }

        // ... 原有返回邏輯
        todo!()
    }
}
```

### 1.5 Telegram 指令

```
/metrics [hand_name] [days]     — 查看 Hand 效能報告
/slowest [days]                 — 最慢的 5 個 phase
/worst [days]                   — 品質最差的 5 個 phase
/rate <run_id> <score>          — 人工評分特定執行
/trend <hand_name> [days]       — 效能趨勢圖（文字版）
```

---

## 2. Prompt 自動優化 (`src/evolution/prompt_optimizer.rs`)

### 2.1 流程

```
MetricsStore
    │
    │ 查詢：quality_score < 6.0 的 phase（最近 7 天）
    ▼
┌──────────────────────────────────┐
│ PromptOptimizer::diagnose()      │
│                                  │
│ 輸入：                           │
│   - phase.system_prompt          │
│   - 最近 5 次執行的 output       │
│   - 對應的 quality_score         │
│                                  │
│ 用 Gemini Flash 分析：            │
│   1. 為什麼輸出品質差？           │
│   2. system_prompt 哪裡不清晰？  │
│   3. 建議的改進版本              │
│   4. 改進的預期效果              │
└──────────────┬───────────────────┘
               │ PromptImprovement
               ▼
┌──────────────────────────────────┐
│ Telegram 審批                     │
│                                  │
│ 📋 Prompt 改進建議               │
│ Hand: content                    │
│ Phase: topic_research            │
│ 目前平均分: 4.2/10               │
│                                  │
│ 問題診斷:                        │
│ - 太泛的搜尋指令                 │
│ - 未要求數據來源                 │
│                                  │
│ 改進 Diff:                       │
│ - 舊: "Research the topic..."    │
│ + 新: "Research the topic using  │
│   at least 3 different sources.  │
│   For each source, cite the URL  │
│   and publication date..."       │
│                                  │
│ /approve prompt_abc123           │
│ /deny prompt_abc123              │
└──────────────┬───────────────────┘
               │ Approved
               ▼
┌──────────────────────────────────┐
│ SafeGuard::apply_prompt_change() │
│                                  │
│ 1. 備份原 hand.toml              │
│ 2. 修改 system_prompt            │
│ 3. 標記 version + timestamp      │
│ 4. 下次執行後比較效能            │
└──────────────────────────────────┘
```

### 2.2 數據結構

```rust
/// Prompt 改進建議
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptImprovement {
    pub id: String,
    pub hand_name: String,
    pub phase_name: String,
    pub phase_index: usize,
    /// 目前的 system_prompt
    pub current_prompt: String,
    /// 建議的新 system_prompt
    pub suggested_prompt: String,
    /// 問題診斷
    pub diagnosis: String,
    /// 預期改進幅度（分）
    pub expected_improvement: f64,
    /// 基於最近 N 次執行的平均品質分
    pub current_avg_quality: f64,
    /// 審批狀態
    pub status: ImprovementStatus,
    /// 生成時間
    pub created_at: DateTime<Utc>,
    /// 審批時間
    pub reviewed_at: Option<DateTime<Utc>>,
    /// 套用後的新平均品質分（回測用）
    pub post_apply_avg_quality: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ImprovementStatus {
    Pending,
    Approved,
    Denied,
    Applied,
    /// 套用後效果更差，已 rollback
    RolledBack,
}

/// Prompt 優化器
pub struct PromptOptimizer {
    metrics_store: Arc<MetricsStore>,
    router: Arc<LlmRouter>,
    /// 用於診斷的 provider（推薦 gemini — 免費 + 長 context）
    eval_provider: String,
    eval_model: String,
    /// 觸發優化的品質門檻
    quality_threshold: f64,    // 預設 6.0
    /// 最少需要多少次執行才觸發分析
    min_runs_for_analysis: u32, // 預設 5
}

impl PromptOptimizer {
    /// 掃描所有 phase，找出需要優化的
    pub async fn scan_for_improvements(
        &self,
        hands: &HandRegistry,
    ) -> Result<Vec<PromptImprovement>> {
        let low_quality = self.metrics_store.lowest_quality_phases(7, 20)?;

        let mut improvements = Vec::new();
        for perf in low_quality {
            if perf.avg_quality >= self.quality_threshold {
                continue; // 已達標，跳過
            }
            if perf.run_count < self.min_runs_for_analysis {
                continue; // 樣本不足
            }

            if let Some(hand) = hands.get(&perf.hand_name) {
                if let Some(phase) = hand.phases.get(perf.phase_name_to_index(&hand.phases)) {
                    let improvement = self.diagnose_and_suggest(
                        &hand, phase, &perf
                    ).await?;
                    improvements.push(improvement);
                }
            }
        }

        Ok(improvements)
    }

    /// 診斷單一 phase 並生成改進建議
    async fn diagnose_and_suggest(
        &self,
        hand: &Hand,
        phase: &Phase,
        perf: &PhasePerformance,
    ) -> Result<PromptImprovement> {
        // 取得最近的低分 output 樣本
        let recent_outputs = self.metrics_store
            .recent_phase_outputs(&hand.name, &phase.name, 5)?;

        let diagnosis_prompt = format!(
            r#"你是 AI prompt 工程專家。分析以下 prompt 的問題並建議改進。

## 目前的 System Prompt
```
{}
```

## 最近 5 次執行結果（品質分 0-10）
{}

## 統計
- 平均品質分: {:.1}/10
- 平均耗時: {:.1}s
- 平均 token: {}

## 任務
1. 找出 prompt 導致低品質輸出的根本原因（最多 3 個）
2. 寫一個改進版的 system prompt
3. 改進版必須保留原版的核心意圖，但修正模糊/低效之處

回覆 JSON：
{{
  "diagnosis": "根本原因分析...",
  "improved_prompt": "完整的改進版 system prompt...",
  "changes_summary": "修改了什麼...",
  "expected_improvement": 1.5
}}"#,
            phase.system_prompt,
            recent_outputs.iter().enumerate().map(|(i, (output, score))| {
                format!("### 樣本 {} (分: {:.1})\n{}\n",
                    i + 1,
                    score,
                    &output[..output.len().min(500)]
                )
            }).collect::<Vec<_>>().join("\n"),
            perf.avg_quality,
            perf.avg_duration_secs,
            perf.avg_tokens,
        );

        // 用 Gemini Flash 做分析（免費）
        // ... router.chat() 調用，解析回覆
        todo!()
    }
}
```

### 2.3 自動回測

```rust
/// 套用 prompt 改進後的自動回測
pub struct PromptBacktester {
    metrics_store: Arc<MetricsStore>,
    /// 套用後需要觀察多少次執行
    observation_runs: u32, // 預設 5
    /// 如果新版品質低於舊版超過此閾值，自動 rollback
    regression_threshold: f64, // 預設 1.0 分
}

impl PromptBacktester {
    /// 檢查已套用的改進是否有效
    /// 在每次 Hand 執行後調用
    pub async fn check_and_rollback(
        &self,
        improvement: &PromptImprovement,
    ) -> Result<Option<RollbackAction>> {
        // 取得套用後的執行數據
        let post_runs = self.metrics_store.phase_quality_since(
            &improvement.hand_name,
            &improvement.phase_name,
            &improvement.reviewed_at.unwrap_or(Utc::now()),
        )?;

        if post_runs.len() < self.observation_runs as usize {
            return Ok(None); // 觀察期未結束
        }

        let post_avg: f64 = post_runs.iter().sum::<f64>() / post_runs.len() as f64;

        if post_avg < improvement.current_avg_quality - self.regression_threshold {
            // 效能退步，自動 rollback
            Ok(Some(RollbackAction {
                improvement_id: improvement.id.clone(),
                reason: format!(
                    "品質退步: {:.1} → {:.1} (閾值: -{:.1})",
                    improvement.current_avg_quality, post_avg, self.regression_threshold
                ),
                original_prompt: improvement.current_prompt.clone(),
            }))
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug, Clone)]
pub struct RollbackAction {
    pub improvement_id: String,
    pub reason: String,
    pub original_prompt: String,
}
```

---

## 3. Tool 自動擴展 (`src/evolution/tool_expander.rs`)

### 3.1 缺失能力偵測

```rust
/// 缺失能力記錄
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityGap {
    pub id: String,
    /// 發現來源（哪個 Hand 的哪個 phase）
    pub discovered_in: String,
    /// 描述：agent 表達了什麼需求
    pub description: String,
    /// 建議的 tool 名稱
    pub suggested_tool_name: String,
    /// 建議的 tool 功能
    pub suggested_actions: Vec<String>,
    /// 需要的外部 API/服務
    pub external_dependencies: Vec<String>,
    /// 優先級（基於出現頻率和營收影響）
    pub priority: GapPriority,
    /// 出現次數
    pub occurrence_count: u32,
    /// 狀態
    pub status: GapStatus,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum GapPriority {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GapStatus {
    /// 已識別但尚未處理
    Identified,
    /// 已排程生成
    Scheduled,
    /// 已生成代碼，等待審核
    CodeGenerated { code_path: String },
    /// 已審核通過
    Approved,
    /// 已編譯整合
    Integrated,
    /// 放棄（不值得實作）
    Dismissed { reason: String },
}
```

### 3.2 偵測邏輯

在 agent 輸出中掃描特定模式來偵測缺失能力：

```rust
/// 缺失能力偵測器
pub struct CapabilityDetector {
    /// 已知 tool 名稱列表（用於排除已有的）
    known_tools: Vec<String>,
}

impl CapabilityDetector {
    /// 分析 agent 輸出，偵測是否提到了缺少的能力
    pub fn detect_gaps(&self, output: &str, hand_name: &str, phase_name: &str) -> Vec<CapabilityGap> {
        let mut gaps = Vec::new();

        // 模式 1: 明確表達「我無法...」「我沒有...的能力」
        let inability_patterns = [
            r"(?i)(I (?:can't|cannot|don't have|lack|am unable to|don't have access to))\s+(.+?)[\.\n]",
            r"(?i)(沒有|無法|缺少|不具備|沒辦法)\s*(.+?)(的功能|的工具|的能力|的 API|tool)",
            r"(?i)(would need|requires?|need access to)\s+(a |an |the )?(\w+ (?:API|tool|service|integration))",
        ];

        // 模式 2: 提到了特定 API 或服務
        let api_patterns = [
            ("linkedin", "LinkedIn API integration"),
            ("github api", "GitHub API tool"),
            ("slack", "Slack messaging tool"),
            ("discord", "Discord bot integration"),
            ("notion", "Notion API tool"),
            ("google sheets", "Google Sheets API tool"),
            ("calendar", "Calendar/scheduling API"),
            ("payment", "Payment processing tool"),
            ("sms", "SMS sending tool"),
            ("whatsapp", "WhatsApp messaging tool"),
        ];

        for (keyword, description) in &api_patterns {
            if output.to_lowercase().contains(keyword)
                && !self.known_tools.iter().any(|t| t.to_lowercase().contains(keyword))
            {
                gaps.push(CapabilityGap {
                    id: uuid::Uuid::new_v4().to_string(),
                    discovered_in: format!("{}::{}", hand_name, phase_name),
                    description: description.to_string(),
                    suggested_tool_name: keyword.replace(' ', "_"),
                    suggested_actions: vec![], // 由 LLM 分析填入
                    external_dependencies: vec![keyword.to_string()],
                    priority: GapPriority::Medium,
                    occurrence_count: 1,
                    status: GapStatus::Identified,
                    first_seen: Utc::now(),
                    last_seen: Utc::now(),
                });
            }
        }

        gaps
    }
}
```

### 3.3 Tool 代碼生成流程

```
CapabilityGap (priority >= High, occurrence >= 3)
    │
    ▼
┌──────────────────────────────────────────────┐
│ ToolGenerator::generate_tool_code()           │
│                                              │
│ 使用 Claude Code (subprocess) 生成：          │
│                                              │
│ 輸入：                                       │
│   - gap.description                          │
│   - gap.suggested_actions                    │
│   - 現有 tool 的程式碼範本                    │
│     (src/tools/http_request.rs 作為參考)      │
│   - ToolSpec trait 的介面定義                │
│                                              │
│ 輸出：                                       │
│   - src/tools/{new_tool}.rs                  │
│   - 對應的 tests                             │
│   - 需要加入的 Cargo.toml 依賴              │
│                                              │
│ 生成後存入：                                  │
│   ~/.phantom-mesh/evolution/generated_tools/      │
│     {tool_name}/                             │
│       code.rs                                │
│       tests.rs                               │
│       deps.toml                              │
│       review_notes.md                        │
└─────────────────┬────────────────────────────┘
                  │
                  ▼
      Telegram 審批 (包含代碼 diff)
                  │
                  ▼ Approved
┌──────────────────────────────────────────────┐
│ ToolIntegrator::integrate()                   │
│                                              │
│ 1. 複製 code.rs → src/tools/                 │
│ 2. 更新 src/tools/mod.rs (加入 pub mod)      │
│ 3. 更新 main.rs (註冊 tool)                  │
│ 4. 更新 Cargo.toml (加入依賴)                │
│ 5. cargo check — 編譯測試                    │
│ 6. cargo test — 跑測試                       │
│ 7. 如果失敗 → rollback 所有修改              │
│ 8. 如果成功 → 通知 Telegram                  │
└──────────────────────────────────────────────┘
```

```rust
/// Tool 代碼生成器
pub struct ToolGenerator {
    /// Claude Code CLI 路徑
    claude_code_path: String,
    /// 模板 tool 的路徑（作為範例給 Claude Code）
    template_tool_path: String,
    /// 生成結果存放目錄
    output_dir: String,
}

impl ToolGenerator {
    /// 使用 Claude Code 生成新 tool 的 Rust 程式碼
    pub async fn generate(&self, gap: &CapabilityGap) -> Result<GeneratedTool> {
        let prompt = format!(
            r#"為 phantom-mesh AI agent 系統生成一個新的 Rust tool。

## 需求
名稱: {}
描述: {}
動作: {}

## 現有架構
- Tool trait: execute(&self, name: &str, args: Value) -> Result<ToolResult>
- 每個 tool 在 src/tools/ 下有獨立 .rs 檔
- 使用 serde_json::Value 作為參數
- 回傳 ToolResult {{ success: bool, output: String }}
- 必須包含 #[cfg(test)] mod tests

## 參考範本
{}

## 輸出
生成完整的 Rust 程式碼檔案，包含：
1. Tool struct 和實作
2. 所有必要的 HTTP/API 呼叫邏輯
3. 錯誤處理
4. 至少 3 個單元測試
5. 必要的外部 crate 依賴列表"#,
            gap.suggested_tool_name,
            gap.description,
            gap.suggested_actions.join(", "),
            std::fs::read_to_string(&self.template_tool_path).unwrap_or_default(),
        );

        // 調用 Claude Code subprocess
        let output = tokio::process::Command::new(&self.claude_code_path)
            .args(["--print", "--dangerously-skip-permissions", &prompt])
            .output()
            .await?;

        // 解析輸出，提取 Rust 代碼
        let code = String::from_utf8(output.stdout)?;
        // ... 存檔到 output_dir
        todo!()
    }
}

#[derive(Debug, Clone)]
pub struct GeneratedTool {
    pub tool_name: String,
    pub code_path: String,
    pub test_path: String,
    pub dependencies: Vec<String>,
    pub review_notes: String,
}
```

---

## 4. 模型自動選擇 (`src/evolution/model_selector.rs`)

### 4.1 A/B 測試引擎

```rust
/// A/B 測試設定
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbTest {
    pub id: String,
    /// 測試哪個 Hand 的哪個 phase
    pub hand_name: String,
    pub phase_name: String,
    /// 候選 provider + model 組合
    pub candidates: Vec<ModelCandidate>,
    /// 每個候選需要跑多少次
    pub runs_per_candidate: u32,
    /// 已完成的執行記錄
    pub completed_runs: Vec<AbTestRun>,
    /// 測試狀態
    pub status: AbTestStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCandidate {
    pub provider: String,
    pub model: String,
    /// 已完成次數
    pub completed: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbTestRun {
    pub candidate_index: usize,
    pub provider: String,
    pub model: String,
    pub quality_score: f64,
    pub duration_secs: f64,
    pub tokens: u32,
    pub cost_usd: f64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AbTestStatus {
    Running,
    Completed,
    Cancelled,
}

/// A/B 測試結果
#[derive(Debug, Clone, Serialize)]
pub struct AbTestResult {
    pub winner: ModelCandidate,
    pub scores: Vec<CandidateScore>,
    /// 是否建議切換路由
    pub recommend_switch: bool,
    pub recommendation: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CandidateScore {
    pub provider: String,
    pub model: String,
    pub avg_quality: f64,
    pub avg_duration_secs: f64,
    pub avg_cost_usd: f64,
    /// 綜合分 = quality * 0.5 + speed_score * 0.3 + cost_score * 0.2
    pub composite_score: f64,
}
```

### 4.2 模型選擇器

```rust
/// 模型自動選擇器
pub struct ModelSelector {
    metrics_store: Arc<MetricsStore>,
    /// 正在進行的 A/B 測試
    active_tests: Arc<RwLock<HashMap<String, AbTest>>>,
}

impl ModelSelector {
    /// 建立新的 A/B 測試
    pub async fn create_ab_test(
        &self,
        hand_name: &str,
        phase_name: &str,
        candidates: Vec<(String, String)>, // (provider, model) pairs
        runs_per_candidate: u32,
    ) -> Result<AbTest> { todo!() }

    /// 在 Hand 執行前調用：決定這次執行用哪個 provider
    /// 如果有 active A/B test，按輪替分配；否則返回 Hand 的預設 provider
    pub fn select_provider(
        &self,
        hand_name: &str,
        phase_name: &str,
    ) -> Option<(String, String)> { // (provider, model)
        // 檢查是否有 active A/B test
        // 如果有，返回尚未達到 runs_per_candidate 的下一個候選
        // 如果沒有，返回 None（使用預設）
        todo!()
    }

    /// A/B 測試完成後，分析結果
    pub fn analyze_results(&self, test: &AbTest) -> AbTestResult {
        let mut scores: Vec<CandidateScore> = Vec::new();

        for (i, candidate) in test.candidates.iter().enumerate() {
            let runs: Vec<&AbTestRun> = test.completed_runs.iter()
                .filter(|r| r.candidate_index == i)
                .collect();

            if runs.is_empty() { continue; }

            let avg_quality = runs.iter().map(|r| r.quality_score).sum::<f64>() / runs.len() as f64;
            let avg_duration = runs.iter().map(|r| r.duration_secs).sum::<f64>() / runs.len() as f64;
            let avg_cost = runs.iter().map(|r| r.cost_usd).sum::<f64>() / runs.len() as f64;

            // 正規化分數 (higher is better)
            let speed_score = 10.0 / (1.0 + avg_duration / 10.0); // 越快分越高
            let cost_score = 10.0 / (1.0 + avg_cost * 100.0);     // 越便宜分越高

            let composite = avg_quality * 0.5 + speed_score * 0.3 + cost_score * 0.2;

            scores.push(CandidateScore {
                provider: candidate.provider.clone(),
                model: candidate.model.clone(),
                avg_quality,
                avg_duration_secs: avg_duration,
                avg_cost_usd: avg_cost,
                composite_score: composite,
            });
        }

        scores.sort_by(|a, b| b.composite_score.partial_cmp(&a.composite_score).unwrap());
        let winner = test.candidates[test.completed_runs.iter()
            .find(|r| r.provider == scores[0].provider && r.model == scores[0].model)
            .map(|r| r.candidate_index)
            .unwrap_or(0)].clone();

        AbTestResult {
            winner,
            recommend_switch: scores.len() > 1 && (scores[0].composite_score - scores[1].composite_score) > 0.5,
            recommendation: format!(
                "建議: {} ({}): 綜合分 {:.2} (品質 {:.1}, 速度 {:.1}s, 成本 ${:.4})",
                scores[0].provider, scores[0].model,
                scores[0].composite_score, scores[0].avg_quality,
                scores[0].avg_duration_secs, scores[0].avg_cost_usd,
            ),
            scores,
        }
    }

    /// 自動更新路由表（需要 Telegram 審批）
    pub async fn apply_routing_change(
        &self,
        hand_name: &str,
        new_provider: &str,
        new_model: &str,
    ) -> Result<RoutingChange> { todo!() }
}

#[derive(Debug, Clone, Serialize)]
pub struct RoutingChange {
    pub hand_name: String,
    pub old_provider: String,
    pub old_model: String,
    pub new_provider: String,
    pub new_model: String,
    pub reason: String,
}
```

### 4.3 Telegram 指令

```
/abtest <hand> <phase> <provider1:model1> <provider2:model2> [runs=5]
    — 建立新的 A/B 測試
/abtest status
    — 查看進行中的 A/B 測試
/abtest results <test_id>
    — 查看測試結果
/switch <hand> <provider> <model>
    — 手動切換 Hand 的 provider（跳過 A/B 測試）
```

---

## 5. Hand 自動改進 (`src/evolution/hand_improver.rs`)

### 5.1 失敗分析

```rust
/// Hand 改進器
pub struct HandImprover {
    metrics_store: Arc<MetricsStore>,
    revenue_tracker: Arc<RevenueTracker>,
    router: Arc<LlmRouter>,
}

impl HandImprover {
    /// 分析失敗的 Hand 執行，生成改進建議
    pub async fn analyze_failures(
        &self,
        hand_name: &str,
        days: u32,
    ) -> Result<HandImprovementPlan> {
        let failures = self.metrics_store.failed_runs(hand_name, days)?;

        if failures.is_empty() {
            return Ok(HandImprovementPlan::no_issues(hand_name));
        }

        // 分類失敗模式
        let mut patterns: HashMap<String, Vec<&HandMetrics>> = HashMap::new();
        for failure in &failures {
            let pattern = self.classify_failure(failure);
            patterns.entry(pattern).or_default().push(failure);
        }

        // 用 LLM 分析失敗模式並建議修改
        let analysis_prompt = format!(
            r#"分析以下 Hand 工作流的失敗模式並建議改進。

Hand: {}
失敗次數: {} (最近 {} 天)

## 失敗模式分類
{}

## 任務
1. 找出最常見的失敗根因
2. 建議 Hand TOML 的結構性修改（新增/修改/刪除 phase）
3. 建議工具配置修改
4. 預估修改後的成功率提升

回覆 JSON：
{{
  "root_causes": ["..."],
  "structural_changes": [
    {{"type": "modify_phase", "phase": "...", "change": "..."}},
    {{"type": "add_phase", "after": "...", "new_phase": {{...}}}},
    {{"type": "add_tool", "tool": "..."}},
    {{"type": "adjust_max_rounds", "phase": "...", "new_value": N}}
  ],
  "expected_success_rate_improvement": 0.2,
  "confidence": "high|medium|low"
}}"#,
            hand_name,
            failures.len(),
            days,
            patterns.iter().map(|(pattern, runs)| {
                format!("- {} ({}次): {}", pattern, runs.len(),
                    runs.first().map(|r| r.user_input_preview.as_str()).unwrap_or(""))
            }).collect::<Vec<_>>().join("\n"),
        );

        // ... 調用 LLM，解析回覆
        todo!()
    }

    /// 基於營收數據建議新 Hand
    pub async fn suggest_new_hands(&self, days: u32) -> Result<Vec<NewHandSuggestion>> {
        // 查詢營收數據：哪條路線最賺錢？
        let by_route = self.revenue_tracker.by_route(days)?;
        let by_source = self.revenue_tracker.by_source(days)?;

        // 查詢 Hand 執行數據：哪個 Hand 跟營收關聯最高？
        let hand_revenue_correlation = self.correlate_hands_to_revenue(days).await?;

        let analysis_prompt = format!(
            r#"基於以下營收和工作流數據，建議新的 Hand 工作流。

## 營收按路線 (最近 {} 天)
{}

## 營收按來源
{}

## 現有 Hand → 營收關聯
{}

## 任務
建議 1-3 個新的 Hand 工作流，能夠：
1. 強化最賺錢的路線
2. 開拓未利用的高潛力路線
3. 自動化目前需要人工的步驟

每個建議包含：
- Hand 名稱和描述
- Phase 列表（名稱 + system_prompt 摘要）
- 需要的 tools
- 預估每月營收貢獻

回覆 JSON 格式。"#,
            days,
            by_route.iter().map(|r| format!("  {} — ${:.2} ({} 筆)", r.group, r.total_usd, r.count)).collect::<Vec<_>>().join("\n"),
            by_source.iter().map(|r| format!("  {} — ${:.2}", r.group, r.total_usd)).collect::<Vec<_>>().join("\n"),
            hand_revenue_correlation.iter().map(|(h, r)| format!("  {} → ${:.2}", h, r)).collect::<Vec<_>>().join("\n"),
        );

        // ... 調用 LLM，解析回覆
        todo!()
    }

    /// 分類失敗類型
    fn classify_failure(&self, metrics: &HandMetrics) -> String {
        match &metrics.outcome {
            HandOutcome::Timeout => "timeout".to_string(),
            HandOutcome::Failed { error } => {
                if error.contains("rate limit") || error.contains("Rate limit") {
                    "rate_limited"
                } else if error.contains("Budget exceeded") {
                    "budget_exceeded"
                } else if error.contains("no model") || error.contains("provider") {
                    "provider_unavailable"
                } else if error.contains("loop detected") {
                    "infinite_loop"
                } else {
                    "other_error"
                }
            }.to_string(),
            HandOutcome::PartialSuccess { completed_phases, total_phases } => {
                format!("partial_{}_of_{}", completed_phases, total_phases)
            },
            HandOutcome::Success => "false_alarm".to_string(),
        }
    }
}

/// 新 Hand 建議
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewHandSuggestion {
    pub name: String,
    pub description: String,
    pub category: String,
    pub phases: Vec<SuggestedPhase>,
    pub tools: Vec<String>,
    pub target_route: String,
    pub estimated_monthly_revenue: f64,
    pub confidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestedPhase {
    pub name: String,
    pub system_prompt_summary: String,
    pub max_rounds: u32,
}

/// Hand 改進計畫
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandImprovementPlan {
    pub hand_name: String,
    pub root_causes: Vec<String>,
    pub structural_changes: Vec<StructuralChange>,
    pub expected_improvement: f64,
    pub confidence: String,
}

impl HandImprovementPlan {
    pub fn no_issues(hand_name: &str) -> Self {
        Self {
            hand_name: hand_name.to_string(),
            root_causes: vec![],
            structural_changes: vec![],
            expected_improvement: 0.0,
            confidence: "n/a".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StructuralChange {
    ModifyPhase { phase: String, change: String },
    AddPhase { after: String, new_phase: SuggestedPhase },
    RemovePhase { phase: String, reason: String },
    AddTool { tool: String },
    RemoveTool { tool: String, reason: String },
    AdjustMaxRounds { phase: String, new_value: u32 },
    ChangeProvider { new_provider: String, new_model: String },
}
```

### 5.2 自動生成 Hand TOML

```rust
/// Hand TOML 生成器
pub struct HandTomlGenerator;

impl HandTomlGenerator {
    /// 從建議生成完整的 Hand TOML 檔案
    pub fn generate(suggestion: &NewHandSuggestion) -> String {
        let mut toml = format!(
            r#"name = "{}"
description = "{}"
category = "{}"
provider = "auto"
output_format = "markdown"
tools = [{}]

[settings]
"#,
            suggestion.name,
            suggestion.description,
            suggestion.category,
            suggestion.tools.iter()
                .map(|t| format!("\"{}\"", t))
                .collect::<Vec<_>>()
                .join(", "),
        );

        for phase in &suggestion.phases {
            toml.push_str(&format!(
                r#"
[[phases]]
name = "{}"
system_prompt = """{}"""
max_rounds = {}
"#,
                phase.name,
                phase.system_prompt_summary,
                phase.max_rounds,
            ));
        }

        toml
    }

    /// 對現有 Hand 套用結構性變更，生成新版 TOML
    pub fn apply_changes(
        original_toml: &str,
        changes: &[StructuralChange],
    ) -> Result<String> {
        let mut hand: Hand = toml::from_str(original_toml)?;

        for change in changes {
            match change {
                StructuralChange::AdjustMaxRounds { phase, new_value } => {
                    if let Some(p) = hand.phases.iter_mut().find(|p| &p.name == phase) {
                        p.max_rounds = *new_value;
                    }
                },
                StructuralChange::AddTool { tool } => {
                    if !hand.tools.contains(tool) {
                        hand.tools.push(tool.clone());
                    }
                },
                StructuralChange::RemoveTool { tool, .. } => {
                    hand.tools.retain(|t| t != tool);
                },
                StructuralChange::AddPhase { after, new_phase } => {
                    let idx = hand.phases.iter().position(|p| &p.name == after)
                        .unwrap_or(hand.phases.len() - 1);
                    hand.phases.insert(idx + 1, Phase {
                        name: new_phase.name.clone(),
                        system_prompt: new_phase.system_prompt_summary.clone(),
                        max_rounds: new_phase.max_rounds,
                        condition: None,
                    });
                },
                StructuralChange::RemovePhase { phase, .. } => {
                    hand.phases.retain(|p| &p.name != phase);
                },
                StructuralChange::ModifyPhase { phase, change } => {
                    if let Some(p) = hand.phases.iter_mut().find(|p| &p.name == phase) {
                        // change 是改進後的 system_prompt
                        p.system_prompt = change.clone();
                    }
                },
                StructuralChange::ChangeProvider { new_provider, new_model } => {
                    hand.provider = new_provider.clone();
                    hand.model = new_model.clone();
                },
            }
        }

        Ok(toml::to_string_pretty(&hand)?)
    }
}
```

---

## 6. 安全護欄 (`src/evolution/safeguard.rs`)

### 6.1 核心原則

```
┌─────────────────────────────────────────────────────────────────┐
│                      安全護欄規則                               │
│                                                                 │
│  1. 所有自我修改都必須經過 Telegram 人工審批                    │
│  2. 修改前自動備份到 ~/.phantom-mesh/evolution/backups/             │
│  3. 自動 rollback 如果新版效能更差（超過 regression_threshold） │
│  4. 禁區：不能修改以下檔案                                     │
│     - src/security/*                                           │
│     - src/approval.rs                                          │
│     - src/estop.rs                                             │
│     - src/evolution/safeguard.rs (不能修改自己)                │
│     - Cargo.toml 的 [dependencies] (防止引入惡意 crate)       │
│  5. 每日修改上限：3 個 prompt 變更 + 1 個 tool 生成            │
│  6. 變更必須附帶 diff，人工可在 Telegram 中看到具體修改       │
│  7. 每次變更都記錄審計日誌                                     │
└─────────────────────────────────────────────────────────────────┘
```

### 6.2 數據結構

```rust
use std::path::{Path, PathBuf};

/// 安全護欄
pub struct SafeGuard {
    /// 備份目錄
    backup_dir: PathBuf,
    /// 審計日誌 DB
    audit_db_path: String,
    /// 禁止修改的檔案/目錄模式
    forbidden_patterns: Vec<String>,
    /// 每日修改計數器
    daily_limits: DailyLimits,
}

#[derive(Debug, Clone)]
pub struct DailyLimits {
    pub max_prompt_changes: u32,
    pub max_tool_generations: u32,
    pub max_routing_changes: u32,
    /// 今日已使用
    pub prompt_changes_today: u32,
    pub tool_generations_today: u32,
    pub routing_changes_today: u32,
}

impl Default for DailyLimits {
    fn default() -> Self {
        Self {
            max_prompt_changes: 3,
            max_tool_generations: 1,
            max_routing_changes: 2,
            prompt_changes_today: 0,
            tool_generations_today: 0,
            routing_changes_today: 0,
        }
    }
}

/// 審計記錄
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub change_type: ChangeType,
    pub target: String,           // 被修改的檔案或配置
    pub description: String,
    pub diff: String,             // 文字 diff
    pub approved_by: String,      // Telegram user
    pub backup_path: String,      // 備份位置
    pub rolled_back: bool,
    pub rollback_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChangeType {
    PromptUpdate,
    HandStructure,
    NewTool,
    RoutingChange,
    NewHand,
}

impl SafeGuard {
    pub fn new(base_dir: &str) -> Result<Self> {
        let backup_dir = PathBuf::from(base_dir).join("evolution").join("backups");
        std::fs::create_dir_all(&backup_dir)?;

        let forbidden_patterns = vec![
            "src/security/".to_string(),
            "src/approval.rs".to_string(),
            "src/estop.rs".to_string(),
            "src/evolution/safeguard.rs".to_string(),
            "Cargo.toml".to_string(),       // 依賴修改單獨處理
            ".env".to_string(),
            "secrets".to_string(),
        ];

        Ok(Self {
            backup_dir,
            audit_db_path: format!("{}/evolution/audit.db", base_dir),
            forbidden_patterns,
            daily_limits: DailyLimits::default(),
        })
    }

    /// 檢查是否允許修改特定檔案
    pub fn can_modify(&self, file_path: &str) -> Result<()> {
        for pattern in &self.forbidden_patterns {
            if file_path.contains(pattern) {
                anyhow::bail!(
                    "安全護欄阻止: 檔案 '{}' 匹配禁止修改模式 '{}'",
                    file_path, pattern
                );
            }
        }
        Ok(())
    }

    /// 檢查每日限額
    pub fn check_daily_limit(&self, change_type: &ChangeType) -> Result<()> {
        match change_type {
            ChangeType::PromptUpdate => {
                if self.daily_limits.prompt_changes_today >= self.daily_limits.max_prompt_changes {
                    anyhow::bail!("每日 prompt 修改上限已達 ({})", self.daily_limits.max_prompt_changes);
                }
            },
            ChangeType::NewTool => {
                if self.daily_limits.tool_generations_today >= self.daily_limits.max_tool_generations {
                    anyhow::bail!("每日 tool 生成上限已達 ({})", self.daily_limits.max_tool_generations);
                }
            },
            ChangeType::RoutingChange => {
                if self.daily_limits.routing_changes_today >= self.daily_limits.max_routing_changes {
                    anyhow::bail!("每日路由修改上限已達 ({})", self.daily_limits.max_routing_changes);
                }
            },
            _ => {}
        }
        Ok(())
    }

    /// 備份檔案（修改前調用）
    pub fn backup(&self, file_path: &str) -> Result<String> {
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let file_name = Path::new(file_path)
            .file_name()
            .unwrap_or_default()
            .to_str()
            .unwrap_or("unknown");
        let backup_path = self.backup_dir.join(format!("{}_{}", timestamp, file_name));
        std::fs::copy(file_path, &backup_path)?;
        Ok(backup_path.to_str().unwrap_or("").to_string())
    }

    /// 回滾到備份版本
    pub fn rollback(&self, backup_path: &str, target_path: &str) -> Result<()> {
        std::fs::copy(backup_path, target_path)?;
        Ok(())
    }

    /// 生成可讀的 diff（給 Telegram 審批用）
    pub fn generate_diff(old_content: &str, new_content: &str) -> String {
        // 逐行比較，生成簡易 diff
        let old_lines: Vec<&str> = old_content.lines().collect();
        let new_lines: Vec<&str> = new_content.lines().collect();

        let mut diff = String::new();
        let max_len = old_lines.len().max(new_lines.len());

        for i in 0..max_len {
            let old = old_lines.get(i).unwrap_or(&"");
            let new = new_lines.get(i).unwrap_or(&"");
            if old != new {
                if !old.is_empty() {
                    diff.push_str(&format!("- {}\n", old));
                }
                if !new.is_empty() {
                    diff.push_str(&format!("+ {}\n", new));
                }
            }
        }

        if diff.len() > 3000 {
            format!("{}...\n(diff 太長，已截斷，共 {} 行差異)", &diff[..3000], diff.lines().count())
        } else {
            diff
        }
    }

    /// 記錄審計日誌
    pub fn record_audit(&self, entry: &AuditEntry) -> Result<()> {
        let conn = rusqlite::Connection::open(&self.audit_db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS audit_log (
                id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                change_type TEXT NOT NULL,
                target TEXT NOT NULL,
                description TEXT,
                diff TEXT,
                approved_by TEXT,
                backup_path TEXT,
                rolled_back INTEGER DEFAULT 0,
                rollback_reason TEXT
            )"
        )?;
        conn.execute(
            "INSERT INTO audit_log VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                entry.id,
                entry.timestamp.to_rfc3339(),
                format!("{:?}", entry.change_type),
                entry.target,
                entry.description,
                entry.diff,
                entry.approved_by,
                entry.backup_path,
                entry.rolled_back as i32,
                entry.rollback_reason,
            ],
        )?;
        Ok(())
    }

    /// 格式化 Telegram 審批訊息
    pub fn format_approval_message(
        &self,
        change_type: &ChangeType,
        target: &str,
        description: &str,
        diff: &str,
    ) -> String {
        let type_emoji = match change_type {
            ChangeType::PromptUpdate => "Prompt 優化",
            ChangeType::HandStructure => "Hand 結構修改",
            ChangeType::NewTool => "新 Tool 生成",
            ChangeType::RoutingChange => "路由表變更",
            ChangeType::NewHand => "新 Hand 建議",
        };
        format!(
            "Self-Evolution 變更請求\n\n\
             類型: {}\n\
             目標: {}\n\
             說明: {}\n\n\
             變更 Diff:\n```\n{}\n```\n\n\
             /approve evo_{{id}} — 批准\n\
             /deny evo_{{id}} — 拒絕",
            type_emoji, target, description, diff
        )
    }
}
```

---

## 7. 進化調度器 (`src/evolution/scheduler.rs`)

### 7.1 週期性任務

```rust
/// 自我進化調度器 — 整合所有子系統
pub struct EvolutionScheduler {
    metrics_store: Arc<MetricsStore>,
    prompt_optimizer: Arc<PromptOptimizer>,
    model_selector: Arc<ModelSelector>,
    hand_improver: Arc<HandImprover>,
    capability_detector: Arc<CapabilityDetector>,
    safeguard: Arc<SafeGuard>,
    hand_registry: Arc<HandRegistry>,
}

impl EvolutionScheduler {
    /// 每日進化掃描（建議排在凌晨 3:00 執行）
    /// 透過 cron 系統調度
    pub async fn daily_evolution_scan(&self) -> Result<EvolutionReport> {
        let mut report = EvolutionReport::new();

        // 1. 效能摘要
        report.performance_summary = self.generate_performance_summary(7).await?;

        // 2. 掃描需要優化的 prompt
        let prompt_improvements = self.prompt_optimizer
            .scan_for_improvements(&self.hand_registry).await?;
        report.prompt_suggestions = prompt_improvements.len();

        // 3. 掃描缺失能力
        let capability_gaps = self.scan_capability_gaps().await?;
        report.capability_gaps = capability_gaps.len();

        // 4. 檢查 A/B 測試結果
        let ab_results = self.model_selector.check_completed_tests().await?;
        report.ab_test_results = ab_results.len();

        // 5. 分析失敗的 Hand 並建議改進
        let hand_improvements = self.scan_hand_improvements(7).await?;
        report.hand_improvements = hand_improvements.len();

        // 6. 基於營收建議新 Hand
        let new_hand_suggestions = self.hand_improver.suggest_new_hands(30).await?;
        report.new_hand_suggestions = new_hand_suggestions.len();

        // 7. 發送報告到 Telegram
        // (由呼叫端處理)

        Ok(report)
    }
}

/// 每日進化報告
#[derive(Debug, Clone, Serialize)]
pub struct EvolutionReport {
    pub date: String,
    pub performance_summary: PerformanceSummary,
    pub prompt_suggestions: usize,
    pub capability_gaps: usize,
    pub ab_test_results: usize,
    pub hand_improvements: usize,
    pub new_hand_suggestions: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PerformanceSummary {
    pub total_hand_runs: u32,
    pub success_rate: f64,
    pub avg_quality: f64,
    pub total_cost_usd: f64,
    pub total_revenue_usd: f64,
    pub roi: f64,              // revenue / cost
    pub slowest_phase: String,
    pub lowest_quality_phase: String,
    pub best_performing_hand: String,
    pub worst_performing_hand: String,
}
```

### 7.2 Cron 整合

在 `src/cron.rs` 的 `JobAction` 中新增：

```rust
pub enum JobAction {
    Shell { command: String },
    Agent { agent: String, prompt: String },
    Notify { chat_id: String, message: String },
    Hand { hand_name: String, input: String },
    // 新增：
    Evolution { action: EvolutionAction },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EvolutionAction {
    /// 每日全面掃描
    DailyScan,
    /// 只掃描 prompt 優化
    PromptScan,
    /// 只檢查 A/B 測試
    AbTestCheck,
    /// 回測已套用的改進
    BacktestCheck,
}
```

預設 cron job:

```
# 每日凌晨 3:00 — 全面進化掃描
"0 3 * * *" evolution:DailyScan

# 每 6 小時 — 回測已套用改進
"0 */6 * * *" evolution:BacktestCheck
```

---

## 8. 模組結構

```
src/evolution/
    mod.rs              — pub mod 聲明 + EvolutionEngine 整合入口
    monitor.rs          — PhaseMetrics, HandMetrics, MetricsStore
    quality.rs          — QualityEvaluator, HumanFeedback
    prompt_optimizer.rs — PromptOptimizer, PromptImprovement
    backtester.rs       — PromptBacktester, RollbackAction
    tool_expander.rs    — CapabilityDetector, CapabilityGap
    tool_generator.rs   — ToolGenerator, ToolIntegrator
    model_selector.rs   — ModelSelector, AbTest, AbTestResult
    hand_improver.rs    — HandImprover, HandImprovementPlan
    toml_generator.rs   — HandTomlGenerator
    safeguard.rs        — SafeGuard, AuditEntry, DailyLimits
    scheduler.rs        — EvolutionScheduler, EvolutionReport
```

---

## 9. 整合到 main.rs 的啟動順序

```rust
// 在 main() 中，現有組件初始化之後：
let metrics_store = Arc::new(MetricsStore::new(&format!("{}/.phantom-mesh/evolution/metrics.db", home))?);
let quality_evaluator = QualityEvaluator::new("gemini", "gemini-2.5-flash-lite");
let safeguard = Arc::new(SafeGuard::new(&format!("{}/.phantom-mesh", home))?);

let prompt_optimizer = Arc::new(PromptOptimizer::new(
    metrics_store.clone(),
    router.clone(),
    "gemini".to_string(),
    "gemini-2.5-flash-lite".to_string(),
));

let model_selector = Arc::new(ModelSelector::new(metrics_store.clone()));

let hand_improver = Arc::new(HandImprover::new(
    metrics_store.clone(),
    revenue_tracker.clone(),
    router.clone(),
));

let evolution_scheduler = Arc::new(EvolutionScheduler::new(
    metrics_store.clone(),
    prompt_optimizer.clone(),
    model_selector.clone(),
    hand_improver.clone(),
    safeguard.clone(),
    hand_registry.clone(),
));

// 註冊 cron job
cron_scheduler.add_job(CronJob {
    name: "daily_evolution_scan".to_string(),
    schedule: Schedule::Cron { expr: "0 3 * * *".to_string() },
    action: JobAction::Evolution { action: EvolutionAction::DailyScan },
    ..Default::default()
});
```

---

## 10. Telegram 指令總覽

| 指令 | 功能 | 模組 |
|------|------|------|
| `/metrics [hand] [days]` | 查看效能報告 | monitor |
| `/slowest [days]` | 最慢的 phase | monitor |
| `/worst [days]` | 品質最差的 phase | monitor |
| `/rate <run_id> <score>` | 人工評分 | quality |
| `/trend <hand> [days]` | 效能趨勢 | monitor |
| `/optimize [hand]` | 手動觸發 prompt 優化掃描 | prompt_optimizer |
| `/gaps` | 查看缺失能力列表 | tool_expander |
| `/generate_tool <gap_id>` | 手動觸發 tool 生成 | tool_generator |
| `/abtest ...` | A/B 測試管理 | model_selector |
| `/switch <hand> <provider>` | 手動切換 provider | model_selector |
| `/improve <hand>` | 手動觸發 Hand 改進分析 | hand_improver |
| `/suggest_hand` | 基於營收建議新 Hand | hand_improver |
| `/evo_status` | 進化系統狀態總覽 | scheduler |
| `/evo_report` | 手動觸發進化報告 | scheduler |
| `/audit [days]` | 查看審計日誌 | safeguard |
| `/approve evo_<id>` | 批准進化變更 | safeguard |
| `/deny evo_<id>` | 拒絕進化變更 | safeguard |
| `/rollback <audit_id>` | 手動 rollback | safeguard |

---

## 11. 實施順序（建議分 3 個 Sprint）

### Sprint 1: 監控基礎 (2-3 天)
1. `MetricsStore` — SQLite 表結構 + CRUD
2. `PhaseMetrics` / `HandMetrics` — 數據記錄
3. `HandRunner::run_with_metrics()` — 整合到 Hand 執行
4. Telegram `/metrics`, `/slowest`, `/worst` 指令
5. 測試：至少 15 個單元測試

### Sprint 2: 品質評估 + Prompt 優化 (2-3 天)
1. `QualityEvaluator` — LLM 自動評分 + 人工回饋
2. `PromptOptimizer` — 診斷 + 建議生成
3. `PromptBacktester` — 自動回測 + rollback
4. `SafeGuard` — 備份 + 禁區 + 審計
5. Telegram `/rate`, `/optimize`, `/audit` 指令
6. 測試：至少 20 個單元測試

### Sprint 3: A/B 測試 + Hand 改進 + Tool 擴展 (3-4 天)
1. `ModelSelector` — A/B 測試引擎
2. `HandImprover` — 失敗分析 + 新 Hand 建議
3. `CapabilityDetector` — 缺失能力掃描
4. `ToolGenerator` — Claude Code 代碼生成（基礎版）
5. `EvolutionScheduler` — 整合 cron 調度
6. Telegram 完整指令集
7. 測試：至少 25 個單元測試

### 預估總工作量
- 約 3,000 - 4,000 行新 Rust 程式碼
- 約 60 個新測試
- 12 個新 Telegram 指令
- 3 個新 SQLite 表
- 1 個新 cron job

---

## 12. 風險與限制

| 風險 | 緩解措施 |
|------|----------|
| LLM 評分不穩定 | 多次評分取平均 + 人工校準 |
| Prompt 修改讓效能更差 | 自動回測 + regression_threshold |
| 生成的 tool 代碼有 bug | cargo check/test 自動驗證 + 人工審核 |
| 進化系統修改安全代碼 | 禁區白名單 + 每日上限 |
| 成本失控（大量 LLM 評分） | 用免費 Gemini Flash + 每日預算上限 |
| 無限循環自我修改 | 每日修改次數上限 + 冷卻期 |
| A/B 測試拖慢正常執行 | 測試只佔 20% 的執行次數 |

---

## 13. 成功指標

1. **監控覆蓋率**: 100% 的 Hand 執行都有指標記錄
2. **品質提升**: Prompt 優化後平均品質分提升 >= 1.0
3. **自動化程度**: >= 80% 的低分 phase 能自動產生改進建議
4. **安全記錄**: 0 次未經審批的修改
5. **ROI**: 進化系統每月成本 < $5 (主要用免費 Gemini)
6. **回測準確率**: 自動 rollback 的假陽性率 < 10%
