# apex-④ flagship loop — operator live smoke

As-automated-as-possible 2-machine smoke for the apex-④ differentiator:
**dispatch a governed task → pre-action approval card → approve → the task
continues**, and (the keystone) it **asserts the govern↔dispatch correlation**:
the dispatch task row's `approval_id` equals the live pending card's
`approval_id` (= `ExecutionContract.id`). That assertion is **D7** of
`docs/superpowers/specs/2026-06-24-govern-dispatch-correlation-design.md`; the
manual procedure is `docs/superpowers/runbooks/apex4-loop-live-smoke.md`.

Two scripts:

| Script | Runs on | Does |
|---|---|---|
| `worker-up.sh`  | the WORKER (Mac / a-worker-node / Linux sandbox) | preflight + `SPECTYN_GOVERN_CLI=1` + `SPECTYN_HOME` + start `spectyn serve`, poll `/healthz` |
| `run-smoke.sh`  | the COORDINATOR (e.g. the-coordinator) | dispatch a high-risk claude task, find the card, **assert D7**, approve, watch it finish, print `APEX4 SMOKE: PASS\|FAIL` |

Everything is leak-safe: no IPs / node names / secrets are hardcoded. The peer
is a CLI arg; the cluster secret + port come from `agents.toml` (or env). The
secret is never printed.

---

## One-time manual prereqs

These are the genuinely-manual bits the scripts cannot do for you:

1. **claude logged in on the WORKER.** The pre-action Approve/Deny gate fires
   ONLY for claude (`PreActionDelegated` via the PreToolUse hook;
   codex/opencode/agy are `PostActionObserved` — no pre-action card). Install
   Claude Code and run `claude` once interactively to complete login.
2. **API keys / provider creds** for whatever the worker agent needs.
3. **Tailnet (or routable network) up** between coordinator and worker.
4. **Shared cluster secret.** Both nodes must have the SAME
   `[cluster].cluster_secret` in their `agents.toml`. Establish it with
   `spectyn cluster join <name>` (or set it by hand identically on both).
5. **A worker agent that drives claude.** `agents.toml` on the worker must have
   an `[agent.<name>]` whose provider resolves to claude. Pass that name as
   `--agent`.
6. **Coordinator peers.json** (`spectyn config pull`) so `--peer <name>`
   resolves — or skip it and pass `--peer http://<host>:<port>` directly.

---

## How to run

**On the worker, first:**

```bash
cd scripts/apex4-smoke
./worker-up.sh                 # uses $SPECTYN_HOME or ~/.spectyn-mesh
# or:  ./worker-up.sh --home ~/.spectyn-mesh --foreground
```

It preflights, exports `SPECTYN_GOVERN_CLI=1` + `SPECTYN_HOME`, starts
`spectyn serve`, polls `/healthz`, and prints the listen addr.

**On the coordinator, then:**

```bash
cd scripts/apex4-smoke
./run-smoke.sh --peer <worker-name-or-url> --agent <claude-agent> --approve auto
# manual phone approval instead of an auto curl:
./run-smoke.sh --peer <worker-name-or-url> --agent <claude-agent> --approve manual
```

Flags: `--peer` (name from `peers.json` OR a full `http://host:port`),
`--agent` (the claude agent), `--approve auto|manual` (default `auto`),
`--secret-from agents.toml|env` (env reads `SPECTYN_CLUSTER_SECRET`),
`--prompt "<task>"`, `--timeout-await`, `--timeout-finish`.

---

## How to read PASS / FAIL

The last line is always `APEX4 SMOKE: PASS` or `APEX4 SMOKE: FAIL — <reasons>`,
and the exit code matches (0 / 1). Artifacts (job_id, both approval_ids,
before/after status JSON, the card, the approvals list) are saved to a
timestamped dir under `$TMPDIR`/`/tmp` and the path is printed.

The two gates that must both hold for PASS:

- **D7 correlation** — `/rpc/task/status/<job_id>.approval_id` ==
  `/rpc/approvals/list[].approval_id` for the card whose `task_id == job_id`.
- **task completed** — final status is `done` after the approve.

### What a FAIL at the correlation step means

`D7 CORRELATION: FAIL` with an **empty** row `approval_id` is the *pre-fix gap*:
the dispatch row never learned which card it's blocked on. The fix
(`set_approval_id` called from the claude PreToolUse hook's `with_dispatch_store`
at card-write time, keyed by `SPECTYN_GOVERN_TASK_ID` == the dispatch
`job_uuid`) is what makes them equal. A non-empty but *different* id means two
id universes are still in play — re-read the design doc §"The problem".

### Other loud failures (each prints an actionable message)

- `dispatch (FAIL)` — no `job_id`: HMAC rejected (secret mismatch), agent
  missing on the worker, or wire-version mismatch.
- `never reached awaiting-approval` — the run isn't governed
  (`SPECTYN_GOVERN_CLI=1`?), the agent isn't claude, or (see below) the
  dispatched-claude path doesn't reach the gate.
- `no pre-action approval reached` — the task `done` without pausing.
- peer-unreachable / missing secret / `claude` not found — caught in preflight.

---

## The loop is now CLOSED — configure a `claude_session` agent

The end-to-end loop is code-complete: the dispatch↔approval correlation
(`@90f6c12b`) + the **governed-claude dispatch routing** (`@3fe00407`).

REQUIRED worker config: the agent you pass to `--agent` must have
`provider_type = "claude_session"` (NOT `claude_agent`). `claude_session` is the
GOVERNED claude path — a dispatched task on it runs `run_cli_session(CliKind::Claude,
dispatch_task_id)` → `run_govern_folded` → claude's PreToolUse pre-action gate →
`set_approval_id` on the dispatch row. (`claude_agent` = the ungoverned `claude -p`
and will NOT escalate; codex/opencode/agy are `PostActionObserved` = notify-only,
no pre-action card.)

Example worker `agents.toml`:
```toml
[providers.claude_session]
provider_type = "claude_session"

[[agents]]
name = "coder"
provider = "claude_session"      # ← governed claude
# model = "claude-opus-4-8"      # optional
```
Then `worker-up.sh` (SPECTYN_GOVERN_CLI=1) + `run-smoke.sh --agent coder`. The D7
assert should PASS: the pending card's `approval_id` == the dispatch row's
`approval_id` for the `job_id`.

If `run-smoke.sh` still FAILs at "never reached awaiting-approval", check: (1) the
`--agent` resolves to `provider_type=claude_session` (not `claude_agent`); (2)
`SPECTYN_GOVERN_CLI=1` is in the worker `serve` env; (3) the dispatched prompt
actually triggers a high-risk (Bash) tool so the gate fires.
