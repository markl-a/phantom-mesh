#!/bin/bash
# Verify SSH connectivity + spectyn-mesh status on the three Windows nodes.
# Run after `windows-bootstrap.ps1` has been executed on each Windows machine.
#
# Usage:
#   USER_NODEA=user USER_NODEB=user USER_NODEC=user ./scripts/verify-windows-nodes.sh
#   或編輯下面預設

set -uo pipefail

USER_NODEA="${USER_NODEA:-user}"
USER_NODEB="${USER_NODEB:-user}"
USER_NODEC="${USER_NODEC:-user}"

declare -A NODES
NODES[node-a]="$USER_NODEA@192.0.2.11"
NODES[node-b]="$USER_NODEB@192.0.2.12"
NODES[node-c]="$USER_NODEC@192.0.2.13"

probe_node() {
    local name="$1"
    local target="$2"
    echo "─── $name ($target) ─────────────────────────"

    # SSH probe
    local ssh_out
    ssh_out=$(timeout 10 ssh -o ConnectTimeout=5 -o BatchMode=yes -o StrictHostKeyChecking=accept-new \
        "$target" 'hostname; whoami; tasklist 2>nul | findstr spectyn-mesh; tailscale ip -4 2>nul | findstr 100.' 2>&1 \
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
        echo "  ✅ spectyn-mesh HTTP ok (/healthz)"
    elif [ -z "$http" ]; then
        echo "  ⚠ spectyn-mesh HTTP no response (尚未啟動?)"
    else
        echo "  ⚠ spectyn-mesh HTTP: $http"
    fi
    echo ""
}

for n in node-a node-b node-c; do
    probe_node "$n" "${NODES[$n]}"
done

echo "═══ 結束 ═══"
echo "如果有節點 SSH ✅ 但 spectyn-mesh HTTP ❌：表示 SSH 通了但 spectyn 沒在跑，"
echo "Mac 端可以接著 SCP 新 binary 上去並啟動。"
