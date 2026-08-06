#!/usr/bin/env bash
# smoke-chat-code.sh — Phase 1 seed: verify onboarding→chat→code on a node.
# Non-destructive read/exec smoke. Run on any full node (Mac/Win/Linux/Termux).
#
#   bash scripts/smoke-chat-code.sh
#
# Exits non-zero on first failed gate. Captures the lifecycle scenarios proven
# on Mac 2026-06-02 (chat, code-via-tools, recall). Login/logout (interactive
# broker OAuth) are NOT covered here — they're the user-driven part of Phase A.
set -uo pipefail

PASS=0 FAIL=0
ok()   { echo "  ✓ $1"; PASS=$((PASS+1)); }
bad()  { echo "  ✗ $1"; FAIL=$((FAIL+1)); }
hdr()  { echo; echo "== $1 =="; }

command -v spectyn >/dev/null || { echo "spectyn not on PATH"; exit 2; }

hdr "0. node health"
spectyn --version >/dev/null 2>&1 && ok "spectyn binary runs" || bad "spectyn --version"
curl -sf http://127.0.0.1:7878/healthz >/dev/null 2>&1 \
  && ok "serve healthz 200 (localhost)" || echo "  ⚠ serve not up on 7878 (exec may run inline)"

hdr "1. chat (exec returns an answer)"
ANS=$(timeout 90 spectyn exec --quiet "Reply with exactly: PONG" </dev/null 2>/dev/null | tail -1)
[ -n "$ANS" ] && ok "chat answered: ${ANS:0:40}" || bad "chat returned empty"

hdr "2. code (agent uses a tool to create a file)"
# Write inside an allowed workspace root — file_write sandboxes to the repo +
# ~/.spectyn-mesh (it correctly REJECTS /tmp). Use ~/.spectyn-mesh here.
T="$HOME/.spectyn-mesh/spectyn_smoke_$$.txt"; rm -f "$T"
timeout 120 spectyn exec --quiet "Use your file_write tool to write the single line SMOKE_OK into the absolute path $T" </dev/null >/dev/null 2>&1
if [ -f "$T" ] && grep -q SMOKE_OK "$T"; then ok "agent created $T with SMOKE_OK"; rm -f "$T"; else bad "code task did not produce $T"; fi

hdr "3. recall (read back life-node events)"
N=$(spectyn recall "" --json --limit 5 </dev/null 2>/dev/null | python3 -c "import sys,json;print(len(json.load(sys.stdin)))" 2>/dev/null)
[ -n "${N:-}" ] && ok "recall returned $N event(s)" || bad "recall failed/empty"

echo; echo "== result: $PASS passed, $FAIL failed =="
[ "$FAIL" -eq 0 ]
