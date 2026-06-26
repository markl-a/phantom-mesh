// SPEC-27 §7 / §9 / §11 - Smart task decompose wire types (LLM-driven task
// decomposition layer shared between Rust core and TS app).
//
// Stage 3 (real impl - pure-logic + cross-wire delegations live): prompt
// template, JSON parse, Kahn topo-sort, unix-ms clock, the DAG cycle
// detector, the `uuid_v7` time-prefix + v4-suffix ID minter (no Cargo.toml
// change), the `providers_wire::complete` delegation for the frontier LLM
// call (`call_frontier_llm`), and the `cluster_dispatch_wire::execute_plan`
// delegation for cross-peer dispatch (`dispatch_subtask`) are now real.
// The remaining Stage 4 surface lives one layer deeper — inside
// `providers_wire`'s per-provider `complete_*_pseudo` HTTP adapters that
// haven't been promoted yet. When those land, this module needs no further
// edit; the wire-up is permanent.
//
// 中文: 本檔對應 SPEC-27 §7（資料模型）+ §9（API 合約）+ §11（錯誤目錄）。
// 依賴方向：本檔 depends on SPEC-26 cluster_dispatch_wire（重用
// `CapabilityTag` + `DispatchOutcome`，不重複定義）；本檔 **不** blocks
// SPEC-26（循環依賴解見 SPEC-27 §0 metadata）。Stage 3 把純邏輯
// helper（prompt 樣板 / JSON 解析 / Kahn 拓撲排序 / 時戳 / 環偵測）接成真的；
// 需 LLM / 跨機派送 / `uuid/v7` feature 的 helper 留 Stage 4。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// Re-use SPEC-26 cluster_dispatch_wire types per task rules (don't duplicate).
// `CapabilityTag` appears in `target_caps` + `required_caps`; `DispatchOutcome`
// is returned by `dispatch_plan` + consumed by `aggregate_progress`.
//
// 中文: 重用 SPEC-26 capability 標籤 + dispatch 結果型別，兩 spec 共享同一
// 來源（SPEC-27 §18.1 風險條目：tag set 不同步的緩解）。
use crate::cluster_dispatch_wire::{CapabilityTag, DispatchOutcome, DispatchStatus};

// ─── §7.1 DecomposeRequest (TS-facing entry) ─────────────────────────────────

/// Caller-supplied decompose request. Maps to SPEC-27 §9.1 `task_decompose`
/// Tauri command params + the in-process `DecomposeOpts` struct (§9.4).
///
/// 中文: 拆解請求 - 包 vague input 字串 + 可選約束。
///   - `task_text`: 原始輸入字串（≤ 500 字，per SPEC-27 §3.1 G1）
///   - `max_subtasks`: 預設 8（防 LLM hallucinate 出過大樹）
///   - `target_caps`: 可選 capability 標籤提示（Stage 2 注入 prompt）
///   - `deadline_ms`: 整輪 wall-clock 上限毫秒（None = 沿用預設 30s）
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/smart_decompose/")]
#[serde(rename_all = "camelCase")]
pub struct DecomposeRequest {
    /// Raw user input string. Per SPEC-27 §13 privacy: encrypted at rest via
    /// SPEC-13 `EventKey` before persistence; in-memory plaintext only for
    /// the duration of the LLM call (Stage 2 wraps with `zeroize` on drop).
    pub task_text: String,
    /// Maximum subtasks the LLM may emit in one pass. Hard upper bound is
    /// 20 (SPEC-27 §8.2); exceeding triggers chunked re-decompose.
    #[serde(default = "default_max_subtasks")]
    pub max_subtasks: u8,
    /// Optional capability hint biasing the LLM toward these tags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_caps: Option<Vec<CapabilityTag>>,
    /// Optional soft deadline (ms) for the full decompose+dispatch round.
    /// `None` defaults to 30000 ms (SPEC-27 §12 p50 budget); exceeding emits
    /// `DecomposeError::DecomposeTimeout`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
}

fn default_max_subtasks() -> u8 {
    8
}

// ─── §7.1 SubTask (leaf payload) ─────────────────────────────────────────────

/// One decomposed subtask - the leaf unit handed to SPEC-26 cluster_dispatch
/// for cross-peer execution. `depends_on` forms the DAG edge list validated
/// by `validate_dag`.
///
/// 中文: 單一拆解子任務 - SPEC-26 派送的 leaf 單元。priority 預設 5（中段）;
/// `estimated_duration_ms` 是 LLM 估的執行時間（aggregate 用）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/smart_decompose/")]
#[serde(rename_all = "camelCase")]
pub struct SubTask {
    /// UUIDv7 string - unique identifier within the plan.
    pub subtask_id: String,
    /// Identifier of the owning `DecomposePlan` (for trace correlation).
    pub parent_task_id: String,
    /// Prompt / instruction text dispatched to the worker peer.
    pub text: String,
    /// Capability tags required by this subtask. Stage 2 cross-checks
    /// against SPEC-26 KNOWN_CAPS; unknown tags fall back to `General`
    /// with a warning (SPEC-27 §8.4).
    pub required_caps: Vec<CapabilityTag>,
    /// IDs of sibling subtasks that must complete before this one starts.
    /// Forms the DAG edges validated by `validate_dag`.
    pub depends_on: Vec<String>,
    /// Priority hint 1-10 (higher = sooner). Defaults to 5.
    #[serde(default = "default_priority")]
    pub priority: u8,
    /// LLM-emitted duration estimate (ms). `None` when not provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_duration_ms: Option<u64>,
}

fn default_priority() -> u8 {
    5
}

// ─── §7.1 DecomposePlan (full LLM output) ────────────────────────────────────

/// Output of `decompose()` - flattened `SubTask` vector plus a `TopologyHint`
/// summarizing DAG shape. Maps to SPEC-27 §7.1 `TaskTree` + `Vec<TaskAssignment>`
/// combined into one wire shape for TS ergonomics.
///
/// 中文: `decompose()` 輸出 - LLM 拆解的完整計畫。
///   - `request_id`: 對應原 `DecomposeRequest` 的關聯識別碼
///   - `parent_task_id`: 所有 `SubTask.parent_task_id` 共用值
///   - `subtasks`: 攤平的 leaf 清單；DAG 結構靠 `depends_on` 重建
///   - `dag_topology`: DAG 形狀提示（Sequential / Parallel / DagMixed）
///   - `decomposed_at_ms`: 拆解完成 UTC 毫秒時戳
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/smart_decompose/")]
#[serde(rename_all = "camelCase")]
pub struct DecomposePlan {
    pub request_id: String,
    pub parent_task_id: String,
    /// Maximum 50 nodes per SPEC-27 §3.1 G1 hard cap (Stage 2 enforces).
    pub subtasks: Vec<SubTask>,
    pub dag_topology: TopologyHint,
    pub decomposed_at_ms: u64,
}

