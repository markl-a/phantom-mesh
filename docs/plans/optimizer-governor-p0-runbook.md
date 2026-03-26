# Optimizer Governor P0 Runbook

> Date: 2026-03-21
> Scope: `phantom-mesh`
> Purpose: maintenance notes for the initial optimizer-governor foundation

---

## What P0 Added

P0 did two things:

1. Wired `TrajectoryLogger` into the real runtime path.
2. Added a persistent `OptimizerStore` for versioned policy baselines and optimization run history.

This is the minimum foundation needed before building the actual daily optimizer.

---

## Files To Know

### Core wiring

- `src/main.rs`
- `src/agent_runtime.rs`
- `src/llm_router.rs`

### New persistence layer

- `src/optimizer_store.rs`
- `src/lib.rs`

### Design reference

- `docs/deep-analysis/51-optimizer-governor-architecture.md`

---

## What Is Wired Now

### 1. Trajectory logger -> LlmRouter

Startup now creates a `TrajectoryLogger` before `llm_router` is wrapped in `Arc`, then calls:

```rust
llm_router.set_trajectory_logger(tl.clone());
```

Why this matters:

- `LlmRouter::smart_route()` uses trajectory quality stats
- without this, provider selection cannot improve from historical quality/cost data

### 2. Trajectory logger -> AgentRuntime

Startup now also calls:

```rust
agent_runtime.set_trajectory_logger(tl.clone());
```

Why this matters:

- `AgentRuntime` logs successful runs, timeouts, loops, stale loops, and failures
- prompt optimization and future governor logic need these trajectories

### 3. Optimizer store bootstrap

Startup now creates:

`~/.phantom-mesh/optimizer.db`

And ensures these baseline policies exist:

- `prompt.default`
- `routing.default`
- `workflow.default`
- `runtime_tuning.default`

These are bootstrap policies only.
They are intentionally simple and safe.

### 4. Read-only debug endpoints

Two endpoints were added:

- `GET /optimizer/policies`
- `GET /optimizer/runs`

These exist so you can inspect state without opening SQLite manually.

---

## Databases Created

### `~/.phantom-mesh/trajectories.db`

Purpose:

- stores agent execution trajectories

Used by:

- routing improvement
- prompt optimization
- later governor analysis

### `~/.phantom-mesh/optimizer.db`

Purpose:

- stores policy versions
- stores optimization run history

Tables:

- `policy_versions`
- `optimization_runs`

---

## Safe Mental Model

Think of the governor as three layers:

1. `telemetry`
2. `policy`
3. `runtime`

P0 only establishes:

- telemetry persistence
- policy persistence

P0 does **not** yet perform automatic optimization.

---

## How To Verify

### Compile checks

Run:

```powershell
cargo test optimizer_store --lib
cargo check --bin phantom-mesh
```

Expected:

- optimizer store tests pass
- binary compiles

### Runtime logs

When startup is correct, logs should include messages like:

- `TrajectoryLogger initialized and wired to LlmRouter`
- `TrajectoryLogger wired to AgentRuntime`
- `Optimizer store initialized`

### HTTP checks

After daemon starts:

```powershell
curl http://localhost:7878/optimizer/policies
curl http://localhost:7878/optimizer/runs
```

Expected:

- `/optimizer/policies` returns at least the baseline policies
- `/optimizer/runs` may be empty initially

---

## Manual Repair Guide

If you later edit this by hand and break it, check in this order.

### Problem A: trajectories stop being useful

Symptoms:

- prompt optimizer has no data
- smart routing behaves like fixed default routing
- trajectory stats endpoints look empty or stale

Check:

1. `src/main.rs` still creates `TrajectoryLogger` before wrapping `llm_router` and `agent_runtime` in `Arc`
2. `llm_router.set_trajectory_logger(...)` still exists
3. `agent_runtime.set_trajectory_logger(...)` still exists

If one of those is missing, restore the wiring first.

### Problem B: optimizer.db never appears

Symptoms:

- `/optimizer/policies` says optimizer store unavailable
- no baseline policy exists

Check:

1. `src/optimizer_store.rs` still exports `OptimizerStore`
2. `src/lib.rs` still has `pub mod optimizer_store;`
3. `src/main.rs` still initializes `OptimizerStore::new(...)`
4. `AppState` still includes `optimizer_store`

### Problem C: optimizer endpoints compile but return empty errors

Check:

1. `optimizer_store` is stored inside `AppState`
2. router still registers:
   - `/optimizer/policies`
   - `/optimizer/runs`
3. startup bootstrap still calls `ensure_baseline_policy(...)`

### Problem D: policy baselines are malformed

Symptoms:

- startup warnings about bootstrap failure
- later governor code fails to parse policy JSON

Check the four baseline JSON strings in `src/main.rs`.

Current baseline policy IDs:

- `prompt.default`
- `routing.default`
- `workflow.default`
- `runtime_tuning.default`

If you change their JSON shape, keep it backward-compatible or bump your parser logic with care.

---

## If You Need To Reset Everything

### Reset only optimizer policies

Delete:

```text
~/.phantom-mesh/optimizer.db
```

Then restart daemon.

Startup will recreate the DB and bootstrap baseline policies.

### Keep trajectories but reset policies

This is usually the correct move.

Do **not** delete `trajectories.db` unless you really want to throw away learning data.

---

## Safe Ways To Change P0 By Hand

### Safe edits

- add new `PolicyType`
- add new baseline policy
- add new read-only optimizer endpoint
- add new metadata columns to `optimization_runs`

### Less safe edits

- moving trajectory logger initialization far away from router/runtime wiring
- changing baseline JSON shape without updating consumers
- changing `policy_versions` schema without migration handling

### Do not casually change

- `AgentRuntime` trajectory logging behavior
- `LlmRouter::smart_route()` assumptions
- startup ordering around `Arc<LlmRouter>` and `Arc<AgentRuntime>`

---

## Startup Ordering Constraint

This part matters:

1. build mutable `llm_router`
2. create `TrajectoryLogger`
3. attach logger to `llm_router`
4. wrap `llm_router` in `Arc`
5. build mutable `agent_runtime`
6. attach logger to `agent_runtime`
7. wrap `agent_runtime` in `Arc`

If you wrap too early, you make the wiring harder and risk losing the logger hookup.

---

## Expected Next Step After P0

P1 should read from:

- trajectories
- quality pipeline
- node scores
- consistency history

And only produce:

- prompt candidates
- routing candidates

P1 should **not** generate tools yet.

---

## Suggested Personal Workflow

If you need to touch this manually later, follow this order:

1. Edit code.
2. Run `cargo test optimizer_store --lib`.
3. Run `cargo check --bin phantom-mesh`.
4. Start daemon.
5. Call `/optimizer/policies`.
6. Check startup logs for trajectory wiring lines.

If step 5 or 6 fails, do not continue to bigger optimizer work until P0 is healthy again.

---

## Short Recovery Checklist

When confused, verify these exact conditions:

- `src/optimizer_store.rs` exists
- `src/lib.rs` exports it
- `src/main.rs` stores it in `AppState`
- `src/main.rs` registers `/optimizer/policies`
- `src/main.rs` registers `/optimizer/runs`
- `src/main.rs` wires `TrajectoryLogger` into both router and runtime
- `cargo check --bin phantom-mesh` passes

If all seven are true, P0 is usually healthy.
