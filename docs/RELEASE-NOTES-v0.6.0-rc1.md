# phantom-mesh v0.6.0-rc1 — 發行說明

**Tag**: `v0.6.0-rc1`
**日期**: 2026-05-25
**目標正式版（GA，General Availability，正式發行）**: 2026-06-15
**狀態**: Release candidate 1（候選發行版 1）——四支柱核心功能完整，整合測試進行中

---

## §1 重點摘要（TL;DR）

v0.6.0 是第一個在單一 Mac 維護者環境上端對端交付 phantom-mesh BIG-GOAL（大目標）全部四支柱（pillar）的發行版。此候選發行版把範圍切在「核心可運作、延後清單誠實列出」這條線上。

- **P1 Mesh（網狀網路）** — 透過 mDNS（multicast DNS service discovery，多播 DNS 服務探索）做叢集探索、節點（peer，對等節點）註冊、跨作業系統派工（從單一 orchestrator（協調器）即可觸及 macOS / Linux / iOS / Android worker（工作節點））。`phantom cluster status` 與 `phantom cluster peers` 隨此版本一同發行。
- **P2 Multimodal（多模態）** — 為飲食 / 專注 / 習慣事件設計的擷取管線（capture pipeline），可選擇附加影像；Gemini 多模態分析路徑回傳結構化的 `EventMeta`（event metadata，事件中繼資料），採「裝置端協調、裝置外推論」模式。
- **P3 Evolve（演化）** — 技能庫（skill bank，一個由本地 SQLite 支援、可重用 agent（代理人）技能的函式庫）的 FTS5（full-text search version 5，全文搜尋第 5 版）關鍵字召回後端與讀取 RPC（remote procedure call，遠端程序呼叫）已隨此版本落地，但在 v0.6.0 **預設關閉**：位於 `experimental-memory`（feature flag，功能旗標）之後，且技能寫入端（daily review → 技能庫的 producer，生產者）尚未接線、自動萃取 loop（迴圈）仍在 `experimental-curator` 之後且未完成（見 §6）。embedding（嵌入向量）語意召回延後至 v0.7.0。
- **P4 Encryption（加密）** — 採 age v1（現代化檔案加密格式）的逐事件加密、ed25519（Edwards-curve digital signature，愛德華曲線數位簽章）身分、用於各用途隔離的 HKDF（hashed key derivation function，雜湊金鑰衍生函數）子金鑰，以及用於託管第三方權杖的 OAuth（open authorization，開放授權）broker vault（中介伺服器保險庫）。

如果你只讀一段：v0.6.0-rc1 是四支柱從「願景式 spec（規格）」蛻變為「開發者筆電上可呼叫的二進位執行檔」的切點。針對非維護者使用者的正式生產就緒度，目標訂在 2026-06-15 的 GA。

---

## §2 自 v0.5.0 以來的新增項目

### Spec 目錄

- **51 份 spec 的深度目錄**已落地於 `docs/superpowers/specs/v060-deep-spec/`。每份 spec 遵循 18 節模板（metadata → TL;DR → goals → architecture → data model → API contracts → flows → alternatives → changelog）。
- **5 項跨 spec 不一致性修復**於 Wave 12 審查期間套用（deep-link scheme 統一、fingerprint 寬度、EventKind enum 對 string、broker URL 正規化、identity 檔案路徑正規化）。
- **11 項 spec 小修（minor fixes）**於 Wave 13 完成——格式、缺漏的 `verifies via:` 標籤、Mermaid 渲染修正、詞彙表補充。

### Wire types（單一事實來源 Rust → TypeScript）

- **18 個 wire module（線路型別模組）**以 Rust 為單一事實來源（source of truth），並由 `ts-rs`（TypeScript-from-Rust binding generator，從 Rust 產生 TypeScript 繫結的工具）為桌面 / 行動 / 網頁消費端產出 **204 個自動生成的 `.ts` 檔案**。
- 這些模組涵蓋：叢集探索、節點註冊、事件擷取、事件儲存、加密、身分、OAuth vault、多模態分析、技能庫（skill bank）、遙測、上手導引（onboarding）、每日回顧、demo runner（示範執行器）等等。

