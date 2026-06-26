# Windows CLI 平台 — 全測試用例庫 v1 (2026-06-12)

> **覆蓋範圍**: SPEC-46 的 13 條跨命令不變量（I1–I13）+ ~48 個 `phantom` 命令/子命令的關鍵 CUJ（g1–g9 分組）。
> 對齊 `docs/superpowers/specs/v060-deep-spec/SPEC-46-windows-cli-behavior/`（spine + g1–g9）的逐欄 as-built 現況。
>
> **三件套定位（INV-15）**: 本檔是 SPEC-46（流程文件 + 驗收條件）的**測試 case 件**。win CLI surface 先前無對等 mac.md 級 case DB（Charter §D D.1「win CLI 測試 case DB 🟥 無對等 case DB」、BRK-10、K2）；本檔補上該洞。
>
> **schema**: 對等 `docs/test-cases/mac.md` v2 格式（加 `Auto`/`Setup`/`cmd`/`expected`/`last_run` 欄，CLI runner 直接讀檔跑、不用人翻譯）。每條 case 有唯一 ID、步驟（cmd）、期望（expected）、現況（狀態 PASS/FAIL）。
>
> **編號規約**: `WINCLI-INV-{NN}-{nnn}`（跨命令不變量）或 `WINCLI-{GROUP}-{cmd}-{nnn}`（逐命令 CUJ，GROUP=G1..G9）。ID 永不重用。
>
> **as-built 真相來源**: `.ai-shared/done/windows-terminal-feature-matrix-2026-05-29.md`（59 場景實測矩陣）+ SPEC-46 g1–g9 #13 欄。狀態反映 **2026-05-29 win11 phantom 0.6.0-rc.1** 實測。
>
> **平台環境鐵則（影響多條 case）**: 本機 PATH 上有 stale 0.4.0 `phantom.exe`（`C:\Users\<user>\bin`）遮蔽 0.6.0-rc.1（`.local\bin`）。多條 case 須先 `$env:PHANTOM_BIN` 或絕對路徑釘住受測 binary，否則測到舊版（見 WINCLI-INV-10-002 / WINCLI-G7-selftest-002）。
>
> **docs-only 聲明**: 本檔為文件，**未改任何 `core/src` 程式**。其中標 `[現況 FAIL / code-backlog]` 的 case 是把已知 code 缺陷記成「可重複測試的驗收條件」，待修碼 wave 處理（§11 集中列出三大 FAIL）。

---

## §0. Schema legend

共用 token/schema 字典見 [`README.md`](README.md)；本檔只列 Windows CLI 端補充。

每條 case 欄位（同 mac.md v2）：

| 欄 | 意義 |
|---|---|
| **ID** | 唯一識別 (永不重用) |
| **Type** | `unit` (cargo test 內部) / `integ` (cargo test --test 或 CLI+serve hop) / `e2e` (真終端/真 serve/Appium drive) / `manual` (人眼/人手) / `monitor` (synthetic prod canary cron) |
| **Auto** | `✅` 完全自動跑 / `⚠` 需 env 或 fixture / `❌` manual only / `⏰` cron 排程 |
| **Setup** | 跑前準備 (PHANTOM_HOME=TempDir / env var / 起 serve / PHANTOM_BIN 釘版) |
| **cmd** | 實際命令 (CLI runner 直接抄；PowerShell 可執行) |
| **expected** | 通過條件 (exit code / stdout|stderr match / file exist|absent) |
| **Verifies** | 對應 SPEC-46 不變量 I[X] + 命令 + g* 檔 |
| **last_run** | 最後驗證日時 + runner |
| **狀態** | `✅` 過 / `🟡` partial / `🔴` 重要缺/broken / `⬜` 未做 / `❔` UNVERIFIED |

> **Windows 路徑慣例**：`<BIN>` = 受測的 0.6.0-rc.1 phantom.exe 絕對路徑（如 `$env:USERPROFILE\.local\bin\phantom.exe`）。所有 PHANTOM_HOME 隔離測試用 `$env:PHANTOM_HOME = (New-Item -ItemType Directory -Path "$env:TEMP\pm-$(New-Guid)").FullName`，**絕不**污染真實 `~/.phantom-mesh`。

---

## §1. 跨命令不變量 I1–I13（SPEC-46 核心 · 53 條）

> 每條不變量對映一個已在實機發生過的 bug。`scripts/qa/win-regression.mjs`（規格指定的回歸守護）依這些編號加斷言。本節是 win CLI 三件套的「驗收條件」主幹。

### 1.I1 — 說明即唯讀（help/version is inert）(7 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-INV-01-001 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir；先放一個已知 `PHANTOM.md`（記下 mtime/hash） | `<BIN> init --help` | exit 0、**`PHANTOM.md` 未被覆寫**（mtime/hash 不變）、不連網 | I1 / `init` / g3 | ⬜ | 🔴 `init --help` 忽略旗標直接 scaffold、曾 clobber 追蹤檔（g3 #13 broken） |
| WINCLI-INV-01-002 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir；先 `login` 寫一個 `auth.json` | `<BIN> logout --help` | exit 0、**`auth.json` 仍存在**（未刪） | I1 / `logout` / g3 | 2026-05-30 incident | 🔴 真實事故：`logout --help` 真的登出刪 `auth.json`（`.ai-shared/done/incident-logout-help-2026-05-30.md`） |
| WINCLI-INV-01-003 | e2e | ⚠ non-TTY | PHANTOM_HOME=TempDir；redirected stdin（非 TTY） | `$null | <BIN> login --help` | exit 0、不綁 `:48181`、不開瀏覽器、不 hang（≤2s 回） | I1+I2 / `login` / g3 | ⬜ | 🔴 `login --help` 啟動 OAuth、綁埠、hang（g3 #13 broken） |
| WINCLI-INV-01-004 | e2e | ⚠ non-TTY | PHANTOM_HOME=TempDir | `$null | <BIN> onboarding --help` | exit 0、不開互動網頁流程、不 hang | I1+I2 / `onboarding` / g3 | ⬜ | 🔴 `onboarding --help` 開互動流程（g3 #13 broken） |
| WINCLI-INV-01-005 | e2e | ✅ | — | `<BIN> config --help` | exit 0、stdout/stderr 印完整用法、零副作用 | I1 / `config` / g3（正例） | 2026-05-29 win11 | ✅ `config --help` 正確印用法（g3 #13 works，本組少數 I1 正例） |
| WINCLI-INV-01-006 | e2e | ✅ | — | `<BIN> recall --help`；`<BIN> review --help`；`<BIN> food --help`；`<BIN> habit --help`；`<BIN> note --help` | 每個 exit 0、印用法、零副作用 | I1 / Life Track / g9（正例群） | 2026-05-29 源碼確認 | ✅ Life Track handler 都正確處理 `--help`（g9 正面發現；I1 違反集中在 g3） |
| WINCLI-INV-01-007 | e2e | ⚠ | — | `<BIN> keys --help`；`<BIN> whoami --help` | 應印用法 exit 0；**現況** keys 走 `unknown subcommand` 分支、whoami 忽略旗標直接印身分 | I1（輕微）/ `keys`/`whoami` / g3 | 2026-05-29 win11 | 🟡 輕微不一致（非破壞性，但違 I1 一致性） |

