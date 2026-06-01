# SPEC-20 Capture Food — Windows Prototype（原型）

> **Stage 3/3** · [線框稿（wireframe）](./SPEC-20-capture-food-windows-wireframe.md) → [視覺稿（mockup）](./SPEC-20-capture-food-windows-mockup.md) → 原型（prototype，互動腳本 + 元件草圖）
> **Status**: draft v0.1 · **Last updated**: 2026-05-28
> **Scope**: Windows only — 互動腳本（每個可點處按下去發生什麼）+ 鍵盤路徑 + 動畫 timing + 失敗路徑 + Narrator（朗讀器）focus 順序 + Nielsen 5（尼爾森 5 大易用性 heuristic，啟發式檢核）+ usability（易用性）walkthrough 腳本 + 一段可貼進 Tauri webview 的 HTML 草圖。
> **Spec**: [`SPEC-20-SYSTEM-capture-food`](../specs/v060-deep-spec/SPEC-20-SYSTEM-capture-food.md) · [`SPEC-42`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md)

## 設計溯源（trace）

| 維度 | 對應 |
|---|---|
| **BIG-GOAL pillar** | **P2 多模態理解** `P2.food`；cross-cut P4 加密為先、P3 進化網 |
| **Source spec** | SPEC-20-SYSTEM-capture-food |
| **Platform** | windows（桌面） |
| **Pipeline stage** | 3/3 prototype |

## 為什麼 food capture 要獨立 Windows prototype

SPEC-21（focus）prototype 鎖的是「錄音 long-running session（長時運行工作階段）+ tray menu rebuild + Focus Assist 穿透」。food capture 互動模型完全不同：
1. **瞬時 pipeline，非 session** — 沒有 pause/resume/stop 狀態鏈；按一下來源 → 自動跑完
2. **多 capture 來源切換**（webcam / file / paste / drag-drop）— focus 只有「開始錄」單一動作
3. **樂觀 UI（optimistic UI）+ 8 秒 analyze budget（分析時間預算）** — event 先落地、LLM 後補，互動上要「可隨時關窗不丟資料」
4. **confidence（信心度）+ inline 修正** — focus 沒有「AI 結果可能錯、user 修正」這條路徑

## 互動 FSM（finite state machine，有限狀態機）

```
SOURCE_PICK --(選來源/貼上/拖放)--> CAPTURING --(拿到 bytes)--> ANALYZING --(LLM 回 / 超時)--> RESULT
     ^                                  |                          |                            |
     |                                  v (webcam 無裝置)          v (4 provider 全 fail)      v (修正清單)
     +--(< 換來源)------------------ A' WEBCAM                 RESULT(analysis_failed)      CORRECTING
```
- 任何 state 按 `Esc` / 關窗 → 若已過 ANALYZING（event 已落地）則「完成並關閉」；若還在 SOURCE_PICK 則「取消」
- RESULT 與 RESULT(failed) 共用版型，差別在 item 區內容（見 mockup 螢幕 C / C'）

## Nielsen 5 易用性檢核（Windows 對應）

| Heuristic | 本設計如何滿足 |
|---|---|
| #1 系統狀態可見 | ANALYZING 有 indeterminate 進度 + 「已加密落地」安心文案；tray icon 綠色（working）ambient 提示 |
| #2 貼近真實世界 | 「記一餐」「拍一張」白話；kcal 直接給數字非術語 |
| #3 使用者掌控 + 自由 | Esc 隨時退；event 已落地不怕誤關；「重分析」「修正清單」可逆 |
| #5 錯誤預防 | 低信心 item 標「請手動補」而非塞假數字；無 webcam 磚灰階不可點 |
| #9 錯誤復原 | analysis_failed 給「手動輸入 / 重分析」；camera disabled 給「打開設定 / 重試」 |

## 螢幕 A — Capture Source Picker：tap targets（按下去發生什麼）

| 元件 | 點擊行為 | 鍵盤 |
|---|---|---|
| 「拍一張」磚 | → A' WEBCAM（開 MediaCapture live preview）；無裝置時 disabled | `Tab` 聚焦 + `Enter` |
| 「選檔案」磚 | 開 OS file picker（filter: jpg/png/heic）→ 選檔 → CAPTURING | `Tab` + `Enter` |
| 「貼上」磚 / `Ctrl+V` | 讀剪貼簿 image → 有圖 → CAPTURING；無圖 → toast「剪貼簿沒有圖片」 | `Ctrl+V` 全域（視窗有焦點時） |
| 拖放任一處 | drop image file → CAPTURING | n/a |
| title bar `X` | 取消、關窗（SOURCE_PICK 未落地，無資料損失） | `Esc` |

**動畫 / timing**：磚 hover transition 120ms ease-out（border + bg）；drop 時拖放區 highlight 80ms；選定 → 淡出 picker 150ms → 淡入 ANALYZING。

## 螢幕 B — Analyzing：互動 + 失敗路徑

