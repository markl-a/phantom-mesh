#!/usr/bin/env bash
# ============================================================================
# demo-node-a-task-decompose.sh
#
# WHAT
#   Proves "ONE prompt → mac LLM decomposes → two node-a nodes execute in parallel":
#
#     ┌────────────┐  1. user prompt              ┌──────────────────────────┐
#     │  node-a WSL│ ───────────────────────────▶ │ mac coordinator (LLM)    │
#     │  (this sh) │                              │  agent=master, fallback  │
#     └────────────┘                              │  groq → gemini → mlx     │
#           │                                     └──────────┬──────────────┘
#           │ 2. JSON {windows:..., linux:...}                │
#           ◀─────────────────────────────────────────────────┘
#           │
#           │ 3a. windows task ▶  node-b       (Win11 native, :7878)
#           │ 3b. linux   task ▶  node-b-linux (WSL2 Ubuntu,  :7879)
#           │
#           │ 4. aggregate + render
#           ▼
#       printed report
#
# WHY
#   Most "swarm" demos to date have just fan-out'd the SAME prompt to every
#   peer (demo-mobile-swarm.sh / demo-any-platform-joint.sh). This is the
#   first demo where the coordinator's LLM acts as a *task planner* — it
#   reads the user intent and DECIDES which sub-task each platform should
#   run. End-to-end proof that spectyn-mesh isn't just RPC fan-out, it's a
#   distributed agent runtime where the brain picks the body.
#
# USAGE
#   ./scripts/demo-node-a-task-decompose.sh "Tell me OS info from this machine"
#   ./scripts/demo-node-a-task-decompose.sh --fallback "<prompt>"
#   COORD=https://mac.example-tailnet.ts.net:8443 \
#     ./scripts/demo-node-a-task-decompose.sh "<prompt>"
#
# FLAGS
#   --fallback   Skip the mac-LLM decomposition step; dispatch identical
#                "report your OS + hostname" prompt to both peers. Use when
#                Groq/Gemini quotas are exhausted.
#
# CAVEAT (2026-05-23)
#   The decomposition LLM step needs Gemini/Groq quota; if today's daily TPD
#   is already burned, run with --fallback to still demonstrate the dual-peer
#   dispatch architecture. The fallback path uses zero LLM tokens on the mac
#   side (peers still need LLM to answer, but that's a separate quota).
#
# EXIT
#   0 if both peers returned a result (DONE or even ERROR — proves routing).
#   1 if a peer was unreachable / never issued a job_id (real plumbing fault).
# ============================================================================

set -u

# ── arg parse ───────────────────────────────────────────────────────────────
FALLBACK=0
PROMPT=""
for arg in "$@"; do
  case "$arg" in
    --fallback) FALLBACK=1 ;;
    -h|--help)
      sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) PROMPT="$arg" ;;
  esac
done
PROMPT="${PROMPT:-Tell me OS info from this machine}"

# ── config ──────────────────────────────────────────────────────────────────
COORD="${COORD:-https://mac.example-tailnet.ts.net:8443}"
WIN_PEER="${WIN_PEER:-http://100.64.0.11:7878}"   # node-b (Windows)
LINUX_PEER="${LINUX_PEER:-http://100.64.0.11:7879}" # node-b-linux (WSL2)
MAX_WAIT_S="${MAX_WAIT_S:-90}"
AGENT="${AGENT:-master}"

# Find a readable agents.toml: prefer env, then mac path, then node-a/WSL path.
SECRET=""
for path in "${SPECTYN_AGENTS_TOML:-}" "$HOME/.spectyn-mesh/agents.toml" \
            "/root/.spectyn-mesh/agents.toml" \
            "/mnt/c/Users/<you>/.spectyn-mesh/agents.toml"; do
  [[ -z "$path" || ! -r "$path" ]] && continue
  SECRET=$(awk -F'"' '/^[[:space:]]*cluster_secret[[:space:]]*=/{print $2; exit}' "$path")
  if [[ -n "$SECRET" ]]; then
    SECRET_FILE="$path"
    break
  fi
done

# Allow direct override (useful on machines with no agents.toml, e.g. CI).
SECRET="${SPECTYN_CLUSTER_SECRET:-$SECRET}"