### 1.I2 — 非終端機不阻塞（non-TTY never blocks）(5 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-INV-02-001 | e2e | ⚠ non-TTY | PHANTOM_HOME=TempDir；redirected stdin | `$null | <BIN> login` | exit 非 0（快速失敗）、**不綁 `:48181`**、不開瀏覽器、≤5s 回、無孤兒 phantom.exe | I2 / `login` / g3 | ⬜ | 🔴 非 TTY 啟動 OAuth、綁埠、hang、孤兒化（g3 #13 broken） |
| WINCLI-INV-02-002 | e2e | ✅ | redirected/piped stdin | `"hi" | <BIN> tui` | 印 non-TTY guard 訊息（`stdin is not a terminal …`）、exit 非 0（bail 非 panic 101） | I2+I4 / `tui` / g1 | 2026-05-29 win11 | ✅ guard 正確觸發、不 hang、不 panic（g1 #13 works） |
| WINCLI-INV-02-003 | e2e | ✅ | TTY stdin，無 PROMPT | `<BIN> exec`（互動終端、無 prompt、無 pipe） | exit 2、印 `requires a PROMPT or stdin input`、不 hang | I2+I4 / `exec` / g1 | 2026-05-29 win11 | ✅ 偵測 TTY 無輸入→快速失敗（g1 verified） |
| WINCLI-INV-02-004 | e2e | ✅ | piped stdin | `echo "pipeline note" | <BIN> note` | exit 0、從 stdin 讀文字寫筆記（非 hang、非印用法） | I2 / `note` / g9（正例） | 2026-05-29 源碼確認 | ✅ `note` 用 `std::io::IsTerminal` 正確區分互動 vs 管線（g9 正例） |
| WINCLI-INV-02-005 | e2e | ⚠ non-TTY | redirected stdin | `$null | <BIN> onboarding` | 偵測非 TTY→快速失敗、不開瀏覽器卡住 | I2 / `onboarding` / g3 | ⬜ | 🔴 非 TTY 開瀏覽器無人操作→卡住（g3 #13 broken） |

### 1.I3 — 本機平面不需叢集授權（local plane needs no cluster auth）(4 條) — 含 §11 FAIL-A

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-INV-03-001 | e2e | ⚠ serve | 本機起 `phantom serve`（127.0.0.1:7878）；已 login | `curl.exe -s -o NUL -w "%{http_code}" -X POST http://127.0.0.1:7878/api/chat -d '{...}'` | HTTP **200**（本機 UI 呼叫不該被 `X-Cluster-Auth` 擋） | I3 / `/api/chat`（web，非 CLI）/ spine §2 | 2026-05-29 win11 | 🔴 **[現況 FAIL / code-backlog]** 回 **401**（X-Cluster-Auth 擋本機 UI）。§11 FAIL-A。owner core-mesh / T-AUTH-LOCAL |
| WINCLI-INV-03-002 | e2e | ⚠ serve | 本機起 serve；已 login（有 broker token） | `<BIN> sessions` | exit 0、印 `◆ N active session(s):` 或 `no active sessions …`（本機/使用者命令不被遠端授權擋） | I3 / `sessions` / g8 | 2026-05-29 win11 | 🔴 **[現況 FAIL / code-backlog]** 回 `Error: HTTP 401: {"error":"unauthenticated"}`（對 broker 的 Bearer token 未授權；根因與 /api/chat 不同但同違 I3）。owner core-mesh / T-AUTH-LOCAL |
| WINCLI-INV-03-003 | e2e | ⚠ serve | 本機起 serve | `curl.exe -s -o NUL -w "%{http_code}" http://127.0.0.1:7878/api/sessions` | HTTP **200**（本機讀無需 auth — 對照組，證明 local-plane 設計正確） | I3 / serve local-plane / g2 | 2026-05-29 win11 | ✅ 本機 `GET /api/sessions` 回 200（對照證明 sessions CLI 的 401 是 bug 非設計） |
| WINCLI-INV-03-004 | e2e | ⚠ serve+key | 本機起 serve | `<BIN> event capture --kind text --text "local plane test"`（打 127.0.0.1） | POST 到本機 serve **不**被 cluster auth 擋（exit 0 或 provider 端錯誤，非 auth 401） | I3 / `event capture` / g6 | ⬜ | ❔ 未單測 auth 邊界；client 打 127.0.0.1 應免 cluster auth |

### 1.I4 — 退出碼契約（exit-code contract）(4 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-INV-04-001 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir | `<BIN> exec ""`（空 prompt） | exit **2**（用法/參數錯） | I4 / `exec` / g1 | 2026-05-29 win11 | ✅ 空 prompt→exit 2（g1 verified `phantom.rs:5532/5544`） |
| WINCLI-INV-04-002 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir | `<BIN> logs --since "garbage"` | exit **1**（預期內失敗 + `bad --since value …`） | I4 / `logs` / g6 | ⬜ | ❔ 源碼有此分支；Windows 未實測 |
| WINCLI-INV-04-003 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir | `<BIN> lang set`（缺參數） | exit **2**（用法錯） | I4 / `lang` / g3 | ⬜ | ❔ `lang` Windows 未實機驗（g3 #13 UNVERIFIED） |
| WINCLI-INV-04-004 | e2e | ⚠ all | 跑所有命令的失敗路徑 | （掃描 exit code） | **永不 panic→101**；0=成功/1=預期失敗/2=用法錯 | I4 / 全 CLI / spine §2 | ⬜ | 🟡 selftest 曾於舊 binary panic 101（os error 232）— 釘版後應消失 |

### 1.I5 — 降級不崩潰（degrade-not-fail）(6 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-INV-05-001 | e2e | ⚠ serve+badkey | 起 serve、設無效 Gemini key | `<BIN> event capture --kind food --text "salad"` | **理想**：event 仍持久化、analyze 失敗 async 降級（不 exit 1）。**現況**：serve 寫入路徑同步呼 Gemini、失敗回 502→CLI exit 1 | I5 / `event capture` / g6 | 2026-05-29 win11 | 🟡 違反 I5（寫入路徑耦合同步 analyze；應 async 佇列）。owner core life-track |
| WINCLI-INV-05-002 | e2e | ⚠ freekey | 設 Groq free-tier key | `<BIN> daily-review`（當日多事件） | **理想**：chunk/map-reduce 降級成功。**現況**：單次 LLM 吃爆 6000 TPM→413 | I5 / `daily-review` / g6 | 2026-05-29 win11 | 🟡 違反 I5（單次呼叫爆 TPM；需分塊）。owner core |
| WINCLI-INV-05-003 | e2e | ⚠ no-feature | Windows release binary（無 skill bank curator feature） | `<BIN> skill run .\x.md` | exit 1 + **可操作降級訊息** `✗ … built without 'experimental-curator'. Rebuild with: cargo build --features …`（非晦澀錯誤） | I5 / `skill run` / g5 | 2026-05-29 win11 | 🟡 partial（feature-gated out；已於 `release-windows.yml` 加 feature 修）。降級訊息符合 I5 |
| WINCLI-INV-05-004 | e2e | ⚠ serve | 不起 serve | `<BIN> doctor` | exit **0**、healthz 行印 `⚠ connection refused`（warning 不抬升 exit code） | I5 / `doctor` / g7 | 2026-05-29 win11 | ✅ doctor 各 network row 降級為 ⚠、exit 0（g7 works） |
| WINCLI-INV-05-005 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir（無 events） | `<BIN> logs` | exit 0、stderr `(no events log yet at <path> …)`（空非錯誤） | I5 / `logs` / g6 | ⬜ | ✅（推定）源碼降級為空、exit 0（g6 works） |
| WINCLI-INV-05-006 | e2e | ⚠ serve | 起 serve、`--remote-telegram` 但未編 feature | `<BIN> serve --remote-telegram` | stderr `remote : … requires --features experimental-remote-control-telegram ; flag ignored`、HTTP server **仍續跑**（exit 不因此提升） | I5 / `serve` / g2 | 2026-05-29 win11 | ✅ degrade-not-fail（g2 verified `phantom.rs:3813-3817`） |

