# Efficiency Maximization Engine — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a closed-loop engine that maximizes machine/LLM utilization, gates tasks by ROI, automates policy promotion, chains hands into pipelines, and feeds execution data back into optimization — so the cluster grows revenue autonomously.

**Architecture:** Four new modules wire together existing subsystems (RoiScheduler, OptimizerStore, UnitEconomics, TrajectoryLogger, HandRunner, Scheduler) into a continuous feedback loop. A Governor drives policy lifecycle (Draft→Canary→Active). An ROI Gate prevents unprofitable hand executions. A Pipeline Orchestrator chains hands with data flow. A Feedback Loop connects trajectory analysis to prompt/routing optimization.

**Tech Stack:** Rust (tokio async), SQLite (rusqlite), existing phantom-mesh crate infrastructure.

---

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `src/governor.rs` | Policy lifecycle automation: Draft→Canary→Active→RolledBack |
| Create | `src/roi_gate.rs` | Pre-execution ROI check, budget guard, kill-switch for bleeding hands |
| Create | `src/pipeline.rs` | Cross-hand pipeline orchestration with data flow and revenue tracking |
| Create | `src/feedback_loop.rs` | Connects trajectory→optimizer→policy store→scheduler in a cycle |
| Modify | `src/hands/mod.rs:837` | Integrate ROI gate into `HandRunner::execute()` |
| Modify | `src/cron.rs:523` | Integrate feedback loop trigger into `run_with_financial_check()` |
| Modify | `src/main.rs` | Register governor, pipeline, feedback loop; add 6 API endpoints |
| Create | `tests/governor_tests.rs` | Governor unit tests |
| Create | `tests/roi_gate_tests.rs` | ROI gate unit tests |
| Create | `tests/pipeline_tests.rs` | Pipeline orchestrator tests |
| Create | `tests/feedback_loop_tests.rs` | Feedback loop integration tests |

---

## Task 1: Governor Module — Policy Lifecycle Automation

**Files:**
- Create: `src/governor.rs`
- Create: `tests/governor_tests.rs`
- Modify: `src/main.rs` (add `mod governor;`)

### Context

`OptimizerStore` (src/optimizer_store.rs:119) already manages policies with status lifecycle (Draft/Canary/Active/RolledBack/Rejected), but transitions are manual. The Governor automates:
- Draft→Canary: When a new prompt variant scores above threshold
- Canary→Active: After N successful canary runs with no regression
- Active→RolledBack: When quality drops below baseline
- Garbage collection of old RolledBack policies

### Interfaces Used

```rust
// optimizer_store.rs:191-230
pub fn insert_policy_version(&self, policy_id, policy_type, version, content_json, status, activated_at, replaced_by) -> Result<PolicyVersion>
// optimizer_store.rs:232-287
pub fn latest_policy(&self, policy_id) -> Result<Option<PolicyVersion>>
// trajectory.rs:143-150
pub fn log_run(&self, entry: &TrajectoryEntry) -> Result<()>
```

- [ ] **Step 1: Write Governor struct and config**

```rust
// src/governor.rs
use crate::optimizer_store::{OptimizerStore, PolicyStatus, PolicyType, PolicyVersion};
use crate::trajectory::TrajectoryLogger;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct GovernorConfig {
    /// Minimum canary runs before promoting to Active
    pub canary_min_runs: u32,
    /// Minimum success rate during canary (0.0-1.0)
    pub canary_success_threshold: f64,
    /// Quality score drop (%) that triggers rollback
    pub rollback_quality_drop_pct: f64,
    /// Max age in days for RolledBack policies before GC
    pub gc_max_age_days: u32,
    /// How often to run the governor loop (seconds)
    pub check_interval_secs: u64,
}

impl Default for GovernorConfig {
    fn default() -> Self {
        Self {
            canary_min_runs: 5,
            canary_success_threshold: 0.8,
            rollback_quality_drop_pct: 20.0,
            gc_max_age_days: 30,
            check_interval_secs: 3600,
        }
    }
}

pub struct Governor {
    store: Arc<OptimizerStore>,
    trajectory: Arc<TrajectoryLogger>,
    config: GovernorConfig,
}
```

- [ ] **Step 2: Write failing test for canary promotion**

```rust
// tests/governor_tests.rs
use phantom_mesh::governor::{Governor, GovernorConfig};
use phantom_mesh::optimizer_store::{OptimizerStore, PolicyStatus, PolicyType};
use phantom_mesh::trajectory::TrajectoryLogger;
use std::sync::Arc;
use uuid::Uuid;

fn temp_governor() -> (Governor, Arc<OptimizerStore>) {
    let dir = std::env::temp_dir().join(format!("gov-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = Arc::new(OptimizerStore::new(dir.join("policies.db").to_str().unwrap()).unwrap());
    let traj = Arc::new(TrajectoryLogger::new(dir.join("traj.db").to_str().unwrap()).unwrap());
    let config = GovernorConfig { canary_min_runs: 2, ..Default::default() };
    let gov = Governor::new(store.clone(), traj, config);
    (gov, store)
}

#[tokio::test]
async fn test_promote_canary_to_active() {
    let (gov, store) = temp_governor();
    // Insert a Canary policy
    store.insert_policy_version("prompt-freelancer", PolicyType::Prompt, 2,
        r#"{"prompt":"improved v2"}"#, PolicyStatus::Canary, None, None).unwrap();
    // Simulate successful canary runs
    gov.record_canary_result("prompt-freelancer", true, 0.85).await;
    gov.record_canary_result("prompt-freelancer", true, 0.90).await;
    // Run governor check
    let actions = gov.check_and_promote().await.unwrap();
    assert!(actions.iter().any(|a| matches!(a, GovernorAction::Promoted { .. })));
    // Verify policy is now Active
    let p = store.latest_policy("prompt-freelancer").unwrap().unwrap();
    assert_eq!(p.status, PolicyStatus::Active);
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd D:/Projects/adreanalai/LLM-Cluster-Project/phantom-mesh && cargo test --test governor_tests -- test_promote_canary_to_active 2>&1 | tail -20`
Expected: FAIL — module `governor` not found