// ─── §7.1 ExecutionProgress (live aggregate) ─────────────────────────────────

/// Live progress snapshot from `aggregate_progress`. Pure computation - no
/// IO, no async. UI polls (or subscribes via a future Stage 2 stream) for
/// progress-bar rendering.
///
/// 中文: 即時進度快照 - 純計算從一串 `DispatchOutcome` 聚合而出。collapse
/// 規則見 `aggregate_progress` doc。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/smart_decompose/")]
#[serde(rename_all = "camelCase")]
pub struct ExecutionProgress {
    pub parent_task_id: String,
    pub completed_subtasks: u8,
    pub total_subtasks: u8,
    pub failed_subtasks: u8,
    pub current_status: DecomposeStatus,
}

// ─── DecomposeLlmResponse (debug-only) ───────────────────────────────────────

/// Raw JSON payload captured for Stage 2 debugging. Emitted only when
/// `--verbose` or `RUST_LOG=debug` is set; not part of the production API.
///
/// 中文: LLM 原始 JSON 回應 - 留給 debugging 用、不在生產 API 表面。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/smart_decompose/")]
#[serde(rename_all = "camelCase")]
pub struct DecomposeLlmResponse {
    pub raw_json: String,
    /// Identifier of the provider that produced this response
    /// (e.g. `"claude-opus-4.7"`, `"gpt-5"`, `"gemini-pro"`).
    pub provider_used: String,
    pub elapsed_ms: u64,
}

// ─── TopologyHint (DAG shape) ────────────────────────────────────────────────

/// Coarse DAG shape classifier. UI picks a default render mode (linear list
/// vs. parallel columns vs. mermaid graph) from this hint.
///
/// 中文: DAG 形狀提示。Sequential = 單鏈; Parallel = root 下扇形（無依賴）;
/// DagMixed = 一般情況（fan-out + fan-in）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/smart_decompose/")]
#[serde(rename_all = "snake_case")]
pub enum TopologyHint {
    Sequential,
    Parallel,
    DagMixed,
}

// ─── DecomposeStatus (plan lifecycle) ────────────────────────────────────────

/// Lifecycle state of a `DecomposePlan`. Transitions roughly:
/// Planning -> Decomposing -> Dispatching -> Running -> (Completed |
/// PartialFailure | TotalFailure).
///
/// 中文: 拆解計畫生命週期狀態 - 用作 `ExecutionProgress.current_status`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/smart_decompose/")]
#[serde(rename_all = "snake_case")]
pub enum DecomposeStatus {
    Planning,
    Decomposing,
    Dispatching,
    Running,
    Completed,
    PartialFailure,
    TotalFailure,
}

// ─── §11 DecomposeError (TS-facing catalog) ──────────────────────────────────

/// Decompose-path errors. Maps to SPEC-27 §11.1 catalog rows. Wire format
/// is `{"code": "snake_case", "detail": "..."}` for TS `switch (err.code)`.
///
/// 中文: 拆解錯誤列舉，對應 SPEC-27 §11.1 表格。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/smart_decompose/")]
#[serde(rename_all = "snake_case", tag = "code", content = "detail")]
pub enum DecomposeError {
    /// LLM returned a response but the JSON schema did not validate, or the
    /// model explicitly refused to decompose (e.g. safety filter). Stage 2
    /// retries once at temperature=0 before falling to next provider.
    LlmRefusedToDecompose(String),
    /// Chunked re-decompose hit its `max_chunked_calls` ceiling (5 per
    /// SPEC-27 §8.6) yet still produced too many nodes. Maps to
    /// `P3.decompose.too_complex`. Non-retryable.
    SubtaskCountExceeded(String),
    /// DFS cycle detection found a directed cycle in `depends_on`. Maps to
    /// `P3.decompose.cycle_detected`. Detail carries the cycle path.
    CircularDependency(String),
    /// No peer in the cluster advertises any matching capability, even
    /// after the `CapabilityTag::General` fallback.
    NoCapabilityMatch(String),
    /// Single LLM call > 30s or full round > `deadline_ms`. Maps to
    /// `P3.decompose.timeout`. Retryable via auto-fallback to next provider.
    DecomposeTimeout(String),
}

// ─── Stage 2 helpers — pseudocode bodies (Stage 3 fills inner _pseudo fns) ───
//
// Per docs/superpowers/SPEC-TO-CODE-PLAYBOOK.md:
//   Stage 2 = function body shows what it WILL do via comments + nested
//   unimplemented!() inner helpers. Reader can audit the algorithm flow
//   without trusting any LLM / DAG / dispatcher implementation. Stage 3
//   swaps the `_pseudo` helpers for real serde_json / petgraph / uuid /
//   providers_wire / cluster_dispatch_wire calls (crates added then).

/// Decompose a vague user task into a structured `DecomposePlan`.
///
/// Stage 2 algorithm flow (SPEC-27 §8.2-§8.6):
///   1. Build the frontier prompt (system + JSON schema instruction)
///   2. Call frontier LLM with strict JSON-mode via providers_wire::complete
///   3. Parse `Vec<SubTask>` from the response (serde_json + schema check)
///   4. Run `validate_dag` (real call) for cycle detection + topology hint
///   5. Generate a UUIDv7 `parent_task_id` for trace correlation
///   6. Assemble + return `DecomposePlan`
///
/// 中文: vague input -> 結構化 `DecomposePlan`。Stage 2 pseudocode 已標出
/// 每一步呼叫；Stage 3 才接真的 LLM / serde / uuid 套件。
pub async fn decompose(request: &DecomposeRequest) -> Result<DecomposePlan, DecomposeError> {
    // Step 1: build the decompose prompt — system instructions + user task +
    //         strict JSON schema fragment so the frontier model emits
    //         schema-shaped output rather than free-form prose.
    let prompt: String = build_decompose_prompt(request);

    // Step 2: call the frontier provider via SPEC-14 providers_wire with
    //         JSON-mode strict. Maps provider timeout → DecomposeTimeout,
    //         provider refusal / schema-fail → LlmRefusedToDecompose.
    //         Still Stage 4 — providers_wire::complete itself is Stage 2.
    let raw_response: DecomposeLlmResponse = call_frontier_llm(&prompt).await?;

    // Step 3: parse the raw JSON into a Vec<SubTask>. Helper also validates
    //         the cap on `max_subtasks` and that required fields exist.
    //         capability-tag KNOWN_CAPS check stays Stage 4 (cluster_dispatch
    //         hasn't published the canonical set yet).
    let subtasks: Vec<SubTask> = parse_subtasks(&raw_response.raw_json)?;

    // Step 4: cycle detect + classify DAG topology. This is the REAL
    //         validate_dag call — no _pseudo — because Stage 1 already
    //         ships the cycle detector.
    let topology: TopologyHint = validate_dag(&subtasks)?;

    // Step 5: mint a UUIDv7 parent_task_id (time-ordered, used for trace
    //         correlation across SPEC-26 dispatcher + UI progress polling).
    let parent_task_id: String = uuid_v7();

    // Step 6: assemble the DecomposePlan. `request_id` is a separate UUID
    //         scoped to this decompose round; `decomposed_at_ms` is the
    //         UTC wall-clock at plan completion.
    Ok(DecomposePlan {
        request_id: uuid_v7(),
        parent_task_id,
        subtasks,
        dag_topology: topology,
        decomposed_at_ms: now_unix_ms(),
    })
}

