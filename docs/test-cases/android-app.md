# Android App 平台 — 全測試用例庫 v1 (2026-06-12 · Charter Wave 3)

> **覆蓋範圍**: Android app surface 的**出貨真相**（= `App.tsx:149` 無條件 `return <AppTemplate/>`，3-tab 殼：對話/機器/設定）。只對「已出貨的 3-tab」寫可過驗收；7-tab `MobileShell` / `MobileDispatch` / `MobileDailyReview` / `MobileRecall` / `MobileOnboardingV2` / swarm 死碼相關測試一律標 `[v0.7.0 deferred]`。
>
> **從屬關係**: 本檔從屬 `../superpowers/BIG-GOAL.md`（INV-1 apex）、`../superpowers/DOCUMENTATION-CHARTER.md`（施工憲法，§A.3 INV-15/16 + §B BRK-10/11 + §F Wave 3 android slice）。不得改寫上層。
>
> **出貨真相 re-baseline（Charter P-12 / BRK-11 強制對帳）**: 文件以 **3-tab AppTemplate** 為準，**不對 7-tab 死碼寫驗收**。所有指向死碼的 case 標 `[v0.7.0 deferred]`，移出 v0.6.0 ship gate。
>
> **docs-only 鐵則**: 本檔只記**可重複測試的驗收條件**。任何 code 問題（硬編 IP / 空 secret / swarm 反目標 / skill_wire unimplemented / 缺後端 primitive）一律記為 `[現況 FAIL / code-backlog]` + owner + 追蹤 issue（無則標 `[需開 issue]`）+ wave 落點，**絕不在本 slice 修碼**。
>
> **編號規約**: `ANDAPP-{feat}-{nnn}`（feat ∈ ONBOARD / CONN / CHAT / DISP / SENSE / SWARM / MEM / SUP / LEAK / SEC / DEAD / EMPTY / PLAT）。永不重用。

---

## §0. Schema legend

共用 token/schema 字典見 [`README.md`](README.md)；本檔只列 Android app 端補充。

每條 case 欄位（對齊 `mac.md` v2 格式，CLI runner / Appium / Maestro 可直讀）：

| 欄 | 意義 |
|---|---|
| **ID** | 唯一識別 (永不重用) |
| **Type** | `unit` / `integ` / `e2e`(Maestro/Appium/WebView drive) / `manual` / `static`(原始碼靜態斷言；舊名 `grep`) / `monitor` |
| **Auto** | `✅` 完全自動 / `⚠` 需 env/fixture/裝置 / `❌` manual only / `⏰` cron / `🔒` 阻於 code-backlog |
| **Setup** | 跑前準備 (裝置 / emulator / 後端 spectyn serve / env) |
| **cmd** | 實際命令或動作 (runner 直接抄) |
| **expected** | 通過條件 |
| **Verifies** | 對應 surface flow §/能力/SPEC/BRK |
| **last_run** | 最後驗證日時 + runner；未跑填 `⬜` |
| **狀態** | `✅` 過 / `🟡` partial / `🔴` 重要缺 / `⬜` 未做 / `🟥` FAIL(code-backlog) / `⏸` v0.7.0 deferred |

---

## §1. 出貨真相聲明（read first）

