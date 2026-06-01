# 行動裝置 vs 桌面 — 各平台在 mesh（網狀協作網路）中能做什麼

**狀態（Status）**：🟡 DESIGN — 記錄使用者 5/2 的決策；目標為 v0.1.0 上線
**配套文件（Companion）**：`docs/CONTRIBUTOR-FUNNEL.md`（recipe 上游流程）、`docs/CO-EVOLUTION.md`（三層模型）
**權威性（Authority）**：本文件決定各平台能扮演哪些 mesh 角色；由 SPEC-FREEZE-V1 §11 強制執行。

問題是：當 OSS（開放原始碼軟體）上線、任意使用者在自己的裝置上跑 phantom 時，
每個平台在 mesh 裡實際上能「做」什麼？答案是不對稱的 —
桌面可以完整發揮，行動裝置受作業系統（OS）烘焙進去的硬性限制約束，
還有第三種「Termux 逃生口」只能在 Android 上運作。

---

## 0. 使用者的問題框架（5/2）

> 「Android / iOS 正式版大概沒辦法做這件事；但桌面版應該可以吧？
> 或者我們也該把 Android/iOS 做成終端機風格的套件，
> 配上傳統 UI 和大致傳統的功能？」

→ 是的，桌面可以。行動裝置無法自我修改程式碼。**UI 風格與
mesh 參與能力是各自獨立的決策。** 本文件把兩者都講清楚。

---

## 1. 各平台的硬性限制

### 1.1 桌面（macOS / Windows / Linux）— 完整發揮

✅ 可以：
- `cargo build` / `cargo test` / `cargo check`（有工具鏈可用）
- `shell` / `fork` / `exec`（OS 允許）
- 在磁碟上重寫自己的二進位檔（binary），並透過 `phantom upgrade` 重新部署
- 跑 autoevolve（自我演化）迴圈，修改 `core/*.rs`
- 收到一份 recipe，套用其 `git format-patch` 後重新建置
- 參與 **Mesh 即 CI** 開發熱重載（hot-reload）迴圈

❌ 不能：
- （沒有根本性的限制 — 只有實務上的限制，例如 macOS 上的
  Apple gatekeeper（守門員機制），但這些有臨時的 codesign（程式碼簽章）變通做法 — 見
  `commit 1f27127`）

### 1.2 iOS（App Store + 沙盒側載 sideload，兩者相同）

❌ 硬性沙盒（sandbox）限制 — 這些是 OS 層級，不是政策層級：
- 沒有 `fork()` / `posix_spawn()`（核心 kernel 禁止沙盒 app 使用）
- 沒有 JIT（即時編譯）（嚴格強制 W^X）
- 執行期不可讀取或修改 app bundle（應用程式包）
- 沒有長時間執行的背景行程（被切到背景後約 30 秒就結束）

❌ App Store 政策再加上：
- 不允許「動態下載並執行程式碼」
- App 必須隨附所有程式碼；安裝後不可從原始碼重新建置
- 沒有 `cargo`、`git`、`make` — 任何會是獨立行程的東西都不行

→ **iOS 上的 phantom 永遠無法跑 autoevolve / cargo / Mesh 即 CI**。
這是一道硬牆，不是我們之後會修掉的暫時性限制。

iOS 上的 phantom 「可以」做的事：
- 跑任何 in-process（行程內）的 Rust 程式碼（靜態連結進 app）
- 呼叫 HTTPS API（LLM 供應商、透過 Tailscale 連到的 mesh 對等節點 peer）
- 在 app 容器內（`Documents/`、`Library/`）讀寫檔案
- 在前景時於 `127.0.0.1` 跑本地 TCP 監聽器
- 渲染 TerminalShell（終端機外殼，Tauri WebView）

### 1.3 Android Tauri APK（Play Store + 側載 sideload）

