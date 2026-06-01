# SPEC-23 Coach Engine — Windows Wireframe（線框稿）

> **Stage 1/3** · 線框稿（wireframe，低保真版型骨架）→ [視覺稿（mockup，待補）] → [原型（prototype，待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-28
> **Scope**: Windows only。SPEC-23 是 **daily review（每日回顧）引擎**（背景跑、產出回顧），非 capture（記錄）pipeline。本檔描述桌面的兩面：(1) Task Scheduler（Windows 工作排程器）每日 21:00 觸發；(2) 回顧卡片在 main window（主視窗）的呈現 + shame-free（不羞辱）狀態。派送通道（toast / Telegram / email）屬 SPEC-24，本檔只接到「review ready（回顧就緒）」handoff（交棒）點。
> **Spec**: [`SPEC-23-SYSTEM-coach-engine`](../specs/v060-deep-spec/SPEC-23-SYSTEM-coach-engine.md) · [`SPEC-42`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md)

## 設計溯源（trace）

| 維度 | 對應 |
|---|---|
| **BIG-GOAL pillar** | **P3 進化網**（coach 隔日 review = 反思迴圈）；cross-cut **P2 多模態**（events 含 image/audio 分析）、**P4 加密為先**（解密讀 events）、**P1 跨裝置 Mesh**（5 OS scheduler 抽象，Windows = Task Scheduler）。**操作原則 shame-free**（教練語氣永不批判）為 hard contract |
| **Source spec** | SPEC-23-SYSTEM-coach-engine |
| **Platform** | windows（桌面） |
| **Pipeline stage** | 1/3 wireframe |

## 為什麼 coach engine 要 Windows wireframe

coach 不是 user 主動點的功能，是**背景定時跑、隔天給回顧**。Windows 兩個平台特定點：
1. **scheduler = Task Scheduler**（非 iOS BGTaskScheduler / Android WorkManager）— 桌面準時性好（不像行動端會延到半夜），21:00 觸發可靠
2. **回顧呈現在 real window（真窗）main window 的 Coach tab（分頁）** + ActionCenter（通知中心）toast 提醒；桌面有大畫面可完整 render markdown（行動端要捲）

## 縮寫對照表

> - **daily review（每日回顧）**：隔日由 coach 引擎產出「昨天怎樣 + 今天試一個小動作」的 markdown
> - **Task Scheduler（工作排程器）**：Windows 內建定時任務系統
> - **shame-free（不羞辱）**：教練語氣硬規則 — 不可出現「你又 / 你終於 / 你居然」等批判句
> - **tomorrow-action（明日行動）**：回顧結尾 LLM 提的「一個最小可做的動作」
> - **fail-closed（失敗即關閉）**：lint（檢查器）攔到可疑輸出 → 寧可整篇不出貨
> - **handoff（交棒）**：引擎產完回顧 emit `coach.review.ready` 事件給 SPEC-24 派送
> - **BGTaskScheduler / WorkManager / FTS5 / LLM**：見 SPEC-23 §1

## 入口點 + 觸發

| 進入點 | v0.6.0 | 說明 |
|---|---|---|
| **Task Scheduler `PhantomMeshCoachReview` 每日 21:00** | ✅ | 自動觸發 `run_daily_review(yesterday)`；user 不需手動 |
| Main window `[Life / Coach tab]` | ✅ | 看今日回顧 + 歷史回顧列表 |
| ActionCenter toast「今天的回顧好了」 | ✅（SPEC-24 派送） | 21:00 跑完 → 點 toast 跳 Coach tab |
| CLI `phantom coach review --date today` | ✅ | 手動觸發 / 重看 |
| Settings → Coach → 回顧時間 | ✅ | 改 21:00 預設（重設 Task Scheduler trigger 時間） |

**v0.6.0**：Task Scheduler 自動跑 + Coach tab 呈現 + Settings 改時間。toast 派送見 SPEC-24。

## 螢幕 A — Coach tab：今日回顧卡（完整版）

```
+--------------------------------------------------+
| Coach                                            |  ← tab header
+--------------------------------------------------+
|  5/27 (二) 的回顧            21:03 由 Claude 產出 |  ← 日期 + 產出時間 + model footnote
|                                                  |
|  昨天你：                                        |  ← events summary（aggregator 純格式化）
|   . 喝水 6 次 . 專注 2 段共 45 分 . 冥想 1 次     |
|   . 午餐沙拉 ~540 kcal                            |
|                                                  |
|  觀察：                                          |  ← takeaways（LLM，shame-free）
|   . 下午 3 點後沒再喝水                           |
|   . 專注時段都在早上                              |
|                                                  |
|  今天試一個小動作：                              |  ← tomorrow-action（恰一個，最小）
|   下午 3 點放一杯水在桌上                         |
|                                                  |
|  [ 看歷史回顧 ]                    [ 沒幫助 ]     |  ← history + feedback（非評分，只標記）
|  (info) 這是陪伴不是醫療建議                      |  ← 醫療免責（hard rule）
+--------------------------------------------------+
```

**設計重點（shame-free 契約視覺化）**：
- **語氣中性** — 「昨天你：」陳述事實，不寫「你只喝了 6 次水（不夠）」這種隱含批判
- **恰一個 tomorrow-action** — 不給清單（清單 = 壓力）；最小可做
- **「沒幫助」非評分** — 不放 1-5 星（評分 = 自我審判）；只一鍵標記「這則沒幫助」餵 SPEC-25 skill 演化
- **醫療免責常駐** — 「陪伴不是醫療建議」（SPEC-23 hard rule，禁 diet plan / 處方）
- **不染紅 / 不畫驚嘆號** — 沒有「警告」系視覺；回顧是溫和的

## 螢幕 B — Action-cooking 變體（shame-free lint reject 後）

當 LLM 輸出被 shame-free lint 攔截（fail-closed，約 5–10%）：events summary + takeaways 仍出，但 tomorrow-action 區換成：

```
|  今天的回顧好了，但建議還在想 ...                |  ← 取 coach.action_cooking
|  [ 再想一個建議 ]                                |  ← retry：重跑 LLM（不重算 aggregator）
```
- **寧可無建議也不出 shaming 建議**（SPEC-23 §1 fail-closed）
- retry 重呼 LLM（aggregator brief 已快取，省成本）；連 3 次 reject → 「今天先這樣，明天再聊」收尾（不無限 retry）

## 螢幕 C — 空狀態（昨天沒 events）

```
|  昨天沒有記錄                                    |
|  記一餐 / 一段專注 / 一個習慣，明天就有回顧了     |  ← 引導去 capture（不責備「你昨天沒記」）
|  [ 記一餐 ]  [ 開始專注 ]  [ 打卡習慣 ]          |  ← 深連結到 SPEC-20/21/22
```
- 空狀態**不羞辱**（不寫「你昨天什麼都沒做」）— 中性引導

## 螢幕 D — 歷史回顧列表

```
|  5/27 (二)  下午多喝水                  >        |  ← 日期 + tomorrow-action 一句摘要
|  5/26 (一)  早睡 30 分鐘                >        |
|  5/25 (日)  (回顧 ready,建議 cooking)   >        |  ← action-cooking 的也列、標記
```
- 點任一 → 展開該日完整回顧卡（螢幕 A 版型）
- 純 sqlite query（`kind="coach"` rows），即時

## Task Scheduler 設定（Windows scheduler 抽象）

- task name `PhantomMeshCoachReview`、daily trigger 21:00 local、action = `phantom coach review --date yesterday --emit`
- 改時間：Settings → Coach → 回顧時間 → 重 register task（`schtasks /change`）
- 桌面準時性高（不像行動端延遲）；user 關機過 21:00 → 開機後 Task Scheduler `StartWhenAvailable` 補跑
- **不需 admin**（user-scope task，同 SPEC-43 PhantomServe 模式）

## 失敗 / 邊界（per SPEC-04 + SPEC-23 §11.1 X.coach.*）

- LLM 4 provider 全 fail → review row 仍寫（takeaways 空、`status=llm_failed`）→ Coach tab 顯 events summary + 「建議還在想」+ retry
- events 解密失敗 → `X.coach.decrypt_fail` → 該日 review skip + log（不崩、不出半截）
- Task Scheduler 沒跑（user 關機整天）→ 下次開機補跑昨日；連續多日沒跑 → 只補最近一日（不洗版）

## 待補（下一 pipeline stage）

- **Stage 2 mockup**：回顧卡配色（溫和、非 dashboard 冷色）、shame-free 文案終值（i18n SPEC-05）、model/cost footnote 樣式、Narrator a11y
- **Stage 3 prototype**：回顧卡三態（full / cooking / empty）切換 + retry 互動 + HTML 草圖
- **SPEC-24 delivery** 是獨立 feature pipeline（toast / Telegram / email 派送路由），本檔只到 `coach.review.ready` handoff
