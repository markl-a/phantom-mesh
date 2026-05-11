# phantom-test — black-box scenario harness

A bash-based test harness that drives the **already-running phantom** through
its public surfaces (CLI, HTTP/RPC, on-disk state) and asserts behavior. No
cargo build required — exists alongside the in-tree `cargo test` suite, not
instead of it.

The two layers are complementary:

| Layer | Lives in | Verifies | Reset between runs |
|---|---|---|---|
| `cargo test` (e.g. `tui_render_tests`) | `core/src/**/*.rs` | Code-level invariants, render paths, parser state | Each test fresh |
| `phantom-test` (this dir) | `scripts/phantom-test/` | Wire protocol, on-disk state, CLI surface, real LLM/RPC roundtrips | Side effects accumulate (events.jsonl grows, etc.) — by design |

## Quick start

```bash
# Run all scenarios against the local phantom serve.
scripts/phantom-test/harness.sh

# List discovered scenarios without executing.
scripts/phantom-test/harness.sh --list

# Run a subset by filename prefix.
scripts/phantom-test/harness.sh 02 06 08

# Retry any FAIL scenario once after a 30s cooldown (helpful for free-tier
# LLM rate-limit flakes — scenarios 13/21 in particular hit OpenCode hard
# enough that under suite-burst load they flake ~10% of the time, but pass
# clean on a calmer second attempt).
scripts/phantom-test/harness.sh --retry-failed
PHANTOM_TEST_RETRY_COOLDOWN_S=60 scripts/phantom-test/harness.sh --retry-failed
```

Exit code is 0 if every scenario passed, 1 if any failed, 2 if no scenarios
matched the filter. Skipped scenarios (exit 77 — usually missing dependency,
e.g. PowerShell on a Linux host) don't count as failures. Scenarios that
fail first-try but pass on retry are reported as `PASS-RETRY` in the
summary so reviewers can see which scenarios needed a second go.

## Required environment

| Var | Default | Purpose |
|---|---|---|
| `PHANTOM_BIN` | `phantom` | Binary on PATH, or absolute path |
| `PHANTOM_HOST` | `127.0.0.1` | serve host |
| `PHANTOM_PORT` | `7879` | serve port (matches `[core].port` in agents.toml) |
| `PHANTOM_CLUSTER_SECRET` | `phantom-cluster-2026` | for HMAC RPC |
| `PHANTOM_CONFIG_DIR` | `~/.phantom-mesh` | where DBs / events.jsonl live |
| `OPENCODE_API_KEY` (or any LLM provider key) | — | needed for scenarios that exercise real LLM calls (06) |

For scenario 07 (no-key graceful fail), the harness explicitly `env -u`s
all common LLM key vars, so its check works regardless of your shell env.

## Scenarios

Each `.sh` file in `scenarios/` is a self-contained scenario. The current
suite (16 scenarios, ~290 s total wall time on a Z13):

```
01-doctor-baseline.sh                phantom doctor structural checks
02-rpc-hmac-roundtrip.sh             POST /rpc/task/assign happy path (ASCII)
03-rpc-hmac-bad-secret.sh            wrong secret → unauthorized
04-rpc-chinese-prompt-encoding.sh    LOCK-IN: MSYS UTF-8 wire-byte gotcha
05-cluster-peer-list.sh              `phantom peer list` shape
06-llm-roundtrip-cli.sh              `phantom repl --agent master -c …` (real LLM)
07-llm-no-key-graceful-fail.sh       missing key → exit 1, no Rust panic
08-tui-snapshot-desktop.sh           PowerShell screen capture works (Windows)
09-events-jsonl-grows.sh             events.jsonl appended after a command
10-rpc-bogus-job-id.sh               GET status for missing job_id
11-cluster-rpc-remote-peer.sh        cross-machine HMAC dispatch (auto-discovers
                                       online non-self peer, or uses
                                       PHANTOM_REMOTE_PEER env var)
12-mock-llm-deterministic.sh         in-process mock LLM server: substring,
                                       regex, default fallback, cost reporting
13-agent-shell-tool-call.sh          agent dispatches shell({whoami}), gets stdout,
                                       echoes the local user back — full tool path
14-swarm-fan-out-synthesis.sh        `phantom swarm` discovery + dispatch + result
                                       collection + synthesis (asserts structure,
                                       not synthesis text — that's LLM-variable)
15-session-persistence-resume.sh     turn 1 writes ./conversations/<sid>.jsonl;
                                       `--session <sid>` recalls the passphrase
16-autoevolve-check-noop.sh          `autoevolve --once --target check` on green
                                       tree exits 0 + appends 1 line to autoevolve.log
                                       (60s timeout catches McAfee/Defender hangs)
```

