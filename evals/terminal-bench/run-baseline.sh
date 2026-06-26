#!/usr/bin/env bash
# One-command Terminal-Bench baseline for the phantom agent.
#
# Encapsulates the fiddly bits the harness needs on macOS + this adapter:
#   - hosts the linux binary over http for the task containers to fetch
#   - points the docker SDK at Docker Desktop's socket (DOCKER_HOST)
#   - loads provider API keys (so the adapter builds its failover chain)
#   - works around the registry prune bug by using a local --dataset-path
#
# Usage:
#   ./run-baseline.sh                          # default model + curated subset
#   MODEL=cerebras/gpt-oss-120b ./run-baseline.sh
#   ./run-baseline.sh fibonacci-server csv-to-parquet   # explicit task ids
#   N_CONCURRENT=2 ./run-baseline.sh
#
# Env overrides:
#   MODEL         provider/model for the primary (default cerebras/gpt-oss-120b)
#   KEYS_FILE     .env of provider keys (default ~/Downloads/llm-keys.env)
#   DATASET       local dataset path (default the cached terminal-bench-core 0.1.1)
#   N_CONCURRENT  concurrent trials (default 1 — gentle on free tiers)
#   PORT          http port for the binary host (default 8077)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$here"

MODEL="${MODEL:-cerebras/gpt-oss-120b}"
KEYS_FILE="${KEYS_FILE:-$HOME/Downloads/llm-keys.env}"
DATASET="${DATASET:-$HOME/.cache/terminal-bench/terminal-bench-core/0.1.1}"
N_CONCURRENT="${N_CONCURRENT:-1}"
PORT="${PORT:-8077}"

# Curated subset that doesn't need the heavy qemu/kernel/ML toolchains — a
# reasonable first signal. Override by passing task ids as args.
DEFAULT_TASKS=(fibonacci-server csv-to-parquet count-dataset-tokens fix-git \
  configure-git-webserver create-bucket sanitize-git-repo hello-world-flask \
  path-tracing-reverse swe-bench-astropy-1)
if [ "$#" -gt 0 ]; then
  TASKS=("$@")
else
  # keep only the curated tasks that actually exist in this dataset
  TASKS=()
  for t in "${DEFAULT_TASKS[@]}"; do
    [ -d "$DATASET/$t" ] && TASKS+=("$t")
  done
fi

export PATH="$HOME/.local/bin:$PATH"
export PYTHONPATH="$here"

# Docker Desktop on macOS doesn't expose /var/run/docker.sock; point the SDK at
# the active context's socket.
if [ -z "${DOCKER_HOST:-}" ]; then
  sock="$(docker context inspect 2>/dev/null \
    | python3 -c 'import sys,json;print(json.load(sys.stdin)[0]["Endpoints"]["docker"]["Host"])' 2>/dev/null || true)"
  [ -n "$sock" ] && export DOCKER_HOST="$sock"
fi

# Load provider keys (CEREBRAS_API_KEY, GROQ_API_KEY, ... — the adapter adds
# every present key to the failover chain).
if [ -f "$KEYS_FILE" ]; then
  set -a; # shellcheck disable=SC1090
  source "$KEYS_FILE"; set +a
  echo "[run-baseline] loaded keys from $KEYS_FILE"
else
  echo "[run-baseline] WARN: keys file $KEYS_FILE not found; relying on current env" >&2
fi

# Pick the linux binary matching the docker arch and host it over http.
arch="$(docker info --format '{{.Architecture}}' 2>/dev/null || uname -m)"
case "$arch" in
  aarch64|arm64) bin="phantom-aarch64-linux" ;;
  *)             bin="phantom-x86_64-linux" ;;
esac
if [ ! -f "$here/$bin" ]; then
  echo "[run-baseline] ERROR: $bin not found — build it with ./build-linux-binary.sh" >&2
  exit 1
fi
if ! pgrep -f "http.server $PORT" >/dev/null 2>&1; then
  ( cd "$here" && python3 -m http.server "$PORT" --bind 0.0.0.0 >/tmp/tb-httpd.log 2>&1 & )
  echo "[run-baseline] started binary host on :$PORT"
fi
export PHANTOM_TB_BINARY_URL="http://host.docker.internal:$PORT/$bin"

echo "[run-baseline] model=$MODEL tasks=${#TASKS[@]} concurrent=$N_CONCURRENT"
echo "[run-baseline] tasks: ${TASKS[*]}"

args=()
for t in "${TASKS[@]}"; do args+=(-t "$t"); done

tb run \
  --dataset-path "$DATASET" \
  "${args[@]}" \
  --agent-import-path phantom_agent:PhantomAgent \
  --model "$MODEL" \
  --n-concurrent "$N_CONCURRENT" \
  --output-path ./runs
