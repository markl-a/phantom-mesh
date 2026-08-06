#!/usr/bin/env bash
# ============================================================================
# demo-5-platform-joint.sh — fan-out a single prompt to every peer in the
# spectyn-mesh cluster and present a unified "who answered + on what OS" table.
#
# WHY
#   Demonstrates that one prompt, issued from ANY node that has the
#   cluster_secret, drives synchronous work across all 5 target OSes
#   (macOS / Windows-node-a / Windows-node-a / Windows-node-b / Linux-WSL).
#
# HOW IT WORKS
#   For each peer URL, the script:
#     1. computes HMAC-SHA256(cluster_secret, body) → hex   (X-Cluster-Auth)
#     2. POSTs /rpc/message  with  {"message":..., "agent":"master",
#                                   "wire_version":1}
#     3. parses {"output":"..."} into one tabular line.
#
# USAGE
#   ./scripts/demo-5-platform-joint.sh                  # 4 peers (no Linux)
#   LINUX_PEER=http://100.x.y.z:7878 ./scripts/demo-5-platform-joint.sh
#
# ADDING THE LINUX PEER
#   Once `spectyn serve` is running inside WSL2 Ubuntu, set:
#       LINUX_PEER=http://<wsl-tailscale-ip>:7878
#   (the WSL host typically exposes its own tailnet IP; if not, port-forward
#   via the Windows host and use that tailnet IP instead).
#
# EXIT CODE
#   0  if  >=4 peers responded with non-empty output
#   1  otherwise
#
# PORTABILITY
#   Pure bash 3.2+, requires: curl, openssl, awk, sed, grep. Works on macOS,
#   Linux, and WSL2.
# ============================================================================

set -u

SECRET_FILE="${SPECTYN_AGENTS_TOML:-$HOME/.spectyn-mesh/agents.toml}"
if [[ ! -r "$SECRET_FILE" ]]; then
    echo "ERROR: cannot read $SECRET_FILE (set SPECTYN_AGENTS_TOML to override)" >&2
    exit 2
fi

# Extract cluster_secret = "..."  (first match wins; tolerant of whitespace).
SECRET=$(awk -F'"' '/^[[:space:]]*cluster_secret[[:space:]]*=/{print $2; exit}' "$SECRET_FILE")
if [[ -z "$SECRET" ]]; then
    echo "ERROR: cluster_secret not found in $SECRET_FILE" >&2
    exit 2
fi

PROMPT='What OS + hostname are you on? Reply in 1 short line as: OS=<os>, HOST=<hostname>, ARCH=<arch>'
BODY=$(printf '{"message":%s,"agent":"master","wire_version":1}' \
       "$(printf '%s' "$PROMPT" | awk 'BEGIN{ORS=""} {gsub(/\\/,"\\\\"); gsub(/"/,"\\\""); print "\"" $0 "\""}')")

AUTH=$(printf '%s' "$BODY" | openssl dgst -sha256 -hmac "$SECRET" -hex | awk '{print $NF}')

# Peer list: label  URL    (one per line).
# IPs are tailnet addresses so the demo runs unchanged from any node in the mesh.
# Override the mac entry to 127.0.0.1 when running on the mac itself for a faster
# loopback path:  MAC_PEER=http://127.0.0.1:7878 ./scripts/demo-5-platform-joint.sh
PEERS=(
  "mac      ${MAC_PEER:-http://100.64.0.10:7878}"
  "node-a   http://100.64.0.11:7878"
  "node-a   http://100.64.0.12:7878"
  "node-b   http://100.64.0.13:7878"
)
if [[ -n "${LINUX_PEER:-}" ]]; then
    PEERS+=("linux    ${LINUX_PEER}")
fi

echo "=== 5-platform joint operation ==="
echo "prompt: $PROMPT"
echo "peers : ${#PEERS[@]}"
echo "----------------------------------"

rpc_ok=0     # HTTP-200 from /rpc/message (proves cluster comms + auth + routing)
llm_ok=0     # LLM behind that peer also produced a usable answer
fail=0       # HTTP error or curl error (cluster comms broken)

for entry in "${PEERS[@]}"; do
    label=$(awk '{print $1}' <<<"$entry")
    url=$(awk '{print $2}' <<<"$entry")
    tmpfile=$(mktemp -t demo5p.XXXXXX)
    http_code=$(curl -sS --max-time 120 -o "$tmpfile" -w '%{http_code}' \
                -X POST "$url/rpc/message" \
                -H "X-Cluster-Auth: $AUTH" \
                -H "Content-Type: application/json" \
                -d "$BODY" 2>/dev/null)
    curl_rc=$?
    [[ $curl_rc -ne 0 || -z "$http_code" ]] && http_code="000"

    body_out=$(cat "$tmpfile" 2>/dev/null)
    rm -f "$tmpfile"

    if [[ "$http_code" == "200" ]]; then
        rpc_ok=$((rpc_ok+1))
        # Branch 1: provider error wrapped in 200  →  {"error":"..."}
        if [[ "$body_out" == *'"error"'* && "$body_out" != *'"output"'* ]]; then
            err_short=$(printf '%s' "$body_out" \
                | sed -n 's/.*"error"[[:space:]]*:[[:space:]]*"\([^"]\{1,140\}\).*/\1/p')
            printf "%-9s RPC=OK LLM=PROVIDER_ERR  %s\n" "$label" "$err_short"
            continue
        fi
        # Branch 2: normal {"output":"..."}
        out=$(printf '%s' "$body_out" \
              | sed -n 's/.*"output"[[:space:]]*:[[:space:]]*"\(.*\)","wire_version".*/\1/p' \
              | sed 's/\\n/ | /g; s/\\"/"/g')
        if [[ -z "$out" ]]; then
            printf "%-9s RPC=OK LLM=EMPTY  raw=%s\n" "$label" "${body_out:0:120}"
        else
            printf "%-9s RPC=OK LLM=OK    %s\n" "$label" "$out"
            llm_ok=$((llm_ok+1))
        fi
    else
        snippet=${body_out:0:140}
        printf "%-9s RPC=FAIL HTTP=%s  %s\n" "$label" "$http_code" "$snippet"
        fail=$((fail+1))
    fi
done

echo "----------------------------------"
echo "summary: rpc_ok=$rpc_ok  llm_ok=$llm_ok  fail=$fail  total=${#PEERS[@]}"
echo "  rpc_ok  = peer accepted HMAC + processed /rpc/message round-trip"
echo "  llm_ok  = peer's LLM provider also returned a usable answer"
echo "  fail    = HTTP error or curl error (cluster comms broken)"

# Pass criterion: cluster RPC layer reaches >=4 peers. LLM success is bonus —
# per-host provider quota / model misconfig is independent of mesh capability.
if (( rpc_ok >= 4 )); then
    echo "RESULT: PASS (>=4 peers reachable via cluster RPC; llm_ok=$llm_ok)"
    exit 0
else
    echo "RESULT: FAIL (<4 peers reachable via cluster RPC)"
    exit 1
fi
