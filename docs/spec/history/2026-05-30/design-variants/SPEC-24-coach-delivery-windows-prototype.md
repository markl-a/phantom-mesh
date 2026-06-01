# SPEC-24 Coach Delivery — Windows Prototype（原型）

> **Stage 3/3** · [線框稿（wireframe）](./SPEC-24-coach-delivery-windows-wireframe.md) → [視覺稿（mockup）](./SPEC-24-coach-delivery-windows-mockup.md) → 原型（prototype，互動腳本 + 元件草圖）
> **Status**: draft v0.1 · **Last updated**: 2026-05-28
> **Scope**: Windows only — 互動腳本（每個可點處按下去發生什麼）+ deep-link（深連結）冷啟動序列 + 「測試」互動 + receipt（回條）列表 + Narrator（朗讀器）focus 順序 + Nielsen 5（尼爾森 5 大易用性 heuristic）+ usability walkthrough（易用性走查）+ HTML 草圖。
> **Spec**: [`SPEC-24-SYSTEM-coach-delivery`](../specs/v060-deep-spec/SPEC-24-SYSTEM-coach-delivery.md) · [`SPEC-42`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md)

## 設計溯源（trace）

| 維度 | 對應 |
|---|---|
| **BIG-GOAL pillar** | **X.coach**（delivery）；cross-cut P4 加密為先、P1 跨裝置 Mesh；操作原則 Reversible |
| **Source spec** | SPEC-24-SYSTEM-coach-delivery |
| **Platform** | windows（桌面） |
| **Pipeline stage** | 3/3 prototype |

## 為什麼 coach delivery 要獨立 Windows prototype

delivery 的互動有三條 coach-engine（SPEC-23）沒有的路徑：
1. **deep-link 冷啟動** — app 沒開時點 toast → launch → 跳 Coach tab，序列要設計（splash、serve ready 等待）
2. **「測試」往返** — Settings 填憑證 → 點測試 → 真發一則 → inline 回結果
3. **per-channel toggle 即時生效** — 開/關通道立刻改 router（不需重啟）

## 互動腳本：deep-link 冷啟動序列

```
[user 點 WinRT Toast / 「開啟回顧」]
  --> OS 解析 phantom-mesh:// protocol --> 喚起 phantom app
      --> app 已在跑：直接 navigate Coach tab + load review(id)（< 300ms）
      --> app 沒跑：
            1. launch phantom-mesh-app.exe（顯 splash，不白屏）
            2. 等 phantom serve ready（healthz 200）
            3. navigate Coach tab + load review(id)
            4. 若 serve 5s 未 ready --> splash 顯「啟動中...」+ 不報錯
```
- deep-link `id` 無效 / review 已刪 → Coach tab 顯「找不到這則回顧」（不崩、不白屏）
- single-instance（SPEC-43）：app 已跑時第二次 launch 不開新窗，傳 deep-link 給既有 instance

## 螢幕 B — Settings delivery：tap targets（按下去發生什麼）

| 元件 | 點擊行為 | 鍵盤 |
|---|---|---|
| 通道 toggle | 開 → router 立即訂閱該通道（下次 review 就發）；關 → 立即退訂；**不需重啟** | `Space` 切換 |
| 「測試」鈕 | 立刻發一則測試 payload 到該通道 → spinner → inline「已送達」/「失敗：{reason}」 | `Enter` |
| token / 密碼欄 | 輸入 → blur 時 vault seal 存 → 顯 `****`；再聚焦不回顯真值（要改先清空重打） | `Tab` 進欄 |
| receipt「重試」 | 重發該 review 到該通道（繞過 dedup，user 明示重試） | `Enter` |
| receipt「設定」 | 跳回該通道憑證欄聚焦 | `Enter` |

**動畫 / timing**：toggle 切換 120ms slide；測試 spinner indeterminate；結果 fade-in 80ms（成功綠 / 失敗紅），停 5s 後淡為小字常駐。

## Nielsen 5 易用性檢核（Windows 對應）

| Heuristic | 本設計如何滿足 |
|---|---|
| #1 系統狀態可見 | receipt 列顯每路 sent/failed/suppressed + 時間；「測試」即時回結果 |
| #3 掌控 + 自由（Reversible） | 每通道一鍵 off、立即生效；「重試」user 主動繞 dedup |
| #5 錯誤預防 | 「測試」鈕讓 user 填完先驗證（不用等到 21:00 才發現 SMTP 錯） |
| #9 錯誤復原 | failed receipt 給「重試」+「設定」；3 路全失敗 fallback banner |
| #10 說明 + 安全 | vault seal 說明常駐；token 遮罩防肩窺 |

