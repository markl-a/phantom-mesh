#!/usr/bin/env bash
# deploy/deploy.sh — 一鍵部署 phantom-mesh 到所有節點
#
# 用法:
#   ./deploy.sh              # 部署到所有節點
#   ./deploy.sh --workers    # 只部署 Workers
#   ./deploy.sh --node m1    # 只部署指定節點
#   ./deploy.sh --build-only # 只編譯，不部署
#   ./deploy.sh --skip-build # 跳過編譯，只部署

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/inventory.sh"

# ── 參數 ──────────────────────────────────────────────────────────

BUILD_ONLY=false
SKIP_BUILD=false
WORKERS_ONLY=false
TARGET_NODE=""
PHANTOM_MESH_SRC="${PHANTOM_MESH_SRC:-$(cd "${SCRIPT_DIR}/.." && pwd)}"
BUILD_DIR="${PHANTOM_MESH_SRC}/target"
RELEASE_DIR="${SCRIPT_DIR}/releases"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --build-only)  BUILD_ONLY=true; shift ;;
        --skip-build)  SKIP_BUILD=true; shift ;;
        --workers)     WORKERS_ONLY=true; shift ;;
        --node)        TARGET_NODE="$2"; shift 2 ;;
        --help|-h)
            echo "用法: deploy.sh [OPTIONS]"
            echo "  --build-only   只編譯，不部署"
            echo "  --skip-build   跳過編譯 (用上次的 binary)"
            echo "  --workers      只部署 Workers"
            echo "  --node NAME    只部署指定節點"
            exit 0
            ;;
        *) log_error "未知參數: $1"; exit 1 ;;
    esac
done

# ── 版本 ──────────────────────────────────────────────────────────

cd "$PHANTOM_MESH_SRC"
GIT_HASH=$(git rev-parse --short HEAD 2>/dev/null || echo "nogit")
BUILD_TIME=$(date +%Y%m%d-%H%M%S)
VERSION="${GIT_HASH}-${BUILD_TIME}"

log_info "Phantom Mesh 部署開始"
log_info "版本: ${VERSION}"
log_info "源碼: ${PHANTOM_MESH_SRC}"
echo ""

# ══════════════════════════════════════════════════════════════════
# Phase 1: 編譯
# ══════════════════════════════════════════════════════════════════

mkdir -p "$RELEASE_DIR/${VERSION}"

if [[ "$SKIP_BUILD" == "false" ]]; then
    log_step "Phase 1: 跨平台編譯"

    # Windows x86_64 (本地)
    log_info "編譯 Windows x86_64 (本地)..."
    cargo build --release 2>&1 | tail -3
    cp "${BUILD_DIR}/release/phantom-mesh.exe" "${RELEASE_DIR}/${VERSION}/phantom-mesh-windows-x86_64.exe"
    log_ok "Windows x86_64 完成"

    # macOS aarch64 (遠端 M1)
    M1_SSH="worker@10.0.2.1"
    if ssh -o ConnectTimeout=5 "$M1_SSH" "true" 2>/dev/null; then
        log_info "編譯 macOS aarch64 (遠端 M1)..."
        rsync -az --delete \
            --exclude target --exclude .git \
            "${PHANTOM_MESH_SRC}/" "${M1_SSH}:/tmp/phantom-mesh-build/"
        ssh "$M1_SSH" "cd /tmp/phantom-mesh-build && cargo build --release 2>&1 | tail -3"
        scp "${M1_SSH}:/tmp/phantom-mesh-build/target/release/phantom-mesh" \
            "${RELEASE_DIR}/${VERSION}/phantom-mesh-macos-aarch64"
        log_ok "macOS aarch64 完成"
    else
        log_warn "M1 不可達，跳過 macOS 編譯"
    fi

    # Linux x86_64 (cross)
    if command -v cross &>/dev/null; then
        log_info "編譯 Linux x86_64 (cross)..."
        cross build --release --target x86_64-unknown-linux-gnu 2>&1 | tail -3
        cp "${BUILD_DIR}/x86_64-unknown-linux-gnu/release/phantom-mesh" \
            "${RELEASE_DIR}/${VERSION}/phantom-mesh-linux-x86_64"
        log_ok "Linux x86_64 完成"
    else
        log_warn "cross 未安裝，跳過 Linux 編譯"
    fi

    echo ""
    log_ok "Phase 1 完成"
    ls -lh "${RELEASE_DIR}/${VERSION}/"
    echo ""
fi

if [[ "$BUILD_ONLY" == "true" ]]; then
    log_info "--build-only，跳過部署"
    exit 0
fi

# ══════════════════════════════════════════════════════════════════
# Phase 2: 滾動部署
# ══════════════════════════════════════════════════════════════════

log_step "Phase 2: 滾動部署"

DEPLOY_SUCCESS=0
DEPLOY_FAIL=0
FAILED_NODES=()