### 1.I6 — 資料根解析（data-root resolution / PHANTOM_HOME）(5 條) — 含 §11 FAIL-B

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-INV-06-001 | integ | ⚠ env | 設 `$env:PHANTOM_HOME` = 一個空 TempDir D | `<BIN> habit checkin morning-run "5km"`（或任何寫事件命令） | event 寫入 **D（PHANTOM_HOME）** 下、**不**寫真實 `~/.phantom-mesh` | I6 / 資料根 / spine §2 | ⬜ | 🔴 **[現況 FAIL / code-backlog]** §11 FAIL-B：多處走 `dirs::home_dir()` 硬編、未吃 PHANTOM_HOME 分支（g6/g8 標 UNVERIFIED/缺口） |
| WINCLI-INV-06-002 | integ | ⚠ env | 設 `$env:PHANTOM_HOME` = TempDir D | `<BIN> keys path` | 印出的路徑在 **D** 下（非 `C:\Users\<user>\.phantom-mesh`） | I6 / `keys` / g3 | ⬜ | 🔴 **[現況 FAIL / code-backlog]** §11 FAIL-B：`auth_path()`/`agents_toml_path()` 子代理回報只有 home-rooted、無 PHANTOM_HOME 分支 |
| WINCLI-INV-06-003 | integ | ⚠ env | 設 `$env:PHANTOM_HOME` = TempDir D | `<BIN> providers list` | 讀 **D\agents.toml**（非 home-rooted） | I6 / `providers` / g8 | ⬜ | 🔴 **[現況 FAIL / code-backlog]** §11 FAIL-B：`agents_toml_path()` = `dirs::home_dir()+.phantom-mesh/agents.toml`，無 PHANTOM_HOME 分支（g8 cross-cutting note 明載） |
| WINCLI-INV-06-004 | integ | ⚠ env | **不**設 `$env:HOME`（Windows）、設 `$env:USERPROFILE` | `<BIN> whoami` | 解析資料根用 USERPROFILE-based home（**證明 Windows 上 `$HOME` 被 dirs 忽略**） | I6 / Windows home quirk / spine §2 | 2026-05-29 win11 | 🟡 已知：Windows `dirs::home_dir()` 不理 `$HOME`（$HOME-redirect 測試會污染真實 ~） |
| WINCLI-INV-06-005 | integ | ⚠ env | 設 PHANTOM_HOME=TempDir | 任何命令寫檔 | 路徑用 OS 分隔符（`\`）、不假設 `~`/POSIX | I6 / 路徑分隔符 / spine §2 | ⬜ | ❔ 未系統性驗；doctor/debug 輸出已見 `\` 分隔符 |

### 1.I7 — ASCII/CP950 安全輸出 (4 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-INV-07-001 | manual | ❌ CP950 | PowerShell 5.1 / CP950 主控台（chcp 950） | `<BIN> providers list` | `✓`/`⚠` glyph 正確渲染或降級 ASCII、**不 mojibake** | I7 / `providers` / g8 | ⬜ | ❔ CP950 渲染 UNVERIFIED（g8 標 mojibake risk） |
| WINCLI-INV-07-002 | e2e | ✅ | — | `<BIN> recall "x" --json`；`<BIN> evolve list --json` | `--json` 輸出**純 ASCII-safe**、單一 JSON 文件、可被 `ConvertFrom-Json` 解析 | I7+I10 / `recall`/`evolve` / g9/g5 | ⬜ | ❔ `--json` Windows 未實測（對照 selftest --json 0-bytes 風險） |
| WINCLI-INV-07-003 | integ | ⚠ tmpHOME | PHANTOM_HOME=TempDir | `<BIN> keys init`（寫 env 檔） | 寫檔為 **BOM-less UTF-8**（避免 CP950 工具誤讀） | I7 / `keys` / g3 | ⬜ | ❔ 未驗 BOM-less |
| WINCLI-INV-07-004 | manual | ❌ CP950 | CP950 主控台 | `<BIN> cluster`（status，含 ✓/✗） | `✓`/`✗` 行不 mojibake 或降級 ASCII | I7 / `cluster` / g4 | ⬜ | ❔ UNVERIFIED 是否降級 ASCII（g4 partial-risk） |

### 1.I8 — 建置不依賴主機工具（build date）(2 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-INV-08-001 | e2e | ✅ | — | `<BIN> --version` | full line 含**真實建置日期**（非字面 `built unknown`） | I8 / `--version` / g7 | 2026-05-29 win11 | 🔴 顯示 `built unknown`（`core/build.rs` 用 `date -u`，Windows 無 GNU date）。owner core/build.rs |
| WINCLI-INV-08-002 | e2e | ✅ | — | `<BIN> --version --short` | 印純 semver `0.6.0`（容忍 `0.6.0[-rc.N]` 後綴）、exit 0 | I8 / `--version --short` / g7 | 2026-05-29 win11 | ✅ short 形式正確（git hash + os-arch 也對；唯日期欄壞） |

### 1.I9 — 破壞性操作雙重保護（destructive double-guarded）(5 條) — 含 §11 FAIL-C

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-INV-09-001 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir + 12 筆 events | `<BIN> data delete --all`（**無** `--yes`） | exit 1、印 `would delete N events … — rerun with --yes to confirm`、**磁碟未動**（events 仍在） | I9 / `data delete` / g6（正例） | 2026-05-29 win11 | ✅ I9 雙重保護正例：先印計畫、不碰磁碟（`dry_run_without_confirmed_does_not_delete` 覆蓋） |
| WINCLI-INV-09-002 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir + events | `<BIN> data delete --all --yes` | exit 0、`✓ deleted N events …`、events/ 清空、**identity.key/agents.toml 不動** | I9 / `data delete` / g6 | 2026-05-29 win11 | ✅ 帶 `--all --yes` 才真刪（g6 works） |
| WINCLI-INV-09-003 | e2e | ⚠ serve | 手動起一個 `phantom serve`（非 task 啟動）、`service install` 過 | `<BIN> service uninstall` | **理想**：只刪 task、不殺手動 serve（或需 `--yes`、先印計畫）。**現況**：無條件 `taskkill /F /IM phantom.exe` 殺掉 live serve | I9 / `service uninstall` / g2 | 2026-05-29 win11 | 🔴 **[現況 FAIL / code-backlog]** §11 FAIL-C：`service/windows.rs:~233-235` 無條件 taskkill（無 guard/確認）。owner core (Windows service) |
| WINCLI-INV-09-004 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir + 既有 `agents.toml` env keys | `<BIN> keys init --force` | 須明確 `--force` 才覆寫（無 `--force` 時不覆寫既有） | I9 / `keys init` / g3 | ⬜ | ❔ 未實測；源碼方向符合 I9（`--force` gate） |
| WINCLI-INV-09-005 | e2e | ⚠ skill | skill bank feature build | `<BIN> skill run .\x.md --sandboxed`（無 `--allow`） | exit 2、`✗ --sandboxed requires at least one --allow <cmd>`（快速失敗、非無限制執行） | I9 / `skill run` / g5 | ⬜ | ❔ feature-gated out（Windows release）；邏輯符合 I9 |

### 1.I10 — `--json` 機器穩定（machine-stable JSON）(3 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-INV-10-001 | e2e | ⚠ key | 有效 provider key | `"summarize this" | <BIN> exec --json` | stdout 每行一個 `AgentEvent` JSON、末行 `{"type":"done",…}`、人類訊息不混入 stdout | I10 / `exec --json` / g1 | 2026-05-29 win11 | ✅ exec round-trip GREEN（groq 413→gemini fallback→answer，唯一端到端驗證命令） |
| WINCLI-INV-10-002 | e2e | ⚠ PHANTOM_BIN | **釘** `$env:PHANTOM_BIN` = 0.6.0-rc.1 絕對路徑（避開 stale 0.4.0） | `<BIN> selftest --json` | stdout 吐**非空**單一 JSON 文件（含 `summary`/`features`） | I10 / `selftest --json` / g7 | 2026-05-29 win11 | 🔴 Windows 吐 **0 bytes**（stdout+stderr 皆空）；根因部分為 PATH 解析到 stale 0.4.0 binary。owner core/scripts |
| WINCLI-INV-10-003 | e2e | ✅ | — | `<BIN> node-capabilities --json` | stdout 單一 pretty JSON（含 `schema`/platform/capabilities 陣列）、非空 | I10 / `node-capabilities --json` / g4 | ⬜ | ❔ `--json` 路徑源碼確認、未實跑（對照 selftest 0-byte 風險，須驗非空） |

### 1.I11 — stdout/stderr 分流 (3 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-INV-11-001 | e2e | ⚠ key | provider key | `<BIN> exec "what is 2+2?" 2>$null` | 答案在 **stdout**（2>$null 丟掉診斷後仍見答案）、`[tool]`/`[done]` 在 stderr | I11 / `exec` / g1 | 2026-05-29 win11 | ✅ exec 結果走 stdout、tool/notice 走 stderr（g1 verified） |
| WINCLI-INV-11-002 | e2e | ✅ | — | `<BIN> exec --help 1>$null` | help payload 走 **stderr**（現況 eprintln）—**驗收記錄**：理想應走 stdout（I11） | I11 / `exec --help` / g1 | 2026-05-29 win11 | 🟡 `exec --help` 印到 stderr（I1 滿足 exit 0，但 I11 期望 help payload 走 stdout）。owner core-CLI |
| WINCLI-INV-11-003 | e2e | ✅ | — | `<BIN> providers list 1>$null`；`<BIN> workspace 1>$null` | **驗收記錄**：現況主結果走 **stderr**（eprintln），嚴格 I11 應走 stdout | I11 / `providers`/`workspace` / g8 | 2026-05-29 win11 | 🟡 providers/models/workspace 主結果走 stderr、sessions 走 stdout（內部不一致，記為 owner backlog） |

### 1.I12 — 設定單一來源（single config source）(2 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-INV-12-001 | integ | ⚠ env | 在 cwd 放 `./agents.toml`（A）+ home 放另一個（B）、值不同 | `<BIN> providers list` | **理想**：依兩段查找 `./agents.toml`→`~/.phantom-mesh/agents.toml` 讀到 A。**現況**：只讀 home-rooted（B） | I12 / `providers` / g8 | ⬜ | 🔴 **[現況 FAIL / aspirational]** 子代理回報程式碼**沒有** `./agents.toml`→home 兩段查找，只有單一 home-rooted（g8 cross-cutting note）。I12 為實作缺口 |
| WINCLI-INV-12-002 | integ | ⚠ env | 同上 | `<BIN> exec --config .\agents.toml "hi"` | `--config PATH` 明示時讀指定檔（exec 有此旗標）— 對照證明只有 exec 接受顯式 config | I12 / `exec --config` / g1 | ⬜ | ❔ exec 有 `--config`；其餘命令無—I12 不一致 |

### 1.I13 — 冪等性（idempotency）(3 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-INV-13-001 | e2e | ⚠ admin | 已 `service install` 過 | 再跑 `<BIN> service install` | exit 0、先 `schtasks /Delete` 舊 task 再 `/Create`、收斂到新 binary 路徑、**不報錯** | I13 / `service install` / g2 | 2026-05-29 win11 | ✅ idempotent（g2 verified `windows.rs:111-115`） |
| WINCLI-INV-13-002 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir、已 `keys init` 過 | 再跑 `<BIN> keys init` | 已存在則不報錯（冪等收斂） | I13 / `keys init` / g3 | ⬜ | ❔ 未實測；I13 期望 |
| WINCLI-INV-13-003 | e2e | ⚠ | 已 `autoevolve schedule install` 過 | 再跑 `<BIN> autoevolve schedule install --interval 3600` | exit 0、`schtasks /Delete`→`/Create` 收斂、無錯 | I13 / `autoevolve schedule` / g5 | 2026-05-29 win11 | ✅ install 先 Delete 再 Create（g5 idempotent，`schedule status` 實測 registered:no 正確） |

---

## §2. g1 互動式入口 — `(bare)` · `repl` · `tui` · `exec` (6 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-G1-bare-001 | manual | ❌ TTY | 真 Windows Terminal、有 agents.toml | `<BIN>`（無子命令） | 全螢幕 TUI 填滿終端 | (bare) / g1 #13 | 2026-05-29 | ✅ 互動 TUI render GREEN（真 console） |
| WINCLI-G1-bare-002 | e2e | ✅ | `$env:PHANTOM_REPL="1"` | `<BIN>` | 印 REPL banner match `phantom v?0\.6\.0(-rc\.[0-9]+)? … Ctrl-D to exit` + prompt | (bare) env mode / g1 | 2026-05-29 win11 | ✅ env 強制 REPL 分支正確 |
| WINCLI-G1-repl-001 | manual | ❌ TTY | 真 console、agents.toml | `<BIN> repl --agent master` | 互動 banner + prompt；`/help` 列 slash 命令 | `repl` / g1 #13 | ⬜ | ❔ works(UNVERIFIED at runtime；共用 exec 的綠 engine path) |
| WINCLI-G1-repl-002 | e2e | ⚠ key | provider key | `<BIN> repl -c "summarize the README in 3 bullets"` | 串流一個答案後退回 shell（one-shot、不進 loop） | `repl -c` one-shot / g1 | ⬜ | ❔ 共用 exec path（known good） |
| WINCLI-G1-tui-001 | e2e | ✅ | piped stdin（非 TTY） | `"hi" | <BIN> tui` | guard 訊息 + exit 非 0（不 hang/panic） | `tui` non-TTY / g1 #13 | 2026-05-29 win11 | ✅（= WINCLI-INV-02-002，g1 works） |
| WINCLI-G1-exec-001 | e2e | ⚠ key | provider key | `<BIN> exec "what is 2 + 2?"` | 答案串流 stdout + 末尾換行 + exit 0 | `exec` round-trip / g1 #13 | 2026-05-29 win11 | ✅ GREEN（唯一端到端驗證；groq 413→gemini fallback→answer） |

---

## §3. g2 常駐服務 — `serve` · `service` · `mcp` (7 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-G2-serve-001 | e2e | ✅ | 釋出 :7878 | `<BIN> serve --port 7878` then `curl.exe http://127.0.0.1:7878/healthz` | banner `listening on …`；healthz 回 `ok` | `serve` / g2 #13 | 2026-05-29 win11 | ✅ serve HTTP + mesh RPC GREEN |
| WINCLI-G2-serve-002 | e2e | ⚠ env | `$env:PHANTOM_PORT="9999"` | `<BIN> serve` then `curl.exe http://127.0.0.1:9999/healthz` | 綁 9999（precedence `--port`>PHANTOM_PORT>config>7878） | `serve` port / g2 | ⬜ | ❔ 源碼確認 precedence；未實測 env 路徑 |
| WINCLI-G2-serve-003 | e2e | ⚠ port | 先佔住 :7878 | `<BIN> serve --port 7878` | exit 1、bind failure 訊息（非 panic） | `serve` / g2 I4 | ⬜ | ❔ 期望 exit 1 |
| WINCLI-G2-service-001 | e2e | ⚠ admin | user session | `<BIN> service install` | exit 0、`✓ Registered Scheduled Task 'PhantomServe'`、firewall rule（admin 時）/skip（非 admin） | `service install` / g2 #13 | 2026-05-29 win11 | ✅ install via Scheduled Task（非 NT service） |
| WINCLI-G2-service-002 | e2e | ⚠ admin | install 過 | `<BIN> service status` | stdout `registered : yes/no`、`healthz : ok (HTTP 200)`（locale-independent `Get-ScheduledTaskInfo`） | `service status` / g2 | 2026-05-29 win11 | ✅ status 實測 `registered:no, healthz:ok` |
| WINCLI-G2-service-003 | e2e | ⚠ admin | 手動 serve + install | `<BIN> service uninstall` | **見 §11 FAIL-C / WINCLI-INV-09-003**：無條件 taskkill 殺 live serve | `service uninstall` / g2 #13 I9 | 2026-05-29 win11 | 🔴 **[現況 FAIL / code-backlog]** §11 FAIL-C |
| WINCLI-G2-mcp-001 | e2e | ✅ | — | `'{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | <BIN> mcp` | stdout 單一 JSON-RPC 回應、`result.tools` 陣列（實際 58 項）；診斷走 stderr | `mcp` / g2 #13 | 2026-05-29 win11 | ✅ GREEN（cosmetic：help 說 50/docstring 40，實際 58） |

