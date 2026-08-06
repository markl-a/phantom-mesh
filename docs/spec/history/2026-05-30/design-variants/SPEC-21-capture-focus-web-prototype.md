# SPEC-21 Capture Focus — Web Prototype（原型）

> **Stage 3/3** of the user-flow chain · [線框稿（Web Wireframe）](./SPEC-21-capture-focus-web-wireframe.md) → [視覺稿（Web Mockup）](./SPEC-21-capture-focus-web-mockup.md) → 原型（Prototype）
> **Status**: draft v0.1 · **Last updated**: 2026-05-27
> **Scope**: Web / mobile-web only — breakpoint 切換 / getUserMedia / MediaRecorder + Web Worker / C' upload-failed / IndexedDB queue / `beforeunload` / `visibilitychange`。共用 FSM / Nielsen 5 / 6 大資料狀態 / SUS 對齊**沿用 [iOS hero prototype](./SPEC-21-capture-focus-prototype.md)**，本檔只列 Web **deltas**。
> **Spec**: [`SPEC-21-SYSTEM-capture-focus`](../specs/v060-deep-spec/SPEC-21-SYSTEM-capture-focus.md) · [`SPEC-17-PROTOCOL-tauri-bridge`](../specs/v060-deep-spec/SPEC-17-PROTOCOL-tauri-bridge.md)（C' upload queue / retry backoff / IndexedDB schema）· [`SPEC-15-PROTOCOL-broker-vault-sync`](../specs/v060-deep-spec/SPEC-15-PROTOCOL-broker-vault-sync.md)（host 不可達策略）
> **這份的工作範圍**：把 Web Mockup 變「可操作」 — 每個 tap target 點下去、Web Worker × main thread 訊息流、retry backoff 曲線、IndexedDB queue flush 順序、`beforeunload` / `visibilitychange` 觸發時機、breakpoint resize 切換動畫。Web 沒 platform catalog spec，本檔 + Wireframe + Mockup 是 Web 唯一 source of truth。
> **參考**：
> - [iOS hero prototype](./SPEC-21-capture-focus-prototype.md) — 共用 6 大資料狀態 / Nielsen 5 / 9-state FSM / SUS 對齊（本檔不重抄）
> - [Web Wireframe](./SPEC-21-capture-focus-web-wireframe.md) — FSM / 螢幕結構 / invariants
> - [Web Mockup](./SPEC-21-capture-focus-web-mockup.md) — 視覺 token / Lucide icon ID / i18n key 視覺出處

## 為什麼 Web 需要獨立 prototype

iOS hero prototype 是底，Web prototype 加上 **6 個 Web-specific 互動軸**：

1. **Breakpoint resize 即時切換**（窗縮 / 接外接觸控螢幕）— React 元件 conditional render 的 timing 必須鎖
2. **getUserMedia permission flow** — Chrome / Safari / Firefox prompt timing 不一，且 `permissions.query()` 行為各家不同
3. **MediaRecorder chunk 切割必須跑 Web Worker** — main thread setInterval 被 `visibilitychange: hidden` throttle 到 1Hz，5min 切點失準（per Agy R1 architectural catch + Mockup §152）
4. **C' upload-failed 雙救濟互動**（retry × save-offline） — 連續失敗自動降級、retry backoff 曲線、IndexedDB queue 寫入順序
5. **IndexedDB quota detection** — 寫入失敗前先 `navigator.storage.estimate()` 預警、超出強制停
6. **`beforeunload` / `visibilitychange` 提示** — tab 切走 / 關 tab 的雙重保護

→ 這 6 點完全沒在 iOS prototype 出現，需獨立規格化。

## 沿用 iOS hero prototype（不重抄）

- **Nielsen 5 對應**：每個 screen 沿用 iOS 對映（Learnability / Efficiency / Memorability / Errors / Satisfaction）
- **6 大資料狀態速查表**：本檔末段 §「6 大資料狀態 — Web Prototype 互動對映」只列 Web 差異
- **9-state FSM**：完全沿用（Idle → Requesting → Recording → Chunking/Interrupted → Finalizing → Transcribing → SummaryGen → Done）
- **SUS 10 題對齊**：沿用 iOS 對映，本檔末段補 Web-specific 風險點

## Web 互動模型總覽（Threading）

```
[Main thread (React UI)]
  ├─ MediaRecorder (getUserMedia stream)  ─┐
  ├─ visibilitychange listener            │
  ├─ beforeunload listener                │  postMessage
  ├─ IndexedDB read (queue display)       │  ◄────────► [Web Worker (chunk timer)]
  └─ fetch retry queue                    │             ├─ setInterval(5min) chunk-close trigger
                                           ┘             ├─ self.clock 不受 main thread throttle
                                                         └─ postMessage('chunk-due') → main flush
```

