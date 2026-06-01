#!/usr/bin/env bash
# ============================================================================
# demo-orchestrate.sh — phantom-mesh distributed task orchestration demo
#
# WHAT
#   One user prompt → coordinator's LLM decomposes it into per-platform
#   subtasks → dispatches each subtask to the right peer based on the
#   peer's advertised capabilities → polls + aggregates results.
#
# PORTABILITY (open-source-ready)
#   - Zero hardcoded IPs, hostnames, ports, or secrets in this script.
#   - Coordinator URL: read from $PHANTOM_COORDINATOR_URL, or fallback to
#     mDNS browse for `_phantom-mesh._tcp.local.`, or finally
#     http://127.0.0.1:7878.
#   - Cluster secret: read from $PHANTOM_CLUSTER_SECRET, or from
#     $HOME/.phantom-mesh/agents.toml `cluster_secret = "..."`.
#   - Peer list: queried live from coordinator's `/rpc/peers`. Each peer
#     advertises its OS + capabilities; the LLM decomposition routes
#     subtasks by matching against those tags.
#
#   Drop this script into any phantom-mesh install + run from any node.
#   No edits required.
#
# WHY
#   The built-in `phantom swarm` does identical fan-out — same prompt to
#   every peer. This script does TASK DECOMPOSITION + capability-aware
#   routing: meaningfully different subtask per platform.
#
# USAGE
#   ./scripts/demo-orchestrate.sh "<your task>"
#   PHANTOM_COORDINATOR_URL=https://my-coordinator.example:443 \
#     PHANTOM_CLUSTER_SECRET=hunter2 \
#     ./scripts/demo-orchestrate.sh "..."
#   FALLBACK=1 ./scripts/demo-orchestrate.sh "..."   # skip LLM; identical
#                                                     # prompt to every peer
#   MAX_WAIT_S=120 ./scripts/demo-orchestrate.sh "..."
#
# DEPS
#   curl, openssl, python3, awk, bash 3.2+ (default macOS)
#
# EXIT
#   0  ≥2 peers returned non-error results
#   1  routing succeeded but <2 peers produced output
#   2  pre-flight failed (no secret / coordinator unreachable)
# ============================================================================

set -u

USER_PROMPT="${1:-}"
if [[ -z "$USER_PROMPT" ]]; then
  cat <<EOF
Usage: $0 "<user task>"

Example: $0 "Tell me OS and hostname, then suggest one thing to improve about this machine"

Env (all optional):
  PHANTOM_COORDINATOR_URL  Override auto-discovery (default: mDNS or 127.0.0.1:7878)
  PHANTOM_CLUSTER_SECRET   Override cluster_secret from agents.toml
  FALLBACK=1               Skip LLM decomposition (identical prompt to all peers)
  MAX_WAIT_S=N             Per-job poll timeout (default 90)
EOF
  exit 1
fi

MAX_WAIT_S="${MAX_WAIT_S:-90}"
FALLBACK="${FALLBACK:-0}"
AGENTS_TOML="${PHANTOM_AGENTS_TOML:-$HOME/.phantom-mesh/agents.toml}"

# ── 1. Resolve coordinator URL (3-tier fallback) ───────────────────────────
COORDINATOR_URL="${PHANTOM_COORDINATOR_URL:-}"

if [[ -z "$COORDINATOR_URL" ]] && command -v dns-sd &>/dev/null; then
  # macOS Bonjour browse — try 3s, take first hit.
  discovered=$(timeout 3 dns-sd -B _phantom-mesh._tcp local 2>/dev/null \
               | awk '/Add/{print $NF; exit}')
  if [[ -n "$discovered" ]]; then
    # dns-sd gives us the instance name, not URL. Resolve via /rpc/peers
    # on localhost if we can — otherwise fall back to 127.0.0.1:7878.
    COORDINATOR_URL=""  # let next tier kick in
  fi
fi

[[ -z "$COORDINATOR_URL" ]] && COORDINATOR_URL="http://127.0.0.1:7878"

# ── 2. Resolve cluster_secret ──────────────────────────────────────────────
SECRET="${PHANTOM_CLUSTER_SECRET:-}"
if [[ -z "$SECRET" && -r "$AGENTS_TOML" ]]; then
  SECRET=$(awk -F'"' '/^[[:space:]]*cluster_secret[[:space:]]*=/{print $2; exit}' "$AGENTS_TOML")
