# phantom-mesh (main) — apex-grounded roadmap (FINAL, 2026-06-17)

> 📍 **Provenance, not SSOT (2026-06-21)**: the roadmap source-of-truth is `docs/OPERATING-STANDARD.md §3`. This file is the apex-grounded detail/derivation that fed it — keep it for the per-item exit criteria + test strategy, but when status conflicts, §3 wins.
> 🔄 **DRIFT**: the "② owned memory is still `unimplemented!()`" framing below is **stale** — `embedding_search()`/`skill_store()` are implemented (`core/src/skill_wire.rs:1372/1836`), recall is wired into the agent loop, and the `phantom skill` CLI shipped. The remaining ② gap is the semantic `ort` leg (deferred / human-led). See `main-P0-owned-memory/DECOMPOSITION.md`.

Pipeline: **codex draft (×2 rounds) → agy review (×2 rounds) → Claude finalize.**
Anchored to `docs/superpowers/BIG-GOAL.md` (FINAL re-lock 2026-06-11). Governance: this
roadmap is subordinate to BIG-GOAL; every item traces to a pillar/ability + a SPEC-NN (§10).

## Apex lock (the only thing that doesn't move)

> **A private AI that is only mine — the more I genuinely use it, the more it compounds my advantage.**
> moat = **I actually use it every day.**