**關鍵 invariant**：chunk 切割 timer **必須在 Web Worker** — main thread 在 tab hidden 時被 throttle 到 1Hz，setInterval(300_000) 雖然單次延後 1s 可接受，但連續累積 25min session 會偏移 ~5-10s → 5min 切點失準 → ASR 邊界錯位。

實作：`new Worker(new URL('./chunk-timer.worker.ts', import.meta.url))`，Vite/Webpack 自動 bundle。Worker thread tick 觸發 `postMessage('chunk-due')`，main thread 收到後執行 `mediaRecorder.requestData()` + `mediaRecorder.start(chunkDuration)` cycle。

## 螢幕 A1 / A2 — Idle（breakpoint 雙版型）

### Nielsen 5 對應（Web delta）

沿用 iOS A，Web 額外：
- **Learnability**：caveat banner `focus.web.caveat` 首次進來即可教 user「Web 限制」（沒鎖屏沒 tray） → 期望管理
- **Errors**：HTTPS 沒 cert 直接撞 onboarding（per Wireframe §201），不會走到 A — 本檔不覆蓋

### Tap targets（A1 mobile-web，< 768px 或 pointer: coarse）

| Target | 動作 |
|---|---|
| 頂部 `[caveat-banner]`（`focus.web.caveat`） | **不可 tap dismissible**（per Mockup §107） — tap 無動作；hover 時不變色，純資訊 |
| `25` / `50` / `自訂` chip | 即時 select；無 haptic（Web 沒 `navigator.vibrate` 在 desktop / iOS Safari，僅 Android Chrome 支援 — 不依賴）；CSS `:active` 變 bg `overlay-ripple-24` 100ms ease-out |
| **PTT 大鈕 — `pointerdown`** | (1) 若 `permissions.query({name:'microphone'})` 回 `prompt` → 立即呼叫 `navigator.mediaDevices.getUserMedia({audio:true})` 觸發 B browser prompt；(2) 若 `granted` → 0ms CSS press 變 bg `spectyn-primary @ 70%`；100ms 內 `MediaRecorder.start(300_000)` + 啟動 Web Worker chunk timer；切到 C Recording。**同時 Timer 副按鈕進入 disabled state**（per Mockup invariant）|
| **PTT 大鈕 — `pointerup` / `pointerleave` / `pointercancel`** | 立即 `mediaRecorder.requestData()` flush 當前 chunk → POST 給 host（C' 失敗路徑見下）；UI 顯示 chunk +1 toast bottom-up enter 100ms / hold 1.5s / exit 200ms；**留在 A1 Idle**（PTT 是一次按一段）；Timer 副按鈕回 enabled |
| **Timer 副按鈕** | (1) 若未授權 → 同上走 getUserMedia；(2) 授權 → 200ms cross-fade 到 C Recording；`MediaRecorder.start(300_000)` 啟動；Web Worker chunk timer 啟動 |
| `trust-badge` tap | 開 modal sheet（Web 用 `<dialog>` 元素 + `showModal()`） — 顯示 trust 全文；ESC / 點背景 / 「關閉」按鈕關閉 |

### Tap targets（A2 desktop-web，≥ 768px 且 pointer: fine）

| Target | 動作 |
|---|---|
| 頂部 caveat banner | 同 A1 |
| `◯ [duration-opt-N]` radio rows | tap 整行（不只 circle）select；keyboard `↑ ↓` 切換；`space` / `enter` 觸發 Start |
| **`[start-timer]` 按鈕** | tap：(1) 觸發 getUserMedia（若未授權）；(2) 授權後 `MediaRecorder.start(300_000)`；hover bg +10% brightness；active bg `overlay-ripple-24`；focus ring `2px spectyn-primary` outline；keyboard `enter` / `space` 等同 tap |
| **A2 無 PTT** — `space` long-press 不觸發 PTT | 桌機鍵盤情境不適合 press-and-hold（per Mockup §84） — `space` 純作 button trigger |

### Breakpoint resize 切換動畫

`window.matchMedia('(min-width: 768px) and (pointer: fine)')` listener：

| 場景 | 行為 |
|---|---|
| user 縮窗從 desktop → mobile（≥ 768px → < 768px） | React 立即 conditional render A1（無 transition；避免布局 jank）；duration 選擇沿用（state 共用）；若已在 C Recording → caveat banner 寬度 reflow，其他不變 |
| user 接外接觸控螢幕（pointer: fine → coarse） | 立即切到 A1（即使視窗仍 ≥ 768px）；ChromeBook / Surface 拔鍵盤接觸控筆即觸發 |
| user 在 A1 縮窗到 < 480px | 容器仍 max-width 480px 居中，container 縮為 viewport 寬度 — padding 16px 保留；不再切版型 |
| Recording 中 resize 觸發版型切換 | **不中斷錄音** — MediaRecorder 與 Web Worker 與版型解耦；只切 UI 元件 render |