### Stage 4 真實實作

- **18 個 wire module 中有 12 個**達到 Stage 4（以真實實作支撐 wire type——非偽碼、非樁碼（stub））。
- **6 個模組**在 Stage 4 為部分完成——逐項清單（哪些是真實、哪些已延後）請見 `docs/superpowers/V0_7_0_DEFERRAL_INVENTORY.md`。

### 整合測試（Phase E）

- **48 個整合測試**已落地，涵蓋 V1（vault）、V3（事件加密往返）、V8（多模態分析管線）。
- V4（叢集探索）、V5（節點註冊）、V7（每日回顧）、V12（上手導引）的整合測試於本次工作階段進行中——預計於 GA 前落地。

### Demos（示範）

- **E006 30 秒 hello demo** 端對端可運作。`bash scripts/demo-30sec-life-hello.sh` 在一台 Mac MBA（MacBook Air）上於 30 秒內跑完完整的擷取 → 加密 → 分析 → 儲存 → 召回迴圈。

### Tooling（工具鏈）

- `scripts/ai/dispatch.sh`（跨工具派工墊片，用於 opencode / codex / agy / claude subagent）以正式的 AI 協調進入點身分發行。
- `.ai-shared/` 共享記憶體目錄已建立，讓外部 CLI 工具看到與 Claude Code 相同的專案事實。

---

## §3 本版「未納入」的項目（延後至 v0.7.0+）

完整延後清單位於 `docs/superpowers/V0_7_0_DEFERRAL_INVENTORY.md`。重點如下：

### Secret storage（密鑰儲存，P4）

- **macOS Keychain 原生繫結** — 已延後。macOS 退回（fall back）使用 phantom home 目錄下的加密檔案儲存。
- **iOS Keychain 原生繫結** — 已延後。iOS 退回使用 app sandbox（應用程式沙箱）內的加密檔案儲存。
- **Linux Secret Service**（D-Bus） — **已納入**本版。GNOME Keyring / KWallet 接取可運作。
- **Android Keystore** — 已延後。Android 退回使用加密檔案儲存。

### LLM 供應商（11 個中有 8 個延後）

**本版已納入**（3 個供應商）：

- `groq` — Groq 雲端，每日測試，主要低成本路徑。
- `anthropic` — Anthropic API，含串流（streaming）+ prompt caching（提示快取）。
- `gemini` — Google Gemini，含多模態。

**延後至 v0.7.0**（8 個供應商）：

- `openai` — OpenAI API
- `cerebras` — Cerebras 雲端
- `opencode` — OpenCode CLI 供應商
- `cloudflare` — Cloudflare Workers AI
- `ollama` — Ollama 本地
- `claude_cli` — Claude Code CLI 子行程
- `codex_cli` — Codex CLI 子行程
- `llamacpp` — llama.cpp 本地

呼叫已延後的供應商會回傳 `ConfigInvalid`，並附上指向延後清單的指標。

### Embedding / 語意召回

- **`ort`（ONNX runtime）+ `all-MiniLM-L6-v2`** embedding 管線已延後。v0.6.0-rc1 的技能召回僅支援 FTS5 關鍵字；語意向量召回於 v0.7.0 抵達。

### 發布自動化

- **App Store Connect API** 自動發布 — 已延後。TestFlight 建置目前以手動上傳。
- **Play Developer API** 自動發布 — 已延後。APK sideload（側載）為受支援的安裝路徑。

### 延後至 v0.7.0+ 的較大型功能

