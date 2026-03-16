#!/usr/bin/env bash
# deploy/health-check.sh — 檢查所有節點狀態
#
# 用法:
#   ./health-check.sh              # 完整檢查
#   ./health-check.sh --quick      # 只檢查連通性
#   ./health-check.sh --json       # JSON 輸出
#   ./health-check.sh --watch 30   # 每 30 秒重複

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/inventory.sh"

QUICK=false
JSON_OUTPUT=false
WATCH_INTERVAL=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --quick) QUICK=true; shift ;;
        --json)  JSON_OUTPUT=true; shift ;;
        --watch) WATCH_INTERVAL="$2"; shift 2 ;;
        *) shift ;;
    esac
done

check_node() {
    local node="$1"
    local name ssh_target os
    name=$(get_name "$node")
    ssh_target=$(get_ssh "$node")
    os=$(get_os "$node")

    local ollama_url="${OLLAMA_URLS[$name]:-}"
    local core_url="${CORE_URLS[$name]:-}"

    local ssh_ok="false"
    local ollama_ok="false"
    local core_ok="false"
    local ollama_models=""
    local core_version=""

    # SSH
    if [[ -n "$ssh_target" ]]; then
        ssh -o ConnectTimeout=5 -o StrictHostKeyChecking=no "$ssh_target" "true" 2>/dev/null && ssh_ok="true"
    else
        ssh_ok="true"
    fi

    # Ollama
    if [[ -n "$ollama_url" ]]; then
        local tags_json
        tags_json=$(curl -sf --max-time 5 "${ollama_url}/api/tags" 2>/dev/null) || true
        if [[ -n "$tags_json" ]]; then
            ollama_ok="true"
            ollama_models=$(echo "$tags_json" | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    print(', '.join(m['name'] for m in data.get('models', [])))
except: print('?')
" 2>/dev/null || echo "?")
        fi
    fi

    # clawtex-core
    if [[ -n "$core_url" ]]; then
        local health_json
        health_json=$(curl -sf --max-time 5 "${core_url}/health" 2>/dev/null) || true
        if echo "$health_json" | grep -q '"status":"ok"' 2>/dev/null; then
            core_ok="true"
            core_version=$(echo "$health_json" | python3 -c "
import sys, json
try: print(json.load(sys.stdin).get('version','?'))
except: print('?')
" 2>/dev/null || echo "?")
        fi
    fi

    # 輸出
    if [[ "$JSON_OUTPUT" == "true" ]]; then
        echo "{\"name\":\"$name\",\"ssh\":$ssh_ok,\"ollama\":$ollama_ok,\"core\":$core_ok,\"models\":\"$ollama_models\",\"version\":\"$core_version\"}"
    else
        local status
        if [[ "$ssh_ok" == "true" && "$ollama_ok" == "true" ]]; then
            status="${GREEN}HEALTHY${NC}"
        elif [[ "$ssh_ok" == "true" ]]; then
            status="${YELLOW}PARTIAL${NC}"
        else
            status="${RED}DOWN${NC}"
        fi

        echo -e "  [$status] $name ($os)"
        echo -e "    SSH:    $(if [[ $ssh_ok == true ]]; then echo -e "${GREEN}OK${NC}"; else echo -e "${RED}FAIL${NC}"; fi)"
        echo -e "    Ollama: $(if [[ $ollama_ok == true ]]; then echo -e "${GREEN}OK${NC} ($ollama_models)"; else echo -e "${RED}FAIL${NC}"; fi)"
        if [[ -n "$core_url" ]]; then
            echo -e "    Core:   $(if [[ $core_ok == true ]]; then echo -e "${GREEN}OK${NC} v$core_version"; else echo -e "${YELLOW}OFF${NC}"; fi)"
        fi
    fi
}

run_check() {
    if [[ "$JSON_OUTPUT" != "true" ]]; then
        echo ""
        echo "================================================"
        echo " Clawtex 集群健康檢查 - $(date '+%Y-%m-%d %H:%M:%S')"
        echo "================================================"
        echo ""
    fi

    for node in "${NODES[@]}"; do
        check_node "$node"
    done

    if [[ "$JSON_OUTPUT" != "true" ]]; then
        echo ""
    fi
}

if [[ $WATCH_INTERVAL -gt 0 ]]; then
    while true; do
        clear
        run_check
        echo "下次: ${WATCH_INTERVAL}s (Ctrl+C 退出)"
        sleep "$WATCH_INTERVAL"
    done
else
    run_check
fi
