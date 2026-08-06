# Spectyn-Mesh Ecosystem Roadmap — FINAL

> Unified synthesis of the nine finalized project plans (`overnight/plans/FINAL-*.md` +
> the companion/ai-feed/enterprise `PLAN-*.md` planning outputs). Source of truth for
> cross-project sequencing. Apex anchor: "a compounding private AI I use daily"
> (`docs/superpowers/BIG-GOAL.md`, FINAL re-lock 2026-06-11). Every project below was
> re-grounded against its own code-verified FINAL; this layer only sequences across them.

---

## 1. Dependency graph

`spectyn-mesh` ("main") is the **runtime substrate** — the single Rust binary (daemon +
CLI + Tauri/React app), `~/.spectyn-mesh/` data root, encrypted Life Track event store,
`spectyn event capture` / `recall` / FTS5, the local model router, and HMAC mesh RPC. Every
other project either **produces events into** it or **consumes events out of** it.
`spectyn-companion` is the **keystone consumer**: it aggregates the outputs of all six
producers (and main's own events) into the daily/weekly shame-free report — the artifact the
user actually reads each day.

```
                          ┌───────────────────────────────────────────┐
                          │            spectyn-mesh (MAIN)             │
                          │  runtime · event store · recall/FTS5 ·     │
                          │  local model router · mesh RPC · app       │
                          └───────────────────────────────────────────┘
                            ▲ emit events        │ provides router/recall/capture
        ┌───────────────────┼────────────────────┼───────────────────┐
        │ emit              │ emit               ▼ consume             │ emit
 ┌─────────────┐    ┌─────────────┐      ┌──────────────┐     ┌─────────────┐
 │ spectyn-    │    │ spectyn-    │      │   mesh-site  │     │ spectyn-    │
 │ ai-feed     │    │ finance     │      │ (broker +    │     │ flow        │
 │ (RSS→digest │    │ (ledger→    │      │  landing):   │     │ (YAML       │
 │  →SRS)      │    │  report)    │      │  login/OAuth │     │  runner)    │
 └─────┬───────┘    └─────┬───────┘      │  vault/clust │     └─────┬───────┘
       │ events           │ events       │  /dispatch   │           │ events
       │ (logs/ +         │ (mesh event  └──────┬───────┘           │ (exec via
       │  capture)        │  under        consumes│ main's API       │  spectyn exec)
       │                  │  events/)     wire-contract             │
       ▼                  ▼                       │                  ▼
 ┌──────────────────────────────────────────────────────────────────────────┐
 │                       spectyn-companion  (KEYSTONE)                        │
 │  aggregate_range() over: main recall/events + ai-feed logs + finance      │
 │  events + flow logs + heartbeats → daily/weekly insight report (delivery: │
 │  file/Telegram/email, shame_free_check gated)                             │
 └──────────────────────────────────────────────────────────────────────────┘

 ┌─────────────┐         ┌──────────────┐
 │ spectyn-    │ events  │ spectyn-     │  (independent satellites: emit standardized
 │ quant       │────────▶│ enterprise   │   events into main; not consumed by companion
 │ (台股 bt)   │  (P3)   │ (on-prem)    │   in the daily loop — adjacent, not core)
 └─────────────┘         └──────────────┘
```

Edges that matter:

- **Everything ⇒ main.** All producers write under `~/.spectyn-mesh/` and/or call
  `spectyn event capture`; the event-store + recall + FTS5 schema is the shared contract.
  If main's capture/recall path is dishonest or fragile, every downstream insight is poisoned.
- **Producers ⇒ companion.** Companion's `aggregate_range()` reads main `recall`/event-dir,
  ai-feed logs, finance mesh events, flow logs, and heartbeats. It is the only project whose
  value is *defined by* the others existing and emitting. Today it passes `health_data={}` /
  `commits=[]` because the upstream emitters are thin — companion can't outrun its inputs.
- **mesh-site ⇒ main wire-contract.** The broker (`spectynmesh-io`) implements the
  `spectyn login` / OAuth / vault / cluster / dispatch contract main's CLI calls; any change to
  the loopback token handoff is a two-repo contract change (main `login_broker` ↔ broker
  `/auth/cli/exchange`).
- **quant + enterprise = adjacent satellites.** They emit into main but are *not* on
  companion's daily-loop critical path; they widen the ecosystem rather than deepen the core loop.

---

## 2. Cross-project priority order

| Pri | Project(s) | One-line rationale |
|-----|------------|--------------------|
| **P0** | **spectyn-mesh (main)** | The runtime everything compiles, captures, recalls, and routes through — its v0.6.0 honesty floor (no mock/leak surfaces) + SYS-B/C/D correctives are the precondition for any downstream value; a poisoned event store poisons the whole graph. |
| **P1** | **spectyn-companion** | The keystone the user actually reads daily; it converts six producers' outputs into the compounding daily report, so its `AggregateWindow` data plane is the single highest-leverage consumer to make real. |
| **P2** | **spectyn-ai-feed / spectyn-finance / spectyn-flow** | The daily-use producers that *feed* the keystone — until they emit real, captured, deduped events (FTS5 capture, ledger reports, flow runs) companion has nothing rich to aggregate; these three turn the loop from baseline-shaped to compounding. |
| **P3** | **spectyn-quant / mesh-site** | quant = a deep but adjacent satellite (台股 backtest realism, not on the core daily loop); mesh-site = the public/account surface — important for distribution and the login wire-contract, but not blocking the personal daily compounding loop. |
| **P4** | **spectyn-enterprise** | Furthest from the apex "private AI I use daily"; on-prem Git/LDAP/SSO connectors serve the future 副業/portfolio angle, which per governance never shapes the product — freeze contracts, build last. |

> Note: P3 mesh-site contains one item that punches above P3 — the **plaintext loopback
> token leak** (`oauth.ts`/`email.ts` ship raw tokens as `?p=base64(json)`). Treat *that single
> security fix* as a P0-adjacent ship-blocker even though the project is P3 overall (see §3).

---

## 3. Three highest-leverage next moves (whole ecosystem)

1. **main — land the v0.6.0 honesty floor + SYS-B offline bypass.**
   *Why:* it is the P0 gate for the entire graph. Replace `SecurityPanel.tsx` `MOCK_EVENTS`
   with real flight-recorder data + honest empty states, fix `setup-cloud-linux.sh` verbs/IPs,
   green `npm run build` + `tsc` + the skill_wire panic-gate regression, and add the first-run
   local-ed25519 path so a fresh install is usable with the broker/OAuth unreachable. Until main
   is honest and single-machine-usable offline, every downstream event is suspect and the
   login-first dead-end blocks first use. One project, but it unblocks all six others.

2. **companion — build the `AggregateWindow` + normalized-schema data plane and wire real
   inputs.** *Why:* companion is the keystone and it is currently starved — `aggregate_range()`
   is just a dict of `DailyAggregate` and `reporter._run_insights()` hard-codes `health_data={}`
   / `commits=[]`. Making the cross-day data plane real (typed normalized records for mesh
   events, satellite logs, health/commit samples) is the one change that converts the ecosystem
   from "six tools that log" into "a compounding daily report" — i.e. it directly realizes the
   apex anchor and is the highest-ROI single consumer move.

3. **ai-feed (+finance) — verify/harden the `spectyn` FTS5 capture adapter.** *Why:* this is the
   shared seam between every producer and the keystone. ai-feed's `_try_capture_fts5()` is
   unverified best-effort; if capture silently fails, companion aggregates nothing and recall
   can't find the entries — the compounding loop is broken at the source. Extracting a tested
   `capture_entry()` adapter (monkeypatched `which`/`subprocess`, plus a recall round-trip smoke)
   proves the produce→store→consume path end-to-end. It is small, but it de-risks the exact
   contract that turns three P2 producers into real companion fuel; the same adapter pattern then
   hardens finance's `events.emit` path. (Pair with the mesh-site loopback-token security fix as
   the one cross-cutting ship-blocker to schedule alongside.)

