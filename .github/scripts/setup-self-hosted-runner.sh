#!/bin/bash
# ============================================================================
# Phantom Mesh-Core Self-Hosted Runner 安裝腳本
# 在 Z13 (或其他 Windows 機器) 上安裝 GitHub Actions Self-Hosted Runner
#
# 用法：bash setup-self-hosted-runner.sh <GITHUB_TOKEN> <RUNNER_NAME>
# ============================================================================

set -euo pipefail

GITHUB_TOKEN="${1:?Usage: $0 <GITHUB_TOKEN> <RUNNER_NAME>}"
RUNNER_NAME="${2:-$(hostname)}"
REPO="markl-a/Phantom Mesh"
RUNNER_DIR="$HOME/actions-runner"

echo "=== Setting up GitHub Actions Self-Hosted Runner ==="
echo "Repository: ${REPO}"
echo "Runner Name: ${RUNNER_NAME}"
echo "Install Dir: ${RUNNER_DIR}"

# 下載最新 runner
mkdir -p "${RUNNER_DIR}"
cd "${RUNNER_DIR}"

# 取得最新 runner 版本
LATEST=$(curl -s https://api.github.com/repos/actions/runner/releases/latest | grep -oP '"tag_name": "\K[^"]+')
VERSION="${LATEST#v}"

echo "Downloading runner ${VERSION}..."
curl -o actions-runner.tar.gz -L \
    "https://github.com/actions/runner/releases/download/${LATEST}/actions-runner-win-x64-${VERSION}.tar.gz"

tar xzf actions-runner.tar.gz
rm actions-runner.tar.gz

# 取得 registration token
REG_TOKEN=$(curl -s \
    -X POST \
    -H "Authorization: token ${GITHUB_TOKEN}" \
    -H "Accept: application/vnd.github.v3+json" \
    "https://api.github.com/repos/${REPO}/actions/runners/registration-token" | \
    grep -oP '"token": "\K[^"]+')

echo "Configuring runner..."
./config.sh \
    --url "https://github.com/${REPO}" \
    --token "${REG_TOKEN}" \
    --name "${RUNNER_NAME}" \
    --labels "self-hosted,windows,x64,z13" \
    --work "_work" \
    --runasservice

echo ""
echo "=== Runner installed successfully ==="
echo ""
echo "To start: ./run.sh"
echo "To install as service: sudo ./svc.sh install && sudo ./svc.sh start"
echo ""
echo "Labels: self-hosted, windows, x64, z13"