CSS animation：**無**（resize 是 user 主動，不該再加 transition 增加 jank）。

### Failure paths

- HTTPS 缺 cert → onboarding 攔截，Idle 不該被觸發 — 本檔不覆蓋
- `permissions.query({name:'microphone'})` 不支援的 browser（舊 Firefox / WebKit < 16）→ 直接呼 getUserMedia，失敗才 fallback 到 B' Denied 卡

### Walkthrough script（usability test：「請開始一段 25 分鐘的專注錄音」）

1. user 看到 A1 / A2（依裝置），預期注意到頂部 caveat banner
2. **觀察點 1**：caveat banner 是否引起焦慮？若 30% 以上 user 反問「這 app 有問題嗎」，要重寫 `focus.web.caveat` 文案
3. user 點 25 → Start → 觸發 browser permission prompt
4. **觀察點 2**：user 是否能在 5 秒內辨認 browser native prompt 跟 spectyn UI 是兩件事？

## 螢幕 B — getUserMedia Permission Prompt

### 互動限制

Browser native — spectyn 唯一能控的是 **trigger timing**（user 主動 tap 才呼叫，不在 page load）+ **pre-permission education**（A1 / A2 trust-badge + 安撫文）。

### Tap targets（browser native dialog，spectyn 不可自訂）

| Browser | 樣式 / 文案 |
|---|---|
| Chrome / Edge | 頂部下拉 banner `<origin> wants to use your microphone` + `Block` / `Allow` |
| Safari | 中央 modal `Allow "<origin>" to use your microphone?` + `Don't Allow` / `Allow` |
| Firefox | 頂部下拉 + `Remember this decision` checkbox + `Block` / `Allow` |

### 結果回 spectyn

```
[user 點 Allow]
  → getUserMedia Promise resolve(stream)
  → 切到 C Recording
  → MediaRecorder.start() + Web Worker chunk timer 啟動

[user 點 Block / Don't Allow]
  → getUserMedia Promise reject(NotAllowedError)
  → 切到 B' Denied 卡（覆蓋 Idle）

[user 關 prompt（按 ESC / 點外面）]
  → Chrome / Firefox: Promise pending（不 resolve 不 reject）→ spectyn 視為等待中，UI 仍 spinner
  → Safari: 視同 Block → reject
```

### Timing 觀察

- Chrome：prompt 出現 < 100ms（main thread 不卡）
- Safari：prompt 出現 ~300-500ms（macOS / iOS Safari）
- Firefox：prompt 出現 < 200ms，但若 user 之前 `Remember` Block 過 → 直接 reject 不顯 prompt

spectyn UI 在 getUserMedia call 後立即進入 micro-state「Requesting」（PTT / Start button 顯示 Lucide `loader-2` + `animate-spin` 32px，文案 `focus.perm.requesting`）— **若 > 2s 仍未 resolve**，顯示 hint「請查看瀏覽器頂部對話框」（文案 `focus.web.perm_hint_browser_top`，新 key v0.7+ 補）。

## 螢幕 B' — Denied 卡（覆蓋 Idle）

### Tap targets

| Target | 動作 |
|---|---|
| 主文 `focus.perm.denied` + 安撫文 `focus.perm.denied_reassure` | 純文字，不可 tap |
| in-card 步驟提示框（取 `focus.web.perm_settings_hint`） | 純文字（per Mockup §125） — **無 deep-link button**（Web 不開放）；tap 整框無動作 |
| 卡片外圍 / ESC | 不可關（沒授權無路可走，強迫 user 處理） — 與 iOS B' 不同（iOS 可 swipe-down dismiss） |
| 「重新請求權限」按鈕（**本檔提案 — defer 到 Web mockup 補規格**）| 再次呼叫 `navigator.mediaDevices.getUserMedia({audio:true})` — Chrome 若曾 Block 會直接 reject（不再彈 prompt），轉而顯示 toast 提示 user 去地址列鎖頭圖示。**mockup §137 未列此 CTA，需 mockup R 輪補後 prototype 再鎖** |

**設計決策**：本 prototype 提案在 B' 加「重新請求」CTA，因 Mockup §137 「不畫 deep-link button」描述的是「移除 iOS 的『打開設定』那顆」 — 但 Web 應提供等效自助路徑。文案沿用 `focus.perm.retry`（hero key）。

### Failure paths

- user 在 browser 設定 manually Allow 後回 tab → `permissionchange` event 觸發（若 browser 支援）→ 自動 dismiss B' 卡 + 回 A1 / A2 Idle；不支援 `permissionchange` 的 browser → user 必須手動 refresh page

## 螢幕 C — Recording（含 Web Worker chunk timer）

### Web Worker × Main thread 通訊 sequence