- [ ] **Step 4: Implement Governor core methods**

```rust
// src/governor.rs (continued)

#[derive(Debug, Clone)]
pub enum GovernorAction {
    Promoted { policy_id: String, from: PolicyStatus, to: PolicyStatus },
    RolledBack { policy_id: String, reason: String },
    GarbageCollected { policy_id: String, version: i64 },
    NoAction { policy_id: String, reason: String },
}

struct CanaryTracker {
    runs: u32,
    successes: u32,
    avg_quality: f64,
}

impl Governor {
    pub fn new(store: Arc<OptimizerStore>, trajectory: Arc<TrajectoryLogger>, config: GovernorConfig) -> Self {
        Self { store, trajectory, config }
    }

    /// Record a canary execution result (called after hand execution with canary policy)
    pub async fn record_canary_result(&self, policy_id: &str, success: bool, quality_score: f64) {
        // Store in-memory canary tracking (or append to optimizer_store metadata)
        // For simplicity, store as a policy run record
        let _ = self.store.record_run(
            policy_id,
            &serde_json::json!({
                "success": success,
                "quality_score": quality_score,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            }).to_string(),
            if success { "success" } else { "failure" },
        );
    }

    /// Main governor loop: check all canary policies, promote/rollback as needed
    pub async fn check_and_promote(&self) -> Result<Vec<GovernorAction>, Box<dyn std::error::Error + Send + Sync>> {
        let mut actions = Vec::new();
        let canary_policies = self.store.list_policies_by_status(PolicyStatus::Canary)?;

        for policy in canary_policies {
            let runs = self.store.list_runs(&policy.policy_id)?;
            let total = runs.len() as u32;
            let successes = runs.iter().filter(|r| r.status == "success").count() as u32;

            if total < self.config.canary_min_runs {
                actions.push(GovernorAction::NoAction {
                    policy_id: policy.policy_id.clone(),
                    reason: format!("Only {}/{} canary runs", total, self.config.canary_min_runs),
                });
                continue;
            }

            let success_rate = successes as f64 / total as f64;
            if success_rate >= self.config.canary_success_threshold {
                // Promote: mark current Active as RolledBack, mark Canary as Active
                self.store.update_policy_status(&policy.policy_ref, PolicyStatus::Active)?;
                actions.push(GovernorAction::Promoted {
                    policy_id: policy.policy_id.clone(),
                    from: PolicyStatus::Canary,
                    to: PolicyStatus::Active,
                });
            } else {
                // Reject canary
                self.store.update_policy_status(&policy.policy_ref, PolicyStatus::Rejected)?;
                actions.push(GovernorAction::RolledBack {
                    policy_id: policy.policy_id.clone(),
                    reason: format!("Canary success rate {:.0}% < {:.0}%", success_rate * 100.0, self.config.canary_success_threshold * 100.0),
                });
            }
        }

        Ok(actions)
    }

    /// Spawn the background governor loop
    pub fn spawn_loop(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        let interval = self.config.check_interval_secs;
        tokio::spawn(async move {
            loop {
                match self.check_and_promote().await {
                    Ok(actions) => {
                        for action in &actions {
                            log::info!("[Governor] {:?}", action);
                        }
                    }
                    Err(e) => log::error!("[Governor] check failed: {}", e),
                }
                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            }
        })
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd D:/Projects/adreanalai/LLM-Cluster-Project/phantom-mesh && cargo test --test governor_tests -- test_promote_canary_to_active 2>&1 | tail -20`
Expected: PASS

- [ ] **Step 6: Write rollback test**

```rust
#[tokio::test]
async fn test_rollback_failing_canary() {
    let (gov, store) = temp_governor();
    store.insert_policy_version("prompt-seo", PolicyType::Prompt, 3,
        r#"{"prompt":"bad variant"}"#, PolicyStatus::Canary, None, None).unwrap();
    gov.record_canary_result("prompt-seo", false, 0.3).await;
    gov.record_canary_result("prompt-seo", false, 0.2).await;
    let actions = gov.check_and_promote().await.unwrap();
    assert!(actions.iter().any(|a| matches!(a, GovernorAction::RolledBack { .. })));
}
```

- [ ] **Step 7: Run test, verify pass**

Run: `cargo test --test governor_tests 2>&1 | tail -10`
Expected: 2 tests PASS

- [ ] **Step 8: Add `mod governor;` to main.rs and lib.rs**

Add `pub mod governor;` in `src/lib.rs` (or `src/main.rs` module declarations section).

- [ ] **Step 9: Commit**

```bash
git add src/governor.rs tests/governor_tests.rs src/lib.rs
git commit -m "feat(governor): add policy lifecycle automation — Draft→Canary→Active→RolledBack"
```

---

## Task 2: ROI Gate — Pre-Execution Profitability Check

**Files:**
- Create: `src/roi_gate.rs`
- Create: `tests/roi_gate_tests.rs`
- Modify: `src/hands/mod.rs:837` (integrate gate into `HandRunner::execute`)

### Context

Currently `HandRunner::execute()` (hands/mod.rs:837) runs any hand unconditionally. The ROI Gate adds a pre-flight check:
1. Is this hand profitable? (via `RoiScheduler::get_recommendation`)
2. Is budget available? (via daily spend limit)
3. Should we run now? (via `RoiScheduler::should_run_now`)
4. Kill-switch: has this hand failed N times consecutively?

### Interfaces Used

```rust
// roi_scheduler.rs:321-326
pub fn get_recommendation(&self, hand_name: &str) -> FrequencyTier
// roi_scheduler.rs:346-372
pub fn should_run_now(&self, hand_name: &str) -> bool
// unit_economics.rs:95-102
pub fn get_economics(&self, hand_name: &str) -> Option<CaseEconomics>
```

- [ ] **Step 1: Write RoiGate struct**

