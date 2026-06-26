# FINAL Development Plan — phantom-mesh ("main")

> Synthesis of codex DRAFT + agy REVIEW, re-grounded against actual code on branch
> `feat/platform-flows-design-fixes`. agy's review was **usable and largely correct** — it
> caught that the draft over-scoped post-cut work into P1 and listed already-done tasks.
> Every claim below was re-verified by reading the cited files.

## What it is + current state
Phantom Mesh is a local-first private AI agent mesh: a single Rust binary (daemon + CLI) plus a
Tauri/React app, with encrypted Life Track event capture, daily coach review, recall/search,
provider fallback, and HMAC-authenticated mesh RPC. Broad and usable for v0.6.0 exploration but
**pre-stable**: the v0.6.0 cut is gated on app-facing honesty (no mock/leak surfaces), a clean
build, and the four systemic design correctives (SYS-A..D). Compounding owned memory (②) and
durable unattended dispatch (④) are **explicitly post-cut** per
`plans/cut-backlog-worklist-2026-06-12.md` — do **not** start them before the cut.

Verified ground truth (changes the plan):
- `core/src/skill_wire.rs:888 embedding_search` and `:1128 skill_store` **already return typed
  `Err`, not `unimplemented!()`** ("v0.6 GA floor" comments + the assertion at `:1469`). The
  draft's and agy's panic-gate ask is **already done** — keep only a regression-test/verify task.
- `app/src/components/mobile/AppTemplate.tsx` IP/secret leak is **already fixed** (`DEFAULT_SECRET=""`,
  no hardcoded IP map; ANDAPP-LEAK-002 closed). Drop it from P1 — agy was right.
- Still live: `SecurityPanel.tsx:36/187/223/242` `MOCK_EVENTS`; `setup-cloud-linux.sh` bad verb +
  hardcoded tailnet IPs; app build never re-verified this session.

## Prioritized next builds

- **P1 — Cut-blockers (finish v0.6.0 honesty floor):** replace `SecurityPanel` `MOCK_EVENTS` with
  real audit/flight-recorder data + honest empty state; fix `setup-cloud-linux.sh` verbs/IPs;
  green a full `npm run build` + `tsc` + `cargo test -p` skill_wire panic-gate regression.
- **P1 — SYS-B offline bypass:** first-run local ed25519 identity → single-machine usable when
  broker/OAuth is unreachable (removes the login-first hard dead-end across mac/win app + CLIs).
- **P1 — SYS-C / SYS-D fail-safe + reversibility:** unattended runs default to **safe-pause** when
  the supervisor device is unresponsive; add GUI/CLI off-switches (`coach uninstall-schedule`, fix
  the broken `service uninstall` that `taskkill /F`-es all phantom.exe, GUI "delete my data").
- **P2 — SYS-A planned/as-built split:** mark `/rpc/task/resume|approve`, push channel, and
  `awaiting_approval` UI as `[PLANNED v0.7]` (dashed) in all six flow diagrams so implementers stop
  treating goal-state as a built contract.
- **P2 — Life Track day-0 UX:** honest empty states for `review`/`coach review`/`recall`
  ("no events yet — capture your first with `phantom note ...`"); README 30-second local-first path.
- **P2 — Capture never fails on enrichment:** persist the event first (exit 0), make Gemini analysis
  async best-effort (`analysis: pending` + `phantom event reanalyze <id>`) — today an analyze
  failure exits 1 and loses the event (win-CLI capture death path).
- **P3 — perf pass (post-cut):** move `serve.rs` blocking `std::fs` off the Tokio hot path
  (`:526/1021/1347/3798/3870` → `tokio::fs`/`spawn_blocking`); `session.rs:71` global async Mutex →
  RwLock/DashMap; `cost.rs:128` async Mutex → sync.
- **P3 — ② owned memory + ④ durable dispatch (post-cut):** real `skill_store`/`embedding_search`
  + `0008_hermes_skills.sql`; durable task lifecycle + `/rpc/task/resume` + phone cancel UI.
  **Do not start before the v0.6.0 cut** (was wrongly P1 in the draft).

## Top 3 breakdown

### 1. P1 — Cut-blocker honesty floor
- `SecurityPanel.tsx`: replace `MOCK_EVENTS` (lines 36/187/223/242) with a real audit/flight-recorder
  fetch; when no data, render an honest empty state instead of fake events (incl. the
  `state.isOffline ? MOCK_EVENTS` fallback at `:242`).
- `scripts/setup-cloud-linux.sh`: replace the invalid `phantom run --node` (not in KNOWN_SUBCOMMANDS,
  ~`:243`) with current verbs (`peer assign`/`send-async`); replace hardcoded `100.64.0.10-13`
  (`:149-152`) with prompts/placeholders.
- Run and green `npm run build` + `tsc` in `app/`, and `cargo test -p <core> skill_wire` to lock the
  already-landed panic-gate (the `:1469` GA-floor assertion) against regression.
- Smoke the README path: `phantom serve`, a capture verb, `phantom coach review`.

### 2. P1 — SYS-B offline bypass
- Add a first-run path that mints a local ed25519 identity and reaches a usable single-machine
  `serve` + `set_provider` with **zero** broker/OAuth round-trip.
- Demote cloud account binding to a post-onboarding Settings action across mac-app/win-app and
  mac/win CLI; sign-in failure copy gets an "offline continue (local only)" exit.
- Decouple `ONBOARDED_KEY` from SelfCheck: temporary provider degradation must NOT re-trigger
  onboarding (only a missing identity key does).
- Tests: fresh-install with broker unreachable still reaches a working local chat.

### 3. P1 — SYS-C/SYS-D fail-safe + reversibility
- Governor default = **safe-pause** (not run-on, not infinite-hold) when the supervisor phone is
  unresponsive past a timeout; freeze billing; persist to flight-recorder; surface "paused — no
  response" on next app open.
- Add a lifetime-level escalation/budget cap (撞頂 → forced `Cancelled`) so reaper auto-resume +
  governor escalation can't become unbounded push.
- Symmetric off-switches: `coach uninstall-schedule` (unload+delete unit); fix win `service uninstall`
  so it stops only its own task with a printed plan, not `taskkill /F` of every phantom.exe.
- GUI reversibility: Settings → "delete all my data" wired to the existing kill-switch, with copy
  distinguishing logout (keeps data) vs delete (irreversible).

## Changes from draft
- **Cut from P1:** R1 owned memory and R2 durable dispatch — both are post-cut per the cut-backlog
  worklist; promoting them to P1 was the draft's main sequencing error (agy flagged this correctly).
  Moved to P3 with an explicit "do not start before cut" gate.
- **Cut entirely:** the AppTemplate IP/secret-leak task (already fixed on this branch — verified) and
  the standalone skill_wire panic-gate task (already landed; kept only as a regression test).
- **Added:** the three systemic correctives SYS-B / SYS-C / SYS-D as P1, SYS-A diagram split as P2,
  the capture-never-fails-on-enrichment fix, and an explicit `npm run build`/`tsc` verification gate —
  all from agy's review and the design-soundness worklist, none in the draft.
- **agy review usability:** usable and accurate. It correctly identified the over-scoping, the
  already-done AppTemplate task, the missing SYS-A..D axes, and the missing build gate. Its one
  imprecision — implying the skill_wire panics still need gating — is now stale (they're already
  typed-`Err`); corrected above.
