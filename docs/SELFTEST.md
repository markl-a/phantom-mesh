# Self-Test Suite

A drop-in, registry-style test framework. **Every new feature ships with one
file** under `scripts/selftest.d/`; the orchestrator (`scripts/selftest.sh`)
auto-discovers it. Output is human-readable by default and machine-readable
(JSON) on demand, so both a developer and an LLM agent (Claude Code, phantom
itself, CI) can run it the same way.

## Run it

There are three entry points — they all do the same thing, pick whichever
matches your context:

```bash
# 1. Native subcommand (works anywhere phantom is on PATH)
phantom selftest                          # text, all features
phantom selftest --json --out r.json      # JSON report
phantom selftest --feature mcp            # just one feature
phantom selftest --p0-only                # CI smoke run
phantom selftest --list                   # show registered features

# 2. make targets (in the repo)
make selftest
make selftest-json
make selftest-list

# 3. The script directly (when you don't have a phantom binary handy)
scripts/selftest.sh --json --out a.json

# Env knobs (apply to all three)
PHANTOM_BIN=/path/to/phantom phantom selftest
COORD=http://10.0.0.5:7878   phantom selftest
PHANTOM_SELFTEST_SCRIPT=/abs/path/selftest.sh phantom selftest   # script override
PHANTOM_BASH=/path/to/bash.exe phantom selftest                  # bash override (Windows)
```

## Platform support

| Platform | Status |
|---|---|
| macOS    | ✅ first-class — bash + `python3` ship by default |
| Linux    | ✅ first-class — bash + `python3` ship by default |
| Windows + Git Bash | ✅ — `phantom selftest` auto-finds `bash.exe`. Pure-bash JSON builder runs when `python3` is missing, so no extra installs needed. `jq` is optional (only for consuming the JSON). |
| Windows + WSL | ✅ — same first-class story as Linux |
| Windows native (cmd / PowerShell, no Git/WSL) | ❌ — there is no bash. `phantom selftest` prints a one-shot install hint pointing at `https://git-scm.com/download/win`. |

The `phantom selftest` Rust shim probes for bash in this order on Windows:

1. `$PHANTOM_BASH` (if set; missing file → exit 2 with a clear error)
2. `bash` / `bash.exe` on `$PATH`
3. `C:\Program Files\Git\bin\bash.exe`
4. `C:\Program Files (x86)\Git\bin\bash.exe`
5. `%LOCALAPPDATA%\Programs\Git\bin\bash.exe` (per-user Git install)

`phantom selftest` is a thin Rust shim: it locates `scripts/selftest.sh` in
the repo (cwd → walk-up from cwd → walk-up from the binary →
`~/.phantom-mesh/scripts/selftest.sh`) and execs bash with all your args
forwarded. So an LLM agent driving phantom — or phantom driving itself
through its own `shell` tool — only needs to know `phantom selftest`.

## Exit codes

| code | meaning |
|------|---------|
| 0    | no P0 failure |
| 1    | at least one P0 test failed |
| 2    | orchestrator error (missing `selftest.d/`, bad arg) |

## How an LLM agent should consume the output (self-debug loop)

The suite is built so an agent — Claude Code, phantom itself, or CI — can
run, diagnose, and fix without a human in the loop.

```bash
scripts/selftest.sh --json --out /tmp/r.json   # exit 0 = green / 1 = P0 red

# 1. Triage — what failed?
jq '.summary'                                                 /tmp/r.json

# 2. For each failure, get THREE things needed to debug:
#    - the exact shell command to re-run just that check (repro)
#    - a path to a file with full stdout+stderr+exit code (artifact)
#    - source paths to grep first (hints, declared in feature meta)
jq -r '
  .features[] as $f
  | $f.tests[]
  | select(.status=="fail")
  | "── \($f.name)/\(.name)\nrepro:    \(.repro // "n/a")\nartifact: \(.artifact // "n/a")\nhints:    \($f.hints | join(" "))\n"
' /tmp/r.json

# 3. Re-run a single failing check directly (faster than the whole suite):
scripts/selftest.sh --feature <feature-name>

# 4. Read the artifact for full output:
cat <artifact-path>          # contains: command, cwd, date, stdout+stderr, exit
```

The JSON shape is stable. Failure rows always include `repro` and (for
helpers `t_run`/`t_check` or any test that set `T_ARTIFACT`) an `artifact`.
Each feature ships a `hints` array — paths the agent should grep first.

```json
{
  "phantom_version": "phantom 0.x.y (abc1234, ...)",
  "started_at": "2026-05-05T12:00:00Z",
  "duration_s": 14,
  "artifacts_dir": "test-results/selftest-20260505T120000Z",
  "summary": {"pass": 27, "fail": 0, "skip": 3, "p0_failures": 0},
  "features": [
    {
      "name": "binary",
      "priority": "P0",
      "requires": "",
      "description": "...",
      "hints": ["core/src/bin/phantom.rs", "core/src/main.rs"],
      "file": "00-binary.sh",
      "tests": [
        {
          "name": "phantom --version",
          "status": "pass",
          "detail": "phantom 0.5.1 (abc1234, macos-aarch64, ...)",
          "repro":  "/Users/me/.cargo/bin/phantom --version | grep -qE '^phantom [0-9]+\\.[0-9]+'",
          "artifact": "test-results/selftest-.../binary/phantom-version.log"
        }
      ]
    }
  ]
}
```

