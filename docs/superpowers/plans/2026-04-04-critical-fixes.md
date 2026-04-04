# Phantom Mesh Critical Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix all P0 security vulnerabilities, SQLite async blocking issues, documentation inconsistencies, and ClusterHub mutex contention identified in the 4-perspective deep analysis.

**Architecture:** Four independent fix tracks executed in parallel: (A) Security — constant-time auth comparison, (B) SQLite — spawn_blocking + WAL + connection reuse, (C) Documentation — align all number claims with reality, (D) ClusterHub — consolidate 7 Mutexes into 2.

**Tech Stack:** Rust (tokio, subtle, rusqlite), Markdown

---

## Track A: Security Fixes

### Task A1: Constant-Time Auth Comparison in main.rs

**Files:**
- Modify: `src/main.rs:170-179` (hub_api_key auth)
- Modify: `src/main.rs:330-340` (cluster secret auth)
- Test: `tests/integration.rs` (add auth timing test)

- [ ] **Step 1: Write the failing test**

Add to `tests/integration.rs` (or a new `tests/auth_security.rs`):

```rust
#[cfg(test)]
mod auth_security_tests {
    use subtle::ConstantTimeEq;

    #[test]
    fn constant_time_eq_matches_identical_tokens() {
        let a = "my-secret-key-12345";
        let b = "my-secret-key-12345";
        let result: bool = a.as_bytes().ct_eq(b.as_bytes()).into();
        assert!(result);
    }

    #[test]
    fn constant_time_eq_rejects_different_tokens() {
        let a = "my-secret-key-12345";
        let b = "wrong-secret-key-99";
        let result: bool = a.as_bytes().ct_eq(b.as_bytes()).into();
        assert!(!result);
    }

    #[test]
    fn constant_time_eq_rejects_different_length() {
        let a = "short";
        let b = "much-longer-token";
        let result: bool = a.as_bytes().ct_eq(b.as_bytes()).into();
        assert!(!result);
    }
}
```

- [ ] **Step 2: Run test to verify it passes** (these test the primitive, not the integration)

Run: `cargo test auth_security_tests -- --nocapture`
Expected: PASS (testing subtle crate usage)

- [ ] **Step 3: Fix hub_api_key comparison (main.rs:176)**

Replace line 176 in `src/main.rs`:
```rust
// BEFORE (timing attack vulnerable):
Some(token) if token == key => Ok(next.run(req).await),

// AFTER (constant-time comparison):
Some(token) if {
    use subtle::ConstantTimeEq;
    token.as_bytes().ct_eq(key.as_bytes()).into()
} => Ok(next.run(req).await),
```

- [ ] **Step 4: Fix cluster secret comparison (main.rs:335)**

Replace line 335 in `src/main.rs`:
```rust
// BEFORE (timing attack vulnerable):
Some(token) if token == secret => Ok(()),

// AFTER (constant-time comparison):
Some(token) if {
    use subtle::ConstantTimeEq;
    token.as_bytes().ct_eq(secret.as_bytes()).into()
} => Ok(()),
```

- [ ] **Step 5: Run full test suite**

Run: `cargo test`
Expected: All existing tests pass

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "fix(security): use constant-time comparison for auth tokens

Replace == with subtle::ConstantTimeEq for hub_api_key and
cluster_secret comparisons to prevent timing side-channel attacks.
The subtle crate was already in Cargo.toml (line 114)."
```

---

### Task A2: Fix circuit_breaker .expect() Panic Risk

**Files:**
- Modify: `src/circuit_breaker.rs` (all `.expect("circuit_breaker lock poisoned")` calls)

- [ ] **Step 1: Search for all .expect() calls in circuit_breaker.rs**

Run: `grep -n '.expect(' src/circuit_breaker.rs`
Expected: Find ~5 occurrences at lines 140, 182, 234, 266, 286

- [ ] **Step 2: Replace all .expect() with poison recovery**

For each occurrence, replace:
```rust
// BEFORE:
let mut states = self.states.lock().expect("circuit_breaker lock poisoned");