```
[Start (A1 PTT / A1 Timer / A2 Start)]
  Main:  worker = new Worker('chunk-timer.worker.ts')
  Main:  mediaRecorder = new MediaRecorder(stream, {mimeType:'audio/webm;codecs=opus'})
  Main:  mediaRecorder.ondataavailable = (e) => uploadChunk(e.data)
  Main:  mediaRecorder.start(300_000)  // 5min timeslice as fallback if Worker miss
  Main:  worker.postMessage({type:'start', interval:300_000})
                                              │
                                              ▼
                                       Worker: setInterval(() => self.postMessage('tick'), 300_000)

[每 5min Worker tick]
  Worker: postMessage('tick')
  Main:   mediaRecorder.requestData()  // 立即觸發 ondataavailable
  Main:   uploadChunk(blob) → fetch POST → success → toast chunk+1
                                          → fail → C' Upload Failed

[Stop (C Stop button)]
  Main:  mediaRecorder.stop()  → 觸發最後 ondataavailable
  Main:  worker.postMessage({type:'stop'})
  Main:  worker.terminate()
  Main:  uploadFinalChunk + 切 E Finalizing

[Tab visibilitychange: hidden]
  Main:  document.visibilityState === 'hidden'
  Main:  waveform refresh 降為 5fps（用 requestAnimationFrame 自然降）— acceptable degradation
  Worker: 不受影響，continue ticking on schedule  ← 關鍵
```

### Tap targets

| Target | 動作 |
|---|---|
| 頂部 `[caveat-banner]`（`focus.web.caveat`） | bg 從 `overlay-web-warn-20` → `overlay-web-warn-30`（recording 進入時 200ms ease-out transition）；不可 dismissible |
| `⏸ pause` 按鈕（Lucide `pause`） | `mediaRecorder.pause()`；Worker postMessage `{type:'pause'}` 停止 tick；waveform 凍結（CSS `animation-play-state: paused`）；按鈕 morph 成 `▶ resume`（Lucide `play`） icon 200ms ease-in-out；計時器數字 1Hz 閃爍（`@keyframes blink 1s infinite`）；無 haptic（Web 不依賴） |
| `▶ resume` 按鈕 | `mediaRecorder.resume()`；Worker postMessage `{type:'resume'}` 從停止點繼續 tick；waveform 重跑；icon morph 回 `pause`；計時器停止閃爍 |
| `⏹ stop` 按鈕（Lucide `square`） | 立即 `mediaRecorder.stop()` + `worker.terminate()`；250ms cross-fade 到 E Finalizing；最後 chunk flush；無 confirm dialog（同 iOS 設計決策） |
| chunk count chip（Lucide `folder` + 數字） | 純資訊，tap 無動作；99 → 100 切到 `99+` 顯示 `focus.limit.chunk_overflow`；數字區塊 min-width 鎖死避免抖動 |
| `trust-badge` | 同 A1 / A2 |

### `visibilitychange` 行為（tab 切走 / 回來）

| 事件 | 行為 |
|---|---|
| `visibilitychange → hidden`（user 切 tab / 切 app） | (1) MediaRecorder 持續錄音（不變）；(2) Web Worker chunk timer 繼續準確 tick（不變）；(3) waveform render 自動降幀（rAF 被 throttle 到 1Hz）— 視覺退化 acceptable；(4) 計時器數字停更（顯示最後一刻數值，回前景立即追上）；(5) **不顯示 toast / 不切螢幕** — user 切走自己知道在做啥 |
| `visibilitychange → visible`（user 切回 tab） | (1) waveform 立即恢復 60fps；(2) 計時器數字立即同步（用 `Date.now() - startedAt` 計算，不靠累加）；(3) **不顯示「歡迎回來」toast** — 避免打擾；(4) 檢查 IndexedDB queue：若有 pending uploads → 自動 retry（不需 user 動作） |

### `beforeunload` 行為（user 關 tab / 切網址）

```javascript
window.addEventListener('beforeunload', (e) => {
  if (isRecording || hasOfflineQueue) {
    e.preventDefault()
    e.returnValue = i18n('focus.web.offline_unload_warn')
    return e.returnValue
  }
})
```

| 場景 | 行為 |
|---|---|
| Recording 中 user 關 tab | 觸發 browser native `beforeunload` dialog（不可自訂版面） — Chrome 顯示「離開網站？」 + `Leave` / `Cancel`；若 user 選 Leave → tab 關 → MediaRecorder + Worker 強制終止 → **未 flush chunk 全丟**（degraded scope per Wireframe §201） |
| save-offline 模式有 pending queue 時關 tab | 同上 dialog，但訊息文案 `focus.web.offline_unload_warn`「還有 X 段未上傳，關了會留在 browser」 — 強調「不會立刻丟」（IndexedDB 持久），但提醒 user 下次回來才能 flush |
| 完全沒 recording / 沒 queue 時關 tab | **不觸發 dialog** — `beforeunload` listener `e.preventDefault()` 條件不成立 |