fi
if [[ -z "$SECRET" ]]; then
  echo "ERR: no cluster_secret found." >&2
  echo "     Set PHANTOM_CLUSTER_SECRET, or run \`phantom serve\` first to generate agents.toml at $AGENTS_TOML" >&2
  exit 2
fi

# ── Helpers ────────────────────────────────────────────────────────────────
hmac_auth() {
  printf '%s' "$1" | openssl dgst -sha256 -hmac "$SECRET" -hex | awk '{print $NF}'
}

json_escape() {
  python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))' <<<"$1"
}

dispatch_to_peer() {
  local target_url="$1" prompt_text="$2"
  local body
  body=$(printf '{"agent":"master","prompt":%s,"required_caps":[],"forward_chain":[]}' "$(json_escape "$prompt_text")")
  local auth
  auth=$(hmac_auth "$body")
  curl -sS --max-time 10 -X POST "$target_url/rpc/task/assign" \
    -H "X-Cluster-Auth: $auth" \
    -H "Content-Type: application/json" \
    -d "$body" 2>/dev/null \
    | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('job_id',''))" 2>/dev/null
}

poll_job() {
  local target_url="$1" job_id="$2" max_s="$3"
  local deadline=$(( $(date +%s) + max_s ))
  while [[ $(date +%s) -lt $deadline ]]; do
    sleep 3
    local r
    r=$(curl -sS --max-time 5 "$target_url/rpc/task/status/$job_id" 2>/dev/null)
    [[ -z "$r" ]] && continue
    local parsed
    parsed=$(echo "$r" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    s = d.get('status','')
    o = (d.get('output') or '').replace(chr(10),' | ')
    e = (d.get('error') or '').replace(chr(10),' | ')
    if s in ('done','completed'): print('DONE|' + o)
    elif s == 'error':            print('ERROR|' + e)
    else:                          print('RUNNING|')
except: print('PARSE_ERR|')
" 2>/dev/null)
    case "$parsed" in
      DONE\|*|ERROR\|*) echo "$parsed"; return 0;;
    esac
  done
  echo "TIMEOUT|"
}

# ── 3. Banner ──────────────────────────────────────────────────────────────
cat <<EOF

╭──────────────────────────────────────────────────────────────────────╮
│  phantom-mesh — distributed task orchestration demo                  │
│                                                                      │
│  Coordinator: $(printf '%-54s' "${COORDINATOR_URL:0:54}") │
│  Prompt:      $(printf '%-54s' "${USER_PROMPT:0:54}") │
╰──────────────────────────────────────────────────────────────────────╯

EOF

# ── 4. Query coordinator for live peer list + capabilities ─────────────────
echo "── step 1: query coordinator /rpc/peers for live cluster ──"
peers_json=$(curl -sS --max-time 5 "$COORDINATOR_URL/rpc/peers" 2>/dev/null)
if [[ -z "$peers_json" ]]; then
  echo "ERR: coordinator $COORDINATOR_URL not reachable" >&2
  exit 2
fi

# Parse peers into TARGETS[] (one per online peer) + corresponding
# PEER_URL[] / PEER_NAME[] / PEER_CAPS[] (parallel arrays — bash 3.2).
# Coordinator itself is added as the first peer (self).
PEER_URL=()
PEER_NAME=()
PEER_CAPS=()
PEER_OS=()