## Adding a new scenario

```bash
cat > scripts/phantom-test/scenarios/11-my-new-thing.sh <<'EOF'
#!/usr/bin/env bash
source "$PHANTOM_TEST_LIB/common.sh"
source "$PHANTOM_TEST_LIB/cluster-rpc.sh"   # if you need RPC helpers
source "$PHANTOM_TEST_LIB/inspect.sh"        # if you need event/db helpers

scenario "what this scenario verifies"
require_cmd "$PHANTOM_BIN"   # skip if binary missing

step "doing the first observable thing"
out=$("$PHANTOM_BIN" some-subcommand 2>&1)
ASSERT_CONTAINS "$out" "expected substring" "label for this assertion"

# Many more ASSERT_* helpers — see lib/common.sh.

[ "$PHANTOM_TEST_FAILED" -eq 0 ]   # final exit code reflects assertion outcome
EOF
chmod +x scripts/phantom-test/scenarios/11-my-new-thing.sh
```

The runner discovers scenarios by glob — no central registry to update.
Use a `NN-` numeric prefix so the order in `--list` is stable.

### Assertions available (`lib/common.sh`)

```
ASSERT_EQ <a> <b> [label]                  exact string equality
ASSERT_CONTAINS <hay> <needle> [label]     substring present
ASSERT_NOT_CONTAINS <hay> <needle> [label] substring absent
ASSERT_HTTP <url> <expected_code> [label]  GET, expect HTTP code
ASSERT_FILE_GREW <path> <prev_size> [label] file is bigger than before
```

### RPC helpers (`lib/cluster-rpc.sh`)

```
rpc_url                                     -> http://host:port
rpc_dispatch <agent> <prompt>               -> job_id (or empty on auth fail)
rpc_dispatch_with_secret <secret> <agent> <prompt>
rpc_status <job_id>                         -> raw JSON
rpc_state <job_id>                          -> running|done|failed
rpc_output <job_id>                         -> agent text output
rpc_error <job_id>                          -> error string or empty
rpc_wait_done <job_id> <max_seconds>        -> 0 done / 1 failed / 2 timeout
```

### State inspectors (`lib/inspect.sh`)

```
events_count                                # lines in events.jsonl
events_tail <n>                             # last n lines
events_since <ts_ms>                        # lines after ts_ms
conversations_count                         # *.jsonl in conversations/
conversation_latest                         # path to newest jsonl
costs_recent <n>                            # last n LLM cost rows
cluster_peer_count                          # rows in cluster_nodes
doctor_summary                              # `phantom doctor` ANSI-stripped
now_ms                                      # current epoch ms
```

### Mock LLM server (`lib/mock-llm-server.py` + `lib/mock.sh`)

A standalone Python HTTP server that implements just enough of the
OpenAI-compatible API for phantom to think it's talking to a real LLM:

- `GET  /v1/models`              — returns scripted model list
- `POST /v1/chat/completions`    — returns scripted reply (streaming or not)
- `GET  /healthz`                — for liveness checks

Scripted responses live in `fixtures/mock-responses.toml`:

```toml
[[response]]
prompt_match = "ping"          # substring (case-insensitive)
text = "pong"

[[response]]
prompt_regex = "(?i)reply.*ok" # regex (re.search)
text = "ok"
delay_ms = 50                  # optional simulated latency

[default]
text = "MOCK: no scripted response matched"
```

Wire it in from a scenario via `lib/mock.sh`:

```bash
source "$PHANTOM_TEST_LIB/mock.sh"
trap 'mock_stop' EXIT          # always clean up

mock_start                     # uses fixtures/mock-responses.toml by default
agents_dir=$(mock_temp_agents_dir)
out=$(cd "$agents_dir" && phantom repl --agent master -c "ping")
ASSERT_CONTAINS "$out" "pong" "ping → pong"
```

`mock_temp_agents_dir` writes a temp `./agents.toml` that wires `[providers.mock]`
to `http://127.0.0.1:11999/v1`. phantom prefers a cwd-local `agents.toml`
over `$HOME/.phantom-mesh/agents.toml` (per the precedence note in the
project's `agents.toml.example`), so `cd $agents_dir && phantom …` runs
deterministic without touching the user's real config.

Stdlib-only Python (uses `tomllib` on 3.11+, falls back to a 25-line inline
TOML reader on older Pythons). No `pip install` required.

Why this instead of `[providers.mock]` baked into the binary?
- **Zero Rust changes** — works today regardless of build state
- **No new public surface in phantom** — the binary stays narrowly scoped
- **Easy to evolve scripted-response semantics** — fixture format can grow
  (tool-call mocking, multi-turn state machines) without touching core/

### TUI snapshot (`lib/snapshot.ps1`, Windows-only)

```powershell
# Full virtual desktop (all monitors)
powershell -ExecutionPolicy Bypass -File lib/snapshot.ps1

# Just one window matching a title substring
powershell -ExecutionPolicy Bypass -File lib/snapshot.ps1 -Window "phantom"
```

Output: prints PNG path on stdout, saves to `~/.phantom-mesh/snapshots/`.

## Limitations & roadmap

What this harness CANNOT do today (and what would fix it):

| Gap | Fix |
|---|---|
| Inject keystrokes into a running TUI | needs a `phantom tui --headless --script keys.txt` mode in core (deferred — requires Rust changes) |
| Headless TUI buffer dump (without screen capture) | needs `phantom tui render --state fixture.json --output snap.txt` subcommand (planned) |
| ~~Deterministic LLM (no real API call)~~ | ✅ shipped: see "Mock LLM server" above |
| Linux/macOS TUI snapshot | port `snapshot.ps1` to `xdotool` / `screencapture` (mechanical) |
| Concurrent / load testing | wrap dispatch helpers in xargs / GNU parallel (cookbook recipe, not a feature) |
| Tool-call mocking | extend `lib/mock-llm-server.py` to emit OpenAI tool_calls deltas based on a fixture entry |

Each gap is filed as a follow-up; the framework is structured so adding any
of them is additive (new lib/ helper + new scenarios), not invasive.

## Why bash, not Rust integration tests?

- **Verifies the wire protocol, not the abstract function call** — exercises
  the real HMAC, the real serve binary, the real conversation persistence.
  A `cargo test` in core/ that constructs an `AgentRuntime` directly will
  miss serializer bugs, header-name typos, or service-startup-order issues.
- **Drives a cross-language surface** — the same scenario can equally well
  hit a phantom serve running on Linux/Mac/Android. The harness only needs
  bash + curl + python3 + openssl, all of which exist on every dev machine
  in this repo's matrix.
- **No build required** — works on machines where `cargo build` is broken
  (e.g. Z13 with McAfee real-time scan stomping `.rmeta` writes), which is
  exactly when you most need a regression check.
- **Survives major refactors** — internal types / function signatures can
  change freely; as long as the CLI args, RPC payload, and on-disk format
  stay stable, scenarios continue to pass.

## When to add a `cargo test` instead

Use a code-level test (in `core/src/**/*.rs` under `#[cfg(test)]`) when:

- the assertion is about an internal data structure or pure function
- the test should run in CI without network or running serve
- failure should block compilation, not happen at runtime

Use a `phantom-test` scenario when:

- the assertion is about end-to-end behavior across process boundaries
- the test exercises persistence, networking, or CLI ergonomics
- you want to catch regressions in shipping behavior even when the code
  inside hasn't changed (e.g. dependency upgrade silently broke wire format)
