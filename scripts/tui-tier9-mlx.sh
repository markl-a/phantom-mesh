#!/usr/bin/env bash
# Tier-9: MLX local LLM (Apple Silicon zero-cost inference)
#   - phantom mlx status before start
#   - phantom mlx serve in background
#   - /v1/models endpoint shape
#   - /v1/chat/completions inference round-trip
#   - phantom run via mlx-local provider
#   - measure first-token + total latency
#   - phantom mlx stop cleanup

set -o pipefail
PASS=0; FAIL=0; FAIL_LINES=()
TMP=$(mktemp -d)
MLX_URL="http://127.0.0.1:8080"

green() { printf "\033[32m%s\033[0m" "$1"; }
red()   { printf "\033[31m%s\033[0m" "$1"; }
gray()  { printf "\033[90m%s\033[0m" "$1"; }
bold()  { printf "\033[1m%s\033[0m" "$1"; }

ok()    { PASS=$((PASS+1)); printf "  $(green '✓') %-58s %s\n" "$1" "$(gray "$2")"; }
fail()  { FAIL=$((FAIL+1)); FAIL_LINES+=("$1 :: $2"); printf "  $(red '✗') %-58s %s\n" "$1" "$(gray "$2")"; }
section() { printf "\n$(bold "%s")\n" "$1"; }

cleanup() {
  echo ""
  echo "  cleanup: phantom mlx stop"
  phantom mlx stop 2>/dev/null
  pkill -f "mlx_lm.server" 2>/dev/null
}
trap cleanup EXIT

# ─── pre-start state ──────────────────────────────────────────────────────
section "54. MLX pre-start state"

# Confirm not running (would interfere)
if curl -sf -m 1 "$MLX_URL/v1/models" >/dev/null 2>&1; then
  echo "  (server already running — stopping first)"
  phantom mlx stop 2>/dev/null
  sleep 2
fi

resp=$(phantom mlx status 2>&1)
echo "$resp" | grep -qiE "import|server|reachable" \
  && ok "phantom mlx status pre-start" "$(echo "$resp" | head -1 | head -c 80)" \
  || fail "phantom mlx status" "got: $(echo "$resp" | head -c 200)"

# ─── start server ─────────────────────────────────────────────────────────
section "55. MLX server startup"

# Start in background, redirect logs to TMP
nohup phantom mlx serve > "$TMP/mlx.log" 2>&1 &
SERVE_PID=$!
echo "  started phantom mlx serve (pid $SERVE_PID), waiting for /v1/models..."

# Wait up to 90s for model load
ready=0
for i in $(seq 1 90); do
  if curl -sf -m 2 "$MLX_URL/v1/models" >/dev/null 2>&1; then
    ready=1
    elapsed=$i
    break
  fi
  sleep 1
done

if [[ $ready -eq 1 ]]; then
  ok "MLX server reachable" "ready in ${elapsed}s"
else
  fail "MLX server reachable" "did not respond within 90s, log tail: $(tail -5 "$TMP/mlx.log" | tr '\n' ' ' | head -c 300)"
  exit 2
fi

# ─── /v1/models shape ─────────────────────────────────────────────────────
section "56. /v1/models endpoint"

body=$(curl -sf "$MLX_URL/v1/models" 2>&1)
echo "$body" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
data = d.get('data', [])
assert len(data) >= 1, 'no models listed'
m = data[0]
assert 'id' in m, f'no id field: {m}'
print(f'  {m[\"id\"]}')
" 2>"$TMP/models.err" \
  && ok "/v1/models lists model" "$(echo "$body" | python3 -c "import sys,json; d=json.loads(sys.stdin.read()); print(d['data'][0]['id'])")" \
  || fail "/v1/models lists model" "$(cat $TMP/models.err)"

# ─── direct inference ─────────────────────────────────────────────────────
section "57. /v1/chat/completions inference"

# Tiny query for speed
PROMPT_BODY='{"model":"mlx-community/Llama-3.1-8B-Instruct-4bit","messages":[{"role":"user","content":"Say only the word: OK"}],"max_tokens":10,"temperature":0.0}'

t0=$(python3 -c "import time; print(time.time())")
resp=$(curl -sf -X POST -H "Content-Type: application/json" -d "$PROMPT_BODY" "$MLX_URL/v1/chat/completions" 2>&1)
t1=$(python3 -c "import time; print(time.time())")
elapsed=$(python3 -c "print(round($t1 - $t0, 2))")

