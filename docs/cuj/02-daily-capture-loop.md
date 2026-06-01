# CUJ-02: daily capture loop (food + focus + habit → coach review)

> **Outcome**: user 一天內 log 多筆生活事件 (任 1 模態)，隔日 7:00 收到 coach 的 daily review markdown，內含 yesterday pattern + 建議。
>
> **Lifecycle phase**: daily use — **主 retention driver、產品本體**。CUJ-01 過關後、user 從 day-2 起活在這條 loop 內。
>
> **Primary pillar**: P2 (multimodal — 3 模態都點到) + P3 (evolve — coach 越用越懂 user)
>
> **SLO**: chip tap → event 寫入 p50 < 50 ms (SPEC-22 G6) / daily coach review 自動觸發成功率 ≥ 95% / coach review 抵達 user (notification or markdown) ≤ 5 min after 觸發

## SPECs implementing this CUJ

| SPEC | 角色 |
|---|---|
| SPEC-20 capture-food | photo → AI 抽食物 + 量 |
| SPEC-21 capture-focus | 專注時段音訊 → ASR + takeaway |
| SPEC-22 capture-habit | chip palette + freetext |
| SPEC-23 coach-engine | daily aggregator + LLM pattern review |
| SPEC-24 coach-delivery | markdown / Telegram / app notification |
| SPEC-14 LLM-providers | coach 跑哪個 model (fallback chain) |
| SPEC-16 event-storage | sqlite + FTS5 retrieval |

## Happy path (sketched daily loop, ~6 ops over 1 day)

1. (07:30) user 拍早餐 → SPEC-20 → 抽出「燕麥 80g + 香蕉 1 根」→ event 寫入
2. (09:00-12:00) user 啟動 SPEC-21 focus session → 環境音 + 結束 takeaway「KSCH 第 7 章 done」
3. (10:15) user 點 widget「咖啡」chip → SPEC-22 record + streak 顯示
4. (15:30) user CLI `phantom habit "讀完 SICP ch3"` → SPEC-22 freetext fallback
5. (21:00) user 拍晚餐 → SPEC-20
6. (隔日 07:00) coach.agent → SPEC-23 aggregator 拉昨日 events → SPEC-14 跑 Gemini/Groq → 產出 review markdown
7. (07:00-07:05) SPEC-24 把 markdown push 給 user (notification / Telegram / in-app)

## Degraded paths (must-test)

### offline (整天沒網)
- 1-5 步驟寫入本地 sqlite 一切如常
- 6 步驟 coach 跑不出來（沒 LLM 連線）→ 隔日上線後 catch-up: review 內提到「補做 5/30 到 5/31 的 review」
- 不掉 event

### LLM provider 全部 quota exhausted (SPEC-14 fallback chain 用完)
- 6 步驟 fallback to local LLM (ollama)、若本地也沒設定 → coach review 退化成「pure stats + template summary, no LLM insight」
- user 看到通知「coach 今日 stats-only 模式、3 條 LLM provider 都到上限」

### widget tap 但 service 沒在跑 (Android background killed by MIUI)
- SPEC-22 widget tap 寫 SharedPreferences queue
- 下次 open app → drain SP queue 到 EventStore
- coach review 仍能看到（雖然 timestamp 跟 user tap 時間有飄）

### user 一天 0 筆 event
- 隔日 7:00 coach 仍應跑、產出「昨日無資料、建議：開始記第一筆 habit」review
- **不**靜默跳過、user 才不會以為產品壞了

## Test scaffolding

- **Maestro YAML**: `flow/cuj-02-daily-loop.maestro.yaml` (mobile widget tap + photo capture)
- **Playwright TS**: `flow/cuj-02-daily-loop.playwright.ts` (desktop CLI + tray menu)
- **Promptfoo eval**: `eval/cuj-02-coach/cases.yaml` ── 30 frozen examples of (events JSON → expected review themes)、trajectory assertion 確認 SPEC-14 fallback chain 順序
- **Manual playbook**: `playbook/cuj-02.md` ── 24h 真實使用 checklist
- **Synthetic monitor**: nightly job at 06:55 staging → expect review at 07:00 ± 5min

## 已知 gap

- ✓ SPEC-20/21/22 capture 都有 code (39/22/45 ref)
- ✓ SPEC-23 coach 有 code + test (29 / 5 ref)
- ✓ SPEC-14 fallback chain 5/27 validated
- ✗ daily review pipeline 自動觸發沒被 e2e 測過
- ✗ degraded path "LLM 全 quota" 沒測 (應該已有 SPEC-14 fallback、但 stats-only mode 未實作)
- ✗ Promptfoo eval set 不存在
- ✗ 24h soak test 不存在

## 出範圍

- mobile → desktop 同步 → CUJ-03
- coach review 中 user 怎麼採取 action → 不在 v0.6.0 (v0.7.0+ skill extraction)
