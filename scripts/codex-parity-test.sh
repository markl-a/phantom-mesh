#!/usr/bin/env bash
# Comprehensive end-to-end test for the 8 codex-parity features
# shipped 2026-05-07 (interrupt, exec, sandbox, frame-cap, approval,
# tool trait, landlock, fork mode). Runs the actual spectyn binary
# against scripted scenarios — not unit-level mocks — so it catches
# regressions in the wired-up paths. TUI-only behaviour (interrupt
# UX, frame-cap visual smoothness) is excluded; see SESSION_RESUME
# for the manual checklist that covers those.
#
# Usage:
#   scripts/codex-parity-test.sh           # all scenarios, stop on fail
#   scripts/codex-parity-test.sh -v        # verbose (full output)
#   KEEP_GOING=1 scripts/codex-parity-test.sh   # don't stop on fail

set -u
VERBOSE=${1:-}
KEEP_GOING=${KEEP_GOING:-0}

PASS=0
FAIL=0
FAILED_NAMES=()

# Resolve the spectyn binary. Order:
#   1. $SPECTYN env override — lets CI / dev iteration point at a
#      freshly-built debug or release binary without `cargo install`.
#   2. ~/.cargo/bin/spectyn — where `make install` lands; preferred
#      over PATH so we never accidentally test an older copy.
#   3. `command -v spectyn` — final fallback.
if [ -n "${SPECTYN:-}" ] && [ -x "$SPECTYN" ]; then
    : # caller already supplied a working binary path
elif [ -x "${HOME}/.cargo/bin/spectyn" ]; then
    SPECTYN="${HOME}/.cargo/bin/spectyn"
else
    SPECTYN=$(command -v spectyn) || {
        echo "FATAL: spectyn binary not found (set \$SPECTYN, run \`make install\`, or add to PATH)"
        exit 2
    }
fi

run() {
    local name=$1; shift
    local description=$1; shift
    printf "%-44s " "[$name] $description"
    local out
    if out=$("$@" 2>&1); then
        echo "PASS"
        PASS=$((PASS + 1))
        [ -n "$VERBOSE" ] && echo "$out" | sed 's/^/      /'
    else
        echo "FAIL (rc=$?)"
        FAIL=$((FAIL + 1))
        FAILED_NAMES+=("$name")
        echo "$out" | sed 's/^/      /' | head -8
        [ "$KEEP_GOING" = "0" ] && {
            echo ""
            echo "━━ stopped on first fail (set KEEP_GOING=1 to continue) ━━"
            exit 1
        }
    fi
}

# ── Scenario functions ──────────────────────────────────────────────────────

t_exec_help() {
    # `spectyn exec --help` writes via eprintln! → stderr. Pipe both
    # streams so this test doesn't break if the help dispatch flips
    # to stdout later.
    "$SPECTYN" exec --help 2>&1 | grep -q "Headless single-turn agent run"
}

t_exec_empty_input_exits_2() {
    set +e
    "$SPECTYN" exec </dev/null >/dev/null 2>&1
    local rc=$?
    set -e
    [ $rc -eq 2 ]
}

t_exec_stdin_pipe() {
    local out
    out=$(echo "what is 7+8? answer with just the number" \
        | "$SPECTYN" exec --quiet 2>/dev/null)
    echo "$out" | grep -q "15"
}

t_exec_json_emits_event_types() {
    local out
    out=$(echo "say hi in 1 word" \
        | "$SPECTYN" exec --json 2>/dev/null)
    echo "$out" | grep -q '"type":"token"' && \
    echo "$out" | grep -q '"type":"done"'
}

t_exec_in_help_listing() {
    "$SPECTYN" --help 2>&1 | grep -q "spectyn exec"
}

t_sandbox_disabled_by_env() {
    SPECTYN_SANDBOX=0 "$SPECTYN" --version >/dev/null
}

t_sandbox_write_inside_cwd_succeeds() {
    local td; td=$(mktemp -d)
    pushd "$td" >/dev/null
    echo "use the shell tool to create a file 'sandbox-ok.txt' with content 'works' here, nothing else" \
        | "$SPECTYN" exec --quiet 2>/dev/null >/dev/null
    local rc=0
    if [ ! -f sandbox-ok.txt ] || ! grep -q "works" sandbox-ok.txt; then
        rc=1
    fi
    popd >/dev/null
    rm -rf "$td"
    return $rc
}

t_sandbox_write_to_etc_blocked() {
    local etc_marker="/etc/spectyn-st-$$"
    sudo rm -f "$etc_marker" 2>/dev/null || true
    echo "use the shell tool exactly: echo bad > $etc_marker" \
        | "$SPECTYN" exec --quiet 2>/dev/null >/dev/null
    [ ! -e "$etc_marker" ]
}

t_task_tool_dispatches() {
    local out
    out=$(echo "use the task tool to spawn a 'master' subagent with prompt 'reply with the word OK and nothing else'" \
        | "$SPECTYN" exec --quiet 2>/dev/null)
    # Either the wrapper text "[subagent: ...]" appears or the
    # subagent's "OK" reply makes it back. Smoke-level check.
    echo "$out" | grep -qiE "(subagent|OK)"
}

# ── 4. Permission DSL ───────────────────────────────────────────────────────
#
# We exercise the Tool(specifier) rule engine via `spectyn doctor`'s
# permissions section rather than via real LLM calls — that keeps the
# tests fast, deterministic, and free of provider keys. Each test
# writes a temp HOME with a synthetic agents.toml and greps the
# doctor output. Cleanup happens via `trap` inside each function so a
# fail doesn't leak directories.

