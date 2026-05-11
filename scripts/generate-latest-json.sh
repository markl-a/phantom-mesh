#!/bin/bash
# 用法: ./scripts/generate-latest-json.sh v1.2.3
# 在 release-desktop.yml 的 CI 中自動執行（tauri-action 會處理）
# 手動用於測試或特殊情況

set -euo pipefail

VERSION="${1:?請提供版本號，例如: v1.2.3}"
REPO="${GITHUB_REPOSITORY:-markl-a/phantom-mesh}"
BASE_URL="https://github.com/$REPO/releases/download/$VERSION"
NOTES="${RELEASE_NOTES:-版本 $VERSION 更新}"
PUB_DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

# 簽名檔案（由 CI 中的 tauri signer 產生）
sign_file() {
  local file="$1"
  if [ -f "$file" ] && [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
    cargo tauri signer sign -k "$TAURI_SIGNING_PRIVATE_KEY" "$file" 2>/dev/null \
      | grep -o 'dW5.*' || echo ""
  else
    echo ""
  fi
}

cat > latest.json << EOF
{
  "version": "${VERSION#v}",
  "notes": "$NOTES",
  "pub_date": "$PUB_DATE",
  "platforms": {
    "linux-x86_64": {
      "signature": "$(sign_file phantom-mesh-x86_64.AppImage.tar.gz.sig 2>/dev/null || echo '')",
      "url": "$BASE_URL/phantom-mesh_${VERSION#v}_amd64.AppImage.tar.gz"
    },
    "darwin-aarch64": {
      "signature": "$(sign_file phantom-mesh-aarch64.app.tar.gz.sig 2>/dev/null || echo '')",
      "url": "$BASE_URL/phantom-mesh_${VERSION#v}_aarch64.dmg"
    },
    "darwin-x86_64": {
      "signature": "$(sign_file phantom-mesh-x86_64.app.tar.gz.sig 2>/dev/null || echo '')",
      "url": "$BASE_URL/phantom-mesh_${VERSION#v}_x64.dmg"
    },
    "windows-x86_64": {
      "signature": "$(sign_file phantom-mesh-x86_64-setup.exe.sig 2>/dev/null || echo '')",
      "url": "$BASE_URL/phantom-mesh_${VERSION#v}_x64-setup.exe"
    },
    "android": {
      "signature": "",
      "url": "$BASE_URL/phantom-mesh-$VERSION.apk"
    }
  }
}
EOF

echo "已產生 latest.json (版本 $VERSION)"
cat latest.json