### Run artifacts directory

Each run creates `test-results/selftest-<utc-timestamp>/` containing:

- `selftest.log`  — TSV: `feature\tstatus\tname\tdetail\trepro\tartifact`
- `<feature>/<test-slug>.log` — full stdout+stderr+exit for any check using
  `t_run` / `t_check` or that set `T_ARTIFACT`. The header records command,
  cwd, and timestamp so the agent can re-run faithfully.

Old run dirs are pruned automatically (last 10 kept).

## Add a self-test for a new feature (60 seconds)

1. Pick a number prefix and a short name. Convention:
   - `00-09` bootstrap (binary, version)
   - `10-29` core CLI (doctor, run, init)
   - `30-49` network surfaces (serve, mcp, mesh)
   - `50-69` platform integrations (snapshot-mac, launchd, systemd)
   - `70-89` heavy / optional (mlx, autoevolve full cycle)
   - `90+`   experimental

2. `cp scripts/selftest.d/_template.sh scripts/selftest.d/35-myfeature.sh`

3. Fill in `selftest_feature_meta` and `selftest_run`. Use only the helpers:

   | helper | use for |
   |---|---|
   | `t_pass <name> [detail]` | success |
   | `t_fail <name> [detail]` | failure |
   | `t_skip <name> [reason]` | known-irrelevant on this host |
   | `t_run   <name> <argv...>` | run argv, full output → artifact, repro recorded |
   | `t_check <name> "<shell>"` | run shell string, full output → artifact, repro = the string |
   | `t_have <cmd>`           | predicate: command on PATH? |
   | `t_http <url> [code]`    | predicate: HTTP returns expected code? |

   **Always prefer `t_check` / `t_run` over manual `t_pass`/`t_fail`** when a
   shell command is what's being tested. They auto-capture stdout+stderr to
   `$SELFTEST_ARTIFACTS/<slug>.log` and store the command as the `repro`
   field, which is what an LLM agent uses to re-run and debug just that check.

   For manual `t_pass`/`t_fail` (e.g. you computed a value), set
   `T_REPRO=<cmd>` and/or `T_ARTIFACT=<path>` immediately before the call —
   they're auto-cleared after.

7. **Add `hints=` to your feature meta** — space-separated source paths an
   agent should grep first when this feature breaks. Example:
   `hints=core/src/serve.rs core/src/main.rs`. These propagate into the JSON
   report so an LLM has a starting set without guessing.

4. If your feature needs a precondition the orchestrator can't infer (daemon
   running, model file present, peer reachable), define
   `selftest_requires` — return non-zero with a one-line reason on stderr to
   skip the whole feature cleanly.

5. `chmod +x scripts/selftest.d/35-myfeature.sh && make selftest-list` to
   confirm it's picked up, then `make selftest` to run.

### Style rules

- **No echo PASS/FAIL.** Always use `t_pass / t_fail / t_skip` so the JSON
  report sees your row.
- **One assertion per line.** Don't bundle "everything in feature X works"
  into one giant test — five small assertions tell you _what_ broke.
- **Idempotent and read-only by default.** A self-test must be safe to run
  on the user's live machine. If a check writes state, undo it in the same
  function or move it to a `requires=destructive` priority-P2 file gated by
  an env var like `PHANTOM_SELFTEST_DESTRUCTIVE=1`.
- **Cheap.** Aim for the whole suite to finish under 30 seconds on a warm
  laptop. Move expensive checks (full evolve cycle, MLX inference) to P2 and
  guard them with `selftest_requires`.
- **Pick the right priority.** P0 = ship-blocker, P1 = expected-to-pass on
  any healthy install, P2 = nice-to-have / environment-dependent.

### What goes in the suite vs. `cargo test`

| | self-test | cargo test |
|---|---|---|
| target | the **installed** `phantom` binary, the **running** daemon, real network | Rust units / integration |
| run by | user, Claude, CI smoke job | dev loop, CI build job |
| assertion style | exit code, HTTP, JSON keys, presence of section labels | `assert_eq!` |

If you're testing pure Rust logic, write a `cargo test`. If you're testing
"does the installed binary still produce the right `phantom doctor` output",
that's a self-test feature.

## Worked example: adding a self-test for the new `phantom backup` command

```bash
# scripts/selftest.d/45-backup.sh
selftest_feature_meta() {
  echo "name=backup"
  echo "priority=P1"
  echo "requires="
  echo "description=phantom backup create + list round-trip on a temp dir"
}

selftest_run() {
  dest="$TMP/backup-test"
  mkdir -p "$dest"
  if "$PHANTOM" backup create --dest "$dest" --dry-run >"$TMP/bk.out" 2>&1; then
    t_pass "backup create --dry-run" "$(head -1 $TMP/bk.out)"
  else
    t_fail "backup create --dry-run" "$(tail -1 $TMP/bk.out)"
    return
  fi

  out="$("$PHANTOM" backup list --json 2>/dev/null)"
  if echo "$out" | jq -e '. | length >= 0' >/dev/null 2>&1; then
    t_pass "backup list returns JSON" ""
  else
    t_fail "backup list returns JSON" "not valid JSON"
  fi
}
```

That's the entire ritual: one file, two functions, automatically wired into
`make selftest`, `make selftest-json`, the JSON report, and CI.