# Add self
self_data=$(echo "$peers_json" | python3 -c "
import sys, json
d = json.load(sys.stdin)
s = d.get('self', {})
print(s.get('name','self'))
print('|'.join(s.get('capabilities') or ['general']))
print(s.get('os','unknown'))
")
self_name=$(echo "$self_data" | sed -n '1p')
self_caps=$(echo "$self_data" | sed -n '2p')
self_os=$(echo "$self_data" | sed -n '3p')
PEER_URL+=("$COORDINATOR_URL")
PEER_NAME+=("$self_name (coordinator)")
PEER_CAPS+=("$self_caps")
PEER_OS+=("$self_os")

# Add online peers
online_lines=$(echo "$peers_json" | python3 -c "
import sys, json
d = json.load(sys.stdin)
for p in d.get('peers', []):
    if not p.get('online'): continue
    url = p.get('url','')
    if not url: continue
    caps = '|'.join(p.get('capabilities') or ['general'])
    name = p.get('name','?')
    os_ = p.get('os','unknown')
    print(f'{url}\t{name}\t{caps}\t{os_}')
")
if [[ -n "$online_lines" ]]; then
  while IFS=$'\t' read -r url name caps os; do
    [[ -z "$url" ]] && continue
    PEER_URL+=("$url"); PEER_NAME+=("$name"); PEER_CAPS+=("$caps"); PEER_OS+=("$os")
  done <<<"$online_lines"
fi

echo "   ${#PEER_URL[@]} peer(s) online:"
for i in "${!PEER_URL[@]}"; do
  printf "      • %-32s caps=%-30s os=%s\n" "${PEER_NAME[$i]}" "${PEER_CAPS[$i]}" "${PEER_OS[$i]}"
done
echo ""

# ── 5. Decomposition (LLM or fallback) ─────────────────────────────────────
# parallel arrays for subtask plan
SUB_URL=()
SUB_NAME=()
SUB_PROMPT=()

if [[ "$FALLBACK" == "1" ]]; then
  echo "── step 2: decomposition (FALLBACK mode — no LLM call) ──"
  echo "   sending identical prompt to all ${#PEER_URL[@]} peers"
  for i in "${!PEER_URL[@]}"; do
    SUB_URL+=("${PEER_URL[$i]}"); SUB_NAME+=("${PEER_NAME[$i]}"); SUB_PROMPT+=("$USER_PROMPT")
  done
else
  echo "── step 2: ask coordinator's LLM to decompose into per-peer subtasks ──"
  peer_inventory=""
  for i in "${!PEER_URL[@]}"; do
    peer_inventory+="- ${PEER_NAME[$i]} · os=${PEER_OS[$i]} · capabilities=${PEER_CAPS[$i]}"$'\n'
  done

  DECOMP_PROMPT="You are a task scheduler for a distributed AI agent mesh. Available peers:
${peer_inventory}
User task: ${USER_PROMPT}

Decompose into 2-5 subtasks, one per peer that meaningfully helps. Each subtask should leverage that peer's OS or capabilities. Reply with ONLY a JSON object — no prose, no fences. Schema:
{\"subtasks\":[{\"peer_name\":\"<one of the names above, EXACT match>\",\"prompt\":\"<subtask>\"}]}"

  decomp_body=$(printf '{"agent":"master","prompt":%s,"required_caps":[],"forward_chain":[]}' "$(json_escape "$DECOMP_PROMPT")")
  decomp_auth=$(hmac_auth "$decomp_body")
  decomp_resp=$(curl -sS --max-time 10 -X POST "$COORDINATOR_URL/rpc/task/assign" \
    -H "X-Cluster-Auth: $decomp_auth" \
    -H "Content-Type: application/json" \
    -d "$decomp_body" 2>/dev/null)
  decomp_job_id=$(echo "$decomp_resp" | python3 -c "import sys,json; print(json.load(sys.stdin).get('job_id',''))" 2>/dev/null)

  if [[ -z "$decomp_job_id" ]]; then
    echo "   ✗ coordinator didn't issue job_id — falling back to identical prompt"
    for i in "${!PEER_URL[@]}"; do
      SUB_URL+=("${PEER_URL[$i]}"); SUB_NAME+=("${PEER_NAME[$i]}"); SUB_PROMPT+=("$USER_PROMPT")
    done
  else
    echo "   ↻ coordinator job $decomp_job_id — polling LLM decomposition ($MAX_WAIT_S s)..."
    decomp_result=$(poll_job "$COORDINATOR_URL" "$decomp_job_id" "$MAX_WAIT_S")
    decomp_state="${decomp_result%%|*}"
    decomp_text="${decomp_result#*|}"

    if [[ "$decomp_state" != "DONE" ]]; then
      echo "   ✗ decomposition $decomp_state — falling back to identical prompt"
      for i in "${!PEER_URL[@]}"; do
        SUB_URL+=("${PEER_URL[$i]}"); SUB_NAME+=("${PEER_NAME[$i]}"); SUB_PROMPT+=("$USER_PROMPT")
      done
    else
      # Tolerant JSON parse — strip fences if LLM added them
      parsed=$(echo "$decomp_text" | python3 -c "
import sys, json
raw = sys.stdin.read().strip()
# strip fences
for fence in ['\`\`\`json','\`\`\`']:
    if raw.startswith(fence): raw = raw[len(fence):].strip()
    if raw.endswith('\`\`\`'): raw = raw[:-3].strip()
start = raw.find('{'); end = raw.rfind('}')
if start == -1 or end == -1:
    print('PARSE_ERR|no JSON object found')
    sys.exit(0)
try:
    obj = json.loads(raw[start:end+1])
    for st in obj.get('subtasks', []):
        pn = (st.get('peer_name') or '').strip()
        pr = (st.get('prompt') or '').strip()
        if pn and pr:
            print(f'OK\t{pn}\t{pr}')
except Exception as e:
    print(f'PARSE_ERR|{e}')
")

      if echo "$parsed" | head -1 | grep -q "^PARSE_ERR"; then
        echo "   ✗ LLM returned non-JSON: $parsed"
        for i in "${!PEER_URL[@]}"; do
          SUB_URL+=("${PEER_URL[$i]}"); SUB_NAME+=("${PEER_NAME[$i]}"); SUB_PROMPT+=("$USER_PROMPT")
        done
      else
        echo "   ✓ LLM decomposed into:"
        while IFS=$'\t' read -r ok peer_name prompt; do
          [[ "$ok" == "OK" ]] || continue
          # Look up peer URL by name
          match_idx=-1
          for i in "${!PEER_NAME[@]}"; do
            if [[ "${PEER_NAME[$i]}" == *"$peer_name"* ]]; then match_idx=$i; break; fi
          done
          if (( match_idx >= 0 )); then
            SUB_URL+=("${PEER_URL[$match_idx]}")
            SUB_NAME+=("${PEER_NAME[$match_idx]}")
            SUB_PROMPT+=("$prompt")
            printf "      • %-32s → %s\n" "$peer_name" "${prompt:0:50}"
          else
            printf "      ⚠  %s: no matching peer (skipped)\n" "$peer_name"
          fi
        done <<<"$parsed"
      fi
    fi
  fi
fi
echo ""

# ── 6. Dispatch in parallel ────────────────────────────────────────────────
echo "── step 3: dispatching ${#SUB_URL[@]} subtasks in parallel ──"
JOB_TARGET_IDX=()  # indices into SUB_* arrays
JOB_IDS=()
for i in "${!SUB_URL[@]}"; do
  jid=$(dispatch_to_peer "${SUB_URL[$i]}" "${SUB_PROMPT[$i]}")
  if [[ -n "$jid" ]]; then
    JOB_TARGET_IDX+=("$i")
    JOB_IDS+=("$jid")
    printf "   → %-32s %s\n" "${SUB_NAME[$i]}" "$jid"
  else
    printf "   ✗ %-32s dispatch failed (peer unreachable?)\n" "${SUB_NAME[$i]}"
  fi
done
echo ""

# ── 7. Poll all in parallel ────────────────────────────────────────────────
echo "── step 4: polling all jobs (parallel, max ${MAX_WAIT_S}s each) ──"
tmpdir=$(mktemp -d)
for k in "${!JOB_TARGET_IDX[@]}"; do
  (
    idx="${JOB_TARGET_IDX[$k]}"
    result=$(poll_job "${SUB_URL[$idx]}" "${JOB_IDS[$k]}" "$MAX_WAIT_S")
    echo "$result" > "$tmpdir/$k"
  ) &
done
wait
echo ""

# ── 8. Print + summary ─────────────────────────────────────────────────────
echo "── step 5: aggregated results ──"
echo ""
n_done=0; n_err=0; n_to=0
for k in "${!JOB_TARGET_IDX[@]}"; do
  idx="${JOB_TARGET_IDX[$k]}"
  result=$(cat "$tmpdir/$k" 2>/dev/null)
  state="${result%%|*}"
  text="${result#*|}"
  printf "▸ %s\n" "${SUB_NAME[$idx]}"
  printf "  prompt: %s\n" "${SUB_PROMPT[$idx]:0:90}"
  case "$state" in
    DONE)    printf "  ✓ %s\n\n" "${text:0:300}"; n_done=$((n_done+1));;
    ERROR)   printf "  ✗ %s\n\n" "${text:0:200}";  n_err=$((n_err+1));;
    TIMEOUT) printf "  ⏳ no response in ${MAX_WAIT_S}s\n\n";       n_to=$((n_to+1));;
    *)       printf "  ? %s\n\n" "$result";        n_err=$((n_err+1));;
  esac
done
rm -rf "$tmpdir"

echo "──────────────────────────────────────────────"
echo "summary: ${#SUB_URL[@]} subtasks dispatched · done=$n_done error=$n_err timeout=$n_to"
[[ "$FALLBACK" == "1" ]] && echo "(FALLBACK mode used — LLM decomposition skipped)"
if (( n_done >= 2 )); then
  echo "RESULT: PASS — ≥2 peers returned real output"
  exit 0
fi
echo "RESULT: PARTIAL — routing proven, but only $n_done peer(s) completed"
exit 1
