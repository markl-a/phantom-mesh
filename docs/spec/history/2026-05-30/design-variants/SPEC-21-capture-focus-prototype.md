# SPEC-21 Capture Focus — Prototype（原型）· iOS

> **Stage 3/3** of the user-flow chain · [線框稿（Wireframe）](./SPEC-21-capture-focus-wireframe.md) → [視覺稿（Mockup）](./SPEC-21-capture-focus-mockup.md) → 原型（Prototype）
> **Status**: draft v0.3 **(sign-off after R4)**（R0 ship → R1 sync v0.3 wireframe/mockup：9-state FSM + i18n keys + 拔 token 越界 + F Done 800 字 + PTT/Timer 反向互斥 + 7 task + SUS 範圍化 → R2 6 fix：F CTA 行為對齊 mockup + 30s 統一 + 7 hardcoded i18n + Empty task 覆蓋 → R3 5 P1：7 new i18n keys (confirm/disk_full/no_takeaway/etc) + spinner 12pt 拔 + Back-confirm i18n + 30s reference 貫徹 → R4 sign-off：4/5 reviewer 通過，2 doc nit 順手清）· **Last updated**: 2026-05-27
> **Scope**: iOS only（hero 平台；其他 5 平台 prototype 後續迭代）
> **同步狀態**：對齊 wireframe v0.3 (374 行) + mockup v0.3 sign-off (581 行 — R2/R3 補 13 個新 i18n keys)。互動 timing / 手勢 / haptic / 失敗路徑歸本檔；視覺 token / 字級 / 色彩 / 元件尺寸歸 mockup；佈局 / 螢幕結構 / FSM 規格歸 wireframe — 三檔嚴格分層
> **Spec**: [`SPEC-21-SYSTEM-capture-focus`](../specs/v060-deep-spec/SPEC-21-SYSTEM-capture-focus.md) · [`SPEC-31-PLATFORM-iOS-screens`](../specs/v060-deep-spec/SPEC-31-PLATFORM-iOS-screens.md)
> **這份的工作範圍**：把 Mockup 變「可操作」 — 每個 tap target 點下去發生什麼、動畫 / haptic / timing、6 大資料狀態（理想 / 空白 / 極限 / 錯誤 / 局部 / 載入中）、失敗如何重試。為易用性測試（usability test）準備 walkthrough script + SUS 題目對齊。
> **參考**：
> - [iThome Day6 介面設計流程](https://ithelp.ithome.com.tw/articles/10295775) — Wireframe → Mockup → Prototype 三段定義
> - [Tom Liou 給超超超新手的 UI/UX 指南](https://medium.com/@tomliou/給超超超新手的uiux指南-96c80687a20f) — 7 段式流程 + 6 大資料狀態 + Nielsen 5 易用性原則

## 為什麼這份要寫深

Prototype 是「使用者能親自操作、來獲取貼近上線產品的回饋」（iThome 文）。本檔用 markdown 描述互動，等同把可點擊原型「腳本化」 — 工程師讀完直接知道怎麼接 React + Tauri，QA 讀完知道每條 path 怎麼測。Medium 文強調的 **6 大資料狀態** 跟 **Nielsen 5 易用性原則** 是本檔的兩條品檢線。

## Nielsen 5 易用性檢核（總攬，每個 screen 再對照一次）

| 原則 | iOS Focus 表現方式 |
|---|---|
| **可學習性（Learnability）** | 首次開 Idle 螢幕，無 onboarding 即可看懂 PTT 跟 Timer 兩條路徑；trust badge 解釋資料去處 |
| **效率性（Efficiency）** | PTT 按壓 ≤ 100ms 啟動；Timer 預設 25 min 一鍵開始；鎖屏可控（不必解鎖） |
| **記憶性（Memorability）** | 第二次回來 Idle 螢幕跟首次一樣（無模式記憶）；按鈕位置不變；上次 duration 預選 |
| **失誤性（Errors）** | 權限拒絕 → 明確提示 + 設定 deep link；ASR 失敗 → 文字提示 + 重跑按鈕；不靜默失敗 |
| **成就感（Satisfaction）** | Done 卡 takeaway 即時可看；haptic success；session count 累積看得到 |

## 6 大資料狀態速查

每個螢幕都該明確覆蓋以下狀態（Medium 文點名的 UI 設計師核心工作）：

| 狀態 | 何時出現 | i18n key 對應（per mockup v0.3）|
|---|---|---|
| **理想（Ideal）** | 完整資料、所有元素正常 | `focus.done.title` / `focus.done.view_full` |
| **空白（Empty）** | 首次使用 / 無歷史紀錄 | `focus.empty.history`（具體 frame 在平台 catalog，per OoS3） |
| **極限（Maximum）** | 資料量 / 字數 / 時長 達極限 | `focus.limit.chunk_overflow` / `focus.limit.takeaway_truncated_hint` / `focus.limit.view_full_takeaway` |
| **錯誤（Error）** | 權限拒絕 / 網路斷 / ASR 全掛 | `focus.perm.denied` / `focus.perm.denied_reassure` / `focus.perm.open_settings` / `focus.web.upload_failed` |
| **局部（Partial）** | 部分資料載入 / 部分 chunk ASR 失敗 | `focus.partial.chunk_failed` |
| **載入中（Loading）** | 等麥克風暖機 / 等 ASR / 等 LLM | `focus.finalizing.asr` / `focus.finalizing.llm` |

## 9-state FSM 主骨架（per wireframe v0.3）

Prototype 互動以 wireframe 9-state FSM 為主骨：**7 主鏈 + 2 sub-state**

```
Idle → Requesting(perm) → Recording ──┬→ Chunking      (sub-state, 自迴圈)
                                       └→ Interrupted   (sub-state, OS 觸發)
       → Finalizing → Transcribing → SummaryGen → Done
```

每個 state 的互動職責對應到本檔螢幕：

| FSM state | 對應螢幕 | 互動重點 |
|---|---|---|
| `Idle` | A | duration 選擇、PTT/Timer 互斥、haptic selection |
| `Requesting` | A → B 中間 | spinner（尺寸 token per mockup）、OS prompt 出現前 micro-state |
| `Recording` | C | waveform refresh、計時器更新、暫停/停止互動 |
| `Chunking` (sub) | C 內微 state | chunk +1 flash toast（1.5s hold），每 5min 或 PTT release 觸發 |
| `Interrupted` (sub) | C' 變體 | waveform 凍結、AVAudioSession callback、寬限與超時數字 per wireframe FSM |
| `Finalizing` | E | 兩段 phase 切換（ASR → LLM），取消可達 |
| `Transcribing` | E phase 1 | progress 0→100%、Partial inline 提示（per chunk fail）|
| `SummaryGen` | E phase 2 | spinner 持續、pending 訊息變色 |
| `Done` | F | success haptic + scale spring + takeaway card 展開 |

---

## 螢幕 A — Idle（焦點時段預備）

### Nielsen 5 對應

- Learnability：首次進來「按住說話」字面直白 → 試按就懂；25/50/Custom 三檔 + 「開始計時錄音」按鈕清楚分流
- Efficiency：Timer 預設 25 min selected；按 Timer button 直接開（不必先選）
- Memorability：上次使用的 duration 預選（store `last_duration_min` in `@AppStorage`）
- Errors：首次按下會撞權限 prompt，不會錯到「按了好像沒事」
- Satisfaction：頂部 trust badge 給「我懂這 app 不偷我音訊」的安心

### 6 大資料狀態

| 狀態 | UI 表現 |
|---|---|
| 理想 | duration picker 顯示三檔；PTT 按鈕粗框可按；trust badge 顯示 |
| 空白 | 同理想（Idle 沒有「無資料」一說）；如果想顯示「上次 focus 是 _ 分鐘前」這種 nudge，在 v0.7+ 加 |
| 極限 | custom duration 上限 180 min（超過 disable input + 顯示 `focus.limit.max_duration_hint`）；下限 5 min |
| 錯誤 | 麥克風硬體不存在 → PTT + Timer 按鈕 disabled + 顯示 `focus.err.no_mic`（FOCUS-002） |
| 局部 | n/a（Idle 沒有 partial concept） |
| 載入中 | n/a（Idle 不該需要載入） |

### Tap targets（按下去發生什麼）

| Target | 動作 |
|---|---|
| `< 返回` | pop view controller，回上一層；無 confirm（Idle 無未儲存資料） |
| `⚙ 設定` | push `FocusSettingsView`（duration limit、ASR provider opt-in、cloud fallback 開關） |
| `25` pill | 即時 select；haptic `selection`（UISelectionFeedbackGenerator）；其他兩 pill deselect；clock-face dim text "25:00" 更新 |
| `50` pill | 同上，更新 "50:00" |
| `自訂` pill | 同上，且彈出 inline number stepper（5–180）；haptic `selection` |
| **PTT 按鈕 — press-down** (label `focus.btn.ptt`) | (1) 若未授權麥克風 → 走 Perm Prompt（screen B）；(2) 若已授權 → 0ms 視覺 press（press 樣式由 mockup `overlay-disabled-40` 反向定義）+ haptic `light` (UIImpactFeedbackGenerator.light)；100ms 內 AudioRecorder.open + isRecording=true；切到 Recording screen（PTT mode）。**同時 Timer 按鈕進入 disabled state**（per mockup invariant），避免雙模式同時觸發 |
| **PTT 按鈕 — press-up** | 立即 close chunk + push to EventStore；haptic `light`；UI flash 'chunk +1'（Chunking sub-state 視覺對應）；**留在 Idle screen** 不切到 Recording（PTT 是一次按一段，多次累積）；Timer 按鈕回 enabled |
| **Timer 開始按鈕** | (1) 若未授權麥克風 → 走 Perm Prompt；(2) 已授權 → haptic `medium`；切到 Recording screen（Timer mode）；started_at = now，計時器開跑。**反向互斥**：切到 Recording 後 PTT 按鈕從版面消失（不需 disabled，畫面已切換），「Timer 跑中 PTT disable」是邏輯保證、視覺上無從顯示 |
| trust badge 文字 | tap → push `TrustExplainerView`（一頁說明：本地加密 / 本地 ASR / 不上雲 LLM）— 文案取 `focus.trust_badge` 一字不差 |

### Animations / Timings

- Pill 切換：背景色 transition 200ms ease-out（具體色 token 在 mockup）
- PTT button press visual：8ms（≈ 1 frame）回饋；放開回彈 80ms ease-out
- Screen transition Idle → Recording (Timer mode)：iOS 預設 push transition 350ms
- Trust badge tap：開啟新 view 用 modal sheet（不是 push）
- PTT press-down 觸發 Timer disabled 同步 0ms（無動畫，避免 jank）

### Failure paths

- 首次按 PTT 觸發 iOS permission prompt：app process 不能控制 prompt 出現時間（OS 排程）— 可能 100–800ms 後才彈。我們在 prompt 出現前先到 "Requesting" 微 state（PTT button 顯示細 spinner，具體尺寸 token 由 mockup 定）；prompt 出現後若 denied → 切回 Idle + 全螢幕 toast `focus.perm.denied` + deep link
- 麥克風被佔用（其他 app 在錄）：tap PTT → FOCUS-002 toast + 不切螢幕

### Walkthrough script（usability test：「請開始一段 25 分鐘的專注錄音」）

1. 預期 user 看到 Idle 螢幕，自然點 "25 分鐘"（已是預設）→ "開始計時錄音"
2. 首次：撞權限 prompt，期待 user 點 Allow
3. 切到 Recording 螢幕
- **觀察點**：user 是否注意到 PTT 大按鈕並誤按？若 50% 以上 user 試 PTT，要重考慮 Idle 螢幕雙 mode 並存的設計

---

## 螢幕 B — Perm Prompt（iOS system dialog）

不可自訂。spectyn 唯一能控的是 `Info.plist` 的 `NSMicrophoneUsageDescription` 文字：

```
NSMicrophoneUsageDescription = "錄製焦點時段音訊；音訊只儲存在本機並加密。"
```

### 6 大資料狀態

| 狀態 | UI 表現 |
|---|---|
| 理想 | OS 渲染 prompt；user tap Allow |
| 錯誤 | user tap Don't Allow → 後續所有 PTT/Timer tap 都直接導向 Settings deep link |

### Tap targets

| Target | 動作 |
|---|---|
| `Don't Allow` | OS 記下 denied；app 收到 callback `AVAudioSession.recordPermission == .denied`；切到 Denied 狀態（畫面 Idle + 全螢幕半透明遮罩 + 卡片「需要麥克風」） |
| `Allow` | OS 記下 granted；app 進 Recording flow（從觸發點繼續：PTT 或 Timer） |

### Denied 卡片互動（覆蓋在 Idle 上 — 視覺 spec 在 mockup B' frame）

- 卡片版面 / 色 token 完全交給 mockup B' frame（見 mockup `overlay-denied-72`）
- Prototype 只定**文案 key 與互動**：
  - 標題：取 `focus.perm.denied`（需要麥克風才能 focus 錄音 / Microphone permission required）
  - 安撫文：取 `focus.perm.denied_reassure`（我們不會上傳音訊到雲端…）
  - 按鈕：取 `focus.perm.open_settings`（打開設定 / Open settings）
- Tap 「打開設定」→ `UIApplication.shared.open(URL(string: UIApplication.openSettingsURLString)!)`；無 confirm；無 haptic（OS 自處理 transition）
- 覆蓋出現時：Idle 底下 PTT + Timer 同時套 mockup `overlay-disabled-40`，與 PTT/Timer 互斥規則同一機制

---

## 螢幕 C — Recording（Timer mode）

### Nielsen 5 對應

- Learnability：計時器大數字一目了然；停止按鈕高警示色（具體 token 在 mockup）直白
- Efficiency：暫停 + 停止兩顆即可；鎖屏可控（出門用得到）
- Memorability：版面結構跟 Idle 對得起來（trust badge 同位置）
- Errors：誤按停止 → confirm? **不**加（usability：confirm 會打斷流程；停止後仍有 takeaway 可看）；改在 done 後加 "delete this session" 救濟
- Satisfaction：waveform 即時跳動 → 即時回饋「真的在錄」；chunk 計數累積有累積感

### 6 大資料狀態

| 狀態 | UI 表現 |
|---|---|
| 理想 | 計時器在跑；waveform 跟麥克風輸入連動；chunk count 每 5min +1（觸發 Chunking sub-state） |
| 空白 | n/a（Recording 必有 audio stream） |
| 極限 | 50 min / custom max 180 min 到 → 自動 stop；計時器顯示 "50:00 / 50:00" 警示閃 1s 後切 Finalizing。**chunk count 99+ 變體**：chunk ≥ 100 顯示 `99+` 取 `focus.limit.chunk_overflow`（數字區塊 min-width 鎖死，per mockup invariant，prototype 無轉場動畫只純態切換）|
| 錯誤 | OS interrupt（電話來 / mic 被搶 / 系統 sleep） → 切「Interrupted」 sub-state（C' 變體，視覺 spec 在 mockup）；文案取 `focus.interrupted.phone` 或 `focus.interrupted.mic_grabbed`（依 interrupt 來源）+ `focus.interrupted.resume_hint`；寬限 / 超時數字 per wireframe FSM |
| 局部 | n/a（per chunk 失敗在 Finalizing 顯示，見 E）|
| 載入中 | n/a（Recording 不該需要載入） |

### Tap targets

| Target | 動作 |
|---|---|
| `⏸ 暫停` | label 取 `focus.btn.pause`。(1) AVAudioSession 設 inactive；(2) waveform 凍結；(3) 計時暫停（每秒 1Hz 閃爍，具體色 token 在 mockup）；(4) 按鈕變 `▶ 繼續` (`focus.btn.resume`)；haptic `medium` |
| `▶ 繼續` | label 取 `focus.btn.resume`。反向：session 重啟、waveform 重跑、計時繼續、按鈕變回 `⏸ 暫停`；haptic `medium` |
| `⏹ 停止` | label 取 `focus.btn.stop`（mobile 用短版，不用 desktop `focus.btn.stop_finalize`）。立即 transition 到 Finalizing screen（不加 confirm dialog）；haptic `heavy`；AudioRecorder.close；flush 殘留 chunk → 觸發 Chunking → Finalizing 鏈 |
| chunk count `已落地 chunk: {n}` | 顯示用變量 `{n}`；99 → 100 切到 `99+` (`focus.limit.chunk_overflow`)；tap 無動作（純資訊）；v0.7+ 可 tap 跳 history |
| trust badge | tap 同 Idle |

### Background / Lock-screen 行為

- 進背景（按 home / lock）：AVAudioSession 已宣告 `.playAndRecord` + `UIBackgroundModes audio` → 系統允許繼續錄；不切 UI（不要 backgroundTaskIdentifier hack）
- 鎖屏：`MPNowPlayingInfoCenter` 自動顯示控制；點 `pause` / `stop` 觸發 `MPRemoteCommandCenter` callback → 等同 app 內按
- 電話 interrupt：`AVAudioSessionInterruptionNotification` → 進 Interrupted state；UI 顯示 `focus.interrupted.phone` + `focus.interrupted.resume_hint`；OS resume 自動回 Recording / 超時走 Finalizing — **寬限與超時數字 per wireframe FSM**（不重述）

### Animations / Timings

- waveform refresh：60fps；audio buffer 1024 samples 為一禎（柱數 / 高度 / 色 token 在 mockup）
- 計時器數字：每秒 update，不加 animation
- pause/resume icon morph：200ms ease-in-out
- **chunk +1 flash**（Chunking sub-state 視覺對應）：toast bottom-up enter 100ms / hold 1.5s / exit 200ms
- stop → Finalizing：螢幕 cross-fade 250ms（不用 push transition，因 finalize 是「結束」感）
- **PTT/Timer 互斥切換**：press-down 同 frame disable 對方（0ms）；press-up 同 frame re-enable

### Failure paths

- 麥克風被搶（用戶開另一個錄音 app）：`AVAudioSessionInterruptionNotification .began` → 進 Interrupted；超出寬限未恢復 → 強制 finalize + 標 `interrupted=true`（**寬限與超時數字 per wireframe FSM Interrupted sub-state**，本檔不重述）
- 儲存空間滿（chunk encrypt 失敗 → disk full）：toast `focus.err.disk_full` + 切 Finalizing（保留已落地 chunk）
- App 進背景 5 分鐘剛好 chunk close 觸發 → 走背景 chunk write 路徑（`URLSessionDataTask` 在 BG 不可用 → 用 dispatch queue + file write，已測 iOS 18 OK）

### Walkthrough script（usability test：「開始錄音 30 秒、暫停、繼續、停止」）

1. 從 Idle → 25 min Timer 按下 → Recording 螢幕
2. **觀察點 1**：user 是否意識到「進來了 = 在錄」？若 30% 以上 user 反問「我有在錄嗎」，要強化 recording 視覺指示（具體強化方式請 mockup 提案 — prototype 只記指標）
3. user 按暫停 → 期待 waveform 凍結
4. **觀察點 2**：user 是否認得 `▶ 繼續` 按鈕？icon 改 play 是否清楚？
5. user 按繼續 → user 按停止 → 切 Finalizing
6. **觀察點 3**：user 是否預期 stop 後資料還在？若 user 有「不會被刪掉吧」焦慮，要在 stop 後 toast「已儲存」

---

## 螢幕 D — Lock-screen（iOS MPNowPlayingInfoCenter）

OS 渲染，spectyn 無法控版面。只能設：

```swift
MPNowPlayingInfoCenter.default().nowPlayingInfo = [
  MPMediaItemPropertyTitle: "Spectyn Mesh",
  MPMediaItemPropertyArtist: "Focus · \(elapsed_mm_ss)",
  MPMediaItemPropertyArtwork: MPMediaItemArtwork(boundsSize: CGSize(width: 512, height: 512)) { _ in
    UIImage(named: "spectyn-icon-mono-512")!
  },
  MPNowPlayingInfoPropertyPlaybackRate: isPaused ? 0.0 : 1.0,
]
```

並註冊 remote command targets：
```swift
let center = MPRemoteCommandCenter.shared()
center.pauseCommand.addTarget { _ in handlePause(); return .success }
center.playCommand.addTarget { _ in handleResume(); return .success }
center.stopCommand.addTarget { _ in handleStop(); return .success }
```

### Tap targets（lock-screen）

| Target | 動作 |
|---|---|
| `pause` icon | 等同 app 內 `⏸ 暫停`（透過 remote command） |
| `play` icon（暫停狀態） | 等同 `▶ 繼續` |
| `stop` icon | 等同 `⏹ 停止`；觸發 finalize；haptic 不可控（OS 自己處理） |
| 唱片封面 tap | 解鎖跳回 spectyn Recording screen |

### 失敗路徑

- iOS 18 lock-screen 對 `MPNowPlayingInfoCenter` 限制收緊：若 30 min 內無 user 互動，OS 可能 evict info → lock-screen 控制消失（app 仍在錄）；user 解鎖回 spectyn 仍見計時器
- AirPods 中鍵 pause → 觸發 `pauseCommand` → 等同 app 內按

---

## 螢幕 E — Finalizing（過渡）

### Nielsen 5 對應

- Learnability：訊息字面直白 `focus.finalizing.asr`「整理逐字稿 (2/5)」+ `focus.finalizing.llm`「產生 takeaway 中…」
- Efficiency：兩段（Transcribing → SummaryGen FSM state 切換）progress，視覺等待時間 ≈ 實際處理時間
- Memorability：載入畫面延續 Recording 色系（具體 token 在 mockup E frame）
- Errors：兩條 path 各有失敗訊息 + 重試按鈕；FSM state 失敗時 inline 顯示，不阻斷主流程
- Satisfaction：progress 真實反映（不偽造）；長度 ≤ 25 min audio 預期 < 90s 看到 takeaway

### 6 大資料狀態（這個螢幕本身就是「載入中」，視覺 spec 在 mockup E frame）

| 狀態 | 互動行為（不寫 UI token） |
|---|---|
| 理想 | Transcribing phase progress 0→100% → SummaryGen phase spinner 持續 |
| 載入中 | 同上（本螢幕的常態），第二行 pending 訊息切色由 mockup 定 |
| 局部 | 部分 chunk ASR 失敗 → inline 顯示 `focus.partial.chunk_failed`（per chunk）→ 繼續 stitch 其他 chunks，progress 跳過失敗 chunk |
| 錯誤 | 所有 chunk ASR 都掛 → 顯示 FOCUS-003 + `重試 ASR` `先用空白 transcript 跑 LLM` 兩按鈕 |
| 極限 | 50min audio + 10 chunks → 預期 ASR ≤ 80s（whisper.cpp small on M-series）；超過 120s 顯示 `focus.finalizing.taking_longer` 訊息 |

### Tap targets

| Target | 動作 |
|---|---|
| `取消並先看逐字稿` (`focus.btn.cancel_show_transcript`) | 中斷 LLM call；用 stitched transcript 直接寫 focus row（takeaway 留空字串 + `takeaway_model="(skipped)"`）；切 Done screen。**行為由本檔 prototype 鎖**（per mockup E 註）|
| `重試 ASR` (`focus.btn.retry_asr`) | 錯誤態出現。重跑所有失敗 chunk 的 ASR；若 user 在 settings 開了 cloud fallback，用 Groq Whisper |
| `先用空白 transcript 跑 LLM` (`focus.btn.use_empty_transcript`) | 全 ASR 掛時錯誤態出現。LLM 跑空字串 → takeaway = "(無 audio 可分析)" → 寫 row（保留 audio 以後可 re-asr） |
| 螢幕本身 swipe-down | 不可關（modal-style，避免誤觸） |

### Timings

- ASR 各 chunk：on-device whisper small 1min audio ≈ 6–8s on M3；5 chunks 序列 ≈ 30–40s（平行版 v0.7+）
- LLM takeaway：Groq llama-3.1-70b ~ 2–4s for 5000-token transcript
- spinner 滾速：常數 60°/s 不變速（不要假裝快）

### Failure paths

- 所有 ASR 都掛：顯示 FOCUS-003 + 兩按鈕；若 user 選「先用空白 transcript」→ LLM 仍跑（拿空字串）→ takeaway = "(無 audio 可分析)" → 寫 row（保留 audio 以後可 re-asr）
- LLM 失敗：FOCUS-004 + 留 transcript + takeaway = "(摘要失敗，可手動重跑)" + 寫 row
- 取消 LLM：transcript 仍 stitch、row 仍寫；takeaway 為空字串 + `spectyn focus reasr <id>` 可後補

---

## 螢幕 F — Done（Takeaway card）

### Nielsen 5 對應

- Learnability：顯示 session metadata（時長 / chunks）讓 user 確認；卡片結構直白
- Efficiency：兩顆按鈕 ≤ 1 tap 到下一步（看逐字稿 / 新 session）
- Memorability：takeaway 卡片版式跟 history list item 一致（學一次到處用）
- Errors：n/a（這是 success state）
- Satisfaction：success icon + haptic `success`（UINotificationFeedbackGenerator.success）；25 min 投入有可見產出

### 6 大資料狀態

| 狀態 | UI 表現 |
|---|---|
| 理想 | 完整 takeaway 三段：主要 ideas / action items / 情緒卡點 |
| 空白 | takeaway 是空字串（user 取消 LLM 或 LLM 全掛）→ 卡片顯示 `focus.empty.no_takeaway` + `重跑摘要` 按鈕 (`focus.btn.retry_summary`) |
| 極限 | takeaway 觸發 truncation（**字數閾值 / 卡片 max-height / fade gradient 高度全 per mockup F Limit invariant**，本檔不重述數字避免 drift）+ inline hint `focus.limit.takeaway_truncated_hint`（「（摘要過長已截斷）」）+ CTA `focus.limit.view_full_takeaway`（「看完整摘要」） |
| 錯誤 | 全 ASR 都掛 → takeaway = "(無 audio 可分析)" → 顯示 `focus.err.no_takeaway` + `重跑 ASR` 按鈕 (`focus.btn.retry_asr`) |
| 局部 | 5 chunks 中 1 個 ASR 失敗 → 卡片頂部 banner 引 `focus.partial.chunk_failed`（「轉文字失敗 (chunk {i}/{n})，已跳過」）|
| 載入中 | n/a（從 Finalizing 進來時資料已備齊） |

### Tap targets

| Target | 動作 |
|---|---|
| `看完整逐字稿` | label 取 `focus.done.view_full`。push `TranscriptView`，顯示 stitched transcript + 每段 chunk 時間軸 + 編輯按鈕 |
| `新 session` | label 取 `focus.done.new_session`。pop 整 stack 回 Idle；不保留 takeaway preview（已存 events） |
| takeaway card（truncated state）| tap 整張 / inline `focus.limit.view_full_takeaway` CTA → **等同點 `[看完整逐字稿]` 主按鈕**（push `TranscriptView`），與 mockup F Limit invariant 一致；**不在原地展開卡片**（避免雙路徑混淆）|
| nav bar `< 完成` | 同「新 session」（pop 回 Idle） |
| 螢幕 swipe-down | 同「新 session」 |

### Entry animation

- 從 Finalizing 進入：cross-fade 250ms + success icon 從 0→1 scale spring (damping 0.6, response 0.4)；haptic `success`（雙短震）
- takeaway card 從 cardOrigin 起 fade-in + slight slide-up 12pt 持續 350ms ease-out

### Failure paths

- v0.7+ 加「分享」按鈕的話會撞 takeaway 含敏感資訊問題 → v0.6.0 先不加分享，避免 user 不小心 share 走

### Walkthrough script（usability test：「請看完 takeaway 並開新 session」）

1. user 看到卡片 → 預期 user 讀 takeaway
2. **觀察點 1**：user 是否能在 3 秒內 grasp takeaway 結構（三段 vs 平鋪）？若 50% 以上 user 看不出結構，重做 takeaway prompt
3. user 點「新 session」期待回 Idle
4. **觀察點 2**：user 是否預期 takeaway 已保存？若 30% 以上 user 焦慮「會不會就丟了」，要在 done screen 加微妙「已自動儲存」hint

---

## 跨螢幕互動

### Interruption Flow（電話進來）

```
Recording ──[AVAudioSession Interrupt .began]──> Interrupted sub-state
              ↓
   waveform 灰、計時暫停、顯示 toast `focus.interrupted.phone` +
   `focus.interrupted.resume_hint`（樣式 token 在 mockup C' frame）
              ↓
   [.ended received within wireframe FSM 寬限]
              ↓
   AVAudioSession 重啟 → Recording 繼續
              ↓
   [.ended NOT received within 寬限 — 超時數字 per wireframe FSM]
              ↓
   強制 finalize（interrupted=true 標記）→ Finalizing screen
```

### Back-button / Swipe-back 行為

| 螢幕 | back / swipe |
|---|---|
| Idle | pop view controller 回上層；無 confirm |
| Recording | swipe-back disabled（用 `interactivePopGestureRecognizer.delegate` 攔截）；按 `< Back` 顯示 confirm dialog `focus.confirm.leave_recording_msg` + `focus.btn.cancel` / `focus.confirm.leave_recording_stop` |
| Finalizing | swipe-back disabled；無 nav back 按鈕；唯一出口是「取消並先看逐字稿」 |
| Done | swipe-back enabled = pop 整 stack 回 Idle（等同「新 session」） |

### App 進背景 / 回前景

- Idle → 背景：無事
- Recording → 背景：UIBackgroundModes audio kept；no UI change（鎖屏由 MPNowPlayingInfoCenter 接管）
- Finalizing → 背景：背景 task 用 `URLSession backgroundConfiguration` 不適用（whisper 本地跑） → 用 `beginBackgroundTask`，給 30s（夠 ASR 1–2 chunk）；超時就 partial finalize（剩 chunks 標 `pending`，user 回前景可選 re-asr）
- Done → 背景：無事；回前景仍顯示同 takeaway

---

## 通用 Empty / Maximum / Error — 互動補充（視覺 spec 全在 mockup）

> **越界修正（R1）**：本節原版含 ASCII frame + token (`spectyn-muted` / `spectyn-danger` / `192×192pt SVG` 等) 屬 mockup 範圍，全部移除。Prototype 只記**互動行為**，視覺請見 mockup「6 大資料狀態 — Mockup 視覺對映表」。

### Empty state（History tab 首次進入）

- 文案取 `focus.empty.history`（「還沒有 focus session — 開始第一段就會顯示在這」）
- 按鈕 label 取 `focus.empty.go_to_focus`（「前往 Focus」）；tap → push Focus tab；pop transition 350ms iOS 預設；無 haptic
- **註**：History tab ASCII frame 由 SPEC-31 iOS catalog 提供（per wireframe OoS3）

### Maximum state（custom duration 上限觸發）

- user 輸入超過 180 → input 失焦 + 短 shake animation（10pt horizontal × 3 cycles × 80ms 共 240ms）+ haptic `warning`
- input 強制 clamp 回 180；顯示 `focus.limit.max_duration_hint` hint（mockup R8 已補 key）
- 下限 5 min 同樣處理（往下 clamp）

### Error state（global toast，由 mockup invariant 定 visual）

- 觸發時機：任何全螢幕級 error（FOCUS-001 perm denied / FOCUS-002 mic 不存在 / 上傳全失敗）
- Auto-dismiss after 6s OR user taps action / swipes outside（user 互動行為由本檔鎖；toast 視覺由 mockup invariant 鎖）
- 切換時無 haptic（避免高頻 error 連震）；多個 error 排隊顯示，不疊加

---

## SUS（System Usability Scale）題目對齊

10 題 SUS，本 feature 對應預期分布：

| 題目（簡寫） | 預期評分（5-point） | 設計依據 / 風險 |
|---|---|---|
| 1. 想常用 | 3–5 | PTT 流暢 + 鎖屏可控；風險：權限拒絕情境會壓低 |
| 2. 不必要的複雜 | 1–3 | 兩種 mode + 三檔 duration 已是最少；風險：PTT/Timer 並存可能被 user 誤認複雜 |
| 3. 容易使用 | 3–5 | 大按鈕 + 直白文案；風險：first-time perm prompt 中斷感 |
| 4. 需要技術支援 | 1–3 | 自說明 trust badge + 設定 deep link；風險：Bluetooth 耳機切換場景未覆蓋 |
| 5. 各功能整合度高 | 3–5 | Idle / Recording / Done 版式呼應 |
| 6. 太多不一致 | 1–3 | trust badge / buttons 位置固定；風險：PTT/Timer 互斥規則 user 不一定 grasp |
| 7. 多數人能很快學會 | 3–5 | 無 onboarding 即可開始；風險：Finalizing 等待 user 不確定在等什麼 |
| 8. 使用不靈活 | 1–3 | PTT + Timer 兩 mode 並存；風險：custom duration 上限 180 限制 |
| 9. 使用上有信心 | 3–5 | 即時 waveform + chunk count 回饋；風險：背景錄音 user 看不到狀態 |
| 10. 學了很多才能用 | 1–3 | 第一次按 PTT 就會 |

**目標 SUS：65–80 範圍**（預期實測中位數 72，「OK→good」段；< 68 視為 fail per Medium 文標準）。**若 < 68，優先檢討**：
1. Finalizing 等待時長 + 文案（觀察 user 焦慮）
2. PTT/Timer 並存設計（觀察混淆率）
3. 權限拒絕後恢復路徑（觀察是否真的有 user 重開設定）

---

## 開放問題（prototype 層面）

1. **Stop 是否加 confirm**：目前不加（效率優先），但若 usability test 50%+ user 誤按 → 加 long-press（500ms hold）替代 confirm。
2. **PTT haptic 強度**：press-down `light` vs `medium`？目前 `light` — 太弱 user 不確定觸發。可上 `medium` 但連續多次 PTT 會煩。
3. **chunk count 點擊**：v0.6.0 不可點（純資訊） vs v0.7+ 跳 history。要不要在 v0.6.0 就放 hint「v0.7 可看 chunk 列表」？傾向不放（避免假承諾）。
4. **Finalizing「取消並先看逐字稿」**：是否會被誤觸？目前是 caption 字級（小且 muted），降低誤觸風險；但若 ASR 真的太久（> 60s）user 會找這個。
5. **電話 interrupted 30s 寬限**：時間夠嗎？問卷預期一般通話 ≥ 1 min，30s 對短暫 interrupt（如 Siri 啟動）夠；長通話本來就會強制 finalize。
6. **Done 卡片是否顯示「儲存於本機」hint**：給安全感 vs 視覺噪音。傾向只在第一次完成 session 時顯示（`@AppStorage("hasSeenFirstDoneHint")`）。

---

## 易用性測試準備

### 7 個 user task — 涵蓋 6 大資料狀態 + Nielsen 5

| # | Task | 測項 | 6-state 覆蓋 |
|---|---|---|---|
| 1 | **首次使用 + Empty state**：「請先開 Focus app 看一下 History 分頁，再回 Focus 分頁開始一段 25 分鐘的專注錄音」 | first-time flow + perm prompt + History empty 觀察 | **Empty**（History tab 觸發 `focus.empty.history`）/ Loading（perm wait）/ Ideal（done） |
| 2 | **PTT 模式**：「請用按住說話模式錄三段不同想法」 | PTT 按壓辨識 + chunk count + PTT/Timer 互斥 | Ideal |
| 3 | **背景 / 鎖屏**：「開始錄音後請鎖屏，30 秒後從鎖屏停止錄音」 | MPNowPlayingInfoCenter + remote command | Loading（背景錄音感）|
| 4 | **interrupted**：「錄音中接到電話，掛掉後檢查錄音狀態」 | AVAudioSession interrupt + Interrupted sub-state + 30s 寬限 | Error（中斷態）|
| 5 | **Done flow**：「看 takeaway 並開始下一段 session」 | done card 可讀性 + new session | Ideal |
| 6 | **Maximum state**：「請設定 180 分鐘自訂 timer 後立即按停止」 | duration clamp + chunk_overflow 99+ 處理（如可累積到 100+）| Maximum |
| 7 | **Partial / Error state**：「假設第 3 個 chunk ASR 失敗（測試環境注入），請從 Finalizing 跟 Done 卡讀懂發生什麼」 | partial inline + 重試 ASR + takeaway truncated 視覺 | Partial / Error / Maximum (truncation) |

### Sampling

- 目標 5–7 user（per Nielsen「5 個 user 找 80% 問題」）
- 角色：3 行動族 + 2 隱私意識 + 2 OSS contributor（per SPEC-21 §5 personas）
- 環境：iPhone 13 mini + iPhone 15 各跑一次（小螢幕 + 大螢幕）

### 觀察重點

- **screen A**：PTT 跟 Timer 是否引起混淆？互斥規則（按 PTT 時 Timer 灰）是否引起 user 不確定？
- **screen C**：暫停 vs 停止是否明確區分？電話中斷 30s 寬限文案是否清楚？
- **screen E**：載入文字是否引起焦慮（「為什麼這麼久」）？取消 CTA 是否被誤觸或被忽略？
- **screen F**：takeaway 結構是否一目了然？800 字截斷的「看完整摘要」CTA 是否被注意？
- **Maximum task**：custom input clamp 是否引起挫折？shake animation 是否足夠表達拒絕？
- **Partial task**：partial inline 訊息（chunk fail）user 是否解讀為「全部失敗了」？

### 紀錄方式

- 螢幕錄影（user 同意後）
- think-aloud protocol（user 邊操作邊說）
- 結束後 SUS 問卷 + 5 題開放問題（最讚 / 最差 / 困惑點 / 期望加 / 期望砍）

---

## 下一步

→ 拉 5 user 跑 usability test → 收 SUS 分數 + 觀察紀錄 → 回頭修 Wireframe / Mockup / Prototype 對應點
→ 若 iOS prototype 經 usability 驗證 OK，再橫向把 Wireframe / Mockup / Prototype 三層補給 Android / macOS / Windows / Linux / Web
→ 把 6 大資料狀態 + Nielsen 5 + SUS 對齊套到其他 5 平台不一定要全寫滿，可只列差異