**Ordering rule** (from the apex, not from what's easy to build):
1. **② Owned compounding memory FIRST** — the #1 ability. *(2026-06-17 wording — see the DRIFT banner at top: `embedding_search()`/`skill_store()` are now implemented; the residual gap is the semantic `ort` leg, deferred.)*
2. **④ Safe unattended work SECOND** — *partially built tonight* (L0 `cli_session` + L1 `governed_run` + the 4-node governed cluster). The differentiator: bounded, owner-governed, **phone-escalated** — NOT fire-and-forget.
3. **① Life + code capture** feeds ②, but stays **consent-gated**.
4. **⑤ Life/work synthesis** ships **reactive-only** until ② is trustworthy.
5. **③ Real-time proactive nudges** stay **DEFERRED** until memory earns the right to interrupt.

## Priority roadmap

| P | Item | Ability | SPEC | Exit criteria | Test strategy |
|---|---|---|---|---|---|
| **P0** | **skill bank owned-memory loop** — ✅ `embedding_search()`/`skill_store()` implemented + store→recall→apply→measure wired (see top DRIFT banner); **remaining** = semantic `ort` leg (deferred) + loop runner/scheduler/UI + dogfood. *(original 2026-06-17 wording: "implement the two fns")* | ② | 25,16,12,13 (+14 fallback) | A correction/repeated workflow becomes a stored skill, survives restart, is recalled in a later task, can be declined, updates quality. | unit: schema/migration/FTS5 + **panic-gate to FTS5 when semantic unavailable**; integration: 6-step fixture loop; scenario SPEC-61 S26–S30; gate SPEC-60 V2/V3/V4. |
| **P0** | **Safe unattended work spine** — bounded async runner, execution contracts, approval ledger, budget/time/electricity stops, signed flight-recorder. **(Built: L0/L1 governed cluster — harden + wire.)** | ④ | 10,12,13,16 (NOT 26/27 — those are P1 mesh) | Long task runs without live staring, pauses at high-risk/tool/cost boundaries, records tamper-evident history. | unit: classifier + contract fingerprint; integration: deny-until-approved runner; e2e: block→approve/deny→resume; security: fail-closed auth. |
| **P0** | **Phone hard-brake (approval slice)** — the remote approve/redirect/stop UI for the spine above. *(Pulled up from P1 per agy R2: ④'s safety loop is untestable without the phone gate.)* | ④,① | 24,30,17 | Phone receives a high-risk/budget escalation and can approve / redirect / STOP a bounded run; fail-safe Deny on no-channel. | Tauri bridge contract test; notify→inbox→decision e2e; real-device manual checklist. |
| **P0** | **Provider/BYOM failover (deterministic)** — the LLM routing + fallback the memory loop depends on. *(Pulled up from P1 per agy R2: P0 memory loop stalls without robust fallback.)* | ②,④ | 14,04,07 | A provider failure degrades predictably (breaker → next provider), surfaced via the error catalog; local models participate where present. | provider fixtures; breaker behavior; error-catalog assertions; live opt-in smoke. |
| **P0** | **Reactive daily loop** — capture life/work facts → daily review → memory improves tomorrow's plan. No GPS nudges. **External delivery (Telegram/email) gated behind local-store integrity** *(agy R2: protect PII).* | ①,②,⑤ | 20,21,22,23,24,25 | Scheduled/user-triggered review references captured events + recalled prefs; shame-free, non-diagnostic; delivery only after at-rest seal. | golden review fixtures; prompt lint; event-store round-trip; **mockable system clock** for streak/daily-transition; SPEC-61 S11–S25. |
| **P0** | **Single-backend completeness** — phone + ONE backend works end-to-end (no mesh required). | ①,④ | 28,30,33,17,10 | Fresh install connects to a backend, talks, captures, reviews, supervises a task; **first-run local ed25519 identity works with broker unreachable (SYS-B)**. | 30s-hello gate; mobile smoke; backend-URL persistence; no hardcoded fleet data. |
| **P0∥** | **Encryption closure (concurrent)** — seal `conversations/` + `memory.db` *alongside* the P0 memory loop, not after. *(agy R2: avoids schema-migration rework.)* | ④(P4) | 13,16,15,08 | New P0 stores are encrypted from day one (`EventStore::with_key`); no encrypted→plaintext regression. | at-rest e2e; wrong-key fail-closed; secret/PII grep; delete/wipe scenario. |
| P1 | **Mesh as compounding upgrade** — capability-aware routing across owned nodes, single decision line, no swarm pitch. | ④,⑤ | 10,11,26,27 | A task routes to the best owned node by capability/load; one coordinator owns decisions. | cross-host smoke; idempotency; peer-capability fixtures; failure/retry. |
| P1 | **Full mobile supervisor surface** — beyond the P0 approval slice: task state, capture, history. | ①,④ | 30,31,32,33,34 | Phone is the full remote control, not a shell. | bridge contract tests; push/notification fallback; real-device checklist. |
| P2 | **Zero-knowledge cloud backend** — hosted Linux sandbox, only after local encryption + safe-work spine are real. | ④ | 15,50,51 | Cloud runs encrypted artifacts with no plaintext operator access. | threat-model review; sealed-payload tests; no plaintext server logs. |
| P2 | **Release evidence system** — every shipped claim has a green gate / scenario / DRIFT note. | all | 29,60,61,62 | No public claim without proof. | V1–V12 gate matrix; S1–S40; five-platform smoke. |
| P2 | **Spec-index reconcile** — fix index drift (e.g. SPEC-46 windows-cli on disk but absent from SPEC-00-INDEX). *(agy R2.)* | gov | 00 | SPEC-00-INDEX matches on-disk spec leaves; DRIFT markers tracked. | `pwsh scripts/check-doc-tree.ps1` green. |

## Shortest path to "usable daily" (the near-term cut)

Do, in order — anything not serving these is drift unless it unblocks their tests:
1. **P0 owned-memory loop** (②) — ✅ the two fns implemented + the 6-step per-step impls wired (see DRIFT banner); **remaining** = semantic `ort` leg (deferred) + loop runner/scheduler/UI + dogfood.
2. **P0 safe-work spine + phone hard-brake** (④) — harden tonight's L0/L1 cluster + wire the phone approval slice (they ship together; the spine isn't real until the phone can stop it).
3. **P0 provider failover** (②/④) — so the memory loop doesn't stall.
4. **P0 reactive daily loop** (①②⑤) + **encryption closure concurrent** — local-store integrity before any external delivery.
5. **P0 single-backend completeness + SYS-B offline identity** — fresh install usable with broker down.

## Explicit deferrals (apex says NO, near-term)
- **③ real-time GPS/sensor nudges** — until ② earns interruption rights.
- **Multi-agent/persona collaboration as a pitch** — mesh is multi-device, single decision line (MAST/Cognition evidence).
- **Paid broker / SaaS shaping priorities** — commercialization is downstream (§7), never roadmap-driving.
- **Cross-user skill sharing / marketplace** — violates owned-private-memory boundary.
- **Background ambient capture by default** — violates consent-gated capture (§6).

## Test policy (every item ships with)
SPEC trace · characterization-first · unit (no net/device) · integration (real storage/RPC/bridge) · scenario (SPEC-61) · ship-gate (SPEC-60 V1–V12) · **dogfood proof (owner-usable, not demo-only)** · two-lane review (no solo self-approve).
**Enabling test primitives to build first** (agy R2): a **mockable system clock** (coach streak/daily-transition, SPEC-23) and **sync/latency telemetry** (the 5s SLO).

---

## Finalization notes (what I changed from the codex draft + agy review)
- **Adopted all 7 of agy's round-2 critiques.** Pulled the **phone approval slice** and **provider failover** up into **P0** (the ④ safety loop is untestable without the phone gate; the ② memory loop stalls without deterministic fallback). Made **encryption closure concurrent** with the P0 memory loop (avoids schema rework). Gated **external delivery behind local-store integrity**. Added **mock-clock + sync-telemetry** test primitives and a **spec-index reconcile** P2 item.
- **Fixed the SPEC mapping drift:** removed SPEC-26/27 (multi-device orchestration = P1 mesh) from the **P0** safe-work spine — the P0 runner is local/async, not cluster.
- **Kept codex's strong skeleton** (apex-ordered priority table + per-item exit criteria + the deferrals + the 8-point test policy).
- **Apex grounding I enforced:** ② is unambiguously the #1 P0 (the two fns — now implemented, see DRIFT banner — were named here); ④ is framed as bounded/owner-governed/phone-escalated (NOT fire-and-forget); ③ stays deferred; downstream (作品→副業) explicitly does not shape priorities.
- **Honest gap:** codex's round-2 couldn't read DRAFT1/REVIEW1 locally (Windows sandbox `CreateProcessAsUserW 1312`) so DRAFT2 is an apex-aligned rewrite, not a line-by-line revision — but it re-read BIG-GOAL via MCP and round-1's feedback is reflected through agy's round-2 pass, which I folded in here.