---

## §4. g3 身分與初次設定 — `login`·`logout`·`whoami`·`keys`·`config`·`init`·`onboarding`·`lang` (8 條)

> 本組是 I1/I2/I9 違反的集中地（身分/onboarding handler 無集中 `--help` 解析）。

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-G3-login-001 | e2e | ⚠ non-TTY | PHANTOM_HOME=TempDir、redirected stdin | `$null | <BIN> login` | 快速失敗、不綁 :48181、不 hang（= WINCLI-INV-02-001） | `login` / g3 #13 I2 | ⬜ | 🔴 broken（I1/I2）|
| WINCLI-G3-logout-001 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir、已 login | `<BIN> logout` | exit 0、刪 `auth.json`、**不動 identity.key**（合法登出，非 --help 觸發） | `logout` / g3 #13 | ⬜ | 🟡 功能正確；缺確認保護（I9）+ `--help` 副作用（I1，見 INV-01-002） |
| WINCLI-G3-whoami-001 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir、未 login | `<BIN> whoami` | exit 0、stdout `◇ 尚未登入 — 執行 phantom login` | `whoami` / g3 #13 | 2026-05-29 | ✅ works（`--help` 不一致為輕微） |
| WINCLI-G3-keys-001 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir | `<BIN> keys path` | exit 0、印 env 檔路徑 | `keys path` / g3 #13 | 2026-05-29 | ✅ 核心功能正常（`--help` 走 unknown-subcommand 分支） |
| WINCLI-G3-config-001 | e2e | ✅ | — | `<BIN> config show` | exit 0、印已存 url + 遮罩 token；`config --help` 正確印用法 | `config` / g3 #13（正例） | 2026-05-29 | ✅ works（本組唯一 I1 正例） |
| WINCLI-G3-init-001 | e2e | ⚠ tmpdir | 空目錄 | `<BIN> init` | exit 0、產生 `PHANTOM.md`（**勿在有重要 PHANTOM.md 的目錄跑**） | `init` / g3 #13 | ⬜ | 🟡 功能可用；無條件覆寫既有（I9）+ `--help` 副作用（I1，見 INV-01-001） |
| WINCLI-G3-onboarding-001 | manual | ❌ TTY | 真 console | `<BIN> onboarding` | 起本機網頁 + 開瀏覽器（互動，勿在 headless） | `onboarding` / g3 #13 | ⬜ | 🔴 broken（`--help`/非 TTY 開互動流程；I1/I2） |
| WINCLI-G3-lang-001 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir | `<BIN> lang set zh-TW; <BIN> lang show` | exit 0、之後預設繁中、偏好持久化 | `lang` / g3 #13 | ⬜ | ❔ Windows 未實機驗（g3 UNVERIFIED） |