// AFTER:
let mut states = self.states.lock().unwrap_or_else(|e| e.into_inner());
```

This recovers from a poisoned Mutex instead of cascading the panic.

- [ ] **Step 3: Run tests**

Run: `cargo test circuit`
Expected: All circuit breaker tests pass

- [ ] **Step 4: Commit**

```bash
git add src/circuit_breaker.rs
git commit -m "fix: recover from poisoned Mutex in circuit_breaker instead of panicking"
```

---

## Track B: SQLite Async Fixes

### Task B1: Add WAL Mode to All SQLite Initializations

**Files:**
- Modify: `src/memory/sqlite.rs` (add WAL pragma)
- Modify: `src/cost_tracker.rs` (add WAL pragma)
- Modify: `src/observational_memory.rs` (add WAL pragma)
- Modify: `src/revenue_tracker.rs` (add WAL pragma)
- Modify: `src/trajectory.rs` (add WAL pragma)
- Modify: `src/task_queue.rs` (add WAL pragma)
- Modify: `src/goals.rs` (add WAL pragma)
- Modify: `src/cron.rs` (add WAL pragma)

Already has WAL: `src/cluster.rs` (line 32), `src/audit_log.rs`

- [ ] **Step 1: Search for all ensure_schema / table creation that lacks WAL**

Run: `grep -rn "CREATE TABLE" src/ --include="*.rs" -l`
Find all files with schema creation. Cross-reference with:
Run: `grep -rn "journal_mode" src/ --include="*.rs"`
Files missing WAL need the pragma added.

- [ ] **Step 2: Add WAL pragma to each file's schema initialization**

For each file that opens a Connection and creates tables, add immediately after `Connection::open()`:
```rust
conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
```

The `busy_timeout=5000` gives SQLite 5 seconds to wait on a locked database instead of failing immediately.

- [ ] **Step 3: Run tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add src/memory/sqlite.rs src/cost_tracker.rs src/observational_memory.rs src/revenue_tracker.rs src/trajectory.rs src/task_queue.rs src/goals.rs src/cron.rs
git commit -m "fix(sqlite): enable WAL mode and busy_timeout on all database connections"
```

---

### Task B2: Wrap Sync CostTracker in spawn_blocking

**Files:**
- Modify: `src/cost_tracker.rs`

The CostTracker is called from async agent_runtime but uses sync SQLite operations. Since it's used as `Arc<CostTracker>` from AppState, we need to wrap its methods.

- [ ] **Step 1: Identify all public methods that call Connection::open()**

In `src/cost_tracker.rs`, these are: `record()` (line 206), `check_budget()` (line 254), `today_total()`, `recent()`, `by_provider()`, `by_agent()`, `summary()`, `date_range()`

- [ ] **Step 2: Create async wrapper methods**

For each sync method, create an async version. Example for `record`:

```rust
/// Async wrapper for record — safe to call from tokio context
pub async fn record_async(&self, record: CostRecord) -> Result<()> {
    let db_path = self.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
        // ... existing INSERT logic from record()
        Ok(())
    })
    .await?
}
```

Alternatively, if changing the call pattern is too invasive, keep sync methods but document that callers must use `spawn_blocking`:

```rust
// In agent_runtime.rs where cost_tracker is called:
let ct = cost_tracker.clone();
let record = record.clone();
tokio::task::spawn_blocking(move || ct.record(&record)).await??;
```

- [ ] **Step 3: Update callers in agent_runtime.rs**

Search for `cost_tracker.record(` and `cost_tracker.check_budget(` in `src/agent_runtime.rs` and wrap with `spawn_blocking`.

- [ ] **Step 4: Run tests**

Run: `cargo test cost_tracker`
Expected: All cost tracker tests pass

- [ ] **Step 5: Commit**

```bash
git add src/cost_tracker.rs src/agent_runtime.rs
git commit -m "fix(async): wrap CostTracker SQLite calls in spawn_blocking"
```

