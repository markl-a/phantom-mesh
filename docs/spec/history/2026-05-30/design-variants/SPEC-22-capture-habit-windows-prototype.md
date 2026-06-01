# SPEC-22 Capture Habit — Windows Prototype（原型）

> **Stage 3/3** · [線框稿（wireframe）](./SPEC-22-capture-habit-windows-wireframe.md) → [視覺稿（mockup）](./SPEC-22-capture-habit-windows-mockup.md) → 原型（prototype，互動腳本 + 元件草圖）
> **Status**: draft v0.1 · **Last updated**: 2026-05-28
> **Scope**: Windows only — 互動腳本（每個可點處按下去發生什麼）+ 鍵盤路徑 + 動畫 timing + 失敗路徑 + Narrator（朗讀器）focus 順序 + Nielsen 5（尼爾森 5 大易用性 heuristic）+ usability walkthrough（易用性走查）腳本 + 可貼進 Tauri webview 的 HTML 草圖。
> **Spec**: [`SPEC-22-SYSTEM-capture-habit`](../specs/v060-deep-spec/SPEC-22-SYSTEM-capture-habit.md) · [`SPEC-42`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md)

## 設計溯源（trace）

| 維度 | 對應 |
|---|---|
| **BIG-GOAL pillar** | **P2 多模態理解**（text + behavior）；cross-cut P3 進化網、P4 加密為先 |
| **Source spec** | SPEC-22-SYSTEM-capture-habit |
| **Platform** | windows（桌面） |
| **Pipeline stage** | 3/3 prototype |

## 為什麼 habit capture 要獨立 Windows prototype

food（SPEC-20）prototype 是「單張照片 → 分析 → 結果」單程；focus（SPEC-21）是「長時錄音 session」。habit 互動模型第三種：**極高頻、極低摩擦、連續多筆**。
1. **連點多 chip 不關面** — 一次記水+咖啡+冥想，dropdown 不能點一下就收
2. **數字鍵 + 滑鼠混用** — 鍵盤 user 按 `1`–`9`、滑鼠 user 點格子
3. **qty stepper 內嵌** — 需數量的 chip 不跳新窗，inline 展開
4. **streak 即時回饋** — 打卡後 Habit tab heatmap 當下更新（< 10ms sqlite query）

## 互動 FSM（finite state machine，有限狀態機）

```
TRAY_CLOSED --(右鍵 tray)--> GRID_OPEN --(點 chip / 按數字)--> {
   無數量 chip: --> LOGGED(閃 check 200ms) --> 回 GRID_OPEN(不關)
   需數量 chip: --> QTY_INLINE --(調數量 + OK/Enter)--> LOGGED --> 回 GRID_OPEN
   free-text:   --(打字 + Enter)--> [match chip? -> BIND_PROMPT : LOGGED_FREETEXT] --> 回 GRID_OPEN
}
GRID_OPEN --(Esc / 點面外)--> TRAY_CLOSED
```
- **LOGGED 不關面**是核心互動決策（連續打卡）；只有 Esc / 點 dropdown 外才收
- Habit tab heatmap 訂閱 event 寫入 → 任一 LOGGED 即重算當列 streak + 7d grid

## Nielsen 5 易用性檢核（Windows 對應）

| Heuristic | 本設計如何滿足 |
|---|---|
| #1 系統狀態可見 | log 後 chip 閃 check + Habit tab heatmap 即時長一格；streak 數字當下更新 |
| #2 貼近真實世界 | chip = 白話習慣名（水/咖啡/運動）；streak「連續 N 天」直觀 |
| #3 掌控 + 自由 | dropdown 不自動關（自己決定記幾筆）；誤點可在 Habit tab 刪該 event |
| #5 錯誤預防 | 需數量 chip 強制走 qty stepper（不會記「水 1」這種無意義量）；free-text match chip 時提示綁定 |
| #6 認得勝過回想 | chip palette 視覺常駐 + 數字角標；不用記指令 |

## 螢幕 A — Tray chip grid：tap targets（按下去發生什麼）

| 元件 | 點擊行為 | 鍵盤 |
|---|---|---|
| 無數量 chip（運動 / 冥想） | 寫 1 筆 `EventKind::Habit` → 閃 `check` 200ms → 留在 grid | 數字 `1`–`9` 快選 |
| 需數量 chip（水 250ml） | inline 展開 qty stepper → 調量 → Enter/OK 寫入 → 收回 stepper | 數字選中後 `Enter` 進 stepper |
| free-text 框 | 打字 + Enter → 寫 free_text event；開頭 match chip label → BIND_PROMPT | `Tab` 到框、`Enter` 送出 |
| dropdown 外 / Esc | 關閉（TRAY_CLOSED） | `Esc` |

> **鍵盤焦點規則**：`1`–`9` 數字鍵快選 chip **只在 grid 有焦點時生效**；free-text 框聚焦時，數字鍵走輸入（不觸發 chip 快選）— `keydown` handler 先檢查 `document.activeElement` 是否為輸入框。chip 10–12 無數字鍵（僅 `Tab` + 滑鼠；數字鍵只到 9）。

