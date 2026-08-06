# scripts/

可執行檔的家。**規則:此目錄樹只放可執行腳本與其資料夾,不放 .md 審查/產出快照**
(那些住 `docs/ai-reviews/`)。

頂層 `scripts/*.sh|*.ps1|*.py` 是面向用戶/發佈的工具;每個子目錄是一個有特定用途的
腳本家族。下面是**逐子目錄地圖**(隨樹更動請同步)。

## 頂層 — 面向用戶 / 發佈

| 檔案 | 用途 |
|---|---|
| `install.sh` / `install.ps1` | 安裝器(POSIX / Windows) |
| `install-mac.sh` / `install-spectyn-windows.ps1` | 平台專屬安裝 |
| `package-android.sh` / `package-android-apk.sh` | 打包 Android |
| `package-ios.sh` / `run-ios-sandbox.sh` | 打包 / 跑 iOS |
| `package-linux.sh` / `package-macos.sh` / `package-windows.ps1` | 桌面打包 |
| `build-linux.sh` / `build-mac.sh` / `build-windows.ps1` / `build-msi-windows.ps1` | 平台 build |
| `build-update-manifest.py` / `generate-latest-json.sh` | 產生 release manifest / metadata |
| `codesign-windows.ps1` / `sign-windows-ci.ps1` | Windows 簽章 |
| `setup-tailscale-mesh.sh` | 設定 Tailscale mesh |
| `setup-pi.sh` / `setup-oci.sh` / `setup-cloud-linux.sh` | Pi / OCI / 雲端 Linux 安裝 |
| `setup-env.sh` | 環境變數初始化 |
| `termux-setup.sh` | Android Termux 環境 |
| `windows-bootstrap.ps1` / `onboard-windows-dev-machine.ps1` | Windows 節點 bootstrap |
| `spectyn-mesh.service` | systemd unit file |
| `install-launchagent.sh` / `uninstall-launchagent.sh` | macOS LaunchAgent |
| `smoke-test.sh` / `integration-test.sh` | 基本煙霧 / 整合測試 |
| `self-evolve.sh` | 自我演化迴圈觸發 |
| `update-daemon.sh` | daemon 自動更新 |
| `clean-history.sh` | log/history 清理 |
| `scrub-public.sh` / `sync-to-public.sh` / `pre-open-source-checklist.sh` | 公開 repo 同步前清理/檢查 |
| `verify-binary.sh` / `verify-binary.ps1` | 跨平台 binary 健康檢查 — 詳見 `VERIFY-BINARY.md` |
| `ios-shortcuts-fetch.md` | iOS Shortcuts 整合說明 |

> 頂層尚有多個 `check-*.ps1|sh`(doc-tree / mermaid / boot-network / test-citations 等
> 倉庫不變式檢查)、`test-*` / `tui-tier*` / `validate-*` / `demo-*` / `win-*` 開發輔助腳本。
> 完整清單以 `git ls-files scripts/*.{sh,ps1,py}` 為準。

## 子目錄地圖

| 子目錄 | 用途 |
|---|---|
| `bring-up/` | 新節點冷啟動(Android Termux、Oracle Ampere) |
| `ci/` | CI 不變式檢查(persona 直存、PR serve、tauri command 註冊 allowlist) |
| `coord/` | 多節點協調(狀態彙整 `gen-status.sh`) |
| `demos/` | 展示用腳本(多模型比較) |
| `dev/` | 內部維運:含特定環境假設(IP/路徑/節點名),不給一般用戶 — cluster-install、GCP 部署(legacy)、Windows 節點驗證、embed sync |
| `dev-cluster/` | 分散式開發叢集:節點收集、lease/coordinator、baseline 同步、keepalive(自帶 `README.md`) |
| `dev-loop/` | 治理化自動開發迴圈:backlog claim、auto-merge、deviation handler、commute keepalive(自帶 `README.md` + `examples/`) |
| `e2e/` | 端到端旅程(iOS/Mac full-lifecycle、TUI 全程、debug bundle) |
| `eval/` | promptfoo 評測 harness(`run.sh` + `promptfooconfig.yaml`,自帶 `README.md`) |
| `hooks/` | git hooks(pre-commit / pre-push + 安裝器 + 測試) |
| `local-ai/` | 同機呼叫其他本地 AI CLI 做 review(`review.sh`) |
| `spectyn-test/` | 最大的測試家族(47 檔):harness + lib + fixtures + cases(自帶 `README.md` / `README.zh-TW.md`) |
| `qa/` | 程式碼 lint(event payload、tauri command、selftest、Windows 回歸) |
| `release/` | epic 驗收計分 + scoreboard |
| `selftest.d/` | 編號自測階梯(00-binary → ...):doctor / serve / run / mcp / tui fuzz 等 |
| `ship-gate/` | 出貨 gate(v2 本地覆蓋率) |
| `tdd/` | TDD 迴圈驅動(next/run/status/mark-done,自帶 `README.md` / `README.zh-TW.md`) |
| `tests/` | 安裝偵測 bats 測試 |

> **AI 審查產物(舊 `scripts/ai/output/`)已於 2026-06-19 移到 `docs/ai-reviews/`**
> —— 腳本樹不再放 .md 快照。