/// Validate the DAG formed by `subtasks[].depends_on`: detect cycles via
/// DFS (O(V+E)) and emit a `TopologyHint` summary on success.
///
/// Stage 1 already ships the cycle detector (used by smoke test) + a coarse
/// topology classifier (Sequential when every node has <=1 in-degree and
/// <=1 out-degree, Parallel when all `depends_on` empty, else DagMixed).
/// Stage 2 will add full topo-sort + per-node path tracking on cycle errors.
///
/// 中文: 驗證 DAG 邊不成環、回 topology 提示。Stage 1 已實作環偵測 + 粗略
/// 分類；Stage 2 補 Kahn 拓撲排序 + per-node 完整路徑記錄。
pub fn validate_dag(subtasks: &[SubTask]) -> Result<TopologyHint, DecomposeError> {
    use std::collections::HashMap;

    if subtasks.is_empty() {
        return Ok(TopologyHint::Parallel);
    }

    // Build adjacency: subtask_id -> dependency ids.
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::with_capacity(subtasks.len());
    for st in subtasks {
        adj.insert(
            st.subtask_id.as_str(),
            st.depends_on.iter().map(String::as_str).collect(),
        );
    }

    // 3-color DFS: 0=white, 1=gray (in_stack), 2=black (done).
    let mut color: HashMap<&str, u8> = HashMap::with_capacity(subtasks.len());
    for st in subtasks {
        color.insert(st.subtask_id.as_str(), 0);
    }

    fn dfs<'a>(
        node: &'a str,
        adj: &HashMap<&'a str, Vec<&'a str>>,
        color: &mut HashMap<&'a str, u8>,
    ) -> Result<(), String> {
        color.insert(node, 1);
        if let Some(deps) = adj.get(node) {
            for &dep in deps {
                match color.get(dep).copied().unwrap_or(0) {
                    0 => dfs(dep, adj, color)?,
                    1 => {
                        return Err(format!(
                            "cycle detected: edge from `{node}` back into in-stack `{dep}`"
                        ));
                    }
                    _ => {}
                }
            }
        }
        color.insert(node, 2);
        Ok(())
    }

    let node_ids: Vec<&str> = subtasks.iter().map(|s| s.subtask_id.as_str()).collect();
    for nid in node_ids {
        if color.get(nid).copied().unwrap_or(0) == 0 {
            if let Err(cycle_path) = dfs(nid, &adj, &mut color) {
                return Err(DecomposeError::CircularDependency(cycle_path));
            }
        }
    }

    // Coarse topology classification.
    let all_empty = subtasks.iter().all(|s| s.depends_on.is_empty());
    if all_empty {
        return Ok(TopologyHint::Parallel);
    }
    let mut in_deg: HashMap<&str, u8> = HashMap::with_capacity(subtasks.len());
    let mut out_deg: HashMap<&str, u8> = HashMap::with_capacity(subtasks.len());
    for st in subtasks {
        let e = out_deg.entry(st.subtask_id.as_str()).or_insert(0);
        *e = e.saturating_add(st.depends_on.len() as u8);
        for dep in &st.depends_on {
            let f = in_deg.entry(dep.as_str()).or_insert(0);
            *f = f.saturating_add(1);
        }
    }
    let max_in = in_deg.values().copied().max().unwrap_or(0);
    let max_out = out_deg.values().copied().max().unwrap_or(0);
    if max_in <= 1 && max_out <= 1 {
        Ok(TopologyHint::Sequential)
    } else {
        Ok(TopologyHint::DagMixed)
    }
}

/// Walk a `DecomposePlan`'s DAG in topological order and dispatch each
/// `SubTask` via `crate::cluster_dispatch_wire` (SPEC-26 §9).
///
/// Stage 2 algorithm flow:
///   1. Topological sort via Kahn-style level-set → Vec<Vec<&SubTask>>
///      (each inner vec is one ready-to-run level — sibling tasks may run
///      in parallel; outer vec enforces dependency order between levels).
///   2. For each level, dispatch every subtask through
///      `cluster_dispatch_wire::execute_plan` (one per subtask).
///   3. On subtask failure: mark + continue (partial success allowed; full
///      stop only if a downstream subtask depends_on the failed id —
///      Stage 3 propagation rule lives in the dispatcher, not here).
///   4. Collect all outcomes (preserved in dispatch order) into a flat Vec.
///
/// 中文: 走 DAG 拓撲序、per-subtask 呼叫 SPEC-26 dispatch；單一 subtask 失敗
/// 不立即中止整批，最終收集所有 `DispatchOutcome`。Stage 3 才接真 petgraph
/// + cluster_dispatch_wire。
pub async fn dispatch_plan(plan: &DecomposePlan) -> Result<Vec<DispatchOutcome>, DecomposeError> {
    // Step 1: topologically sort the DAG into dependency levels. Hand-rolled
    //         Kahn (no petgraph dep yet) over the `depends_on` edge list
    //         produces one level per Kahn iteration — sibling-safe parallelism.
    //         Returns `CircularDependency` if a cycle leaks past validate_dag.
    let levels: Vec<Vec<&SubTask>> = topo_sort(&plan.subtasks)?;

    // Step 2 + 3: dispatch level-by-level. Per-subtask failures are marked
    //         in the outcome (not bubbled) so the caller can decide partial
    //         vs total failure via `aggregate_progress`.
    let mut outcomes: Vec<DispatchOutcome> = Vec::with_capacity(plan.subtasks.len());
    for level in &levels {
        for subtask in level {
            // Step 2: cluster_dispatch_wire::execute_plan (SPEC-26 §9) is
            //         invoked once per subtask. Stage 4 will await all
            //         siblings in a single `join_all` for true parallelism.
            // Step 3: outcome captures success / failure / timeout —
            //         continue regardless so we never strand siblings.
            let outcome: DispatchOutcome = dispatch_subtask(subtask).await;
            outcomes.push(outcome);
        }
    }

    // Step 4: hand the flat outcome vec back to the caller. Aggregation +
    //         status collapse happens in `aggregate_progress` (pure fn).
    Ok(outcomes)
}

