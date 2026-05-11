#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BINARY="${BINARY:-$PROJECT_ROOT/core/target/release/phantom-mesh}"
PORT="${PORT:-17878}"  # Use non-standard port to avoid conflicts
BASE="http://localhost:$PORT"
PASS=0
FAIL=0
TMPDIR_TEST=$(mktemp -d)
DAEMON_PID=""

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

pass() { echo -e "${GREEN}PASS${NC} $1"; PASS=$((PASS+1)); }
fail() { echo -e "${RED}FAIL${NC} $1: $2"; FAIL=$((FAIL+1)); }
skip() { echo -e "${YELLOW}SKIP${NC} $1: $2"; }

cleanup() {
    if [ -n "$DAEMON_PID" ]; then
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true
    fi
    rm -rf "$TMPDIR_TEST"
}
trap cleanup EXIT

# ── Build if needed ─────────────────────────────────────────────────────────

if [ ! -f "$BINARY" ]; then
    echo "Binary not found at $BINARY — building..."
    (cd "$PROJECT_ROOT/core" && cargo build --release --quiet) || {
        echo "Build failed. Try: cd $PROJECT_ROOT/core && cargo build --release"
        exit 1
    }
fi

# ── Write a minimal test config (no API keys needed for structural tests) ───

cat > "$TMPDIR_TEST/agents.toml" << TOML
[core]
host = "0.0.0.0"
port = $PORT

[agent.master]
provider = "test"
model = "test"
tools = ["shell", "file_read", "file_write"]
instructions = "test agent"
TOML

# ── Start daemon ─────────────────────────────────────────────────────────────

"$BINARY" daemon --port "$PORT" --config "$TMPDIR_TEST/agents.toml" > "$TMPDIR_TEST/daemon.log" 2>&1 &
DAEMON_PID=$!

echo "Waiting for daemon to be ready..."
for i in $(seq 1 20); do
    if curl -sf "$BASE/health" > /dev/null 2>&1; then
        echo "Daemon ready after ${i} attempts."
        break
    fi
    if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
        echo "Daemon exited prematurely. Log:"
        cat "$TMPDIR_TEST/daemon.log"
        exit 1
    fi
    sleep 0.5
done

if ! curl -sf "$BASE/health" > /dev/null 2>&1; then
    echo "Daemon did not become ready in time. Log:"
    cat "$TMPDIR_TEST/daemon.log"
    exit 1
fi

echo ""
echo "=== phantom-mesh integration tests ==="
echo "Daemon: $BASE (PID $DAEMON_PID)"
echo ""

# ── Helper ───────────────────────────────────────────────────────────────────

check_json_field() {
    local label="$1" field="$2" response="$3"
    if echo "$response" | python3 -c "import sys,json; d=json.load(sys.stdin); assert '$field' in d" 2>/dev/null; then
        pass "$label"
    else
        fail "$label" "missing '$field' in: $response"
    fi
}

# ── /health ───────────────────────────────────────────────────────────────────

R=$(curl -sf "$BASE/health")
if echo "$R" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d.get('status')=='ok'" 2>/dev/null; then
    pass "/health returns status:ok"
else
    fail "/health" "bad response: $R"
fi

check_json_field "/health has version field"        "version"        "$R"
check_json_field "/health has service field"        "service"        "$R"
check_json_field "/health has uptime_seconds field" "uptime_seconds" "$R"

# ── /api/version ─────────────────────────────────────────────────────────────

R=$(curl -sf "$BASE/api/version")
check_json_field "/api/version has version field" "version" "$R"

# ── /api/dashboard/status ────────────────────────────────────────────────────

R=$(curl -sf "$BASE/api/dashboard/status")
check_json_field "/api/dashboard/status has tools_count" "tools_count" "$R"
check_json_field "/api/dashboard/status has hands_count" "hands_count" "$R"

# ── /api/providers/health ────────────────────────────────────────────────────

R=$(curl -sf "$BASE/api/providers/health")
check_json_field "/api/providers/health has providers key" "providers" "$R"