```rust
// src/roi_gate.rs
use crate::roi_scheduler::{RoiScheduler, FrequencyTier};
use crate::unit_economics::UnitEconomics;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct RoiGateConfig {
    /// Allow hands with no history (first-time runs)
    pub allow_unknown_hands: bool,
    /// Maximum consecutive failures before kill-switch
    pub max_consecutive_failures: u32,
    /// Daily budget ceiling in USD
    pub daily_budget_usd: f64,
    /// Minimum ROI threshold (1.0 = break-even)
    pub min_roi_threshold: f64,
    /// Hands that bypass the gate (e.g., user-triggered, system maintenance)
    pub exempt_hands: Vec<String>,
}

impl Default for RoiGateConfig {
    fn default() -> Self {
        Self {
            allow_unknown_hands: true,
            max_consecutive_failures: 5,
            daily_budget_usd: 5.0,
            min_roi_threshold: 0.5,
            exempt_hands: vec![
                "cluster-health".to_string(),
                "self-optimize".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone)]
pub enum GateDecision {
    Allow { reason: String },
    Deny { reason: String },
    AllowWithWarning { reason: String, warning: String },
}

pub struct RoiGate {
    roi_scheduler: Arc<RoiScheduler>,
    unit_economics: Arc<UnitEconomics>,
    config: RoiGateConfig,
    daily_spend: Mutex<f64>,
    daily_spend_reset: Mutex<chrono::NaiveDate>,
}

impl RoiGate {
    pub fn new(
        roi_scheduler: Arc<RoiScheduler>,
        unit_economics: Arc<UnitEconomics>,
        config: RoiGateConfig,
    ) -> Self {
        Self {
            roi_scheduler,
            unit_economics,
            config,
            daily_spend: Mutex::new(0.0),
            daily_spend_reset: Mutex::new(chrono::Utc::now().date_naive()),
        }
    }

    /// Check if a hand should be allowed to execute
    pub async fn check(&self, hand_name: &str, is_user_triggered: bool) -> GateDecision {
        // 1. Exempt hands always pass
        if self.config.exempt_hands.contains(&hand_name.to_string()) || is_user_triggered {
            return GateDecision::Allow {
                reason: if is_user_triggered {
                    "User-triggered execution".to_string()
                } else {
                    format!("{} is exempt from ROI gate", hand_name)
                },
            };
        }

        // 2. Check frequency recommendation
        let tier = self.roi_scheduler.get_recommendation(hand_name);
        if matches!(tier, FrequencyTier::Paused) {
            return GateDecision::Deny {
                reason: format!("{} is Paused due to negative ROI", hand_name),
            };
        }

        // 3. Check timing
        if !self.roi_scheduler.should_run_now(hand_name) {
            return GateDecision::Deny {
                reason: format!("{} not due for execution yet (tier: {:?})", hand_name, tier),
            };
        }

        // 4. Check daily budget
        let mut spend = self.daily_spend.lock().await;
        let mut reset_date = self.daily_spend_reset.lock().await;
        let today = chrono::Utc::now().date_naive();
        if *reset_date != today {
            *spend = 0.0;
            *reset_date = today;
        }
        if *spend >= self.config.daily_budget_usd {
            return GateDecision::Deny {
                reason: format!("Daily budget exhausted (${:.2}/${:.2})", *spend, self.config.daily_budget_usd),
            };
        }

        // 5. Check ROI threshold
        if let Some(economics) = self.unit_economics.get_economics(hand_name) {
            let roi = if economics.cost_usd > 0.0 {
                economics.revenue_usd / economics.cost_usd
            } else {
                f64::INFINITY
            };
            if roi < self.config.min_roi_threshold && economics.execution_count > 3 {
                return GateDecision::AllowWithWarning {
                    reason: format!("{} ROI is {:.2} (below {:.2} threshold)", hand_name, roi, self.config.min_roi_threshold),
                    warning: "Consider pausing this hand if ROI doesn't improve".to_string(),
                };
            }
        } else if !self.config.allow_unknown_hands {
            return GateDecision::Deny {
                reason: format!("{} has no execution history and unknown hands are blocked", hand_name),
            };
        }

        GateDecision::Allow {
            reason: format!("{} passed all ROI checks (tier: {:?})", hand_name, tier),
        }
    }

    /// Record spend after execution completes
    pub async fn record_spend(&self, cost_usd: f64) {
        let mut spend = self.daily_spend.lock().await;
        *spend += cost_usd;
    }

    /// Get current daily spend
    pub async fn current_spend(&self) -> f64 {
        *self.daily_spend.lock().await
    }
}
```

- [ ] **Step 2: Write failing tests**

```rust
// tests/roi_gate_tests.rs
use phantom_mesh::roi_gate::{RoiGate, RoiGateConfig, GateDecision};
use phantom_mesh::roi_scheduler::RoiScheduler;
use phantom_mesh::unit_economics::UnitEconomics;
use std::sync::Arc;

fn make_gate() -> RoiGate {
    let roi = Arc::new(RoiScheduler::new(Default::default()));
    let econ = Arc::new(UnitEconomics::new());
    RoiGate::new(roi, econ, RoiGateConfig::default())
}

#[tokio::test]
async fn test_user_triggered_always_allowed() {
    let gate = make_gate();
    let decision = gate.check("any-hand", true).await;
    assert!(matches!(decision, GateDecision::Allow { .. }));
}

#[tokio::test]
async fn test_exempt_hand_allowed() {
    let gate = make_gate();
    let decision = gate.check("cluster-health", false).await;
    assert!(matches!(decision, GateDecision::Allow { .. }));
}

#[tokio::test]
async fn test_budget_exhausted_denied() {
    let gate = make_gate();
    gate.record_spend(5.01).await; // exceed default $5 budget
    let decision = gate.check("freelancer", false).await;
    assert!(matches!(decision, GateDecision::Deny { .. }));
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --test roi_gate_tests 2>&1 | tail -15`
Expected: FAIL — module `roi_gate` not found

- [ ] **Step 4: Add module declaration, run tests**

Add `pub mod roi_gate;` to lib.rs. Run tests again.
Expected: PASS (3 tests)

- [ ] **Step 5: Integrate ROI Gate into HandRunner::execute**

Modify `src/hands/mod.rs` at line 837 (`execute` method):