**動畫 / timing**：chip hover 100ms border；log confirm `check` icon fade-in 80ms + hold 120ms + fade-out 100ms（共 300ms，不擋下一次點）；qty stepper 展開 slide 120ms。

## 螢幕 D — qty stepper：互動

- `[- 250 +] ml [OK]`：`-`/`+` 點一下 ±step（預設 50）；**長按 500ms 後連續 ±**（加速：前 1s 每 200ms、之後每 80ms）
- 數字框可直接打字（數字驗證：非數字 reject、空 = 取消）
- `Enter` = OK 寫入；`Esc` = 取消 stepper 回 grid（不寫）
- 單位 `ml` 唯讀（chip 定義時鎖）

## 螢幕 B — Habit tab heatmap：互動 + 失敗路徑

- heatmap 格 **hover** → tooltip「{日期}：{n} 筆」；**點格** → 展開那天的 event 清單（可刪單筆）
- streak / 30d count 隨 event 寫入即時重算（sqlite query，無 spinner）
- **失敗路徑**：
  - `chip_palette` 表讀失敗 → grid 退化為 hardcode 12 預設 chip + free-text（per wireframe）
  - event 寫入失敗（磁碟滿）→ chip 閃**紅**（非綠 check）+ toast「記錄失敗」；不靜默
  - 連點同 chip 多下 → 各記一筆（lenient，不 debounce）

## Narrator focus order（per SPEC-43 §12.2 + WCAG 2.2 AA 無障礙）

- **A grid**：header → chip 1（含 streak 朗讀）→ ... → chip 12 → free-text 框 → 「Open Habit tab」
- log 後：`aria-live="polite"` 朗讀「已記 {chip}」（不搶焦點，連點不吵）
- **qty stepper**：朗讀「{chip} 數量 {n} {unit}，減鈕 / 加鈕 / 確認」
- **B heatmap**：每列「{chip}，連續 {streak} 天，30 天 {count} 筆」；格不逐一朗讀（太吵），整列 summary 即可

## 元件草圖（HTML，可貼進 Tauri webview 試互動）

```html
<style>
  .chip{background:#1a1a2e;border:1px solid transparent;border-radius:6px;
        padding:8px 10px;color:#e8e8f0;cursor:pointer;position:relative;
        transition:border-color .1s}
  .chip:hover{border-color:#8ab4f8}
  .chip .num{position:absolute;top:2px;left:4px;font-size:10px;color:#6b6b80}
  .chip.logged::after{content:"check";color:#81c995;font-size:11px;margin-left:6px}
  .hm{display:inline-flex;gap:2px}
  .hm i{width:10px;height:10px;border-radius:2px;background:#1a1a2e}
  .hm i.d1{background:rgba(129,201,149,.35)}.hm i.d2{background:rgba(129,201,149,.65)}
  .hm i.d3{background:#81c995}
</style>
<button class="chip" data-k="1" onclick="logChip('water')"><span class="num">1</span>水</button>
<button class="chip" data-k="2" onclick="logChip('coffee')"><span class="num">2</span>咖啡</button>
<!-- 7-day heatmap 範例：滿/淺/空 -->
<span class="hm"><i class="d3"></i><i class="d1"></i><i></i><i class="d2"></i>
<i class="d3"></i><i></i><i class="d1"></i></span>
<script>
  function logChip(id){ /* -> Tauri invoke('habit_log', {chip_id:id}) */
    /* 寫成功 -> 該 chip 加 .logged class 300ms 後移除；dropdown 不關 */ }
  document.addEventListener('keydown', e=>{
    const c=document.querySelector(`.chip[data-k="${e.key}"]`); if(c) c.click();
  });
</script>
```
> 註：`check` 字面僅草圖示意；正式版用 bundled Lucide `check` SVG（mockup §Lucide）。`habit_log` 為 Tauri command 佔位，實際 wire 見 SPEC-17 + SPEC-22 §9。

## Walkthrough 腳本（usability test：「記今天喝了 3 次水、運動 1 次」）

1. 右鍵 tray icon → 預期：看到 chip grid，認得「水」「運動」
2. 點「水」3 下（或按 `1` 三下）→ 預期：每次閃 check、面不關、能連點
3.（若水需數量）每次調 250ml → 預期：qty stepper inline、Enter 即記
4. 點「運動」1 下 → 預期：直接記、不問數量
5. 開 Habit tab → 預期：水 streak +1、今日 summary 顯「水 x3 · 運動 x1」

**通過判準**：受測者無提示完成連續打卡；不會因「面不關」困惑（反而覺得方便）；斷掉的 streak 不讓人有罪惡感（#shame-free 驗證點）。

## 開放問題（留實作 / 後續）

1. qty stepper 長按加速曲線手感 — 待實機調
2. >12 chip 時 tray grid 滾動 vs 分頁 — 待 palette 上限決策
3. 里程碑 toast streak 門檻可否 user 自訂 — 待 Settings 設計

## Pipeline 完成

SPEC-22 capture-habit Windows 三階段（wireframe → mockup → prototype）齊備。完成記錄見 `.ai-shared/done/design-capture-habit-windows.md`。