# ── /tools ───────────────────────────────────────────────────────────────────

R=$(curl -sf "$BASE/tools")
COUNT=$(echo "$R" | python3 -c "import sys,json; print(len(json.load(sys.stdin)['tools']))" 2>/dev/null || echo 0)
if [ "$COUNT" -ge 1 ]; then
    pass "/tools returns tool list ($COUNT tools)"
else
    fail "/tools" "got $COUNT tools, expected >= 1"
fi

# ── /hands ────────────────────────────────────────────────────────────────────

R=$(curl -sf "$BASE/hands")
COUNT=$(echo "$R" | python3 -c "import sys,json; print(len(json.load(sys.stdin)['hands']))" 2>/dev/null || echo 0)
if [ "$COUNT" -ge 1 ]; then
    pass "/hands returns hands list ($COUNT hands)"
else
    fail "/hands" "got $COUNT hands, expected >= 1"
fi

# ── /costs ────────────────────────────────────────────────────────────────────

R=$(curl -sf "$BASE/costs")
check_json_field "/costs has total_usd" "total_usd" "$R"
check_json_field "/costs has requests"  "requests"  "$R"

# ── /cluster/status ───────────────────────────────────────────────────────────

R=$(curl -sf "$BASE/cluster/status")
check_json_field "/cluster/status has node_count" "node_count" "$R"
check_json_field "/cluster/status has nodes"      "nodes"      "$R"

# ── /cluster/workers ──────────────────────────────────────────────────────────

R=$(curl -sf "$BASE/cluster/workers")
check_json_field "/cluster/workers has workers key" "workers" "$R"

# ── /cluster/scores ───────────────────────────────────────────────────────────

R=$(curl -sf "$BASE/cluster/scores")
check_json_field "/cluster/scores has scores key" "scores" "$R"

# ── /task/history ─────────────────────────────────────────────────────────────

R=$(curl -sf "$BASE/task/history")
check_json_field "/task/history has tasks key" "tasks" "$R"

# ── /memory/observations ──────────────────────────────────────────────────────

R=$(curl -sf "$BASE/memory/observations")
check_json_field "/memory/observations has observations key" "observations" "$R"

# ── /memory/observations/stats ───────────────────────────────────────────────

R=$(curl -sf "$BASE/memory/observations/stats")
check_json_field "/memory/observations/stats has total_observations" "total_observations" "$R"

# ── /audit ────────────────────────────────────────────────────────────────────

R=$(curl -sf "$BASE/audit")
check_json_field "/audit has entries key" "entries" "$R"

# ── /rpc/peers ────────────────────────────────────────────────────────────────

R=$(curl -sf "$BASE/rpc/peers")
check_json_field "/rpc/peers has peers key" "peers" "$R"
check_json_field "/rpc/peers has self key"  "self"  "$R"

# ── /rpc/ping (POST) ─────────────────────────────────────────────────────────

R=$(curl -sf -X POST "$BASE/rpc/ping" -H "Content-Type: application/json" -d '{}')
if echo "$R" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d.get('online') == True" 2>/dev/null; then
    pass "/rpc/ping returns online:true"
else
    fail "/rpc/ping" "expected online:true, got: $R"
fi
check_json_field "/rpc/ping has version field" "version" "$R"
check_json_field "/rpc/ping has name field"    "name"    "$R"

# ── /rpc/task/assign — bad auth must return 401 ───────────────────────────────

STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BASE/rpc/task/assign" \
    -H "Content-Type: application/json" \
    -H "X-Cluster-Auth: badtoken" \
    -d '{"agent":"master","prompt":"test"}')
if [ "$STATUS" = "401" ]; then
    pass "/rpc/task/assign rejects bad auth (401)"
else
    fail "/rpc/task/assign auth" "expected 401, got $STATUS"
fi

# ── /rpc/task/assign — no auth header must also return 401 ───────────────────

STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BASE/rpc/task/assign" \
    -H "Content-Type: application/json" \
    -d '{"agent":"master","prompt":"test"}')
