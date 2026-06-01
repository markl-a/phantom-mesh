# SPEC-23 Coach Engine — Windows Mockup（視覺稿）

> **Stage 2/3** · [線框稿（wireframe）](./SPEC-23-coach-engine-windows-wireframe.md) → 視覺稿（mockup，配色 + 文案 + a11y）→ [原型（prototype，待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-28
> **Scope**: Windows only — 回顧卡（review card）配色（溫和、非 dashboard 冷色）+ shame-free（不羞辱）文案終值 + model/cost footnote（模型/成本註腳）樣式 + 三態（full / action-cooking / empty）視覺 + Narrator（朗讀器）AutomationName。沿用 SPEC-20/21/22 mockup 的 design token（設計變數）速查。
> **Spec**: [`SPEC-23-SYSTEM-coach-engine`](../specs/v060-deep-spec/SPEC-23-SYSTEM-coach-engine.md) · [`SPEC-42`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md) · [`SPEC-02-FOUNDATION-design-tokens`](../specs/v060-deep-spec/SPEC-02-FOUNDATION-design-tokens.md)

## 設計溯源（trace）

| 維度 | 對應 |
|---|---|
| **BIG-GOAL pillar** | **P3 進化網**（反思迴圈）；cross-cut P2 / P4 / P1；操作原則 **shame-free** hard contract |
| **Source spec** | SPEC-23-SYSTEM-coach-engine |
| **Platform** | windows（桌面） |
| **Pipeline stage** | 2/3 mockup |

## 為什麼 coach 回顧卡有獨立 Windows mockup

wireframe 鎖了版型 + shame-free 契約；本檔鎖會影響觀感的視覺：
1. **回顧卡配色 = 溫暖非冷** — 不能長得像監控 dashboard（冷藍 / 數據感 = 審判感）
2. **shame-free 文案終值** — 每句話的最終措辭（i18n key）+ 醫療免責語氣
3. **三態視覺**（full / action-cooking / empty）的情緒一致性（都溫和）
4. **model/cost footnote** 低調樣式（透明但不搶戲）
5. **Narrator AutomationName** — 朗讀順序避免「審判感」

## Design token 對映（per SPEC-02，沿用前作 + coach 暖調）

| Token | Hex | coach 用途 |
|---|---|---|
| `phantom-bg` | `#0f0f1a` | Coach tab 背景 |
| `phantom-card` | `#1a1a2e` | 回顧卡背景 |
| `phantom-card-warm` | `#221a2e`（card 偏紫暖 8%） | **tomorrow-action 區塊背景** — 跟事件摘要區（冷）區隔，給「建議」一點溫度 |
| `phantom-primary` | `#8ab4f8` | tomorrow-action 強調字、retry 鈕 |
| `phantom-success` | `#81c995` | 正向觀察（非必要、克制用） |
| `phantom-muted` | `#6b6b80` | footnote、醫療免責、日期 |

→ **不用 `phantom-danger` / `phantom-warning`**（紅 / 橘）任何地方 — coach 回顧沒有「錯誤」或「警告」語意，全程暖中性。

## 文案 keys（per SPEC-05 i18n；shame-free 終值）

| key | 繁中 | English |
|---|---|---|
| `coach.card.date` | {date} 的回顧 | Your {date} review |
| `coach.card.events_header` | 昨天你： | Yesterday you: |
| `coach.card.observations_header` | 觀察： | Noticing: |
| `coach.card.action_header` | 今天試一個小動作： | One small thing to try today: |
| `coach.action_cooking` | 今天的回顧好了，建議還在想... | Review's ready — still thinking of a suggestion... |
| `coach.btn.retry` | 再想一個建議 | Think of another |
| `coach.btn.no_help` | 沒幫助 | Not helpful |
| `coach.btn.history` | 看歷史回顧 | Past reviews |
| `coach.empty.title` | 昨天沒有記錄 | Nothing logged yesterday |
| `coach.empty.body` | 記一餐 / 一段專注 / 一個習慣，明天就有回顧了 | Log a meal, a focus session, or a habit — and tomorrow you'll get a review |
| `coach.disclaimer` | 這是陪伴不是醫療建議 | Companionship, not medical advice |
| `coach.footnote` | {time} 由 {model} 產出 · {cost} | by {model} at {time} · {cost} |

**shame-free 文案規則**（per SPEC-23 §1 lint）：所有 key 禁含「你又 / 你終於 / 你居然 / 你怎麼又 / 還不」；events header 用「昨天你：」陳述，不用「你只 / 你才」。

## Lucide icon 對映

| 角色 | Lucide icon | 用途 |
|---|---|---|
| 醫療免責 | `info` | disclaimer 行首，14px phantom-muted |
| 明日行動 | `sparkle` | action 區標頭，16px phantom-primary（「靈感」感，非「待辦」checkbox） |
| 歷史 | `history` | 歷史回顧鈕，16px |
| 沒幫助 | `thumbs-down` | feedback，14px phantom-muted（小、不搶戲） |
| 再想一個 | `refresh-cw` | retry，14px |

**刻意不用** `check-square` / `alert` — action 不是「待辦清單」（壓力），是「邀請嘗試」。

## 螢幕 A — 今日回顧卡（full，視覺）

```
+--------------------------------------------------+
|  5/27 (二) 的回顧                                |   date 16px text-primary
|                                                  |
|  昨天你：                                        |   events_header 13px phantom-muted
|   水 6 . 專注 45 分 . 冥想 1 . 午餐 ~540 kcal     |   events 14px text-secondary（事實、冷靜）
|                                                  |
|  觀察：                                          |   observations_header 13px phantom-muted
|   下午 3 點後沒再喝水 . 專注都在早上             |   14px text-primary
|  ................................................|
|  +----------------------------------------------+|   <- phantom-card-warm 暖塊
|  | (sparkle) 今天試一個小動作：                 ||   action_header phantom-primary
|  |   下午 3 點放一杯水在桌上                     ||   action 15px text-primary（最大、最暖）
|  +----------------------------------------------+|
|                                                  |
|  [history] 看歷史回顧            [thumbs-down]   |   底列 phantom-muted 小鈕
|  (info) 這是陪伴不是醫療建議                      |   disclaimer 12px phantom-muted
|  ......... 21:03 由 Claude 產出 · $0.012 ........|   footnote 11px phantom-muted（最低調）
+--------------------------------------------------+
```

**設計重點**：
- **視覺層級**：tomorrow-action 在暖塊內、字最大 — 是卡片的「主角」（其餘是鋪陳）
- **events 區冷靜**（text-secondary）、**action 區溫暖**（warm bg + primary 標頭）— 用色溫區隔「事實」vs「邀請」
- **footnote 最低調**（11px muted）— 透明揭露 model + cost，但不搶戲
- 整卡無紅 / 無橘 / 無驚嘆號 / 無評分星

**Narrator AutomationName**：
- 卡：「{date} 的回顧。」→ 「昨天的記錄：{events}」→ 「觀察：{takeaways}」→ 「今天可以試：{action}」（用「可以試」非「你應該」）
- footnote 朗讀順序**最後**（不先報成本，避免「這要花錢」的焦慮先入）

## 螢幕 B — Action-cooking 變體

action 暖塊換成：
```
|  +----------------------------------------------+|
|  | 今天的回顧好了，建議還在想...                ||   coach.action_cooking，phantom-muted 斜體
|  | [refresh-cw] 再想一個建議                    ||   retry 鈕 phantom-primary
|  +----------------------------------------------+|
```
- events + observations 仍正常顯示（只 action 區降級）
- 「再想一個」refresh 圖示 + 文案溫和（非「失敗，重試」的 error 語氣）
- 連 3 次 reject → 暖塊換「今天先這樣，明天再聊」（`coach.action_giveup`）— 溫柔收尾不報錯

## 螢幕 C — 空狀態

```
|  昨天沒有記錄                                    |   empty.title 16px，中性
|  記一餐 / 一段專注 / 一個習慣，明天就有回顧了     |   empty.body 14px phantom-muted
|  [ 記一餐 ]  [ 開始專注 ]  [ 打卡習慣 ]          |   3 個 phantom-primary outline 鈕 → 深連結
```
- 空狀態插畫（可選）：一個淡色 `sparkle` outline，**不畫哭臉 / 空盤子**（避自責暗示）

## ActionCenter toast（21:00 派送，SPEC-24 範疇，本檔給文案）

```xml
<toast scenario="reminder" activationType="protocol"
       launch="phantom-mesh://coach/review?id={review_id}">
  <visual><binding template="ToastGeneric">
    <text>今天的回顧好了</text>
    <text>{one_line_action} . 點開看看</text>
  </binding></visual>
</toast>
```
- **無 `<audio>`**（晚上 21:00、溫和不打擾，shame-free + P4）
- text 只給 action 一句預覽（不報「你昨天只做了 X」）

## Cross-platform invariants 對齊

- shame-free 配色（無紅無橘、暖中性）跨 5 平台一致
- tomorrow-action「恰一個 + 暖塊強調」跨平台一致（手機卡片亦同視覺層級）
- 醫療免責文案跨平台同字（法務一致）

## 已決（per wireframe + 本檔拍板）

- 回顧卡暖中性配色（無紅橘無評分，本檔）；action 用暖塊 + sparkle（本檔）
- footnote 最低調、Narrator 最後朗讀成本（本檔）
- 空狀態 / cooking / giveup 全溫和不自責（本檔）

## 開放問題（留 prototype / 後續）

1. 空狀態插畫要不要做（sparkle outline）— 待視覺資源決策
2. footnote 成本顯示精度（$0.012 vs ~$0.01）— 待 SPEC-07 observability 對齊
3. 歷史回顧的趨勢圖（週 / 月）— v0.7+，本版只列表

## 下一步

**Stage 3 prototype**：回顧卡三態切換 + retry 互動 + Narrator 朗讀順序 + HTML 草圖。