---

## §5. g4 叢集 — `cluster`·`peer`·`dispatch`·`git`·`node-capabilities`·`worker-setup`·`coordinator` (7 條)

> 本組是 cluster-plane：**正確**送 `X-Cluster-Auth`（對照 I3 — 跨機需 auth，本機免 auth）。

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-G4-cluster-001 | e2e | ⚠ peers | agents.toml [cluster] peers + CLUSTER_SECRET | `<BIN> cluster`（status） | stdout `this node:`、每 peer ✓/✗ + RTT、`summary: X/N peers reachable` | `cluster status` / g4 #13 | 2026-05-29 win11 | ✅ works（實測 `3/4 peers reachable`，最健康組 7 works） |
| WINCLI-G4-peer-001 | e2e | ⚠ peers | configured peers | `<BIN> peer list` | 表 `URL · NAME · STATUS · TASKS` | `peer list` / g4 #13 | 2026-05-29 win11 | ✅ works（cosmetic：offline row NAME 顯示 raw URL） |
| WINCLI-G4-peer-002 | e2e | ⚠ non-TTY | — | `<BIN> peer --help` | **驗收記錄**：現況印 `Unknown peer subcommand: --help` + usage（違 I1） | `peer --help` / g4 I1 | 2026-05-29 win11 | 🟡 無 `peer --help`（I1 輕微） |
| WINCLI-G4-dispatch-001 | e2e | ⚠ peers | peers.json + cluster_secret | `<BIN> dispatch --tag rust "cargo build"` | 路由到 cap=rust 的 peer、回結果（或降級 fallback） | `dispatch` / g4 #13 | 2026-05-29 win11 | ✅ dispatch-avail GREEN |
| WINCLI-G4-nodecap-001 | e2e | ✅ | — | `<BIN> node-capabilities` | stdout `Platform: windows x86_64`、`Service model: windows_service`、`gpu_compute:directx`、`local_llm:ollama` | `node-capabilities` / g4 #13 | 2026-05-29 win11 | ✅ human report 實測正確（`--json` 路徑未驗，見 INV-10-003） |
| WINCLI-G4-worker-001 | e2e | ✅ | — | `<BIN> worker-setup --hub http://x:7878 --name z13` | 印 capability report + `--- Setup Runbook for windows ---`（`sc.exe create … start= auto`），**不**自己跑 sc.exe | `worker-setup` / g4 #13 | 2026-05-29 win11 | ✅ runbook + report 實測（純印、無副作用） |
| WINCLI-G4-coord-001 | manual | ❌ block | — | `<BIN> coordinator` | 起 hub 前景 process（會 block）— **不在自動套件跑** | `coordinator` / g4 #13 | ⬜ | ❔ UNVERIFIED（只驗一行 usage；running 會 block） |