| 項 | as-built 真相 | 證據（唯讀引用） |
|---|---|---|
| **出貨殼** | 行動端無條件 `return <AppTemplate/>`，**3 個 tab**：對話/機器/設定 | `app/src/App.tsx:148-149`、`AppTemplate.tsx:239-242`（`TABS` 三筆：對話/機器/設定）|
| **死碼殼** | `MobileShell`(7-tab) + `MobileDispatch`/`MobileDailyReview`/`MobileRecall`/`MobileOnboardingV2`/`MobileBrokerLogin` 全 commented out，永不渲染 | `App.tsx:150-151` 註解掉 `<MobileApp/>` |
| **onboarding** | 出貨路徑 = `OnboardingHello`（英文、強制登入、no-skip），gate 全平台 | `App.tsx:113` |
| **後端連線** | 無硬編 IP（`DEFAULT_BASE_URL=""`，2026-06-16 去洩漏）：使用者於 設定 tab 輸入 Base URL，存 `localStorage` 持久化 + **空 secret**；無 mDNS/coordinator picker 接線 | `AppTemplate.tsx:18`(`DEFAULT_BASE_URL`)、`:21`(`DEFAULT_SECRET=""`) |
| **聊天意圖** | `classifyIntent` → chat/dispatch/sense/**swarm**；swarm 公開渲染為一等意圖 | `AppTemplate.tsx:531`（swarm 分支）、`:456-515`（`runSwarm` body）|
| **硬編 IP 表** | `NODE_LABELS` 內嵌 5 台私有 Tailnet IP（資訊洩漏）| `AppTemplate.tsx:52-58` |

> 註：原碼註解自稱「4 iOS tabs」(`App.tsx` 內 comment) 為**過時陳述**；`TABS` 實際只有 3 筆（`AppTemplate.tsx:239-242`）。本檔以 3-tab 為準。

---

## §2. 出貨 3-tab 可過驗收（對「已出貨」寫，ANDAPP-* 主表）

### 2.A 對話 tab（ChatTab / you-talk landing）— 8 條

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| ANDAPP-CHAT-001 | e2e | ⚠ device+backend | emulator/裝置 + 一台可達 `spectyn serve` + 已填 baseUrl/secret | 啟動 app → onboarding 完成 → landing | 落地 tab = **對話**(full-bleed)，底部 3-tab bar(對話/機器/設定)，無 7-tab | surface §2 landing / §1 出貨殼 | ⬜ | ⬜ |
| ANDAPP-CHAT-002 | e2e | ⚠ backend | 後端 `/healthz` online | 對話框輸入「你好」→ 送出 | partner 泡泡回 LLM 文字，connection pill = online | surface §2 chat intent | ⬜ | ⬜ |
| ANDAPP-CHAT-003 | e2e | ⚠ backend offline | 後端不可達 | 送任一訊息 | 泡泡印「連線失敗 (status)。請到「設定」確認連線。」+ pill 翻 offline | surface §5 offline / `AppTemplate.tsx:311` | ⬜ | ⬜ |
| ANDAPP-CHAT-004 | unit | ✅ | - | `classifyIntent("幫我去 X")` 類分類測試 | 回 chat/dispatch/sense/swarm 其一（分類器存在且 deterministic） | surface §2 classifyIntent | ⬜ | ⬜ |
| ANDAPP-CHAT-005 | e2e | ⚠ backend | 已選 0 台機器 | 輸入派工意圖句 | 印「請先在「機器」tab 選一台機器,我才能幫你派工。」(不靜默 fan-out) | `AppTemplate.tsx:418` 守門 | ⬜ | ⬜ |
| ANDAPP-CHAT-006 | manual | ❌ | - | 觀察初始 partner 歡迎泡泡 | 含「我是你的 spectyn 夥伴」引導文（非空白）| `AppTemplate.tsx:286` | ⬜ | ⬜ |
| ANDAPP-CHAT-007 | e2e | ⚠ device | - | 旋轉/鍵盤彈出 | 版面不亂、輸入框 placeholder「說點什麼…」可見 | `AppTemplate.tsx:564` | ⬜ | ⬜ |
| ANDAPP-CHAT-008 | manual | ❌ | - | onboarding(英文) → 落地對話(繁中) | **記錄語言跳轉**：登入英文→落地繁中（非驗收 PASS，記為 UX-debt，見 §9 UXDEBT-001）| surface UX finding(medium) | ⬜ | 🟡 記 debt |

### 2.B 機器 tab（MachineTab / dispatch-target selector）— 6 條

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| ANDAPP-DISP-001 | e2e | ⚠ backend | ≥1 peer 線上 | 開「機器」tab | 列出線上機器 + 可勾選（多選）| surface §4① dispatch / `AppTemplate.tsx:624` | ⬜ | ⬜ |
| ANDAPP-DISP-002 | e2e | ⚠ backend | 勾 ≥1 機器 | 回對話送派工句 → `runDispatch` | 每台機器回**最終結果泡泡**（per-node result bubble；**無** live token stream、**無** cancel — 誠實現況）| surface §4④ / `runDispatch` | ⬜ | ⬜ |
| ANDAPP-DISP-003 | e2e | ⚠ backend | 派工到失敗節點 | (節點回錯) | 該節點回 failed 泡泡（`friendlyDispatchError` 映射）| surface §5 dispatch failed | ⬜ | ⬜ |
| ANDAPP-DISP-004 | manual | ❌ | - | 機器 tab 底部說明 | 印「勾選的機器會成為派工目標(可多選)」| `AppTemplate.tsx:659` | ⬜ | ⬜ |
| ANDAPP-DISP-005 | static | ✅ | repo | `grep -n 'live token stream\|cancel_dispatch' app/src/components/mobile/AppTemplate.tsx` | **0 命中**（確認出貨殼無 live supervise/cancel；驗收只驗 result-list 語意，不誤判可 cancel）| surface §2「Supervision honestly stated」/ T3 誠實 | ⬜ | ⬜ |
| ANDAPP-DISP-006 | e2e | ⚠ backend | 長跑 dispatch | 觀察派工中 | 無中途 cancel 控制（fire-and-wait）；此為現況限制非 bug — 真 supervise 屬 §3 SUP-* deferred | surface §4④ / D2/D3 | ⬜ | ⬜ |

### 2.C 設定 tab（SettingsTab / 連線 + 診斷）— 6 條

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| ANDAPP-CONN-001 | e2e | ⚠ device | - | 設定 tab → 填 Base URL + secret → 測試 | `/healthz` 探測 → pill online | surface §1b 出貨路徑連線 | ⬜ | ⬜ |
| ANDAPP-CONN-002 | e2e | ⚠ device | 空 secret + 空 Base URL | 全新裝置首啟（不改設定）| 無預設後端（`DEFAULT_BASE_URL=""`）+ 空 secret：使用者須於 設定 tab 輸入 Base URL；輸入後存 `localStorage` 跨重啟保留 | `AppTemplate.tsx:18,21` / surface §5「No backend chosen」（設計如此）| 2026-06-16 | ✅ 見 ANDAPP-LEAK-002（已修） |
| ANDAPP-CONN-003 | e2e | ⚠ backend | 已連線 | 觀察 connection pill 狀態機 | unknown→connecting→online/offline 三態正確 | surface §2 health probe | ⬜ | ⬜ |
| ANDAPP-CONN-004 | manual | ❌ | - | 設定 tab 診斷 log | 顯示 per-session 動作 log（非簽章、非 owner-scoped — 誠實標 §3 SUP）| surface §4④ flight-recorder PLANNED | ⬜ | 🟡 |
| ANDAPP-CONN-005 | e2e | ⚠ device | - | 設定改 Base URL → reload | 新 URL 生效（手輸路徑唯一可改後端方式）| surface §1b「hand-typing Base URL」| ⬜ | ⬜ |
| ANDAPP-CONN-006 | static | ✅ | repo | `grep -n 'pickCoordinator\|syncFromVault' app/src/components/mobile/AppTemplate.tsx` | **0 命中**（確認出貨殼不跑 vault sync / coordinator picker — 那些是死碼，見 §4）| surface §1b / §4 handoff | ⬜ | ⬜ |

---

## §3. 已知問題 = `[現況 FAIL / code-backlog]`（記成可重複條件，不修碼）

> 每條：步驟 / 期望 / 現況 + owner + 追蹤 issue + wave 落點。對齊 INV-15（驗收三件套含裁決依據）+ INV-9/11（④邊界、反目標守恆）+ K7（leak 掃描）。

### 3.A 資訊洩漏：硬編 5 台私有機 IP（LEAK，含 grep 斷言）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| **ANDAPP-LEAK-001** | static | ✅ | repo 根（唯讀）| `grep -nE '100\.(87\|115\|106\|107)\.[0-9]+\.[0-9]+' app/src/components/mobile/AppTemplate.tsx` | **0 命中**（標籤應動態來自 `/rpc/peers`，硬編 IP 應移除）| K7 leak / surface UX finding(low) | 2026-06-16 | ✅ **PASS**：`NODE_LABELS` 硬編 IP→label map 已整個移除；peer 標籤現在動態派生自 `/rpc/peers` 的 `name/host/id`（fallback 到 IP 字串），APK 不再內嵌任何私有 Tailnet IP。grep 0 命中。 |
| ANDAPP-LEAK-002 | static | ✅ | repo | `grep -n 'DEFAULT_BASE_URL = "http://100\.' app/src/components/mobile/AppTemplate.tsx` | 0 命中（預設後端不應硬編私有 IP）| K7 leak | 2026-06-16 | ✅ **PASS**：`DEFAULT_BASE_URL` 改為 `""`（不再硬編私有 IP）；使用者於 設定 tab 輸入後端 URL，存 `localStorage`（`spectyn.baseUrl`）跨重啟保留。grep 0 命中。 |

> **LEAK 裁決（已修，2026-06-16）**: **as-built 真相 = 硬編 IP 已移除（現況 PASS）**。`NODE_LABELS` IP→label map 整個刪除，標籤改派生自 `/rpc/peers` reported `name/host/id`（fallback IP 字串）；`DEFAULT_BASE_URL` 改 `""` + `localStorage` 持久化。ANDAPP-LEAK-001/002 grep 皆 0 命中。
> - **owner**: mobile（app/ 維護者，= 單一 operator）。
> - **觸發**: dev-host（公司機，operator 離職）下架時的全面私有 IP 去洩漏；連帶移除自家 Mac/z13/acer/ayaneo 的硬編 Tailnet IP。
> - **wave 落點**: Charter §F Wave 3 android slice（quick-win，mobile-references「⚡ ~1h 動態標籤去洩漏」）。**已完成轉綠。**

### 3.B 空 secret loopback（SEC）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| **ANDAPP-SEC-001** | static | ✅ | repo | `grep -n 'DEFAULT_SECRET = ""' app/src/components/mobile/AppTemplate.tsx` | secret 應自 vault 同步取得，非空字串預設 | INV-12 consent/可逆 / surface UX finding(high)「pull secret from vault」| ⬜ | 🟥 **FAIL**：`AppTemplate.tsx:21` `DEFAULT_SECRET = ""`；新用戶須手貼 64-char cluster secret 才能派工（recognition-not-recall 失敗）。空 secret = 派工 HMAC 無法成立 loopback。**[現況 FAIL / code-backlog]** |
| ANDAPP-SEC-002 | manual | ❌ | 全新裝置 | 首次派工（未填 secret）| 友善提示「到設定填 secret」而非靜默失敗 | surface §5 / INV-12 | ⬜ | 🟡 待驗 |

> **SEC 裁決（docs-only）**: as-built = 空 secret 預設、須手貼。修法屬 code（登入後自 `syncFromVault` 拉 secret，對齊死碼 `MobileBrokerLogin` 已有但未接線之邏輯）。**owner** = mobile；**追蹤 issue** `[需開 issue]` 建議 `[onboard] 出貨殼自 vault 拉 cluster secret，免手貼`；**wave 落點** Charter §F Wave 3「🏔 接線 mDNS 發現 + vault 握手」(mobile-references big item)。

### 3.C swarm 多代理意圖違反 BIG-GOAL §8（SWARM 反目標）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| **ANDAPP-SWARM-001** | static | ✅ | repo | `grep -n 'intent.intent === "swarm"' app/src/components/mobile/AppTemplate.tsx` + 檢查 fan-out confirm UI | chat intent 不得隱式路由到 swarm：swarm 自 classifyIntent 使用者面移除/隱藏，或任何多機 fan-out 前有顯式 confirm | INV-11 反目標守恆 / BIG-GOAL §8 | ⬜ | 🟥 **FAIL**：`AppTemplate.tsx:531` swarm 分支公開渲染為一等聊天意圖（`runSwarm` body `:456-515` 驅動 orchestrator :7900「四台機器一起檢查」= 多代理協作），且 fan-out 前無顯式 confirm。**[現況 FAIL / code-backlog]** |
| ANDAPP-SWARM-002 | manual | ❌ | backend | 對話送「機隊」類句 | 期望：被導向 dispatch 單節點 / 或隱藏；現況：觸發 `swarmStart`→:7900 live 多機 feed | INV-11 / surface §2 anti-goal | ⬜ | 🟥 FAIL（同上根因）|

> **SWARM 裁決（docs-only）**: 兩份手機 DB 採同一句可判定條件：**chat intent 不得隱式路由到 swarm：swarm 自 classifyIntent 使用者面移除/隱藏，或任何多機 fan-out 前有顯式 confirm**。修法屬 code。**owner** = mobile；**追蹤 issue** `[需開 issue]` 建議 `[INV-11] 移除/隱藏 AppTemplate swarm 一等意圖，或 fan-out 前加顯式 confirm`；**wave 落點** Charter §F Wave 3 android slice。

### 3.D onboarding demo-first apex 對帳（ONBOARD）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| **ANDAPP-ONBOARD-001** | manual | ❌ | 全新安裝 Android | 首次開 app | 首屏可在 0 帳號下達第一個回覆（demo-relay / embedded-core），且預設 zh-TW | BIG-GOAL §0/§6 demo-first+可逆 / 與 IOSAPP-ONB-001 同一元件裁決 | ⬜ | 🟥 **FAIL**：as-built `App.tsx:113` render `OnboardingHello`（英文、login-first、no-skip）。依工單採選項 A：demo-first apex 對齊，Android 與 iOS 同 verdict/wave；operator 可另行裁決改為 v0.7.0 deferred，但須雙檔同步。**[現況 FAIL / code-backlog]** |

---

## §4. ②記憶（owned-memory · apex #1）主線 — 全紅，標 owner+issue+wave

> 對齊 Charter K6（②/④兩條全紅有明確 wave 落點）+ BRK-1 + INV-2（向上追溯）。出貨殼 `AppTemplate`（3-tab）**零記憶/recall/timeline 表面** → 能力② 在 MVP 重心裝置上**完全不可見**。

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| **ANDAPP-MEM-001** | static | ✅ | repo | `grep -n 'memory\|recall\|timeline\|skill' app/src/components/mobile/AppTemplate.tsx`（**出貨殼**內）| 出貨殼應有 ≥1 記憶/recall 表面（apex #1）| BIG-GOAL §3② / surface §3② | ⬜ | 🟥 **FAIL**：出貨殼 0 記憶表面。能力②（apex 自稱 #1 未實作優先）在手機完全不可見。**[現況 FAIL / code-backlog]** |
| **ANDAPP-MEM-002** | integ | 🔒 | features `experimental-memory` | `cargo test --features experimental-memory -p spectyn-core skill_wire` | `embedding_search()` / `skill_store()` 回真實結果 | BRK-1 / FIDX-FAIL-D | ⬜ | 🟥 **FAIL**：`core/src/skill_wire.rs:882 embedding_search()` / `:1127 skill_store()` = `unimplemented!()`，全 7 面 panic；`0008_hermes_skills.sql` migration 不存在。封死手機記憶表面。**[現況 FAIL / code-backlog]** |
| ANDAPP-MEM-003 | e2e | ⏸ | (死碼) | `MobileRecall` `recall_search` over Life Node events | (空狀態「尚無事件…」) | surface §3② EXISTS-DEAD | ⬜ | ⏸ **v0.7.0 deferred**：`MobileRecall.tsx` 真碼但 off render path（`App.tsx:149` 先 return AppTemplate）。不對死碼寫 v0.6.0 驗收。 |
| ANDAPP-MEM-004 | e2e | ⏸ | (死碼) | `MobileMemory` panel | (記憶面板) | surface §3② EXISTS-DEAD | ⬜ | ⏸ **v0.7.0 deferred**：同上，死碼。 |

> **②記憶主線裁決**:
> - **owner**: core（`skill_wire.rs` 引擎，= 單一 operator）+ mobile（手機記憶表面 from-scratch add）。
> - **追蹤 issue**: BRK-1 = Charter R1（切版後第一優先）。建議 `[BRK-1/R1] skills schema 0008_hermes_skills.sql + skill_store()/embedding_search() 實作`（核心）+ `[②] AppTemplate 加 Memory/Recall tab + day-0 retroactive 非空狀態`（手機）。皆 `[需開 issue]`。
> - **wave 落點**: Charter §F **Wave 3 [②記憶] slice**（R1 skills schema + 全形 wire 文件落地，skill_wire panic 點標 BRK-1 追蹤）。mobile-references：🏔 big「補 skills schema migration + skill_store() 實作（輕量版）」。
> - **可見性裁決（docs-only）**: 出貨殼加記憶表面屬 from-scratch（非 re-enable 死碼）；引擎未實作前，表面應讀「warming up」非「broken」（surface UX finding fix）。`memory.db` 加密為 v0.7.0 scope（BIG-GOAL §P4 表，現仍明文）。

---

## §5. ④安全無人值守（supervision remote · OSS-gap 差異化）主線 — 全紅，標 owner+issue+wave

> 對齊 Charter K6 + INV-9（④邊界：有界/owner-governed/consent-gated async，非無界 fire-and-forget）+ D2/D3。**🚫 DEAD-CORNER**：phone supervise 阻於**缺後端 primitive**，非僅缺 UI。

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| **ANDAPP-SUP-001** | integ | 🔒 | backend `spectyn serve` | `curl -X POST <serve>/rpc/task/resume` | route 存在，可 resume/redirect 暫停的長跑 | D2 / surface §4④ | ⬜ | 🟥 **FAIL**：**無 `/rpc/task/resume` route、無 takeover driver**。手機 Approve/Redirect **無後端 primitive 可呼叫**。**[現況 FAIL / code-backlog]** |
| **ANDAPP-SUP-002** | integ | 🔒 | backend | (grep repo) `governor\|wall-clock\|hard-brake` 實碼 | 存在 wall-clock governor；邊界偵測 producer 可用且能升級到手機 | D3 / surface §4④ | ⬜ | 🟥 **FAIL**：邊界偵測 producer EXISTS（round cap / budget break loop），但 **wall-clock governor 與升級 transport 缺**；手機無可用升級閉環。**[現況 FAIL / code-backlog]** |
| **ANDAPP-SUP-003** | e2e | 🔒 | backend+FCM | tool-call 邊界 → push → 手機 Approve/Redirect/Stop sheet | push 喚醒 + in-app 核准/改向/中止 | surface §4④ phone escalation | ⬜ | 🟥 **FAIL**：無 FCM（manifest `:77` 自承「API31+ background start = FCM follow-up」）、無 `POST_NOTIFICATIONS`、無核准 UI。Stop 的最近 primitive = `cancel_dispatch`，但 unreachable + transport 不符；仍記全缺。**[現況 FAIL / code-backlog]** |
| ANDAPP-SUP-004 | integ | ⚠ backend | 已派工長跑 | 觸發可達停止控制 | 長跑停止，且覆蓋出貨 `/rpc/task/assign` 流 | surface §4④ Stop | ⬜ | 🟥 FAIL/code-backlog：`cancel_dispatch` primitive 存在（dispatch.rs:453）但出貨殼不可達（唯一 caller=死碼 `MobileDispatch`），且僅覆蓋 Tauri `dispatch_task` 流、非出貨 `/rpc/task/assign` 流 → 出貨手機 0 種停止方式。**[現況 FAIL / code-backlog]** |
| ANDAPP-SUP-005 | manual | 🔒 | backend | 簽章 flight-recorder（your-key-signed append-only owner-only）| 存在簽章審計軌 | surface §4④ flight-recorder PLANNED | ⬜ | 🟥 **FAIL**：僅 per-session 診斷 log（`AppTemplate` SettingsTab / `ActionLog.tsx`），非簽章/非 owner-scoped。**[現況 FAIL / code-backlog]** |

> **④無人值守主線裁決**:
> - **owner**: core（後端 primitive：governor + `/rpc/task/resume` + takeover driver + FCM 推送 + 簽章 flight-recorder，= 單一 operator）+ mobile（Supervision/Approvals surface）。
> - **追蹤 issue**: D2/D3 = Charter R2。建議 `[D3/R2] wall-clock governor + hard-brake primitive`、`[D2/R2] /rpc/task/resume route + takeover driver`、`[④] 手機 push→approve/redirect/stop 閉環 + 簽章 flight-recorder`。皆 `[需開 issue]`。
> - **wave 落點**: Charter §F **Wave 3 [④無人值守] slice**（R2 task lifecycle durable 單一真相 + boot reaper + `/rpc/task/resume` route 規格、手機 approve/redirect/stop primitive 設計）。mobile-references：🏔 big「Home Assistant actionable-notifications 當 ④ 核准流藍本」+「push→tool-call 邊界核准 plugin」。
> - **INV-9 守恆**: ④ 須記為**有界/owner-governed/consent-gated async**，**絕不**寫成無界 fire-and-forget/AutoGPT（否則違 INV-11 §8 反目標）。現況 fire-and-wait 且出貨手機 0 種停止方式 —— 記為 FAIL，非把無界當特性。

---

## §6. 7-tab / swarm 死碼相關測試 — 一律 `[v0.7.0 deferred]`

> Charter P-12 / BRK-11：**不對死碼寫 v0.6.0 驗收**。以下 case 描述死碼元件的**未來**驗收，標 ⏸ deferred、移出 ship gate；待選定單一殼（promote MobileShell 或 extend AppTemplate）後啟用。

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| ANDAPP-DEAD-001 | e2e | ⏸ | (死碼) | `MobileShell` 7-tab（對話/專注/集群/派送/歷史/教練/設定）渲染 | 7 tab 可達 | surface §0 EXISTS-DEAD | ⬜ | ⏸ **v0.7.0 deferred**：`App.tsx:149` 先 return AppTemplate；MobileShell 永不渲染。 |
| ANDAPP-DEAD-002 | e2e | ⏸ | (死碼) | `MobileDispatch` 派送 tab：`dispatch_task`→`dispatch::token::<id>` stream + Cancel | token stream + cancel | surface §4④ EXISTS-DEAD | ⬜ | ⏸ **v0.7.0 deferred**：死碼 **且** 走 incompatible `{broker}/api/squad/dispatch` transport（與出貨 `runDispatch` Tailscale+HMAC 不共用碼/auth）。不可借用補出貨 supervise。 |
| ANDAPP-DEAD-003 | e2e | ⏸ | (死碼) | `MobileDailyReview` 每日回顧（has-events/empty/locked 三態）| 空狀態 shame-free 文案 | surface §3③ EXISTS-DEAD | ⬜ | ⏸ **v0.7.0 deferred**：死碼；後端 coach review live（`serve.rs`）但手機無表面渲染。 |
| ANDAPP-DEAD-004 | e2e | ⏸ | (死碼) | `MobileOnboardingV2` 中文登入 + 「先不登入」skip(`:148`) | skip 逃生口可達 | surface §1a EXISTS-DEAD | ⬜ | ⏸ **v0.7.0 deferred**：出貨 = `OnboardingHello`(英文 no-skip)；V2 skip 永不可達。 |
| ANDAPP-DEAD-005 | e2e | ⏸ | (死碼) | `MobileBrokerLogin` coordinator picker(`:202`)→`pickCoordinator` thin-shell | mDNS 探索 + 選協調者 | surface §1b/§4 EXISTS-DEAD | ⬜ | ⏸ **v0.7.0 deferred**：出貨殼硬編後端，不呼叫 picker。 |
| ANDAPP-DEAD-006 | static | ✅ | repo | `grep -n 'return <MobileApp' app/src/App.tsx` | 應為**註解狀態**（確認死碼未啟用，守 P-12 baseline）| BRK-11 / P-12 | ⬜ | ⬜ 守門：確認 `App.tsx:151` 仍註解（出貨 = AppTemplate）|

---

## §7. 空狀態 / cold-start（出貨殼，可過驗收）— EMPTY

> apex ②(C) retro-activation：裝完當天就像已懂你。出貨殼 `AppTemplate` 無記憶表面 → 無記憶空狀態（= §4 MEM-001 FAIL 的延伸）。本節記出貨殼**確實會渲染**的空/錯狀態。

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| ANDAPP-EMPTY-001 | e2e | ⚠ device | 全新裝置、未連後端 | 啟動 → 對話 | 不可為死聊天；應導向設定連線（現況：fallback 硬編 Mac IP，記 §3 GAP）| surface §5「No backend chosen」GAP | ⬜ | 🟡 |
| ANDAPP-EMPTY-002 | e2e | ⚠ backend | 後端 online、0 事件 | 對話送一般訊息 | exit 正常、LLM 回覆（chat 不依賴記憶）| surface §2 | ⬜ | ⬜ |
| ANDAPP-EMPTY-003 | manual | ❌ | - | 觀察 day-0 記憶表面 | **無記憶表面可顯示** → 記為 §4 MEM-001 FAIL（apex「已懂你」時刻在手機缺席）| BIG-GOAL §3②C | ⬜ | 🟥 見 ANDAPP-MEM-001 |

---

## §8. Android 平台基建缺口（PLAT，記 FAIL，不修碼）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| ANDAPP-PLAT-001 | static | ✅ | manifest | `grep -n 'POST_NOTIFICATIONS\|com.google.firebase' app/src-tauri/gen/android/app/src/main/AndroidManifest.xml` | 應有 FCM + POST_NOTIFICATIONS（④ push 核准前提）| mobile-references 缺口表 / surface §3④ | ⬜ | 🟥 **FAIL**：manifest 無 FCM、無 `POST_NOTIFICATIONS`；自承「API31+ background start = FCM follow-up」(`:77`)。**[現況 FAIL / code-backlog]** |
| ANDAPP-PLAT-002 | static | ✅ | manifest | 檢 deep-link intent-filter(`:31-33`) | autoVerify App Links（HTTPS 驗證式，承載核准喚醒）| mobile-references (6) deep-link | ⬜ | 🟥 **FAIL**：intent-filter 為空 AUTO-GENERATED 佔位；custom scheme 不可驗證。**[現況 FAIL / code-backlog]** |
| ANDAPP-PLAT-003 | static | ✅ | manifest | `grep -n 'FOREGROUND_SERVICE_SPECIAL_USE\|BootReceiver' …/AndroidManifest.xml` | FGS specialUse + BootReceiver 存在（背景長跑復活）| mobile-references 缺口表 | ⬜ | ✅ **PASS**：manifest `:10` FGS specialUse + `:79` BootReceiver 已存在（相對成熟）。 |
| ANDAPP-PLAT-004 | static | ✅ | manifest | 檢 Keystore 宣告 | AndroidKeyStore StrongBox 宣告（加密為先硬體後盾）| mobile-references (5) 安全儲存 | ⬜ | 🟥 **FAIL**：manifest 無 Keystore 宣告（號稱加密為先卻無硬體後盾宣告）。**[現況 FAIL / code-backlog]** |

> **PLAT 主線**: owner = mobile；追蹤 issue 皆 `[需開 issue]`（`[④] Android FCM + POST_NOTIFICATIONS`、`[deep-link] App Links autoVerify + Digital Asset Links`、`[SEC] AndroidKeyStore StrongBox 宣告`）；wave 落點 = Charter §F Wave 3 ④ slice + mobile-references 🔧/🏔 items。PLAT-003 為唯一已 PASS（背景服務基建相對成熟）。

---

## §9. UX-debt（記錄，非 ship gate FAIL）— UXDEBT

| ID | 性質 | 證據 | 處置 |
|---|---|---|---|
| UXDEBT-001 | onboarding 語言跳轉：`OnboardingHello`(英文 no-skip) → `AppTemplate`(繁中) | `App.tsx:113` / `AppTemplate.tsx` 繁中 UI | 記 debt；操作者 memory 偏好繁中 → 建議 onboarding 繁中化（mobile-references 🏔 big「Onboarding 重構」）|
| UXDEBT-002 | 意圖誤判無 undo：`classifyIntent` 靜默路由 chat/dispatch/sense/swarm；dispatch 直接 fan-out、sense 讀 GPS | `AppTemplate.tsx:531` 等 | 記 debt；建議 side-effecting intent 加 1-tap confirm（surface UX finding medium）|
| UXDEBT-003 | 後端發現須手輸 IP+secret（recognition-not-recall 失敗）| `AppTemplate.tsx:18,21` | 與 §3 LEAK/SEC 同根；mDNS 發現 + vault 拉 secret（Wave 3 big）|
| UXDEBT-004 | 雲端沙盒 onboarding 無 in-app 入口（`scripts/setup-cloud-linux.sh` 手跑）| surface §1b Path B GAP | 記 debt；apex 要求 cloud sandbox 為一等 no-desktop fallback（Wave 3+）|

---

## §10. 統計

```
Total ANDAPP-* cases: 49 條（機械重數 §2-§8 行首 | ANDAPP-）
  §2 出貨 3-tab 可過驗收:  20  (CHAT 8 + DISP 6 + CONN 6)
  §3 [現況 FAIL] LEAK/SEC/SWARM/ONBOARD: 7  (LEAK 2 + SEC 2 + SWARM 2 + ONBOARD 1)
  §4 ②記憶主線:             4
  §5 ④無人值守主線:         5
  §6 死碼群:                6  (DEAD-001..005 deferred + DEAD-006 ⬜守門)
  §7 空狀態:                3
  §8 平台基建:              4 (含 1 PASS: PLAT-003)
  小計 = 20+7+4+5+6+3+4 = 49 ✓
  (+ UXDEBT 4 條為記錄表，不計入 case Total)

By 狀態（機械重數，加總 = 49）:
  ✅ PASS / 守門:           1  (PLAT-003)
  🟡 partial / debt:        4
  🟥 [現況 FAIL/code-backlog]: 18  (機械重數；§3/§4/§5/§8 彙整)
  ⏸ v0.7.0 deferred:        7  (DEAD-001..005 五條 + MEM-003/004 兩條；DEAD-006 為 ⬜ 守門非 deferred)
  ⬜ 待驗(出貨殼可過):      19  (待裝置/後端 runner 跑出證據 + 守門)

出貨真相 re-baseline: 全表以 3-tab AppTemplate 為準；
  7-tab/swarm 死碼 case 全標 ⏸ deferred（§6），不進 v0.6.0 ship gate。
②/④兩條全紅主線（§4/§5）皆標 owner + 追蹤 issue([需開 issue]) + Wave 3 落點。
ANDAPP-LEAK-001 grep 斷言就位（K7 leak 守門）。
```

---

## §11. CLI / grep runner 範例（可 ✅ Auto 的靜態斷言全跑）

```bash
#!/bin/bash
# scripts/test-runner-android-app-static.sh
# 跑所有 grep 型靜態斷言（不需裝置）。期望：洩漏/反目標 case 現況 FAIL（紅），
# 守門 case（DEAD-006 / PLAT-003）綠。修碼後 LEAK/SWARM 應轉綠。
set +e
AT=app/src/components/mobile/AppTemplate.tsx
MAN=app/src-tauri/gen/android/app/src/main/AndroidManifest.xml

echo "== ANDAPP-LEAK-001 (期望 0 命中；已修 2026-06-16 → PASS) =="
grep -nE '100\.(87|115|106|107)\.[0-9]+\.[0-9]+' "$AT"

echo "== ANDAPP-SWARM-001 (期望修後 0 命中；現況 FAIL) =="
grep -n 'intent.intent === "swarm"' "$AT"

echo "== ANDAPP-SEC-001 (期望修後 0 命中；現況 FAIL) =="
grep -n 'DEFAULT_SECRET = ""' "$AT"

echo "== ANDAPP-DEAD-006 (守門：應命中註解的 MobileApp) =="
grep -n 'return <MobileApp' app/src/App.tsx

echo "== ANDAPP-PLAT-003 (應 PASS：FGS specialUse + BootReceiver) =="
grep -n 'FOREGROUND_SERVICE_SPECIAL_USE\|BootReceiver' "$MAN"
```

---

## §12. 對齊 Charter 檢核（本檔自證）

- **INV-15（surface 三件套）**: §1 流程真相 + §2 驗收條件 + §2-§8 測試 case DB（對等 mac.md 逐條格式）。✅
- **INV-16（feature↔test）**: 死碼 feature（F100-107 orphan-archived）不繼承假綠；出貨殼 case 綁可重複 grep/e2e。✅
- **K2（surface case DB）**: 本檔 = android-app 對等 mac.md 級逐條 case DB（CLI runner 可直讀）。✅
- **K7（leak 掃描）**: ANDAPP-LEAK-001/002 grep 斷言就位。✅
- **K8（出貨真相對帳）**: 全表以 3-tab AppTemplate re-baseline；死碼 case 標 ⏸ deferred 移出 ship gate（P-12）。✅
- **K6（②④全紅有 wave 落點）**: §4/§5 皆標 owner + issue + Wave 3 落點。✅
- **INV-9/11（④邊界 + 反目標）**: §5 INV-9 守恆註；§3 SWARM 記 §8 反目標 FAIL。✅
- **INV-1/3/6**: 本檔從屬 apex，未改寫 BIG-GOAL。✅