**Browser 限制**：現代 Chrome / Firefox 已不允許自訂 `e.returnValue` 字串顯示，只能彈標準訊息。spectyn 無法強制顯示 `focus.web.offline_unload_warn` 文字 — 但 listener 註冊本身仍能觸發 dialog（dialog 文案由 browser 統一），文案會 fallback 到 i18n key 內容僅 for screen-reader / 開發者 debug。

### Failure paths

- **MediaRecorder API 不支援**（極舊 browser）→ Idle 直接顯示「請升級 browser」screen（不會走到 C）
- **`audio/webm;codecs=opus` 不支援**（Safari < 14.1）→ fallback 到 `audio/mp4;codecs=mp4a.40.2`（AAC）；後端 host 必須能解 — defer SPEC-17
- **Stream interrupted by OS / hardware**（user 拔外接 USB mic）→ MediaRecorder `error` event → 進 Interrupted sub-state（沿用 iOS Interrupted UI，但 Web 沒寬限機制 — 直接強制 finalize，per Wireframe §201）

### Walkthrough script（usability test：「開始錄音、切到另一個 tab 30 秒、回來繼續、停止」）

1. user 從 A1 / A2 Start → C Recording
2. **觀察點 1**：caveat banner 加深是否被 user 注意？預期 < 20% user 主動提及（潛意識訊號夠）
3. user 切到 YouTube tab 看 30 秒
4. **觀察點 2**：user 回來時是否預期錄音還在跑？若 30% 以上 user 反問「還在錄嗎」，要在 visibilitychange visible 時加 brief「✓ 仍在錄」toast
5. user 按 Stop → 切 E Finalizing
6. **觀察點 3**：user 是否信任「切 tab 沒掉錄音」？這是 Web 平台特有信任點

## 螢幕 C' — Upload Failed Sub-state（Web 獨有）

### 進入觸發條件

```
[Recording 中每次 chunk POST 失敗]
  ↓
  retry attempt #1 (immediate)
  ↓ fail
  retry attempt #2 (backoff 2s)
  ↓ fail
  retry attempt #3 (backoff 4s)
  ↓ fail
  → 進入 C' Upload Failed UI（不再自動 retry，等 user 決定）
```

**Retry backoff 曲線（per SPEC-17 defer，本檔提案）**：
- Attempt 1: 0ms（即時）
- Attempt 2: 2000ms
- Attempt 3: 4000ms
- 全 3 次失敗 → 進入 C' UI；不再 auto-retry，user 必須選 Retry 或 Save-offline

### Tap targets

| Target | 動作 |
|---|---|
| 頂部 `[upload-failed-msg]` banner（`focus.web.upload_failed`，Lucide `wifi-off` 20px） | 純資訊，tap 無動作；bg `overlay-error-16` 從 C 的 caveat 變色 transition 250ms |
| `[retry-btn]`（`focus.web.retry`，bg `spectyn-danger`，左、主要救濟） | 立即重新嘗試 POST 失敗 chunk；UI 顯示 spinner 32px；成功 → 200ms cross-fade 回 C；失敗 → retry counter +1，**連續失敗 5 次自動切 save-offline 模式**（per Wireframe §165） |
| `[save-offline-btn]`（`focus.web.save_offline`，bg `spectyn-card`，右、次要救濟） | 立即寫 IndexedDB queue → 200ms cross-fade 回 C；caveat banner 文案切 `focus.web.offline_pending`「已暫存 {n} 段」；bg 切回 `overlay-web-warn-20`（warn 但非加深）；**錄音繼續不中斷**；後續 chunk 直接寫 IndexedDB（不再嘗試 POST）|
| waveform（凍結態） | 純視覺，無 tap 互動；color 從 `spectyn-warning` 切 `spectyn-muted` 250ms |
| `trust-badge` | 同 C — 仍提醒「就算 upload 失敗，本地仍加密」|

### IndexedDB queue 寫入順序（save-offline 模式）

```
[Save-offline 啟動]
  → 把當前失敗的 chunk N 寫入 IndexedDB `chunk_queue` store
  → 後續每個 chunk 切割點：
       MediaRecorder.ondataavailable → blob
       → 直接 IndexedDB.put({id: chunkId, blob, sessionId, ts, status:'pending'})
       → 不嘗試 fetch
  → caveat banner 更新 `已暫存 {n} 段`

[User 連網 / host 恢復後]
  → 偵測：navigator.onLine 變 true OR fetch HEAD /health 200
  → 自動觸發 flush queue：
       for chunk in IndexedDB.getAll() sorted by ts asc:
         POST /chunks → success → IndexedDB.delete(chunk.id) → toast `chunk_uploaded`
         POST → fail → 停止 flush，等下次 online event
  → 全部 flush 完 → caveat banner 切回 `focus.web.caveat`
```

