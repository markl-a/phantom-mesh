# SPEC-21 Capture Focus — Linux Mockup（視覺稿）

> **Stage 2/3** · [線框稿（Linux）](./SPEC-21-capture-focus-linux-wireframe.md) → 視覺稿 → [原型（待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-27
> **Scope**: Linux only — Lucide SVG / GTK CSD / libnotify / SNI tray / DE-aware fallback 視覺規格。**hero 平台是 iOS**（見 [SPEC-21-capture-focus-mockup.md](./SPEC-21-capture-focus-mockup.md) §iOS hero + §Linux section L451-469）— 本檔擴展 Linux section 為完整視覺稿，**不重抄 hero**。
> **Spec**: [`SPEC-21-SYSTEM-capture-focus`](../specs/v060-deep-spec/SPEC-21-SYSTEM-capture-focus.md) · [`SPEC-44-PLATFORM-Linux-foundations`](../specs/v060-deep-spec/SPEC-44-PLATFORM-Linux-foundations.md) · [`SPEC-45-PLATFORM-Linux-screens-flows`](../specs/v060-deep-spec/SPEC-45-PLATFORM-Linux-screens-flows.md) · [`SPEC-02-FOUNDATION-design-tokens`](../specs/v060-deep-spec/SPEC-02-FOUNDATION-design-tokens.md)

## 為什麼 Linux 有獨立 mockup

hero mockup §Linux（L451-469）只列「跟 Win deltas」18 行。實際 Linux 視覺需鎖：

1. **Lucide SVG bundled** — Linux 無單一 OS icon system（SF/Material 都不適用），全部 icon 走 app 內 bundle，**不依賴 distro icon theme**
2. **GTK CSD（client-side decorations）真窗** — 非 macOS sheet，需鎖 titlebar / min-size / WM_CLASS / decorations 行為
3. **SNI tray icon spec**（KDE/Plasma）+ **AppIndicator spec**（GNOME + extension）視覺一致 — 24×24 single-color SVG，3 種狀態
4. **libnotify 兩種 urgency 樣式** — Done = `low`（不打斷）/ Interrupted = `critical`（不折疊）
5. **GTK theme follow**（Adwaita / Yaru / Breeze）— spectyn token 在 dark/light 兩模式都要不違和
6. **Device-error 螢幕**（取代 mic perm denied — Linux 無 OS gate）— 視覺需有，但跟 mobile 的 B' 略不同（按鈕走 DE-aware deep link）

→ 這 6 點值得獨立 mockup 級規格，不要塞在 hero §Linux 18 行裡。

## Design token 對映（per SPEC-02 + SPEC-44 / SPEC-45）

Linux 不像 Android 有 Material 3 tonal palette 強制對映 — 直接用 spectyn token，但要跟 GTK theme 在「dark/light 切換」與「accent color」上不打架：

| GTK 環境 | spectyn 對應策略 |
|---|---|
| GTK dark（Adwaita-dark / Yaru-dark / Breeze-dark） | spectyn dark token 直接用（見 hero mockup §design token 速查），window bg `spectyn-bg` 對 GTK `@theme_bg_color` |
| GTK light（Adwaita / Yaru / Breeze） | **TBD（同 hero mockup 開放）** — v0.6.0 webkit2gtk webview 先強制 dark；light 對映歸 SPEC-02 §7 補完表 |
| `prefers-color-scheme` 切換 | follow per SPEC-45 G4（timing 由 Prototype 鎖）；webkit2gtk < 2.40 fallback light（不爆畫面，僅違和） |
| GTK accent color（GNOME 47+ / KDE） | **不 follow** — 為保 spectyn brand 一致（per hero invariant `focus.trust_badge` 跨平台一字不差），accent 永遠用 `spectyn-primary` |

**為什麼不 follow GTK accent**：跟 Android Dynamic Color 預設關同理 — focus 是「trust」場景，用戶看到的 primary action 顏色跨平台一致比 follow desktop accent 重要。Settings 可選 toggle「Use GTK accent」留給 v0.7+。

## Lucide icon 規範（per hero Icon 對照矩陣 L58-72）

Linux column = Lucide（hero mockup 已鎖）。Linux 用實作要點：

- **bundled with app**：build 時 SVG inline 進 webview asset，**不靠 `/usr/share/icons/` distro 主題**（避免 Adwaita / Papirus / Breeze 三家畫風混搭）
- **size convention**：tray = 24×24px single-color（StatusNotifierItem 規範）/ inline button = 20×20px / large illustration = 64×64px
- **color**：icon 走 currentColor，由父元素 token 決定（`spectyn-primary` / `spectyn-warning` / `spectyn-danger` / `spectyn-muted`），不在 SVG 內 hardcode 色
- **tray icon mono**：SNI 規範 single-color，**用 spectyn mono variant**（無漸層、無多色） — 讓 KDE 自動 tint 到 panel theme 色不打架

### Linux capture-focus 用到的 Lucide ID

| 角色 | Lucide ID | 用途 |
|---|---|---|
| 麥克風 | `mic` | tray idle / recording / PTT button |
| 麥克風關 | `mic-off` | tray paused / B' device-error / interrupted |
| 播放 | `play` | start session btn |
| 暫停 | `pause` | C recording pause btn |
| 停止 | `square` | C recording stop btn（hero icon 矩陣鎖：Lucide 用 filled square 表 stop） |
| 資料夾 | `folder` | chunk count badge |
| 完成 | `check-circle` | F Done success icon |
| 設定 | `settings` | B' open-settings btn / Focus tab header |
| 警告 | `triangle-alert` | interrupted libnotify icon / partial inline |

## Linux 共用文案 keys

繼承 hero mockup §75-136 全部 i18n keys。Linux 不新增 key（device-error 走 `focus.err.no_mic`、interrupted 走 `focus.desktop.interrupt_notif_title` + `focus.interrupted.resume_hint` + `focus.desktop.interrupt_notif_action`、done libnotify 走 `focus.done.title` 縮短版）。

**libnotify body PII 約束**（per SPEC-45 §12.1 STRIDE Information Disclosure）：
- Done notification body **不放 takeaway 第一行** — 只放結構化 metadata（`25 min · 5 chunks · takeaway ready`）
- 避免 lock-screen / notification history 預覽外洩 personal observation

## 螢幕 A — Start Window（GTK CSD 真窗，per Linux wireframe §A）

非 macOS sheet（Linux 無 sheet 慣例）— 真窗 480×320px。

```
┌──────────────────────────────────────┐  GTK CSD window 480×320px, min 400×280px
│ 開始焦點時段                   [_][□][✕] │  ← GTK CSD titlebar（系統繪），WM_CLASS=spectyn-mesh
│ ────────────────────────────────────│  divider 1px spectyn-border
│                                      │
│ 時長：                                │  title-sm spectyn-text
│  ○ 25 分鐘 Pomodoro                  │  radio rows 32px, accent spectyn-primary
│  ○ 50 分鐘                            │
│  ◉ 自訂： [ 30 ] 分鐘                 │  custom input 60×28px, radius 6px, bg spectyn-bg
│                                      │
│ 目標標籤（選填）：                     │  title-sm spectyn-text
│ ┌──────────────────────────────┐    │  text input full-width, height 32px,
│ │ deep_work, spec_writing       │    │   bg spectyn-bg, radius 6px, padding 8px
│ └──────────────────────────────┘    │   placeholder "輸入標籤…" spectyn-muted
│                                      │
│ 🔒 本地加密 · 麥克風 ASR              │  caption spectyn-muted（取 `focus.trust_badge` — 一字不差）
│                                      │
│      [ 取消 ]      [ 開始 ]          │  Cancel 96×32px, bg transparent, text spectyn-muted
│                                      │  Start 96×32px, bg spectyn-primary, text spectyn-bg
└──────────────────────────────────────┘
```

**Visual states**：
- radio row: idle / selected（`spectyn-primary` outer ring + filled dot） / hover（bg `spectyn-card` lighten 8%）
- Start button: idle / hover（`spectyn-primary @ 90%`）/ pressed（`spectyn-primary @ 80%`）/ disabled（`overlay-disabled-40`）
- Cancel button: idle / hover（text `spectyn-text`）
- text input focus: border `spectyn-primary` 2px

**GTK CSD 注意**：
- `decorations: true`（per SPEC-45 §3.2 NG4） — 不自家畫 titlebar，讓 GTK + WM 處理 min/max/close 按鈕順序（macOS-like 左對齊 / Windows-like 右對齊由 user GTK setting 決定）
- WM_CLASS = `spectyn-mesh`（tiling WM rule 認得）
- `.desktop` 內 `StartupWMClass=spectyn-mesh` 對齊（per SPEC-45 §7.1 `LinuxScreenSpec`）

## 螢幕 B — Mic Permission（Linux 無 OS gate，無此螢幕）

per Linux wireframe §B — PulseAudio / PipeWire 不 prompt，直接成功進 C。mockup 無視覺。

## 螢幕 B' — No-mic Device Error（取代 mic perm denied）

```
┌──────────────────────────────────────┐  bg: spectyn-bg（覆蓋 Start window 或 main window Focus tab）
│                                      │
│           [mic-off-icon]             │  Lucide `mic-off` 64px, spectyn-danger
│                                      │
│        找不到麥克風裝置               │  title (24px/600) spectyn-text, centered
│                                      │
│      請檢查系統音訊設定               │  body spectyn-muted, centered, line-height 1.5
│                                      │
│   ┌──────────────────────────┐      │  Open-settings btn: 240×40px centered, radius 8px
│   │  ⚙  打開音訊設定          │      │  bg spectyn-primary, text spectyn-bg, body-lg/600
│   └──────────────────────────┘      │  icon: Lucide `settings` 18px spectyn-bg
│                                      │
└──────────────────────────────────────┘
```

**Visual states**：
- Open-settings btn: idle / hover（`spectyn-primary @ 90%`） / pressed（`spectyn-primary @ 80%`） / **disabled**（`overlay-disabled-40`，當測不到任何 audio control panel binary 時）
- 文字 `找不到麥克風裝置` 取 `focus.err.no_mic`（hero mockup §116 key 一字不差）

**DE-aware deep link 對映**（mockup 層只列分支，prototype 補 detection 邏輯）：

| `XDG_CURRENT_DESKTOP` | 按鈕 exec | Fallback |
|---|---|---|
| `GNOME` / `Unity` | `gnome-control-center sound` | `xdg-open` |
| `KDE` | `kcmshell5 kcm_pulseaudio` | `xdg-open` |
| `XFCE` | `pavucontrol` | `xdg-open` |
| `LXQt` / `Budgie` / 其他 | `pavucontrol`（多數 distro 預載） | `xdg-open audio:///` → disable btn |
| 未測到任何 binary | n/a | disable btn + 顯示文字「請從系統設定開啟音訊面板」 |

## 螢幕 C — Recording（GTK 真窗 / Focus tab 內）

版型同 hero iOS C — per hero mockup §iOS C L180-197。Linux delta（mockup-level）：

```
┌──────────────────────────────────────┐  bg: spectyn-bg, Focus tab 內或獨立 GTK window
│             05:23 / 25:00            │  display (48px/700) spectyn-warning
│                                      │
│  ▁▂▃▅▇▇▇▅▃▂▁                          │  waveform: 32 bars × 4px wide × 2px gap,
│                                      │   height 0-120px dynamic, color spectyn-warning
│                                      │
│  ┌──────┐    ┌──────────┐            │  Pause: 120×56px, radius 12px, bg spectyn-card,
│  │ ⏸暫停│    │ ⏹ 停止    │            │   icon Lucide `pause` 20px spectyn-secondary
│  └──────┘    └──────────┘            │  Stop: 120×56px, radius 12px, bg spectyn-danger,
│                                      │   icon Lucide `square` 20px spectyn-text
│  📁 已落地 chunk: 3                   │  body spectyn-muted, icon Lucide `folder` 14px
│                                      │  **Limit 變體**：chunk ≥ 100 顯示 `99+`（取 `focus.limit.chunk_overflow`）
│                                      │  數字 min-width 48px 鎖死（同 hero iOS C 規範）
│  🔒 本地加密                          │  caption spectyn-muted
└──────────────────────────────────────┘
```

**Visual states**：
- Pause button: idle / hover（bg `spectyn-card` lighten 8%） / pressed（lighten 12%）
- Stop button: idle / hover（`spectyn-danger @ 90%`） / pressed（`spectyn-danger @ 80%`）
- waveform：recording 期間 dynamic update（fps 由 Prototype 鎖）；interrupted 切 spectyn-muted 凍結

**GTK window 行為**：
- 跑在 main window Focus tab 內（canonical surface） — 不浮 separate window
- 不切 nav bar tint（Linux 無 iOS 「整條 status bar 染色」慣例）— 改用 tray icon 變色當提示

## 螢幕 C-tray — Tray icon 視覺（SNI / AppIndicator 共用 spec）

24×24px single-color SVG，3 種狀態：

| State | Lucide | 顏色策略 | SNI status |
|---|---|---|---|
| **Idle**（未錄音） | `mic` outlined | spectyn mono（panel theme tint） | `Passive` |
| **Recording** | `mic` filled | spectyn-warning（橘） | `Active` |
| **Paused** | `mic-off` | spectyn-muted（灰） | `Active` |
| **Interrupted**（app 進背景時） | `mic` filled + dot overlay | spectyn-warning + spectyn-danger 小圓點 6px | `NeedsAttention` |

**SNI 限制**：
- single-color SVG —「橘 vs 灰 vs danger 點」靠 SVG 內 `currentColor` + 兩層 path（base mono + overlay color）達成
- 但 GNOME Shell 內建 indicator 模塊**強制 monochrome panel 色**，spectyn 的 warning/danger 色可能被吃掉 — 此情境**仰賴 main window title** 補充狀態（title 加 `[Recording]` prefix）
- KDE Plasma SNI 支援 full color → 視覺最佳；Xfce 介於中間

### Tray right-click menu（per Linux wireframe §C-tray）

```
┌──────────────────────────────┐  bg: 系統 menu theme（GTK / Plasma 樣式）
│ 🔴 Focus 05:23/25:00          │  disabled row (italic), body spectyn-warning
│ ──                            │  divider
│ ⏹ 停止並收工                  │  body, Lucide `square` 16px spectyn-danger
│ ⏸ 暫停                        │  body, Lucide `pause` 16px spectyn-secondary
│ ──                            │
│ Open Spectyn Mesh             │  body
└──────────────────────────────┘
```

- label 取 `focus.btn.stop_finalize` / `focus.btn.pause` i18n key
- **Stop 在 Pause 上方**（per hero invariant + Win tray 規範）— Recording 期間最高優先
- menu item icon 用 Lucide 16px（menu 比 tray small）
- GNOME 無預設 tray → menu 不存在，user 從 main window Focus tab 內按鈕停（onboarding 已提示安裝 `appindicatorsupport`）

## 螢幕 C' — Interrupted（waveform 凍結 + libnotify critical）

GTK window 內視覺等同 hero iOS C'（waveform 退色 spectyn-muted、計時器退到 spectyn-muted、Stop 仍可按）— 但 **desktop 強制系統通知**（per hero invariants + Linux wireframe §C'）：

```
[libnotify D-Bus org.freedesktop.Notifications — interrupted]
┌─────────────────────────────────────────┐  渲染由 dunst / mako / GNOME Shell / Plasma 處理
│ [spectyn mono icon 32]                  │  bundled SVG mono variant
│  Spectyn Mesh 焦點時段中斷               │  summary 取 `focus.desktop.interrupt_notif_title`
│                                         │
│  5:23 / 25:00 · mic 被佔用               │  body line 1（依 interrupt 來源動態填）
│  30 秒內回復將自動繼續                    │  body line 2 取 `focus.interrupted.resume_hint`
│                                         │
│  [ 開啟並停止 ]                          │  action btn 取 `focus.desktop.interrupt_notif_action`
└─────────────────────────────────────────┘
```

**libnotify hint 規格**：
- `urgency = critical`（per hero mockup §467 + Linux wireframe）— 不讓 dunst / mako compositor 折疊
- `expire_timeout = 0`（不自動消失）
- `sound-name = dialog-warning`（freedesktop sound theme，**best-effort** — 部分 compositor 不 play）
- `category = im.received`（讓 KDE/GNOME 排序對應通知群）
- `desktop-entry = spectyn-mesh`（連結回 .desktop file）

**Compositor 支援度落差（mockup 層僅列、prototype 細鎖）**：
- KDE / GNOME Shell：actions 顯示為按鈕、persistent OK
- xfce4-notifyd：actions 部分支援
- dunst / mako：依 user 自家配置；actions 可能不顯示 → **二級救濟靠 tray attention 變體 + main window 訊息**
- 全平台共通：**點 body 仍應 fire `default` action** 回 main window（compositor 多半支援）

**Interrupt 來源動態文案對映**（body line 1）：

| 來源 | body 文字 |
|---|---|
| PipeWire `node-removed` / PA `source-output-removed` | `5:23 / 25:00 · mic 被其他 app 使用` |
| systemd-logind `PrepareForSleep` | `5:23 / 25:00 · 系統即將進入睡眠` |
| PipeWire profile change（藍牙切換） | `5:23 / 25:00 · 麥克風切換` |
| Fallback（unknown source） | `5:23 / 25:00 · 錄音中斷` |

## 螢幕 D — Lock-screen / Now-playing 控制（**不存在**）

per hero NG6 + SPEC-45 §12.1 — Linux 無 MPRemote / SMTC 對等品。loginctl 不對等。**mockup 不畫**。

## 螢幕 E — Finalizing

版型同 hero iOS E — per hero mockup L252-269。Linux delta：

- spinner: 32px CSS animated circle, stroke 3px spectyn-warning（webview 內，**不用 GTK native spinner** — 為跟 mac/Win mockup 視覺一致）
- progress bar: 240×4px, fg spectyn-warning, bg spectyn-border
- tray icon 同步切 attention 變體（best-effort，SNI `NeedsAttention`）

## 螢幕 F — Done（Takeaway card + libnotify low）

版型同 hero iOS F — 落在 main window Focus tab，card width 640px。Linux 同時發 libnotify：

```
[libnotify — focus done]
┌─────────────────────────────────────────┐
│ [spectyn mono icon 32]  Focus complete  │  summary（en：`focus.done.title` 縮短版）
│                                         │
│  25 min · 5 chunks · takeaway ready     │  body（結構化 metadata，**不放 takeaway 內容**）
│                                         │
│  [ 開啟 ]                                │  action → `spectyn://focus/done/<session_id>` deep link
└─────────────────────────────────────────┘
```

**libnotify hint 規格**：
- `urgency = low`（per hero mockup §461）— 不打斷 user
- `expire_timeout = default`（distro 處理，多數 5-10s）
- `category = transfer.complete`
- `desktop-entry = spectyn-mesh`
- **body 不含 takeaway 內容 / chip / personal observation**（per SPEC-45 §12.1 STRIDE Information Disclosure）

**Empty 變體（ASR 全靜音）**：
- libnotify **不發**（避免騷擾 — 沒語音也沒重要事件）
- main window F 卡片顯示安撫文：「本次時段未偵測到語音，已為您記錄時長」（取 `focus.empty.no_speech`）+ session 仍寫 events row
- 卡片視覺：success icon 改 `triangle-alert` 18px spectyn-muted（不是 danger，是「值得注意但非錯誤」）+ 兩個 button「重錄這次」/「完成」

## Cross-platform invariants 對齊（per hero mockup §546）

繼承全部 hero invariants（trust badge 文字 / Stop danger color / 計時器顏色 / takeaway card 尺寸 / Notification body 截字 / Interrupted 系統通知強制觸發）。Linux 額外：

- **Lucide icon bundled, 不依賴 distro icon theme** — build asset 內 inline SVG
- **GTK CSD 真窗 + WM_CLASS=spectyn-mesh** — tiling WM rule 認得
- **Tray 是 bonus，main window Focus tab 為 canonical** — GNOME 無 tray 不擋功能
- **libnotify body 不含 PII**（per SPEC-45 §12.1） — takeaway 不入 body
- **Interrupted libnotify urgency = critical** + timeout = 0（per Linux wireframe §C'）
- **Done libnotify urgency = low** + default timeout（per hero mockup §461）
- **GTK theme follow dark/light（5s 內）** — webkit2gtk < 2.40 fallback light（不爆畫面）
- **GTK accent color NOT follow** — spectyn-primary 永遠 `#8ab4f8`（保 brand trust 一致性）

## 6 大資料狀態 — Linux Mockup 視覺對映

| 狀態 | 視覺 |
|---|---|
| **理想（Ideal）** | F Done card 完整三段 takeaway + Lucide `check-circle` 64px spectyn-success + libnotify low 通知 |
| **空白（History）** | main window Focus tab 內 history 區：mono SVG illustration 192px spectyn-muted + `focus.empty.history` + 「前往 Focus」按鈕 |
| **空白（ASR 無語音）** | F 卡片 Lucide `triangle-alert` spectyn-muted + `focus.empty.no_speech` 安撫文 + 重錄/完成雙按鈕（**libnotify 不發**） |
| **極限（Limit）** | C chunk `99+` chip / F takeaway > 800 字截斷（hero invariant — fade-out gradient + 「看完整摘要」CTA） |
| **錯誤（Error）** | B' No-mic device-error（Lucide `mic-off` 64px spectyn-danger + 「打開音訊設定」CTA）/ Interrupted libnotify critical / `LINUX-SCREEN-LIBNOTIFY-NO-SERVICE` headless server fallback main window inline 訊息 |
| **局部（Partial）** | E `focus.partial.chunk_failed` inline Lucide `triangle-alert` 14px spectyn-warning |
| **載入中（Loading）** | E spinner-32 spectyn-warning + progress bar 40% + tray icon attention 變體（best-effort） |

## 已決（per Linux wireframe §已決 + hero invariants）

繼承 Linux wireframe §已決 7 項，mockup 層追加：

1. **Lucide bundled, 不靠 distro icon theme** — 跨 distro 視覺一致 > follow 系統風格
2. **GTK accent color 不 follow** — 保 spectyn brand trust 一致
3. **libnotify Done body 結構化 metadata only**（per SPEC-45 §12.1 PII 約束）
4. **Interrupt 來源動態 body line 1** 四種對映（mic-grabbed / sleep / bt-switch / unknown）
5. **Tray icon 24×24px single-color SVG**（SNI 規範） + GNOME Shell mono 限制下靠 main window title 補救

## 開放問題（mockup 層面）

1. **GTK light mode token 對映**：v0.6.0 webkit2gtk webview 強制 dark；light 對映歸 SPEC-02 §7 補完表。是否提早在本檔列「臨時 light 對映」？提案：暫不，等 SPEC-02 統一處理。
2. **Tray attention 變體在 GNOME 不可見的二級救濟**：main window title 加 `[Recording]` prefix 是否夠？或要 panel notification（GNOME 沒 tray 但有 notification banner）？提案：兩條並行（title prefix + persistent notification when window minimized）— prototype 鎖細節。
3. **Interrupted libnotify `sound-name = dialog-warning`** 在無 sound theme 的 minimal distro（Alpine / Void）會 silent — 是否 fallback `bell`？提案：仰賴 freedesktop sound theme 標準，無 sound 接受降級（per Linux best-effort 原則）。
4. **Lucide SVG `currentColor` 在 GNOME Shell SNI mono 限制下失效**：interrupted attention 變體的 spectyn-danger 紅點可能被吃掉變灰。提案：接受視覺降級（per Linux best-effort），main window 加同步提示補強。

## 下一步

→ 進 [Linux Prototype（待補）] 鎖定 SNI tray click → window raise 行為、libnotify action 回呼 D-Bus 細節、DE detection fallback 鏈、GTK theme switch 過渡時序、whisper.cpp 進度回報 → tray attention 切換 timing。
