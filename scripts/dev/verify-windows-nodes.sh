#!/bin/bash
# Verify SSH connectivity + phantom-mesh status on the three Windows nodes.
# Run after `windows-bootstrap.ps1` has been executed on each Windows machine.
#
# Usage:
#   USER_YOYOGOOD=user USER_AYANEO=user USER_LAPTOP=user ./scripts/verify-windows-nodes.sh
#   或編輯下面預設

set -uo pipefail

USER_YOYOGOOD="${USER_YOYOGOOD:-user}"
USER_AYANEO="${USER_AYANEO:-user}"
USER_LAPTOP="${USER_LAPTOP:-user}"

declare -A NODES
NODES[yoyogood]="$USER_YOYOGOOD@100.87.70.65"
NODES[ayaneo]="$USER_AYANEO@100.107.205.98"
NODES[laptop-gur943mk]="$USER_LAPTOP@100.106.176.125"

probe_node() {
    local name="$1"
    local target="$2"
    echo "─── $name ($target) ─────────────────────────"

    # SSH probe
    local ssh_out
    ssh_out=$(timeout 10 ssh -o ConnectTimeout=5 -o BatchMode=yes -o StrictHostKeyChecking=accept-new \
        "$target" 'hostname; whoami; tasklist 2>nul | findstr phantom-mesh; tailscale ip -4 2>nul | findstr 100.' 2>&1 \
        | grep -v "post-quantum\|store now")
    local ssh_rc=$?

    if [ $ssh_rc -eq 0 ]; then
        echo "  ✅ SSH ok"
        echo "$ssh_out" | sed 's/^/     /'
    else
        echo "  ❌ SSH failed:"
        echo "$ssh_out" | sed 's/^/     /'
    fi

    # HTTP probe
    local ts_ip
    ts_ip=$(echo "$target" | cut -d@ -f2)
    local http
    http=$(curl -s --max-time 5 "http://$ts_ip:7878/healthz" 2>&1)
    if [ "$http" = "ok" ]; then
        echo "  ✅ phantom-mesh HTTP ok (/healthz)"
    elif [ -z "$http" ]; then
        echo "  ⚠ phantom-mesh HTTP no response (尚未啟動?)"
    else
        echo "  ⚠ phantom-mesh HTTP: $http"
    fi
    echo ""
}

for n in yoyogood ayaneo laptop-gur943mk; do
    probe_node "$n" "${NODES[$n]}"
done

echo "═══ 結束 ═══"
echo "如果有節點 SSH ✅ 但 phantom-mesh HTTP ❌：表示 SSH 通了但 phantom 沒在跑，"
echo "Mac 端可以接著 SCP 新 binary 上去並啟動。"