- **SPEC-70 Web dashboard** — 給非 CLI 使用者的瀏覽器 UI。
- **SPEC-71 Multi-user household** — 跨家庭成員的共享 mesh，具備逐使用者的加密邊界。
- **SPEC-72 Paid broker** — 商業託管 broker 層級（以 IP（智慧財產，intellectual property）邊界閘控，見 `docs/design/COMMERCIAL-DESIGN.md`）。
- **SPEC-73 Watch surface** — Apple Watch / Wear OS 擷取介面。
- **SPEC-74 Extensions** — 第三方能力擴充協定。

---

## §4 自 v0.5.0 起的破壞性變更

### Wire type 統一

- `life_node::storage::EventMeta`（舊版）現具備一座 `From → event_storage_wire::EventMeta` 橋接。
- `kind: String` 對應到新的 `EventKind` enum：
  - `"food_log"` → `EventKind::FoodLog`
  - `"focus_session"` → `EventKind::FocusSession`
  - `"habit_log"` → `EventKind::HabitLog`
  - 其他任何值 → `EventKind::Text`（後備值，在 `subkind` 欄位中保留原始字串）。
- 既有的磁碟上事件可前向讀取（read forward）而無須重寫。新的寫入採用 enum 形式。

### Deep-link scheme（深層連結配置）

- `phantom-mesh://...` → `phantom://...`
- Wave 12 期間落地了 5 項 spec 修補，以翻轉每一個有文件記載的 URI handler。
- 桌面註冊（macOS `Info.plist`、Linux `xdg-mime`）在升級後需重新安裝一次——見 §5。

### Identity fingerprint 寬度

- 舊版：6-hex（24 位元）短 fingerprint。
- 新版：依 `SPEC-12 §7` 採 12-hex（48 位元）fingerprint。
- 已儲存的身分會在首次讀取時惰性遷移（lazily migrated）；既有的 6-hex 節點在直到 v0.7.0 的棄用期間內仍持續被接受。

---

## §5 遷移指南

拉取 v0.6.0-rc1 之後：

1. **重啟 `phantom serve`** 讓新的二進位執行檔接手。serve 迴圈會自行偵測 schema 升版，並在 phantom home 目錄下寫入一個標記檔。
2. 位於 `~/.phantom-mesh/events/`（Linux/macOS）或相應 app-sandbox 路徑（行動裝置）的**既有事件**會透過舊版讀取器加上新的 `From` 橋接**前向讀取**。無須、也不建議批次重寫。
3. **更新桌面上的 deep-link 註冊**：
   - macOS — 重新安裝 app bundle，讓 `Info.plist` 中新的 `CFBundleURLSchemes` 條目被 Launch Services 接取。
   - Linux — 重新執行 `xdg-mime default phantom.desktop x-scheme-handler/phantom`。
4. **行動 app** 持續運作而無須遷移。deep-link scheme 變更對使用者是透明的，因為行動建置一向只發行單一 scheme。
5. vault 中的 **OAuth 權杖**會在首次存取時透過新的 HKDF 子金鑰路徑惰性重新加密。無須動作。

---

## §6 已知問題

- **iOS / Android Keychain 後備** — Stage 4 原生繫結延後至 v0.7.0。後備方案（app sandbox 內的加密檔案）可運作，但無法受益於作業系統層級的 secure enclave（安全隔離區）。
- **8 個 LLM 供應商回傳 `ConfigInvalid`** — 清單見 §3。3 個主要供應商（`groq`、`anthropic`、`gemini`）涵蓋所有 in-tree（樹內）的 demo 與 selftest 路徑。
- **一台開發用行動裝置離線** — 測試機群中一台 iPhone 13 mini 目前離線（電池沒電）；非發行阻擋項，僅為跨作業系統派工測試面的透明度而提及。
- **2 個 `service::macos` 測試失敗** — `launchctl` plist 測試在維護者機器上因既有的基礎設施怪癖而失敗（沙箱化的測試執行器無法與 `launchd` 通訊）。執行期路徑可運作；僅測試 fixture 受影響。已追蹤至 v0.6.0 GA。
- **P3 Evolve 路徑預設為 experimental（實驗性）** — 技能庫的 FTS5 記憶後端與 3 個讀取 RPC 端點位於 `experimental-memory`（feature flag）之後，**預設 build 不啟用**；即使啟用，在未設定記憶庫前端點會 fail-closed（故障即關閉）回 `503`。技能萃取 producer（`phantom skill extract --commit`）尚未提供，evolve→萃取→儲存自動 loop 仍在 `experimental-curator` 之後且未完成。因此**預設安裝的 v0.6.0 不會自動產生或曝露技能庫**——P3 演化在 GA 屬實驗性能力，完整接線追蹤至 v0.7.0（決策：2026-06-02 採「GA 維持 gated + 誠實標注」，不在截止前 13 天半接線上線）。