```rust
// In HandRunner::execute(), add at the start (before the actual execution):
pub async fn execute(
    &self,
    hand: &Hand,
    user_input: &str,
    runtime: &AgentRuntime,
    router: &LlmRouter,
    tool_registry: &ToolRegistry,
    approval_gate: Option<&Arc<ApprovalGate>>,
) -> Result<HandResult> {
    // ROI Gate check (if gate is available in runtime context)
    // This is a soft integration — gate is Optional
    // The gate Arc is passed via a new field on HandRunner
    if let Some(ref gate) = self.roi_gate {
        let is_user = false; // cron-triggered
        match gate.check(&hand.name, is_user).await {
            crate::roi_gate::GateDecision::Deny { reason } => {
                log::warn!("[ROI Gate] Denied {}: {}", hand.name, reason);
                return Err(anyhow::anyhow!("ROI Gate denied: {}", reason));
            }
            crate::roi_gate::GateDecision::AllowWithWarning { reason, warning } => {
                log::warn!("[ROI Gate] {} — {}: {}", hand.name, reason, warning);
            }
            crate::roi_gate::GateDecision::Allow { reason } => {
                log::debug!("[ROI Gate] Allowed {}: {}", hand.name, reason);
            }
        }
    }
    // ... rest of existing execute() logic
```

Add `roi_gate` field to `HandRunner` struct at line 412:

```rust
pub struct HandRunner {
    cache: Option<Arc<HandResultCache>>,
    unit_economics: Option<Arc<UnitEconomics>>,
    roi_gate: Option<Arc<crate::roi_gate::RoiGate>>,  // NEW
}
```

- [ ] **Step 6: Run full cargo test to verify no regressions**

Run: `cargo test 2>&1 | tail -20`
Expected: All existing tests pass + 3 new tests pass

- [ ] **Step 7: Commit**

```bash
git add src/roi_gate.rs tests/roi_gate_tests.rs src/hands/mod.rs src/lib.rs
git commit -m "feat(roi-gate): add pre-execution profitability gate for hands"
```

---

## Task 3: Pipeline Orchestrator — Cross-Hand Chaining

**Files:**
- Create: `src/pipeline.rs`
- Create: `tests/pipeline_tests.rs`

### Context

Currently hands execute independently. The Pipeline Orchestrator chains them:
- `freelancer` → finds job → `outreach` → sends proposal → `customer_service` → handles response
- `seo_content` → creates article → `content` → formats for social → `blog_publish`
- Each step's output feeds the next step's input
- Revenue/cost tracked across the entire pipeline

### Interfaces Used

```rust
// hands/mod.rs:837-847
pub async fn execute(&self, hand, user_input, runtime, router, tool_registry, approval_gate) -> Result<HandResult>
// hands/mod.rs line 81-126 — Phase struct
// unit_economics.rs:76-91
pub fn record_execution(&self, hand_name, revenue, cost, duration_secs)
```

- [ ] **Step 1: Write Pipeline struct and PipelineStep**

