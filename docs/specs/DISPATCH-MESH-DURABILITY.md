# DISPATCH-MESH-DURABILITY — closing the cross-machine dispatch gaps

> STAGE-1 design spec. **No code in this document — design only.** Every file:line
> citation below was read from the canonical tree on branch
> `dev/dispatch-durable-store-spec` and is real at the time of writing.
> Supervisor: cross-check each citation before implementing.
>
> Scope: three concrete gaps in cross-machine async dispatch —
> (a) **durable async job store**, (b) **peer-to-peer (worker→worker) handoff**,
> (c) **failure-recovery routing (auto re-dispatch)**.
> Anchors: `../_archive/MASTER-SPEC.md` §3.4 rows `do-things-mesh`, `CROSS-DISPATCH`,
> `COORDINATOR-FAILOVER`, `SQLITE-TASK-STORE`; `../design/SWARM-ARCHITECTURE.md` §6
> ("狀態模型 / 失敗語意") and roadmap row #5.

---

## 0. Current state — what actually exists (read first)

### 0.1 The async job path (in-memory today)

`POST /rpc/task/assign` → background `tokio::spawn` → poll via
`GET /rpc/task/status/:id`. The job state lives **only in process memory**:

- `ClusterJob` struct (`status`/`output`/`error`) — `core/src/serve.rs:103-108`.
- `type ClusterJobStore = Arc<RwLock<HashMap<String, ClusterJob>>>` —
  `core/src/serve.rs:110`.
- The map is **constructed fresh on every `router()` build** —
  `core/src/serve.rs:124` (`Arc::new(RwLock::new(HashMap::new()))`). It is held
  only as an axum `Extension(jobs)` layer (`core/src/serve.rs:129`); nothing
  persists it.
- Writes: job marked `running` at `core/src/serve.rs:2175`; `done` at
  `core/src/serve.rs:2188`; `error` at `core/src/serve.rs:2198` (inside the
  spawned task started at `core/src/serve.rs:2185`).
- Read: `rpc_task_status` at `core/src/serve.rs:2526`, `jobs.read()` at
  `core/src/serve.rs:2530`.
- The swarm path reuses the same store: `rpc_swarm` writes at
  `core/src/serve.rs:2438` and `core/src/serve.rs:2453`.

**Consequence:** a daemon restart (crash, deploy, `phantom serve` relaunch)
silently drops every in-flight and completed job; a subsequent
`/rpc/task/status/:id` returns `{"error":"job not found"}`
(`core/src/serve.rs:2537-2539`). This is the `do-things-mesh` "partial" gap in
`../_archive/MASTER-SPEC.md` §3.4 (line 107).

### 0.2 The durable store that already exists (unused by dispatch)

A complete SQLite task store is already in the tree, but the async dispatch path
above does **not** use it:

- `TaskStore` over `~/.phantom-mesh/phantom.db` — `core/src/tasks/store.rs:12`,
  `open_default` at `core/src/tasks/store.rs:18`, schema (`tasks` table) at
  `core/src/tasks/store.rs:44-69`.
- CRUD: `insert` `core/src/tasks/store.rs:73`, `get`
  `core/src/tasks/store.rs:102`, `list` `core/src/tasks/store.rs:113`,
  `update_status` `core/src/tasks/store.rs:154`, `record_progress`
  `core/src/tasks/store.rs:192`.
- `TaskQueue` facade with legal-transition enforcement —
  `core/src/tasks/state.rs:12`; `create` `core/src/tasks/state.rs:26`;
  `transition` `core/src/tasks/state.rs:44`; `is_legal_transition`
  `core/src/tasks/state.rs:138`; **crash recovery** `mark_interrupted`
  `core/src/tasks/state.rs:109`.
- Record + status types: `TaskRecord` `crates/pm-types/src/task.rs:49`
  (note `assigned_node` at `:60`, `error` at `:63`), `TaskStatus`
  `crates/pm-types/src/task.rs:9` (terminal set at `:42`).
- Already wired into `AppState`: `task_queue: Option<TaskQueue>` at
  `core/src/lib.rs:193`; initialised at boot in `core/src/main.rs:95-107`,
  which **already calls `mark_interrupted()`** at `core/src/main.rs:98`.

