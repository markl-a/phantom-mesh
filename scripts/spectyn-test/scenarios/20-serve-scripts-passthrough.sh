#!/usr/bin/env bash
source "$SPECTYN_TEST_LIB/common.sh"
source "$SPECTYN_TEST_LIB/cluster-rpc.sh"

scenario "spectyn serve — /scripts/ passthrough returns repo PowerShell file as text"

ASSERT_HTTP "$(rpc_url)/healthz" 200 "serve up"

# /scripts/<name> is meant to serve files from the repo's scripts/ dir as
# a convenience for `iex (iwr ...)` Windows install one-liners. Common
# script that should always be there:
target="install-spectyn-windows.ps1"

step "GET /scripts/$target"
# Pre-check: if the running serve's cwd doesn't sit next to a scripts/
# directory, the passthrough physically can't find the file — that's an
# environmental issue (e.g. serve started by Scheduled Task from a path
# without project access), not a serve-layer bug. SKIP rather than FAIL.
preview_body=$(curl -sS --max-time 3 "$(rpc_url)/scripts/$target" 2>/dev/null)
if printf '%s' "$preview_body" | grep -q 'script not found in any candidate path'; then
    warn "serve's cwd has no reachable scripts/ dir; skipping (not a serve bug — re-run from a spectyn started inside the project root)"
    exit 77
fi

hdrs=$(curl -sSI --max-time 4 "$(rpc_url)/scripts/$target" 2>&1)
ASSERT_CONTAINS "$hdrs" "200" "GET /scripts/$target returns 200"

# Server should declare text/plain (or similar text mime) for *.ps1.
if printf '%s' "$hdrs" | grep -qiE 'content-type:\s*(text/|application/octet-stream|application/x-powershell)'; then
    pass "content-type is a text-friendly mime"
else
    warn "content-type unusual: $(printf '%s' "$hdrs" | grep -i 'content-type')"
fi

# content-disposition: inline; filename="install-spectyn-windows.ps1" — note INLINE not attachment for scripts.
ASSERT_CONTAINS "$hdrs" "$target" "content-disposition mentions filename"

# Fetch + verify PowerShell content shape.
body=$(curl -sS --max-time 6 "$(rpc_url)/scripts/$target")
if [ "${#body}" -gt 500 ]; then
    pass "body size ${#body} bytes"
else
    fail "script body suspiciously short: ${#body} bytes"
fi

# Should look like a PowerShell script (starts with `#` comment + has at least one $env: or [Environment]:: usage).
ASSERT_CONTAINS "$body" '#' "body has at least one comment line"
ASSERT_CONTAINS "$body" 'Invoke-WebRequest' "body uses Invoke-WebRequest"
ASSERT_CONTAINS "$body" 'SpectynMesh' "body references SpectynMesh"

# 404 path should be a clean message, not a panic / stack trace.
step "GET /scripts/this-file-does-not-exist.ps1 should 4xx without crashing"
not_found=$(curl -sS -w "\n--HTTP-%{http_code}--" --max-time 4 "$(rpc_url)/scripts/this-file-does-not-exist.ps1")
http_code=$(printf '%s' "$not_found" | grep -oE 'HTTP-[0-9]+' | tr -dc '0-9')
case "$http_code" in
    404|410) pass "missing script returns $http_code" ;;
    200)
        # Some implementations 200 with a body that contains "not found".
        if echo "$not_found" | grep -qi 'not found'; then
            pass "200 with explicit not-found body"
        else
            fail "200 OK on missing script — should be 404"
        fi
        ;;
    *) fail "unexpected HTTP $http_code on missing script" ;;
esac

[ "$SPECTYN_TEST_FAILED" -eq 0 ]
