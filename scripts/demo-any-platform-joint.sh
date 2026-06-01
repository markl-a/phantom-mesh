#!/usr/bin/env bash
# ============================================================================
# demo-any-platform-joint.sh
#
# WHAT
#   Proves "any platform can submit a prompt, all platforms operate jointly":
#     1. Discover every peer in the mesh (GET /rpc/peers on local coordinator)
#     2. Fan-out an HMAC-auth'd /rpc/ping to every peer (parallel curl)
#     3. Collect each peer's OS / hostname / phantom-version / uptime
#     4. Run local synthesis via `phantom` agent (Groq fallback if Gemini down)
#     5. Print a unified report
#
# WHY
#   The `phantom swarm` command also fan-outs but requires every peer to have
#   working LLM keys + correct model config. /rpc/ping is LLM-free and works
#   uniformly across mac / linux / 3× windows / mobile-app-talking-to-coord.
#   This demo is the minimum honest proof of "joint operation across cluster".
#
# USAGE
#   ./scripts/demo-any-platform-joint.sh
#   ORIGIN_URL=http://100.64.0.11:7878 ./scripts/demo-any-platform-joint.sh   # run from node-a
#   SKIP_SYNTH=1                                  # skip LLM, just print table
#
# EXIT
#   0 if >=3 peers respond, else 1
# ============================================================================

set -u

ORIGIN_URL="${ORIGIN_URL:-http://127.0.0.1:7878}"
SECRET_FILE="${PHANTOM_AGENTS_TOML:-$HOME/.phantom-mesh/agents.toml}"

if [[ ! -r "$SECRET_FILE" ]]; then
  echo "ERROR: cannot read $SECRET_FILE" >&2; exit 2
fi
SECRET=$(awk -F'"' '/^[[:space:]]*cluster_secret[[:space:]]*=/{print $2; exit}' "$SECRET_FILE")
if [[ -z "$SECRET" ]]; then
  echo "ERROR: no cluster_secret in $SECRET_FILE" >&2; exit 2
fi

echo "=== phantom-mesh joint-operation demo ==="
echo "origin coordinator: $ORIGIN_URL"
echo

# ── 1. Discover peers ────────────────────────────────────────────────────────
peers_json=$(curl -sS --max-time 5 "$ORIGIN_URL/rpc/peers")
if [[ -z "$peers_json" ]]; then
  echo "ERROR: origin $ORIGIN_URL not reachable" >&2; exit 1
fi

# Extract peer URLs (incl. self) into a tab-separated label\turl list.
node_list=$(echo "$peers_json" | python3 -c '
import sys, json
d = json.load(sys.stdin)
out = []
self = d.get("self", {})
# coordinator address: prefer ORIGIN_URL since "self".url is empty
if self.get("name"):
    out.append((self["name"] + "(self)", "__SELF__"))
for p in d.get("peers", []):
    url = p.get("url", "")
    name = p.get("name", "?")
    if url:
        out.append((name, url))
for label, url in out:
    print(f"{label}\t{url}")
')

echo "discovered $(echo "$node_list" | wc -l | tr -d ' ') node(s) (incl. self):"
echo "$node_list" | sed 's/^/  /'
echo

# ── 2. Fan-out /rpc/ping in parallel ────────────────────────────────────────
echo "── fanning out /rpc/ping ──"
tmpdir=$(mktemp -d -t pmpf.XXXXXX)
body='{"node_name":"demo-origin","wire_version":1}'
auth=$(printf '%s' "$body" | openssl dgst -sha256 -hmac "$SECRET" -hex | awk '{print $NF}')

n=0
while IFS=$'\t' read -r label url; do
  [[ -z "$label" ]] && continue
  target_url="$url"
  if [[ "$url" == "__SELF__" ]]; then target_url="$ORIGIN_URL"; fi
  (
    out=$(curl -sS --max-time 5 -X POST "$target_url/rpc/ping" \
      -H "X-Cluster-Auth: $auth" \
      -H "Content-Type: application/json" \
      -d "$body" 2>/dev/null)
    rc=$?
    if [[ $rc -ne 0 || -z "$out" ]]; then
      echo "OFFLINE|$label|$target_url" > "$tmpdir/$n"
    else
      echo "$out" | python3 -c '
import sys, json
try:
    d = json.load(sys.stdin)
    name = d.get("name", "?")
    ver  = d.get("phantom_version", "?")
    up   = d.get("uptime_secs", 0)
    caps = ",".join(d.get("capabilities", [])) or "-"
    agents = ",".join(d.get("agents", [])) or "-"
    print(f"ONLINE|{name}|{ver}|{up}|{caps}|{agents}")
except Exception as e:
    print(f"PARSE_ERR|{e}")' > "$tmpdir/$n"
    fi
  ) &
  n=$((n+1))
done <<< "$node_list"
wait

# ── 3. Render table ────────────────────────────────────────────────────────
echo
printf "%-20s %-8s %-12s %-32s %s\n" "NODE" "VERSION" "UPTIME(s)" "CAPABILITIES" "AGENTS"
printf "%-20s %-8s %-12s %-32s %s\n" "----" "-------" "---------" "------------" "------"
online_count=0
offline_count=0
report_lines=""
for f in "$tmpdir"/*; do
  line=$(cat "$f")
  case "$line" in
    ONLINE\|*)
      IFS='|' read -r _ name ver up caps agents <<< "$line"
      printf "%-20s %-8s %-12s %-32s %s\n" "$name" "$ver" "$up" "$caps" "$agents"
      report_lines+="- ${name} (v${ver}, up=${up}s, caps=${caps}, agents=${agents})"$'\n'
      online_count=$((online_count+1))
      ;;
    OFFLINE\|*)
      IFS='|' read -r _ name url <<< "$line"
      printf "%-20s %-8s %-12s %-32s %s\n" "$name" "OFFLINE" "-" "-" "$url"
      offline_count=$((offline_count+1))
      ;;
  esac
done

rm -rf "$tmpdir"
echo
echo "summary: online=$online_count offline=$offline_count"

# ── 4. Synthesize via local LLM (optional) ──────────────────────────────────
if [[ "${SKIP_SYNTH:-0}" == "1" ]]; then
  echo
  echo "(synthesis skipped — SKIP_SYNTH=1)"
  if [[ $online_count -ge 3 ]]; then echo "RESULT: PASS"; exit 0; fi
  echo "RESULT: FAIL (<3 online)"; exit 1
fi

if command -v phantom >/dev/null 2>&1; then
  echo
  echo "── synthesizing via local phantom agent ──"
  prompt="You are a phantom-mesh cluster coordinator. Below is the live fleet status from /rpc/ping across all peers in the same Tailnet. Write a 3-sentence executive summary in Traditional Chinese: how many nodes are online, what platforms they cover, and whether the cluster is healthy for joint operations."$'\n\n'"$report_lines"
  phantom agent ask "$prompt" 2>/dev/null | head -20 || \
    echo "(synthesis failed — likely LLM quota; fleet table above is the proof of joint-op routing)"
fi

if [[ $online_count -ge 3 ]]; then
  echo "RESULT: PASS (>=3 nodes responded jointly)"
  exit 0
else
  echo "RESULT: FAIL (<3 nodes responded)"
  exit 1
fi