deploy_node() {
    local node="$1"
    local name ssh_target os arch deploy_path binary_name binary_src

    name=$(get_name "$node")
    ssh_target=$(get_ssh "$node")
    os=$(get_os "$node")
    arch=$(get_arch "$node")
    deploy_path=$(get_deploy_path "$node")
    binary_name=$(get_binary_name "$os")

    case "${os}-${arch}" in
        windows-x86_64)  binary_src="${RELEASE_DIR}/${VERSION}/phantom-mesh-windows-x86_64.exe" ;;
        macos-aarch64)   binary_src="${RELEASE_DIR}/${VERSION}/phantom-mesh-macos-aarch64" ;;
        linux-x86_64)    binary_src="${RELEASE_DIR}/${VERSION}/phantom-mesh-linux-x86_64" ;;
        *)               log_error "[$name] 不支援: ${os}-${arch}"; return 1 ;;
    esac

    if [[ ! -f "$binary_src" ]]; then
        log_error "[$name] Binary 不存在: $binary_src"
        return 1
    fi

    log_info "[$name] 開始部署 (${os}/${arch})..."

    # 建立目錄
    if [[ -n "$ssh_target" ]]; then
        run_remote "$ssh_target" "mkdir -p '${deploy_path}/bin' '${deploy_path}/releases/${VERSION}' '${deploy_path}/config' '${deploy_path}/data'"
    else
        mkdir -p "${deploy_path}/bin" "${deploy_path}/releases/${VERSION}" "${deploy_path}/config" "${deploy_path}/data"
    fi

    # 上傳
    log_info "[$name] 上傳 binary..."
    if [[ -n "$ssh_target" ]]; then
        scp -q "$binary_src" "${ssh_target}:${deploy_path}/releases/${VERSION}/${binary_name}"
        if [[ "$os" != "windows" ]]; then
            run_remote "$ssh_target" "chmod +x '${deploy_path}/releases/${VERSION}/${binary_name}'"
        fi
    else
        cp "$binary_src" "${deploy_path}/releases/${VERSION}/${binary_name}"
    fi

    # 停止
    log_info "[$name] 停止舊服務..."
    if [[ "$os" == "windows" ]]; then
        run_remote "$ssh_target" "taskkill /f /im phantom-mesh.exe 2>/dev/null || true" 2>/dev/null || true
    else
        run_remote "$ssh_target" "pkill -f phantom-mesh 2>/dev/null || true" 2>/dev/null || true
    fi
    sleep 2

    # 替換 (保留 .prev)
    if [[ -n "$ssh_target" ]]; then
        run_remote "$ssh_target" "cd '${deploy_path}/bin' && { test -f '${binary_name}' && cp '${binary_name}' '${binary_name}.prev' || true; } && cp '${deploy_path}/releases/${VERSION}/${binary_name}' '${binary_name}'"
    else
        cd "${deploy_path}/bin"
        [[ -f "${binary_name}" ]] && cp "${binary_name}" "${binary_name}.prev" || true
        cp "${deploy_path}/releases/${VERSION}/${binary_name}" "${binary_name}"
    fi

    # 啟動
    log_info "[$name] 啟動新服務..."
    if [[ "$os" == "windows" ]]; then
        run_remote "$ssh_target" "cd '${deploy_path}' && start /b bin/${binary_name} --host 0.0.0.0 --config config/agents.toml daemon" &
    elif [[ "$os" == "macos" ]]; then
        run_remote "$ssh_target" "cd '${deploy_path}' && nohup bin/phantom-mesh --host 0.0.0.0 --config config/agents.toml daemon > /tmp/phantom-mesh.log 2>&1 &"
    else
        run_remote "$ssh_target" "if systemctl is-active phantom-mesh &>/dev/null; then sudo systemctl restart phantom-mesh; else cd '${deploy_path}' && nohup bin/phantom-mesh --host 0.0.0.0 --config config/agents.toml daemon > /tmp/phantom-mesh.log 2>&1 &; fi"
    fi

    # 等待
    sleep 10

    # 健康檢查
    local core_url="${CORE_URLS[$name]:-}"
    if [[ -n "$core_url" ]]; then
        local retries=3
        for i in $(seq 1 $retries); do
            if check_core "$core_url"; then
                log_ok "[$name] 部署成功"
                return 0
            fi
            log_warn "[$name] 健康檢查 #${i} 失敗，重試..."
            sleep 5
        done
        log_error "[$name] 健康檢查失敗"
        return 1
    else
        log_warn "[$name] 跳過健康檢查"
        return 0
    fi
}

# 決定部署順序
deploy_list=()
if [[ -n "$TARGET_NODE" ]]; then
    node=$(find_node "$TARGET_NODE")
    if [[ -z "$node" ]]; then
        log_error "找不到節點: $TARGET_NODE"
        exit 1
    fi
    deploy_list+=("$node")
elif [[ "$WORKERS_ONLY" == "true" ]]; then
    while IFS= read -r node; do
        deploy_list+=("$node")
    done < <(get_workers)
else
    while IFS= read -r node; do
        deploy_list+=("$node")
    done < <(get_workers)
    hub=$(get_hub)
    [[ -n "$hub" ]] && deploy_list+=("$hub")
fi

for node in "${deploy_list[@]}"; do
    name=$(get_name "$node")
    echo ""
    echo "========================================"
    log_step "部署節點: $name"
    echo "========================================"
    if deploy_node "$node"; then
        ((DEPLOY_SUCCESS++))
    else
        ((DEPLOY_FAIL++))
        FAILED_NODES+=("$name")
        log_error "$name 部署失敗，中止"
        break
    fi
done

# ══════════════════════════════════════════════════════════════════
# Phase 3: 報告
# ══════════════════════════════════════════════════════════════════

echo ""
echo "========================================"
log_step "部署報告"
echo "========================================"
echo "版本:    ${VERSION}"
echo "成功:    ${DEPLOY_SUCCESS} 台"
echo "失敗:    ${DEPLOY_FAIL} 台"
[[ ${#FAILED_NODES[@]} -gt 0 ]] && echo "失敗節點: ${FAILED_NODES[*]}"

if [[ $DEPLOY_FAIL -eq 0 ]]; then
    notify_telegram "Phantom Mesh 部署完成 v${VERSION} (${DEPLOY_SUCCESS} 台)"
    log_ok "部署完成"
else
    notify_telegram "Phantom Mesh 部署失敗! v${VERSION} (${DEPLOY_FAIL} 台: ${FAILED_NODES[*]})"
    log_error "部署有失敗"
    exit 1
fi
