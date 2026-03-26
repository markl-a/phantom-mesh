# Power Economics Runbook

> Date: 2026-03-21
> Scope: `phantom-mesh`
> Purpose: maintenance notes for the initial hardware cost accounting layer

---

## What Was Added

This change adds a first-class `PowerEconomics` module so the daemon can estimate:

- per-node hourly hardware cost
- per-run local execution cost
- whether a node should merely run or be saturated based on expected revenue

The module does not sample live wattage from the OS.
It stores editable assumptions per node and computes consistent estimates from them.

---

## Files To Know

### New module

- `src/power_economics.rs`

### Wiring

- `src/lib.rs`
- `src/main.rs`
- `src/agent_runtime.rs`
- `src/cost_tracker.rs`

### Design reference

- `docs/deep-analysis/52-economic-scheduler-policy.md`

---

## Storage

Startup now creates:

`~/.phantom-mesh/power.db`

and extends:

`~/.phantom-mesh/costs.db`

SQLite table:

- `node_power_profiles`

Stored fields:

- `node_id`
- `idle_watts`
- `active_watts`
- `electricity_usd_per_kwh`
- `depreciation_usd_per_hour`
- `cooling_usd_per_hour`
- `notes`
- `updated_at`

`cost_records` now also stores:

- `node_id`
- `api_estimated_cost_usd`
- `hardware_estimated_cost_usd`
- `estimated_cost_usd`

---

## Default Profiles

The module seeds editable default profiles for:

- `local`
- `Z13`
- `M1Mac`
- `AYANEO`
- `Acer`

These are bootstrap assumptions only.
They are not measurements.
Adjust them to match your real power draw, electricity rate, and depreciation model.

---

## Formula

### Average watts

```text
avg_watts = idle_watts + (active_watts - idle_watts) * load_factor
```

Where `load_factor` is clamped to `0.0 .. 1.0`.

### Hourly electricity cost

```text
electricity_usd_per_hour = (avg_watts / 1000) * electricity_usd_per_kwh
```

### Hourly node cost

```text
total_usd_per_hour =
    electricity_usd_per_hour
  + depreciation_usd_per_hour
  + cooling_usd_per_hour
```

### Per-run cost

```text
run_cost = total_usd_per_hour * (duration_secs / 3600)
```

### Profitability

Two thresholds are exposed:

```text
break_even_revenue_per_hour =
    api_cost_per_hour
  + node_cost_per_hour
```

```text
aggressive_utilization_floor_per_hour =
    api_cost_per_hour
  + depreciation_per_hour
  + cooling_per_hour
  + 2 * electricity_per_hour
```

Interpretation:

- `should_run` means expected revenue clears break-even
- `should_saturate` means expected revenue also clears the more aggressive "run it hard" floor

---

## API Endpoints

### `GET /power/nodes`

Returns:

- stored power profiles
- matching cluster metadata if the node is registered
- live hourly estimate using current `cpu_load`
- full-load hourly estimate

### `GET /power/nodes/:node_id`

Returns:

- the stored profile
- cluster metadata if available
- hourly estimates at idle, mid, full, and current live load

### `POST /power/nodes/:node_id`

Upserts a profile.

Body:

```json
{
  "idle_watts": 18.0,
  "active_watts": 70.0,
  "electricity_usd_per_kwh": 0.10,
  "depreciation_usd_per_hour": 0.05,
  "cooling_usd_per_hour": 0.015,
  "notes": "ROG Flow Z13 tuned after smart-plug measurement"
}
```

### `POST /power/estimate`

Body:

```json
{
  "node_id": "Z13",
  "duration_secs": 5400,
  "load_factor": 0.85
}
```

Returns a per-run cost estimate.

### `POST /power/profitability`

Body:

```json
{
  "node_id": "Z13",
  "expected_revenue_per_hour_usd": 1.20,
  "api_cost_per_hour_usd": 0.10,
  "load_factor": 0.90
}
```

Returns:

- break-even threshold
- aggressive utilization threshold
- projected profit per hour
- `should_run`
- `should_saturate`

---

## Runtime Integration

This module is now merged into the main `CostTracker` ledger.

At runtime:

- `AgentRuntime` records `estimated_cost_usd` as `api + hardware`
- `CostRecord` keeps both the combined total and the `api/hardware` split
- `/costs` now returns today's API and hardware breakdown
- trajectory entries in `AgentRuntime` use the combined total instead of API-only cost

Current load-factor heuristic:

- local inference providers (`ollama`, `lmstudio`, `lemonade`) use `load_factor = 1.0`
- remote/API providers use `load_factor = 0.25`

If `PHANTOM_MESH_NODE_ID` is set, that node id is written into cost records.
If the named node has no power profile, runtime cost estimation falls back to the `local` profile.

Historical compatibility:

- old `cost_records` rows are migrated in place
- legacy rows backfill `api_estimated_cost_usd = estimated_cost_usd`
- legacy rows keep `hardware_estimated_cost_usd = 0`

---

## Safe Manual Edits

Safe:

- adjust any node profile through the `/power/nodes/:node_id` endpoint
- change the default seeded profile values in `src/power_economics.rs`
- tune the break-even and aggressive utilization formula if you want a stricter rule
- set `electricity_usd_per_kwh = 0.0` if your current marginal electricity cost is effectively zero

Be careful:

- if you rename node IDs in taxonomy or cluster registration, update the matching power profile names too
- if you change `PHANTOM_MESH_NODE_ID`, make sure a matching power profile exists or the runtime will fall back to `local`
- do not assume seeded defaults are accurate measurements

---

## Suggested Next Step

Once profile values are tuned from real smart-plug or OS measurements, the next patch should:

1. feed hardware-aware cost into `UnitEconomics`
2. teach `NodeScorer` to consume real node-hour cost instead of placeholder totals
3. replace the fixed provider load-factor heuristic with measured per-task or per-node defaults

That is the step that turns this from "combined run-level accounting" into "economics-aware scheduling."