- **不可互動**（除了關窗）— spinner + indeterminate 進度掃動（1.2s 週期 CSS 動畫）
- **8 秒 budget 超時**：切 optimistic → 直接進 RESULT 標「分析較久，稍後更新」，背景 LLM 回來後 patch item 區
- **失敗路徑**（`FOOD_IMAGE_DECODE` / `FOOD_PERSIST_FAIL` 為**建議新增至 SPEC-20 §11.1**，目前 spec 僅 `FOOD_ANALYSIS_FAILED` / `FOOD_BLOB_TOO_LARGE` / `FOOD_DECRYPT_FAILED`）：
  - 壓縮失敗（圖損毀）→ `FOOD_IMAGE_DECODE`（proposed）→ 退回 SOURCE_PICK + toast
  - 加密落地失敗（磁碟滿）→ `FOOD_PERSIST_FAIL`（proposed）→ 紅卡 + 「重試」（不進 analyze）
  - blob 過大 → `FOOD_BLOB_TOO_LARGE`（spec 既有）→ 提示重拍 / 換圖
  - 4 provider 全 fail → `FOOD_ANALYSIS_FAILED` → RESULT(analysis_failed)（event 仍在）

## 螢幕 C — Result：tap targets

| 元件 | 行為 |
|---|---|
| 「修正清單」 | item 區切 inline 編輯（name 文字框 + kcal 數字框）→ 「儲存」append correction event（P4 append-only，不改原 row） |
| 「重分析」 | 重跑 fallback chain（image 已加密落地，不重拍）→ 回 ANALYZING |
| 「完成」（主鈕） | 關窗、tray 回 idle、main window Food tab 列頭插這筆、發 ActionCenter toast |
| confidence 點 | 純展示，不可點；hover tooltip 顯示原始 confidence 數值 |

**動畫**：item row 進場 stagger 60ms/row；confidence 點由左到右點亮 40ms/點。

## Narrator focus order（per SPEC-43 §12.2 + WCAG 2.2 AA 無障礙）

- **A**：視窗標題 → 來源說明 → 磚1 → 磚2 → 磚3 → 拖放提示 → trust badge
- **B**：「分析中」status（`aria-live="polite"` 朗讀一次）
- **C**：完成標題 → 各 item（name + kcal + 信心度朗讀「高/中/低信心」）→ 合計 → 修正清單 → 重分析 → 完成
- 低信心 item 額外朗讀「看不清楚，請手動補」

## 元件草圖（HTML，可貼進 Tauri webview 試互動）

```html
<!-- source picker tile + confidence dots: 最小可跑互動骨架 -->
<style>
  .tile{background:#1a1a2e;border:1px solid transparent;border-radius:8px;
        width:120px;height:96px;display:inline-grid;place-items:center;
        color:#e8e8f0;cursor:pointer;transition:border-color .12s,background .12s}
  .tile:hover{border-color:#8ab4f8;background:#22223a}
  .tile[disabled]{opacity:.4;cursor:not-allowed}
  .dot{width:8px;height:8px;border-radius:50%;display:inline-block;margin:0 1px}
  .dot.on-hi{background:#81c995}.dot.on-mid{background:#8ab4f8}
  .dot.on-lo{background:#ff9800}.dot.off{background:#6b6b80}
</style>
<button class="tile" onclick="pick('webcam')">📷 拍一張</button>
<button class="tile" onclick="pick('file')">📁 選檔案</button>
<button class="tile" onclick="pick('paste')">📋 貼上</button>
<!-- confidence: 高信心範例 ●●●○ -->
<span class="dot on-hi"></span><span class="dot on-hi"></span>
<span class="dot on-hi"></span><span class="dot off"></span>
<script>
  function pick(src){ /* -> Tauri invoke('food_capture_source', {src}) */ }
  document.addEventListener('paste', e => {
    if ([...e.clipboardData.files].some(f => f.type.startsWith('image/'))) pick('paste');
  });
</script>
```
> 註：emoji 僅原型草圖示意；正式版用 bundled Lucide SVG（mockup §Lucide icon 對映）。`food_capture_source` 為 Tauri command 佔位名，實際 wire 見 SPEC-17 Tauri bridge + SPEC-20 §9 API。

## Walkthrough 腳本（usability test：「記錄你剛吃的午餐」）

1. 開「記一餐」視窗 → 預期：看到 3 來源磚，知道可拍 / 選 / 貼
2. 拖一張餐點照片進視窗 → 預期：立刻進「分析中」，看到縮圖 + 「已加密落地」
3. 等 ~8 秒 → 預期：看到食物清單 + 熱量 + 信心點，合計顯著（若 analyze 逾 8 秒 → 先見「分析較久，稍後更新」optimistic 狀態，清單之後 patch 進來 — 與 mockup §螢幕 B 8 秒 budget 一致）
4. 某項信心低標橘字（非紅 — 低信心不是 error，per mockup confidence 配色）→ 預期：user 知道要點「修正清單」補
5. 按「完成」→ 預期：視窗關、之後收到一則 toast「已記錄一餐」

**通過判準**：受測者無需提示完成 1-5；低信心 item 不被誤認為「系統壞了」（#5 錯誤預防驗證點）。

## 開放問題（留實作 / 後續）

1. webcam live preview frame rate / 解析度上限 — 待實機量測
2. 多盤合併（一張多食物）item 上限 + 滾動 — SPEC-20 §multi-item 待對齊
3. correction event undo 視窗長度 — 待 coach engine（SPEC-23）對齊 event append 語意

## Pipeline 完成

SPEC-20 capture-food Windows 三階段（wireframe → mockup → prototype）齊備。完成記錄見 `.ai-shared/done/design-capture-food-windows.md`。
