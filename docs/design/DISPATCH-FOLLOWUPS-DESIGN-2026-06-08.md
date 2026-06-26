# Dispatch follow-ups — design + scope findings (2026-06-08)

Produced by a parallel design pass (read-only) after the `EventKind::Dispatch`
observability slice (`33dbf7d3`). **Both items below are bigger than "follow-ups"
— they need an owner decision, not an autonomous hotfix.** Captured here so the
work is ready when you choose to proceed.

---

## #2 — App ↔ broker dispatch path mismatch (the `/api/squad/dispatch` 404)

**Finding (the real reason it's not a hotfix):** the app and broker encode
**incompatible architectures**.
- **App** (`app/src-tauri/src/commands/dispatch.rs:410`) is a **data-plane**
  client: it POSTs to `{broker}/api/squad/dispatch` and expects the HTTP response
  body to be a live SSE token stream (`data: {"type":"token"|...}`).
- **Broker** (`phantommesh-io/src/routes/dispatch.ts`) is a **control-plane** by
  explicit design — it **never streams tokens**. `POST /api/me/dispatch/start`
  only writes a D1 `dispatches` row (status `pending`) and returns
  `{job_id, started_at}`. Tokens are meant to be produced by the SPA talking to
  localhost `phantom serve` directly, then PUSHed to `POST .../stream/:job_id`
  for cross-tab fan-out; `GET .../stream/:job_id` only *replays* pushed chunks.

So:
- **Approach A** (add `/api/squad/dispatch` to the broker) is impossible as a
  pass-through — nothing upstream of the broker generates tokens; it would emit
  zero frames and needs a prod redeploy of a just-un-bricked worker. ❌
- **Approach B / recommended** (app-only, **zero broker redeploy**): make
  `dispatch.rs` speak the broker's real two-call control-plane protocol
  (`POST /start` → `{job_id}` → `GET /stream/:job_id` SSE), with a **frame
  adapter** (DO emits `event: <kind>` + raw-text `data:`; app expects JSON `data`
  with a `type`). Also: add a required non-empty `peer` field (broker `/start`
  rejects empty peer), a `job_id↔dispatch_id` map, and **rewrite the existing
  test** `dispatch_commands.rs:110` (it hard-codes the old `/api/squad/dispatch`
  contract → would stay green against a contract the broker no longer matches =
  fake-green).

**⚠️ The dangling dependency (decision point):** even after B, **streaming tokens
still don't work** — the F205 producer (SPA→localhost→`publishChunk`) that feeds
`GET /stream` **does not exist** (`MobileDispatch.tsx` only *listens*). So B kills
the 404 and aligns contracts, but delivers status-only / empty streams until the
producer is built. **If the product intent is "broker proxies an agent and
streams tokens" (data-plane), that is a much larger broker+peer-protocol build**
— a deliberate architecture choice, not a 404 fix.

**Recommendation:** don't ship a half-fix that looks done but streams nothing.
Decide first: is the cloud/app dispatch path meant to be control-plane (SPA drives
localhost) or data-plane (broker proxies)? The CLI path (`phantom dispatch`)
already works end-to-end today and is the dogfoodable surface.

---

## #3 done note — for context

`#1` (plain-path dispatch persistence) + `#3` (serve tracing subscriber) are the
clean, shippable Rust follow-ups and are being committed separately.

---

## #4 — TaskTreeView (visualize tri-role subtask tree)

**Finding:** the tri-role task/subtask structure (`decompose → assign_subtasks →
DispatchOutcome[] → integrate`) exists **only in core** (`cluster_dispatch_wire`)
and is reachable only from the CLI (`phantom dispatch --tri`). **No existing data
source carries the tree** to a UI:
- the broker SSE wire carries opaque flat `{kind,data}` chunks (no subtask info);
- the persisted dispatch event is **lossy** (`persist_dispatch_event` stores only
  `IntegratedResult.markdown` + aggregate counts, discarding the per-subtask
  `SubtaskAssignment`/`DispatchOutcome` objects).

**Recommended v1 (local, no broker/SSE change):** a new Tauri command
`dispatch_tri` that calls the in-process orchestrator (`RpcRunner`/`execute_plan`)
and returns a structured `TriDispatchReport` (subtasks + outcomes), rendered by a
new `TaskTreeView.tsx` + `dispatchTreeStore.ts`. Honest constraints:
- **blocks until all subtasks finish** (no streaming) — mitigate by rendering a
  skeleton from a pure `dispatch_decompose(prompt)->Subtask[]` preview first;
- **local-cluster only** — needs reachable `phantom serve` peers; on a phone with
  no cluster every subtask is `NoCandidate` (surface as cards, not a crash);
- needs a **ts-rs export re-run** for the new `TriDispatchReport` type;
- watch `bigint` fields (`startedAtMs`/`completedAtMs`/`totalLatencyMs`) in the
  generated TS — `Number()`/null-guard or React crashes.

**Scope:** a genuine feature slice (new command + component + store + codegen),
not a small add. Live per-subtask progress on the cloud path = the "bigger slice"
(real `/api/squad/dispatch` + subtask-aware DispatchStream DO + new SSE frames +
broker deploy) — explicitly out of v1.

---

## Net recommendation

Ship the clean Rust follow-ups (#1, #3) now. Treat #2 and #4 as **owner decisions**
about dispatch architecture (control-plane vs data-plane; local-tri vs cloud-stream)
— full diff-level plans exist in this session's workflow output
(`wf_efa4de30-c5c`) and can be executed once the direction is chosen.
