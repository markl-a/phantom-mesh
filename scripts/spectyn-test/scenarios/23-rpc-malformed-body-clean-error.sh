#!/usr/bin/env bash
source "$SPECTYN_TEST_LIB/common.sh"
source "$SPECTYN_TEST_LIB/cluster-rpc.sh"

scenario "RPC — malformed bodies fail cleanly (sync 4xx OR async job→failed), never panic"

ASSERT_HTTP "$(rpc_url)/healthz" 200 "serve up"

# Spectyn's RPC layer mixes two error modes:
#   - structurally invalid input  → sync HTTP 4xx
#   - semantically wrong input    → HTTP 202 + job_id, then job ends in
#                                    state=failed (async dispatch error)
# Both are acceptable as long as the failure is visible somewhere and no
# 5xx / panic / dropped connection occurs.

post_with_hmac() {
    local body="$1"
    local auth
    auth=$(printf '%s' "$body" | openssl dgst -sha256 -hmac "$SPECTYN_CLUSTER_SECRET" -hex | awk '{print $2}')
    curl -sS -w "\n--HTTP-%{http_code}--" --max-time 5 \
        -X POST "$(rpc_url)/rpc/task/assign" \
        -H "X-Cluster-Auth: $auth" \
        -H "Content-Type: application/json" \
        -d "$body" 2>&1
}

extract_http() { printf '%s' "$1" | grep -oE 'HTTP-[0-9]+' | tr -dc '0-9'; }
extract_body() { printf '%s' "$1" | sed 's/--HTTP-[0-9]*--//'; }
extract_jobid() { printf '%s' "$1" | grep -oE '"job_id":"[^"]+"' | head -1 | sed -E 's/.*"job_id":"([^"]+)".*/\1/'; }

# ── Case A: structurally invalid JSON → sync 4xx ────────────────────────────
step "case A: non-JSON body → expect sync HTTP 4xx (parser rejects)"
respA=$(post_with_hmac 'this is not json{')
httpA=$(extract_http "$respA")
case "$httpA" in
    4*) pass "non-JSON body → HTTP $httpA" ;;
    5*) fail "non-JSON body → HTTP $httpA (server panic likely): $(extract_body "$respA" | head -c 200)" ;;
    *)  fail "non-JSON body → unexpected HTTP $httpA — should be 400 (server should not enqueue an unparseable body)" ;;
esac

# ── Case B: well-formed JSON, missing required `agent` field ────────────────
# 2026-05-02: spectyn currently SILENTLY DEFAULTS to the master agent when
# `agent` is missing. Whether that's a feature (graceful default) or a bug
# (security-relevant silent fallback) is a design call. This scenario only
# enforces what the server MUST guarantee: no 5xx, no panic, the job
# reaches a terminal state. The fallback behavior is recorded as a WARN
# so a future spec change is visible in the test log.
step "case B: missing 'agent' field → must reach terminal state without 5xx/panic"
respB=$(post_with_hmac '{"prompt":"hi"}')
httpB=$(extract_http "$respB")
bodyB=$(extract_body "$respB")
case "$httpB" in
    4*) pass "missing-agent → sync HTTP $httpB (rejected at parse time)" ;;
    5*) fail "missing-agent → HTTP $httpB (panic)" ;;
    202|200)
        jidB=$(extract_jobid "$bodyB")
        if [ -z "$jidB" ]; then
            fail "$httpB without job_id and without error: $bodyB"
        else
            step "  enqueued as $jidB; polling for terminal state (max 20s)"
            rpc_wait_done "$jidB" 20 || true
            stB=$(rpc_state "$jidB")
            case "$stB" in
                done|completed)
                    pass "missing-agent → reached terminal state ($stB; spectyn silently defaulted)"
                    warn "  finding: server treated missing-agent as default 'master' — review whether this is intended"
                    ;;
                failed|error)
                    pass "missing-agent → reached terminal state ($stB; rejected at dispatch)"
                    ;;
                *)
                    fail "missing-agent → job stuck in '$stB' (no terminal state in 20s)"
                    ;;
            esac
        fi
        ;;
    *)  fail "missing-agent → unexpected HTTP $httpB" ;;
esac

# ── Case C: well-formed JSON, agent name does not exist in agents.toml ──────
step "case C: unknown agent name → must reach terminal state without 5xx/panic"
respC=$(post_with_hmac '{"agent":"definitely-not-a-real-agent-9911","prompt":"hi"}')
httpC=$(extract_http "$respC")
bodyC=$(extract_body "$respC")
case "$httpC" in
    4*) pass "unknown-agent → sync HTTP $httpC" ;;
    5*) fail "unknown-agent → HTTP $httpC (panic)" ;;
    202|200)
        jidC=$(extract_jobid "$bodyC")
        if [ -z "$jidC" ]; then
            fail "$httpC without job_id and without error: $bodyC"
        else
            step "  enqueued as $jidC; polling for terminal state (max 20s)"
            rpc_wait_done "$jidC" 20 || true
            stC=$(rpc_state "$jidC")
            case "$stC" in
                done|completed)
                    pass "unknown-agent → reached terminal state ($stC; spectyn silently defaulted)"
                    warn "  finding: server accepted nonexistent agent and ran the default — review intent"
                    ;;
                failed|error)
                    pass "unknown-agent → reached terminal state ($stC; rejected at dispatch)"
                    ;;
                *)
                    fail "unknown-agent → job stuck in '$stC' (no terminal state in 20s)"
                    ;;
            esac
        fi
        ;;
    *)  fail "unknown-agent → unexpected HTTP $httpC" ;;
esac

[ "$SPECTYN_TEST_FAILED" -eq 0 ]