❌ 沙盒限制（與 iOS 同根源，但稍微寬鬆些）：
- 預設沒有 `cargo` / `git` / 外部工具鏈
- App 容器是可寫入的範圍
- 可有前景服務（foreground service）（類似背景 daemon（常駐服務），但
  帶有持續性通知）

❌ Play Store 政策：
- 與 App Store 相同的「不可動態下載程式碼」規則
- 不可自我修改 APK 內容

→ 就我們的目的而言，**Android Tauri APK = 與 iOS 相同的角色**。

### 1.4 Android Termux（逃生口，僅限側載 sideload）

✅ Termux 是跑在 Android 核心上的 Linux 使用者空間（userspace）：
- 有 shell / fork / exec / `cargo`（需安裝套件）
- 可在裝置上從原始碼建置 phantom
- 可作為長存的背景行程運作（搭配 WAKE_LOCK）
- 已出貨：`phantom-aarch64-linux-android` 原始二進位檔
  （`feat/android` commit `963c3fe`）

❌ 但是：
- 不可透過 App Store / Play Store 散布
- 僅限進階使用者 — 多數使用者不會安裝 Termux
- Apple 沒有對應品 — 不存在「iOS 版 Termux」

→ Android Termux = **與 Linux 桌面相同的能力**，透過側載 sideload 取得。
它是給想在手機上跑 Mesh 即 CI 的進階使用者的逃生口。

---

## 2. UI 風格與 mesh 參與能力彼此獨立

一個常見的混淆：「如果行動裝置無法自動修改程式碼，那把 TerminalShell
放上去有什麼意義？」這兩件事是正交（orthogonal，互不影響）的：

| | UI 風格 | 能否自我修改程式碼 |
|---|---|---|
| Mac/Win/Linux 上的桌面 TerminalShell | 終端機風格 | ✅ |
| 桌面傳統 GUI（Tauri 預設風格） | 點按/點擊 | ✅（UI 不影響 mesh） |
| 行動 TerminalShell（5/9 demo 路線） | 終端機風格 | ❌ 沙盒封鎖 |
| 行動傳統 iOS/Android UI | 點按/滑動原生 | ❌ 沙盒封鎖 |

→ 在行動裝置上選 TerminalShell 是一個 UX（使用者體驗）選擇（對 geek（技術控）友善 +
跨 5 平台視覺一致），而非能力選擇。不論 UI 為何，我們都會
受到相同的 iOS 沙盒限制。

→ 決策是：**行動 UI 對使用者來說長什麼樣子？**
功能上，行動 app 就是沙盒所允許的那一部分 mesh 功能子集。

---

## 3. 行動裝置在 mesh 中能扮演的三種角色（不需自我修改）

即便行動裝置無法自動修改自己的程式碼，它仍能以三種有意義的方式
參與 mesh：

### 角色 1 — 沙盒工作者（Sandbox worker）（SPEC-FREEZE-V1 §11.2 / §11.5 已規格化）

Mac 協調者（coordinator）派發一個任務 → 行動對等節點接受（透過
`/rpc/squad/dispatch`）→ 在 app 的行程內本地跑 agent → 回傳結果。

```
worker_caps on iOS / Android Tauri:
  ["file_in_container", "memory", "web", "subagent", "llm_local"]
```

符合這些 caps（能力）的任務：
- 透過 HTTPS API 跑一次 LLM 補全（groq / openai / anthropic / gemini）
- 在 `Documents/`（加沙盒前綴）內讀寫檔案
- 搜尋本地記憶體（sled 為後端）
- 透過 `web_fetch` 擷取一個 URL
- 衍生一個行程內子代理（in-process subagent）（tokio task，不是 OS 行程）

會被自動繞過（route AROUND）行動裝置的任務（Squad Pipeline 派發器
會過濾掉）：
- `shell`（沒有 fork）
- `git_*`（沒有 exec）
- `cargo_*`（沒有工具鏈）
- `xcode_simctl` / `spotlight_search`（僅限 macOS）