---

### Task B3: Wrap ObservationalMemory in spawn_blocking

**Files:**
- Modify: `src/observational_memory.rs`

- [ ] **Step 1: Identify sync methods calling Connection::open()**

`observe()` (line 113), `recall()` (line 195), `recall_recent()`, `count()`, `search_by_tags()`, `delete_by_session()`

- [ ] **Step 2: Add async wrappers using spawn_blocking**

Same pattern as Task B2. For `recall()`:
```rust
pub async fn recall_async(&self, query: &str, limit: usize) -> Result<Vec<Observation>> {
    let db_path = self.db_path.clone();
    let query = query.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
        // ... existing recall logic
    })
    .await?
}
```

- [ ] **Step 3: Update callers**

Search `observational_memory.recall(` and `observational_memory.observe(` across the codebase, update to async versions.

- [ ] **Step 4: Run tests**

Run: `cargo test observational`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/observational_memory.rs
git commit -m "fix(async): wrap ObservationalMemory SQLite calls in spawn_blocking"
```

---

### Task B4: Fix Remaining async fn new() Constructors

**Files (15 files with blocking Connection::open in async constructors):**
- `src/task_queue.rs:99`
- `src/distributed_state.rs:95`
- `src/node_scoring.rs:128`
- `src/power_economics.rs:84`
- `src/provider_pricing.rs:49`
- `src/task_preemption.rs:71`
- `src/auto_diagnosis.rs:347`
- `src/multi_tenant.rs:52`
- `src/order_workflow.rs:155`
- `src/quality_pipeline.rs:55`
- `src/service_tier.rs:205`
- `src/optimizer_store.rs:131`
- `src/trajectory.rs:103`

- [ ] **Step 1: For each file, wrap the constructor's Connection::open in spawn_blocking**

Pattern for each `async fn new()`:
```rust
// BEFORE:
pub async fn new(db_path: &str) -> Result<Self> {
    let conn = Connection::open(db_path)?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS ...")?;
    Ok(Self { conn: Mutex::new(conn) })
}