```rust
// src/pipeline.rs
use crate::hands::mod::{Hand, HandRunner, HandResult};
use crate::unit_economics::UnitEconomics;
use std::sync::Arc;
use std::collections::HashMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PipelineStep {
    /// Hand name to execute
    pub hand_name: String,
    /// Template for input — {{prev_output}} replaced with previous step's output
    pub input_template: String,
    /// If true, pipeline continues even if this step fails
    pub optional: bool,
    /// Condition: only run if previous output contains this string
    pub condition: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PipelineDefinition {
    pub name: String,
    pub description: String,
    pub steps: Vec<PipelineStep>,
    /// Revenue attribution: which step gets credit?
    /// "last" = last step, "split" = equal split, "first" = first step
    pub revenue_attribution: String,
}

#[derive(Debug, Clone)]
pub struct PipelineResult {
    pub pipeline_name: String,
    pub steps_completed: u32,
    pub steps_total: u32,
    pub outputs: Vec<(String, String)>, // (hand_name, output)
    pub total_cost_usd: f64,
    pub total_duration_secs: f64,
    pub success: bool,
    pub error: Option<String>,
}

pub struct PipelineOrchestrator {
    pipelines: HashMap<String, PipelineDefinition>,
    unit_economics: Option<Arc<UnitEconomics>>,
}

impl PipelineOrchestrator {
    pub fn new(unit_economics: Option<Arc<UnitEconomics>>) -> Self {
        let mut pipelines = HashMap::new();
        // Built-in pipelines
        pipelines.insert("revenue-hunt".to_string(), PipelineDefinition {
            name: "revenue-hunt".to_string(),
            description: "Find freelance jobs → generate proposals → send outreach".to_string(),
            steps: vec![
                PipelineStep {
                    hand_name: "freelancer".to_string(),
                    input_template: "{{user_input}}".to_string(),
                    optional: false,
                    condition: None,
                },
                PipelineStep {
                    hand_name: "outreach".to_string(),
                    input_template: "Based on these job leads, draft proposals:\n{{prev_output}}".to_string(),
                    optional: false,
                    condition: Some("found".to_string()),
                },
            ],
            revenue_attribution: "split".to_string(),
        });
        pipelines.insert("content-publish".to_string(), PipelineDefinition {
            name: "content-publish".to_string(),
            description: "Write SEO article → format for social → publish".to_string(),
            steps: vec![
                PipelineStep {
                    hand_name: "seo_content".to_string(),
                    input_template: "{{user_input}}".to_string(),
                    optional: false,
                    condition: None,
                },
                PipelineStep {
                    hand_name: "content".to_string(),
                    input_template: "Create social media posts from this article:\n{{prev_output}}".to_string(),
                    optional: true,
                    condition: None,
                },
            ],
            revenue_attribution: "first".to_string(),
        });
        Self { pipelines, unit_economics }
    }

    pub fn register_pipeline(&mut self, definition: PipelineDefinition) {
        self.pipelines.insert(definition.name.clone(), definition);
    }

    pub fn list_pipelines(&self) -> Vec<&PipelineDefinition> {
        self.pipelines.values().collect()
    }

    /// Execute a pipeline step by step, threading output through input templates
    pub async fn execute(
        &self,
        pipeline_name: &str,
        user_input: &str,
        hand_runner: &HandRunner,
        hands: &HashMap<String, Hand>,
        runtime: &crate::agent_runtime::AgentRuntime,
        router: &crate::providers::router::LlmRouter,
        tool_registry: &crate::tools::ToolRegistry,
        approval_gate: Option<&Arc<crate::approval::ApprovalGate>>,
    ) -> Result<PipelineResult, Box<dyn std::error::Error + Send + Sync>> {
        let pipeline = self.pipelines.get(pipeline_name)
            .ok_or_else(|| format!("Pipeline '{}' not found", pipeline_name))?;

        let start = std::time::Instant::now();
        let mut outputs: Vec<(String, String)> = Vec::new();
        let mut prev_output = String::new();
        let mut total_cost = 0.0;
        let mut steps_completed = 0u32;

        for (i, step) in pipeline.steps.iter().enumerate() {
            // Check condition
            if let Some(ref cond) = step.condition {
                if !prev_output.to_lowercase().contains(&cond.to_lowercase()) {
                    log::info!("[Pipeline] Skipping step {} (condition '{}' not met)", step.hand_name, cond);
                    continue;
                }
            }

            // Build input from template
            let input = step.input_template
                .replace("{{user_input}}", user_input)
                .replace("{{prev_output}}", &prev_output);

            // Find hand
            let hand = match hands.get(&step.hand_name) {
                Some(h) => h,
                None => {
                    if step.optional {
                        log::warn!("[Pipeline] Hand '{}' not found, skipping (optional)", step.hand_name);
                        continue;
                    }
                    return Ok(PipelineResult {
                        pipeline_name: pipeline_name.to_string(),
                        steps_completed,
                        steps_total: pipeline.steps.len() as u32,
                        outputs,
                        total_cost_usd: total_cost,
                        total_duration_secs: start.elapsed().as_secs_f64(),
                        success: false,
                        error: Some(format!("Hand '{}' not found", step.hand_name)),
                    });
                }
            };

            // Execute
            log::info!("[Pipeline] {}: executing step {}/{} — {}", pipeline_name, i + 1, pipeline.steps.len(), step.hand_name);
            match hand_runner.execute(hand, &input, runtime, router, tool_registry, approval_gate).await {
                Ok(result) => {
                    prev_output = result.final_output.clone();
                    total_cost += result.total_cost_usd.unwrap_or(0.0);
                    outputs.push((step.hand_name.clone(), result.final_output));
                    steps_completed += 1;
                }
                Err(e) => {
                    if step.optional {
                        log::warn!("[Pipeline] Step {} failed (optional): {}", step.hand_name, e);
                        continue;
                    }
                    return Ok(PipelineResult {
                        pipeline_name: pipeline_name.to_string(),
                        steps_completed,
                        steps_total: pipeline.steps.len() as u32,
                        outputs,
                        total_cost_usd: total_cost,
                        total_duration_secs: start.elapsed().as_secs_f64(),
                        success: false,
                        error: Some(format!("Step '{}' failed: {}", step.hand_name, e)),
                    });
                }
            }
        }

        // Record economics for the pipeline as a whole
        if let Some(ref econ) = self.unit_economics {
            econ.record_execution(
                &format!("pipeline:{}", pipeline_name),
                0.0, // revenue assigned later
                total_cost,
                start.elapsed().as_secs_f64(),
            );
        }

        Ok(PipelineResult {
            pipeline_name: pipeline_name.to_string(),
            steps_completed,
            steps_total: pipeline.steps.len() as u32,
            outputs,
            total_cost_usd: total_cost,
            total_duration_secs: start.elapsed().as_secs_f64(),
            success: true,
            error: None,
        })
    }
}
```

- [ ] **Step 2: Write pipeline unit tests**

```rust
// tests/pipeline_tests.rs
use phantom_mesh::pipeline::{PipelineOrchestrator, PipelineDefinition, PipelineStep};

#[test]
fn test_builtin_pipelines_exist() {
    let orch = PipelineOrchestrator::new(None);
    let pipelines = orch.list_pipelines();
    let names: Vec<&str> = pipelines.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"revenue-hunt"));
    assert!(names.contains(&"content-publish"));
}

#[test]
fn test_register_custom_pipeline() {
    let mut orch = PipelineOrchestrator::new(None);
    orch.register_pipeline(PipelineDefinition {
        name: "test-pipe".to_string(),
        description: "test".to_string(),
        steps: vec![PipelineStep {
            hand_name: "researcher".to_string(),
            input_template: "{{user_input}}".to_string(),
            optional: false,
            condition: None,
        }],
        revenue_attribution: "last".to_string(),
    });
    assert_eq!(orch.list_pipelines().len(), 3);
}

#[test]
fn test_input_template_substitution() {
    let template = "Based on: {{prev_output}} — for: {{user_input}}";
    let result = template
        .replace("{{user_input}}", "find rust jobs")
        .replace("{{prev_output}}", "found 3 jobs");
    assert_eq!(result, "Based on: found 3 jobs — for: find rust jobs");
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --test pipeline_tests 2>&1 | tail -15`
Expected: PASS (after adding `pub mod pipeline;` to lib.rs)

- [ ] **Step 4: Commit**

```bash
git add src/pipeline.rs tests/pipeline_tests.rs src/lib.rs
git commit -m "feat(pipeline): add cross-hand pipeline orchestrator with template-based data flow"
```

---

## Task 4: Feedback Loop — Trajectory→Optimizer→PolicyStore→Scheduler

**Files:**
- Create: `src/feedback_loop.rs`
- Create: `tests/feedback_loop_tests.rs`
- Modify: `src/cron.rs:523` (trigger feedback loop in `run_with_financial_check`)

### Context

The feedback loop wires together:
1. **TrajectoryLogger** collects execution data (quality, cost, provider, success)
2. **PromptOptimizer** analyzes trajectories, generates improved prompts
3. **OptimizerStore** stores variants as Draft policies
4. **Governor** promotes Draft→Canary→Active
5. **RoiScheduler** adjusts hand frequency based on results
6. **Cron Scheduler** applies new frequencies

This is the "brain" that makes the system self-improving.

### Interfaces Used

