# CROSS-REVIEW-AUTOMATION — multi-AI cross-review as a core merge gate

> **Stage:** 1 (design only — no code changes in this branch).
> **Status:** spec, awaiting Stage-2 wire-up.
> **Authority chain:** subordinate to [`docs/_archive/NORTH-STAR.md`](../_archive/NORTH-STAR.md) and [`docs/_archive/MASTER-SPEC.md`](../_archive/MASTER-SPEC.md). Answers `MASTER-SPEC §5 Q7` (quorum is undefined — line 263) and concretizes the `ACCEL-REVIEWER-ROLE` row of `MASTER-SPEC §3.5` (line 147) plus `ACCEL-MULTI-AI-CROSSREVIEW` (line 144).
> **Prerequisite for:** un-gating `ACCEL-SELF-IGNITING-LOOP` (`MASTER-SPEC §3.5` line 149) — the §5 BLOCKER decision (lines 250–255) explicitly defers self-ignition "until ≥2-AI cross-review is automated into core."

---

## 0. Why this exists

`MASTER-SPEC §3.5` (the acceleration framework table) ships **three** rows that today can only run as an external thin layer because the core has no Reviewer concept:

1. `ACCEL-REVIEWER-ROLE` — *"Reviewer agent role (read-only diff inspection, non-author verify → consensus + cross-platform green = merge gate); extends DispatchRole"* (status: **designed**, prio P3, `../_archive/MASTER-SPEC.md:147`).
2. `ACCEL-MULTI-AI-CROSSREVIEW` — *"≥2 AI consensus gate ...; native integration pending P3 — **the key missing trust mechanism**"* (status: **partial**, prio P2, `../_archive/MASTER-SPEC.md:144`).
3. `ACCEL-SPEC-GATE` — *"`phantom dev spec validate`: spec must declare which of 4 abilities + which MVP component"* (status: **designed**, prio P3, `../_archive/MASTER-SPEC.md:146`).

`MASTER-SPEC §5 Q7` (line 263) calls this out as the **trust mechanism** for moving from supervised stage-1 to the unattended self-igniting loop:

> *"Multi-AI consensus quorum is undefined. 'All available agents agree' specifies no quorum/tiebreaker/offline handling (blocks P2–P3 Reviewer role + is the trust mechanism for un-gating the self-igniting loop). Define it."*

This document answers that question and the surrounding gaps, end-to-end, without touching code.

---

## 1. Real surfaces this spec extends (cited)

This is a design — every claim must hook into a real surface that already exists in `core/`. The four anchor surfaces, with exact line citations:

### 1.1 `DispatchRole` enum + role-keyword routing
**File:** `core/src/cluster_dispatch_wire.rs`
- `DispatchRole` is defined at `cluster_dispatch_wire.rs:360-367` with variants `Master`, `Coder`, `Researcher` (SPEC-26 §6.2 tri-role).
- `Subtask` (one role-targeted unit) at `cluster_dispatch_wire.rs:373-380`.
- `role_required_caps()` (the per-role capability-tag bundle) at `cluster_dispatch_wire.rs:419-432` — currently maps `Coder → ["role-coder","cargo","git"]`, `Researcher → ["role-researcher","webSearch"]`, `Master → []`.
- Pure keyword routing in `decompose()` at `cluster_dispatch_wire.rs:440-468`, driven by `CODER_KEYWORDS` (line 388) and `RESEARCHER_KEYWORDS` (line 395).
- Capability scoring + peer selection in `plan_dispatch()` at `cluster_dispatch_wire.rs:959-1024` (§6.2 weighted-sum: `cap_match × 0.5 + latency × 0.3 + load × 0.15 + penalty × 0.05`, with `top.score < 0.1 → NoMatchingPeer` at line 991).
- HMAC POST + status poll + one-hop fallback in `execute_plan()` at `cluster_dispatch_wire.rs:1043-1125`.
- Pluggable per-subtask runner: `SubtaskRunner` trait at `cluster_dispatch_wire.rs:629-631`; the parallel-fan-out orchestrator `run_dispatch_with()` at `cluster_dispatch_wire.rs:641-670`; the production binding `RpcRunner` at `cluster_dispatch_wire.rs:713-726`.
- Outcome integration in `integrate()` at `cluster_dispatch_wire.rs:556-611` — counts `Completed` vs `Failed|Timeout|NoCandidate`, produces a stable markdown summary, computes a parallel wall-clock span.