> **Critical schema finding (do not skip):** the `tasks` table
> (`core/src/tasks/store.rs:46-62`) has **no output/result column** — it stores
> `status`, `error`, `cost_usd`, `turns`, timestamps, `assigned_node`, but never
> the agent's output text. `ClusterJob.output` (`core/src/serve.rs:105`) and the
> `output` field returned by `rpc_task_status` (`core/src/serve.rs:2534`) have no
> home in the current schema. **Gap (a) is therefore not a pure swap — it
> requires an additive schema migration** (§1.2).

### 0.3 The forwarding / handoff path (partial, env-gated)

Worker→worker handoff machinery exists but fires only on a *capability mismatch*
and only behind an env flag:

- `forward_task_to_capable_peer` — `core/src/mesh.rs:2020`; transport
  `assign_task_to_peer_full` — `core/src/mesh.rs:1944`.
- Server decision point in `rpc_task_assign`: `CapsDecision::ForwardTo` branch —
  `core/src/serve.rs:2115`; gated by `forward_on_caps_mismatch_enabled()`
  (`PHANTOM_FORWARD_ON_CAPS_MISMATCH`) — `core/src/serve.rs:2050`,
  `core/src/serve.rs:2084`.
- Cycle guard (inbound chain) — `core/src/serve.rs:2009-2032`;
  `FORWARD_CHAIN_LIMIT = 2` — `core/src/mesh.rs:2305`; `forward_chain` field on
  `TaskAssignRequest` — `core/src/mesh.rs:2218-2224`.

**Consequence:** there is no handoff for *load* or *failure* (only caps), it is
single-extra-hop (limit 2), and it is off by default. Coordinator→worker
selection is `assign_task_async` (`core/src/mesh.rs:1836`) →
`assign_task_to_best_peer` (`core/src/mesh.rs:1767`, picks
`min_by_key(active_tasks)` over online peers at `core/src/mesh.rs:1776`).

### 0.4 The failure-recovery path (designed, not built)

- `../design/SWARM-ARCHITECTURE.md` §6 (lines 225-238) specifies: heartbeat 30s / 3-miss →
  mark unhealthy + **re-dispatch running subtasks**; subtask timeout → cancel +
  retry on another peer; `task_id` idempotency to make retry safe; battery-host
  re-dispatch after `2 × typical_duration`.
- Roadmap row #5 (`../design/SWARM-ARCHITECTURE.md:454`): *"子任務逾時 + 心跳遺漏時重新派發 …
  部分完成（心跳存在，重新派發尚無）"* — **heartbeat exists, re-dispatch does
  not.**
- What exists: `record_probe_result` flips `PeerHealth` — `core/src/mesh.rs:1531`;
  health-aware selection `select_best_peer_with_caps` — `core/src/mesh.rs:2416`
  (healthy-tier first, unhealthy fallback at `:2457`); `poll_task` —
  `core/src/mesh.rs:2046`; `DispatchError::Timeout` variant —
  `core/src/mesh.rs:93`. Heartbeat is feature-gated `experimental-cluster-heartbeat`
  (`../_archive/MASTER-SPEC.md` §3.4 `PEER-HEALTH`, line 118).
- **Idempotency reality check:** `idempotency_key` exists on the wire
  (`core/src/mesh.rs:2229`) but the doc-comment says dedup is *"planned for
  v0.7.0"* — it is **not implemented**. `../_archive/MASTER-SPEC.md:120` cites
  `core/src/idempotency.rs`, **but that file does not exist in the current
  tree** (verified by glob). Safe re-dispatch (gap c) therefore has a hard
  prerequisite that is itself unbuilt — see §3.0.

---

## 1. Gap (a) — Durable async job store

**Goal:** async job state survives a daemon restart; `/rpc/task/status/:id`
answers correctly for a job created before the restart, returning
`failed("interrupted: daemon restart")` for jobs that were mid-flight (reusing
the existing `mark_interrupted` semantics, `core/src/tasks/state.rs:109`).

### 1.1 Files / functions to change