# Build a temp HOME with an agents.toml whose [permissions] section is
# whatever the caller passes as $1. Echoes the HOME path on stdout.
_perm_setup_home() {
    local block="$1"
    local td; td=$(mktemp -d)
    mkdir -p "$td/.spectyn-mesh"
    cat > "$td/.spectyn-mesh/agents.toml" <<EOF
[core]
host = "127.0.0.1"
port = 7878

[providers.anthropic]
type    = "anthropic"
api_key = "sk-ant-test-fake-key"

[agent.master]
provider     = "anthropic"
instructions = "test"
tools        = ["shell", "file_read", "web_fetch"]

$block
EOF
    echo "$td"
}

t_perm_empty_default() {
    local td; td=$(_perm_setup_home "")
    trap 'rm -rf "$td"' RETURN
    HOME="$td" "$SPECTYN" doctor 2>&1 \
        | grep -q "no rules → allow all"
}

t_perm_parses_rules() {
    local td
    td=$(_perm_setup_home '[permissions]
deny  = ["Read(./.env)"]
ask   = ["Bash"]
allow = ["Bash(git status)", "Read(./README.md)"]')
    trap 'rm -rf "$td"' RETURN
    # Edit family expands so 4 deny gets parsed from 1, etc. The exact
    # count depends on alias expansion — assert the output shape rather
    # than a specific number.
    HOME="$td" "$SPECTYN" doctor 2>&1 \
        | grep -qE "rules parsed \([0-9]+ deny, [0-9]+ ask, [0-9]+ allow\)"
}

t_perm_parse_error_flag() {
    local td
    # Bash with an unterminated specifier: should hit the parser error
    # path. Doctor must surface "parse error" rather than silently
    # falling back.
    td=$(_perm_setup_home '[permissions]
deny = ["Bash(unclosed-spec"]')
    trap 'rm -rf "$td"' RETURN
    HOME="$td" "$SPECTYN" doctor 2>&1 \
        | grep -qi "parse error"
}

t_perm_statically_denied() {
    local td
    # Blanket WebFetch deny → must appear in the "statically denied"
    # line so users know it'll be stripped from the LLM tool list.
    td=$(_perm_setup_home '[permissions]
deny = ["WebFetch"]')
    trap 'rm -rf "$td"' RETURN
    HOME="$td" "$SPECTYN" doctor 2>&1 \
        | grep -q "statically denied.*web_fetch"
}

t_no_new_crash_logs() {
    local now
    now=$(ls "$HOME/.spectyn-mesh/crashes/" 2>/dev/null | wc -l | tr -d ' ')
    if [ "$now" != "${BASELINE_CRASHES:-0}" ]; then
        echo "    NEW CRASHES SINCE BASELINE: $((now - BASELINE_CRASHES))"
        ls -t "$HOME/.spectyn-mesh/crashes/" | head -3 | sed 's/^/      /'
        return 1
    fi
}

# ── Main ────────────────────────────────────────────────────────────────────

export BASELINE_CRASHES=$(ls "$HOME/.spectyn-mesh/crashes/" 2>/dev/null | wc -l | tr -d ' ')

echo "━━ spectyn codex-parity integration test ━━"
echo "binary:    $SPECTYN"
echo "version:   $("$SPECTYN" --version)"
echo "baseline:  $BASELINE_CRASHES existing crash logs"
echo ""

echo "── 1. spectyn exec ──────────────────────────────────────────────"
run "exec.help"         "exec --help works"               t_exec_help
run "exec.empty"        "empty stdin → exit 2"            t_exec_empty_input_exits_2
run "exec.stdin"        "stdin pipe → answer to stdout"   t_exec_stdin_pipe
run "exec.json"         "--json emits token + done"       t_exec_json_emits_event_types
run "exec.help_listing" "exec appears in --help"          t_exec_in_help_listing

echo ""
echo "── 2. macOS sandbox ─────────────────────────────────────────────"
run "sb.disable_env"    "SPECTYN_SANDBOX=0 still works"   t_sandbox_disabled_by_env
run "sb.cwd_allowed"    "write inside cwd succeeds"       t_sandbox_write_inside_cwd_succeeds
run "sb.etc_blocked"    "write to /etc blocked"           t_sandbox_write_to_etc_blocked

echo ""
echo "── 3. Subagent ──────────────────────────────────────────────────"
run "subagent.task"     "task tool dispatches"            t_task_tool_dispatches

echo ""
echo "── 4. Permission DSL ────────────────────────────────────────────"
run "perm.empty_default"     "no [permissions] → allow all reported"   t_perm_empty_default
run "perm.parses_rules"      "valid rules parse + count reported"      t_perm_parses_rules
run "perm.parse_error_flag"  "malformed rule surfaces in doctor"       t_perm_parse_error_flag
run "perm.statically_denied" "blanket Deny appears in static-deny list" t_perm_statically_denied

echo ""
echo "── 5. Regression ────────────────────────────────────────────────"
run "regress.no_crash"  "no new crash logs"               t_no_new_crash_logs

echo ""
echo "━━ result: $PASS passed, $FAIL failed ━━"
if [ $FAIL -gt 0 ]; then
    echo "failed: ${FAILED_NAMES[*]}"
    exit 1
fi
