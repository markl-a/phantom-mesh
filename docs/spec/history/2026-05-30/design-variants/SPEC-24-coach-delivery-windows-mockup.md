# SPEC-24 Coach Delivery — Windows Mockup（視覺稿）

> **Stage 2/3** · [線框稿（wireframe）](./SPEC-24-coach-delivery-windows-wireframe.md) → 視覺稿（mockup，配色 + toast XML + 文案 + a11y）→ [原型（prototype，待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-28
> **Scope**: Windows only — WinRT Toast（通知中心快顯）XML 終值 + Settings（設定）delivery toggle / 憑證欄視覺 + vault seal（保險庫封存）badge + receipt（回條）狀態色 + 終版文案 + Narrator（朗讀器）AutomationName。沿用 SPEC-20/21/22/23 mockup 的 design token（設計變數）速查。
> **Spec**: [`SPEC-24-SYSTEM-coach-delivery`](../specs/v060-deep-spec/SPEC-24-SYSTEM-coach-delivery.md) · [`SPEC-42`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md) · [`SPEC-02-FOUNDATION-design-tokens`](../specs/v060-deep-spec/SPEC-02-FOUNDATION-design-tokens.md)

## 設計溯源（trace）

| 維度 | 對應 |
|---|---|
| **BIG-GOAL pillar** | **X.coach**（delivery）；cross-cut P4 加密為先、P1 跨裝置 Mesh；操作原則 Reversible |
| **Source spec** | SPEC-24-SYSTEM-coach-delivery |
| **Platform** | windows（桌面） |
| **Pipeline stage** | 2/3 mockup |

## 為什麼 coach delivery 要 Windows mockup

wireframe 鎖了通道結構；本檔鎖實作視覺：
1. **WinRT Toast XML 終值** — title / action / launch / scenario 屬性確值
2. **Settings toggle + 憑證欄視覺** — token 欄遮罩 / 「測試」鈕狀態 / vault badge
3. **receipt 狀態色** — sent / failed / suppressed 用什麼色（克制：只 failed 才 danger）
4. **終版文案** + **Narrator AutomationName**

## Design token 對映（per SPEC-02，沿用前作）

| Token | Hex | delivery 用途 |
|---|---|---|
| `phantom-bg` | `#0f0f1a` | Settings delivery 分頁背景 |
| `phantom-card` | `#1a1a2e` | 通道區塊 / 憑證欄 bg |
| `phantom-primary` | `#8ab4f8` | toggle ON 軌道、「測試」鈕 |
| `phantom-success` | `#81c995` | receipt 「已送達」、測試成功 |
| `phantom-danger` | `#f28b82` | receipt 「失敗」（**唯一**用 danger 處） |
| `phantom-muted` | `#6b6b80` | suppressed「重複已略過」、token **** 遮罩、vault badge |

## 文案 keys（per SPEC-05 i18n）

| key | 繁中 | English |
|---|---|---|
| `coach.deliv.toast_title` | 今天的回顧好了 | Your review is ready |
| `coach.deliv.toast_action` | 開啟回顧 | Open review |
| `coach.deliv.ch_os` | 桌面通知（ActionCenter） | Desktop notification |
| `coach.deliv.ch_telegram` | Telegram | Telegram |
| `coach.deliv.ch_email` | Email | Email |
| `coach.deliv.test` | 測試 | Test |
| `coach.deliv.test_ok` | 已送達 | Delivered |
| `coach.deliv.test_fail` | 失敗：{reason} | Failed: {reason} |
| `coach.deliv.vault_note` | 金鑰 / 密碼都本地加密封存，broker 看不到 | Keys/passwords are sealed locally; the broker never sees them |
| `coach.deliv.receipt_sent` | 已送達 {time} | Delivered {time} |
| `coach.deliv.receipt_suppressed` | 已送達（重複已略過） | Delivered (duplicate suppressed) |
| `coach.deliv.fallback_banner` | 上次回顧已備好 | Your last review is ready |

## 螢幕 A — WinRT Toast（Markdown channel）XML 終值

```xml
<toast scenario="reminder" activationType="protocol"
       launch="phantom-mesh://coach/review?id={review_id}">
  <visual>
    <binding template="ToastGeneric">
      <text>今天的回顧好了</text>
      <text>{one_line_action} . 點開看看</text>
      <image placement="appLogoOverride" hint-crop="circle"
             src="phantom-tray-idle.png"/>
    </binding>
  </visual>
  <actions>
    <action content="開啟回顧" activationType="protocol"
            arguments="phantom-mesh://coach/review?id={review_id}"/>
  </actions>
</toast>
```
- `scenario="reminder"` → persist 到 dismiss（不自動消）
- **無 `<audio>`**（21:00 溫和；shame-free + P4）
- AppLogo `hint-crop="circle"` 圓形 phantom logo
- launch + action 同 deep-link（點 body 或鈕都進 Coach tab）

## 螢幕 B — Settings delivery（toggle + 憑證欄視覺）

```
+--------------------------------------------------+
| Settings . Coach . Delivery                      |
+--------------------------------------------------+
|  桌面通知（ActionCenter）            [===O]  ON  |   toggle ON：軌道 phantom-primary
|    回顧好了在右下角提醒你                         |   說明 13px phantom-muted
|  ................................................|
|  Telegram                            [O===] OFF  |   toggle OFF：軌道 phantom-card
|    bot token  [****************] (lock)  [測試]   |   token 欄 **** + lock icon + 測試鈕
|    chat id    [____________]                     |
|  ................................................|
|  Email                               [O===] OFF  |
|    SMTP host [____________] port [___]           |
|    帳號 [_________] 密碼 [********] (lock) [測試] |
|    收件 [_________]                              |
|                                                  |
|  (lock) 金鑰 / 密碼都本地加密封存，broker 看不到 |   vault badge 12px phantom-muted
+--------------------------------------------------+
```

**設計重點**：
- **toggle**：ON 軌道 phantom-primary + 圓鈕右；OFF 軌道 phantom-card + 圓鈕左（Fluent ToggleSwitch）
- **token / 密碼欄**：輸入後存 vault → 顯 `****`（固定長度遮罩、不回顯真值、不洩長度）；旁 `lock` icon 表已封存
- **「測試」鈕**：點 → spinner →「已送達」phantom-success / 「失敗：{reason}」phantom-danger inline
- OFF 通道的憑證欄灰階 disabled（toggle 開才可填）
- vault badge 常駐底部（P4 信任溝通）

**Narrator AutomationName**：
- toggle：「桌面通知，開啟 / 關閉。切換開關。」
- token 欄：「Telegram bot token，已加密封存。編輯框。」（不朗讀 **** 內容）
- 測試鈕：「測試此通道。按鈕。」

## 螢幕 C — Delivery receipt 狀態色

```
|  最近派送                                        |
|  5/27 桌面通知  (check) 已送達 21:03             |   phantom-success check
|  5/27 Telegram  (check) 已送達 21:03             |
|  5/27 Email     (x) 失敗：SMTP 認證錯誤 [重試]   |   phantom-danger x（唯一 danger）
|  5/26 桌面通知  已送達（重複已略過）             |   phantom-muted（suppressed 不搶眼）
```
- **sent** = phantom-success `check`；**failed** = phantom-danger `x` + 「重試」+「設定」；**suppressed** = phantom-muted 灰字（dedup 正常、不是錯）
- 視覺克制：只 failed 用紅；sent 綠小 icon；suppressed 灰（不讓「重複略過」看起來像問題）

## Lucide icon 對映

| 角色 | Lucide icon | 用途 |
|---|---|---|
| 已封存 | `lock` | token/密碼欄 + vault badge，14px phantom-muted |
| 測試成功 / sent | `check` | receipt + 測試結果，14px phantom-success |
| 失敗 | `x` | receipt failed，14px phantom-danger |
| 重試 | `refresh-cw` | failed receipt 行動，14px |
| 通道 icon | `bell`(OS) / `send`(TG) / `mail`(Email) | Settings 通道標頭，16px |

## ActionCenter fallback banner（3 通道全失敗）

下次開 app → Coach tab 頂插一條 banner：
```
| (info) 上次回顧已備好 . 點開看                   |   phantom-card bg, info icon, 不染紅
```
- 3 路全失敗不是 user 的錯 → 用 info（非 danger）語氣；review 仍在 file，不丟

## Cross-platform invariants 對齊

- receipt 狀態色（sent 綠 / failed 紅 / suppressed 灰）跨 5 平台一致
- vault seal 信任文案跨平台同字（P4 一致溝通）
- 「測試」鈕行為（即發一則 + inline 結果）跨平台一致

## 已決（per wireframe + 本檔拍板）

- toast 無 audio、persist、deep-link（本檔）；token 欄固定長度 **** 不洩長度（本檔）
- receipt 只 failed 用 danger，suppressed 用 muted（本檔）
- 全失敗 fallback banner 用 info 非 danger（本檔）

## 開放問題（留 prototype / 後續）

1. 「測試」失敗的 reason 要顯多細（避洩 SMTP host 細節到 log）— 待 SPEC-08 threat model 對齊
2. token 欄「顯示明文」眼睛 icon 要不要做（方便 vs 肩窺風險）— 待決
3. receipt 保留幾天 / 幾筆 — 待 SPEC-16 retention 對齊

## 下一步

**Stage 3 prototype**：toggle 開關 + 「測試」互動 + deep-link 冷啟動序列 + receipt 列表 + HTML 草圖。