### 1.2 `dev_verify` — the existing single shared "is it green?" gate
**File:** `core/src/tools/diagnostic.rs`
- `dev_verify` doc-comment + tool entrypoint at `diagnostic.rs:608-627` and `diagnostic.rs:627-730`.
- Returns the structured verdict shape `{passed, exit_code, summary, failed, warnings, log_path, command, ...}` (line 611–613). `passed` is the process's real `exit_code == 0` (line 613) — *"a tool-returned fact, not an agent's claim."*
- Modes (line 617–623): `shell`, `background` (returns `{job_id}` at line 708–714), `job` (poll an existing job at line 629), `remote` (HMAC-signed dispatch to a peer at line 645–646).
- Remote variant `run_remote_verify` builds an HMAC-SHA256 body signature matching the legacy body-HMAC scheme the server's `require_cluster_auth_dual` accepts (line 734–735).
- This is the existing primitive every Reviewer vote MUST go through — see §4 below.

### 1.3 `partner.rs` — origin handling + the anti-pollution wall
**File:** `core/src/partner.rs`
- `Intent` enum at `partner.rs:45-49` (`Record { body }` and `Ask`), with the pure detector `detect_intent()` at `partner.rs:57-79`.
- `signals_path()` at `partner.rs:115-125` — the JSONL ledger location, overridable by `PHANTOM_PARTNER_SIGNALS` (line 116–119), defaulting to `~/.phantom-mesh/partner-signals.jsonl` (line 121–124).
- `record_signal()` at `partner.rs:147-151` — the canonical append helper used by every entry point.
- `handle_message()` reactive entry point at `partner.rs:193-227` — the place the partner currently routes inbound text (one of two existing client-agnostic entry points the Reviewer role must NOT contaminate).
- `record_location_behavior()` at `partner.rs:165-179` — typed wrapper for proactive sensor signals.

The MASTER-SPEC `MessageOrigin` / `record_signal_with_origin` machinery (`§3.5` line 139 `ACCEL-ANTIPOLLUTION-WALL`, line 152 `ACCEL-DETECTION-HEURISTICS`, line 160 `STATE-DEV-LOOP-LOG-JSONL`) is the **policy intent** that any Reviewer-emitted signal MUST be routed to the dev-loop ledger, not partner-signals. As of this writing those helper names are not yet present in `partner.rs` (line range 1–951) — Stage 2 of THIS spec will add them at the same time it wires the Reviewer. Cross-review traffic is, by definition, machine-origin: it can never count as moat evidence.

### 1.4 `../_archive/MASTER-SPEC.md` §3.5 + §5 Q7
**File:** `docs/_archive/MASTER-SPEC.md`
- §3.5 acceleration-framework table at `../_archive/MASTER-SPEC.md:134-162`.
- §5 Q7 — quorum question — at `../_archive/MASTER-SPEC.md:263`.
- The BLOCKER paragraph deferring self-ignition until cross-review is automated into core at `../_archive/MASTER-SPEC.md:250-255`.

---

## 2. (a) The Reviewer role — extending `DispatchRole`

### 2.1 Wire-level change
Add one variant to the existing `DispatchRole` enum at `cluster_dispatch_wire.rs:360-367`:

```text
enum DispatchRole { Master, Coder, Researcher, Reviewer }   // ← Reviewer is new
```

Because `DispatchRole` is `#[derive(Serialize, Deserialize, TS)]` with `#[serde(rename_all = "snake_case")]` (line 362) and is exported to `app/src/lib/generated/cluster_dispatch/` (line 361), the wire string is the deterministic `"reviewer"` and the TypeScript binding regenerates as a free side-effect of the next `cargo build`. No transport change.

