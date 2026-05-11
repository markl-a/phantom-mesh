# Testing the Windows binary end-to-end

How to verify a fresh `phantom.exe` doesn't regress any of the 18 fixes
from the 2026-05-01 Z13 hardening sweep. The full suite lives in
`scripts/test-windows.ps1` — this doc explains what each phase covers,
what passing looks like, and how to decode failures.

## TL;DR

```powershell
$env:OPENROUTER_API_KEY = 'sk-or-v1-...'
.\scripts\test-windows.ps1
```

Output ends with one of:

```
ALL CLEAR    -> exit 0, ship it
FAILED       -> exit code = number of failed phases, investigate
```

Per-phase: `[PASS]` (green), `[SKIP]` (yellow — environment limitation
or `-Skip*` flag), `[FAIL]` (red — actual regression).

### Known runner caveats (Windows PowerShell 5.1)

The test runner is best-effort on PS 5.1 because of two well-known
quirks the language won't let us cleanly avoid:

1. **OpenRouter free-tier rate limits.** Phase 3 fires two consecutive
   real LLM calls. If you've been hammering the same key from another
   shell, the second call may 429 and the assertion misses
   `$0.0000`. Wait 60 seconds and re-run with `-Phase 3`.
2. **`cmd /c` quote nesting.** Anything with embedded double quotes in
   the prompt has to be passed as a single shell-escaped string.
   The runner uses single-word prompts (`hi`, `hello`) for that
   reason. Don't change them to multi-word phrases.
3. **`evolve` cwd dependency.** Phase 6 cd's into `core/` before
   invoking `phantom evolve` because the agent runs `cargo test`
   through the shell tool with relative paths and needs `Cargo.toml`
   one level up. Without this, evolve ends with `stopped after N
   rounds` instead of `EVOLVE_DONE`.

If a phase intermittently fails despite a manually-verified clean
binary, treat the runner output as a smoke signal, not a verdict —
the manual flow in the rest of this doc is the canonical truth.

## Pre-requisites