---

## §7 致謝

phantom-mesh v0.6.0-rc1 是在 `docs/superpowers/BIG-GOAL.md` 所記載的單一維護者 dogfood（自家試吃）模式下開發：一位人類審查者兼指揮、由 AI agent 處理絕大多數程式碼合成，而專案本身在整個開發過程中即作為維護者的每日回顧工具運行。

- **Spec 目錄審查**（Wave 12） — codex CLI 與 Claude Code subagent 平行進行，由人類在衝突上擔任裁決者。
- **Phase A / B / C / D / E** 整合掃描 — 透過 Claude Code 的多 agent 平行模式經由 `scripts/ai/dispatch.sh` 協調，各階段依「經驗教訓」的分塊規則上限為 ≤200 行、≤5 分鐘。
- **跨工具記憶體** — `.ai-shared/memory/` 確保 opencode、codex、agy 與 Claude Code 全都看到相同的專案事實。

若沒有這套多工具協調模式，51 份 spec 目錄加上 18 個 wire module 加上 48 個整合測試，根本塞不進距 GA 僅 24 天的時間窗。

---

## §8 升級路徑

```sh
# pull latest from the private maintainer repo
git pull origin main

# rebuild the binary
cd core && cargo build --release --bin phantom

# verify the install
core/target/release/phantom selftest

# run the 30-second end-to-end demo
bash scripts/demo-30sec-life-hello.sh
```

預期的 `phantom selftest` 輸出：`scripts/selftest.d/` 下每一項已註冊的檢查都通過，僅有 §6 所述的 2 個已知 `service::macos` 失敗以唯一的紅色行（red lines）浮現。30 秒 demo 會寫入一筆樣本飲食 / 專注 / 習慣事件、加密它、執行多模態分析樁、儲存結果並讀回——全程在一台 Mac MBA 上於 30 秒實際時間內完成。

若 `phantom selftest` 回報超過這 2 個已知失敗，請視為 bug，並在晉升至 GA 前針對 `v0.6.0-rc1` 提報。

---

## §9 統計數據

- **51** 份 deep catalog 中的 spec
- **18** 個以 Rust 為單一事實來源的 wire module
- **204** 個自動生成的 TypeScript 繫結檔案
- **48** 個整合測試（Phase E）已落地；更多於 GA 前進行中
- **12 / 18** 個 wire module 達完整 Stage 4 真實實作；6 個部分完成
- **3 / 11** 個 LLM 供應商納入本版；8 個延後
- **5** 項 Wave 12 的跨 spec 不一致性修復
- **11** 項 Wave 13 的 spec 小修（minor fixes）
- **~13,000** 行位於 `core/` 下的 Rust
- **~17,000** 行位於 `docs/superpowers/specs/v060-deep-spec/` 下的 spec markdown
- **1** 台 Mac MBA + 僅限 Tailscale 的開發合約（開發隔離合約——僅在維護者受僱於他處期間使用個人裝置 / 網路 / 身分 / AI 訂閱）

---

## 變更紀錄（Changelog）

- **2026-05-25** — v0.6.0-rc1 初版草稿。把候選發行版切在「四支柱在維護者筆電上可呼叫、並附誠實的延後清單」。目標訂於 2026-06-15 GA。
