#!/usr/bin/env bash
# deploy/rollback.sh — 回滾指定節點到上一版本
#
# 用法:
#   ./rollback.sh <node_name>
#   ./rollback.sh --all

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/inventory.sh"

[[ $# -lt 1 ]] && { echo "用法: rollback.sh <node_name|--all>"; exit 1; }

rollback_node() {
    local node="$1"
    local name ssh_target os deploy_path binary_name
    name=$(get_name "$node")
    ssh_target=$(get_ssh "$node")
    os=$(get_os "$node")
    deploy_path=$(get_deploy_path "$node")
    binary_name=$(get_binary_name "$os")

    log_step "[$name] 回滾..."

    # 檢查 .prev
    local has_prev
    if [[ -n "$ssh_target" ]]; then
        has_prev=$(run_remote "$ssh_target" "test -f '${deploy_path}/bin/${binary_name}.prev' && echo yes || echo no")
    else
        [[ -f "${deploy_path}/bin/${binary_name}.prev" ]] && has_prev="yes" || has_prev="no"
    fi

    if [[ "$has_prev" != "yes" ]]; then
        log_error "[$name] 沒有 .prev 可回滾"
        return 1
    fi

    # 停止
    if [[ "$os" == "windows" ]]; then
        run_remote "$ssh_target" "taskkill /f /im clawtex-core.exe 2>/dev/null || true" 2>/dev/null || true
    else
        run_remote "$ssh_target" "pkill -f clawtex-core 2>/dev/null || true" 2>/dev/null || true
    fi
    sleep 2

    # 交換
    if [[ -n "$ssh_target" ]]; then
        run_remote "$ssh_target" "cd '${deploy_path}/bin' && mv '${binary_name}' '${binary_name}.failed' && mv '${binary_name}.prev' '${binary_name}'"
    else
        cd "${deploy_path}/bin"
        mv "${binary_name}" "${binary_name}.failed"
        mv "${binary_name}.prev" "${binary_name}"
    fi

    # 啟動
    if [[ "$os" == "windows" ]]; then
        run_remote "$ssh_target" "cd '${deploy_path}' && start /b bin/${binary_name} --host 0.0.0.0 --config config/agents.toml daemon" &
    else
        run_remote "$ssh_target" "cd '${deploy_path}' && nohup bin/clawtex-core --host 0.0.0.0 --config config/agents.toml daemon > /tmp/clawtex-core.log 2>&1 &"
    fi
    sleep 5

    local core_url="${CORE_URLS[$name]:-}"
    if [[ -n "$core_url" ]] && check_core "$core_url"; then
        log_ok "[$name] 回滾成功"
    else
        log_warn "[$name] 回滾後健康檢查未通過"
    fi
}

if [[ "$1" == "--all" ]]; then
    for node in "${NODES[@]}"; do
        rollback_node "$node"
    done
else
    node=$(find_node "$1")
    if [[ -z "$node" ]]; then
        log_error "找不到節點: $1"
        exit 1
    fi
    rollback_node "$node"
fi
