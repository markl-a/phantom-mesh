# SPEC-21 Capture Focus — Android Prototype（原型）

> **Stage 3/3** · [線框稿（Android）](./SPEC-21-capture-focus-android-wireframe.md) → [視覺稿（Android）](./SPEC-21-capture-focus-android-mockup.md) → 原型（Android）
> **Status**: draft v0.1 · **Last updated**: 2026-05-27
> **Scope**: Android only。hero 平台是 iOS — 互動骨架（FSM / Nielsen 5 / SUS / 7 user task）繼承 [iOS hero prototype](./SPEC-21-capture-focus-prototype.md)，本檔只列 **Android-specific 互動 delta**（Material ripple / FG-service notification / WorkManager / Quick Settings tile / OnBackPressedDispatcher / TalkBack / MIUI 引導）。
> **同步狀態**：對齊 [Android wireframe v0.1](./SPEC-21-capture-focus-android-wireframe.md)（210 行）+ [Android mockup v0.1](./SPEC-21-capture-focus-android-mockup.md)（222 行）+ [SPEC-34 Android-screens-flows](../specs/v060-deep-spec/SPEC-34-PLATFORM-Android-screens-flows.md)。互動 timing / haptic / 失敗路徑歸本檔；視覺 token 歸 mockup；FSM 規格歸 wireframe — 三檔嚴格分層。
> **Spec**: [`SPEC-21-SYSTEM-capture-focus`](../specs/v060-deep-spec/SPEC-21-SYSTEM-capture-focus.md) · [`SPEC-33-PLATFORM-Android-foundations`](../specs/v060-deep-spec/SPEC-33-PLATFORM-Android-foundations.md) · [`SPEC-34-PLATFORM-Android-screens-flows`](../specs/v060-deep-spec/SPEC-34-PLATFORM-Android-screens-flows.md)
> **這份的工作範圍**：把 Android Mockup 變「可操作」 — 每個 Material 元件按下去發生什麼、ripple / haptic / animation timing、FG-service notification 互動 sequence、Quick Settings tile 互動、WorkManager Expedited Job 觸發、system back 處理、TalkBack focus order、MIUI 引導 dialog 互動、6 大資料狀態 Android-specific 互動。
> **參考**：iOS hero prototype（488 行 R4 sign-off）— 同樣風格 + 嚴謹度；[iThome Day6](https://ithelp.ithome.com.tw/articles/10295775) + [Tom Liou Medium](https://medium.com/@tomliou/給超超超新手的uiux指南-96c80687a20f)。

## 為什麼 Android 有獨立 prototype

iOS hero prototype 488 行涵蓋 FSM / Nielsen / SUS / 7 task 完整骨架。Android 互動上有 8 個本質差異不能塞 hero 一段帶過：

1. **Material ripple** 不是 iOS press 樣式 — duration / 顏色 / origin 都不同
2. **FG-service notification** 取代 iOS lock-screen — tap target / pendingIntent / 文字更新時機
3. **WorkManager Expedited Job** 處理 app 被 OS 殺後的 ASR 完成 — iOS 沒這 API
4. **Quick Settings tile 1 tap 25min** — tile click sequence + broadcast → MeshNodeService 喚醒延遲
5. **OnBackPressedDispatcher** 派 system back 進 React Router — iOS swipe-back 機制完全不同
6. **HapticFeedbackConstants** 而非 UIImpactFeedbackGenerator — 強度級距不同
7. **TalkBack focus order + contentDescription** — iOS VoiceOver 有 implicit ordering，Android 必須顯式
8. **MIUI 引導 dialog** + **B2 skip degraded UI bar 互動** — iOS 不存在的場景

→ 8 點值得獨立 prototype 級腳本化。

## Nielsen 5 易用性檢核（Android 對應）

繼承 iOS hero §Nielsen 5 表格五大原則，Android 表現差異：

| 原則 | Android Focus 表現方式 |
|---|---|
| **可學習性** | bottom-nav 4 tab 標準 Material 3 IA → user 一看就會；FG-service notification 第一次出現有 channel 描述「焦點 session」說明用途 |
| **效率性** | Quick Settings tile 1 tap 25 min focus（per SPEC-34 G5，6 步壓 1 步）；通知欄 stop button 不必開 app；back 鍵自然回上一層（React Router 接管） |
| **記憶性** | bottom-nav 不變；ripple 顏色全 app 統一（`overlay-ripple-24`）；MIUI dialog「不再提示」永久記住 |
| **失誤性** | 3 perm gate 各有獨立 deep link；B2 skip 仍可錄音（不強制）；MIUI dialog deep-link 失敗時 fallback 文字步驟 |
| **成就感** | check_circle 64dp + haptic `CONFIRM` 雙短震；通知欄完成短暫顯示「完成 · 25 分鐘 · 5 chunks」3s 後自動 dismiss |

## 6 大資料狀態速查（Android 對應）

繼承 iOS hero 表格，Android 場景對應：

| 狀態 | Android 何時出現 | i18n key（per Android mockup v0.1）|
|---|---|---|
| **理想** | F Done card 完整 takeaway | `focus.done.title` |
| **空白** | History tab in Capture 無 session / ASR 無語音 | `focus.empty.history` / `focus.empty.no_speech` |
| **極限** | C chunk ≥ 100 / F takeaway > 800 字 | `focus.limit.chunk_overflow` / `focus.limit.takeaway_truncated_hint` |
| **錯誤** | B' Denied 卡（B1）/ Interrupted / 全 ASR 掛 | `focus.perm.denied` / `focus.interrupted.*` / `focus.err.no_takeaway` |
| **局部** | E 內 inline chunk 失敗 | `focus.partial.chunk_failed` |
| **載入中** | E Finalizing + 通知欄文字同步更新 | `focus.finalizing.asr` / `focus.finalizing.llm` |

## 9-state FSM 主骨架（per Android wireframe v0.1，同 iOS）

FSM 完全同 iOS hero — Android 不重抄。每個 state 的 Android-specific 互動職責：

| FSM state | Android 對應 | Android-specific 互動 |
|---|---|---|
| `Idle` | A | bottom-nav 顯示；duration ripple + selection haptic |
| `Requesting` | B1 / B2 系統 prompt | OS 接管，phantom 只設 rationale 字串 |
| `Recording` | C + FG-service notification（D）| 通知欄常駐；back 鍵 confirm；status bar tint |
| `Chunking` | C 內微 state | chunk +1 Snackbar bottom-up |
| `Interrupted` | C' | AUDIOFOCUS_LOSS callback；通知文字切「電話中已暫停」 |
| `Finalizing` | E + 通知欄文字同步 | LinearProgressIndicator；通知 channel 文字更新 |
| `Transcribing` | E phase 1 | progress 0→100%；per chunk Snackbar partial |
| `SummaryGen` | E phase 2 | CircularProgressIndicator 持續 |
| `Done` | F + 通知短暫 3s dismiss | check_circle 動畫 + haptic CONFIRM；通知顯示「完成」3s 後自動 cancel |

---

## 螢幕 A — Focus Idle（in Capture tab）

### Nielsen 5 對應

- Learnability：Capture tab 標 mic icon → user 直覺；duration picker 三檔 + PTT/Timer 雙鈕版面同 iOS hero
- Efficiency：上次選的 duration 預選（DataStore 存 `last_duration_min`）；Timer 預設 25min 一 tap 開
- Memorability：bottom-nav 4 tab 永遠在；trust badge 同位
- Errors：3 perm gate 各有獨立失敗路徑；MIUI 場景由 dialog 主動提示
- Satisfaction：trust badge 給安全感；ripple 即時觸覺回饋

### 6 大資料狀態

| 狀態 | UI 表現 |
|---|---|
| 理想 | duration picker 三檔顯示；PTT + Timer 雙鈕可按；trust badge 顯示；FG-service 通知未顯示（Idle 不錄）|
| 空白 | 同理想（Idle 無資料概念） |
| 極限 | custom duration 上限 180 / 下限 5 — clamp + shake animation（具體數字 per iOS hero invariant，本檔不重述）|
| 錯誤 | 麥克風硬體不存在 → PTT + Timer disabled + Snackbar `focus.err.no_mic`（FOCUS-002） |
| 局部 | n/a |
| 載入中 | n/a |

### Tap targets（按下去發生什麼）

| Target | 動作 |
|---|---|
| `← arrow_back` top app bar | 觸發 React Router `navigate(-1)`；history 空時放行系統退出（per SPEC-34 §30(C)） |
| `settings` icon top app bar | navigate `/settings/focus`；ripple 從 icon 中心擴散 |
| `25` chip | Material ripple `overlay-ripple-24` 從 tap 點擴散，duration 60ms enter / 600ms hold / 100ms exit（Material 3 規範）；haptic `HapticFeedbackConstants.CLOCK_TICK`（輕觸覺）；其他兩 chip deselect；clock-face dim "25:00" 更新 |
| `50` chip | 同上，更新 "50:00" |
| `自訂` chip | 同上，且彈 Material `TextField` numeric stepper（5–180）；haptic `CLOCK_TICK` |
| **PTT button — press-down** (`focus.btn.ptt`) | (1) 若 RECORD_AUDIO 未授權 → 走 B1 perm prompt；(2) 已授權 → ripple 從觸點擴散 + haptic `HapticFeedbackConstants.KEYBOARD_PRESS`（≈ iOS light）；100ms 內 AudioRecord.open + isRecording=true；**留在 Idle screen**（PTT 是一次一段累積）；同 frame Timer button 進 disabled state（per mockup invariant） |
| **PTT button — press-up** | 立即 close chunk + push to EventStore；haptic `KEYBOARD_RELEASE`；Material `Snackbar` bottom-up 顯示「已落地 chunk +1」（Chunking sub-state 視覺對應，per mockup C frame）；Timer button 回 enabled |
| **Timer 開始 button** | (1) 若 RECORD_AUDIO 未授權 → 走 B1 perm prompt；(2) 已授權 → haptic `HapticFeedbackConstants.CONFIRM`（≈ iOS medium）；先嘗試 startForegroundService(MeshNodeService) + FOREGROUND_SERVICE_TYPE_MICROPHONE；切到 C Recording screen（Timer mode）；started_at = now，計時開跑。**反向互斥**：切到 C 後 PTT button 從版面消失 |
| trust badge | tap → push `TrustExplainerView`（Material `BottomSheet`）；文案 `focus.trust_badge` 一字不差；ripple `overlay-ripple-24` |
| bottom-nav 切 tab | 切到 Home / Coach / Settings；Idle 無未儲存資料 → 不 confirm |

### Animations / Timings

- Chip ripple：Material 3 標準 60ms enter / 持續 hold until release / 100ms fade-out
- Chip selection background transition：state-layer 200ms ease-out
- PTT button press visual：ripple 同上；放開回彈無額外 animation（ripple fade-out 已足夠）
- Screen transition Idle → C Recording (Timer mode)：Material 3 `SharedAxis.Z` forward 300ms（不是 iOS push 350ms — Material 慣例較快）
- Trust badge bottom sheet：slide-up 250ms emphasized easing
- PTT press-down 觸發 Timer disabled 同 frame（0ms，避免 jank）
- bottom-nav tab switch：fade + slight horizontal slide 200ms

### Failure paths

- 首次按 PTT/Timer 觸發 B1 RECORD_AUDIO runtime perm prompt：OS 接管渲染（無法控時間）；prompt 出現前先進 "Requesting" 微 state（button 顯示 CircularProgressIndicator 16dp，per mockup §136 spinner spec）；deny → 切 B' Denied 卡（覆蓋 Idle）
- 麥克風被佔用（其他 app 在錄）：AudioRecord.read 回 ERROR → Snackbar `focus.err.mic_busy`（FOCUS-002）+ 不切螢幕
- MIUI 場景：Timer button 第一次 tap 若 MeshNodeService 啟動失敗 + `is_miui=true` → 跳 MIUI 引導 dialog（見 §MIUI 引導 dialog）

### TalkBack focus order

1. top app bar back button — "返回"
2. top app bar settings icon — "焦點設定"
3. trust badge — "本地加密，本機 ASR，不上雲端"
4. duration picker — "選擇焦點時長"，三 chip 依序 "25 分鐘"/"50 分鐘"/"自訂時長"
5. PTT button — "按住說話，錄音中可放開"
6. Timer button — "開始 {min} 分鐘計時錄音"
7. bottom-nav 4 tab — Home / Coach / Capture（current）/ Settings

### Walkthrough script（usability test：「請開始一段 25 分鐘的專注錄音」）

1. 預期 user 開 app → 看到 bottom-nav Capture 已選 → 看到 Focus 區
2. user 點 "25 分鐘"（已預設）→ 點 "開始計時錄音"
3. 首次：撞 B1 RECORD_AUDIO prompt → 期待 user 點 Allow
4. 撞 B2 POST_NOTIFICATIONS prompt → 期待 user 點 Allow 或 skip
5. 切到 C Recording 螢幕 + 通知欄 D 出現
- **觀察點 1**：user 是否注意到通知欄常駐通知？若 80% user 沒注意，要強化 D 顯著度
- **觀察點 2**：B2 skip 後 user 是否被頂部 degraded UI bar 干擾？

---

## 螢幕 B1 — RECORD_AUDIO Runtime Perm Prompt

OS 渲染，不可控版面。phantom 只能設 `permission_request_text`（manifest rationale）。

### Tap targets

| Target | 動作 |
|---|---|
| `不允許` | OS 記下 denied；app 收到 `onRequestPermissionsResult` callback；切 B' Denied 卡覆蓋 Idle；無 haptic（OS 自處理） |
| `允許` | OS 記下 granted；繼續下一步：若 Android 13+ 跳 B2 POST_NOTIFICATIONS；否則直接進 Recording flow |
| `僅在使用 app 時允許` | 同 `允許`（Android 11+ one-time grant 非適用 RECORD_AUDIO — 此選項 Android 不提供，僅 LOCATION 有；列此供 reviewer 參照）|

### B' Denied 卡互動（覆蓋 Idle）

- 視覺 spec 在 mockup B'（mic_off 48dp `colorError`）
- 文案 keys 同 iOS hero（`focus.perm.denied` / `focus.perm.denied_reassure` / `focus.perm.open_settings`）
- Tap 「打開設定」→ `Intent("android.settings.APPLICATION_DETAILS_SETTINGS").setData(Uri.parse("package:dev.phantom.mesh"))` startActivity；無 confirm；無 haptic
- 覆蓋出現時：Idle 底下 PTT + Timer 同套 mockup `overlay-disabled-40`
- user 從設定回 app → `onResume` 重檢權限 → granted 則自動拿掉遮罩 + scroll-restore

---

## 螢幕 B2 — POST_NOTIFICATIONS Perm Prompt（Android 13+）

OS 渲染，不可控。Android 12 以下無此 prompt（自動授權）。

### Tap targets

| Target | 動作 |
|---|---|
| `允許` | granted；繼續進 Recording flow；FG-service notification 將正常顯示 |
| `不允許` | denied；繼續進 Recording flow（**不阻止錄音**）；Idle 頂部顯示 degraded UI bar |

### B2 skip 後 degraded UI bar 互動

- bar 位置：Idle 頂部，高 32dp（per mockup spec）
- 文案 `focus.android.notif_optional`：「沒給通知權限也可錄，但通知欄不會顯示控制」
- **dismiss 互動**：
  - swipe right：手指右滑 ≥ 60dp → bar 跟手位移 + ease-out fade，總 duration 200ms；haptic `CLOCK_TICK`
  - tap `×` icon：ripple 從 icon 擴散 → bar 上滑出畫面 150ms ease-in；無 haptic
  - dismiss 後存 DataStore `notif_optional_dismissed_at` → 同 session 不再顯示；下次冷啟動仍顯示（per wireframe invariant — 不持久 dismiss，避免 user 忘）
- **recovery 入口**（per Android wireframe R1 catch）：settings → "重新授權通知" button → 同樣 deep link 進 `APPLICATION_DETAILS_SETTINGS`；user 開回權限後下次 Recording 自動顯示 FG-service shade UI（不需重啟 app — onResume 重檢）

---

## 螢幕 C — Recording（Timer mode）

### Nielsen 5 對應

- Learnability：計時器 48sp 大數字 displayMedium → 一眼看清；Stop button `colorError` 警示直白
- Efficiency：Pause + Stop 雙鈕在拇指範圍；通知欄 D 鎖屏可控（不必解鎖）；back 鍵有 confirm（防誤觸）
- Memorability：版型 + trust badge 跟 Idle 同位；bottom-nav 在 Recording 中隱藏（per SPEC-34 IA — 避免 user 切走丟 session）
- Errors：誤按 Stop 不加 confirm（效率優先）；back 鍵加 confirm（更易誤觸）
- Satisfaction：waveform 32 bars 即時跳動；chunk count Chip 累積看得到

### 6 大資料狀態

| 狀態 | UI 表現 |
|---|---|
| 理想 | 計時器跑；waveform 連動；chunk count 每 5min +1 觸發 Chunking |
| 空白 | n/a |
| 極限 | 50min / custom 180min 到 → 自動 stop；chunk ≥ 100 顯 `99+` chip（`focus.limit.chunk_overflow`，數字區塊 min-width 鎖死 per mockup invariant，無轉場動畫純態切換）|
| 錯誤 | AUDIOFOCUS_LOSS / 系統 sleep → 切 C' Interrupted；通知欄文字同步切「電話中已暫停」 |
| 局部 | n/a（per chunk 失敗在 E）|
| 載入中 | n/a |

### Tap targets

| Target | 動作 |
|---|---|
| `Pause` (OutlinedButton `focus.btn.pause`) | (1) AudioRecord.stop（保留 file handle）；(2) waveform 凍結 `phantom-muted`；(3) 計時暫停（每秒 1Hz 閃爍 `colorTertiary` 標警；色 token per mockup）；(4) button morph 為 `Resume` (`focus.btn.resume`) FilledTonalButton；haptic `HapticFeedbackConstants.CONFIRM`；通知欄 D 文字切「Focus · 已暫停」 |
| `Resume` | 反向：AudioRecord.startRecording、waveform 重跑、計時繼續、button 變回 `Pause`；haptic `CONFIRM`；通知欄回 "Focus · {elapsed} / {total}" |
| `Stop` (FilledTonalButton `colorError` `focus.btn.stop`) | 立即 cross-fade 250ms 到 E Finalizing（不加 confirm dialog）；haptic `HapticFeedbackConstants.LONG_PRESS`（≈ iOS heavy）；AudioRecord.close；flush 殘留 chunk → 觸發 Chunking → Finalizing 鏈；MeshNodeService 不停（finalize phase 仍跑 FG）|
| chunk count Chip `已落地: {n}` | 99 → 100 切 `99+`（`focus.limit.chunk_overflow`）；tap 無動作（v0.6.0 純資訊；v0.7+ 跳 history）|
| trust badge | tap 同 Idle 開 BottomSheet |
| **system back button** | 觸發 OnBackPressedDispatcher → React Router 攔截 → Material `AlertDialog` 顯示 `focus.confirm.leave_recording_msg` + `focus.btn.cancel` / `focus.confirm.leave_recording_stop`（per iOS hero same dialog）；點 stop 走 Stop 路徑；點 cancel 留在 C |

### Background / Notification（FG-service）行為

- 進背景（home 鍵 / lock）：MeshNodeService 已 `startForeground(FOREGROUND_SERVICE_TYPE_MICROPHONE)` → 系統允許繼續錄；UI 不切（背景仍是 React routing 內）
- 鎖屏：通知欄 D 顯示「Focus · 05:23 / 25:00」+ STOP action
- 來電 / 其他 app 抓 mic：AudioManager.requestAudioFocus 失去 → AUDIOFOCUS_LOSS callback → 進 C' Interrupted；UI 顯示 `focus.interrupted.phone` + `focus.interrupted.resume_hint`；OS resume 自動回 C / 超時走 E — 寬限與超時數字 per wireframe FSM

### Animations / Timings

- waveform refresh：60fps；AudioRecord buffer 1024 samples 為一禎（柱數 / 高度 / 色 token 在 mockup）
- 計時器數字：每秒 update，無 animation
- Pause/Resume icon morph：Material `AnimatedVisibility` crossfade 200ms
- **chunk +1 Snackbar**：bottom-up enter 150ms / hold 2s（Material 標準 Long）/ exit 100ms；不阻擋 user 操作
- Stop → E Finalizing：螢幕 cross-fade 250ms（Material `SharedAxis.Z` 不適用 — Stop 是結束感）
- status bar tint Recording 中切 `colorTertiary`：300ms ease-out

### Failure paths

- 麥克風被搶（其他 app 開錄音）：AUDIOFOCUS_LOSS → 進 C' Interrupted；寬限與超時數字 per wireframe FSM；超出寬限未恢復 → 強制 finalize（標 `interrupted=true`）
- 儲存空間滿（chunk encrypt 失敗）：Snackbar `focus.err.disk_full`（FOCUS-005，per iOS hero R3 補的 i18n key）+ 切 E Finalizing（保留已落地 chunk）
- App 被 OS 殺（low memory / MIUI 強殺）：MeshNodeService 也被殺 → 已落地 chunks 留 disk → WorkManager Expedited Job 排程 ASR（見下方 §WorkManager 接管）
- 通知 channel 被 user 設「靜音」：仍 work（IMPORTANCE_LOW 本來就靜音）；user 拒 channel → 該 channel notification 不顯示但錄音正常（同 B2 skip 效果）

### Walkthrough script（usability test：「開始錄音 30 秒、暫停、繼續、停止」）

1. 從 Idle → 25min Timer 按下 → 撞 perm → 切到 C
2. **觀察點 1**：user 是否看通知欄？若 50% user 不知道有 D 通知存在，要在第一次 Recording 加 onboarding tooltip「您可從通知欄停止」
3. user 按 Pause → 期待 waveform 凍結 + 通知欄同步切「已暫停」
4. **觀察點 2**：user 是否認得 Resume button？Material icon `play_arrow` 是否清楚？
5. user 按 Resume → user 按 Stop → 切 E Finalizing
6. **觀察點 3**：user 是否預期 Stop 後資料還在？比 iOS 多一個焦慮源：「app 被系統殺掉怎辦」→ 觀察是否有人問這

---

## 螢幕 D — FG-service Notification（取代 iOS lock-screen）

通知 channel `focus_session` IMPORTANCE_LOW + setOngoing(true) + smallIcon `R.drawable.ic_phantom_mono`。視覺 spec 在 mockup §104。

### Tap targets

| Target | 動作 |
|---|---|
| 通知 body | startActivity launchIntent → 開回 phantom 進 Focus tab；若 app process 死了走 cold start，但因 MeshNodeService 仍在，UI 重建後直接到 C Recording（state 從 service restore）|
| `STOP` action button | PendingIntent broadcast → MeshNodeService 收 → 等同 app 內 `Stop`；haptic 不可控（OS 系統手感）；通知欄文字 200ms 內切 `focus.finalizing.asr`；UI 若在前景同步切 E |
| 通知欄左下 timestamp / app icon | OS 渲染，無動作 |
| user swipe-dismiss attempt | setOngoing(true) 鎖死 → 滑不掉；user 感受手指能拖但放開 spring-back（OS 行為）|

### FG-service notification 文字更新 sequence

per Android wireframe §122，通知文字隨 FSM 切：

```
Recording        → "Focus · {elapsed} / {total}"      每秒 update
[Pause tap]      → "Focus · 已暫停"                    瞬間切
[Resume tap]     → "Focus · {elapsed} / {total}"      恢復每秒
[Stop tap / 自動結束] → 觸發 Finalizing chain：
                  → "整理逐字稿 (2/5)…"               (focus.finalizing.asr)
                  → "產生 takeaway 中…"               (focus.finalizing.llm)
                  → "完成 · 25 分鐘 · 5 chunks"      (3s display only)
                  → notificationManager.cancel()        3s 後自動消
```

- 文字 update 用 `NotificationCompat.Builder.setContentText().build()` + `notify(id, ...)` 重 post 同 id
- 不重 post smallIcon / channel / action（避免閃爍）
- update 頻率 限 1Hz（per Android NotificationManager rate limit best practice）

### 失敗路徑

- POST_NOTIFICATIONS B2 skip：通知不顯示但錄音正常；FG-service 仍 work（service alive 不需 notification 顯示）— Android 14+ 嚴格要求 FG notification 顯示，但 user 拒了 OS 自動處理（service 不會被殺，只是通知欄不見）
- 通知 channel user 手動關：同 B2 skip 效果；recovery 入口同 §B2 recovery
- AirPods 中鍵 / 藍牙耳機 play-pause：Android 上對應 `KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE` → MediaSession callback → 等同 app 內 Pause/Resume；haptic 不可控

---

## 螢幕 C' — Interrupted sub-state

OS interrupt 來源（Android-specific，per wireframe §111）：
- 來電（AudioManager AUDIOFOCUS_LOSS / AUDIOFOCUS_LOSS_TRANSIENT）
- 其他 app 抓 mic（同上）
- 系統 sleep / Doze mode
- 藍牙耳機切換（mic source change）

### UX 互動

- 視覺等同 iOS C'（waveform 凍結 `phantom-muted` + interrupted toast）
- Snackbar 文案：`focus.interrupted.phone`（來電）/ `focus.interrupted.mic_grabbed`（被搶）+ `focus.interrupted.resume_hint`
- 計時器：暫停閃爍同 Pause state，但 user 無法主動 Resume — 必須等 AUDIOFOCUS_GAIN
- 通知欄 D 同步切「電話中已暫停」（取 `focus.interrupted.phone`）
- 寬限與超時數字 per wireframe FSM（不重述）
- AUDIOFOCUS_GAIN_TRANSIENT 收到 → 自動 resume + waveform 回 active + Snackbar fade out
- 超時未 GAIN → 強制走 E Finalizing（標 `interrupted=true`）

### 失敗路徑

- 來電通話太久（> 寬限）：強制 finalize，user 掛電話回 app 已在 E
- 系統 Doze 直接殺 MeshNodeService：WorkManager Expedited Job 接管（見下方）
- 藍牙耳機切換：mic source 換到 BT mic 後 AUDIOFOCUS_GAIN → 自動 resume，但 audio 可能短暫斷（≤ 500ms 切換 gap）— chunk 不切，audio buffer 容忍

---

## 螢幕 E — Finalizing（過渡）

### Nielsen 5 對應（同 iOS hero §E）

文案直白 / 兩段 progress / 載入色系延續 / 失敗 inline / progress 真實。

### 6 大資料狀態

| 狀態 | 互動行為 |
|---|---|
| 理想 | LinearProgressIndicator 0→100% (Transcribing) → CircularProgressIndicator 持續 (SummaryGen) |
| 載入中 | 同上（本螢幕常態）；通知欄 D 同步更新文字 |
| 局部 | per chunk ASR 失敗 → inline Snackbar `focus.partial.chunk_failed`；progress 跳過失敗 chunk |
| 錯誤 | 全 ASR 掛 → FOCUS-003 + `重試 ASR` `先用空白 transcript 跑 LLM` 雙 button |
| 極限 | 50min audio + 10 chunks：whisper.cpp small on Pixel 7 ≈ 50-70s；超過 120s 顯示 `focus.finalizing.taking_longer` |

### Tap targets

| Target | 動作 |
|---|---|
| `取消並先看逐字稿` (`focus.btn.cancel_show_transcript`) TextButton caption 級 | 中斷 LLM call；用 stitched transcript 直接寫 focus row（takeaway 留空 + `takeaway_model="(skipped)"`）；切 F Done；ripple 從 tap 點擴散 |
| `重試 ASR` (`focus.btn.retry_asr`) FilledButton | 錯誤態出現；重跑所有失敗 chunk 的 ASR；若 settings 開了 cloud fallback → Groq Whisper |
| `先用空白 transcript 跑 LLM` (`focus.btn.use_empty_transcript`) TextButton | 全 ASR 掛時出現；LLM 跑空字串 → takeaway="(無 audio 可分析)" → 寫 row（保留 audio 可後補 re-asr） |
| **system back button** | 攔截 → AlertDialog 警告「離開會丟 takeaway」+ 取消 / 確定離開；確定 = 同「取消並先看逐字稿」路徑 |
| 螢幕 swipe-down | Material 螢幕本身無 modal 可滑（E 是 full screen Composable）— 唯一出口是 button |

### Timings

- ASR 各 chunk：on-device whisper small on Pixel 7 ≈ 10–14s per 1 min audio；5 chunks 序列 ≈ 50–70s（平行版 v0.7+）
- LLM takeaway：Groq llama-3.1-70b ≈ 2–4s for 5000-token transcript
- CircularProgressIndicator 滾速：Material 標準（不可改）
- LinearProgressIndicator update：每 chunk 完成 +20%（5 chunks → 20/40/60/80/100）；不假裝平滑（user 信賴度）
- **同時更新 FG-service notification**：每進 Transcribing phase 立即切 `focus.finalizing.asr`；進 SummaryGen 切 `focus.finalizing.llm`

### Failure paths

- 所有 ASR 掛：FOCUS-003 雙 button；user 選「先用空白 transcript」→ LLM 仍跑 → takeaway="(無 audio 可分析)" → 寫 row
- LLM 失敗：FOCUS-004 + 留 transcript + takeaway="(摘要失敗，可手動重跑)" + 寫 row
- 取消 LLM：transcript 仍 stitch、row 仍寫；`phantom focus reasr <id>` 可後補
- App 進背景：MeshNodeService 持續跑 finalize；user 回前景 UI 從 service state restore；若 app process 被殺 → WorkManager 接管

### Walkthrough script（usability test：「等 finalize 完成」）

1. user 從 C Stop → 進 E
2. **觀察點 1**：user 是否會切走 app 看別的？若 80% user 在 30s 內切走，要強化「進背景仍會完成」訊息
3. **觀察點 2**：等待 50s+ 後 user 是否會找 cancel button？caption 字級夠小避免誤觸，但夠大讓焦慮 user 找得到？

---

## 螢幕 F — Done（Takeaway card）

### Nielsen 5 對應（同 iOS hero §F）

session metadata 直白 / 兩 button 一 tap 下一步 / 卡片版式跟 history list 一致 / success haptic + scale spring。

### 6 大資料狀態

| 狀態 | UI 表現 |
|---|---|
| 理想 | 完整 takeaway 三段：主要 ideas / action items / 情緒卡點 |
| 空白（無 takeaway）| takeaway 空（user 取消 LLM 或 LLM 全掛）→ 卡片顯 `focus.empty.no_takeaway` + `重跑摘要` (`focus.btn.retry_summary`) |
| **空白（無語音）** | ASR 偵測無語音 → 顯示 `focus.empty.no_speech`：「本次時段未偵測到語音，已為您記錄時長：25 分鐘」+ `[重錄這次]` `[完成]` 雙 button（per Android mockup §145 新增變體）|
| 極限 | takeaway 截斷（字數閾值 / fade gradient per mockup F Limit invariant）+ inline hint `focus.limit.takeaway_truncated_hint` + CTA `focus.limit.view_full_takeaway` |
| 錯誤 | 全 ASR 掛 → takeaway="(無 audio 可分析)" + `focus.err.no_takeaway` + `重跑 ASR` |
| 局部 | 5 chunks 中 1 個 ASR 失敗 → 卡片頂部 banner `focus.partial.chunk_failed` |
| 載入中 | n/a |

### Tap targets

| Target | 動作 |
|---|---|
| `看完整逐字稿` (`focus.done.view_full`) FilledButton | navigate `/focus/{id}/transcript`；ripple；haptic `KEYBOARD_PRESS` |
| `新 session` (`focus.done.new_session`) OutlinedButton | popBackStack 整 stack 回 A Idle；不保留 takeaway preview（已存 events）；ripple |
| takeaway card (truncated state) tap 整張 OR `focus.limit.view_full_takeaway` CTA | **等同點 `看完整逐字稿`**（per iOS hero invariant 一致）；不在原地展開 |
| **空白（無語音）`重錄這次` button** | 跳回 A Idle 同 mode（Timer / PTT）；舊 session row 仍寫（duration 完整）；haptic `CONFIRM` |
| **空白（無語音）`完成` button** | 寫 events row + 切到 Capture tab History 區 |
| top app bar `← arrow_back` | 同「新 session」 |
| **system back button** | 同「新 session」（不加 confirm — Done 已是 success state）|

### Entry animation

- 從 E Finalizing 進入：Material `SharedAxis.Z` forward 300ms + check_circle 64dp 從 0→1 scale spring (Material spring `dampingRatio = 0.6, stiffness = 800`)
- haptic `HapticFeedbackConstants.CONFIRM` 雙短震
- takeaway card：fade-in + slight slide-up 12dp，350ms emphasized easing
- 通知欄 D：同 frame 切「完成 · 25 分鐘 · 5 chunks」短暫顯 3s 後 notificationManager.cancel()

### Failure paths

- 從 WorkManager 路徑進來（app cold start 後）：F 顯示同樣 takeaway card；haptic 跳過（避免 user 嚇到）；通知欄 D 已自動 cancel（finalize 結束時 cancel）— user 可能會問「啥時做完的」→ session metadata 顯示完成時間
- v0.7+ 加分享按鈕問題同 iOS：v0.6.0 不加

### Walkthrough script

1. user 看到 F card → 預期讀 takeaway
2. **觀察點 1**：user 是否能 3s 內 grasp takeaway 三段結構？
3. **觀察點 2**：user 是否預期 takeaway 已存？比 iOS 多焦慮源：「會不會 app 被殺 takeaway 就丟」→ 觀察是否需在 F 加「已永久保存於本機」hint

---

## 跨螢幕互動 — Android-specific

### Interruption Flow（來電 / 其他 app 搶 mic）

```
C Recording ──[AudioManager AUDIOFOCUS_LOSS]──> C' Interrupted sub-state
       │
       ▼
   waveform `phantom-muted`、計時暫停、Snackbar `focus.interrupted.phone` + resume_hint
   通知欄 D 文字切「電話中已暫停」
       │
       ▼
   [AUDIOFOCUS_GAIN 寬限內收到]
       │
       ▼
   AudioRecord.startRecording → C 繼續，Snackbar fade out，通知欄文字回原
       │
       ▼
   [GAIN 寬限超時 — 數字 per wireframe FSM]
       │
       ▼
   強制 finalize（interrupted=true 標記）→ E Finalizing
   通知欄 D 文字切「整理逐字稿 (2/5)…」
```

### system back button 行為（per SPEC-34 §30(C) OnBackPressedDispatcher）

OnBackPressedDispatcher → JNI emit Tauri event `system.back` → React Router 攔截：

| 螢幕 | back 行為 |
|---|---|
| A Idle | React Router navigate(-1)；history 空 → 放行系統退出 |
| C Recording | 攔截 → 顯 AlertDialog `focus.confirm.leave_recording_msg`；確定 = 走 Stop |
| C' Interrupted | 攔截 → 同 C confirm |
| E Finalizing | 攔截 → 顯 AlertDialog「離開會丟 takeaway」；確定 = 走「取消並先看逐字稿」 |
| F Done | navigate(-1) 回 A Idle（等同「新 session」） |

### App 進背景 / 回前景

- A Idle → 背景：無事；回前景檢查 perm（onResume）
- C Recording → 背景：MeshNodeService 持續跑；UI state 由 service state 還原（onResume）；通知欄 D 在
- C' Interrupted → 背景：同 C，但 user 回前景看到 Interrupted state（不切回 C 除非 GAIN）
- E Finalizing → 背景：MeshNodeService 持續 finalize；若 app process 被 OS 殺 → WorkManager 接管（user 下次開 app 看到 F）
- F Done → 背景：無事；回前景仍是 F；通知欄 D 已 cancel（finalize 結束）

### WorkManager Expedited Job 接管（app 被殺路徑）

per Android wireframe §139，best-effort 非 guarantee：

```
C/E ──[app process killed by OS / MIUI 強殺]──> 已落地 chunks 留 disk
       │
       ▼
   session stop 時排 1 個 Expedited 聚合 ASR job（不是 per-chunk job）
   setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
       │
       ▼
   [OS Doze / Battery Saver 容許執行]
       │
       ▼
   ASR 跑完 stitch transcript → LLM 跑 takeaway → 寫 events row + 通知顯「focus 完成 — tap 查看」
       │
       ▼
   user tap 通知 → 直接到 F Done card
       OR
   user 開 app（不點通知）→ History 區頂部標「上次有未完成 session」prompt（per wireframe FSM NG4）
```

**Retry policy**：
- WorkManager 預設 `BackoffPolicy.EXPONENTIAL` initial 10s，本場景 override 為 `BackoffPolicy.LINEAR` initial 30s（避免快重試燒電）
- 最多重試 3 次；3 次失敗 → 通知欄顯「focus 完成但摘要失敗，可開 app 手動重跑」+ row 寫但 takeaway 留空 + `takeaway_model="(workmanager_failed)"`
- Expedited quota 用盡 → fallback `OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST` → 普通 job（無 Doze 豁免），user 下次充電解 Doze 才跑

**Best-effort 不保證**：MIUI / EMUI custom Doze 仍可能 defer 執行；user 若不主動充電 + 不開 app → 可能延遲到下次充電。文案 / 通知不對 user 承諾 SLA。

---

## MIUI 引導 dialog 互動（per SPEC-34 §30(F) G6）

僅在 `is_miui=true` + MeshNodeService 啟動失敗時跳（首次 Timer / Quick-tile / PTT 觸發 service 起不來）。

### Tap targets

| Target | 動作 |
|---|---|
| `自啟動` TextButton | `Intent("miui.intent.action.OP_AUTO_START")` → MIUI 安全中心對應頁；若 deep-link 失敗（MIUI 改 API）→ Snackbar「請手動進 安全中心 → 自啟動管理 → 開啟 Phantom Mesh」+ 顯示文字步驟卡片；haptic `KEYBOARD_PRESS` |
| `省電` TextButton | `Intent("miui.intent.action.POWER_HIDE_MODE_APP_LIST")` → 省電白名單頁；失敗 fallback 同上；haptic `KEYBOARD_PRESS` |
| `不再提示` TextButton | DataStore `miui_guide_dont_show_again=true`（per mockup §190）→ dialog dismiss + 永不再跳（除 settings 主動重設）；haptic `CLOCK_TICK` |
| dialog 外點擊 | 不可關閉（setCancelable(false) — 強制 user 處理）|
| **system back button** | 同「不再提示」（user 明確拒）|

### Failure paths

- MIUI deep-link `OP_AUTO_START` action 在新版 MIUI 改名 / 移除 → ActivityNotFoundException → 顯示文字步驟 fallback：
  ```
  1. 開「安全中心」app
  2. 點「應用管理」→「權限」→「自啟動管理」
  3. 找「Phantom Mesh」→ 開啟
  4. 回到本 app，再試一次
  ```
- user 點「不再提示」後 MIUI 仍殺 service：WorkManager Expedited Job 接管；通知顯「focus 完成」仍可達；user 體驗略降但不阻斷
- settings 主動重設「不再提示」：Settings → "重新顯示 MIUI 引導" button → DataStore reset → 下次 service 啟動失敗會再跳

---

## Quick Settings tile 互動（per SPEC-34 §146 G5）

`FocusQuickTile : TileService`，1 tap 啟 25min focus session。

### Tap target sequence

```
[user 下拉通知欄展 Quick Settings]
       │
       ▼
[tile state = inactive]   icon: mic outlined, label: "Phantom 焦點", subtitle: "1 tap 啟 25min"
       │
       ▼ user tap
       │
[TileService.onClick]
   - tile.state = Tile.STATE_ACTIVE
   - tile.icon = mic filled colorPrimary
   - tile.updateTile()
       │
       ▼
[broadcast Intent("dev.phantom.mesh.ACTION_START_FOCUS") → MeshNodeService]
   - Service 收 → startForeground(FOREGROUND_SERVICE_TYPE_MICROPHONE)
   - 起 25 min Timer mode session
       │
       ▼
[FG-service notification D 出現於通知欄頂部]
       │
       ▼ user 收 Quick Settings panel
       │
[user 看 home screen — 仍可繼續其他 app；session 在背景跑]
```

### 互動細節

- tile click → broadcast 延遲：≤ 100ms（Android system service 喚醒）；user 主觀感受瞬間
- tile subtitle update 頻率：每 1 min（不是 1 秒 — 省電 + tile API rate limit）；TileService.onStartListening 觸發時抓最新 elapsed
- **再 tap 啟用態 tile**：跳 app 到 C Recording screen（per SPEC-34 §146 G5）— **不直接停止**（防誤觸）
- **session 結束自動切回 inactive**：MeshNodeService finalize 完成 → broadcast `system.tile.focus_end` → FocusQuickTile 收 → tile.state = INACTIVE + tile.updateTile()
- haptic：tile click 由 OS 處理（無法控）；app 內 FocusActive screen 開時加 `CONFIRM` haptic 補回饋

### 失敗路徑

- B1 RECORD_AUDIO 未授權：tile click → broadcast → Service 起 → AudioRecord.read 失敗 → 通知顯「需要麥克風權限 — tap 設定」；tile 切回 inactive
- B2 POST_NOTIFICATIONS 拒：tile click 仍 work，但通知欄 D 不顯示（user 必須開 app 才看到 session UI）
- MIUI 場景：tile click → Service 起失敗 → 跳 MIUI 引導 dialog（先把 app 打開到 dialog）
- Android 13+ 嚴格 FGS launch restriction：tile click 屬 user-initiated 從 system UI 啟 → 符合 restriction，不會被擋（per wireframe §163）

---

## 通用 Empty / Maximum / Error — Android 互動補充

視覺 spec 全在 mockup；本檔記互動行為。

### Empty state（History tab in Capture，首次無 session）

- mono SVG illustration 192dp + `focus.empty.history` 文案 + `[前往 Focus]` FilledButton
- button tap → 切到 Capture tab Focus 子畫面（in-tab navigation 200ms slide）；ripple；haptic `KEYBOARD_PRESS`
- **空白（無語音）** state in F：見 §F Done 表

### Maximum state（custom duration 上限觸發）

- user 輸入超過 180 → TextField 失焦 + Material shake animation（10dp horizontal × 3 cycles × 80ms 共 240ms）+ haptic `HapticFeedbackConstants.REJECT`
- input clamp 回 180；顯 `focus.limit.max_duration_hint`（per mockup R8 補 key）
- 下限 5 min 同樣處理

### Error state（global Snackbar）

- 觸發時機：任何全螢幕級 error（FOCUS-001 perm denied / FOCUS-002 mic 不存在 / 上傳全失敗）
- Material Snackbar `LENGTH_LONG`（4s）OR user tap action / swipe dismiss；swipe 任一方向皆可 dismiss
- 切換無 haptic（避免高頻 error 連震）；多 error 排隊（Snackbar 自帶 queue）

---

## SUS（System Usability Scale）題目對齊

10 題 SUS，Android 預期分布（vs iOS hero 預期 65–80 範圍）：

| 題目（簡寫） | 預期評分 | 設計依據 / 風險 |
|---|---|---|
| 1. 想常用 | 3–5 | Quick Settings tile 1 tap 25min；風險：3 perm gate 流程 + MIUI 用戶 dialog 干擾感 |
| 2. 不必要的複雜 | 2–4 | bottom-nav 4 tab + Capture 內子畫面；風險：B2 degraded bar 多餘感 |
| 3. 容易使用 | 3–5 | Material 3 標準元件直白；風險：FG-service 通知概念 user 不熟 |
| 4. 需要技術支援 | 2–4 | MIUI user 必須懂自啟/省電；風險：deep-link 失敗 fallback 文字步驟長 |
| 5. 各功能整合度高 | 3–5 | 通知欄 / Quick-tile / app 三介面同 FSM；風險：tile / app 雙入口可能混淆 |
| 6. 太多不一致 | 1–3 | Material 3 統一 ripple；風險：MIUI 廠商換皮 user 看的不一定是 Material 3 |
| 7. 多數人能很快學會 | 3–5 | back 鍵符合 Android 直覺；風險：B1+B2 雙 perm 連 prompt 累 |
| 8. 使用不靈活 | 1–3 | 通知欄 stop / tile / app 三路徑可達；風險：custom duration 180 限制 |
| 9. 使用上有信心 | 3–5 | 通知欄常駐回饋；風險：app 被 MIUI 殺後 user 不確定 session 還在不在 |
| 10. 學了很多才能用 | 1–3 | 第一次按就會；MIUI 用戶要學一次 dialog |

**目標 SUS：65–78 範圍**（預期實測中位數 70，略低於 iOS 因 3 perm gate + MIUI 場景；< 65 視為 fail）。**若 < 65 優先檢討**：

1. 3 perm gate 連 prompt 是否太累（觀察 user 是否煩躁）
2. FG-service 通知 user 是否 grasp（觀察 stop button 使用率）
3. MIUI dialog 文字步驟 fallback 是否被讀完（觀察 deep-link 失敗時 user 行為）
4. WorkManager 接管路徑 user 是否信任（觀察 app 殺後重開反應）

---

## 開放問題（Android prototype 層面）

1. **Stop 是否加 confirm**：同 iOS hero — 不加（效率優先）；若 usability test 50%+ user 誤按 → 加 long-press（500ms hold）替代 confirm
2. **PTT haptic 強度**：press-down `KEYBOARD_PRESS` vs `CONFIRM`？目前 `KEYBOARD_PRESS`（≈ iOS light）— 連續多次 PTT 不煩
3. **FG-service notification stop button label**：「STOP」(英大寫) vs「停止」(繁中)？Material 規範 capital；但 user 第一語言繁中 → 提案「停止」（與 app 內 button 一致），等 Material 3 review 確認
4. **Quick-tile tap 再 tap 行為**：跳 app vs 直接停？目前跳 app（防誤觸 per SPEC-34）；若 usability test user 抱怨「tile 不能直接停」→ v0.7+ 加 long-press tile = stop
5. **WorkManager retry 3 次**：夠嗎？Doze 重啟可能讓單次 retry 失敗 → 3 次保守；若 telemetry 顯示 5%+ session 卡在 WorkManager 失敗 → 加到 5 次
6. **MIUI dialog 三 button vs 兩 button**：「自啟動 + 省電 + 不再提示」三選一不是邏輯互斥（user 可能想兩個都設）→ v0.7+ 改 checklist：user 一個個跳設定 → 回 app 標 ✓ → 全 ✓ 後 dismiss
7. **B2 degraded UI bar 是否同 session 不再顯示 vs 永久 dismiss**：目前同 session（per Android wireframe invariant）；若 user 抱怨「每次開 app 都看到很煩」→ 改 7 天 cool-down
8. **TalkBack focus order 跳 bottom-nav 還是先 hero content**：目前先 hero content（duration → PTT → Timer → bottom-nav）；A11y 顧問建議 bottom-nav 先（global nav 優先）— 等 SPEC-06 A11y review

---

## 易用性測試準備

### 7 個 user task — 涵蓋 6 大資料狀態 + Nielsen 5 + Android-specific

| # | Task | 測項 | 6-state 覆蓋 |
|---|---|---|---|
| 1 | **首次使用 + 3 perm gate**：「請開 Focus 並錄 25 分鐘」 | A flow + B1 + B2 + Idle Empty 觀察 | Empty（History tab 觸發 `focus.empty.history`）/ Loading（perm wait）/ Ideal |
| 2 | **Quick Settings tile**：「請從通知欄下拉，加入 Phantom 焦點 tile，然後 1 tap 啟動 focus」 | tile 加入 + 1 tap 啟動 + tile 倒數 | Ideal + Loading |
| 3 | **通知欄 stop**：「session 跑中請從通知欄停止（不要開 app）」 | FG-service notification stop action | Loading |
| 4 | **Interrupted**：「錄音中接到電話，掛掉後檢查錄音狀態」 | AUDIOFOCUS_LOSS + 通知文字切換 | Error（中斷態）|
| 5 | **B2 skip 路徑**：「在 perm prompt 拒絕通知權限，看看會發生什麼」 | degraded UI bar + 仍可錄 + 通知不顯示 | Partial（功能 degraded） |
| 6 | **system back + Done flow**：「看 takeaway 並用 back 鍵回 Idle」 | OnBackPressedDispatcher → React Router | Ideal |
| 7 | **Maximum + Partial**：「設定 180 min 自訂後立即停；假設第 3 chunk ASR 失敗請從 F 卡讀懂發生什麼」 | duration clamp + partial inline | Maximum / Partial / Error |

### Sampling

- 目標 5–7 user（Nielsen「5 個 user 找 80% 問題」）
- 角色：3 stock Android（Pixel 7 / Pixel 6a）+ 2 MIUI（Redmi Note 12 / Xiaomi 13）+ 2 OEM custom（Samsung S22 / OPPO Find X5）— 涵蓋 Android 生態系
- 環境：手持 Android 13+ 各跑一次；其中 1 個 MIUI user 跑「強殺 + WorkManager 接管」場景

### 觀察重點

- **screen A**：bottom-nav 4 tab user 是否找得到 Capture？PTT/Timer 互斥規則是否引起不確定？
- **screen B1+B2**：連兩 perm prompt user 是否煩躁？拒絕 B2 後 user 是否注意到 degraded UI bar？
- **screen C + D**：通知欄 D 是否被 user 注意？stop button 是否被誤觸（按到通知欄滑動而誤觸）？
- **screen E**：通知欄文字 update 是否被 user 觀察到（會降低焦慮）？
- **screen F**：empty 變體（無語音）user 是否理解「重錄 vs 完成」？
- **MIUI task**：dialog 三 button user 是否懂哪個先點？fallback 文字步驟是否被讀完？
- **Quick-tile task**：user 是否找得到「加入 tile」入口？第一次點 tile 是否預期 1 tap 就啟？
- **WorkManager task（高階）**：app 殺後重開 user 是否信任「session 還在跑」？

### 紀錄方式

- 螢幕錄影 + 手機側拍（看 user 拇指動作 + 通知欄互動）
- think-aloud protocol（user 邊操作邊說）
- 結束後 SUS 問卷 + 5 題開放問題（最讚 / 最差 / 困惑點 / 期望加 / 期望砍）+ Android 特定問題：「如何停止 session」（觀察 user 答 app 內 / 通知欄 / tile 三條路徑哪條優先想到）

---

## 下一步

→ 拉 5–7 Android user 跑 usability test（含 ≥ 1 MIUI user）→ 收 SUS 分數 + 觀察紀錄 → 回頭修 Android Wireframe / Mockup / Prototype 對應點
→ Android prototype 經 usability 驗證 OK 後，再橫向把 Wireframe / Mockup / Prototype 三層補 macOS / Windows / Linux / Web（4 剩餘平台）
→ 6 大資料狀態 + Nielsen 5 + SUS 對齊套到其他 4 平台不一定要全寫滿，可只列差異（同本檔 vs iOS hero 的處理模式）