---

## 4. How to build it with the governed cluster

The mechanics, mapped onto the standing multi-AI delegation rule (`AGENTS.md §3.5`,
`.claude/skills/local-ai/ask.sh`) and the L1 governed-run substrate. Loop per work item:

**Decompose** — Take the per-project FINAL top-3 task breakdowns as the unit of work. Slice each
into single-file, single-function, independently-verifiable changes (the mesh-references plan
proves single-file fixes are the highest ROI). One item = one branch off the relevant repo's
base. Order strictly by §2 (P0 main floor → P1 companion data plane → P2 producer capture →
P3/P4), and inside a project by its own P1→P3 list.

**Dispatch** — Fan out, never execute solo (operator directive 2026-06-12). Route by tool
strength: **codex** = per-file mechanical edits/codegen (one file per call, lint before commit —
e.g. the `cli.py --bank` wiring, `term.rs` NO_COLOR fix, `eprintln!→println!` swap);
**opencode** = repo-file reading/synthesis (e.g. mapping companion's `aggregate_range`
call-sites before refactor); **agy** = pure Q&A / second-opinion on a design seam (e.g. "is the
loopback `code`→token exchange the right shape?"). Wrap remote/unattended dispatch in
`spectyn govern <codex|opencode|agy>` so the flight-recorder + governor + phone-escalation
capture each command/file-change; governor default = **safe-pause** (SYS-C). Dispatch
independent items concurrently across nodes; respect the cross-OS gotchas (Windows
`CreateProcessAsUserW 1312` sandbox failures already broke the codex planners — prefer the
read-only/MCP path or native PTY for agy).

**Verify** — Hard, exit-code-gated, on the producing node. Run the item's own tests
(`cargo test -p …`, `pytest`, `npm run build` + `tsc`, `wrangler deploy --dry-run`), never mask
exit codes through pipes (check `$?` on the command itself), and run `pwsh
scripts/check-doc-tree.ps1` ALL-GREEN for any doc-touching change. For ecosystem seams, verify
the *contract end-to-end* (ai-feed capture → main recall round-trip; finance event → companion
aggregate) not just the unit.

**Review** — The ≥2-different-AI double-gate (`review-gate` skill): two distinct local AIs must
both APPROVE the diff before land. Land non-trivial changes only on consensus; CHANGES blocks.
Claude is the orchestrator + adversarial verifier + final judgment, not a counted reviewer.
Contract changes spanning two repos (main `login_broker` ↔ broker `/auth/cli/exchange`) gate on
review **in both** repos.

**Merge** — Branch-only, commit/push only when the operator asks; log each verdict to
`done/<uuid>.md`. Trivial mechanical ops are exempt from the double-gate. After merging a P0 main
change that moves the event/recall/capture contract, re-run the downstream producers' capture
round-trip and companion's aggregate before declaring the loop green — the keystone must stay
fed.