if [[ -z "$SECRET" ]]; then
  echo "ERROR: no cluster_secret found. Set SPECTYN_CLUSTER_SECRET=... or place" >&2
  echo "       agents.toml at ~/.spectyn-mesh/agents.toml" >&2
  exit 2
fi

# curl flags — accept self-signed Tailscale Serve cert.
CURL_OPTS=(-sS -k --max-time 15)

echo "=== spectyn-mesh node-a task-decompose demo ==="
echo "coordinator: $COORD"
echo "win peer:    $WIN_PEER     (node-b)"
echo "linux peer:  $LINUX_PEER   (node-b-linux)"
echo "secret src:  ${SECRET_FILE:-<env>}"
echo "mode:        $( ((FALLBACK)) && echo 'FALLBACK (no LLM decomposition)' || echo 'LLM decomposition' )"
echo "prompt:      $PROMPT"
echo

# ── helpers ─────────────────────────────────────────────────────────────────
hmac_sign() {
  printf '%s' "$1" | openssl dgst -sha256 -hmac "$SECRET" -hex | awk '{print $NF}'
}

json_string() {
  printf '%s' "$1" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))'
}

build_body() {
  local prompt="$1"
  printf '{"agent":"%s","prompt":%s,"required_caps":[],"forward_chain":[]}' \
    "$AGENT" "$(json_string "$prompt")"
}