**Flush 順序**：永遠按 `ts asc`（時間序），不平行（保證 ASR 順序正確；host 端 chunk index 必須連續）。

### IndexedDB quota 預檢

每次 `ondataavailable` 觸發、寫 IndexedDB 前：

```javascript
const {quota, usage} = await navigator.storage.estimate()
if (usage / quota > 0.9) {
  // 90% 警告（不阻擋）
  showToast(i18n('focus.web.quota_warn', {percent: 90}))
}
if (usage + blob.size > quota * 0.95) {
  // 95% 強制停
  mediaRecorder.stop()
  worker.terminate()
  showToast(i18n('focus.web.quota_exceeded'))  // 全寬 48px bg spectyn-danger@95%
  navigate(A1)
  return
}
await db.put(blob)
```

| 閾值 | 行為 |
|---|---|
| < 80% | 正常寫入，無提示 |
| 80-90% | 正常寫入，無提示（避免過早煩擾） |
| 90-95% | toast 警告 `focus.web.quota_warn`（**新 key，v0.7 補**），持續錄音 |
| > 95% | 強制停 + `focus.web.quota_exceeded` toast（per Mockup §183）+ 切回 A1 |

### Failure paths

- **Retry 5 次全失敗** → 自動切 save-offline 模式（不再彈 C'，避免騷擾）
- **save-offline + IndexedDB quota 已滿** → 直接觸發 quota_exceeded 路徑（停錄音 + toast）
- **save-offline + user 關 tab** → `beforeunload` 提示 `focus.web.offline_unload_warn`；user 若 Leave → IndexedDB 資料留著（持久），下次回來 auto-flush
- **save-offline + 24h 過期清除**（per SPEC-17 defer） → user 下次回來時若 IndexedDB chunk ts > 24h → 顯示「過期暫存資料？保留 / 清除」對話（v0.7+）

### Walkthrough script（usability test：「錄音中假裝網路斷掉 30 秒、選 Retry、再斷、選 Save-offline」）

1. 測試環境注入 fetch error → C' 出現
2. **觀察點 1**：user 是否能在 5 秒內辨認哪顆是 Retry 哪顆是 Save-offline？視覺權重區分是否清楚？
3. user 點 Retry → 注入仍失敗 → C' 仍在
4. **觀察點 2**：user 連點幾次 Retry 才放棄改選 Save-offline？預期 2-3 次內 — 若 > 5 次，視覺權重需調整
5. user 點 Save-offline → 回 C + caveat banner 變文案
6. **觀察點 3**：user 是否信任「暫存資料不會丟」？這直接影響 SUS 第 9 題（使用上有信心）

## 螢幕 E — Finalizing（Web 等同 iOS，但全 host 端計算）

### 互動差異（vs iOS）

- **ASR / LLM 全跑 host**（spectyn-serve），browser 只 POST chunk + render progress
- E phase 1 (Transcribing) progress：host 推 SSE `event: chunk_transcribed`，每次收到 progress +1/N
- E phase 2 (SummaryGen) spinner：host 推 `event: summary_started` → spinner / `event: summary_done` → 切 F

### Tap targets

| Target | 動作 |
|---|---|
| `[cancel-show-transcript]`（`focus.btn.cancel_show_transcript`） | 立即斷 SSE connection；用已 stitch 的 transcript 寫 focus row（takeaway 留空字串）；切 F Done |
| `[retry-asr]`（錯誤態出現） | 重新 POST 失敗 chunks 給 host /reasr endpoint |
| `[use-empty-transcript]` | 同 iOS — LLM 跑空字串 |

### Failure paths（Web 額外）

- **host 在 Finalizing 中失聯** → SSE connection drop → 顯示 toast「host 失聯，請刷新重試」+ stitched transcript 保留在 IndexedDB；user refresh page 後回 E 繼續
- **tab 切走超過 5 分鐘** → host 端 session timeout（可設定） → user 切回時 SSE 已斷 → 同上路徑
- **`beforeunload` 在 Finalizing 中** → 觸發 dialog `focus.web.offline_unload_warn`「Finalize 中關閉會丟失 takeaway」— 但 transcript 已寫 IndexedDB 不會丟

## 螢幕 F — Done（Web 等同 iOS）

### 互動差異（vs iOS）

- **無 swipe-down dismiss** — Web 沒有 swipe-down gesture（mobile-web 在 iOS Safari 有 pull-to-refresh 但會 reload page，不能 hijack）→ 改用「`< 完成`」nav button
- **無 haptic** — Web 不依賴
- **Entry animation**：cross-fade 250ms + success icon (Lucide `check-circle`) scale 0→1 spring 用 CSS `@keyframes scale-spring`（CSS spring polyfill，非 SwiftUI 原生）

### Tap targets（沿用 iOS，僅 i18n key 一致）

