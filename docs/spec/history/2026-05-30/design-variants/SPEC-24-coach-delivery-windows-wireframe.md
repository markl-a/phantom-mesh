# SPEC-24 Coach Delivery — Windows Wireframe（線框稿）

> **Stage 1/3** · 線框稿（wireframe，低保真版型骨架）→ [視覺稿（mockup，待補）] → [原型（prototype，待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-28
> **Scope**: Windows only。SPEC-24 是 coach（教練）回顧的**派送通道**（怎麼送到 user 眼前）。本檔描述桌面三件事：(1) Markdown channel 的 **WinRT Toast**（Windows 通知中心快顯）+ deep-link（深連結）；(2) Settings → Coach → Delivery（派送設定）的 per-channel（逐通道）opt-in（選擇加入）+ 憑證輸入；(3) delivery receipt（派送回條）狀態。回顧本文渲染屬 SPEC-23（Coach tab），本檔只管「怎麼送 + 設定」。
> **Spec**: [`SPEC-24-SYSTEM-coach-delivery`](../specs/v060-deep-spec/SPEC-24-SYSTEM-coach-delivery.md) · [`SPEC-42`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md)

## 設計溯源（trace）

| 維度 | 對應 |
|---|---|
| **BIG-GOAL pillar** | 主要服務 **P1 跨裝置 Mesh**（cross-OS 通知面，Windows = WinRT Toast）+ **P4 加密為先**（bot token / SMTP 密碼走 SPEC-15 vault seal）；實作的是 **`X.coach` 派送能力**（SPEC-01 §8 cross-pillar capability anchor，非單一 pillar，服務 Life Track 的 coach review 送達）。**操作原則 Reversible**（可逐通道 off） |
| **Source spec** | SPEC-24-SYSTEM-coach-delivery |
| **Platform** | windows（桌面） |
| **Pipeline stage** | 1/3 wireframe |

## 為什麼 coach delivery 要 Windows wireframe

派送是「SPEC-23 寫完 review → 送到 user 眼前」。Windows 三個平台特定點：
1. **OS 通知 = WinRT Toast**（非 iOS UNUserNotificationCenter / Android NotificationManager / macOS NSUserNotification / Linux libnotify）— ActionCenter persist 行為 + deep-link 點擊冷啟動序列都不同
2. **桌面是輸入 SMTP / bot token 的好地方**（鍵盤友善）— 行動端打長 token 痛苦；Settings 的憑證輸入 UX 在桌面最完整
3. **deep-link `phantom-mesh://coach/review?id=` 冷啟動** — app 沒開時點 toast，要 launch app → 跳 Coach tab → 載入該 review

## 縮寫對照表

> - **channel（通道）**：一條派送路徑（Markdown+通知 / Telegram / Email）
> - **WinRT Toast**：Windows 10/11 ActionCenter（通知中心）的快顯通知
> - **deep-link（深連結）**：`phantom-mesh://coach/review?id=...` 點擊喚起 app 跳到特定畫面
> - **opt-in（選擇加入）**：使用者主動開啟某通道（預設只開 OS 通知）
> - **vault seal（保險庫封存）**：憑證客戶端加密存，broker 只看密文（SPEC-15）
> - **dedup（去重）**：同一 review 不重複發（24 小時 ledger 帳本）
> - **receipt（回條）**：每次派送的結果（pending/sent/failed/suppressed）
> - **SMTP / bot token / APNs / FCM**：見 SPEC-24 §1

## 三通道（per SPEC-24 §1）

| 通道 | Windows 行為 | v0.6.0 預設 | 憑證 |
|---|---|---|---|
| **Markdown + OS 通知** | 寫 `~/.phantom-mesh/coach/YYYY-MM-DD.md` + WinRT Toast（deep-link） | ✅ ON | 無 |
| **Telegram** | bot 發整份 markdown（>4096 自動 chunk）到 chat_id | ❌ OFF（opt-in） | bot token + chat_id（vault seal） |
| **Email** | user 自帶 SMTP，markdown → HTML | ❌ OFF（opt-in） | SMTP host/port/user/pass（vault seal） |
| Push（APNs/FCM） | — | ❌ 不做（v0.7+，SPEC-24 OoS） | — |

## 螢幕 A — WinRT Toast（Markdown channel，21:00 派送）

```
+------------------------------------------+
| Phantom Mesh                             |  ← AppLogo + app name
|  今天的回顧好了                          |  ← 標題（取 coach.deliv.toast_title）
|  下午 3 點放一杯水在桌上 . 點開看看      |  ← action 一句預覽（不報「你昨天只...」）
|                            [ 開啟回顧 ]  |  ← action button → deep-link
+------------------------------------------+
```
- `scenario="reminder"` → **persist 到 user dismiss**（Win 用戶常 miss 一閃通知，per SPEC-43）
- **無 `<audio>`**（晚上 21:00 溫和，shame-free + P4）
- 點 toast / 「開啟回顧」→ deep-link `phantom-mesh://coach/review?id=<review_id>`：
  - app 已開 → 直接跳 Coach tab 載該 review
  - app 沒開 → 冷啟動 app → 等 serve ready → 跳 Coach tab（中間顯 splash，不卡白屏）
- Focus Assist（專注模式）期間 → toast 折疊到 ActionCenter（不彈），user 之後自己看（review 非急事）

## 螢幕 B — Settings → Coach → Delivery（派送設定，真窗分頁）

```
+--------------------------------------------------+
| Settings . Coach . Delivery                      |
+--------------------------------------------------+
|  桌面通知（ActionCenter）         [ ON  o]        |  ← Markdown channel toggle（預設 ON）
|    回顧好了在右下角提醒你                         |
|                                                  |
|  Telegram                         [o OFF ]        |  ← opt-in toggle
|    bot token: [**********************]  [測試]   |  ← 輸入後 vault seal；測試發一則
|    chat id:   [____________]                     |
|                                                  |
|  Email                            [o OFF ]        |  ← opt-in toggle
|    SMTP host: [____________]  port:[___]         |
|    帳號:[__________] 密碼:[********]  [測試]      |  ← vault seal；測試寄一封
|    收件:[__________]                             |
|                                                  |
|  (lock) 金鑰 / 密碼都本地加密封存，broker 看不到 |  ← vault seal 信任說明
+--------------------------------------------------+
```
- **每通道一個 toggle**（Reversible 原則：一鍵 off 該路）
- 憑證輸入後存 SPEC-15 vault（密文）；UI 只顯 `****`（不回顯明文）
- **「測試」鈕** → 立刻發一則測試到該通道，回「已送達 / 失敗：{原因}」
- 預設只 OS 通知 ON；Telegram / Email 要 user 主動開 + 填憑證

## 螢幕 C — Delivery receipt 狀態（最近派送）

```
|  最近派送                                        |
|  5/27 OS通知   已送達 21:03                       |  ← sent
|  5/27 Telegram 已送達 21:03                       |
|  5/27 Email    失敗：SMTP 認證錯誤  [重試][設定]  |  ← failed + 行動
|  5/26 OS通知   已送達 (重複已略過)                |  ← suppressed（dedup）
```
- 每路獨立 receipt（pending → sent / failed / suppressed，per SPEC-24 §8）
- **單路失敗不阻其他路**（並行 `tokio::join_all`）— Email 掛了 Telegram 照送
- failed 給「重試」+「設定」（跳去修憑證）

## 失敗 / 邊界（per SPEC-04 + SPEC-24 §11 COACH_DELIV_*）

- 3 通道全失敗 → review 仍在 markdown file；下次開 app 顯橫幅「上次回顧已備好」（不靜默丟）
- Telegram chunk >4096 → 自動分段發（不截斷）
- SMTP 認證錯 → `COACH_DELIV_SMTP_AUTH` → receipt failed + 不重試到天荒地老（退避）
- dedup：同 review_id + channel 24h 內重發 → suppress（不洗 user）

## 待補（下一 pipeline stage）

- **Stage 2 mockup**：toast XML 終值（兩變體）、Settings toggle / 憑證欄視覺、vault seal badge、receipt 狀態色（sent 綠 / failed 才用 danger）、終版文案、Narrator a11y
- **Stage 3 prototype**：toggle 開關 + 「測試」互動 + deep-link 冷啟動序列 + receipt 列表 + HTML 草圖
- 跟 SPEC-23 的關係：SPEC-23 emit `coach.review.ready` → 本檔 channel router 接手；review 本文 viewer 在 SPEC-23 Coach tab