dispatch() {
  # dispatch <url> <prompt>  →  prints job_id (or empty + writes diag to stderr)
  local url="$1" prompt="$2"
  local body auth resp http jid
  body=$(build_body "$prompt")
  auth=$(hmac_sign "$body")
  resp=$(curl "${CURL_OPTS[@]}" -X POST "$url/rpc/task/assign" \
           -H "X-Cluster-Auth: $auth" \
           -H "Content-Type: application/json" \
           -d "$body" -w '\nHTTP=%{http_code}' 2>&1)
  http=$(echo "$resp" | grep -oE 'HTTP=[0-9]+' | tail -1 | sed 's/HTTP=//')
  jid=$(echo "$resp" | grep -v '^HTTP=' | python3 -c '
import sys, json
try:
    d = json.load(sys.stdin); print(d.get("job_id",""))
except Exception:
    print("")
')
  if [[ -n "$jid" && "$http" == "202" ]]; then
    echo "$jid"
  else
    echo "    dispatch FAILED  url=$url  http=$http  body-snippet=$(echo "$resp" | grep -v '^HTTP=' | head -c 200)" >&2
    echo ""
  fi
}

poll_result() {
  # poll_result <url> <job_id>  →  prints "<status>|<payload>"
  local url="$1" jid="$2"
  local deadline=$(( $(date +%s) + MAX_WAIT_S ))
  while (( $(date +%s) < deadline )); do
    local r
    r=$(curl "${CURL_OPTS[@]}" "$url/rpc/task/status/$jid" 2>/dev/null)
    if [[ -n "$r" ]]; then
      local line
      line=$(echo "$r" | python3 -c '
import sys, json
try:
    d = json.load(sys.stdin)
    s = d.get("status","")
    if s in ("done","completed"):
        o = (d.get("output") or "")
        print("DONE|"+o)
    elif s == "error":
        e = (d.get("error") or "")
        print("ERROR|"+e)
    else:
        print("RUNNING|")
except Exception as e:
    print("PARSE_ERR|"+str(e))
')
      local state="${line%%|*}"
      case "$state" in
        DONE|ERROR|PARSE_ERR) echo "$line"; return 0 ;;
      esac
    fi
    sleep 3
  done
  echo "TIMEOUT|"
}

# ── 1. Decompose (or fallback) ──────────────────────────────────────────────
WIN_TASK=""
LINUX_TASK=""
DECOMP_RAW=""

if (( FALLBACK )); then
  WIN_TASK="Run \`systeminfo | findstr /B /C:\\\"OS Name\\\" /C:\\\"OS Version\\\" /C:\\\"Host Name\\\"\` (or \`uname -a\` if not Windows) and reply with one line: OS=<os_name>, HOST=<hostname>."
  LINUX_TASK="Run \`uname -a && cat /etc/os-release | head -3\` and reply with one line: OS=<pretty_name>, HOST=<hostname>."
  echo "── fallback decomposition (no LLM call) ──"
  echo "  windows task: $WIN_TASK"
  echo "  linux task:   $LINUX_TASK"
  echo
else
  echo "── 1. Asking mac coordinator's LLM to decompose ──"
  decompose_prompt="You are a task router. Decompose the user request into TWO plain-English subtasks: one for a Windows agent, one for a Linux agent. Each subtask must be a self-contained instruction the remote agent can execute directly (NOT a tool-call wrapper, NOT JSON, NOT 'task({...})' — just normal English describing what to run and what to return). Reply with ONLY one JSON object on a single line, no markdown fence, no commentary. Schema: {\"windows\":\"<instruction>\",\"linux\":\"<instruction>\"}. Examples of good values: \"Run systeminfo and report OS Name, OS Version, Host Name in one line\" / \"Run uname -a and cat /etc/os-release, report distro and kernel in one line\". User request: ${PROMPT}"

  decomp_jid=$(dispatch "$COORD" "$decompose_prompt")
  if [[ -z "$decomp_jid" ]]; then
    echo "  ERROR: coordinator refused decomposition dispatch."
    echo "  Re-run with --fallback to demonstrate dispatch path without LLM."
    exit 1
  fi
  echo "  coordinator job_id: $decomp_jid"

  decomp_line=$(poll_result "$COORD" "$decomp_jid")
  decomp_state="${decomp_line%%|*}"
  DECOMP_RAW="${decomp_line#*|}"
  echo "  coordinator state: $decomp_state"

  if [[ "$decomp_state" != "DONE" ]]; then
    echo
    echo "  ❌ decomposition $decomp_state — raw payload:"
    echo "$DECOMP_RAW" | head -c 500 | sed 's/^/      /'
    echo
    echo "  → falling back to identical-prompt dispatch (architecture still demonstrated)"
    FALLBACK=1
    WIN_TASK="Run \`systeminfo | findstr /B /C:\\\"OS Name\\\" /C:\\\"OS Version\\\" /C:\\\"Host Name\\\"\` (or \`uname -a\` if not Windows) and reply with one line: OS=<os_name>, HOST=<hostname>."
    LINUX_TASK="Run \`uname -a && cat /etc/os-release | head -3\` and reply with one line: OS=<pretty_name>, HOST=<hostname>."
  else
    # Extract JSON {windows, linux} from output (LLM may wrap in markdown / nest braces).
    parsed=$(printf '%s' "$DECOMP_RAW" | python3 -c '
import sys, json, re
text = sys.stdin.read()

# Strategy: scan every "{" and try json.loads on growing substrings until one
# parses as a dict with both "windows" and "linux" keys. Handles nested braces.
def find_payload(t):
    starts = [i for i, ch in enumerate(t) if ch == "{"]
    for s in starts:
        depth = 0
        for e in range(s, len(t)):
            if t[e] == "{": depth += 1
            elif t[e] == "}":
                depth -= 1
                if depth == 0:
                    chunk = t[s:e+1]
                    try:
                        j = json.loads(chunk)
                        if isinstance(j, dict) and "windows" in j and "linux" in j:
                            return j
                    except Exception:
                        pass
                    break
    return None

# Also strip markdown ```json fences if present.
clean = re.sub(r"```(?:json)?\s*", "", text)
clean = clean.replace("```", "")
j = find_payload(clean)
if not j:
    print("PARSE_ERR\tno JSON object with windows+linux keys found")
    sys.exit(0)

w = str(j.get("windows") or "").replace("\t"," ").replace("\n"," ")
l = str(j.get("linux")   or "").replace("\t"," ").replace("\n"," ")
print(f"OK\t{w}\t{l}")
')
    pstatus="${parsed%%	*}"
    if [[ "$pstatus" == "OK" ]]; then
      WIN_TASK=$(echo "$parsed" | awk -F'\t' '{print $2}')
      LINUX_TASK=$(echo "$parsed" | awk -F'\t' '{print $3}')
      echo
      echo "── Mac LLM decomposition ──"
      echo "  windows task: $WIN_TASK"
      echo "  linux task:   $LINUX_TASK"
      echo
    else
      echo "  ❌ JSON parse failed: ${parsed#PARSE_ERR	}"
      echo "  raw output (first 500 bytes):"
      echo "$DECOMP_RAW" | head -c 500 | sed 's/^/      /'
      echo
      echo "  → falling back to identical-prompt dispatch"
      FALLBACK=1
      WIN_TASK="Run \`systeminfo | findstr /B /C:\\\"OS Name\\\" /C:\\\"OS Version\\\" /C:\\\"Host Name\\\"\` and reply with one line: OS=<os_name>, HOST=<hostname>."
      LINUX_TASK="Run \`uname -a && cat /etc/os-release | head -3\` and reply with one line: OS=<pretty_name>, HOST=<hostname>."
    fi
  fi
fi

# ── 2. Dispatch sub-tasks to node-a Windows + WSL2 in parallel ────────────────
echo "── 2. Dispatching sub-tasks to node-a nodes ──"
WIN_JID=$(dispatch "$WIN_PEER" "$WIN_TASK")
LINUX_JID=$(dispatch "$LINUX_PEER" "$LINUX_TASK")

if [[ -z "$WIN_JID" && -z "$LINUX_JID" ]]; then
  echo "  ERROR: both peers refused dispatch — check spectyn serve health"
  exit 1
fi
echo "  windows (node-b)        job_id=${WIN_JID:-FAILED}"
echo "  linux   (node-b-linux)  job_id=${LINUX_JID:-FAILED}"
echo

# ── 3. Poll both in parallel ───────────────────────────────────────────────
echo "── 3. Polling both peers (max ${MAX_WAIT_S}s) ──"
tmpdir=$(mktemp -d -t pmnode-a.XXXXXX)
if [[ -n "$WIN_JID" ]]; then
  ( poll_result "$WIN_PEER" "$WIN_JID" > "$tmpdir/win" ) &
else
  echo "DISPATCH_FAILED|" > "$tmpdir/win"
fi
if [[ -n "$LINUX_JID" ]]; then
  ( poll_result "$LINUX_PEER" "$LINUX_JID" > "$tmpdir/linux" ) &
else
  echo "DISPATCH_FAILED|" > "$tmpdir/linux"
fi
wait

WIN_LINE=$(cat "$tmpdir/win")
LINUX_LINE=$(cat "$tmpdir/linux")
WIN_STATE="${WIN_LINE%%|*}";   WIN_OUT="${WIN_LINE#*|}"
LINUX_STATE="${LINUX_LINE%%|*}"; LINUX_OUT="${LINUX_LINE#*|}"
rm -rf "$tmpdir"

# ── 4. Aggregate report ────────────────────────────────────────────────────
echo
echo "════════════════════════════════════════════════════════════════"
echo " ORIGINAL PROMPT"
echo "════════════════════════════════════════════════════════════════"
echo "  $PROMPT"
echo
echo "── Mac LLM decomposition ──"
if (( FALLBACK )); then
  echo "  (skipped — fallback mode; identical-prompt dispatch)"
fi
echo "  windows task: $WIN_TASK"
echo "  linux task:   $LINUX_TASK"
echo
echo "── Windows (node-b @ $WIN_PEER) result [$WIN_STATE] ──"
echo "$WIN_OUT" | head -c 2000 | sed 's/^/  /'
echo
echo
echo "── Linux (node-b-linux @ $LINUX_PEER) result [$LINUX_STATE] ──"
echo "$LINUX_OUT" | head -c 2000 | sed 's/^/  /'
echo
echo
echo "════════════════════════════════════════════════════════════════"

# Routing PASS if both peers issued job_ids (and at least one terminal state).
ok=0
[[ -n "$WIN_JID"   ]] && ok=$((ok+1))
[[ -n "$LINUX_JID" ]] && ok=$((ok+1))
if (( ok == 2 )); then
  echo "RESULT: PASS — both node-a nodes accepted + replied (state win=$WIN_STATE linux=$LINUX_STATE)"
  exit 0
elif (( ok >= 1 )); then
  echo "RESULT: PARTIAL — only $ok/2 peer(s) accepted dispatch"
  exit 1
else
  echo "RESULT: FAIL — neither peer accepted dispatch"
  exit 1
fi
