# scripts/

兩層分流：

## `scripts/` — 公開、給用戶的工具

| 檔案 | 用途 |
|---|---|
| `package-android.sh` | 打包 Android APK |
| `package-ios.sh` | 打包 iOS IPA |
| `build-update-manifest.py` | 產生 release manifest |
| `generate-latest-json.sh` | 產生 release metadata |
| `setup-tailscale-mesh.sh` | 設定 Tailscale mesh |
| `setup-pi.sh` | Raspberry Pi 安裝 |
| `setup-env.sh` | 環境變數初始化 |
| `windows-bootstrap.ps1` | Windows 節點 bootstrap |
| `termux-setup.sh` | Android Termux 環境 |
| `phantom-mesh.service` | systemd unit file |
| `smoke-test.sh` | 基本煙霧測試 |
| `integration-test.sh` | 整合測試 |
| `verify-binary.sh` | 跨平台 binary 健康檢查(bash;Linux/Mac/WSL/Git Bash 用)— 詳見 `VERIFY-BINARY.md` |
| `verify-binary.ps1` | 同上(PowerShell;Windows 用)— 詳見 `VERIFY-BINARY.md` |
| `update-daemon.sh` | daemon 自動更新 |
| `self-evolve.sh` | 自我演化迴圈觸發 |
| `clean-history.sh` | log/history 清理 |
| `pre-open-source-checklist.sh` | release 前檢查 |
| `ios-shortcuts-fetch.md` | iOS Shortcuts 整合說明 |

## `scripts/dev/` — 內部維運（開發者用）

| 檔案 | 用途 |
|---|---|
| `cluster-install.sh` | 多節點叢集安裝（含 IP 變數，需自行替換）|
| `verify-windows-nodes.sh` | Windows 節點連通性檢查 |
| `day2-windows-verify.sh` | Day 2 sprint 驗證腳本 |
| `deploy-gcp.sh` | GCP 部署（legacy）|
| `push-to-github.sh` | git push 自動化 |
| `run-embed-sync.ps1` | Windows embedding sync |

> 這層腳本**含有特定環境假設**（IP、路徑、節點名），不適合直接給一般用戶。