### 2.2 Capability tags
Extend `role_required_caps()` at `cluster_dispatch_wire.rs:419-432` so that a Reviewer subtask requires a peer to advertise:

```text
DispatchRole::Reviewer => &["role-reviewer", "git", "dev-verify"]
```

- `role-reviewer` — peer opted in to the reviewer pool. Required so a Coder-only node is never asked to vote on its own diff.
- `git` — Reviewer needs to fetch + diff the branch (read-only). Same tag the Coder role already requires.
- `dev-verify` — peer has the `dev_verify` tool registered (every node that registers the MCP `dev_verify` tool gets this tag automatically; see `core/src/mcp.rs::to_mcp_tool` referenced by `MASTER-SPEC §3.5:156 INFRA-MCP-INTERFACE`).

`plan_dispatch()` (line 959) already filters out peers missing any required cap (lines 965–975) before scoring, so a Coder-only or unverified peer can never be selected as a Reviewer. The same `top.score < 0.1` cutoff (line 991) keeps the pool honest.

### 2.3 Subtask payload
The existing `Subtask` (line 373–380) already carries `prompt: String + required_caps`. For a Reviewer subtask, the `prompt` field carries a structured review-request blob (JSON-as-string, matching the opaque-payload cycle-break note at `cluster_dispatch_wire.rs:32-38`):

```text
{
  "kind": "review_request",
  "branch": "dev/cross-review-spec",
  "head_sha": "<author-machine commit SHA>",
  "base_sha": "<merge-base against main>",
  "diff_uri": "git://<author-peer>/<branch>",   // fetched read-only by the Reviewer
  "verify_command": "cd core && cargo test --no-run && cargo test",
  "author_peer_id": "<peer id of the AI that produced the diff>"
}
```

No schema change to `DispatchTask` (line 134–154) is needed — payload stays the opaque `String` Stage-1 chose at line 144–151 deliberately to preserve the cycle break with SPEC-27.

### 2.4 Decomposition
`decompose()` at `cluster_dispatch_wire.rs:440-468` is rule-based and pure. It does NOT need to emit Reviewer subtasks itself — Reviewer dispatch is triggered by the merge gate, not by user prompts. Specifically: the merge gate (described in §4) constructs Reviewer `Subtask`s **directly** via `assign_subtasks()` (line 495–533), bypassing `decompose()`. Rationale: a user prompt of *"please review this"* must go to the Coder/Researcher path that the user expects; turning user prompts into Reviewer votes would conflate human intent with the merge gate.

### 2.5 Read-only invariant
The Reviewer agent is invoked as a normal subtask via `RpcRunner` (line 713–726) → `execute_plan` (line 1043) → `/rpc/task/assign` (line 1060). On the **executing** peer, the agent receives the review-request blob and is constrained to:

- Read the diff (`git fetch + git diff base_sha..head_sha`).
- Run `dev_verify` (`diagnostic.rs:627`) against the supplied `verify_command`. The returned JSON is the **only** machine-trusted signal.
- Emit a structured `ReviewVote` (see §3.1) as the `DispatchOutcome.result_summary` (line 220). Vote is `Approve | Reject | Abstain` + 1-line reason.

The Reviewer MUST NOT push commits, MUST NOT call `git_add`/`git_commit`/`git_checkout`, MUST NOT mutate the partner-signals ledger. Stage 2 enforces this with a process-level capability gate on the agent runtime (separate from the wire). For Stage 1 (this spec) the constraint is normative: any vote produced by a peer that mutated the working tree is treated as `Abstain` by the quorum tallier (§3.4).

---

## 3. (b) Non-author second-AI review of a diff before merge

### 3.1 Vote shape
The Reviewer emits a `ReviewVote` serialized into `DispatchOutcome.result_summary` (`cluster_dispatch_wire.rs:220-221`, which is `Option<String>` capped at ≤ 256 chars per the existing comment). Shape:

```text
ReviewVote {
  task_id: String,                  // mirrors the Reviewer Subtask id (assign_subtasks line 509)
  voter_peer_id: String,            // executed_by_peer_id (line 209)
  decision: "approve"|"reject"|"abstain",
  verify_passed: bool,              // = dev_verify verdict.passed
  verify_exit_code: i32,
  verify_log_path: String,          // dev_verify verdict.log_path
  reason: String,                   // ≤ 200 chars; human-readable, machine-line-grepable
  reviewed_head_sha: String,        // proves what was actually reviewed
}
```

The 256-char cap is conservative but real: the existing comment on `result_summary` (line 220) says "Short human-readable summary for UI (≤ 256 chars). ... full ... still wr[itten] into SPEC-16 events row." `verify_log_path` is the pointer to the long form. The full vote JSON also lands in the SPEC-16 audit ledger (`MASTER-SPEC §3.6 EVENT-STORE` line 185) for post-hoc audit — the wire summary is just the digest.

### 3.2 Non-author selection rule
The merge gate constructs Reviewer subtasks via `assign_subtasks()` (line 495–533) using a **filtered peer set**: the input slice `peers: &[PeerCapabilities]` (line 498) is the current process-local `PeerRegistry` cache (`peer_registry()` at `cluster_dispatch_wire.rs:893-896`) **minus the author peer**.

Concretely, the gate computes:

```text
let reviewer_pool: Vec<PeerCapabilities> = all_peers
    .into_iter()
    .filter(|p| p.peer_id != author_peer_id)   // non-author invariant
    .collect();
```

Then `plan_dispatch()` (line 959) is invoked once per desired vote. Because `plan_dispatch` ranks by `score_peer` (line 757) and the top scorer + fallback chain are all distinct (the filtering at line 965–975 + the dedup-by-id implicit in the `Vec`), two successive votes pick distinct peers as long as ≥ 2 non-author peers advertise `role-reviewer`.

If only **one** non-author peer advertises `role-reviewer`, the gate emits a `NoMatchingPeer` outcome for the second vote (existing `DispatchError::NoMatchingPeer` at `cluster_dispatch_wire.rs:333-336`), which `dispatch_error_to_outcome` (line 684–706) folds into a `NoCandidate` terminal — and the quorum rule (§3.4) handles it as an offline reviewer.

### 3.3 How many votes does the gate request?
**Two votes (the floor).** The MASTER-SPEC ACCEL row uses *"≥ 2 AI consensus"* (line 144); this spec fixes the floor at exactly 2 for stage-1 supervised and stage-2 native. Three is supported but not required.

Rationale: requesting two non-author votes is the **smallest** form that still answers the §0 anchor question ("did a second AI verify?"). Requesting three would block any 2-peer cluster (`dev-host + author`) and is therefore disallowed at the floor.

The gate MAY request a third vote as a **tiebreaker only** — see §3.4.

### 3.4 (c) Quorum rule — answering `MASTER-SPEC §5 Q7`

> **MASTER-SPEC §5 Q7 (line 263):** *"Multi-AI consensus quorum is undefined. 'All available agents agree' specifies no quorum/tiebreaker/offline handling..."*

This spec defines it:

**Quorum (the floor, answers Q7):**

```
MERGE IFF
    ( votes_received >= 2 )
  AND ( approve_count >= 2 )
  AND ( reject_count == 0 )
  AND ( every approving vote has verify_passed == true )
  AND ( every approving vote has reviewed_head_sha == head_sha_at_merge )
```

All four conjuncts are necessary. Spelled out:

1. **`votes_received ≥ 2`** — there is **no** single-reviewer path. A 2-peer cluster (author + one other) can NOT self-merge; the gate degrades to "supervised stage-1" and a human merges. This is intentional and consistent with the stage-1 supervised note at `../_archive/MASTER-SPEC.md:55-56` and the §5 BLOCKER deferral at `../_archive/MASTER-SPEC.md:250`.
2. **`approve_count ≥ 2`** — at least two distinct non-author reviewers cast `Approve`. Two `Approve + zero Reject + zero Abstain` is the green path.
3. **`reject_count == 0`** — a single `Reject` blocks the merge unconditionally. There is **no** outvoting a reject. Reject = "I saw something wrong." A reject can only be cleared by amending the diff and re-running the gate (which produces a new `head_sha` and invalidates all prior votes per conjunct 5).
4. **Every approving vote's `verify_passed == true`** — an `Approve` whose `dev_verify` exit code is non-zero is a **bug** in the Reviewer agent (the prompt forbids approving red). The tallier treats such a vote as `Abstain` and logs the discrepancy to the dev-loop ledger.
5. **`reviewed_head_sha == head_sha_at_merge`** — guards against the race where the author force-pushes between vote collection and merge. If any approver reviewed an older SHA, all votes are invalidated and the gate restarts.

**Tiebreaker (decides ambiguous outcomes):**

The only ambiguous outcome under §3.4's rule is `1 Approve + 1 Abstain` (because `Reject` blocks unconditionally and `0 Approve` blocks for failing conjunct 2). When the initial 2 votes return `1 Approve + 1 Abstain`, the gate dispatches **exactly one additional Reviewer subtask** to the highest-scoring not-yet-used non-author peer. Then:

- `2 Approve + 1 Abstain` → MERGE.
- `1 Approve + 2 Abstain` → BLOCK (insufficient evidence). Human escalation; treated as `MASTER-SPEC §3.5:148 ACCEL-WORK-QUEUE` returning the task to the queue.
- Any `Reject` at any point → BLOCK (conjunct 3 still holds).

The tiebreaker fires **at most once per gate invocation**. A second tiebreaker would risk turning the gate into a polling loop that drains the cluster.

**Offline-agent handling:**

A reviewer peer that fails to return a vote within the existing `DispatchTask.deadline_ms` (`cluster_dispatch_wire.rs:153`, default 90 000 ms per `cluster_dispatch_wire.rs:153`) yields a `DispatchStatus::Timeout` outcome (line 311). The tallier treats Timeout exactly like `Abstain` — the peer is offline-equivalent. Failure to count Timeouts toward Reject is deliberate: a slow-network peer is not a Reject signal.

A reviewer peer whose `DispatchOutcome.status == Failed` (the executing peer reported a task-level failure, line 309–310) is also treated as `Abstain` and the failure is logged to the dev-loop ledger. Repeated failures lower the peer's `failures_last_5_min` (`failure_history` at `cluster_dispatch_wire.rs:935-945`), which already feeds the `recent_failure_penalty` term of the §6.2 scorer (line 280–283), so a flapping Reviewer naturally drops out of the pool.

A reviewer peer that returns `DispatchStatus::NoCandidate` because **no** non-author peer advertised `role-reviewer` (the single-peer edge case in §3.2) yields `votes_received < 2` and the gate blocks at conjunct 1 → human merge.

**Author-self-vote ban:**

The author peer is filtered out of the pool in §3.2 *before* `plan_dispatch`, so the wire cannot route a vote to the author. As a defense in depth, the tallier rejects any vote whose `voter_peer_id == author_peer_id` regardless. A Reviewer subtask that lands on the author (e.g. a misconfigured cluster where two peers share an id) is logged as `voter_peer_id_collision` to the dev-loop ledger and counted as `Abstain`.

### 3.5 Why these rules
The four conjuncts of the floor (§3.4) plus the offline-equivalent rule together answer **all three** parts of §5 Q7: quorum (≥ 2 distinct non-author approvals), tiebreaker (one extra vote on the unique ambiguous case), offline handling (Timeout = Abstain, never Reject; flapping reviewers drop out via the existing failure-window penalty).

---

## 4. (d) Composition with `dev_verify` and `--no-ff` branch merges

### 4.1 Composition with `dev_verify` (`diagnostic.rs:627`)
The Reviewer agent does NOT redo `dev_verify` from scratch — it **calls** `dev_verify` as the single shared "is it green?" primitive that the rest of the codebase already trusts. Three concrete bindings:

