#!/usr/bin/env bash
# deploy/update-models.sh — 管理所有節點的 Ollama 模型
#
# 用法:
#   ./update-models.sh                    # 同步所有節點
#   ./update-models.sh --node m1          # 只更新 M1
#   ./update-models.sh --pull qwen3:8b    # 全部節點拉取指定模型
#   ./update-models.sh --status           # 顯示模型列表

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/inventory.sh"

TARGET_NODE=""
PULL_MODEL=""
STATUS_ONLY=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --node)    TARGET_NODE="$2"; shift 2 ;;
        --pull)    PULL_MODEL="$2"; shift 2 ;;
        --status)  STATUS_ONLY=true; shift ;;
        --help|-h)
            echo "用法: update-models.sh [OPTIONS]"
            echo "  --node NAME    只更新指定節點"
            echo "  --pull MODEL   全部拉取指定模型"
            echo "  --status       只顯示模型列表"
            exit 0
            ;;
        *) shift ;;
    esac
done

get_remote_models() {
    local ollama_url="$1"
    curl -sf --max-time 10 "${ollama_url}/api/tags" 2>/dev/null | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    for m in data.get('models', []):
        size_gb = m.get('size', 0) / 1073741824
        print(f'{m[\"name\"]}|{size_gb:.1f}GB')
except: pass
" 2>/dev/null
}

pull_model() {
    local ollama_url="$1"
    local model="$2"
    log_info "拉取 $model ..."
    curl -sf --max-time 3600 \
        -X POST "${ollama_url}/api/pull" \
        -d "{\"name\": \"${model}\", \"stream\": false}" \
        2>/dev/null
}

echo ""
echo "================================================"
echo " Clawtex Ollama 模型管理"
echo "================================================"
echo ""

for node in "${NODES[@]}"; do
    name=$(get_name "$node")
    [[ -n "$TARGET_NODE" && "$name" != "$TARGET_NODE" ]] && continue

    ollama_url="${OLLAMA_URLS[$name]:-}"
    [[ -z "$ollama_url" ]] && continue

    echo "----------------------------------------"
    log_step "[$name] ${ollama_url}"
    echo "----------------------------------------"

    if ! check_ollama "$ollama_url"; then
        log_error "[$name] Ollama 不可達"
        continue
    fi

    # 取得現有模型
    current_models=()
    while IFS='|' read -r model_name model_size; do
        current_models+=("$model_name")
        [[ "$STATUS_ONLY" == "true" ]] && echo "  - $model_name ($model_size)"
    done < <(get_remote_models "$ollama_url")

    [[ "$STATUS_ONLY" == "true" ]] && { echo ""; continue; }

    # 拉取單一模型
    if [[ -n "$PULL_MODEL" ]]; then
        if printf '%s\n' "${current_models[@]}" | grep -qx "$PULL_MODEL"; then
            log_ok "[$name] 已有 $PULL_MODEL"
        else
            pull_model "$ollama_url" "$PULL_MODEL" && log_ok "[$name] $PULL_MODEL 完成" || log_error "[$name] $PULL_MODEL 失敗"
        fi
        continue
    fi

    # 同步模型清單
    expected_str="${NODE_MODELS[$name]:-}"
    [[ -z "$expected_str" ]] && { log_warn "[$name] 無模型清單"; continue; }

    read -ra expected <<< "$expected_str"

    for exp in "${expected[@]}"; do
        if printf '%s\n' "${current_models[@]}" | grep -qx "$exp"; then
            log_ok "[$name] $exp (已有)"
        else
            log_info "[$name] $exp (缺少)"
            pull_model "$ollama_url" "$exp" && log_ok "[$name] $exp 完成" || log_error "[$name] $exp 失敗"
        fi
    done

    for cur in "${current_models[@]}"; do
        if ! printf '%s\n' "${expected[@]}" | grep -qx "$cur"; then
            log_warn "[$name] $cur 不在清單中"
        fi
    done

    echo ""
done

log_ok "模型管理完成"
