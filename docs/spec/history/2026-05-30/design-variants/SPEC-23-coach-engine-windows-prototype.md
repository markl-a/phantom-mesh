# SPEC-23 Coach Engine — Windows Prototype（原型）

> **Stage 3/3** · [線框稿（wireframe）](./SPEC-23-coach-engine-windows-wireframe.md) → [視覺稿（mockup）](./SPEC-23-coach-engine-windows-mockup.md) → 原型（prototype，互動腳本 + 元件草圖）
> **Status**: draft v0.1 · **Last updated**: 2026-05-28
> **Scope**: Windows only — 互動腳本（每個可點處按下去發生什麼）+ 三態切換 + retry（重試）+ Narrator（朗讀器）focus 順序 + Nielsen 5（尼爾森 5 大易用性 heuristic）+ usability walkthrough（易用性走查）腳本 + HTML 草圖。
> **Spec**: [`SPEC-23-SYSTEM-coach-engine`](../specs/v060-deep-spec/SPEC-23-SYSTEM-coach-engine.md) · [`SPEC-42`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md)

## 設計溯源（trace）

| 維度 | 對應 |
|---|---|
| **BIG-GOAL pillar** | **P3 進化網**（反思迴圈）；cross-cut P2 / P4 / P1；操作原則 shame-free |
| **Source spec** | SPEC-23-SYSTEM-coach-engine |
| **Platform** | windows（桌面） |
| **Pipeline stage** | 3/3 prototype |

## 為什麼 coach engine 要獨立 Windows prototype

前三個 feature（food / focus / habit）都是 **user 主動觸發**的互動；coach 是 **被動接收**（背景產出、user 來看）。互動腳本重點不同：
1. **沒有「開始」動作** — user 打開 Coach tab 就看到結果（或空 / cooking）
2. **唯一互動是 retry + feedback** — 不是操作流程，是「閱讀 + 輕回饋」
3. **shame-free 在互動層的體現** — retry 不報錯、feedback 不評分、空狀態不自責

## 互動狀態機（被動，非操作 FSM）

```
[Task Scheduler 21:00] --> run_daily_review --> {
   has events + LLM ok + lint pass  --> CARD_FULL
   has events + (LLM fail | lint reject) --> CARD_COOKING --(retry x<=3)--> CARD_FULL | CARD_GIVEUP
   no events yesterday              --> CARD_EMPTY
}
user 開 Coach tab --> 顯示當日 state ；點歷史 --> HISTORY_LIST --(點某日)--> 該日 CARD_FULL
```
- 全程**無 loading 卡住 user** — review 是背景已產好的；打開即見
- CARD_COOKING 的 retry 是唯一會觸發 LLM 的前景動作

## Nielsen 5 易用性檢核（Windows 對應）

| Heuristic | 本設計如何滿足 |
|---|---|
| #1 系統狀態可見 | footnote 顯「{time} 由 {model} 產出」— user 知道這是何時、誰寫的 |
| #2 貼近真實世界 | 「昨天你：」「今天試一個小動作」白話陪伴語氣，非數據儀表板 |
| #6 認得勝過回想 | 回顧主動推到 Coach tab + toast；user 不用記得來看 |
| #8 美學最小化 | 一個 action（非清單）、無評分星、無紅橘 — 視覺極簡、無壓力 |
| #10 說明文件 | 醫療免責常駐「陪伴不是醫療建議」— 設定使用者預期 |

## 螢幕 A — 回顧卡（full）：tap targets

| 元件 | 點擊行為 | 鍵盤 |
|---|---|---|
| 「看歷史回顧」 | → HISTORY_LIST（過去 review 列表） | `Tab` + `Enter` |
| 「沒幫助」(thumbs-down) | 標記該 review `helpful=false`（餵 SPEC-25 skill 演化）→ 鈕變「已記錄」灰；**不彈評分、不問為什麼**（低摩擦） | `Tab` + `Enter` |
| tomorrow-action 文字 | **不可點**（純閱讀；不是 checkbox、無「完成」狀態 — 對齊「邀請非待辦」） | — |
| footnote | 不可點（純資訊） | — |

**動畫 / timing**：卡進場 fade-in 200ms；action 暖塊比其餘區晚 100ms 淡入（視覺引導視線到主角）；「沒幫助」點擊後 80ms 變灰，不需確認。

## 螢幕 B — CARD_COOKING：retry 互動

| 元件 | 行為 |
|---|---|
| 「再想一個建議」(refresh-cw) | 重呼 LLM（aggregator brief 已快取，不重算）→ 暖塊轉 spinner「想想看...」→ 回 CARD_FULL（lint pass）或留 COOKING（再 reject） |
| retry 計數 | 內部計 retry；第 3 次仍 reject → CARD_GIVEUP「今天先這樣，明天再聊」（retry 鈕消失） |