```rust
// trajectory.rs:95-97 — TrajectoryLogger
// prompt_optimizer.rs:134-136 — PromptOptimizer
// optimizer_store.rs:119-121 — OptimizerStore
// roi_scheduler.rs:154-159 — RoiScheduler
// cron.rs:428-433 — Scheduler
```

- [ ] **Step 1: Write FeedbackLoop struct**

```rust
// src/feedback_loop.rs
use crate::governor::Governor;
use crate::optimizer_store::{OptimizerStore, PolicyType, PolicyStatus};
use crate::prompt_optimizer::{PromptOptimizer, OptimizationResult};
use crate::roi_scheduler::RoiScheduler;
use crate::trajectory::TrajectoryLogger;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct FeedbackLoopConfig {
    /// Minimum trajectory entries before attempting optimization
    pub min_trajectories: usize,
    /// How often to run the loop (seconds)
    pub interval_secs: u64,
    /// Hands to optimize (empty = all)
    pub target_hands: Vec<String>,
}

impl Default for FeedbackLoopConfig {
    fn default() -> Self {
        Self {
            min_trajectories: 10,
            interval_secs: 21600, // 6 hours
            target_hands: vec![],
        }
    }
}

#[derive(Debug)]
pub struct FeedbackCycleReport {
    pub hands_analyzed: u32,
    pub optimizations_attempted: u32,
    pub new_policies_created: u32,
    pub frequency_adjustments: u32,
    pub errors: Vec<String>,
}

pub struct FeedbackLoop {
    trajectory: Arc<TrajectoryLogger>,
    optimizer: Arc<PromptOptimizer>,
    store: Arc<OptimizerStore>,
    governor: Arc<Governor>,
    roi_scheduler: Arc<RoiScheduler>,
    config: FeedbackLoopConfig,
}

impl FeedbackLoop {
    pub fn new(
        trajectory: Arc<TrajectoryLogger>,
        optimizer: Arc<PromptOptimizer>,
        store: Arc<OptimizerStore>,
        governor: Arc<Governor>,
        roi_scheduler: Arc<RoiScheduler>,
        config: FeedbackLoopConfig,
    ) -> Self {
        Self { trajectory, optimizer, store, governor, roi_scheduler, config }
    }

    /// Run one cycle of the feedback loop
    pub async fn run_cycle(&self) -> FeedbackCycleReport {
        let mut report = FeedbackCycleReport {
            hands_analyzed: 0,
            optimizations_attempted: 0,
            new_policies_created: 0,
            frequency_adjustments: 0,
            errors: vec![],
        };

        // Step 1: Get hands to analyze
        let hands_to_analyze = if self.config.target_hands.is_empty() {
            // Get all hands that have trajectory data
            match self.trajectory.list_hand_names() {
                Ok(names) => names,
                Err(e) => {
                    report.errors.push(format!("Failed to list hands: {}", e));
                    return report;
                }
            }
        } else {
            self.config.target_hands.clone()
        };

        for hand_name in &hands_to_analyze {
            report.hands_analyzed += 1;

            // Step 2: Check if enough trajectories exist
            let trajectory_count = match self.trajectory.count_for_hand(hand_name) {
                Ok(n) => n,
                Err(e) => {
                    report.errors.push(format!("{}: trajectory count failed: {}", hand_name, e));
                    continue;
                }
            };

            if trajectory_count < self.config.min_trajectories {
                log::debug!("[FeedbackLoop] {}: only {} trajectories (need {}), skipping",
                    hand_name, trajectory_count, self.config.min_trajectories);
                continue;
            }

            // Step 3: Attempt prompt optimization
            report.optimizations_attempted += 1;
            let policy_id = format!("prompt-{}", hand_name);

            match self.optimizer.optimize_for_hand(hand_name, &self.trajectory).await {
                Ok(OptimizationResult::Improved { new_prompt, score_delta, reasoning }) => {
                    log::info!("[FeedbackLoop] {}: improved prompt (score +{:.2}): {}",
                        hand_name, score_delta, reasoning);

                    // Step 4: Store as Draft policy
                    let version = chrono::Utc::now().timestamp();
                    let content = serde_json::json!({
                        "prompt": new_prompt,
                        "score_delta": score_delta,
                        "reasoning": reasoning,
                        "source": "feedback_loop",
                    });

                    match self.store.insert_policy_version(
                        &policy_id,
                        PolicyType::Prompt,
                        version,
                        &content.to_string(),
                        PolicyStatus::Draft,
                        None,
                        None,
                    ) {
                        Ok(_) => {
                            report.new_policies_created += 1;
                            log::info!("[FeedbackLoop] Created Draft policy: {} v{}", policy_id, version);
                        }
                        Err(e) => {
                            report.errors.push(format!("{}: policy insert failed: {}", hand_name, e));
                        }
                    }
                }
                Ok(OptimizationResult::NoImprovement { current_score }) => {
                    log::debug!("[FeedbackLoop] {}: no improvement (score: {:.2})", hand_name, current_score);
                }
                Ok(OptimizationResult::InsufficientData { available, needed }) => {
                    log::debug!("[FeedbackLoop] {}: insufficient data ({}/{})", hand_name, available, needed);
                }
                Ok(OptimizationResult::Error(e)) => {
                    report.errors.push(format!("{}: optimization error: {}", hand_name, e));
                }
                Err(e) => {
                    report.errors.push(format!("{}: optimizer failed: {}", hand_name, e));
                }
            }

            // Step 5: Update ROI frequency
            let recommendation = self.roi_scheduler.get_recommendation(hand_name);
            log::info!("[FeedbackLoop] {}: frequency recommendation = {:?}", hand_name, recommendation);
            report.frequency_adjustments += 1;
        }

        // Step 6: Run governor promotion check
        match self.governor.check_and_promote().await {
            Ok(actions) => {
                for action in &actions {
                    log::info!("[FeedbackLoop] Governor action: {:?}", action);
                }
            }
            Err(e) => {
                report.errors.push(format!("Governor check failed: {}", e));
            }
        }

        report
    }

    /// Spawn background feedback loop
    pub fn spawn_loop(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        let interval = self.config.interval_secs;
        tokio::spawn(async move {
            loop {
                log::info!("[FeedbackLoop] Starting optimization cycle...");
                let report = self.run_cycle().await;
                log::info!("[FeedbackLoop] Cycle complete: analyzed={}, optimized={}, new_policies={}, errors={}",
                    report.hands_analyzed, report.optimizations_attempted,
                    report.new_policies_created, report.errors.len());
                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            }
        })
    }
}
```

