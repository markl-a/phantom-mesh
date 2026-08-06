#!/usr/bin/env bash
# apex4-smoke/worker-up.sh — bring up the WORKER node for the apex-④ flagship
# live smoke (dispatch → govern → phone approve → continue).
#
# Run this ON THE WORKER (the machine that will execute the dispatched task —
# Mac / a-worker-node / a Linux sandbox). It:
#   1. preflights: spectyn on PATH, claude CLI present + logged in, agents.toml
#      has (a) a [cluster].cluster_secret, (b) a claude-capable agent;
#   2. exports SPECTYN_GOVERN_CLI=1 + SPECTYN_HOME so dispatched CLI work is
#      routed through L1 governance and the pending-card writer + serve share a
#      data root;
#   3. starts `spectyn serve` and polls /healthz until ready;
#   4. prints the listen addr the coordinator should target.
#
# LEAK-SAFE: nothing hardcoded. Host/port come from agents.toml [core]; the
# secret is never printed. Override the data root with SPECTYN_HOME before
# running, or pass --home <dir>.
#
# Usage:
#   ./worker-up.sh [--home <dir>] [--no-serve] [--foreground]
#
#   --home <dir>    data root (default: $SPECTYN_HOME or ~/.spectyn-mesh)
#   --no-serve      run preflight + print the env only; do NOT start serve
#   --foreground    run `spectyn serve` in the foreground (default: background,
#                   logged to <home>/apex4-smoke-serve.log)
set -euo pipefail

# ── tiny output helpers ────────────────────────────────────────────────────
err()  { printf '\033[31mERROR:\033[0m %s\n' "$*" >&2; }
ok()   { printf '\033[32m✓\033[0m %s\n'      "$*" >&2; }
info() { printf '\033[36m◆\033[0m %s\n'      "$*" >&2; }
die()  { err "$*"; exit 1; }

HOME_DIR="${SPECTYN_HOME:-$HOME/.spectyn-mesh}"
START_SERVE=1
FOREGROUND=0
while [ $# -gt 0 ]; do
  case "$1" in
    --home)       HOME_DIR="${2:?--home needs a dir}"; shift 2 ;;
    --no-serve)   START_SERVE=0; shift ;;
    --foreground) FOREGROUND=1; shift ;;
    -h|--help)    grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *)            die "unknown arg: $1 (see --help)" ;;
  esac
done

# ── 1. preflight: spectyn on PATH ──────────────────────────────────────────
command -v spectyn >/dev/null 2>&1 \
  || die "spectyn not on PATH. Build it and add the binary to PATH first."
ok "spectyn found: $(command -v spectyn)"

# ── 1b. preflight: claude CLI present + logged in ──────────────────────────
# The apex-④ PRE-ACTION gate only fires for claude (PreActionDelegated via the
# PreToolUse hook). codex/opencode/agy are PostActionObserved (no pre-action
# approval card). So a TRUE approve smoke needs a working `claude`.
if ! command -v claude >/dev/null 2>&1; then
  err "claude CLI not on PATH."
  err "  The apex-④ pre-action approval gate ONLY fires for claude."
  err "  Install Claude Code + run \`claude\` once to log in, then re-run."
  exit 1
fi
ok "claude found: $(command -v claude)"
# Best-effort login probe — claude prints its version when authed; a hard
# auth check is not scriptable, so we only WARN if the version call fails.
if ! claude --version >/dev/null 2>&1; then
  err "\`claude --version\` failed — claude may not be logged in."
  err "  Run \`claude\` interactively once to complete login, then re-run."
  exit 1
fi
ok "claude responds to --version (assumed logged in)"

# ── 2. locate + parse agents.toml ──────────────────────────────────────────
# Resolution order mirrors the runtime: $SPECTYN_HOME/agents.toml, then
# ~/.spectyn-mesh/agents.toml. (config::AgentsConfig::find_and_load.)
AGENTS_TOML=""
for cand in "$HOME_DIR/agents.toml" "$HOME/.spectyn-mesh/agents.toml" "./agents.toml"; do
  if [ -f "$cand" ]; then AGENTS_TOML="$cand"; break; fi
done
[ -n "$AGENTS_TOML" ] \
  || die "no agents.toml found (looked in $HOME_DIR, ~/.spectyn-mesh, .). Run \`spectyn cluster join <name>\` first."
ok "agents.toml: $AGENTS_TOML"

