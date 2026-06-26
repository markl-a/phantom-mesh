# Mac CLI + serve test results — read-only subset (2026-05-30)

> Actual run of the CLI-automatable, **read-only / non-mutating** cases from
> `2026-05-30-mac-app-cli-test-playbook.md`, on the M-series Mac (mac-coordinator,
> phantom 0.6.0-rc.1, serve up on :7878).
> Mutating cases (note/event capture, focus start/stop, evolve run, self-update,
> swarm/LLM-cost) and GUI/playwright cases were **not** run here.
>
> ⚠️ **PII NOTE for whoever extends this:** several commands print real secrets in
> their output — `phantom doctor` shows API-key prefixes, `/api/nodes` + `phantom
> peer list` return real tailnet IPs. **Scrub key prefixes + tailnet IPs before
> committing any raw capture.** This file records pass/fail + sanitized notes only.

## Result: 0 product bugs. All cases pass or are environmental/known-gap.

| ☑ | 編號 | 功能 | 結果 | 備註(已 scrub) |
|---|------|------|------|------|
| ☑ | ONB-002 | phantom init (/tmp) | PASS | PHANTOM.md created |
| ☑ | ONB-003 | phantom whoami | PASS | prints state = **尚未登入**(playbook note said "logged in via Google" — login state drifted) |
| ☑ | ONB-004 | phantom keys show | PASS | prints ed25519 pubkey + path |
| ☑ | ONB-005 | phantom lang show | PASS | zh-TW |
| ☑ | CHA-006 | evolve goals/list --json | PASS | valid JSON (`{"pending":[],"done":[]}` + active checkpoint) |
| ☑ | CHA-007 | autoevolve log / schedule status | PASS | registered=yes, 463 runs, recent all green |
| ☑ | CHA-015 | autoevolve digest --json | PASS | |
| ✗ | CHA-008 | skill run --dry-run | KNOWN-GAP | experimental-curator not in rc build (per playbook note) — not a regression |
| ☑ | LTR-001 | recall focus --json | PASS | |
| ☑ | LTR-002 | review --json | PASS | |
| ☑ | LTR-005 | data stats / export --out /tmp | PASS | export wrote /tmp/lifelog.md |
| ☑ | CLM-001 | cluster status | PASS | |
| ☑ | CLM-004 | node-capabilities | PASS | |
| ☑ | CLM-011 | /rpc/peers (GET) | PASS | 200 |
| ☑ | CLM-011 | /rpc/task/assign no-auth | PASS | **401** (auth gate works) ✓ |
| ☑ | CLM-012 | /rpc/admin/shell no-auth | PASS | **401** (admin shell auth-gated) ✓ |
| ☑ | SEC-001 | providers / models | PASS | |
| ☑ | SEC-003 | snapshot list | PASS | |
| ☑ | SEC-004 | mlx status | PASS | |
| △ | SEC-005 | sessions | NEEDS-LOGIN | prints "not logged in" (consistent w/ whoami). Minor: prints `Error:` but exit 0 — slightly inconsistent. |
| ☑ | SVI-002 | service status | PASS | |
| △ | SVI-004 | selftest --list | PASS-from-repo | fails from /tmp ("could not locate scripts/selftest.sh" — helpful error); works + lists P0/P1 registry from repo root. UX note: needs repo cwd or `$PHANTOM_SELFTEST_SCRIPT`. |
| ☑ | SVI-005 | doctor / doctor --json | PASS | all-green; (shows key prefixes — PII) |
| ☑ | SVI-009 | / (web shell) | PASS | 200 |
| ☑ | SVI-011 | /version, /api/version | PASS | 0.6.0-rc.1 |
| ☑ | SVI-012 | /healthz, /readyz | PASS | 200 / 200 |

## Findings
1. **No product bugs** in the read-only CLI/serve surface.
2. **Auth gates verified**: unauthenticated `/rpc/task/assign` and `/rpc/admin/shell` both return 401.
3. Two apparent fails were environmental: `sessions` (needs login), `selftest` (needs repo cwd) — not bugs.
4. Minor UX nits: `sessions` prints `Error:` but exits 0; `selftest` requires repo cwd (error msg is helpful).
5. `whoami` shows 尚未登入 — login state drifted from the playbook's note.
