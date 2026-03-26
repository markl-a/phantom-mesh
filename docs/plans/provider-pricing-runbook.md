# Provider Pricing Runbook

> Date: 2026-03-21
> Scope: `phantom-mesh`
> Purpose: runtime-editable LLM/API pricing rules for cost accounting

---

## What Was Added

This change adds a first-class `ProviderPricingStore` so API pricing is no longer hardcoded inside runtime execution.

It now supports:

- runtime-editable provider/model pricing rules
- exact provider bucket control for free-tier, subscription, or paid API assumptions
- automatic wiring into `AgentRuntime` cost accounting
- HTTP endpoints for listing, updating, and estimating price rules

---

## Files To Know

- `src/provider_pricing.rs`
- `src/agent_runtime.rs`
- `src/main.rs`
- `src/lib.rs`

---

## Storage

Startup now creates:

`~/.phantom-mesh/pricing.db`

SQLite table:

- `provider_price_rules`

Stored fields:

- `provider`
- `model_pattern`
- `input_usd_per_1m_tokens`
- `output_usd_per_1m_tokens`
- `notes`
- `updated_at`

Primary key:

- `(provider, model_pattern)`

---

## Matching Rules

Rules match by:

1. exact provider
2. longest matching `model_pattern`
3. fallback `*` pattern if present
4. final fallback to legacy builtin pricing

Pattern semantics:

- `*` matches any model under that provider
- any other pattern is a lowercase substring match against the model id

Example:

- `openai / gpt-4o`
- `openai / *`

When estimating `gpt-4o-mini`, the `gpt-4o` rule wins over `*`.

---

## Runtime Behavior

`AgentRuntime` now checks `ProviderPricingStore` before using builtin pricing.

That means:

- if you update a provider rule while the daemon is running, subsequent runs use the new price
- recorded `api_estimated_cost_usd` reflects the current store rule
- `estimated_cost_usd` continues to mean total cost, not API-only cost

---

## API Endpoints

### `GET /pricing/rules`

Returns all pricing rules.

### `GET /pricing/rules/:provider`

Returns all pricing rules for a single provider.

### `POST /pricing/rules`

Upserts one pricing rule.

Body:

```json
{
  "provider": "anthropic",
  "model_pattern": "claude-sonnet-4",
  "input_usd_per_1m_tokens": 3.0,
  "output_usd_per_1m_tokens": 15.0,
  "notes": "Updated after provider pricing change"
}
```

### `POST /pricing/estimate`

Body:

```json
{
  "provider": "openai",
  "model": "gpt-4o",
  "tokens_in": 1200,
  "tokens_out": 800
}
```

Returns:

- matched rule pattern
- price source
- input cost
- output cost
- total cost

---

## How To Represent Different Resource Types

### Paid API

Use real token prices.

Example:

- `input_usd_per_1m_tokens = 2.5`
- `output_usd_per_1m_tokens = 10.0`

### Free API

Set both prices to `0.0`.

### Subscription Bucket

If you want to treat a subscription-backed provider as zero marginal cost:

- set both prices to `0.0`
- use budget/quota logic elsewhere to decide how aggressively to consume it

If you later want to amortize a monthly subscription into per-token or per-call cost, update the rule values without code changes.

---

## No Electricity Scenario

If you currently do not pay electricity, you do not need code changes.

Use the existing power profile API and set:

- `electricity_usd_per_kwh = 0.0`

Then decide whether you still want to count:

- `depreciation_usd_per_hour`
- `cooling_usd_per_hour`

Typical choices:

- strict accounting: electricity `0.0`, depreciation non-zero
- pure cash accounting: electricity `0.0`, depreciation `0.0`, cooling `0.0`

That choice depends on whether you want to optimize for cash flow or true hardware wear.

---

## Safe Manual Edits

Safe:

- update provider rules through `/pricing/rules`
- set free/subscription buckets to `0.0`
- adjust `electricity_usd_per_kwh` to `0.0` when your current marginal electricity cost is zero

Be careful:

- do not confuse subscription cost with marginal token cost
- if you zero out a provider, make sure some other quota/budget layer still prevents abuse
- if you zero out electricity, keep depreciation if you still care about hardware wear

---

## Suggested Next Step

The next meaningful upgrade is to add a higher-level `economics policy` layer that can:

1. switch provider pricing rules by date or billing cycle
2. model subscription pools separately from paid API pools
3. feed current pricing directly into route scheduling and unit economics

That is the step that turns runtime-editable pricing into full economics-aware routing.
