# CUJ-04: degraded states (offline / quota / token expired / no LLM key)

> **Outcome**: 無論哪種「不是 happy path 的環境」(offline / LLM quota 用完 / broker token 過期 / 沒設 LLM key)，core capture 功能仍可用、user 看到誠實的狀態訊息、不靜默失敗、不掉資料。
>
> **Lifecycle phase**: degraded — **多數團隊漏測、bug 出在這**。production-ready 與否的分水嶺。
>
> **Primary pillar**: P1 (local-first 不強制雲端) + P4 (失敗時資料仍加密、不退化成 plaintext)
>
> **SLO**: 每種 degraded scenario 的 user-visible error message 必須含 (a) 發生什麼 (b) user 能做什麼 (c) 資料安全嗎 / 0 個 silent failure mode / 0 個 plaintext degradation

## SPECs implementing this CUJ

| SPEC | 角色 |
|---|---|
| SPEC-04 error-catalog | 統一錯誤碼 + user-facing 文案 (繁中 + en) |
| SPEC-14 LLM-providers | fallback chain + quota handling |
| SPEC-15 broker-vault | token expired graceful path |
| SPEC-13 encryption-age | 沒 identity.key 時的 fail-loud 不 fail-silent |

## 各種 degraded scenario + 期望行為

### S1: 完全 offline (沒任何網)
- capture (food/focus/habit): ✓ 寫本地 sqlite 正常
- coach review: ✗ 不跑、UI 顯示「目前離線、隔日 review 補做」
- cross-device sync: ✗ queue 留本地、上線後 drain
- 任何 LLM 操作: 退化 to local LLM (ollama 若設) 或顯示「需要網路 / local LLM」

### S2: LLM provider quota exhausted (按 SPEC-14 fallback chain)
- 期望 fallback 順序: codex → agy → gemini → claude → opencode → groq → local ollama
- chain 用完 → coach review 進 stats-only mode (純數字 + template 文案、無 LLM insight)
- user 看到通知「coach 今日 stats-only 模式 — 3 條 LLM provider 都到上限」
- 第二天若有 quota 恢復、自動回 normal mode

### S3: broker token expired (中途過期)
- capture 操作: ✓ 本地仍可寫
- sync attempt: 401 → UI 顯示「同步暫停、設定 → 重新登入」
- coach review on this device: 仍可跑 (用本地 events)
- user 重 login 後 queue 自動 drain、無資料遺失

### S4: 沒設任何 LLM key (fresh install + 沒過 setup wizard)
- capture: ✓ 寫本地、不靠 LLM
- coach review trigger: 顯示「設定 → providers → 加 Groq / Gemini 免費 key (Cmd-K 開精靈)」
- 不靜默不跑 coach、不假裝 coach 跑了但沒 output

### S5: identity.key 損毀 / 磁碟滿 (P4 fail-loud)
- 不降級成 plaintext store ← 絕對紅線
- UI 顯示「身分金鑰無法讀取/寫入 — 暫停 capture」+ 修復選項
- 修復後可繼續、之前 event 仍可讀

### S6: sqlite database corrupted (磁碟錯)
- 顯示「事件儲存損毀 — 自動 backup 到 events.sqlite.corrupt-<ts>」+ 引導 export 殘存
- 新建 empty events.sqlite、繼續工作 (不 block user 一直無法 log)

### S7: Android background killed (MIUI / Samsung battery saver)
- BootReceiver 重啟 mesh node 服務
- widget queue (SharedPreferences) 在 main app 開啟時 drain 進 EventStore
- coach review 跨 background-kill 後仍能算 (events 不掉)

## Test scaffolding

- **Maestro YAML**: `flow/cuj-04-degraded.maestro.yaml` (模擬 offline mode + quota + token expired 各跑一遍)
- **Playwright TS**: `flow/cuj-04-degraded.playwright.ts` (desktop 模擬 — `tc netem` 模擬丟包 / API mocked 401)
- **Integration test**: `core/tests/cuj04_degraded_states.rs` ── 7 種 scenario 各一個 #[test]
- **Promptfoo eval**: `eval/cuj-04-fallback/cases.yaml` ── 驗 SPEC-14 fallback chain order
- **Manual playbook**: `playbook/cuj-04.md` ── 7 scenario checklist (含 flight mode 跑一天)
- **Synthetic monitor**: 每日 staging 跑 1 個 scenario、輪替 7 條 (每週掃完一輪)

## 已知 gap

- ✓ SPEC-04 error catalog 文件存在、code 有 ref
- ✓ SPEC-14 fallback chain 5/27 validated (但 stats-only mode 未實作)
- ✗ **degraded UX 從沒系統測過** ── 多數錯誤 surface 都沒 user-facing 文案
- ✗ S5 / S6 不存在 (沒人碰過、可能 silently degrade)
- ✗ S7 background revival 是 SPEC-22 follow-up「未 ship」項

## 為何這條最被忽略

「測 happy path 容易、測 degraded path 麻煩」── 沒人主動跑「斷網 24h」測試。但**用 spectyn 的人多數會經歷至少 1 個 degraded scenario / 月**。漏這條 = 部分 user 沉默離開。