---

## §6. g5 自我改進 — `evolve`·`autoevolve`·`swarm`·`skill run` (5 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-G5-evolve-001 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir + EVOLVE-GOALS.md | `<BIN> evolve goals list`；`<BIN> evolve list --json` | goals 列 pending/done；`list --json` 單一 JSON（空集→`[]`） | `evolve` / g5 #13 | 2026-05-29 win11 | ✅ works（實測 goals 11 pending/8 done、list 41 checkpoints）。cosmetic：`evolve --help` exit 1 truncated |
| WINCLI-G5-autoevolve-001 | e2e | ⚠ admin | — | `<BIN> autoevolve schedule status` | `registered : no`（未裝時）；install 用 `schtasks /SC MINUTE` | `autoevolve schedule` / g5 #13 | 2026-05-29 win11 | ✅ works（cosmetic：`log -n 3` 印 10 行；help 文案仍寫 macOS LaunchAgent） |
| WINCLI-G5-swarm-001 | e2e | ⚠ peers | online peers | `<BIN> swarm "summarize each node's recent commits"` | banner + per-peer 回應 + `✓ swarm complete cost: $X` | `swarm` / g5 #13 | 2026-05-29 win11 | ✅ works（共用綠 mesh RPC path；`swarm --help` 印 usage 不動作=I1 正）。INV-11 註記：CLI 顯式輸入 `phantom swarm <PROMPT>` = owner 顯式同意，與手機 chat 隱式路由不同層，故不違 INV-11。 |
| WINCLI-G5-swarm-002 | e2e | ⚠ none | 無 prompt | `<BIN> swarm` | exit 1 + `Usage: phantom swarm <PROMPT> …`（I4） | `swarm` arg / g5 | ⬜ | ❔ 期望 exit 1 |
| WINCLI-G5-skill-001 | e2e | ⚠ release | Windows release binary | `<BIN> skill run .\x.md --dry-run` | exit 1 + 可操作 rebuild 訊息（= WINCLI-INV-05-003，degrade-not-fail） | `skill run` / g5 #13 | 2026-05-29 win11 | 🟡 partial（feature-gated out；`release-windows.yml` 已加 feature 修） |

---

## §7. g6 生活節點 — `event capture`·`coach review`·`focus`·`logs`·`data delete`·`daily-review` (8 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-G6-event-001 | e2e | ⚠ serve+key | 起 serve + 有效 Gemini key | `<BIN> event capture --kind food --text "grilled chicken salad" --tag fat_loss` | exit 0、stdout `{"event_id":…,"analysis":{…}}`、event age 加密落地 events/ | `event capture` / g6 #13 | 2026-05-29 win11 | 🟡 partial（寫入路徑同步 analyze、失敗 exit 1 違 I5，見 INV-05-001）；`--help` 違 I1 |
| WINCLI-G6-coach-001 | e2e | ⚠ tmpHOME+key | PHANTOM_HOME=TempDir + events + identity.key | `<BIN> coach review --date 2026-05-29 --save` | exit 0、stdout markdown 回顧、stderr `saved (age-encrypted) to …\reviews\2026-05-29.md` | `coach review` / g6 #13 | 2026-05-29 win11 | ✅ works（EventStore age 加密解密正常；明日行動段免費金鑰可能降級） |
| WINCLI-G6-coach-002 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir、無 identity.key | `<BIN> coach review --save` | exit 0、stderr `saved (plaintext — no identity.key found) …`（降級非失敗） | `coach review` / g6 I5 | ⬜ | ✅（推定，源碼降級分支） |
| WINCLI-G6-focus-001 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir、**重建後**接線 binary | `<BIN> focus start --minutes 50 --task "write SPEC-46"; <BIN> focus status; <BIN> focus stop` | start→active→complete + 存 focus event；exit 0 | `focus` / g6 #13 | 2026-05-29 win11 | 🔴→❔ **已安裝舊 binary** 回 `擷取尚未接線 — 卡在 SPEC-21 Stage 4`（exit 2）；**源碼已接線**（`focus_session.rs` 2026-05-29）→ stale-binary，重建後應轉綠。owner SPEC-21/重建 |
| WINCLI-G6-logs-001 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir + events.jsonl | `<BIN> logs --since 1h --kind food` | stderr 標頭 `# N events …`、stdout `[<iso>] food  <summary>`、exit 0 | `logs` / g6 #13 | ⬜ | ✅（推定 works；`--help` 正常印用法） |
| WINCLI-G6-data-001 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir + 12 events | `<BIN> data delete --all`（dry）then `--all --yes` | dry：exit 1 印計畫不刪；`--yes`：exit 0 真刪（= INV-09-001/002） | `data delete` / g6 #13（I9 正例） | 2026-05-29 win11 | ✅ I9 雙重保護正例 |
| WINCLI-G6-daily-001 | e2e | ⚠ freekey | Groq free key + 當日多事件 | `<BIN> daily-review` | **理想**：分塊降級成功。**現況**：單次吃爆 6000 TPM→413（見 INV-05-002） | `daily-review` / g6 #13 | 2026-05-29 win11 | 🟡 partial（I5 違反）。**命名待釐清**：源碼無字面 `daily-review` handler（最接近 `review`/`coach review`），dispatch 位置 UNVERIFIED |
| WINCLI-G6-daily-002 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir | `<BIN> review 2026-05-29 --json` | **離線**彙整（不呼 LLM）、`--json` ASCII-safe（與 daily-review 區別：review 離線） | `review` / g9 #2 | ⬜ | ❔ `--json` Windows 未驗；釐清 review vs daily-review vs coach review 命名（g9 待辦） |

---

