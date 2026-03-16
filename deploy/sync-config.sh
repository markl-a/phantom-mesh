#!/usr/bin/env bash
# deploy/sync-config.sh — 同步配置到 Workers
#
# 用法:
#   ./sync-config.sh                   # 同步所有
#   ./sync-config.sh --node ayaneo     # 指定節點

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/inventory.sh"

CONFIG_DIR="${SCRIPT_DIR}/config"
TARGET_NODE=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --node) TARGET_NODE="$2"; shift 2 ;;
        *) shift ;;
    esac
done

log_step "同步配置到 Workers"

for node in "${NODES[@]}"; do
    name=$(get_name "$node")
    role=$(get_role "$node")
    ssh_target=$(get_ssh "$node")
    deploy_path=$(get_deploy_path "$node")
    os=$(get_os "$node")

    [[ "$role" == "hub" ]] && continue
    [[ -n "$TARGET_NODE" && "$name" != "$TARGET_NODE" ]] && continue
    [[ -z "$ssh_target" ]] && continue

    log_info "[$name] 同步配置..."

    # 尋找配置 (節點專用 > 通用)
    worker_config="${CONFIG_DIR}/workers/${name}/agents.toml"
    if [[ ! -f "$worker_config" ]]; then
        worker_config="${CONFIG_DIR}/workers/default/agents.toml"
    fi

    if [[ ! -f "$worker_config" ]]; then
        log_error "[$name] 找不到配置"
        continue
    fi

    scp -q "$worker_config" "${ssh_target}:${deploy_path}/config/agents.toml"
    log_ok "[$name] 配置已同步"

    # 重啟服務
    log_info "[$name] 重啟..."
    if [[ "$os" == "windows" ]]; then
        run_remote "$ssh_target" "taskkill /f /im clawtex-core.exe 2>/dev/null; cd '${deploy_path}' && start /b bin/clawtex-core.exe --host 0.0.0.0 --config config/agents.toml daemon" 2>/dev/null &
    else
        run_remote "$ssh_target" "pkill -f clawtex-core 2>/dev/null; sleep 2; cd '${deploy_path}' && nohup bin/clawtex-core --host 0.0.0.0 --config config/agents.toml daemon > /tmp/clawtex-core.log 2>&1 &"
    fi

    log_ok "[$name] 完成"
done

log_ok "配置同步完成"