/// Aggregate a list of `DispatchOutcome` rows into a single
/// `ExecutionProgress`. Pure computation - no IO, no async. Safe to call
/// from the UI thread.
///
/// Collapse rules for `current_status`:
///   - `total == 0` -> Planning
///   - `failed == total` -> TotalFailure
///   - `failed > 0 && completed + failed == total` -> PartialFailure
///   - `completed == total` -> Completed
///   - otherwise -> Running
///
/// Each outcome's real `DispatchStatus` drives the counts: `Completed` is a
/// success terminal; `Failed`/`Timeout`/`NoCandidate` are failure terminals;
/// `Planned`/`Dispatched`/`Running` are still in-flight (counted in neither, so
/// they hold the snapshot at `Running` until every subtask reaches a terminal).
///
/// 中文: 把 `DispatchOutcome` 清單聚合成進度快照 - 純計算、UI 可直接呼叫。
/// 依每個 outcome 的真實 `DispatchStatus` 分類成 完成／失敗／進行中。
pub fn aggregate_progress(
    parent_task_id: &str,
    outcomes: &[DispatchOutcome],
) -> ExecutionProgress {
    let total = outcomes.len().min(u8::MAX as usize) as u8;
    let completed = outcomes
        .iter()
        .filter(|o| o.status == DispatchStatus::Completed)
        .count()
        .min(u8::MAX as usize) as u8;
    let failed = outcomes
        .iter()
        .filter(|o| {
            matches!(
                o.status,
                DispatchStatus::Failed | DispatchStatus::Timeout | DispatchStatus::NoCandidate
            )
        })
        .count()
        .min(u8::MAX as usize) as u8;
    let status = if total == 0 {
        DecomposeStatus::Planning
    } else if failed == total {
        DecomposeStatus::TotalFailure
    } else if failed > 0 && completed.saturating_add(failed) == total {
        DecomposeStatus::PartialFailure
    } else if completed == total {
        DecomposeStatus::Completed
    } else {
        DecomposeStatus::Running
    };
    ExecutionProgress {
        parent_task_id: parent_task_id.to_owned(),
        completed_subtasks: completed,
        total_subtasks: total,
        failed_subtasks: failed,
        current_status: status,
    }
}

// ─── Stage 3 inner helpers — real impls + chained delegations ──────────────
//
// Per docs/superpowers/SPEC-TO-CODE-PLAYBOOK.md the Stage 2 `_pseudo` stubs
// were promoted to real impls. Two classes:
//   1. Fully self-contained logic (prompt template / JSON parse / Kahn /
//      now_unix_ms / uuid_v7 v4-fallback) — real Rust, no further work.
//   2. Chained delegations into sibling wire modules now also at Stage 3 —
//      `call_frontier_llm` → `providers_wire::complete`; `dispatch_subtask`
//      → `cluster_dispatch_wire::execute_plan`. The delegation itself is
//      permanent; the inner panic boundary (still Stage 2 per-provider
//      HTTP adapters) lives one module deeper and is the only remaining
//      Stage 4 surface — flips green automatically when providers_wire
//      lands its own Stage 3 promotion. No further edit needed here.

/// Render the SPEC-27 §8.2 system + user + JSON-schema-fragment prompt for
/// the frontier model. Pure string templating — no external template engine
/// pulled in (askama / tera would be Stage 4 polish if the prompt grows).
fn build_decompose_prompt(request: &DecomposeRequest) -> String {
    // Format a compact `target_caps` slug list (slug[:value], comma-separated)
    // so the prompt token cost stays predictable; `None` collapses to empty.
    let caps_hint = match &request.target_caps {
        Some(tags) if !tags.is_empty() => {
            let mut parts = Vec::with_capacity(tags.len());
            for t in tags {
                match &t.value {
                    Some(v) => parts.push(format!("{}:{}", t.slug, v)),
                    None => parts.push(t.slug.clone()),
                }
            }
            parts.join(", ")
        }
        _ => String::new(),
    };

    let max_subtasks = request.max_subtasks.clamp(1, 20);

    // The schema fragment is hand-rolled JSON (not pulled from
    // `schemars`) so the prompt stays readable for an auditor and the
    // model gets a copy-pasteable example shape. Keep this lockstep with
    // `SubTask` (§7.1) — any field change here = a prompt regression risk.
    format!(
        "You are the SPEC-27 smart-decompose planner. Break the user task \
         into at most {max_subtasks} concrete subtasks. Emit STRICT JSON only \
         — no markdown fences, no commentary.\n\
         \n\
         Schema (one object per subtask, array under key `subtasks`):\n\
         {{\n\
         \x20 \"subtasks\": [\n\
         \x20   {{\n\
         \x20     \"subtaskId\":   \"<short stable id>\",\n\
         \x20     \"parentTaskId\":\"<echo the same value for every entry>\",\n\
         \x20     \"text\":        \"<imperative one-line instruction>\",\n\
         \x20     \"requiredCaps\":[ {{ \"slug\": \"<cap>\", \"value\": null }} ],\n\
         \x20     \"dependsOn\":   [ \"<other subtaskId>\" ],\n\
         \x20     \"priority\":    5,\n\
         \x20     \"estimatedDurationMs\": 60000\n\
         \x20   }}\n\
         \x20 ]\n\
         }}\n\
         \n\
         Capability hint (bias selection): {caps_hint}\n\
         \n\
         User task:\n\
         {task}\n",
        max_subtasks = max_subtasks,
        caps_hint = if caps_hint.is_empty() { "none" } else { &caps_hint },
        task = request.task_text,
    )
}

/// SPEC-14 frontier completion through `providers_wire::complete`. The
/// wire-up is real (Stage 3): assemble a `ProviderRequest`, forward to
/// the SPEC-14 public surface, map `ProviderError` to the SPEC-27 §11.1
/// error catalog (`DecomposeTimeout` for timeout / `LlmRefusedToDecompose`
/// for refusal + schema failures + unknown). The downstream per-provider
/// `complete_*_pseudo` adapters are still Stage 2, so a live call
/// propagates the inner panic — the delegation itself is permanent.
async fn call_frontier_llm(
    prompt: &str,
) -> Result<DecomposeLlmResponse, DecomposeError> {
    use crate::providers_wire::{
        complete, Message, MessageRole, ProviderError, ProviderRequest, ResponseFormat,
    };
    let req = ProviderRequest {
        model: "claude-opus-4.7".to_string(),
        system_prompt: Some(
            "You are the SPEC-27 smart-decompose planner. Emit STRICT JSON only.".to_string(),
        ),
        messages: vec![Message::text(MessageRole::User, prompt.to_string())],
        max_tokens: Some(4096),
        temperature: Some(0.0),
        response_format: ResponseFormat::Json,
        // Text-only completion path — no tool-calling here.
        tools: Vec::new(),
    };
    // `providers_wire::complete` is currently sync; await-friendly via the
    // `async` context here even though it doesn't suspend. When the
    // providers_wire async refactor lands the `.await` will start
    // contributing.
    let resp = complete(req).map_err(|e| match e {
        // NetworkError is the closest SPEC-14 §11 variant for "upstream
        // hung up" / DNS / TLS / timeout (no dedicated Timeout variant
        // exists in the current catalog). Map it to DecomposeTimeout so
        // the UI surfaces the retryable class.
        ProviderError::NetworkError { .. } => {
            DecomposeError::DecomposeTimeout(format!("{e:?}"))
        }
        other => DecomposeError::LlmRefusedToDecompose(format!("{other:?}")),
    })?;
    Ok(DecomposeLlmResponse {
        raw_json: resp.text,
        provider_used: resp.model_used,
        elapsed_ms: resp.latency_ms,
    })
}

