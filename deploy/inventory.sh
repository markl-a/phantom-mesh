#!/usr/bin/env bash
# deploy/inventory.sh — Clawtex 集群清單與共用函數
# 所有其他腳本 source 此檔
#
# 維護者: 修改 NODES 陣列來增減節點

set -euo pipefail

# ══════════════════════════════════════════════════════════════════
# 集群節點定義
# 格式: NAME|SSH|OS|ARCH|DEPLOY_PATH|OLLAMA_PORT|ROLE
# ══════════════════════════════════════════════════════════════════

NODES=(
    "z13||windows|x86_64|C:/clawtex|11434|hub"
    "m1|worker@10.0.2.1|macos|aarch64|/opt/clawtex|11434|worker"
    "ayaneo|worker@10.0.1.2|windows|x86_64|C:/clawtex|11434|worker"
    "acer|worker@10.0.1.3|windows|x86_64|C:/clawtex|11434|worker"
)

# Ollama API endpoints
declare -A OLLAMA_URLS=(
    [z13]="http://localhost:11434"
    [m1]="http://10.0.2.1:11434"
    [ayaneo]="http://10.0.1.2:11434"
    [acer]="http://10.0.1.3:11434"
)

# clawtex-core HTTP endpoints
declare -A CORE_URLS=(
    [z13]="http://localhost:7878"
    [m1]="http://10.0.2.1:7878"
    [ayaneo]="http://10.0.1.2:7878"
    [acer]="http://10.0.1.3:7878"
)

# 每台節點的目標模型
declare -A NODE_MODELS=(
    [z13]="qwen3:8b qwen3-coder:30b llama3.1:8b"
    [m1]="llama3.1:8b qwen2.5:14b"
    [ayaneo]="qwen2.5:7b"
    [acer]="llama3.1:8b qwen2.5:13b"
)

# ══════════════════════════════════════════════════════════════════
# 顏色輸出
# ══════════════════════════════════════════════════════════════════

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

log_info()  { echo -e "${BLUE}[INFO]${NC} $*"; }
log_ok()    { echo -e "${GREEN}[OK]${NC} $*"; }
log_warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
log_error() { echo -e "${RED}[ERROR]${NC} $*"; }
log_step()  { echo -e "${CYAN}[STEP]${NC} $*"; }

# ══════════════════════════════════════════════════════════════════
# 節點解析函數
# ══════════════════════════════════════════════════════════════════

get_node_field() {
    local node_str="$1"
    local field_num="$2"
    echo "$node_str" | cut -d'|' -f"$field_num"
}

get_name()        { get_node_field "$1" 1; }
get_ssh()         { get_node_field "$1" 2; }
get_os()          { get_node_field "$1" 3; }
get_arch()        { get_node_field "$1" 4; }
get_deploy_path() { get_node_field "$1" 5; }
get_ollama_port() { get_node_field "$1" 6; }
get_role()        { get_node_field "$1" 7; }

get_binary_name() {
    local os="$1"
    if [[ "$os" == "windows" ]]; then
        echo "clawtex-core.exe"
    else
        echo "clawtex-core"
    fi
}

get_target() {
    local os="$1"
    local arch="$2"
    case "${os}-${arch}" in
        windows-x86_64)  echo "x86_64-pc-windows-msvc" ;;
        macos-aarch64)   echo "aarch64-apple-darwin" ;;
        linux-x86_64)    echo "x86_64-unknown-linux-gnu" ;;
        linux-aarch64)   echo "aarch64-unknown-linux-gnu" ;;
        *)               echo "unknown" ;;
    esac
}

# ══════════════════════════════════════════════════════════════════
# 遠端執行
# ══════════════════════════════════════════════════════════════════

run_remote() {
    local ssh_target="$1"
    shift
    if [[ -z "$ssh_target" ]]; then
        eval "$@"
    else
        ssh -o ConnectTimeout=10 -o StrictHostKeyChecking=no "$ssh_target" "$@"
    fi
}

# ══════════════════════════════════════════════════════════════════
# 健康檢查
# ══════════════════════════════════════════════════════════════════

check_ollama() {
    local url="$1"
    local timeout="${2:-5}"
    curl -sf --max-time "$timeout" "${url}/api/tags" > /dev/null 2>&1
}

check_core() {
    local url="$1"
    local timeout="${2:-5}"
    local response
    response=$(curl -sf --max-time "$timeout" "${url}/health" 2>/dev/null) || return 1
    echo "$response" | grep -q '"status":"ok"'
}

# ══════════════════════════════════════════════════════════════════
# 節點篩選
# ══════════════════════════════════════════════════════════════════

get_workers() {
    for node in "${NODES[@]}"; do
        if [[ "$(get_role "$node")" == "worker" ]]; then
            echo "$node"
        fi
    done
}

get_hub() {
    for node in "${NODES[@]}"; do
        if [[ "$(get_role "$node")" == "hub" ]]; then
            echo "$node"
            return
        fi
    done
}

find_node() {
    local target_name="$1"
    for node in "${NODES[@]}"; do
        if [[ "$(get_name "$node")" == "$target_name" ]]; then
            echo "$node"
            return
        fi
    done
}

# ══════════════════════════════════════════════════════════════════
# Telegram 通知
# ══════════════════════════════════════════════════════════════════

TELEGRAM_BOT_TOKEN="${TELEGRAM_BOT_TOKEN:-}"
TELEGRAM_CHAT_ID="${TELEGRAM_CHAT_ID:-}"

notify_telegram() {
    local message="$1"
    if [[ -n "$TELEGRAM_BOT_TOKEN" && -n "$TELEGRAM_CHAT_ID" ]]; then
        curl -sf -X POST \
            "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/sendMessage" \
            -d "chat_id=${TELEGRAM_CHAT_ID}" \
            -d "text=${message}" \
            -d "parse_mode=HTML" > /dev/null 2>&1 || true
    fi
}