// AFTER:
pub async fn new(db_path: &str) -> Result<Self> {
    let path = db_path.to_string();
    let conn = tokio::task::spawn_blocking(move || -> Result<Connection> {
        let conn = Connection::open(&path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS ...")?;
        Ok(conn)
    }).await??;
    Ok(Self { conn: Mutex::new(conn) })
}
```

- [ ] **Step 2: Apply to all 13 files**

Each file follows the same pattern. The `Connection::open` + schema creation moves into `spawn_blocking`.

- [ ] **Step 3: Run full test suite**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add src/task_queue.rs src/distributed_state.rs src/node_scoring.rs src/power_economics.rs src/provider_pricing.rs src/task_preemption.rs src/auto_diagnosis.rs src/multi_tenant.rs src/order_workflow.rs src/quality_pipeline.rs src/service_tier.rs src/optimizer_store.rs src/trajectory.rs
git commit -m "fix(async): wrap all SQLite constructors in spawn_blocking

15 async fn new() constructors were calling Connection::open()
synchronously, blocking the tokio runtime. Now wrapped in
spawn_blocking with WAL mode and busy_timeout=5000ms."
```

---

## Track C: Documentation Truthfulness

### Task C1: Fix README.md Number Claims

**Files:**
- Modify: `README.md`

**Current contradictions in README.md:**
- Line 3: "53 tools, 48 hands, 11 providers"
- Line 15: "10 LLM providers...29 multi-phase workflow automations"
- Line 22: "48 Hands"
- Line 45 diagram: "(29 workflows)" and "(10 providers)"

**Ground truth (from code analysis):**
- Tools: 59 tool files in src/tools/ (excluding mod.rs and support files)
- Hands: 0 shipped as TOML, engine supports loading from ~/.phantom-mesh/hands/
- Providers: ~11 trait-based implementations (via OpenAI-Compat for LM Studio/etc)
- Iroh QUIC: stub (iroh_transport.rs line 1: "stub implementation")

- [ ] **Step 1: Update README.md header**

```markdown
<!-- BEFORE -->
53 tools, 48 hands, 11 providers, 4-node cluster + mobile workers

<!-- AFTER -->
59 tools, 11 providers, 4-node cluster + mobile workers, TOML-defined workflow engine
```

- [ ] **Step 2: Fix body text (line 15)**

```markdown
<!-- BEFORE -->
10 LLM providers, executes 53 built-in tools, runs 29 multi-phase workflow automations called **Hands**

<!-- AFTER -->
11 LLM providers (including OpenAI-compatible), executes 59 built-in tools, and a multi-phase workflow engine called **Hands** (user-defined via TOML)
```

- [ ] **Step 3: Fix features section (lines 21-24)**

```markdown
<!-- BEFORE -->
- **53 Tools**
- **48 Hands**

<!-- AFTER -->
- **59 Tools** — file I/O, web, shell, AI, media, communication, data, enterprise
- **Hands Engine** — TOML-defined multi-phase workflows with guardrails, caching, and ROI scheduling
```

- [ ] **Step 4: Fix architecture diagram (line 45)**

Replace "(29 workflows)" with "(TOML workflow engine)" and "(10 providers)" with "(11 providers)".

- [ ] **Step 5: Remove or qualify Iroh QUIC claims**

If README mentions Iroh QUIC as a feature, add "(planned)" or remove it.

- [ ] **Step 6: Commit**

```bash
git add README.md
git commit -m "docs: fix inflated number claims in README

- Tools: 53 → 59 (actual count from src/tools/)
- Hands: remove '48 hands' claim — engine exists but 0 hand.toml shipped
- Providers: standardize on '11' across all mentions
- Iroh QUIC: mark as planned/stub"
```

---

### Task C2: Fix CLAUDE.md and Cargo.toml

**Files:**
- Modify: `CLAUDE.md`
- Modify: `Cargo.toml` (description field, line 5)

- [ ] **Step 1: Update CLAUDE.md line 4**

```markdown
<!-- BEFORE -->
53 tools, 48 hands, 11 LLM providers, 4-node cluster + mobile workers

<!-- AFTER -->
59 tools, 11 LLM providers, 4-node cluster + mobile workers, TOML workflow engine (Hands)
```

- [ ] **Step 2: Fix CLAUDE.md architecture section (lines 20-21)**

Update the number counts to match actual file counts.

- [ ] **Step 3: Update Cargo.toml description (line 5)**

```toml
# BEFORE:
description = "Phantom Mesh — distributed AI agent daemon with 53 tools, 48 hands, 11 providers, cluster orchestration, and self-evolution"

# AFTER:
description = "Phantom Mesh — distributed AI agent daemon with 59 tools, 11 providers, TOML workflow engine, cluster orchestration, and self-evolution"
```

- [ ] **Step 4: Commit**

```bash
git add CLAUDE.md Cargo.toml
git commit -m "docs: align CLAUDE.md and Cargo.toml with actual feature counts"
```

---

### Task C3: Create 5 Example Hand TOML Files

**Files:**
- Create: `examples/hands/content/hand.toml`
- Create: `examples/hands/researcher/hand.toml`
- Create: `examples/hands/lead/hand.toml`
- Create: `examples/hands/seo_content/hand.toml`
- Create: `examples/hands/auto_report/hand.toml`

- [ ] **Step 1: Create examples/hands/ directory**

```bash
mkdir -p examples/hands/{content,researcher,lead,seo_content,auto_report}
```

- [ ] **Step 2: Create content hand.toml**

```toml
name = "content"
description = "Generate blog or social media content on a given topic"
category = "content"
provider = "auto"
model = ""
output_format = "markdown"

[[phases]]
name = "research"
system_prompt = "You are a content researcher. Search the web for current information about the given topic. Collect key facts, statistics, and recent developments."
max_rounds = 5
tools = ["web_search", "content_search"]

[[phases]]
name = "draft"
system_prompt = "You are a professional content writer. Using the research provided, write a comprehensive, engaging article. Include an introduction, 3-5 key sections, and a conclusion."
max_rounds = 3
condition = "min_length:200"

[[phases]]
name = "polish"
system_prompt = "You are an editor. Review the draft for grammar, flow, and engagement. Make improvements and ensure the content is publication-ready."
max_rounds = 2
condition = "min_length:500"
```

- [ ] **Step 3: Create researcher hand.toml**

```toml
name = "researcher"
description = "Deep research on a topic with parallel web searches"
category = "research"
provider = "auto"
model = ""
output_format = "markdown"

[[phases]]
name = "gather"
system_prompt = "You are a research analyst. Conduct comprehensive web searches on the given topic. Focus on finding primary sources, recent data, and expert opinions."
max_rounds = 5
tools = ["web_search", "http_request", "content_search"]
parallel_queries = ["latest developments", "expert analysis", "statistics and data"]

[[phases]]
name = "analyze"
system_prompt = "You are a senior analyst. Synthesize the gathered research into a structured analysis. Identify patterns, contradictions, and key insights. Cite sources."
max_rounds = 3
condition = "min_length:500"
tools = ["data_analysis"]
```

- [ ] **Step 4: Create lead, seo_content, auto_report hand.toml files** (similar pattern)

- [ ] **Step 5: Add README note about example hands**

Add to README.md under Hands section:
```markdown
### Getting Started with Hands

Copy example hand definitions to your config:
\`\`\`bash
cp -r examples/hands/* ~/.phantom-mesh/hands/
\`\`\`
```

- [ ] **Step 6: Commit**

```bash
git add examples/hands/ README.md
git commit -m "feat: add 5 example hand.toml workflow definitions

Provides working examples for: content, researcher, lead, seo_content, auto_report.
Users can copy these to ~/.phantom-mesh/hands/ to get started."
```

---

## Track D: ClusterHub Mutex Consolidation

### Task D1: Consolidate 7 Mutexes into 2

**Files:**
- Modify: `src/cluster_hub.rs`

**Current state (7 Mutexes):**
```
pending_tasks: Mutex<HashMap<String, VecDeque<PendingTask>>>
inflight_results: Mutex<HashMap<String, oneshot::Sender<Value>>>
inflight_counts: Mutex<HashMap<String, u32>>
shared_mobile_pool: Mutex<VecDeque<PendingTask>>
pending_agent_tasks: Mutex<HashMap<String, VecDeque<PendingAgentTask>>>
shared_agent_pool: Mutex<VecDeque<PendingAgentTask>>
dispatch_log: Mutex<HashMap<String, Instant>>
```

**Target (2 Mutexes):**
```
task_state: Mutex<TaskState>       // all task queues + inflight tracking
dispatch_log: Mutex<HashMap<String, Instant>>  // idempotency (separate lifecycle)
```

- [ ] **Step 1: Define consolidated TaskState struct**

```rust
/// Consolidated task state — single lock for all task routing data.
struct TaskState {
    /// Per-worker queue of pending tasks (for polling/mobile workers)
    pending_tasks: HashMap<String, VecDeque<PendingTask>>,
    /// Map of task_id → oneshot sender, for results that arrive via POST /cluster/result
    inflight_results: HashMap<String, tokio::sync::oneshot::Sender<Value>>,
    /// Number of in-flight tasks per worker (for load-aware routing)
    inflight_counts: HashMap<String, u32>,
    /// Shared task pool for mobile workers (any mobile worker can pick up)
    shared_mobile_pool: VecDeque<PendingTask>,
    /// Per-worker queue of agent tasks
    pending_agent_tasks: HashMap<String, VecDeque<PendingAgentTask>>,
    /// Shared agent task pool for mobile workers
    shared_agent_pool: VecDeque<PendingAgentTask>,
}
```

- [ ] **Step 2: Update ClusterHub struct**

```rust
pub struct ClusterHub {
    pub registry: Arc<ClusterRegistry>,
    pub metrics: Arc<ClusterMetrics>,
    http_client: reqwest::Client,
    task_state: Mutex<TaskState>,
    dispatch_log: Mutex<HashMap<String, Instant>>,
    taxonomy: Option<Arc<TaskTaxonomy>>,
    concurrency: Option<Arc<ConcurrencyManager>>,
}
```

- [ ] **Step 3: Update effective_load to use batch read**

```rust
// BEFORE (locks per worker):
async fn effective_load(&self, worker: &ClusterNode) -> f32 {
    let counts = self.inflight_counts.lock().await;
    let inflight = counts.get(&worker.name).copied().unwrap_or(0);
    worker.cpu_load + (inflight as f32 * 0.15)
}

// AFTER (batch snapshot):
async fn effective_loads(&self, workers: &[ClusterNode]) -> Vec<f32> {
    let state = self.task_state.lock().await;
    workers.iter().map(|w| {
        let inflight = state.inflight_counts.get(&w.name).copied().unwrap_or(0);
        w.cpu_load + (inflight as f32 * 0.15)
    }).collect()
}
```

- [ ] **Step 4: Update all methods that previously locked individual Mutexes**

Key methods to update:
- `dispatch_tool_ext()` — replace separate lock acquisitions with single `task_state.lock()`
- `dispatch_to_mobile()` — use `task_state` instead of `pending_tasks` + `inflight_results`
- `poll_task()` — replace 4 separate locks with single `task_state.lock()`
- `submit_result()` — use `task_state` for inflight_results lookup
- `inc_inflight()` / `dec_inflight()` — use `task_state.inflight_counts`

- [ ] **Step 5: Run all cluster tests**

Run: `cargo test cluster`
Expected: All cluster tests pass

- [ ] **Step 6: Run integration tests**

Run: `cargo test --test integration`
Expected: All integration tests pass

- [ ] **Step 7: Commit**

```bash
git add src/cluster_hub.rs
git commit -m "refactor(cluster): consolidate 7 Mutexes into 2

Merge pending_tasks, inflight_results, inflight_counts,
shared_mobile_pool, pending_agent_tasks, shared_agent_pool
into a single TaskState struct behind one Mutex.

This eliminates:
- Dead lock risk from acquiring 4 locks in poll_task()
- Lock contention from N lock/unlock cycles in effective_load()
- Potential ordering issues across independent locks

dispatch_log kept separate (different lifecycle/TTL cleanup)."
```

---

## Post-Fix Verification Protocol

After all 4 tracks complete, run the following verification:

### V1: Full Test Suite
```bash
cd D:/Projects/adreanalai/LLM-Cluster-Project/clawtex-core
cargo test 2>&1 | tail -5
```
Expected: All 3914+ tests pass

### V2: Security Audit (grep for remaining issues)
```bash
# No more == comparisons for auth tokens:
grep -n 'token == key\|token == secret' src/main.rs
# Expected: 0 matches

# No more .expect() in circuit_breaker:
grep -n '\.expect(' src/circuit_breaker.rs
# Expected: 0 matches
```

### V3: SQLite Async Audit
```bash
# All Connection::open in async context should be in spawn_blocking:
grep -rn 'Connection::open' src/ --include="*.rs" | grep -v test | grep -v '#\[cfg(test)\]'
# Manual review: each must be in spawn_blocking or sync-only context
```

### V4: Documentation Consistency
```bash
# All docs should say same numbers:
grep -rn '53 tools\|48 hands' src/ README.md CLAUDE.md Cargo.toml
# Expected: 0 matches (all updated)
```

---

## Review Protocol

After fixes are applied, 4 parallel review agents run:

1. **Code Reviewer** — standard code review against this plan
2. **OpenCode perspective** — code quality, anti-patterns, remaining smells
3. **Codex perspective** — scalability, did fixes introduce new bottlenecks?
4. **Gemini perspective** — completeness, did fixes match what was promised?