| Item | How to check |
|---|---|
| `phantom` on PATH | `Get-Command phantom` resolves to `~\.local\bin\phantom.exe` |
| Latest binary deployed | `phantom --version` matches `git log -1 --format='%h'` (with `+` if dirty) |
| OpenRouter env var set | `$env:OPENROUTER_API_KEY` is non-empty (phases 3, 6 need it) |
| No `phantom serve` already running | The runner kills + restarts on its own port |
| Admin orphan `PhantomServe` task removed | See [§4.1 below](#41-admin-orphan-blocks-phase-4) — only matters for phase 4 |

## The 8 phases

### Phase 1 — Pre-flight setup

Stops any stray `phantom.exe`, confirms PATH, checks `OPENROUTER_API_KEY`.
Quick. If this fails, nothing else will work.

### Phase 2 — Read-only smoke

Touches every non-mutating top-level surface:

- `phantom --version` → expect `phantom 0.4.0 (<sha>+, windows-x86_64, ...)`
- `phantom doctor` → expect `configured port` line + `OpenRouter` listed in `provider keys`
- `phantom mcp` initialize + tools/list → expect ≥40 tools (currently 49)
- `phantom self-update --dry-run` → expect `phantom-x86_64-pc-windows.exe` target
- `phantom mlx status` / `phantom snapshot apply` → expect graceful Apple-only rejection

If `doctor` says `OpenRouter: not in env or agents.toml`, your shell didn't
see the env var — restart PowerShell or set the User-scope registry value.

### Phase 3 — LLM round-trips (master + coder via OpenRouter)

Real network calls to `openrouter.ai`. Free Llama tier, so cost is `$0.0000`.

- `phantom -c "..."` master agent → expect `$0.0000` in output banner.
- `phantom -c --agent coder "..."` coder agent → same.
- MCP `tools/call shell { cwd: "~" }` regression test → expect `home_test\r\n[exit code: 0]`, **no** `cwd '~' does not exist` error. This was the tilde-expansion bug fixed in commit `7752330`.

If LLM calls fail with `404 model … not found`, your `agents.toml` provider is misrouted (model name belongs to a different provider). Phase 2's `doctor` `provider keys` section should already have caught this.

### Phase 4 — `phantom service install/status/uninstall`

Round-trips the **PhantomServe** Scheduled Task:

- install via `Register-ScheduledTask -AtLogOn -User $env:USERNAME` (PowerShell, not `schtasks /SC ONLOGON` — see commit `8259330`).
- Adds Defender firewall rule for the configured port (best-effort — needs admin).
- status reports `registered: yes`, `last run: <timestamp>`, `last state: running` once phantom serve is up.
- uninstall removes the task + kills phantom.exe + drops firewall rule.

#### 4.1 Admin orphan blocks phase 4

If a previous **elevated** install left a `PhantomServe` task around, your
user-level `phantom service install` returns:

```
PermissionDenied (HRESULT 0x80070005): Register-ScheduledTask
```

Phase 4 detects this and **skips** with a clear next step. To clear:

```powershell
# from an elevated (admin) PowerShell
Unregister-ScheduledTask -TaskName 'PhantomServe' -Confirm:$false
```

If even that returns Access Denied:

```powershell
schtasks /Delete /TN "PhantomServe" /F          # admin cmd / PowerShell
```

Or open `taskschd.msc` → Task Scheduler Library → right-click `PhantomServe` → Delete.

Re-run phase 4 from your normal (non-admin) shell:

```powershell
.\scripts\test-windows.ps1 -Phase 4
```

### Phase 5 — `phantom autoevolve schedule install/status/uninstall`

Round-trips the **PhantomAutoevolve** task.  Different name from PhantomServe, so admin orphans are not a concern. Confirms:

- Interval renders as ISO 8601 (`PT1H`) from XML, not localized strings.
- `last state: never run` shows up correctly (Windows pre-2000 placeholder filtered server-side — commit `8259330` then refined).

### Phase 6 — `autoevolve --once` + `evolve --max-rounds 1`

Real cargo pipeline + real LLM:

- `autoevolve --once --target check` should print `cargo check green — nothing to evolve.` This proves the `CARGO_TARGET_DIR=~/.phantom-mesh/autoevolve-target` isolation + AV-lock detection from commit `4d3f78d` work — without this, Defender-locked `build-script-build.exe` would falsely trigger the LLM and we got the `main.rs` 1330→8-byte overwrite incident on 2026-05-01 12:38–12:57.
- `evolve --max-rounds 1 --target check` should walk one round and print `EVOLVE_DONE: all tests pass` at `$0.0000`.

### Phase 7 — `phantom serve` concurrent load

Boots `phantom serve --port <ServePort>` in a background job, then:

- 16-way parallel `/healthz` probe → all 16 must return `200`.
- `/api/version` → expect `version`, `commit`, `target=windows`, `wire_version`.
- `/rpc/ping` → expect `wire_version`, `core_sha`, `phantom_version` (post-macos-merge contract from commit `2680ff6`).
- Stop-Process clean shutdown.

Known: 30+ mixed concurrent (`/healthz` + `/api/version` + `/api/status`) is slow because `/api/status` has a global lock — see [§7.1 Known perf issues](#71-known-perf-issues).

### Phase 8 — Tilde edge cases + broken-pipe panic suppression

Last sanity check:

- `cwd: "~/"` (tilde with trailing slash) → expands to `$HOME`.
- Pipe-close: `phantom doctor | Select-Object -First 1` must exit 0 with no new entry in `~/.phantom-mesh/crashes/`. Pre-fix this leaked one crash log per piped invocation; commit `f0ec83b` installs the panic-hook filter.

## Phase mapping → commits

| Phase | Verifies commit |
|---|---|
| 2 doctor `OpenRouter` | `68d02d3` |
| 2 doctor `configured port` | `b50621a` |
| 3 cwd:'~' regression | `7752330` |
| 4 service install (PhantomServe + PowerShell) | `8259330`, `6f9344f` |
| 4 firewall rule auto-install | `b50621a` |
| 4 status `last state` decoding | `eec7a12` |
| 5 schedule install/status/uninstall | `3efbed2` |
| 5 schedule status XML interval | `8259330` |
| 6 autoevolve AV-lock detection | `4d3f78d` |
| 7 wire_version on /rpc/ping | merged from macos `2680ff6` |
| 7 `--port` flag honored | `eec7a12` |
| 8 broken-pipe panic suppression | `f0ec83b` |
| 8 cwd:'~/foo' expansion | `7752330` |

## §7.1 Known perf issues

`/api/status` has a global mutex that serialises requests. 30 concurrent
mixed hits take ~25s. Single-request and 4-concurrent are fine. Filed as
non-blocking — fix is to scope the read-side cluster snapshot to a
finer-grained lock.

## Re-deploy + re-test loop

After any code change to `core/`:

```powershell
.\scripts\build-windows.ps1 -Deploy        # rebuild + cp to ~/.phantom-mesh/bin + ~/.local/bin
.\scripts\test-windows.ps1                 # full E2E sweep
```

If only doc / config changed, skip the rebuild — the deployed binary
hasn't moved.
