# CUJ-01: install → first habit captured

> **Outcome**: user 從 0 (沒裝過 phantom) 到 ≤90 秒內成功 log 一筆習慣事件，且該事件可在隔日 coach review 看到。
>
> **Lifecycle phase**: activation (Day 1) — **最高槓桿** CUJ。day-1 churn 全業界 20-25%，唯一預測指標是「user 是否完成 meaningful first action」。第一條 habit event 就是這個 meaningful action。
>
> **Primary pillar**: P2 (multimodal — text 子模態落地) + P1 (mesh — 本地 first-tap 就能寫入、不需登入)
>
> **SLO**: install 完成 → 第一筆 habit event 寫入 sqlite p50 < 90s、p95 < 180s；成功率 ≥ 90%

## SPECs implementing this CUJ

| SPEC | 角色 |
|---|---|
| SPEC-28 30s-hello | onboarding 主流程 (install → first run) |
| SPEC-12 identity-keypair | 第一次啟動產 identity.key (P4 加密前置) |
| SPEC-13 encryption-age | first-launch 加密初始化 |
| SPEC-16 event-storage | sqlite + FTS5 建表 |
| SPEC-22 capture-habit | starter palette 12 chip + first chip tap |
| SPEC-30/33/40/44 platform foundations | 各平台 install 體驗 |

## Happy path (7 steps)

1. user 從 phantommesh.io 或 store 下載 + install
2. 第一次開啟 app → 看到「Phantom Mesh — 你的私人 AI 隊伍」歡迎畫面 + 一鍵繼續
3. 系統背景產 identity.key (Ed25519 + HKDF event-key) + 初始化 ~/.phantom-mesh/events.sqlite
4. 顯示 12 starter habit palette (水 / 咖啡 / 運動 / 冥想 / ...)
5. user tap 任一 chip (例: 水) → 直接寫入第一筆 habit event (P4 加密)
6. 顯示「✓ 已記錄。streak=1」+ 引導下一步 (「想看到 coach review？登入 phantommesh.io」 / 「先逛逛」)
7. 完成 ── ≤90 秒、≤3 tap (welcome / palette / chip)

## Degraded paths (must-test)

### offline (沒網)
- step 5 仍應寫入本地 sqlite（**不**要求網路）
- step 6 不顯示「coach review」CTA、改顯示「目前離線、之後同步」
- 重新上線後自動同步（不需 user 動作）

### no identity.key (P4 fallback)
- step 3 寫 identity.key 失敗（磁碟滿 / 權限拒）→ 顯示明確錯誤、**不**降級成 plaintext store
- user 修正後重試應成功

### user 直接 quit 第 2 步沒到 step 5
- 下次 open app **不**從頭歡迎、直接到 palette（已 identity.key 視為已 onboard）
- day-1 內若 user 還沒 first chip tap、第 2 次 open 顯示「你還沒記第一筆、現在試試？」

### Android 13+ POST_NOTIFICATIONS 拒絕
- 不 block 第一筆 habit event（log 不需要通知權限）
- 但顯示提示「未來 coach review 提醒收不到、之後可在設定開」(SPEC-22 follow-up gate)

## Test scaffolding (TBD when CUJ approved)

- **Maestro YAML**: `flow/cuj-01-install-first-habit.maestro.yaml` (iOS + Android)
- **Playwright TS**: `flow/cuj-01-install-first-habit.playwright.ts` (mac/win/linux desktop)
- **Promptfoo eval**: n/a (CUJ-01 不需要 LLM、純 local install + capture path)
- **Manual playbook**: `playbook/cuj-01.md` ── 重灌測試 checklist
- **Synthetic monitor**: install.sh + first-tap automation against staging

## 已知 gap (current state vs SLO)

- ✓ SPEC-28 30s-hello 在 main 有實作 (8/10 SPEC ref)
- ✓ SPEC-22 starter palette 在 main 有實作 (45/0 ref, no automated test)
- ✗ 沒有 CUJ-01 整條 e2e — install + first chip 沒有自動化驗證
- ✗ day-1 retention SLO 沒被測量、沒 baseline
- ✗ Android 13+ POST_NOTIFICATIONS edge 未測

## 出 CUJ 範圍 (deliberately not here)

- 登入 phantommesh.io broker → CUJ-03 (cross-device resume) 的事
- 第一次 food / focus capture → CUJ-02 (daily loop) 的事
- 多 device 同步 → CUJ-03