## §8. g7 診斷 — `doctor`·`selftest`·`self-update`·`debug`·`--version` (6 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-G7-doctor-001 | e2e | ✅ | — | `<BIN> doctor` | exit 0、分段健康報告（binary/config/keys/serve/network/Windows integrations）、warning=⚠ 不抬 exit | `doctor` / g7 #13 | 2026-05-29 win11 | ✅ works（exit 0；echo 的 `built unknown` 是 --version 缺陷） |
| WINCLI-G7-selftest-001 | e2e | ⚠ gitbash | Git Bash 在 PATH | `<BIN> selftest --list` | fixed-width 表 `FILE FEATURE PRIORITY DESCRIPTION`、exit 0 | `selftest` text / g7 #13 | 2026-05-29 win11 | ✅ text/`--list`/`--help` works |
| WINCLI-G7-selftest-002 | e2e | ⚠ PHANTOM_BIN | 釘 PHANTOM_BIN=0.6.0-rc.1（避 stale 0.4.0） | `<BIN> selftest --json` | 非空單一 JSON（= INV-10-002） | `selftest --json` / g7 #13 | 2026-05-29 win11 | 🔴 Windows 吐 0 bytes（含 PATH 解析 stale 0.4.0）。owner core/scripts |
| WINCLI-G7-selfupdate-001 | e2e | ✅ | — | `<BIN> self-update --dry-run` | exit 0、stderr 印 plan（current/source/`would download but not install`）、**不下載不替換** | `self-update --dry-run` / g7 #13 | 2026-05-29 win11 | ✅ dry-run works（真 swap 未跑；I9：真跑會 taskkill phantom.exe 可中斷 live serve） |
| WINCLI-G7-debug-001 | e2e | ✅ | — | `<BIN> debug` | exit 0、`=== phantom debug bundle ===` … `=== end …`、secrets 遮罩/`[REDACTED]` | `debug` / g7 #13 | 2026-05-29 win11 | ✅ works（full bundle、masked） |
| WINCLI-G7-version-001 | e2e | ✅ | — | `<BIN> --version` | full line；**驗收**：日期欄須真實（非 `built unknown`，= INV-08-001） | `--version` / g7 #13 | 2026-05-29 win11 | 🔴 `built unknown`（build.rs 用 `date -u`）。owner core/build.rs |

---

## §9. g8 代理與工作階段 — `providers`·`models`·`sessions`·`workspace` (4 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-G8-providers-001 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir + agents.toml | `<BIN> providers list` | 印 providers + per-agent failover order；`priority <agent> <p1>…` 原地改寫 agents.toml | `providers` / g8 #13 | 2026-05-29 | ✅ works（cosmetic glyph/CP950 risk；主結果走 stderr=I11 註記） |
| WINCLI-G8-models-001 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir + models-cache.json | `<BIN> models status` | 印 cache 路徑 + `TTL: 60m` + per-provider 行 + `[stale, Nd ago]` 標記 | `models` / g8 #13 | 2026-05-29 | ✅ works（cosmetic：status 不 auto-refresh stale） |
| WINCLI-G8-sessions-001 | e2e | ⚠ login | 已 login（broker token） | `<BIN> sessions` | exit 0、`◆ N active session(s):`（= INV-03-002） | `sessions` / g8 #13 I3 | 2026-05-29 win11 | 🔴 **[現況 FAIL / code-backlog]** 回 `HTTP 401 unauthenticated`（§11 FAIL-A 的 broker-token 變體）。owner core-mesh T-AUTH-LOCAL |
| WINCLI-G8-workspace-001 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir | `<BIN> workspace set C:\Users\<u>\Projects\foo coder; <BIN> workspace show` | set 改寫 [workspace] block；show 印 default_dir/pinned_agent | `workspace` / g8 #13 | 2026-05-29 | ✅ works |

---

## §10. g9 附加命令（dispatch-only，未列 `--help`）— `food`·`habit`·`note`·`recall`·`review`·`hello`·`update`·`version` (8 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINCLI-G9-food-001 | e2e | ⚠ tmpHOME+key | PHANTOM_HOME=TempDir + serve/key | `<BIN> food "雞胸肉沙拉" --tag fat_loss` | exit 0、寫 food event（加密）、shame-free 教練回饋 | `food` / g9 #13 | ⬜ | ❔ UNVERIFIED（Windows 未跑；`--help` 源碼符合 I1） |
| WINCLI-G9-habit-001 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir | `<BIN> habit create morning-run --label "晨跑"; <BIN> habit checkin morning-run "5km"` | exit 0、寫習慣定義 + checkin 事件 | `habit` / g9 #13 | ⬜ | ❔ UNVERIFIED（`--help` 符合 I1） |
| WINCLI-G9-note-001 | e2e | ✅ | PHANTOM_HOME=TempDir | `echo "pipeline note" | <BIN> note --tag idea` | exit 0、從 stdin 讀寫 text event（= INV-02-004） | `note` / g9 #13 I2 | 2026-05-29 源碼確認 | ✅ I2 正例（IsTerminal 區分管線）；Windows 管線編碼未實測 |
| WINCLI-G9-recall-001 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir + events | `<BIN> recall "蛋白質" --kind food --since 2026-05-01 --json` | exit 0、符合事件清單；`--json` ASCII-safe 單一文件 | `recall` / g9 #13 | ⬜ | ❔ `--json` Windows 未測（對照 selftest 0-byte，須驗 I10 非空）；`--help` 符合 I1 |
| WINCLI-G9-review-001 | e2e | ⚠ tmpHOME | PHANTOM_HOME=TempDir + events | `<BIN> review 2026-05-29 --json` | exit 0、**離線**彙整（不呼 LLM） | `review` / g9 #13 | ⬜ | ❔ 命名待釐清（review 離線 vs daily-review LLM vs coach review LLM）；`--help` 符合 I1 |
| WINCLI-G9-hello-001 | e2e | ⚠ | — | `<BIN> hello` | （行為待定） | `hello` / g9 #13 | ⬜ | ❔ UNVERIFIED：列於命令清單但找不到獨立 handler→spec 決定保留/移除（g9 待辦） |
| WINCLI-G9-update-001 | e2e | ⚠ | — | `<BIN> update`（疑 self-update alias） | 若為 alias 則同 self-update | `update` / g9 #13 | ⬜ | ❔ UNVERIFIED：確認是否 = `self-update` 別名（g9 待辦） |
| WINCLI-G9-version-001 | e2e | ✅ | — | `<BIN> version` | 印版本（推測等同 `--version`） | `version` / g9 #13 | ⬜ | 🟡 partial（繼承 I8：Windows `built unknown`）；確認與 `--version` 共用路徑 |

---

## §11. 已知 code 問題 → FAIL 驗收條件（修碼 wave 前現況 FAIL）

> **docs-only 鐵則**：以下三項是 code 缺陷，本檔**不修碼**，只記成「可重複測試的驗收條件」+ 標 `[現況 FAIL / code-backlog]`。修碼後對映 case 應轉綠（PASS），屆時本節與對映 INV/G case 的狀態欄同步更新。

### FAIL-A — I3：本機平面被遠端授權擋（`/api/chat` 401 + `sessions` 401）

| 欄 | 內容 |
|---|---|
| **對映 case** | `WINCLI-INV-03-001`（`/api/chat`）· `WINCLI-INV-03-002` / `WINCLI-G8-sessions-001`（`sessions`） |
| **現況** | 本機 `POST /api/chat` 回 **401**（`X-Cluster-Auth` 擋本機 UI）；`phantom sessions` 回 **401 `{"error":"unauthenticated"}`**（對 broker 的 Bearer token 未授權）。兩者違反 I3「本機/使用者命令不該被遠端授權擋」，但**根因不同**（一個是 cluster-auth HMAC、一個是 broker bearer token），須分別修。 |
| **驗收（修碼後 PASS 條件）** | (1) 本機起 serve 後 `curl POST http://127.0.0.1:7878/api/chat` 回 **200**（非 401）。(2) 已 login 後 `phantom sessions` exit 0 並印 session 清單（非 `HTTP 401`）。(3) 對照組 `GET /api/sessions` 仍 200（WINCLI-INV-03-003 不退化）。 |
| **標記** | 🔴 `[現況 FAIL / code-backlog]` · owner **core-mesh** · task **T-AUTH-LOCAL** · SPEC-46 spine §2 I3 / g8 #13 |