| File | Symbol | Change |
|------|--------|--------|
| `core/src/tasks/store.rs` | `init_schema` `:44`; `insert` `:73`; `SELECT_BASE`/`SELECT_FULL` `:207`/`:211`; `row_to_task` `:215` | Additive migration: add nullable `output TEXT` column; persist + read it. |
| `core/src/tasks/state.rs` | `TaskQueue` `:12` | Add `set_output(task_id, &str)` + a `record_result(task_id, status, output, error)` convenience that transitions and stores output in one call. |
| `core/src/serve.rs` | `ClusterJob` `:103`, `ClusterJobStore` `:110`, `router` `:124`/`:129` | Remove the in-memory map; thread `AppState.task_queue` (`core/src/lib.rs:193`) into the assign/status/swarm handlers instead. |
| `core/src/serve.rs` | `rpc_task_assign` `:1958` (writes `:2175`/`:2188`/`:2198`) | Replace `jobs.write()…insert(ClusterJob{…})` with `task_queue.create(…)` then `transition`/`record_result`. Use the returned `TaskRecord.task_id` as `job_id` (it is already a UUID). |
| `core/src/serve.rs` | `rpc_task_status` `:2526` (read `:2530`) | Replace `jobs.read()` with `task_queue.get(uuid)`; map `TaskStatus`→wire `status` string and read the new `output` column. |
| `core/src/serve.rs` | `rpc_swarm` (writes `:2438`/`:2453`) | Same swap so swarm jobs are durable too. |

### 1.2 Status/field mapping (must be exact)

`ClusterJob.status` strings → `TaskStatus` (`crates/pm-types/src/task.rs:9`):

- `"running"` → `TaskStatus::Running` (`create` yields `Pending`
  `crates/pm-types/src/task.rs:78`, then `transition`→`Running`).
- `"done"`    → `TaskStatus::Completed` + `output` column.
- `"error"`   → `TaskStatus::Failed` + `error` column
  (`core/src/tasks/store.rs:169-173` already persists `error` on terminal).

The wire response in `rpc_task_status` (`core/src/serve.rs:2531-2536`) keeps its
three keys `{status, output, error}`; map `TaskStatus::as_str()`
(`crates/pm-types/src/task.rs:19`) back to the legacy `running|done|error`
strings so existing pollers (`poll_task`, `core/src/mesh.rs:2046`) are
unaffected.

### 1.3 Minimal incremental plan

1. **Schema migration only** (`core/src/tasks/store.rs`): add `output TEXT`
   (nullable) via an idempotent `ALTER TABLE … ADD COLUMN` guarded by a
   `PRAGMA table_info` check in `init_schema` (`:44`); extend `SELECT_*` +
   `row_to_task`. Existing rows/tests stay green (column is nullable). Ship
   alone; no behaviour change.
2. **Queue helper** (`core/src/tasks/state.rs`): add `record_result(...)`.
   Unit-test against a `tempdir` store (mirror existing tests at
   `core/src/tasks/state.rs:156-305`).
3. **Swap the assign handler** (`core/src/serve.rs:1958`) to use
   `state.task_queue` instead of `Extension(jobs)`. If `task_queue` is `None`
   (it is `Option`, `core/src/lib.rs:193`), fall back to the current in-memory
   map so a misconfigured node degrades rather than 500s.
4. **Swap status + swarm handlers**; delete `ClusterJob`/`ClusterJobStore` and
   the `Extension(jobs)` layer once all three handlers are migrated.
5. **Restart correctness:** rely on `mark_interrupted` already running at boot
   (`core/src/main.rs:98`) so pre-restart `Running` jobs become
   `Failed("interrupted: daemon restart")` and `/rpc/task/status` returns a
   definitive terminal answer instead of "not found".

**Done-when:** create a job, kill `phantom serve`, restart, poll the same
`job_id` → get a terminal status (not "job not found"). Add an integration test
that opens a `TaskStore` at a temp path, inserts a `Running` row, reopens, runs
`mark_interrupted`, and asserts `Failed`.

---

## 2. Gap (b) — Peer-to-peer (worker→worker) handoff

**Goal:** a worker that receives a task it should not run locally (over capacity,
or — with gap c — failing) hands it directly to a capable peer, not only back
through the coordinator, and reports the downstream `job_id` to the caller.

### 2.1 What to reuse vs. add

Reuse (already real): `forward_task_to_capable_peer` (`core/src/mesh.rs:2020`),
`assign_task_to_peer_full` (`core/src/mesh.rs:1944`), the `ForwardTo` branch
(`core/src/serve.rs:2115`), cycle guard (`core/src/serve.rs:2009-2032`),
`forward_chain` (`core/src/mesh.rs:2218`), `FORWARD_CHAIN_LIMIT`
(`core/src/mesh.rs:2305`).

Add — broaden the trigger beyond capability mismatch:

