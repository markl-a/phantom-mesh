# CUJ-03: cross-device resume (mobile log → desktop coach reads it)

> **Outcome**: user 在 iPhone widget log 一筆 chip、≤ 5 秒後 mac terminal `phantom habit streak --chip <id>` 看得到、隔日 mac coach review 用到此筆。
>
> **Lifecycle phase**: daily use (multi-device) — P1 mesh 命脈。**BIG-GOAL 句「跑在你所有裝置上」的兌現點**。
>
> **Primary pillar**: P1 (cluster-aware mesh) + P4 (encryption-first sync — broker 永遠拿不到明文)
>
> **SLO**: chip tap on iPhone → desktop 看得到 p50 < 5s / p95 < 15s / 跨裝置 event 一致性 100% / broker plaintext leak 永不發生 (regression-guarded by SPEC-15 invariant test)

## SPECs implementing this CUJ

| SPEC | 角色 |
|---|---|
| SPEC-10 mesh-rpc | P2P RPC 通道 (16 endpoints) |
| SPEC-11 mdns-discovery | 同網段裝置自動發現 |
| SPEC-15 broker-vault-sync | OAuth + sealed E2EE 跨網際同步 |
| SPEC-26 cluster-dispatch | task routing 給能執行 node |
| SPEC-13 encryption-age | 跨裝置 event 保持加密 |
| SPEC-12 identity-keypair | 跨裝置驗證身分 |
| SPEC-50 broker-api | phantommesh.io broker 服務 |

## Happy path (8 steps)

1. user 已在 iPhone 跟 mac 兩端都登入 phantommesh.io 同帳號 (broker token granted)
2. iPhone home widget tap「水」chip → SPEC-22 record_checkin
3. SPEC-15 client `seal_vault_value(event_bytes)` → broker 推 sealed blob
4. mac 端 SPEC-15 client 在 background poll (or push notification 觸發) pull
5. mac 收 sealed blob → `unseal_vault_value` 用本地 identity.key → 寫入本地 events.sqlite
6. user 在 mac terminal: `phantom habit streak --chip water` → 看到剛 log 的那筆 (current_streak=N+1)
7. (隔日 07:00) mac coach.agent 跑 daily review 時讀到本筆 → review markdown 內提到「昨日水 5 杯、其中 3 杯來自 iPhone」
8. 完成 ── 跨裝置一致性達成、broker 全程沒看過 plaintext

## Degraded paths (must-test)

### iPhone 沒網 (Wi-Fi off)
- step 2 寫 iPhone 本地 sqlite 成功
- step 3-4 推不上去、queue 留在 iPhone
- iPhone 上線後自動 retry、≤30s 後 mac 看得到
- **不**掉 event

### mac 整天沒開機 / 沒網
- step 4 不發生
- mac 開機後 pull 補齊、隔日 coach 仍能用到
- 若 coach 7:00 跑時 mac 還沒拉到 → review 內標「partial data (1 device unsynced)」

### broker token expired
- step 3 401 → iPhone widget 顯示靜態警示「同步暫停、請至設定重新登入」
- iPhone 本地 sqlite 仍寫得進 (P1 local-first)
- user 重新 login 後 queue 自動 drain

### broker 被攻破 (假想 worst case)
- 即使 broker 完全惡意、stored blob 仍是 sealed (SPEC-15 E2EE invariant)
- broker 看到的只有 sealed_ciphertext + HMAC + 非機密 metadata
- **regression test**: SPEC-15 `spec15_broker_vault_e2ee_regression.rs` 已存在、core/tests/

### 兩裝置同時 log 同一秒 (race)
- 兩端各自 record + push、broker 兩個 entry 都收
- mac pull 兩筆 sealed blob → unseal → 都寫進 events.sqlite (UUID 不同、不重複)
- streak 計算 dedup 用 timestamp_ms 而非 chip id alone

## Test scaffolding

- **Maestro YAML**: `flow/cuj-03-cross-device.maestro.yaml` (iPhone log + 等待 + 檢查 — 需配對 desktop assertion)
- **Playwright TS**: `flow/cuj-03-cross-device.playwright.ts` (mac CLI poll)
- **Integration test**: `core/tests/cuj03_cross_device_sync.rs` ── 兩個 EventStore instance + mock broker + verify round trip
- **Promptfoo eval**: n/a (純資料同步、無 LLM)
- **Manual playbook**: `playbook/cuj-03.md` ── 真實兩台裝置 walk-through
- **Synthetic monitor**: nightly two-device staging job

## 已知 gap

- ✓ SPEC-10 mesh-rpc 有 13 ref + test
- ✓ SPEC-11 mdns 有 ref + test
- ✓ SPEC-15 broker vault E2EE 有 regression test (`spec15_broker_vault_e2ee_regression.rs`)
- ✓ SPEC-26 cluster-dispatch 有 35 ref + test
- ✗ **整條 cross-device sync timing SLO** 沒被測 ── 沒人量過實際 5s SLO 是否達標
- ✗ degraded path "broker token expired" UX 未驗
- ✗ 同秒 race 沒測

## 出範圍

- broker server 自身 deployment / scaling → SPEC-51 / 不在 CUJ
- end user 分享 event 給他人 → 不在 v0.6.0