| Target | 動作 |
|---|---|
| `[view-full-transcript]` button (`focus.done.view_full`) | React Router push `/transcript/:id` |
| `[new-session]` button (`focus.done.new_session`) | React Router push `/focus`（回 A1 / A2） |
| takeaway card truncated state | tap 整張 → 等同 view-full-transcript（per iOS pattern） |
| nav `< 完成` | 同 new-session |

### Failure paths

- 無新失敗 path — F 是 success 態，資料已寫 host

## 6 大資料狀態 — Web Prototype 互動對映

| 狀態 | Web 互動行為 |
|---|---|
| **理想** | F Done 完整 takeaway 顯示（同 iOS） |
| **空白** | History tab in-tab 顯示 SVG illustration + `focus.empty.history` + 「前往 Focus」按鈕 → tap 切 React Router /focus |
| **極限** | C chunk 99 → 100 顯示 `99+`；F takeaway 截斷 + tap 整張跳 transcript；**IndexedDB quota > 95% 強制停 + toast**（Web 獨有） |
| **錯誤** | B' Denied 卡 + 「重新請求」CTA（本檔提案）；**C' Upload Failed 雙救濟並列**（Web 獨有）；E 全 ASR 掛 → 兩按鈕（同 iOS） |
| **局部** | E inline `focus.partial.chunk_failed`（同 iOS）；**save-offline 模式下 caveat banner 切 `focus.web.offline_pending`**（Web 獨有，「上傳中 X 段，落地 Y 段」）|
| **載入中** | E spinner-32（Lucide `loader-2` + `animate-spin`） + SSE progress；首次 getUserMedia 等待 prompt 期間 Idle button 顯示 spinner |

## Web-specific Failure 全表

| Failure | 偵測 | 回復 |
|---|---|---|
| HTTPS cert 失效 | fetch 觸發 `NetworkError` | 切回 onboarding（不在本檔覆蓋） |
| getUserMedia rejected | Promise reject `NotAllowedError` | 切 B' Denied + 提供「重新請求」CTA |
| MediaRecorder API 不支援 | `typeof MediaRecorder === 'undefined'` | Idle 顯示「請升級 browser」screen |
| MIME type 不支援 | `MediaRecorder.isTypeSupported()` false | fallback 到 `audio/mp4` |
| Chunk POST 失敗 | fetch reject / status ≥ 500 | retry backoff（具體秒數 defer SPEC-17）→ C' |
| Web Worker 載入失敗 | `worker.onerror` | fallback 到 main thread setInterval（warn user 切走 tab 可能失準）|
| IndexedDB write 失敗 | put reject `QuotaExceededError` | toast `focus.web.quota_exceeded` + 強制停 |
| tab 切走 | `visibilitychange` | 沿用錄音、降幀；無中斷 |
| tab 關閉 | `beforeunload` | 觸發 native dialog；user 確認後資料留 IndexedDB |
| host SSE 斷線（Finalizing 中） | EventSource `onerror` | toast + transcript 保留 IndexedDB；user refresh 重連 |
| navigator.onLine 變 false | `offline` event | C' 立即出現（不需等 retry 3 次） |
| navigator.onLine 變 true | `online` event | 自動 flush IndexedDB queue |

## Cross-platform invariants 對齊（per Mockup §189 + Wireframe §194）

繼承全部 hero invariants + Web 額外 invariants（沿用 Wireframe / Mockup 鎖定）：

- **Caveat banner 全程置頂** — idle / recording / C' / save-offline 四段文案 + 顏色切換，不可 dismissible
- **C' Upload Failed 不可阻止繼續錄音** — Retry × Save-offline 雙救濟並列、不能藏進 menu
- **Web Worker chunk timer 必須跑** — main thread setInterval throttle 會破壞 5min 切點（per Agy R1 catch）
- **`beforeunload` 提示必須註冊** — Recording 中 / save-offline queue 有資料時 / Finalizing 中
- **getUserMedia 必須 user 主動觸發** — 不可 page load 自動呼叫（browser policy + UX）

## SUS 對齊（Web 風險點補充，沿用 iOS 10 題）

iOS prototype 已列預期分布，本檔僅補 **Web 額外風險點**：

| 題目 | Web 風險 | 對應 mitigation |
|---|---|---|
| 1. 想常用 | tab 切走焦慮、`beforeunload` 提示煩 | caveat banner 預先教育、visibilitychange 不顯 toast 避免騷擾 |
| 3. 容易使用 | getUserMedia prompt 中斷感更強（不像 iOS native） | pre-permission education（trust-badge）|
| 4. 需要技術支援 | B' Denied 卡無 deep-link，user 自己找瀏覽器設定 | in-card 步驟提示 `focus.web.perm_settings_hint` + 「重新請求」CTA |
| 6. 不一致 | 兩個版型 A1 / A2 在 resize 時切換可能造成混亂 | resize 不加 animation、state 共用避免位元突然消失 |
| 7. 多數人能很快學會 | caveat banner 文案太技術會嚇到 user | 避免「Web Worker」「IndexedDB」等技術詞 — 改用「瀏覽器模式」「暫存」 |
| 8. 不靈活 | tab 必須開著、quota 限制 | save-offline 救濟 + quota 95% 警告早於 100% |
| 9. 有信心 | 怕 tab 關就丟資料 | `beforeunload` + IndexedDB 持久保留 |

