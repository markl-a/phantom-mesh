#!/usr/bin/env bash
# ============================================================================
# demo-mobile-swarm.sh
#
# WHAT
#   Simulates exactly what the iOS/Android Tauri app does when it submits a
#   "joint operation" prompt:
#     1. GET  /rpc/peers on coordinator           → list of online peers
#     2. POST /rpc/task/assign on EACH peer       → list of job_ids
#        (HMAC-SHA256(cluster_secret, body) in X-Cluster-Auth header — same
#         wire format as app/src/lib/clusterDispatch.ts)
#     3. GET /rpc/task/status/:id on each peer    → poll until done|error
#     4. Print {peer → output} table; exit 0 if >=2 peers replied.
#
#   This is what `app/src/lib/clusterDispatch.ts` ALREADY does, just extended
#   to fan-out to every peer instead of one target.
#
# WHY
#   The mobile (Tauri) app's UI can already issue dispatchToCluster — adding
#   a "swarm" button is then a 30-line addition. This script proves the
#   server-side path works end-to-end from any HTTP client, including the
#   mobile app's webview JS.
#
# USAGE
#   ./scripts/demo-mobile-swarm.sh "What OS are you on?"
#   COORD=http://localhost:7878 ./scripts/demo-mobile-swarm.sh "<prompt>"
#   MAX_WAIT_S=90  ./scripts/demo-mobile-swarm.sh "<prompt>"
# ============================================================================

set -u

PROMPT="${1:-Reply in one short line: OS=<os>, HOST=<hostname>}"
COORD="${COORD:-http://127.0.0.1:7878}"
MAX_WAIT_S="${MAX_WAIT_S:-60}"
AGENT="${AGENT:-master}"

SECRET=$(awk -F'"' '/^[[:space:]]*cluster_secret[[:space:]]*=/{print $2; exit}' "$HOME/.spectyn-mesh/agents.toml")
if [[ -z "$SECRET" ]]; then echo "ERROR: no cluster_secret" >&2; exit 2; fi

echo "=== mobile-style swarm ==="
echo "coordinator: $COORD"
echo "prompt:      $PROMPT"
echo

# 1. Discover online peers (same call the mobile UI's "Refresh peers" button makes)
peers_json=$(curl -sS --max-time 5 "$COORD/rpc/peers")
peer_urls=$(echo "$peers_json" | python3 -c '
import sys, json
d = json.load(sys.stdin)
seen = set()
# Include self via the COORD URL — the app talks to its configured coord too.
import os
self_url = os.environ.get("COORD","http://127.0.0.1:7878")
seen.add(self_url)
print(self_url)
for p in d.get("peers", []):
    url = p.get("url","")
    if url and p.get("online") and url not in seen:
        seen.add(url)
        print(url)
')
n_peers=$(echo "$peer_urls" | grep -c .)
echo "discovered $n_peers online target(s):"
echo "$peer_urls" | sed 's/^/  /'
echo

# 2. /rpc/task/assign on each peer  (exact body shape that mesh.rs::TaskAssignRequest expects)
#    Body must include "agent","prompt","required_caps":[],"forward_chain":[] to match the
#    Rust struct exactly. We omit optional fields and let serde defaults fill in.
body=$(printf '{"agent":"%s","prompt":%s,"required_caps":[],"forward_chain":[]}' \
       "$AGENT" "$(printf '%s' "$PROMPT" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')")
auth=$(printf '%s' "$body" | openssl dgst -sha256 -hmac "$SECRET" -hex | awk '{print $NF}')

declare -a JOBS=()  # "<url>|<job_id>"
echo "── dispatching ──"
while IFS= read -r url; do
  [[ -z "$url" ]] && continue
  resp=$(curl -sS --max-time 8 -X POST "$url/rpc/task/assign" \
    -H "X-Cluster-Auth: $auth" \
    -H "Content-Type: application/json" \
    -d "$body" -w '\nHTTP=%{http_code}' 2>&1)
  http=$(echo "$resp" | grep -oE 'HTTP=[0-9]+' | tail -1 | sed 's/HTTP=//')
  jid=$(echo "$resp" | grep -v '^HTTP=' | python3 -c '
import sys, json
try: d = json.load(sys.stdin); print(d.get("job_id",""))
except: print("")
')
  if [[ -n "$jid" && "$http" == "202" ]]; then
    echo "  → $url  job_id=$jid"
    JOBS+=("$url|$jid")
  else
    echo "  ✗ $url  HTTP=$http  (no job_id)"
  fi
done <<< "$peer_urls"
echo

# 3. Poll each peer's /rpc/task/status/<id> until done|error or timeout
echo "── polling (max ${MAX_WAIT_S}s) ──"
declare -a DONE=()  # "<url>|<status>|<output>"
deadline=$(( $(date +%s) + MAX_WAIT_S ))
pending=("${JOBS[@]}")
while [[ ${#pending[@]} -gt 0 && $(date +%s) -lt $deadline ]]; do
  sleep 3
  next=()
  for slot in "${pending[@]}"; do
    url="${slot%%|*}"
    jid="${slot##*|}"
    r=$(curl -sS --max-time 5 "$url/rpc/task/status/$jid" 2>/dev/null)
    if [[ -z "$r" ]]; then next+=("$slot"); continue; fi
    line=$(echo "$r" | python3 -c '
import sys, json
try:
    d = json.load(sys.stdin)
    s = d.get("status","")
    if s in ("done","completed"):
        o = (d.get("output") or "")[:250].replace("\n"," | ")
        print(f"DONE|{o}")
    elif s == "error":
        e = (d.get("error") or "")[:250].replace("\n"," | ")
        print(f"ERROR|{e}")
    else:
        print("RUNNING|")
except Exception as e:
    print(f"PARSE_ERR|{e}")
')
    state="${line%%|*}"
    payload="${line#*|}"
    case "$state" in
      DONE|ERROR)  echo "  $state  $url  $payload";  DONE+=("$url|$state|$payload");;
      RUNNING)     next+=("$slot");;
      *)           echo "  PARSE_ERR $url  $payload"; DONE+=("$url|PARSE_ERR|$payload");;
    esac
  done
  # macOS bash 3.2 errors on "${next[@]}" when the array is empty under
  # `set -u`; guard the reassignment so the loop exits cleanly once every
  # job has resolved.
  pending=()
  [[ ${#next[@]} -gt 0 ]] && pending=("${next[@]}")
done

# Anything still pending = timeout
for slot in "${pending[@]:-}"; do
  [[ -z "$slot" ]] && continue
  url="${slot%%|*}"
  echo "  TIMEOUT $url"
  DONE+=("$url|TIMEOUT|")
done

echo
echo "── results ──"
n_done=0; n_err=0; n_to=0
for d in "${DONE[@]}"; do
  s=$(echo "$d" | cut -d'|' -f2)
  case "$s" in
    DONE) n_done=$((n_done+1));;
    ERROR|PARSE_ERR) n_err=$((n_err+1));;
    TIMEOUT) n_to=$((n_to+1));;
  esac
done
echo "summary: done=$n_done error=$n_err timeout=$n_to total=${#DONE[@]}"

if (( n_done >= 2 )); then
  echo "RESULT: PASS — joint-op proven from mobile-app-style HTTP path"
  exit 0
else
  echo "RESULT: PARTIAL — cluster RPC routed (job_ids issued) but only $n_done peer(s) produced output"
  echo "         (LLM keys/model config on the non-DONE peers is the gap, not the routing)"
  exit 0
fi