### 角色 2 — 唯讀 mesh 觀測者 / 儀表板

行動裝置的 TerminalShell 連到 Mac 協調者，並即時渲染 mesh
狀態：

- 頁首：5 個對等節點圓點，顯示 online/offline/warn
- 捲動回看（Scrollback）：已派發的任務 + 每個對等節點的串流輸出
- 跨對等節點派發事件的記錄
- 來自 Mesh 即 CI 執行的建置/測試狀態（桌面開發模式啟用時）

使用情境：開發者在 Mac 上寫程式，同時看著 iPad 顯示即時的測試
矩陣。iPad 變成一個 **mesh 儀表板**，而不是工作者。

這個角色不需要任何額外能力 — 唯讀 HTTPS 連到
Mac 協調者的 `/rpc/peers` + `/api/dispatch/log`（後者尚未
實作；v0.2）。對隱私友善：除了 mesh 狀態外，沒有東西離開 Mac。

### 角色 3 — 第 1 層（Tier 1）貢獻產生者

依據 `docs/CONTRIBUTOR-FUNNEL.md` §3，Tier 1 = 沙盒擴充
（`~/.phantom-mesh/extensions/{prompts,skills,hooks}/`）。行動裝置的
autoevolve「可以」寫入 extensions（它在自己資料容器內的個人化客製）。
它只是不能寫入 `core/*.rs`。

所以行動使用者可以：
- 客製化自己 phantom 的 prompts（Tier 1）
- 打造個人 skills（Tier 1）
- 把這些當成 recipes 發布（僅 Tier 1 — 不附 patch）
- 當他們的 recipe 被別人採用時，在 CONTRIBUTORS.md 中獲得署名

→ 行動裝置 = **CONTRIBUTOR-FUNNEL Tier 1 的一等參與者**。
他們只是搆不到 Tier 2/3（那需要動到核心程式碼，而行動裝置
做不到 — 但他們的貢獻是真實且可歸屬的）。

---

## 4. v0.1.0 上線決策：一個行動建置，用 TerminalShell

針對 5/15 OSS 上線，**只出一個 iOS / Android Tauri 建置**：
- UI：TerminalShell（與桌面 `/term` 相同的 React 元件）
- 能力：沙盒工作者（角色 1）+ 唯讀觀測者（角色 2）
- 散布：側載 sideload 的 IPA / APK（尚未上 App Store / Play Store —
  付費的 Apple Developer Program / Play Console + 審查需 1-3
  週；不在 v0.1.0 範圍內）

→ 理由：TerminalShell 對 geek 友善、跨全部 5 平台視覺統一，
而且不需要我們為 v0.1.0 打造兩個產品（傳統 + 終端機）。
那是過早優化（premature optimization）。

5/9 demo 賣點不變：「同一個 binary、同一個 UI、5 個平台
全在 mesh 裡。」

---

## 5. v0.2+ 選用的傳統 UI 外殼

如果真實使用者（上線後）回報「TerminalShell 在 4 吋
iPhone 螢幕上做非技術性任務很難用」，v0.3+ 會新增一個傳統
行動 UI **作為包裹外殼**，套在同一個 daemon 之外：

```
Tab-based mobile app (Tauri):
  ├─ Tab 1: Mesh status
  │     Live peer dots, current dispatched tasks,
  │     "tap to approve incoming task"
  ├─ Tab 2: Settings
  │     Provider keys (groq / anthropic / etc.)
  │     Tailscale onboarding + cluster_secret
  │     worker_caps toggle (accept dispatch on/off)
  ├─ Tab 3: Notifications
  │     Push when peer dispatches a task to this phone
  │     Approval flow (auto-approve known agents, prompt for unknown)
  ├─ Tab 4: Power user → switch to TerminalShell
  │     Same component as desktop /term
```