- retry **不報 error**（即使 LLM 4-provider 全 fail）— 語氣始終溫和（「還在想」非「失敗」）
- retry spinner 用 indeterminate（不顯假百分比），逾 60s（background LLM p95）→ 自動回 COOKING + 「等等再試」

## 螢幕 C — CARD_EMPTY / HISTORY：tap targets

- EMPTY 的「記一餐 / 開始專注 / 打卡習慣」→ 深連結 `spectyn-mesh://{food/focus/habit}`（跳對應 capture 面）— 註：`food/focus/habit` prefix 同 SPEC-20/22 待加入 SPEC-43 §12.1 deep-link 白名單（目前只放行 coach/cluster/settings）
- HISTORY 列點某日 → 展開該日 CARD_FULL（同版型）；ESC / 返回 → 回列表

## Narrator focus order（per SPEC-43 §12.2 + WCAG 2.2 AA 無障礙）

- **CARD_FULL**：日期 → 「昨天的記錄：{events}」→ 「觀察：{takeaways}」→ 「今天可以試：{action}」→ 歷史鈕 → 沒幫助鈕 → 免責 → **footnote（成本最後）**
- 用「今天**可以**試」非「你**應該**」（朗讀語氣 = shame-free）
- **CARD_COOKING**：「回顧好了，建議還在想」（`aria-live="polite"`）→ retry 鈕
- footnote 朗讀順序刻意最後 — 避免「這要花 $0.012」的成本焦慮先入耳

## 元件草圖（HTML，可貼進 Tauri webview 試三態）

```html
<style>
  .card{background:#1a1a2e;border-radius:10px;padding:16px;color:#e8e8f0;max-width:520px}
  .hdr{font-size:13px;color:#6b6b80;margin-top:10px}
  .ev{font-size:14px;color:#b8b8c8}
  .action{background:#221a2e;border-radius:8px;padding:12px;margin-top:10px}
  .action .h{color:#8ab4f8;font-size:13px}
  .action .t{font-size:15px;margin-top:4px}
  .foot{font-size:11px;color:#6b6b80;margin-top:12px;text-align:right}
  .disc{font-size:12px;color:#6b6b80;margin-top:8px}
</style>
<div class="card" role="article" aria-label="今天的回顧">
  <div style="font-size:16px">5/27 (二) 的回顧</div>
  <div class="hdr">昨天你：</div>
  <div class="ev">水 6 . 專注 45 分 . 冥想 1 . 午餐 ~540 kcal</div>
  <div class="hdr">觀察：</div>
  <div class="ev">下午 3 點後沒再喝水 . 專注都在早上</div>
  <div class="action">
    <div class="h">今天試一個小動作：</div>
    <div class="t">下午 3 點放一杯水在桌上</div>
  </div>
  <div class="disc">這是陪伴不是醫療建議</div>
  <div class="foot">21:03 由 Claude 產出 . $0.012</div>
</div>
<script>
  // 三態切換：full / cooking / empty -> 換 .action 區內容
  function setCooking(){ /* .action -> "建議還在想..." + retry 鈕 */ }
  // retry -> Tauri invoke('coach_retry_action', {date}) -> 回填 .action
</script>
```
> 註：`coach_retry_action` 為 Tauri command 佔位，實際 wire 見 SPEC-17 + SPEC-23 §9。卡片以 `role="article"` + `aria-label` 讓 Narrator 當整段朗讀。

## Walkthrough 腳本（usability test：「看昨天的回顧」）

1. 早上開 app → Coach tab → 預期：直接看到昨天回顧（不需點「產生」）
2. 讀 events → observations → action → 預期：感覺被理解、不被批判
3. 看 action「下午放杯水」→ 預期：覺得「這我做得到」（最小、具體）
4.（若 cooking）點「再想一個」→ 預期：溫和等待、不覺得是 error
5. 點「沒幫助」→ 預期：一鍵記錄、不被追問「為什麼」

**通過判準**：受測者讀完無被批判感（shame-free 主驗證點）；action 覺得可行；cooking 態不被誤認系統壞掉。

## 開放問題（留實作 / 後續）

1. retry spinner 文案「想想看...」措辭 — 待文案 review
2. 「沒幫助」是否需要 undo（誤點）— 待決
3. 歷史回顧趨勢圖（週/月）— v0.7+

## Pipeline 完成

SPEC-23 coach-engine Windows 三階段（wireframe → mockup → prototype）齊備。完成記錄見 `.ai-shared/done/design-coach-engine-windows.md`。