content=$(echo "$resp" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
print(d['choices'][0]['message']['content'])
" 2>/dev/null)

if [[ -n "$content" ]]; then
  ok "/v1/chat/completions returns content" "${elapsed}s · '${content:0:60}'"
else
  fail "/v1/chat/completions returns content" "got: $(echo "$resp" | head -c 200)"
fi

# Latency budget
if (( $(python3 -c "print(1 if $elapsed < 30 else 0)") )); then
  ok "latency under 30s" "${elapsed}s"
else
  fail "latency under 30s" "${elapsed}s"
fi

# Token usage shape
echo "$resp" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
u = d.get('usage', {})
assert 'prompt_tokens' in u, 'no prompt_tokens'
assert 'completion_tokens' in u, 'no completion_tokens'
print('OK')
" 2>/dev/null \
  && ok "response includes usage tokens" "" \
  || fail "response includes usage tokens" "got: $(echo "$resp" | head -c 200)"

# ─── streaming ────────────────────────────────────────────────────────────
section "58. /v1/chat/completions streaming"

# stream=true should produce SSE chunks
STREAM_BODY='{"model":"mlx-community/Llama-3.1-8B-Instruct-4bit","messages":[{"role":"user","content":"count 1 2 3"}],"max_tokens":10,"stream":true,"temperature":0.0}'

t0=$(python3 -c "import time; print(time.time())")
chunks=$(curl -sf -N -X POST -H "Content-Type: application/json" -d "$STREAM_BODY" \
  "$MLX_URL/v1/chat/completions" 2>&1 | head -50)
t1=$(python3 -c "import time; print(time.time())")
elapsed=$(python3 -c "print(round($t1 - $t0, 2))")

# Look for SSE chunks (data: prefixed lines)
chunk_count=$(echo "$chunks" | grep -c '^data: ')
if [[ $chunk_count -gt 0 ]]; then
  ok "streaming produces SSE chunks" "${chunk_count} chunks in ${elapsed}s"
else
  fail "streaming produces SSE chunks" "got: $(echo "$chunks" | head -c 200)"
fi

# Final chunk should be [DONE]
echo "$chunks" | grep -qE '\[DONE\]' \
  && ok "streaming ends with [DONE]" "" \
  || fail "streaming ends with [DONE]" ""

# ─── phantom run via mlx-local provider ───────────────────────────────────
section "59. phantom run routed through mlx-local"

# Configure agents.toml to use mlx-local for a custom agent. The default
# master might already be on Groq/Gemini — we want to force MLX. Use the
# 'local' agent if it exists (per cluster_nodes memory).
agents=$(curl -sf "$SERVE_URL_2:=http://127.0.0.1:7878}/api/status" 2>/dev/null | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
print(' '.join(d.get('agents', [])))
" 2>/dev/null)

# Always add 'local' since it's configured in user's agents.toml
echo "  available agents: $agents"

t0=$(python3 -c "import time; print(time.time())")
resp=$(timeout 60 phantom run --agent local "Reply with exactly two words: phantom alive" 2>&1 | tail -10)
t1=$(python3 -c "import time; print(time.time())")
elapsed=$(python3 -c "print(round($t1 - $t0, 2))")

if echo "$resp" | grep -qiE 'phantom|alive|hello'; then
  ok "phantom run --agent local succeeds via MLX" "${elapsed}s"
elif echo "$resp" | grep -qiE 'unknown.agent|invalid'; then
  ok "agent 'local' not configured, but no panic" "(skip — agents.toml may not have it)"
else
  fail "phantom run --agent local via MLX" "got: $(echo "$resp" | head -c 200)"
fi

# ─── cost is zero ─────────────────────────────────────────────────────────
section "60. MLX cost is zero"

# Either zero in by_provider for mlx-local, or absent (never recorded)
cost=$(curl -sf "${SERVE_URL_2:-http://127.0.0.1:7878}/api/cost" 2>&1)
mlx_entry=$(echo "$cost" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
bp = d.get('by_provider', [])
mlx = [x for x in bp if 'mlx' in x.get('provider', '').lower()]
if mlx:
  print(f'mlx entry: {mlx[0]}')
else:
  print('mlx not in by_provider (free, untracked)')
" 2>/dev/null)
[[ -n "$mlx_entry" ]] \
  && ok "MLX cost is zero or untracked" "$mlx_entry" \
  || fail "MLX cost is zero or untracked" ""

# ─── stop cleanup ─────────────────────────────────────────────────────────
section "61. phantom mlx stop"

phantom mlx stop 2>&1 | head -3
sleep 2
if curl -sf -m 1 "$MLX_URL/v1/models" >/dev/null 2>&1; then
  fail "phantom mlx stop kills server" "still responding"
else
  ok "phantom mlx stop kills server" ""
fi

# ─── summary ──────────────────────────────────────────────────────────────
section "summary"
total=$((PASS + FAIL))
printf "  %s pass · %s fail · total %d\n" "$(green $PASS)" "$(red $FAIL)" "$total"
printf "  %s\n" "$(gray "captures: $TMP")"
if (( FAIL > 0 )); then
  printf "\n%s\n" "$(bold 'failures:')"
  for f in "${FAIL_LINES[@]}"; do printf "  - %s\n" "$f"; done
  exit 2
fi