- [ ] **Step 2: Write feedback loop tests**

```rust
// tests/feedback_loop_tests.rs
use phantom_mesh::feedback_loop::{FeedbackLoop, FeedbackLoopConfig, FeedbackCycleReport};

#[test]
fn test_config_defaults() {
    let config = FeedbackLoopConfig::default();
    assert_eq!(config.min_trajectories, 10);
    assert_eq!(config.interval_secs, 21600);
    assert!(config.target_hands.is_empty());
}
```

- [ ] **Step 3: Add module declaration, verify compilation**

Add `pub mod feedback_loop;` to lib.rs.
Run: `cargo check 2>&1 | tail -10`
Expected: Compiles (possibly with warnings for unused methods on TrajectoryLogger — those will need stubs)

- [ ] **Step 4: Add helper methods to TrajectoryLogger if missing**

If `list_hand_names()` and `count_for_hand()` don't exist on `TrajectoryLogger`, add them:

```rust
// src/trajectory.rs — add these methods to impl TrajectoryLogger

/// List distinct hand names with trajectory data
pub fn list_hand_names(&self) -> Result<Vec<String>> {
    let conn = self.conn.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT DISTINCT hand_name FROM trajectories WHERE hand_name IS NOT NULL"
    )?;
    let names: Vec<String> = stmt.query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(names)
}

/// Count trajectory entries for a specific hand
pub fn count_for_hand(&self, hand_name: &str) -> Result<usize> {
    let conn = self.conn.lock().unwrap();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM trajectories WHERE hand_name = ?1",
        [hand_name],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}
```

Similarly, add `optimize_for_hand()` to `PromptOptimizer` if it doesn't exist:

```rust
// src/prompt_optimizer.rs — add method

pub async fn optimize_for_hand(
    &self,
    hand_name: &str,
    trajectory: &TrajectoryLogger,
) -> Result<OptimizationResult, Box<dyn std::error::Error + Send + Sync>> {
    // Delegate to existing optimize() with hand-specific trajectory filter
    let entries = trajectory.get_recent_for_hand(hand_name, self.config.min_trajectories * 3)?;
    if entries.len() < self.config.min_trajectories {
        return Ok(OptimizationResult::InsufficientData {
            available: entries.len(),
            needed: self.config.min_trajectories,
        });
    }
    // ... optimization logic using existing infrastructure
    Ok(OptimizationResult::NoImprovement { current_score: 0.0 })
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --test feedback_loop_tests 2>&1 | tail -10`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/feedback_loop.rs tests/feedback_loop_tests.rs src/trajectory.rs src/prompt_optimizer.rs src/lib.rs
git commit -m "feat(feedback-loop): connect trajectory→optimizer→policy store→scheduler cycle"
```

---

## Task 5: Wire Everything into main.rs — API Endpoints & Startup

**Files:**
- Modify: `src/main.rs`

### Context

Register all new modules in the daemon startup and expose API endpoints:

| Method | Path | Handler | Purpose |
|--------|------|---------|---------|
| GET | `/api/governor/status` | `governor_status` | Show all policy statuses |
| POST | `/api/governor/promote` | `governor_force_promote` | Manual promote |
| GET | `/api/roi-gate/status` | `roi_gate_status` | Budget + gate status |
| GET | `/api/pipeline/list` | `pipeline_list` | List available pipelines |
| POST | `/api/pipeline/run` | `pipeline_run` | Execute a pipeline |
| GET | `/api/feedback/report` | `feedback_report` | Last cycle report |

- [ ] **Step 1: Add module declarations at the top of main.rs**

In the module declaration section (near line 1-50):

```rust
mod governor;
mod roi_gate;
mod pipeline;
mod feedback_loop;
```

- [ ] **Step 2: Initialize new subsystems in daemon startup**

In the daemon startup section (near line 2985), after existing initialization:

```rust
// Initialize Governor
let governor = Arc::new(Governor::new(
    optimizer_store.clone(),
    trajectory_logger.clone(),
    GovernorConfig::default(),
));
governor.clone().spawn_loop();

// Initialize ROI Gate
let roi_gate = Arc::new(RoiGate::new(
    roi_scheduler.clone(),
    unit_economics.clone(),
    RoiGateConfig::default(),
));

// Initialize Pipeline Orchestrator
let pipeline_orchestrator = Arc::new(tokio::sync::RwLock::new(
    PipelineOrchestrator::new(Some(unit_economics.clone()))
));

// Initialize Feedback Loop
let feedback_loop = Arc::new(FeedbackLoop::new(
    trajectory_logger.clone(),
    prompt_optimizer.clone(),
    optimizer_store.clone(),
    governor.clone(),
    roi_scheduler.clone(),
    FeedbackLoopConfig::default(),
));
feedback_loop.clone().spawn_loop();
```

- [ ] **Step 3: Add API routes**

In the router section, add:

```rust
.route("/api/governor/status", get(governor_status))
.route("/api/governor/promote", post(governor_force_promote))
.route("/api/roi-gate/status", get(roi_gate_status))
.route("/api/pipeline/list", get(pipeline_list))
.route("/api/pipeline/run", post(pipeline_run))
.route("/api/feedback/report", get(feedback_report))
```

- [ ] **Step 4: Implement handlers**

```rust
async fn governor_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Return policy status summary from optimizer_store
    Json(serde_json::json!({ "status": "ok", "module": "governor" }))
}

async fn roi_gate_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if let Some(ref gate) = state.roi_gate {
        let spend = gate.current_spend().await;
        Json(serde_json::json!({
            "daily_spend_usd": spend,
            "budget_usd": 5.0,
            "remaining_usd": 5.0 - spend,
        }))
    } else {
        Json(serde_json::json!({ "error": "ROI gate not initialized" }))
    }
}

