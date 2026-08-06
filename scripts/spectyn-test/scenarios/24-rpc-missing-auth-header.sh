#!/usr/bin/env bash
source "$SPECTYN_TEST_LIB/common.sh"
source "$SPECTYN_TEST_LIB/cluster-rpc.sh"

scenario "RPC — POST without X-Cluster-Auth header is rejected (auth-bypass guard)"

ASSERT_HTTP "$(rpc_url)/healthz" 200 "serve up"

step "POST /rpc/task/assign with no X-Cluster-Auth header at all"
resp=$(curl -sS -w "\n--HTTP-%{http_code}--" --max-time 5 \
    -X POST "$(rpc_url)/rpc/task/assign" \
    -H "Content-Type: application/json" \
    -d '{"agent":"master","prompt":"x"}' 2>&1)
http=$(printf '%s' "$resp" | grep -oE 'HTTP-[0-9]+' | tr -dc '0-9')
body=$(printf '%s' "$resp" | sed 's/--HTTP-[0-9]*--//')

step "got HTTP $http"

# Acceptable outcomes (in priority order):
#   1. 401 / 403 — explicit auth failure
#   2. 200 with body containing "unauthorized" / "missing" / "header"
# UNACCEPTABLE:
#   - 200 with a job_id (auth bypass — should NEVER happen)
#   - 5xx (panic — also a bug)
case "$http" in
    401|403)
        pass "missing X-Cluster-Auth → HTTP $http (proper auth rejection)"
        ;;
    200)
        if printf '%s' "$body" | grep -qiE 'unauthorized|missing|header|x-cluster-auth'; then
            pass "200 with explicit auth-error body: ${body:0:120}"
        elif printf '%s' "$body" | grep -q '"job_id"'; then
            fail "🚨 AUTH BYPASS — server accepted unauthenticated POST and returned a job_id: ${body:0:200}"
        else
            fail "200 OK with no clear auth error, no job_id either: ${body:0:200}"
        fi
        ;;
    5*)
        fail "HTTP $http (server error — auth check should not panic): ${body:0:200}"
        ;;
    000)
        fail "connection failed"
        ;;
    *)
        fail "unexpected HTTP $http: ${body:0:200}"
        ;;
esac

step "POST with HEADER PRESENT but VALUE EMPTY"
resp2=$(curl -sS -w "\n--HTTP-%{http_code}--" --max-time 5 \
    -X POST "$(rpc_url)/rpc/task/assign" \
    -H "X-Cluster-Auth: " \
    -H "Content-Type: application/json" \
    -d '{"agent":"master","prompt":"x"}' 2>&1)
http2=$(printf '%s' "$resp2" | grep -oE 'HTTP-[0-9]+' | tr -dc '0-9')
body2=$(printf '%s' "$resp2" | sed 's/--HTTP-[0-9]*--//')
case "$http2" in
    401|403) pass "empty auth value → HTTP $http2" ;;
    200)
        if printf '%s' "$body2" | grep -q '"job_id"'; then
            fail "🚨 AUTH BYPASS — empty auth header accepted: ${body2:0:200}"
        elif printf '%s' "$body2" | grep -qiE 'unauthorized|invalid|empty'; then
            pass "200 with explicit error: ${body2:0:120}"
        else
            fail "200 with neither error nor job_id: ${body2:0:200}"
        fi
        ;;
    *) fail "unexpected HTTP $http2 on empty auth value" ;;
esac

[ "$SPECTYN_TEST_FAILED" -eq 0 ]
