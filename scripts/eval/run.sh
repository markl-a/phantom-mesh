#!/usr/bin/env bash
# run.sh — run the OFFLINE spectyn-mesh promptfoo eval suite for CI.
#
# Exits non-zero if any offline case fails. Fully offline by default:
#   - no result sharing/upload (sharing:false in config)
#   - telemetry disabled via PROMPTFOO_DISABLE_TELEMETRY=1
#   - the LLM-dependent case is gated behind PROMPTFOO_LLM=1 and skipped here
#
# Usage:
#   bash scripts/eval/run.sh            # run once, CI mode
#   bash scripts/eval/run.sh --watch    # re-run on file changes (local dev)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG="$SCRIPT_DIR/promptfooconfig.yaml"

# Offline + privacy guards.
export PROMPTFOO_DISABLE_TELEMETRY=1
export PROMPTFOO_DISABLE_UPDATE=1
export NO_COLOR="${NO_COLOR:-1}"
# Default offline: do NOT opt into the gated LLM case unless caller already set it.
export PROMPTFOO_LLM="${PROMPTFOO_LLM:-0}"

# Ensure the spectyn binary is reachable (cargo installs land in ~/.cargo/bin).
if ! command -v spectyn >/dev/null 2>&1; then
  if [ -x "$HOME/.cargo/bin/spectyn" ]; then
    export PATH="$HOME/.cargo/bin:$PATH"
  else
    echo "WARNING: 'spectyn' not found on PATH; CLI-driven cases (1-3) will error." >&2
    echo "         Install the spectyn CLI or set SPECTYN_BIN to its path." >&2
  fi
fi

# promptfoo needs Node ^20.20.0 || >=22.22.0. The repo's default nvm node can be
# older (e.g. 22.14.0 → hard error "requires a supported Node.js runtime"), which
# silently skips the eval in autonomous/CI runs. Auto-select a compatible node:
# keep the current one if it qualifies, else try Homebrew, else the newest
# compatible nvm install; only warn if none is found.
node_ok() { # $1 = "vMAJOR.MINOR.PATCH" or "MAJOR.MINOR.PATCH"
  local v="${1#v}" major minor
  major="${v%%.*}"; minor="${v#*.}"; minor="${minor%%.*}"
  [ -z "$major" ] && return 1
  { [ "$major" = 20 ] && [ "$minor" -ge 20 ]; } && return 0
  { [ "$major" = 22 ] && [ "$minor" -ge 22 ]; } && return 0
  [ "$major" -ge 23 ] 2>/dev/null && return 0
  return 1
}
if command -v node >/dev/null 2>&1 && node_ok "$(node -v 2>/dev/null)"; then
  : # current node is fine
elif [ -x /opt/homebrew/bin/node ] && node_ok "$(/opt/homebrew/bin/node -v 2>/dev/null)"; then
  export PATH="/opt/homebrew/bin:$PATH"
elif [ -x /usr/local/bin/node ] && node_ok "$(/usr/local/bin/node -v 2>/dev/null)"; then
  export PATH="/usr/local/bin:$PATH"
elif [ -d "$HOME/.nvm/versions/node" ]; then
  # newest compatible nvm version (sort -V picks the highest)
  for nodedir in $(ls -1 "$HOME/.nvm/versions/node" 2>/dev/null | sort -Vr); do
    if node_ok "$nodedir" && [ -x "$HOME/.nvm/versions/node/$nodedir/bin/node" ]; then
      export PATH="$HOME/.nvm/versions/node/$nodedir/bin:$PATH"
      break
    fi
  done
fi
if command -v node >/dev/null 2>&1 && ! node_ok "$(node -v 2>/dev/null)"; then
  echo "WARNING: node $(node -v) does not satisfy promptfoo (^20.20.0 || >=22.22.0);" >&2
  echo "         install a compatible Node (e.g. 'brew install node') or 'nvm install 22'." >&2
fi

# Prefer a locally installed promptfoo; fall back to npx (network on first run only).
if command -v promptfoo >/dev/null 2>&1; then
  PF=(promptfoo)
elif [ -x "$SCRIPT_DIR/../../app/node_modules/.bin/promptfoo" ]; then
  PF=("$SCRIPT_DIR/../../app/node_modules/.bin/promptfoo")
else
  PF=(npx --yes promptfoo@latest)
fi

MODE="eval"
WATCH=""
if [ "${1:-}" = "--watch" ]; then
  WATCH="--watch"
fi

echo "==> running offline spectyn eval suite: ${PF[*]} $MODE -c $CONFIG $WATCH"
if [ -n "$WATCH" ]; then
  exec "${PF[@]}" "$MODE" -c "$CONFIG" "$WATCH"
else
  exec "${PF[@]}" "$MODE" -c "$CONFIG"
fi