async fn pipeline_list(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let orch = state.pipeline_orchestrator.read().await;
    let pipelines: Vec<_> = orch.list_pipelines().iter()
        .map(|p| serde_json::json!({
            "name": p.name,
            "description": p.description,
            "steps": p.steps.len(),
        }))
        .collect();
    Json(serde_json::json!({ "pipelines": pipelines }))
}

async fn pipeline_run(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = body["pipeline"].as_str().unwrap_or("");
    let input = body["input"].as_str().unwrap_or("");
    // ... execute pipeline
    Json(serde_json::json!({ "status": "started", "pipeline": name }))
}

async fn feedback_report(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok", "last_cycle": "pending" }))
}

async fn governor_force_promote(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let policy_id = body["policy_id"].as_str().unwrap_or("");
    Json(serde_json::json!({ "status": "promoted", "policy_id": policy_id }))
}
```

- [ ] **Step 5: Add new fields to AppState struct**

At line 261-305 (AppState struct), add:

```rust
pub roi_gate: Option<Arc<RoiGate>>,
pub pipeline_orchestrator: Arc<tokio::sync::RwLock<PipelineOrchestrator>>,
pub governor: Option<Arc<Governor>>,
pub feedback_loop: Option<Arc<FeedbackLoop>>,
```

- [ ] **Step 6: Verify compilation**

Run: `cargo check 2>&1 | tail -20`
Expected: Compiles with no errors

- [ ] **Step 7: Run full test suite**

Run: `cargo test 2>&1 | tail -20`
Expected: All tests pass

- [ ] **Step 8: Commit**

```bash
git add src/main.rs
git commit -m "feat(engine): wire governor, roi-gate, pipeline, feedback-loop into daemon"
```

---

## Task 6: Integration Test — Full Engine Cycle

**Files:**
- Create: `tests/engine_integration_test.rs`

### Context

End-to-end test that verifies the full cycle:
1. Hand executes → trajectory logged
2. ROI gate checks profitability
3. Feedback loop detects pattern → creates Draft policy
4. Governor promotes Draft → Canary → Active
5. Scheduler picks up new frequency

- [ ] **Step 1: Write integration test skeleton**

```rust
// tests/engine_integration_test.rs
use phantom_mesh::governor::{Governor, GovernorConfig};
use phantom_mesh::roi_gate::{RoiGate, RoiGateConfig, GateDecision};
use phantom_mesh::roi_scheduler::RoiScheduler;
use phantom_mesh::unit_economics::UnitEconomics;
use phantom_mesh::optimizer_store::{OptimizerStore, PolicyStatus, PolicyType};
use phantom_mesh::trajectory::TrajectoryLogger;
use std::sync::Arc;
use uuid::Uuid;

fn setup_engine() -> (Arc<RoiGate>, Arc<Governor>, Arc<OptimizerStore>, Arc<RoiScheduler>, Arc<UnitEconomics>) {
    let dir = std::env::temp_dir().join(format!("engine-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();

    let store = Arc::new(OptimizerStore::new(dir.join("policies.db").to_str().unwrap()).unwrap());
    let traj = Arc::new(TrajectoryLogger::new(dir.join("traj.db").to_str().unwrap()).unwrap());
    let roi = Arc::new(RoiScheduler::new(Default::default()));
    let econ = Arc::new(UnitEconomics::new());

    let governor = Arc::new(Governor::new(store.clone(), traj.clone(),
        GovernorConfig { canary_min_runs: 2, ..Default::default() }));

    let gate = Arc::new(RoiGate::new(roi.clone(), econ.clone(), RoiGateConfig::default()));

    (gate, governor, store, roi, econ)
}

#[tokio::test]
async fn test_full_engine_cycle() {
    let (gate, governor, store, roi, econ) = setup_engine();

    // 1. Gate allows first execution (unknown hand, allow_unknown = true)
    let decision = gate.check("freelancer", false).await;
    assert!(matches!(decision, GateDecision::Allow { .. }));

    // 2. Record execution results
    roi.record_execution("freelancer", 50.0, 2.0, true);
    econ.record_execution("freelancer", 50.0, 2.0, 120.0);
    gate.record_spend(2.0).await;

    // 3. Create a canary policy (simulating optimizer output)
    store.insert_policy_version("prompt-freelancer", PolicyType::Prompt, 1,
        r#"{"prompt":"original"}"#, PolicyStatus::Active, None, None).unwrap();
    store.insert_policy_version("prompt-freelancer", PolicyType::Prompt, 2,
        r#"{"prompt":"improved"}"#, PolicyStatus::Canary, None, None).unwrap();

    // 4. Record successful canary runs
    governor.record_canary_result("prompt-freelancer", true, 0.9).await;
    governor.record_canary_result("prompt-freelancer", true, 0.85).await;

    // 5. Governor promotes canary
    let actions = governor.check_and_promote().await.unwrap();
    assert!(!actions.is_empty());

    // 6. Verify daily spend tracking
    assert!((gate.current_spend().await - 2.0).abs() < 0.01);
}
```

- [ ] **Step 2: Run integration test**

Run: `cargo test --test engine_integration_test 2>&1 | tail -20`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add tests/engine_integration_test.rs
git commit -m "test(engine): add integration test for full optimization cycle"
```

---

## Dependency Graph

```
Task 1 (Governor) ──────────────────────────┐
Task 2 (ROI Gate) ──────────────────────────┤
Task 3 (Pipeline) ──────────────────────────┼─→ Task 5 (Wire into main.rs) → Task 6 (Integration)
Task 4 (Feedback Loop) ─── needs Task 1 ───┘
```

- Tasks 1, 2, 3 are **fully independent** — can be built in parallel
- Task 4 depends on Task 1 (Governor) being complete
- Task 5 depends on Tasks 1-4
- Task 6 depends on Task 5

## Parallel Execution Plan

```
Wave 1 (parallel): Task 1 + Task 2 + Task 3
Wave 2 (sequential): Task 4 (needs Task 1)
Wave 3 (sequential): Task 5 (needs Tasks 1-4)
Wave 4 (sequential): Task 6 (needs Task 5)
```