### FAIL-B — I6：`PHANTOM_HOME` 未 honor（`home_dir()` 硬編）

| 欄 | 內容 |
|---|---|
| **對映 case** | `WINCLI-INV-06-001` / `-002` / `-003`（資料根 / `keys`/`providers`） |
| **現況** | `agents_toml_path()` = `dirs::home_dir() + ".phantom-mesh/agents.toml"`，**無 `PHANTOM_HOME` 分支**（g8 cross-cutting note 明載）；`auth_path()`/多處資料根同樣只有 home-rooted 路徑（子代理回報）。Windows 上 `dirs::home_dir()` 又不理 `$HOME`，導致 `$HOME`-redirect 測試污染真實 `~/.phantom-mesh`、且 `PHANTOM_HOME`（spec'd data-root override）無效。 |
| **驗收（修碼後 PASS 條件）** | (1) 設 `$env:PHANTOM_HOME=<TempDir D>` 後，任何寫事件/設定命令一律寫入 **D**、`~/.phantom-mesh` 零新增。(2) `phantom keys path` / `phantom providers list` 解析路徑在 **D** 下。(3) 統一 home-resolver（追蹤 #322），測試文件改引 `PHANTOM_HOME` 而非 `$HOME`-redirect。 |
| **標記** | 🔴 `[現況 FAIL / code-backlog]` · owner **core**（home-resolver 統一 #322）· SPEC-46 spine §2 I6 / g8 cross-cutting note |

### FAIL-C — I9：`service uninstall` 無條件 `taskkill /F /IM phantom.exe`

| 欄 | 內容 |
|---|---|
| **對映 case** | `WINCLI-INV-09-003` / `WINCLI-G2-service-003` |
| **現況** | `service/windows.rs:~233-235` 在 `uninstall` 時**無條件** `taskkill /F /IM phantom.exe`、無 guard 無確認。使用者若手動跑了一個 `phantom serve`（非 task 啟動），`service uninstall` 會靜默殺掉它（違反 I9「uninstall 不得無警告停掉正在跑的 serve」）。 |
| **驗收（修碼後 PASS 條件）** | (1) 手動起一個 `phantom serve`（非 task）後 `phantom service uninstall`：手動 serve **仍存活**（或命令先印「將終止 PID … — 加 `--yes` 確認」並在無 `--yes` 時不殺）。(2) uninstall 仍正確刪 `PhantomServe` task + best-effort 移 firewall rule。(3) 理想實作：只殺 task-spawned 實例（以 PID/來源辨識）或要求明確 `--yes`。 |
| **標記** | 🔴 `[現況 FAIL / code-backlog]` · owner **core (Windows service)** · SPEC-46 spine §2 I9 / g2 #13 |

> **三大 FAIL 與 Charter §D D.1 對齊**：win CLI row 的「關鍵缺口」明列 `I3 401[broken]`、`PHANTOM_HOME 未 honor[broken]`、`uninstall taskkill[broken]`——本節即把該三洞落成可重複驗收條件，補上 INV-15 三件套的「測試件」。

---

## §12. 覆蓋總覽

### 12.1 case 計數（按節）

| 節 | 範圍 | case 數 |
|---|---|---:|
| §1.I1–I13 | 跨命令不變量驗收（SPEC-46 核心） | 53 |
| §2 g1 | `(bare)`/`repl`/`tui`/`exec` | 6 |
| §3 g2 | `serve`/`service`/`mcp` | 7 |
| §4 g3 | 身分/onboarding 8 命令 | 8 |
| §5 g4 | cluster 7 命令 | 7 |
| §6 g5 | self-improve 4 命令 | 5 |
| §7 g6 | life-node 6 命令 | 8 |
| §8 g7 | diagnostics 5 命令 | 6 |
| §9 g8 | agent/session 4 命令 | 4 |
| §10 g9 | dispatch-only 8 命令 | 8 |
| **合計** | I1–I13 + ~48 命令 CUJ | **112** |

> §11 的三大 FAIL 驗收條件不另計 case 數（其對映 case 已含於 §1/§3/§9 內），但獨立成節以高亮 code-backlog。

### 12.2 狀態分佈（依首個 emoji 機械重數）

| 狀態 | 意義 | 數 |
|---|---|---:|
| ✅ | 已驗/works | 48 |
| 🟡 | partial/誠實債 | 15 |
| 🔴 | broken/重要缺（含 3 大 code FAIL + focus stale-binary + selftest --json + --version 日期） | 22 |
| ❔/⬜ | UNVERIFIED/未做（多為需 fixture/真 serve/真 console/CP950/admin） | 27 |
| **合計** | | **112** |

### 12.3 與 Charter 對映（INV-15/16 / K2 / BRK-10）

- **INV-15 三件套補齊**：SPEC-46（流程文件 g1–g9 + 驗收條件 I1–I13）+ **本檔（測試 case DB）** = win CLI surface 三件套齊。Charter §D D.1 win CLI row「測試 case DB 🟥 無對等 case DB」→ **本檔填洞**。
- **K2（surface case DB）**：win CLI 現有對等 mac.md 級逐條 case DB、CLI runner 可直讀（每條有 ID/cmd/expected/狀態）。
- **BRK-10**：「五 surface 無對等 mac.md case DB」中的 **win CLI 一面已補**（linux CLI / 桌面 app / android / ios 仍待後續 slice）。
- **INV-16（feature↔test 雙向）**：本檔每條 case 的 `Verifies` 欄綁回 SPEC-46 不變量 + 命令 + g* 檔；反向由 SPEC-46 各 g* #13 與 spine §3 矩陣指向本檔 WINCLI-* ID。
- **回歸守護**：規格指定 `scripts/qa/win-regression.mjs` 依 I1–I13 編號加斷言（spine §2/§5）；本檔 §1 即該守護的逐條驗收清單來源。

### 12.4 最高槓桿修復序（對映 SPEC-46 spine §5 exec-plan）

1. **E0（環境）**：釘 `PHANTOM_BIN`/清 PATH stale 0.4.0 → 一次解 `selftest --json`（INV-10-002）+ `focus`（G6-focus-001）兩個 broken。
2. **§11 FAIL-A（I3 401）**：T-AUTH-LOCAL，本機平面豁免遠端授權。
3. **§11 FAIL-C（I9 taskkill）**：uninstall 不殺 live serve。
4. **§11 FAIL-B（I6 PHANTOM_HOME）**：home-resolver 統一（#322）。
5. **I1/I2/I9（g3）**：分派層集中 `--help` 攔截 + 破壞性保護 + 非 TTY 快速失敗（INV-01-001~004 / INV-02-001/005 / INV-09）。
6. **I5/I8（降級/日期）**：event capture async、daily-review 分塊、build.rs 跨平台日期（INV-05-001/002 / INV-08-001）。

---

> *本檔為 docs-only 產物（Charter Wave 2 win-cli slice，2026-06-12），對應升格為 ACTIVE-CANDIDATE 的 `SPEC-46-windows-cli-behavior/`。未改任何 `core/src`/`app`/`scripts` 程式。三大 code FAIL（§11）標 `[現況 FAIL / code-backlog]`，待修碼 wave 處理後同步轉綠。*