**Web 目標 SUS：60-75**（比 iOS 65-80 略低 — Web 限制本來就多）。**若 < 60，優先檢討**：
1. C' Retry vs Save-offline 視覺權重（user 真的常用路徑是哪個？）
2. Caveat banner 文案（焦慮 vs 預告）
3. B' Denied 卡「重新請求」CTA 是否被找到

## 開放問題（Web prototype 層面）

1. **C' Retry backoff 曲線**：具體秒數規格 defer 到 SPEC-17 tauri-bridge 鎖（本檔不訂數字 — 監測實際 host 不可達場景的恢復時間後 lock，v0.7+ 調整）。
2. **`visibilitychange visible` 是否該顯「✓ 仍在錄」toast**：若 usability test 30%+ user 反問，要加；目前不加避免騷擾。
3. **save-offline 模式持續錄音時的 chunk 數上限**：IndexedDB quota 95% 強制停外，是否該有更早 chunk 數軟上限（如 100 chunks ≈ 500 min）？提案：不加，quota 預檢已足夠。
4. **Web Worker chunk timer 失敗 fallback**：fallback 到 main thread setInterval 是退化路徑，要不要在 init 時告訴 user「你的 browser 不支援 Worker，切 tab 可能影響錄音」？提案：silent fallback，避免技術細節嚇 user。
5. **B' Denied 卡「重新請求」CTA**：本檔提案加，但 Mockup 未列。需 Mockup 補一個 button 視覺 token + i18n key（沿用 `focus.perm.retry` 還是新增 `focus.web.perm_retry`？） — defer 下次 mockup 修訂。
6. **`beforeunload` dialog 文案無法自訂**：browser 強制 — 但 listener 註冊本身仍重要。要不要在 caveat banner 預先提示「關閉前會提示」？提案：不加，過度溝通。

## 易用性測試準備（Web 補充 task）

iOS hero prototype 7 個 task 沿用，**Web 補 3 個額外 task**（共 10 個）：

| # | Task | 測項 | 6-state 覆蓋 |
|---|---|---|---|
| 8 | **Tab 切換 + 回來**：「開始錄音 → 切到 YouTube 看 30 秒 → 回 spectyn tab → 停止」 | visibilitychange + Web Worker 持續性 + 信任感 | Ideal |
| 9 | **C' Upload Failed 救濟**：「錄音中假裝 host 斷網（測試環境注入），請選擇你信任的救濟方式」 | C' 雙救濟視覺權重 + retry 連續失敗降級 + save-offline 信任 | Error + Partial |
| 10 | **Breakpoint resize**：「desktop 開錄音，縮窗到 mobile 寬度，繼續錄音 30 秒」 | A2 → A1 切換不中斷錄音 + 版型 state 共用 | Ideal |

### Sampling

- 目標 5-7 user（per Nielsen）
- 角色：3 桌機 user + 2 mobile-web user + 2 「會 tab 切走」typical browser user
- 環境：Chrome on macOS + Safari on iOS 17 + Firefox on Windows 11 各跑一次（覆蓋主流 engine）

### 觀察重點（Web-specific）

- **A1 / A2 breakpoint 切換**：resize 時 user 是否注意到版型變了？是否引起混亂？
- **B getUserMedia prompt**：user 是否能在 5s 內辨認「browser native」vs「spectyn UI」？
- **B' Denied 卡**：user 是否能自助找到瀏覽器麥克風設定？或一直按「重新請求」？
- **C visibilitychange**：切 tab 回來時 user 是否信任錄音還在跑？
- **C' Upload Failed**：Retry vs Save-offline user 真實偏好？
- **`beforeunload`**：user 是否會被「離開網站？」dialog 嚇到？

## 下一步

→ 拉 5-7 user 跑 Web usability test → 收 SUS 分數 + 觀察紀錄（特別是 Web-specific 3 個 task） → 回頭修 Wireframe / Mockup / Prototype 對應點
→ 與 SPEC-17 tauri-bridge 對齊：retry backoff 曲線、IndexedDB queue schema、24h 過期清除規則（具體數字 defer SPEC-17）
→ 與 SPEC-15 broker-vault-sync 對齊：host 不可達偵測機制（HEAD /health vs SSE keep-alive）
→ 補新增 i18n keys：`focus.web.perm_hint_browser_top` / `focus.web.quota_warn`（v0.7） / `focus.web.perm_retry`（若 B' 加 CTA） — defer 下次 mockup 修訂