| File | Symbol | Change |
|------|--------|--------|
| `core/src/mesh.rs` | new `enum HandoffReason { CapsMismatch, Overloaded, LocalFailure }` near `CapsDecision` (`core/src/mesh.rs:2237+`) | Distinguish *why* we hand off so telemetry + the response `forwarded` reason are explicit. |
| `core/src/mesh.rs` | new `fn should_handoff_for_load(local_active, limit)` | Pure, unit-testable load predicate (mirror existing pure-fn test style, `core/src/mesh.rs:2960+`). |
| `core/src/serve.rs` | `rpc_task_assign` `:1958`, before the local-run spawn at `:2170` | After the caps decision passes `Allow`/`LogAndAllow`, add a load check: if local in-flight ≥ a configured ceiling AND a healthy capable peer exists (`select_best_peer_with_caps`, `core/src/mesh.rs:2416`), call `forward_task_to_capable_peer` instead of spawning locally. Honour the same cycle guard already at `:2009-2032`. |
| `core/src/mesh.rs` | `ClusterConfig` `:199` | Add `max_local_tasks: Option<u32>` (serde-default `None` = unbounded = today's behaviour) to drive the load predicate. |

### 2.2 Constraints to preserve

- **Cycle safety:** handoff MUST go through the existing inbound chain checks
  (`core/src/serve.rs:2009`, `:2021`) and `FORWARD_CHAIN_LIMIT`
  (`core/src/mesh.rs:2305`). Bump the limit to (say) 3 only if a measured 2-hop
  shortfall appears; do not remove it.
- **Auth:** handoff re-signs HMAC inside `assign_task_to_peer_full`
  (`core/src/mesh.rs:1954-1964`); no new auth path.
- **node_name required:** forwarding already refuses when `node_name` is unset
  (`core/src/serve.rs:2049-2058`) — keep that guard for the load path too.

### 2.3 Minimal incremental plan

1. Add `max_local_tasks` to `ClusterConfig` (`core/src/mesh.rs:199`) +
   `should_handoff_for_load` pure fn + tests. No wiring yet.
2. In `rpc_task_assign` (`core/src/serve.rs:1958`), gate the load-handoff behind
   a new env flag `PHANTOM_HANDOFF_ON_OVERLOAD=1` (mirror
   `forward_on_caps_mismatch_enabled`, `core/src/serve.rs:2050`) so default
   behaviour is unchanged.
3. Reuse the existing `ForwardTo` response shape
   (`core/src/serve.rs:2126-2135`: `job_id`, `dispatched_to`, `forwarded:true`)
   so clients/poll logic need no change.
4. Add the `HandoffReason` to the `forward_decision` tracing event
   (`core/src/mesh.rs:2031-2041`) for observability.

**Done-when:** a 2-node test where node-A is saturated (active ≥ `max_local_tasks`)
and node-B is healthy+capable: a task POSTed to A returns
`{forwarded:true, dispatched_to:"B"}` and B's durable store (gap a) shows the row.

---

## 3. Gap (c) — Failure-recovery routing (auto re-dispatch)

**Goal:** when a peer holding an in-flight job times out or goes unhealthy, the
coordinator re-dispatches that job to another capable peer, at-most-once.
Implements the missing half of `../design/SWARM-ARCHITECTURE.md` roadmap #5 (`:454`) and
the §6 failure semantics (`:225-238`).

### 3.0 Hard prerequisite — idempotency (gap c is unsafe without it)

Re-dispatch can double-execute a destructive task. `../design/SWARM-ARCHITECTURE.md:232`
("Idempotency") makes `task_id` dedup the contract that makes retry safe. Today
`idempotency_key` is wire-only and unenforced (`core/src/mesh.rs:2225-2230`);
`core/src/idempotency.rs` (cited in `../_archive/MASTER-SPEC.md:120`) **does not exist**.
**Therefore the first deliverable of gap (c) is server-side dedup**, not the
retry loop:

| File | Symbol | Change |
|------|--------|--------|
| `core/src/serve.rs` | `rpc_task_assign` `:1958`, before spawn `:2170` | If `req.idempotency_key` (`core/src/mesh.rs:2229`) or `req.task_id` matches an existing non-terminal/terminal row in the durable store (gap a), return the existing `job_id` instead of spawning a duplicate. |
| `core/src/tasks/store.rs` | new `get_by_idempotency_key` (add nullable `idempotency_key TEXT` column + index, same migration pattern as §1.2) | Backing lookup for the dedup check. |

### 3.1 Re-dispatch engine

| File | Symbol | Change |
|------|--------|--------|
| `core/src/mesh.rs` | `record_probe_result` `:1531` (heartbeat path) | On a peer transitioning `Healthy→Unhealthy`, emit the set of `task_id`s last `assigned_node = that peer` (`TaskRecord.assigned_node`, `crates/pm-types/src/task.rs:60`) as re-dispatch candidates. |
| `core/src/mesh.rs` | new `async fn redispatch_orphans(&self, store)` | For each candidate still `Running` in the durable store: pick a new peer via `select_best_peer_with_caps` (`core/src/mesh.rs:2416`), `assign_task_to_peer_full` (`core/src/mesh.rs:1944`) with the **same** `idempotency_key`, and record the new `assigned_node`. |
| `core/src/serve.rs` | `rpc_task_assign` spawn `:2185` | On the local run, persist `assigned_node = self node_name` and set a deadline derived from `ClusterConfig` (see below); on local timeout, mark `Failed` so the sweep re-dispatches. |
| `core/src/mesh.rs` | `ClusterConfig` `:199` | Add `subtask_timeout_secs: Option<u64>` (per §6 "Subtask timeout"). `None` = no auto-timeout (today's behaviour). |
| `core/src/mesh.rs` | heartbeat loop (feature `experimental-cluster-heartbeat`, gated per `../_archive/MASTER-SPEC.md:118`) | Call `redispatch_orphans` after each probe sweep. |

### 3.2 Constraints

- Gate the whole re-dispatch behind `experimental-cluster-heartbeat` (it depends
  on `record_probe_result`, `core/src/mesh.rs:1531`) so default single-node /
  no-heartbeat deploys are byte-for-byte unchanged
  (`../_archive/MASTER-SPEC.md` `CROSS-DISPATCH` line 117 explicitly keeps the P0 slice off
  the experimental heartbeat).
- Re-dispatch MUST be idempotent (§3.0) — no retry ships before dedup ships.
- Respect `FORWARD_CHAIN_LIMIT` (`core/src/mesh.rs:2305`) and reuse
  `DispatchError::Timeout` (`core/src/mesh.rs:93`) classification.
- Battery-host aggressive retry (`../design/SWARM-ARCHITECTURE.md:235-238`,
  `2 × typical_duration`) is a **later** refinement once the basic
  unhealthy-peer sweep works; note it, don't build it stage-1.

### 3.3 Minimal incremental plan

1. **Idempotency first** (§3.0): migration + `get_by_idempotency_key` + dedup
   check in `rpc_task_assign`. Ship + test alone.
2. **Persist `assigned_node`** on every assign (local + forwarded) so the sweep
   has data to act on.
3. **`redispatch_orphans`** + wire into the heartbeat sweep behind the existing
   feature flag.
4. **Subtask timeout**: add `subtask_timeout_secs`, wrap the local run
   (`core/src/serve.rs:2185`) in `tokio::time::timeout`, mark `Failed` on
   expiry so the sweep picks it up.

**Done-when:** a 2-node test: node-B takes a job, B is killed; after the next
heartbeat sweep the coordinator re-dispatches to node-C; the durable store shows
one logical `task_id`, two `assigned_node` transitions, and exactly one
terminal `Completed` (dedup proven — re-running the original on a recovered B is
a no-op).

---

## 4. Sequencing summary

```
(a) durable store  ─┐  (independent; unblocks (c) status survival)
(b) p2p handoff    ─┤  (independent of (a); reuses existing forward path)
(c) failover       ─┘  REQUIRES (a) [durable rows to sweep] + idempotency [§3.0]
```

Ship order: **(a) → idempotency (§3.0) → (c) → (b)** — or (b) any time after (a),
since (b) is env-gated and self-contained. Each numbered sub-step above is an
independently shippable, feature/env-gated increment that leaves default
behaviour unchanged when its flag is off.

## 5. Risks / honest caveats

- The `tasks` table has **no output column today** (`core/src/tasks/store.rs:46`);
  gap (a) is a migration, not a swap. Largest single risk.
- `core/src/idempotency.rs` referenced in `../_archive/MASTER-SPEC.md:120` is **absent**;
  do not assume idempotency exists — gap (c) must build it.
- Heartbeat is feature-gated and off by default; gap (c) inherits that gate, so
  re-dispatch only runs on clusters that opt into
  `experimental-cluster-heartbeat`.
- `FORWARD_CHAIN_LIMIT = 2` (`core/src/mesh.rs:2305`) bounds multi-hop handoff;
  deep chains will be rejected by design — acceptable for hub-and-spoke
  (`../design/SWARM-ARCHITECTURE.md` §1, shipping topology).