1. **In-band:** the Reviewer agent invokes `dev_verify` via MCP (`MASTER-SPEC §3.5:156 INFRA-MCP-INTERFACE`) with `{"command": review_request.verify_command, "path": "<workspace>", "remote": "<author_peer_id>"}`. The `remote` mode (`diagnostic.rs:645-647`) runs the command in the **author's** context — this proves the diff compiles on the platform/toolchain the author actually shipped from, eliminating the "works on my mac" trap. If the cluster spans multiple OSes, the gate MAY dispatch a second `dev_verify` to a non-author non-reviewer cross-platform peer (the existing `MASTER-SPEC §3.5:143 ACCEL-CROSS-PLATFORM-VERIFY` row), but this is not required by §3.4.
2. **Verdict provenance:** the Reviewer copies `verify_passed`, `verify_exit_code`, `verify_log_path` from the structured verdict (`diagnostic.rs:611-613`) verbatim into its `ReviewVote` (§3.1). No re-interpretation. The merge tallier (§3.4 conjunct 4) reads these fields to reject `Approve + red verify` votes.
3. **Background mode:** for builds longer than the default 90 s `deadline_ms` (`cluster_dispatch_wire.rs:153`), the Reviewer uses `dev_verify`'s `background: true` mode (`diagnostic.rs:678-716`) and polls via `{"job": "<id>"}` (line 629). The Reviewer subtask's own `deadline_ms` is then set to the full build budget rather than 90 s.

### 4.2 Composition with `--no-ff` branch merges
`MASTER-SPEC §4 Branch / merge / CI guardrails` (line 241) is the authoritative rule:

> *"Main advances only via reviewed `--no-ff` merge commits (pre-push hook); never force-push/reset/rewrite."*

This rule is the **post-condition** of a passing gate. Stated as a sequence:

```
1. Author peer pushes branch to `dev/<topic>` on its own remote.
2. Author peer constructs the review_request blob (§2.3) with branch + head_sha + base_sha
   + verify_command, signs it, hands it to its local cluster master.
3. Master dispatches 2 Reviewer Subtasks to non-author `role-reviewer` peers (§3.2).
4. Each Reviewer fetches read-only, runs dev_verify, emits a ReviewVote (§3.1).
5. Master tallies per §3.4. On MERGE verdict:
     git checkout main
     git merge --no-ff dev/<topic>   # single commit, never force-push
     git push origin main            # pre-push hook re-validates --no-ff + reviewed
6. On BLOCK verdict, the branch stays open and the work returns to ACCEL-WORK-QUEUE
   (MASTER-SPEC §3.5:148).
```

Step 5's `--no-ff` is the same merge style ACCEL-REVERTABLE-MERGES (`MASTER-SPEC §3.5:145`) already enforces; this gate sits **before** that hook fires. ACCEL-PRE-PUSH-HOOK (`MASTER-SPEC §3.5:153 scripts/hooks/pre-push`) is unchanged — it stays the last-line defense that blocks force-pushes / non-FF / direct-to-main even if the in-core gate is bypassed (e.g. by a human in stage-1 supervised mode).

### 4.3 Composition with the anti-pollution wall (`MASTER-SPEC §4 §2.4` + `§3.5:139 ACCEL-ANTIPOLLUTION-WALL`)
Every signal emitted by the cross-review pipeline is machine-origin. Concretely:

- Reviewer agent invocations are issued with the `MessageOrigin = Machine` marker (the policy intent at `MASTER-SPEC §3.5:152 ACCEL-DETECTION-HEURISTICS`). Stage 2 of this spec adds the `record_signal_with_origin` helper that `MASTER-SPEC §3.5:139` already names but `core/src/partner.rs:115-179` does not yet ship.
- The full `ReviewVote` JSON is appended to `~/.phantom-mesh/dev-loop-log.jsonl` (`MASTER-SPEC §3.5:160 STATE-DEV-LOOP-LOG-JSONL`), NOT to `~/.phantom-mesh/partner-signals.jsonl` (`MASTER-SPEC §3.5:159`, the moat ledger).
- The existing test `dev_loop_never_writes_partner_signals` (`MASTER-SPEC §4 §2.4 item 3`) must remain green after Stage-2 wires the Reviewer; this is normative.