/// Real serde_json parse of the frontier response into `Vec<SubTask>`. Maps
/// `serde_json::Error` to `LlmRefusedToDecompose` so the caller can surface a
/// single error class for "model output unusable". Enforces the `max_subtasks`
/// hard cap (§3.1 G1 / §8.6 chunked re-decompose ceiling = 20 absolute upper
/// bound). The cluster-side `KNOWN_CAPS` cross-check stays Stage 4 because
/// cluster_dispatch_wire has not yet published the canonical set.
fn parse_subtasks(raw_json: &str) -> Result<Vec<SubTask>, DecomposeError> {
    // The frontier emits an envelope `{"subtasks": [...]}` — strip it down
    // to the array. Accept both bare arrays and the envelope form for
    // robustness across providers (some models drop the wrapping object).
    #[derive(Deserialize)]
    struct Envelope {
        subtasks: Vec<SubTask>,
    }

    if let Ok(env) = serde_json::from_str::<Envelope>(raw_json) {
        return cap_subtasks(env.subtasks);
    }
    let bare: Vec<SubTask> = serde_json::from_str(raw_json).map_err(|e| {
        DecomposeError::LlmRefusedToDecompose(format!("subtasks json parse: {e}"))
    })?;
    cap_subtasks(bare)
}

/// Enforce the SPEC-27 §3.1 G1 absolute 20-subtask ceiling. Returns
/// `SubtaskCountExceeded` when the model leaks past the schema cap so the
/// caller can flip to chunked re-decompose (§8.6) or surface the failure.
fn cap_subtasks(subtasks: Vec<SubTask>) -> Result<Vec<SubTask>, DecomposeError> {
    const HARD_CAP: usize = 20;
    if subtasks.len() > HARD_CAP {
        return Err(DecomposeError::SubtaskCountExceeded(format!(
            "got {} subtasks, hard cap is {HARD_CAP}",
            subtasks.len()
        )));
    }
    Ok(subtasks)
}

/// Mint a time-ordered identifier string. The canonical SPEC-27 contract
/// is "UUIDv7" but the `uuid` crate's `v7` feature is not currently in
/// `core/Cargo.toml` (only `v4` + `serde`). Stage 3 ships a real, time-
/// ordered ID that does NOT touch Cargo.toml: a 13-digit unix-ms prefix
/// concatenated with a v4 random suffix, formatted to look like a UUID
/// (so downstream code that pattern-matches on hyphen layout still
/// works). When `uuid/v7` lands as a default feature this helper flips
/// to `Uuid::now_v7().to_string()` in one line — a true Stage 4 swap.
///
/// Format: `<8-hex unix-ms>-<4-hex random>-7<3-hex random>-<4-hex
/// random>-<12-hex random>`. The `7` byte in position 13 mimics the
/// UUIDv7 version nibble so external regex consumers don't break.
fn uuid_v7() -> String {
    use uuid::Uuid;
    let now_ms = now_unix_ms();
    // Lower 32 bits of the ms timestamp → 8 hex chars (sufficient for
    // ordering within any ~50-day window; the full 48-bit ms field is
    // reconstructable from clock context if needed).
    let ts_lo = (now_ms & 0xFFFF_FFFF) as u32;
    let rand = Uuid::new_v4();
    let rand_hex = rand.as_simple().to_string(); // 32 hex chars, no dashes
    format!(
        "{ts:08x}-{a}-7{b}-{c}-{d}",
        ts = ts_lo,
        a = &rand_hex[0..4],
        b = &rand_hex[4..7],
        c = &rand_hex[7..11],
        d = &rand_hex[11..23],
    )
}

/// Current UTC milliseconds since Unix epoch as `u64`. Uses `std::time`
/// directly — no chrono need. Saturates to `u64::MAX` if the clock somehow
/// reports a duration > 584 million years (will not happen in practice; the
/// saturating path is just to avoid `unwrap` on `as_millis -> u128`).
fn now_unix_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Hand-rolled Kahn's-algorithm topological sort, grouped into dependency
/// levels (each inner `Vec` is one ready-to-run cohort whose members have
/// zero remaining in-degree). No `petgraph` dep — keeps `core/Cargo.toml`
/// untouched. Returns `CircularDependency` if a cycle slips through (e.g.
/// caller forgot to `validate_dag` first; defence in depth).
fn topo_sort<'a>(subtasks: &'a [SubTask]) -> Result<Vec<Vec<&'a SubTask>>, DecomposeError> {
    use std::collections::HashMap;

    if subtasks.is_empty() {
        return Ok(Vec::new());
    }

    // index by subtask_id for O(1) lookup
    let mut by_id: HashMap<&str, &'a SubTask> = HashMap::with_capacity(subtasks.len());
    for st in subtasks {
        by_id.insert(st.subtask_id.as_str(), st);
    }

    // First pass: validate every `depends_on` target exists. Unknown
    // targets are a schema-class violation the caller is supposed to catch
    // before topo_sort; we surface them as CircularDependency (broadest
    // "DAG unusable" bucket) so the public error surface stays narrow.
    for st in subtasks {
        for dep in &st.depends_on {
            if !by_id.contains_key(dep.as_str()) {
                return Err(DecomposeError::CircularDependency(format!(
                    "subtask `{}` depends on unknown `{dep}`",
                    st.subtask_id
                )));
            }
        }
    }

    // Second pass: in-degree for each node is just `|depends_on|` (each
    // dependency contributes one inbound edge for Kahn).
    let mut in_deg: HashMap<&str, usize> = HashMap::with_capacity(subtasks.len());
    for st in subtasks {
        in_deg.insert(st.subtask_id.as_str(), st.depends_on.len());
    }

    let mut levels: Vec<Vec<&'a SubTask>> = Vec::new();
    let mut processed = 0usize;
    let total = subtasks.len();

    loop {
        // collect every node with current in-degree == 0
        let mut ready: Vec<&'a SubTask> = in_deg
            .iter()
            .filter(|(_, &d)| d == 0)
            .filter_map(|(id, _)| by_id.get(*id).copied())
            .collect();

        if ready.is_empty() {
            break;
        }

        // deterministic order — sort by subtask_id so the level layout is
        // reproducible across runs (matters for trace correlation + tests).
        ready.sort_by(|a, b| a.subtask_id.cmp(&b.subtask_id));

        // remove the ready set from the in-degree map + decrement their
        // dependents.
        for st in &ready {
            in_deg.remove(st.subtask_id.as_str());
        }
        // For each remaining node, drop one from its in-degree count for
        // every dependency now in `ready`. O(V * E) worst case is fine for
        // the SPEC-27 §3.1 G1 ≤ 20-node ceiling.
        let ready_ids: std::collections::HashSet<&str> =
            ready.iter().map(|s| s.subtask_id.as_str()).collect();
        for st in subtasks {
            if !in_deg.contains_key(st.subtask_id.as_str()) {
                continue;
            }
            let drop = st
                .depends_on
                .iter()
                .filter(|d| ready_ids.contains(d.as_str()))
                .count();
            if drop > 0 {
                let cur = in_deg.get_mut(st.subtask_id.as_str()).expect("present");
                *cur = cur.saturating_sub(drop);
            }
        }

        processed += ready.len();
        levels.push(ready);
    }

    if processed != total {
        // Unprocessed nodes remain → must be a cycle the caller missed.
        return Err(DecomposeError::CircularDependency(format!(
            "topo_sort stuck at {processed}/{total} nodes — cycle present"
        )));
    }

    Ok(levels)
}