實作成本：在 v0.1.0 之上約 1 週（一位工程師）。
**不在 5/15 範圍內。** 會以 v0.3 衝刺為目標（5/22 → 5/29 或
更晚）。

→ 底層引擎維持同一個 Tauri runtime + 同一個 phantom
行程內 daemon。只有 UI 層分歧。與桌面的程式碼重用率 > 90%。

---

## 6. 第三種：Termux-on-Android 逃生口

給已經有 Termux 的 Android 進階使用者：

```
$ pkg install rust git
$ cargo install --git https://github.com/markl-a/phantom-mesh \
    --bin phantom
$ phantom serve
```

這會在 Android 上安裝完整的、等同桌面的 phantom。它可以：
- 跑 autoevolve（修改自己那份 `core/*.rs` 的副本）
- 參與 **Mesh 即 CI** 開發熱重載
- 搆到 Tier 2 / Tier 3 貢獻
- 透過 Termux 的 `termux-wake-lock` 作為前景服務運作

這個逃生口：
- 已有出貨的二進位檔（`feat/android` 上的 commit `963c3fe`）
- 另外以 v0.2 路線記錄（不在 5/15 預設安裝中）
- 行銷標語：「Android 手機 = 可攜的 Linux 完整工作者」

iOS 沒有對應品。Apple 的沙盒是 OS 層級的；沒有 iOS 版 Termux
存在，且在不越獄（jailbreak）的情況下也不可能存在。想要完整參與
mesh-CI 的 iOS 使用者「必須」使用桌面。

---

## 7. 各角色的決策矩陣

| 能力 | 桌面 | iOS / Android Tauri（沙盒） | Android Termux |
|---|---|---|---|
| 跑 autoevolve 修改自己的 `core/*.rs` | ✅ | ❌（沙盒） | ✅ |
| `phantom upgrade` 換掉 binary | ✅ | ⚠ 需重新側載 sideload | ✅ |
| 接收 `/rpc/squad/dispatch`（角色 1） | ✅ | ✅（依沙盒 cap 過濾） | ✅ |
| TerminalShell 唯讀 mesh 觀測者（角色 2） | ✅ | ✅ | ✅ |
| 客製化 prompts/skills/hooks（Tier 1） | ✅ | ✅ | ✅ |
| 向 broker（中介伺服器）發布 Tier 1 recipe | ✅ | ✅ | ✅ |
| 發布 Tier 2 recipe（動到 `scripts/`、`tests/`） | ✅ | ❌ 無法產生 patch | ✅ |
| 觸發 Tier 3 PR（動到 `core/*.rs`） | ✅ | ❌ 無法產生 patch | ✅ |
| Mesh 即 CI 熱重載參與 | ✅ | ❌ 無法重建 | ✅ |
| 在 CONTRIBUTORS.md 中的 Co-Authored-By | ✅ | ✅（僅 Tier 1） | ✅ |

→ 行動裝置是「在角色 1 + 角色 2 + Tier 1 為一等公民；在 Tier
2/3 與 dev-CI 上缺席」。Termux 則「彷彿是手機上的 Linux 桌面」。

---

## 8. 衝刺（Sprint）影響

這個決策為衝刺規劃定下哪些事：

### v0.1.0（5/1 → 5/15）— 最小行動裝置

- ✅ 出單一 Tauri iOS IPA + Android APK
- ✅ TerminalShell UI（已建好 — `9548273`）
- ✅ 角色 1（沙盒工作者）已接線（Squad-a worker_caps + Squad-b
  `/rpc/squad/dispatch` 已出貨）
- ⏳ 角色 2（唯讀觀測者）需要 `/api/dispatch/log` 端點
  （尚未開始；不大 — 約 2 小時）
- ❌ Termux 路線已記錄，但不在預設安裝路徑上
- ❌ 傳統行動 UI：延後

### v0.2（5/15 → 5/22）— Termux + Tier 1 發布

