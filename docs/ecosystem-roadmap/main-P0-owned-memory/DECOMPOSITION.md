# P0 ② Owned-memory — task decomposition (2026-06-17)

> 🔄 **STATUS UPDATE (2026-06-21)**: M1 (Store hand-off) is **DONE** — `skill_store()` now
> persists the queued extract hand-off (`core/src/skill_wire.rs:1836`; tests
> `skill_store_persists_queued_extract_handoff` :3304 / `skill_store_drains_queue_fifo_and_persists_all`
> :3360), and recall-before-run is wired into the agent loop (`agent.rs:730`, `PHANTOM_OWNED_MEMORY`).
> The two BIG-GOAL `unimplemented!()` fns are implemented (no longer stubs). **Remaining = the semantic
> `ort` embedding leg (M3/M5, deferred / human-led)**; the FTS5 keyword path is the live default. The
> 2026-06-17 table below is kept for the M2–M5 breakdown; treat M1 as ✅.

**Honest correction to BIG-GOAL (2026-06-17 — see STATUS UPDATE above for what shipped since):** the two functions BIG-GOAL calls `unimplemented!()`
are now **de-panicked stubs**, and the FTS5 keyword-memory path already works:
- `store_skill(&skill)` = REAL rusqlite write (self-provisions schema, FTS5-recallable). ✅
- `fts5_search` / `recall_skills` = REAL FTS5 keyword recall, degrades cleanly. ✅
- `skill_store()` = ~~a parameterless **dispatch stub**~~ → **now persists the hand-off (M1 DONE, 2026-06-21)**.
- `embedding_search()` = returns `Err(())` by default — the **semantic (embedding) leg is still deferred**
  (`ort` + a model not in deps), FTS5 fallback is the live path. ❌ the real remaining capability.

So "implement ② owned-memory" = ~~wire the Store hand-off +~~ **add the semantic recall leg** (Store hand-off done).

## Executor model — every task here runs on codex, agy, OR a Claude subagent
Each `*.task` file is an **executor-agnostic body** (plain spec + TDD + self-verify; no
tool-specific assumptions). Routing:
- **codex** → cluster worker (`codex_agent`) via `scripts/cluster-dispatch.sh dispatch <node>` — governed, structured tool_calls.
- **agy** → an `agy_agent` worker (z13/Mac have agy) — edits files, but NOT tool-gated (parse_agy fidelity gap), so keep agy on lower-risk tasks + review its diff.
- **subagent** → the Claude `Agent` tool on z13 (full Read/Write/Edit/Bash; uses Claude quota).
Dispatch helper: `overnight/memory-tasks/assign.sh <codex|agy|subagent> <task-file>`.

## Tasks (TDD; touch core/src/skill_wire.rs + migrations unless noted)

| ID | Title | Risk / executor fit | Gap it closes |
|---|---|---|---|
| **M1** | Wire the Store dispatch hand-off | LOW — all 3 | `skill_store()` stub → real store via `store_skill` |
| **M2** | Embedding storage column + `EmbeddingProvider` trait | LOW-MED — all 3 | persistence + abstraction for the semantic leg |
| **M3** | `embedding_search` cosine over stored vectors (fixture provider) | MED — codex/subagent (agy review-only) | the semantic recall leg (deterministic, testable) |
| **M4** | Real API EmbeddingProvider (openai/gemini text-embedding) | MED-HIGH — codex/subagent + review gate | a real embedder (online) |
| **M5** | Local offline embeddings (`ort` + all-MiniLM-L6-v2) | HIGH — **human-led, NOT autonomous** | offline semantic recall (heavy: deps + model + x-platform build) |

**Sequencing:** M1 → M2 → M3 are the autonomous-friendly P0 core (pure logic + deterministic
tests). M4 needs a key + review. M5 is flagged human-led (adds a heavy dep + a model file —
do NOT let an autonomous worker bloat `Cargo.toml`/ship a model unreviewed).

**Apex trace:** all five serve ② (owned compounding memory, the #1 ability) + P3 (Evolve Mesh).
Test gate per task: unit (no net/device) + the skill_wire module's `cargo test` must stay green.
