# spectyn-mesh × Terminal-Bench

Run spectyn-mesh's autonomous agent (`spectyn exec`) against
[Terminal-Bench](https://www.tbench.ai) to produce an eval score.

spectyn is a Rust binary, so this uses Terminal-Bench's **installed-agent** path:
the harness copies an install script into each task container, the script
downloads a linux `spectyn` build and writes a minimal `agents.toml`, then the
agent is driven headlessly with `spectyn exec` (auto-approve on).

## Files

| File | Role |
|---|---|
| `spectyn_agent.py` | `SpectynAgent(AbstractInstalledAgent)` — the adapter |
| `spectyn-setup.sh.j2` | install script run inside each task container |
| `build-linux-binary.sh` | cross-build the x86_64 linux binary (via Docker) |

## Prerequisites

1. **Docker running** (Terminal-Bench runs every task in a container).
2. **Terminal-Bench installed** (Python 3.11+; `uv` recommended):
   ```bash
   uv tool install terminal-bench      # provides the `tb` CLI
   # or: pipx install terminal-bench
   ```
3. **A linux `spectyn` binary, reachable by URL** (see next section).
4. **A provider API key** exported in your shell, e.g. `GROQ_API_KEY=...`.
   The provider is taken from `--model provider/model`; supported provider names
   map to keys in `spectyn_agent.py::_PROVIDER_ENV`.

## 1. Build + host the linux binary

```bash
./build-linux-binary.sh          # -> spectyn-x86_64-linux (needs Docker)

# Host it for the containers to fetch. Local/quick:
python3 -m http.server 8000      # serve this directory
export SPECTYN_TB_BINARY_URL=http://host.docker.internal:8000/spectyn-x86_64-linux

# Real/leaderboard runs: upload spectyn-x86_64-linux as a GitHub release asset
# and set SPECTYN_TB_BINARY_URL to that download URL instead.
```

## 2. Run a single task (smoke)

```bash
export GROQ_API_KEY=...           # provider key
export SPECTYN_TB_BINARY_URL=...  # from step 1

uv run tb run \
  --agent-import-path spectyn_agent:SpectynAgent \
  --model groq/llama-3.3-70b-versatile \
  --task-id hello-world
```

## 3. Run a subset / full dataset

```bash
uv run tb run \
  --agent-import-path spectyn_agent:SpectynAgent \
  --model groq/llama-3.3-70b-versatile \
  --dataset terminal-bench-core \
  --n-concurrent 4 \
  --output-path ./runs
```

Results (pass/fail per task + an overall accuracy) land under `--output-path`.

## Tuning knobs (env)

| Env | Default | Meaning |
|---|---|---|
| `SPECTYN_TB_BINARY_URL` | latest release asset | where the container fetches `spectyn` |
| `SPECTYN_TB_PROVIDER` | `groq` (or `--model` prefix) | agents.toml provider/`type` |
| `SPECTYN_TB_AGENT` | `master` | which configured agent to run |
| `SPECTYN_MAX_ROUNDS` | `40` | agent round cap inside a task |

## Notes & known gaps

- **Model quality drives the score.** Free 70B models score low; a respectable
  headline number needs a strong model (Claude/GPT) + API budget. The adapter is
  model-agnostic — switch with `--model`.
- The container agent runs with `SPECTYN_AUTO_APPROVE=1` so the shell tool's
  confirmation gate doesn't stall non-interactive runs.
- `spectyn exec` is single-turn headless; the agent loops internally up to
  `SPECTYN_MAX_ROUNDS`. Long tasks may need a higher cap.
- Verified locally that `spectyn exec` solves a self-contained terminal task
  end-to-end (writes + runs a script) with the free groq model; the remaining
  work to a real number is hosting the linux binary and a Docker run.