if [ "$STATUS" = "401" ]; then
    pass "/rpc/task/assign requires auth header (401)"
else
    fail "/rpc/task/assign no-auth" "expected 401, got $STATUS"
fi

# ── /rpc/task/status — unknown job returns 404 ───────────────────────────────

STATUS=$(curl -s -o /dev/null -w "%{http_code}" "$BASE/rpc/task/status/nonexistent-job-id")
if [ "$STATUS" = "404" ]; then
    pass "/rpc/task/status returns 404 for unknown job"
else
    fail "/rpc/task/status unknown job" "expected 404, got $STATUS"
fi

# ── /conversations/list ───────────────────────────────────────────────────────

R=$(curl -sf "$BASE/conversations/list")
check_json_field "/conversations/list has conversations key" "conversations" "$R"

# ── /conversations/history (default daemon session) ───────────────────────────

R=$(curl -sf "$BASE/conversations/history")
check_json_field "/conversations/history has messages key" "messages" "$R"

# ── /conversations/:chat_id/history ──────────────────────────────────────────

R=$(curl -sf "$BASE/conversations/test-integration/history")
check_json_field "/conversations/:chat_id/history has messages key" "messages"  "$R"
check_json_field "/conversations/:chat_id/history has chat_id key"  "chat_id"   "$R"

# ── /conversations/:chat_id/reset ────────────────────────────────────────────

STATUS=$(curl -sf -o /dev/null -w "%{http_code}" -X POST "$BASE/conversations/test-reset/reset")
if [ "$STATUS" = "200" ]; then
    pass "/conversations/:chat_id/reset returns 200"
else
    fail "/conversations/:chat_id/reset" "expected 200, got $STATUS"
fi

R=$(curl -sf -X POST "$BASE/conversations/test-reset/reset")
check_json_field "/conversations/:chat_id/reset returns reset field" "reset" "$R"

# ── /oauth/apple/available ────────────────────────────────────────────────────

R=$(curl -sf "$BASE/oauth/apple/available")
check_json_field "/oauth/apple/available has available field" "available" "$R"

# ── /oauth/result ─────────────────────────────────────────────────────────────

R=$(curl -sf "$BASE/oauth/result")
check_json_field "/oauth/result has ok field" "ok" "$R"

# ── Daemon resilience — it should still respond after agent run without LLM ──

# POST to /agent/master/run with no LLM configured — should return an error
# response but NOT crash the daemon.
curl -sf -X POST "$BASE/agent/master/run" \
    -H "Content-Type: application/json" \
    -d '{"prompt":"hello","chat_id":"integration-test"}' > /dev/null 2>&1 || true

if curl -sf "$BASE/health" > /dev/null 2>&1; then
    pass "daemon survives /agent/:name/run call without LLM"
else
    fail "daemon resilience" "daemon became unreachable after agent call"
fi

# ── /scan/hardware ────────────────────────────────────────────────────────────

STATUS=$(curl -sf -o /dev/null -w "%{http_code}" "$BASE/scan/hardware")
if [ "$STATUS" = "200" ]; then
    pass "/scan/hardware returns 200"
else
    fail "/scan/hardware" "expected 200, got $STATUS"
fi

# ── /scan/credentials ────────────────────────────────────────────────────────

STATUS=$(curl -sf -o /dev/null -w "%{http_code}" "$BASE/scan/credentials")
if [ "$STATUS" = "200" ]; then
    pass "/scan/credentials returns 200"
else
    fail "/scan/credentials" "expected 200, got $STATUS"
fi

# ── Final health check ────────────────────────────────────────────────────────

if curl -sf "$BASE/health" > /dev/null 2>&1; then
    pass "final health check — daemon still alive"
else
    fail "final health check" "daemon unreachable at end of test suite"
fi

# ── Summary ───────────────────────────────────────────────────────────────────

echo ""
echo "========================="
echo "Results: ${PASS} passed, ${FAIL} failed"
echo "========================="

[ "$FAIL" -eq 0 ] && exit 0 || exit 1