This is the load-bearing reason the Reviewer role exists in core rather than only in the external thin orchestration layer: the in-core router is the only place we can enforce origin segregation without trusting an external script.

### 4.4 Composition with the gate (`MASTER-SPEC §4 §0.1`)
The 7-day Hard Gate (`MASTER-SPEC §4 §0.1`, line 220–225) is **independent** of this merge gate. The 7-day clock measures real-human moat usage; the merge gate measures per-PR review. They share zero state. The §5 BLOCKER deferral (line 250–255) ties the **self-igniting loop** to "cross-review automated into core" precisely because the merge gate is the trust mechanism the unattended loop needs — without §3.4 there is no machine-checkable answer to *"did a non-author AI verify this?"*

---

## 5. Open questions deferred to Stage 2 (this spec resolves the design only)

1. **Where the Reviewer agent's system prompt lives.** Candidate: a new `core/src/cluster_dispatch/reviewer_prompt.rs` constant; out of scope here.
2. **How the author signs the `review_request` blob.** Likely the existing `crate::rpc_wire::sign_hmac` (referenced at `cluster_dispatch_wire.rs:1166`), but the canonical-string layout is not specified here.
3. **How `reviewed_head_sha` is sealed against branch force-push.** Likely a short-lived signed token on the master, but the freshness window is not specified here.
4. **TS-RS regeneration impact.** Adding `DispatchRole::Reviewer` changes the generated TS union under `app/src/lib/generated/cluster_dispatch/` (line 361). The dashboard's existing `RoleBadge` component must learn the new label; tracked but not specified here.
5. **`../_archive/MASTER-SPEC.md` updates.** After Stage 2 lands, three rows flip status: `ACCEL-REVIEWER-ROLE` (line 147) `designed → built`, `ACCEL-MULTI-AI-CROSSREVIEW` (line 144) `partial → built`, and `§5 Q7` (line 263) is marked resolved with a pointer to this file.

---

## 6. Summary table (Stage-2 punch list, not a commitment)

| Change | File | Anchor line | Stage |
|---|---|---|---|
| Add `Reviewer` to `DispatchRole` | `core/src/cluster_dispatch_wire.rs` | line 360 | 2 |
| Add `Reviewer` arm to `role_required_caps` | `core/src/cluster_dispatch_wire.rs` | line 419 | 2 |
| Add merge-gate orchestrator (calls `assign_subtasks`) | new file under `core/src/cluster_dispatch/` | — | 2 |
| Add `ReviewVote` wire type | `core/src/cluster_dispatch_wire.rs` | after line 226 | 2 |
| Add `record_signal_with_origin` + `MessageOrigin` enum | `core/src/partner.rs` | around line 147 | 2 |
| Wire Reviewer prompt to forbid mutations | `core/src/agent.rs` (referenced via `AgentRuntime` import at `partner.rs:27`) | — | 2 |
| Update `MASTER-SPEC §3.5` rows + close `§5 Q7` | `docs/_archive/MASTER-SPEC.md` | lines 144, 147, 263 | 2 |
| Land in-core integration test: `cross_review_quorum_blocks_on_single_reject` | `core/src/cluster_dispatch_wire.rs` tests block | — | 2 |
| Land in-core integration test: `cross_review_blocks_on_author_self_vote` | `core/src/cluster_dispatch_wire.rs` tests block | — | 2 |
| Land in-core integration test: `cross_review_traffic_never_writes_partner_signals` | `core/src/partner.rs` tests block | extends the existing `dev_loop_never_writes_partner_signals` invariant | 2 |

---

*End of CROSS-REVIEW-AUTOMATION spec (Stage 1).*