# Minimal TOML scalar reader: first `key = "value"` (or key = value) anywhere in
# the file. Good enough for [cluster].cluster_secret and [core].host/port which
# are simple scalars. Strips quotes + inline comments.
toml_scalar() {
  # $1 = key name
  sed -n -E "s/^[[:space:]]*$1[[:space:]]*=[[:space:]]*(.*)$/\1/p" "$AGENTS_TOML" \
    | head -n1 \
    | sed -E 's/[[:space:]]*#.*$//; s/^"//; s/"$//; s/^'"'"'//; s/'"'"'$//'
}

# ── 2a. cluster_secret present (do NOT print it) ───────────────────────────
SECRET="$(toml_scalar cluster_secret || true)"
[ -n "$SECRET" ] \
  || die "[cluster].cluster_secret missing/empty in $AGENTS_TOML. Run \`spectyn cluster join <name>\` so both nodes share a secret."
ok "[cluster].cluster_secret present (value hidden)"

# ── 2b. a claude-capable agent exists ──────────────────────────────────────
# We can't fully resolve the provider ladder from bash, but we surface what's
# there so the operator can confirm an agent that drives claude exists.
if grep -Eq 'provider_type[[:space:]]*=[[:space:]]*"claude' "$AGENTS_TOML" \
   || grep -Eq '^\[agent\.' "$AGENTS_TOML"; then
  ok "agents.toml declares [agent.*] / a claude provider (confirm it resolves to claude)"
  info "agents present:"
  grep -E '^\[agent\.' "$AGENTS_TOML" | sed 's/^/    /' >&2 || true
else
  err "No [agent.*] blocks found in $AGENTS_TOML."
  err "  Add an agent whose provider drives claude so dispatched work runs as claude."
  exit 1
fi

# ── 3. resolve listen addr from [core] ─────────────────────────────────────
CORE_HOST="$(toml_scalar host || true)"; CORE_HOST="${CORE_HOST:-0.0.0.0}"
CORE_PORT="$(toml_scalar port || true)"; CORE_PORT="${CORE_PORT:-7878}"
# /healthz is always served on loopback regardless of bind host.
HEALTH_URL="http://127.0.0.1:${CORE_PORT}/healthz"

# ── 4. export governance env ───────────────────────────────────────────────
export SPECTYN_GOVERN_CLI=1
export SPECTYN_HOME="$HOME_DIR"
mkdir -p "$HOME_DIR"
ok "SPECTYN_GOVERN_CLI=1"
ok "SPECTYN_HOME=$HOME_DIR"
info "worker will listen on ${CORE_HOST}:${CORE_PORT} (per [core] in agents.toml)"

if [ "$START_SERVE" -eq 0 ]; then
  info "--no-serve: env is set, serve NOT started. Start it yourself with:"
  echo  "    SPECTYN_GOVERN_CLI=1 SPECTYN_HOME='$HOME_DIR' spectyn serve" >&2
  exit 0
fi

# ── 5. start serve + poll /healthz ─────────────────────────────────────────
poll_health() {
  local i
  for i in $(seq 1 30); do
    if curl -fsS --max-time 2 "$HEALTH_URL" >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  return 1
}

if [ "$FOREGROUND" -eq 1 ]; then
  info "starting \`spectyn serve\` in the FOREGROUND (Ctrl-C to stop)…"
  info "in another shell, confirm readiness: curl $HEALTH_URL"
  exec spectyn serve
fi

LOG="$HOME_DIR/apex4-smoke-serve.log"
info "starting \`spectyn serve\` in the background → $LOG"
# shellcheck disable=SC2069
( SPECTYN_GOVERN_CLI=1 SPECTYN_HOME="$HOME_DIR" spectyn serve >"$LOG" 2>&1 & echo $! >"$HOME_DIR/apex4-smoke-serve.pid" )
SERVE_PID="$(cat "$HOME_DIR/apex4-smoke-serve.pid" 2>/dev/null || echo '?')"
info "serve pid: $SERVE_PID (poll /healthz up to 30s)…"
if poll_health; then
  ok "READY — /healthz green at $HEALTH_URL"
  echo >&2
  ok "Worker is up. On the COORDINATOR run:"
  echo "    ./run-smoke.sh --peer <this-worker-name-or-url> --agent <claude-agent>" >&2
  echo >&2
  info "stop serve later with:  kill $SERVE_PID    (log: $LOG)"
else
  err "serve did NOT become healthy within 30s. Last 20 log lines:"
  tail -n 20 "$LOG" >&2 2>/dev/null || true
  exit 1
fi