- Termux 安裝腳本作為已記錄的次要路線
- `phantom evolve publish` + ed25519 簽章（CONTRIBUTOR-FUNNEL §5）
- 行動使用者現在可以發布 Tier 1 recipes

### v0.3+ — 行動傳統 UI（若有需求）

- 包裹 TerminalShell 的分頁式（tab-based）行動外殼
- 派發核可用的推播通知
- 工作者接受/拒絕的 UI

→ 「行動裝置該不該用傳統 UI？」這個決策**延後到
上線後的真實使用者回饋**。我們不臆測 — 先出一個
產品、看使用者怎麼說，需要時再拆分。

---

## 9. 對 demo（5/9）的意涵

**5/9 的講稿路線維持不變**：

> 「Phantom-mesh 跑在五個平台上，同一個 binary。每個平台都
> 以對等節點的身分參與 mesh。iOS 與 Android 跑一個沙盒
> 子集 — 它們無法 shell out（Apple 的規則，不是我們的），但它們
> 處理 LLM 端的工作，像是檔案分析、網頁擷取、prompt
> 分類。像 git 操作或 shell 指令這種重活
> 會自動繞送到 Mac、Windows 或 Linux 對等節點。Mac 上的派發器
> 代理（dispatcher agent）藉由查看每個對等節點在 `/rpc/ping` 上的
> `worker_caps` 欄位想清楚這件事。所以當我在這裡說『分析這個
> 程式碼倉庫（codebase）』時，偵察（recon）送到一台 Windows 桌面、日誌
> 加值（enrichment）跑在 Linux 雲端節點、嚴重性分類跑在這支
> iPhone 上，而 Mac 上的一個合成器（synthesizer）代理把結果合併。」

→ 行動裝置上的沙盒變成一個**特性而非限制**。「行動裝置
跑 LLM 動腦的工作，因為那正是行動裝置能安全做的事。」

---

## 10. 反模式：別試圖繞過

我們明確「不」做的事（因為它們會違反 Apple/Google
政策或危及安全）：

- ❌ 用 App-Bound Domains 的把戲在 iOS 上做動態載入
- ❌ 出貨直譯器（interpreter）（Lua / JS），實質上讓擴充
  在 Tier 1 範圍之外修改行為
- ❌ JailbreakMe 式、針對已越獄 iOS 的變通做法
- ❌ 透過「資產下載（asset downloads）」夾帶二進位 patch

這些要麼會讓 app 在商店散布時被拒（當我們最終
要走那條路時），要麼會違反信任模型。

---

## 11. 一段話的總結

**桌面 = 完整發揮（Mesh 即 CI + autoevolve + 全部 3 層貢獻
層級）。行動沙盒（iOS / Android Tauri）= 角色 1 沙盒工作者
+ 角色 2 觀測者 + 角色 3 Tier 1 貢獻者；依 OS 設計，永遠
無法自我修改程式碼。Android Termux = 逃生口，提供完整
等同桌面的能力，僅限進階使用者。v0.1.0 上線時推出
一個行動產品（Tauri 上的 TerminalShell）；若使用者需求足以支撐，
v0.3+ 可能新增一個傳統 UI 包裝層。**

---

## 參考資料

- `docs/SPEC-FREEZE-V1.md` §11.2 — iOS 沙盒工作者規格
- `docs/SPEC-FREEZE-V1.md` §11.5 — Android Tauri 沙盒規格
- `docs/SPEC-FREEZE-V1.md` §3 — 沙盒子契約
- `docs/CONTRIBUTOR-FUNNEL.md` §1-§3 — recipe / broker / PR 流程
- `docs/CO-EVOLUTION.md` §38-69 — 三層模型
- `core/src/mesh.rs::PeerStatus.worker_caps` — 執行期能力宣告
- `feat/android` commit `963c3fe` — Termux 原始二進位檔已出貨
- App Store Review Guideline 3.2.2 — 「不可動態下載程式碼」規則
