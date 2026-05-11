# shellcheck shell=bash
# mock.sh — start/stop a local mock LLM server for deterministic scenarios.
#
# Public surface:
#   mock_start [responses_file]      # starts on $MOCK_PORT (default 11999); echoes nothing
#   mock_stop                        # kills the running server (idempotent)
#   mock_pid                         # echoes PID of running server, or empty
#   mock_temp_agents_dir             # echoes a temp dir holding a ready agents.toml
#                                    # for use as `cd $(mock_temp_agents_dir); phantom repl ...`

require_cmd python

MOCK_PORT="${MOCK_PORT:-11999}"
MOCK_PID_FILE="${PHANTOM_TEST_TMP:-/tmp}/mock-llm.pid"
PHANTOM_TEST_HARNESS_ROOT="${PHANTOM_TEST_HARNESS_ROOT:-$(cd "$PHANTOM_TEST_LIB/.." && pwd)}"
MOCK_DEFAULT_RESPONSES="$PHANTOM_TEST_HARNESS_ROOT/fixtures/mock-responses.toml"
MOCK_AGENTS_TEMPLATE="$PHANTOM_TEST_HARNESS_ROOT/fixtures/agents-mock.toml"

mock_start() {
  local responses="${1:-$MOCK_DEFAULT_RESPONSES}"
  if [ ! -f "$responses" ]; then
    echo "  ✗ mock responses file not found: $responses" >&2
    return 1
  fi
  if [ -f "$MOCK_PID_FILE" ] && kill -0 "$(cat "$MOCK_PID_FILE")" 2>/dev/null; then
    return 0  # already running
  fi
  python "$PHANTOM_TEST_LIB/mock-llm-server.py" \
    --port "$MOCK_PORT" \
    --responses "$responses" \
    >"$PHANTOM_TEST_TMP/mock.stderr" 2>&1 &
  echo $! > "$MOCK_PID_FILE"

  # Wait up to 3s for the listener to bind.
  for _ in 1 2 3 4 5 6; do
    if curl -sS --max-time 1 "http://127.0.0.1:$MOCK_PORT/healthz" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.5
  done
  echo "  ✗ mock server failed to come up on :$MOCK_PORT — see $PHANTOM_TEST_TMP/mock.stderr" >&2
  return 1
}

mock_stop() {
  if [ -f "$MOCK_PID_FILE" ]; then
    local pid
    pid=$(cat "$MOCK_PID_FILE")
    if kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      # On Windows MSYS, kill may need a moment to actually stop the process.
      sleep 0.3
      kill -9 "$pid" 2>/dev/null || true
    fi
    rm -f "$MOCK_PID_FILE"
  fi
}

mock_pid() {
  [ -f "$MOCK_PID_FILE" ] && cat "$MOCK_PID_FILE"
}

# Make a temp dir containing ./agents.toml that points at the running mock.
# The caller can `cd` into the returned dir and run phantom commands; phantom
# will pick up the local agents.toml per the precedence rules in
# agents.toml.example (cwd is checked before $HOME/.phantom-mesh/).
mock_temp_agents_dir() {
  local d="$PHANTOM_TEST_TMP/mock-agents-cwd"
  mkdir -p "$d"
  cp "$MOCK_AGENTS_TEMPLATE" "$d/agents.toml"
  printf '%s\n' "$d"
}