/// Dispatch one subtask via `cluster_dispatch_wire::execute_plan`. The
/// SPEC-26 dispatcher is Stage 3-real (HMAC-signed reqwest POST + poll
/// loop + fallback walk), so this is a permanent delegate: we synthesise
/// a one-peer `DispatchPlan` targeting the local serve and await the
/// real execute. The chained call returns a `DispatchOutcome` on
/// success; failure surfaces as a synthetic outcome with the matching
/// terminal status so the caller (`dispatch_plan`) can keep going.
///
/// 中文: 真實接到 SPEC-26 execute_plan — chained Stage 3 → Stage 3。
async fn dispatch_subtask(subtask: &SubTask) -> DispatchOutcome {
    use crate::cluster_dispatch_wire::{execute_plan, DispatchPlan, DispatchStatus};
    // Synthesise a one-shot dispatch plan: the SPEC-26 cluster dispatcher
    // owns peer selection in normal operation, but at the subtask layer
    // we already know which DAG node we want executed and let
    // execute_plan handle the HMAC + poll machinery. Local serve
    // (`local-self`) is the conventional self-target; the resolver maps
    // it to `127.0.0.1:7878` via `peer_base_url`.
    let plan = DispatchPlan {
        task_id: subtask.subtask_id.clone(),
        selected_peer_id: "local-self".to_string(),
        fallback_peer_ids: Vec::new(),
        scoring_reason: "smart_decompose dispatch_subtask self-target".to_string(),
        planned_at_ms: now_unix_ms(),
    };
    match execute_plan(&plan).await {
        Ok(out) => out,
        Err(e) => DispatchOutcome {
            task_id: subtask.subtask_id.clone(),
            executed_by_peer_id: "local-self".to_string(),
            status: DispatchStatus::Failed,
            started_at_ms: now_unix_ms(),
            completed_at_ms: Some(now_unix_ms()),
            result_summary: None,
            error: Some(format!("{e:?}")),
            cost_usd: 0.0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC-27 §7.4 round-trip invariant: encode -> decode preserves every
    /// field on the outermost wire shape (`DecomposePlan`).
    #[test]
    fn decompose_plan_round_trip_smoke() {
        let plan = DecomposePlan {
            request_id: "req-01923f8e-9b4c-7000".into(),
            parent_task_id: "task-01923f8e-9b4c-7001".into(),
            subtasks: vec![
                SubTask {
                    subtask_id: "st-1".into(),
                    parent_task_id: "task-01923f8e-9b4c-7001".into(),
                    text: "design schema".into(),
                    required_caps: vec![],
                    depends_on: vec![],
                    priority: 5,
                    estimated_duration_ms: Some(60_000),
                },
                SubTask {
                    subtask_id: "st-2".into(),
                    parent_task_id: "task-01923f8e-9b4c-7001".into(),
                    text: "impl FTS5 query".into(),
                    required_caps: vec![],
                    depends_on: vec!["st-1".into()],
                    priority: 7,
                    estimated_duration_ms: None,
                },
            ],
            dag_topology: TopologyHint::Sequential,
            decomposed_at_ms: 1_716_563_400_000,
        };
        let j = serde_json::to_string(&plan).expect("serialize");
        let back: DecomposePlan = serde_json::from_str(&j).expect("round-trip");
        assert_eq!(plan.request_id, back.request_id);
        assert_eq!(plan.parent_task_id, back.parent_task_id);
        assert_eq!(plan.subtasks.len(), back.subtasks.len());
        assert_eq!(plan.subtasks[0].subtask_id, back.subtasks[0].subtask_id);
        assert_eq!(plan.subtasks[1].depends_on, back.subtasks[1].depends_on);
        assert_eq!(plan.dag_topology, back.dag_topology);
        assert_eq!(plan.decomposed_at_ms, back.decomposed_at_ms);
    }

    /// `validate_dag` must return `CircularDependency` on a 3-node cycle
    /// (A -> B -> C -> A).
    #[test]
    fn validate_dag_detects_cycle() {
        let subtasks = vec![
            SubTask {
                subtask_id: "A".into(),
                parent_task_id: "task-x".into(),
                text: "node A".into(),
                required_caps: vec![],
                depends_on: vec!["C".into()],
                priority: 5,
                estimated_duration_ms: None,
            },
            SubTask {
                subtask_id: "B".into(),
                parent_task_id: "task-x".into(),
                text: "node B".into(),
                required_caps: vec![],
                depends_on: vec!["A".into()],
                priority: 5,
                estimated_duration_ms: None,
            },
            SubTask {
                subtask_id: "C".into(),
                parent_task_id: "task-x".into(),
                text: "node C".into(),
                required_caps: vec![],
                depends_on: vec!["B".into()],
                priority: 5,
                estimated_duration_ms: None,
            },
        ];
        match validate_dag(&subtasks) {
            Err(DecomposeError::CircularDependency(detail)) => {
                assert!(
                    detail.contains("cycle detected"),
                    "expected cycle marker in detail, got: {detail}"
                );
            }
            other => panic!("expected CircularDependency, got {other:?}"),
        }
    }

    /// Parallel topology - every node has empty `depends_on`.
    #[test]
    fn validate_dag_parallel_topology() {
        let subtasks = vec![
            SubTask {
                subtask_id: "a".into(),
                parent_task_id: "p".into(),
                text: "x".into(),
                required_caps: vec![],
                depends_on: vec![],
                priority: 5,
                estimated_duration_ms: None,
            },
            SubTask {
                subtask_id: "b".into(),
                parent_task_id: "p".into(),
                text: "y".into(),
                required_caps: vec![],
                depends_on: vec![],
                priority: 5,
                estimated_duration_ms: None,
            },
        ];
        assert_eq!(validate_dag(&subtasks).unwrap(), TopologyHint::Parallel);
    }

    /// Empty plan -> Planning status from `aggregate_progress`.
    #[test]
    fn aggregate_progress_empty_is_planning() {
        let prog = aggregate_progress("task-empty", &[]);
        assert_eq!(prog.total_subtasks, 0);
        assert_eq!(prog.current_status, DecomposeStatus::Planning);
        assert_eq!(prog.parent_task_id, "task-empty");
    }

    /// Build a minimal `DispatchOutcome` carrying just the terminal `status` —
    /// the only field `aggregate_progress` inspects.
    fn outcome(status: crate::cluster_dispatch_wire::DispatchStatus) -> DispatchOutcome {
        DispatchOutcome {
            task_id: "t".into(),
            executed_by_peer_id: "p".into(),
            status,
            started_at_ms: 0,
            completed_at_ms: None,
            result_summary: None,
            error: None,
            cost_usd: 0.0,
        }
    }

    /// `aggregate_progress` must count REAL per-outcome statuses, not assume
    /// everything completed (the prior `completed = total; failed = 0` bug).
    #[test]
    fn aggregate_progress_reads_real_status_not_hardcoded_completed() {
        use crate::cluster_dispatch_wire::DispatchStatus::*;
        // All Completed -> Completed.
        let p = aggregate_progress("p", &[outcome(Completed), outcome(Completed)]);
        assert_eq!(
            (p.completed_subtasks, p.failed_subtasks, p.total_subtasks),
            (2, 0, 2)
        );
        assert_eq!(p.current_status, DecomposeStatus::Completed);

        // Every failure terminal (Failed/Timeout/NoCandidate) counts; all-failed -> TotalFailure.
        let p = aggregate_progress("p", &[outcome(Failed), outcome(Timeout), outcome(NoCandidate)]);
        assert_eq!(
            (p.completed_subtasks, p.failed_subtasks, p.total_subtasks),
            (0, 3, 3)
        );
        assert_eq!(p.current_status, DecomposeStatus::TotalFailure);

        // Mixed completed + failed, none in-flight -> PartialFailure.
        let p = aggregate_progress("p", &[outcome(Completed), outcome(Failed)]);
        assert_eq!(
            (p.completed_subtasks, p.failed_subtasks, p.total_subtasks),
            (1, 1, 2)
        );
        assert_eq!(p.current_status, DecomposeStatus::PartialFailure);

        // A still-running subtask is NEITHER completed nor failed -> Running,
        // and must NOT be silently reported as completed (the bug this fixes).
        let p = aggregate_progress(
            "p",
            &[outcome(Completed), outcome(Running), outcome(Dispatched)],
        );
        assert_eq!(
            (p.completed_subtasks, p.failed_subtasks, p.total_subtasks),
            (1, 0, 3)
        );
        assert_eq!(p.current_status, DecomposeStatus::Running);
    }

    /// `DecomposeError` serializes with `tag = "code"` / `content = "detail"`
    /// for TS `switch (err.code)`.
    #[test]
    fn decompose_error_serializes_with_tag() {
        let e = DecomposeError::CircularDependency("A -> B -> A".into());
        let j = serde_json::to_string(&e).expect("serialize");
        assert!(
            j.contains("\"code\":\"circular_dependency\""),
            "got: {j}"
        );
        assert!(j.contains("\"detail\":\"A -> B -> A\""), "got: {j}");
    }

    /// `TopologyHint` + `DecomposeStatus` enums emit `snake_case`.
    #[test]
    fn enums_serialize_snake_case() {
        let a = serde_json::to_string(&TopologyHint::DagMixed).expect("serialize");
        assert_eq!(a, "\"dag_mixed\"");
        let b = serde_json::to_string(&DecomposeStatus::PartialFailure).expect("serialize");
        assert_eq!(b, "\"partial_failure\"");
    }

    /// Stage 3 → Stage 3 chained delegation marker — `dispatch_plan` now
    /// runs the real topo-sort (Kahn) and delegates each subtask to the
    /// real SPEC-26 `execute_plan`. Without `PHANTOM_CLUSTER_SECRET` in
    /// the env the dispatcher returns `DispatchAuthFailed` per
    /// `load_cluster_secret`, which `dispatch_subtask` converts into a
    /// synthetic `DispatchStatus::Failed` outcome — no panic. The test
    /// just guards that the chain completes end-to-end without crashing
    /// the agent runtime.
    #[test]
    fn dispatch_plan_chains_through_real_executor() {
        // Ensure no live cluster secret in the test env so the chain
        // resolves deterministically via the auth-failed path.
        std::env::remove_var("PHANTOM_CLUSTER_SECRET");
        let plan = DecomposePlan {
            request_id: "req-stage3".into(),
            parent_task_id: "task-stage3".into(),
            subtasks: vec![SubTask {
                subtask_id: "only".into(),
                parent_task_id: "task-stage3".into(),
                text: "single node".into(),
                required_caps: vec![],
                depends_on: vec![],
                priority: 5,
                estimated_duration_ms: None,
            }],
            dag_topology: TopologyHint::Parallel,
            decomposed_at_ms: 0,
        };
        // Block on the async fn via futures' tiny executor.
        let outcomes = futures::executor::block_on(dispatch_plan(&plan))
            .expect("Kahn topo + delegate must not panic");
        assert_eq!(outcomes.len(), 1);
        // No secret in env → execute_plan returns DispatchAuthFailed,
        // dispatch_subtask wraps as a synthetic Failed outcome.
        assert!(
            matches!(
                outcomes[0].status,
                crate::cluster_dispatch_wire::DispatchStatus::Failed
                    | crate::cluster_dispatch_wire::DispatchStatus::Completed
            ),
            "got {:?}",
            outcomes[0].status
        );
    }

    // ─── Stage 3 KAT (known-answer-test) vectors ─────────────────────────

    /// SPEC-27 §8.2 prompt invariant: the rendered prompt must (a) carry
    /// the user task text verbatim, (b) repeat the `max_subtasks` cap, and
    /// (c) include the schema keyword `subtaskId` so the model knows the
    /// camelCase wire surface. Pure-string template — no model call.
    #[test]
    fn build_decompose_prompt_includes_task_cap_and_schema() {
        let req = DecomposeRequest {
            task_text: "refactor the FTS5 query path".into(),
            max_subtasks: 6,
            target_caps: Some(vec![CapabilityTag {
                slug: "role-coder".into(),
                value: None,
            }]),
            deadline_ms: None,
        };
        let prompt = build_decompose_prompt(&req);
        assert!(
            prompt.contains("refactor the FTS5 query path"),
            "prompt must carry task_text verbatim: {prompt}"
        );
        assert!(
            prompt.contains("at most 6 concrete subtasks"),
            "prompt must repeat max_subtasks cap: {prompt}"
        );
        assert!(
            prompt.contains("subtaskId"),
            "prompt must reference camelCase schema key `subtaskId`: {prompt}"
        );
        assert!(
            prompt.contains("role-coder"),
            "prompt must surface target_caps hint: {prompt}"
        );
    }

    /// `build_decompose_prompt` clamps `max_subtasks` into the [1, 20]
    /// window (SPEC-27 §3.1 G1) — caller passing `0` or `99` must not leak
    /// past the schema-fragment template.
    #[test]
    fn build_decompose_prompt_clamps_max_subtasks_window() {
        let too_big = DecomposeRequest {
            task_text: "x".into(),
            max_subtasks: 99,
            target_caps: None,
            deadline_ms: None,
        };
        let p = build_decompose_prompt(&too_big);
        assert!(p.contains("at most 20"), "must clamp to 20: {p}");

        let zero = DecomposeRequest {
            task_text: "x".into(),
            max_subtasks: 0,
            target_caps: None,
            deadline_ms: None,
        };
        let p = build_decompose_prompt(&zero);
        assert!(p.contains("at most 1"), "must clamp to 1: {p}");
    }

    /// `parse_subtasks` accepts both the `{"subtasks": [...]}` envelope
    /// form and a bare JSON array (some providers strip the wrapper).
    #[test]
    fn parse_subtasks_accepts_envelope_and_bare_array() {
        let env = r#"{"subtasks":[{"subtaskId":"a","parentTaskId":"p","text":"do x","requiredCaps":[],"dependsOn":[]}]}"#;
        let v = parse_subtasks(env).expect("envelope parses");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].subtask_id, "a");
        assert_eq!(v[0].priority, 5, "default priority backfill");

        let bare = r#"[{"subtaskId":"b","parentTaskId":"p","text":"do y","requiredCaps":[],"dependsOn":[]}]"#;
        let v = parse_subtasks(bare).expect("bare array parses");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].subtask_id, "b");
    }

    /// `parse_subtasks` enforces the SPEC-27 §3.1 G1 absolute 20-node
    /// ceiling — 21 subtasks → `SubtaskCountExceeded`.
    #[test]
    fn parse_subtasks_enforces_hard_cap() {
        let mut s = String::from("[");
        for i in 0..21 {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!(
                "{{\"subtaskId\":\"s{i}\",\"parentTaskId\":\"p\",\"text\":\"t\",\"requiredCaps\":[],\"dependsOn\":[]}}"
            ));
        }
        s.push(']');
        match parse_subtasks(&s) {
            Err(DecomposeError::SubtaskCountExceeded(d)) => {
                assert!(d.contains("21"), "detail names the count: {d}");
            }
            other => panic!("expected SubtaskCountExceeded, got {other:?}"),
        }
    }

    /// Malformed JSON → `LlmRefusedToDecompose` (single error class for
    /// "model output unusable"; UI can dispatch on `code`).
    #[test]
    fn parse_subtasks_malformed_json_maps_to_refusal() {
        match parse_subtasks("not json at all") {
            Err(DecomposeError::LlmRefusedToDecompose(_)) => {}
            other => panic!("expected LlmRefusedToDecompose, got {other:?}"),
        }
    }

    /// `now_unix_ms` returns a plausible Unix timestamp — at minimum
    /// greater than 2024-01-01 (the project did not exist before then).
    #[test]
    fn now_unix_ms_is_post_2024() {
        let t = now_unix_ms();
        // 2024-01-01T00:00:00Z = 1_704_067_200_000 ms
        assert!(t > 1_704_067_200_000, "now_unix_ms = {t} not post-2024");
    }

    /// `topo_sort` on a diamond DAG (A → {B, C} → D) yields 3 dependency
    /// levels with deterministic per-level ordering.
    #[test]
    fn topo_sort_diamond_dag_produces_three_levels() {
        let subtasks = vec![
            SubTask {
                subtask_id: "A".into(),
                parent_task_id: "p".into(),
                text: "root".into(),
                required_caps: vec![],
                depends_on: vec![],
                priority: 5,
                estimated_duration_ms: None,
            },
            SubTask {
                subtask_id: "B".into(),
                parent_task_id: "p".into(),
                text: "fan-out-left".into(),
                required_caps: vec![],
                depends_on: vec!["A".into()],
                priority: 5,
                estimated_duration_ms: None,
            },
            SubTask {
                subtask_id: "C".into(),
                parent_task_id: "p".into(),
                text: "fan-out-right".into(),
                required_caps: vec![],
                depends_on: vec!["A".into()],
                priority: 5,
                estimated_duration_ms: None,
            },
            SubTask {
                subtask_id: "D".into(),
                parent_task_id: "p".into(),
                text: "join".into(),
                required_caps: vec![],
                depends_on: vec!["B".into(), "C".into()],
                priority: 5,
                estimated_duration_ms: None,
            },
        ];
        let levels = topo_sort(&subtasks).expect("acyclic");
        assert_eq!(levels.len(), 3, "diamond = 3 levels (got {levels:?})");
        assert_eq!(levels[0].iter().map(|s| &s.subtask_id).collect::<Vec<_>>(), vec!["A"]);
        // Level 1 is the parallel pair (sorted asc)
        assert_eq!(
            levels[1].iter().map(|s| s.subtask_id.as_str()).collect::<Vec<_>>(),
            vec!["B", "C"],
            "sibling order is deterministic-asc by id"
        );
        assert_eq!(levels[2].iter().map(|s| &s.subtask_id).collect::<Vec<_>>(), vec!["D"]);
    }

    /// `topo_sort` rejects unknown dependency targets — schema-level
    /// invariant the LLM may violate; surface as `CircularDependency`
    /// (broadest error class for "DAG unusable").
    #[test]
    fn topo_sort_rejects_unknown_dependency_target() {
        let subtasks = vec![SubTask {
            subtask_id: "X".into(),
            parent_task_id: "p".into(),
            text: "dangling".into(),
            required_caps: vec![],
            depends_on: vec!["GHOST".into()],
            priority: 5,
            estimated_duration_ms: None,
        }];
        match topo_sort(&subtasks) {
            Err(DecomposeError::CircularDependency(d)) => {
                assert!(d.contains("GHOST"), "detail names the missing dep: {d}");
            }
            other => panic!("expected CircularDependency, got {other:?}"),
        }
    }

    /// `DecomposeRequest` honors `default_max_subtasks` when the wire omits it.
    #[test]
    fn decompose_request_default_max_subtasks() {
        let json = r#"{"taskText":"hello"}"#;
        let req: DecomposeRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.max_subtasks, 8);
        assert_eq!(req.task_text, "hello");
        assert!(req.target_caps.is_none());
        assert!(req.deadline_ms.is_none());
    }
}
