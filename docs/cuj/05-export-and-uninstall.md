# CUJ-05: data export + uninstall (GDPR + churn + re-install safety net)

> **Outcome**: user 想離開 → 能在 ≤ 1 min 內導出所有資料 (events + habit + coach review) 為可攜格式、能從本地 + broker 真實刪除、能重新安裝後選擇是否恢復身分。
>
> **Lifecycle phase**: churn / safety — **法務 + 信任**。GDPR Article 17 (right to erasure) 強制、不測會吃罰單。
>
> **Primary pillar**: P4 (encryption-first 涵蓋「user 想真的刪掉」) + 信任承諾 (BIG-GOAL: 「只你能讀」也包含「能完全刪」)
>
> **SLO**: 完整 export ≤ 60s / uninstall 後 broker server-side data 刪除 ≤ 24h 並可驗證 / 重灌恢復身分 ≤ 90s

## SPECs implementing this CUJ

| SPEC | 角色 |
|---|---|
| SPEC-12 identity-keypair | identity.key 是 export 的核心 |
| SPEC-13 encryption-age | events 加密 → decrypt for export |
| SPEC-15 broker-vault-sync | broker `DELETE /vault/*` 真實刪除 |
| SPEC-16 event-storage | sqlite events 完整 dump |
| SPEC-50 broker-api | server-side delete endpoint + tombstone |

## Happy path A: data export (準備離開但不刪)

1. user 開 app → 設定 → 「導出我的所有資料」
2. 系統產生 `spectyn-mesh-export-<ts>.tar.gz`、內含:
   - `identity.key` (P4 加密身分)
   - `events.sqlite` (所有 capture event, 加密原樣)
   - `chip_palette.sqlite` (habit chip 設定)
   - `coach-reviews/` (markdown daily review)
   - `README.txt` 說明如何用 spectyn CLI 再 import 或自己解 age
3. 下載到 user 選的位置 (Downloads / 隨身碟)
4. UI 顯示「✓ 導出完成、N events、X MB」
5. user 可以離開或選擇繼續 happy path B (真實刪除)

## Happy path B: uninstall (要真實刪除)

1. user 從 export 完之後 (or 直接) → 「永久刪除我的資料」
2. UI 顯示「這會 (a) 刪除本機 ~/.spectyn-mesh/ (b) 從 phantommesh.io broker 刪除所有 sealed blob (c) 撤銷 broker token、無法復原」+ 二次確認
3. user 二確 → 系統:
   - DELETE all broker vault entries via SPEC-50 API
   - DELETE broker session (token 失效)
   - DELETE ~/.spectyn-mesh/ 整個目錄
4. UI 顯示「✓ 完成、N events 已刪、broker 24h 內完成 GC、可放心 uninstall app」
5. user uninstall app
6. (24h 內) broker 後台 confirm hard-delete + tombstone log
7. 完成 ── user 可 verify 重新試「假登入」失敗

## Happy path C: re-install with identity recovery

1. user 之前有跑 happy path A export、保留 `identity.key`
2. 重新 install spectyn + 開啟 → 「我有 identity.key」選項
3. user 載入 .key → 系統 verify 合法、繼續恢復流程
4. (若 broker 還在) 用 identity 驗證後 sync 所有舊 events 下來
5. (若 broker 已刪) export 中的 events.sqlite 自帶 → import → 恢復完成
6. 完成 ≤ 90s ── user 像沒中斷一樣繼續

## Degraded paths (must-test)

### export 中途斷網 / 中斷
- 半成品 `.tar.gz` 應被刪、不留垃圾
- user 看到「導出中斷、未生效、可再試」、原資料完整

### broker 已關 server (server side outage)
- happy path B step 3 (a) (c) 本地刪 OK
- (b) broker delete 失敗 → queue retry、UI 顯示「本機已刪、雲端待 24h 重試、可手動到 phantommesh.io 後台確認」

### user 重灌沒留 identity.key (典型遺失)
- 顯示明確警示「沒 identity.key 無法解 events、若 broker 還有 sealed blob 也讀不了」
- 不假裝可以恢復、不 silently 開新身分

### user 在多裝置只在 1 個地方刪
- broker delete 是 account 級、同 token 的其他裝置下次 sync 會收到「broker 空了」
- 其他裝置本地 sqlite 仍存在 → 顯示「broker 已被另一裝置清空、本機資料保留」+ 引導刪本機

## Test scaffolding

- **Maestro YAML**: `flow/cuj-05-export-uninstall.maestro.yaml` (mobile uninstall + re-install)
- **Playwright TS**: `flow/cuj-05-export-uninstall.playwright.ts` (desktop export + verify .tar.gz 結構)
- **Integration test**: `core/tests/cuj05_export_delete.rs` ── tarball 結構 + broker DELETE 呼叫 + 重灌 import
- **Promptfoo eval**: n/a
- **Manual playbook**: `playbook/cuj-05.md` ── **GDPR 合規 walkthrough** + 簽 user 跑過
- **Synthetic monitor**: 每月 staging 跑完整 A→B 循環、verify broker GC 在 24h 內

## 已知 gap

- ✗ **整條 CUJ 完全沒實作** ── 沒 export UI、沒 DELETE 流程、沒重灌 import
- ✗ 沒 GDPR 合規文件
- ✗ broker hard-delete 24h GC job 不存在
- ✗ ⚠️ 這是 **release-blocker** for v0.6.0 公開：沒法律安全網不能 ship 給歐盟 user

## 立刻可做的最小子集 (v0.6.0 MVP)

1. CLI `spectyn export --to <path>` ── 產 tar.gz、不需 UI
2. CLI `spectyn delete-all --confirm` ── 刪本機 + 呼 broker DELETE
3. broker 端 `DELETE /vault/all` endpoint + 24h GC
4. README 說明 user 怎麼自己 import 個別 events (用 age + sqlite3)

UI 化、自動 import 可延 v0.6.1。