## 失敗路徑（per SPEC-04 + SPEC-24 §11 COACH_DELIV_*）

- 「測試」SMTP 認證錯 → `COACH_DELIV_SMTP_AUTH` → inline「失敗：認證錯誤」（**不顯 host 細節**，避洩 per SPEC-08 threat model）
- Telegram bot token 無效 → 「失敗：bot token 不對」
- 3 通道全失敗（真實 21:00 派送）→ review 仍寫 file → 下次開 app Coach tab 頂 banner「上次回顧已備好」（info 非 danger）
- deep-link app 啟動逾 30s → splash 顯「啟動較久，請稍候」（不 crash）

## Narrator focus order（per SPEC-43 §12.2 + WCAG 2.2 AA 無障礙）

- **Settings B**：分頁標題 → 通道 1 toggle（朗讀「桌面通知，開啟」）→ 說明 → 通道 2 toggle → token 欄（「已加密封存」**不讀 ****值**）→ 測試鈕 → ... → vault note
- 「測試」結果用 `aria-live="assertive"`（user 主動觸發、需即時知道成敗）
- receipt 列：每列「{date} {channel} {status}」；failed 額外朗讀「可重試」

## 元件草圖（HTML，可貼進 Tauri webview 試互動）

```html
<style>
  .row{display:flex;align-items:center;gap:10px;padding:8px 0;color:#e8e8f0}
  .tog{width:40px;height:22px;border-radius:11px;background:#1a1a2e;position:relative;
       cursor:pointer;transition:background .12s}
  .tog.on{background:#8ab4f8}
  .tog i{position:absolute;top:2px;left:2px;width:18px;height:18px;border-radius:50%;
         background:#e8e8f0;transition:left .12s}
  .tog.on i{left:20px}
  .tok{background:#1a1a2e;border:1px solid #2a2a40;border-radius:6px;padding:6px 8px;
       color:#6b6b80;font-family:monospace}
  .test{background:#8ab4f8;border:none;border-radius:6px;padding:6px 12px;cursor:pointer}
  .ok{color:#81c995}.fail{color:#f28b82}.sup{color:#6b6b80}
</style>
<div class="row"><div class="tog on" onclick="toggle('os',this)"><i></i></div>桌面通知</div>
<div class="row"><div class="tog" onclick="toggle('telegram',this)"><i></i></div>Telegram
  <span class="tok">****************</span>
  <button class="test" onclick="testCh('telegram')">測試</button></div>
<!-- receipts -->
<div class="row"><span class="ok">check 已送達 21:03</span></div>
<div class="row"><span class="fail">x 失敗：認證錯誤</span> <button class="test">重試</button></div>
<div class="row"><span class="sup">已送達（重複已略過）</span></div>
<script>
  function toggle(ch,el){ el.classList.toggle('on');
    /* -> Tauri invoke('coach_delivery_set', {channel:ch, enabled:el.classList.contains('on')}) */ }
  function testCh(ch){ /* -> invoke('coach_delivery_test', {channel:ch}) -> inline ok/fail */ }
</script>
```
> 註：`coach_delivery_set` / `coach_delivery_test` 為 Tauri command 佔位，實際 wire 見 SPEC-17 + SPEC-24 §9。token span 永遠顯固定長度 ****（不綁真長度）。

## Walkthrough 腳本（usability test：「設定 Telegram 接收回顧」）

1. Settings → Coach → Delivery → 預期：看到 3 通道、桌面通知預設 ON
2. 開 Telegram toggle → 填 bot token + chat id → 預期：欄變 ****、看到 lock icon
3. 點「測試」→ 預期：幾秒內「已送達」（手機 Telegram 真收到一則）
4. 隔天 21:00（或 CLI 觸發）→ 預期：Telegram 收到完整 review markdown
5. 故意填錯 token 再測試 → 預期：「失敗：bot token 不對」、不洩細節、給修的入口

**通過判準**：受測者能在不等 21:00 的情況下驗證通道（#5）；token 遮罩讓人安心（肩窺）；關掉通道立即生效（Reversible 驗證點）。

## 開放問題（留實作 / 後續）

1. token 欄「顯示明文」眼睛 icon（方便 vs 肩窺）— 待決
2. 「測試」失敗 reason 顯多細（避洩 SMTP host）— 待 SPEC-08 對齊
3. receipt 保留筆數 / 天數 — 待 SPEC-16 retention

## Pipeline 完成

SPEC-24 coach-delivery Windows 三階段（wireframe → mockup → prototype）齊備。完成記錄見 `.ai-shared/done/design-coach-delivery-windows.md`。
