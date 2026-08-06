# spectyn-mesh v0.6.0 — 發行說明

**Tag**: `v0.6.0`（目標切版日 2026-06-15，S1 已鎖定；S2 = 06-17 僅為滑點備援）
**日期**: 2026-06-11（草稿；切版前需操作者審閱 + 陌生人測試）
**承接**: `docs/RELEASE-NOTES-v0.6.0-rc1.md`（2026-05-25）。rc1 的 §2（自 v0.5.0 新增）、§3（延後清單）、§4（破壞性變更）、§5（遷移指南）**仍然有效且不在此重複**；本檔記錄 rc1 草稿之後落地的內容，並以更新後的已知例外清單取代 rc1 §6。
**支柱對應說明**: 本檔採 2026-05-17 轉向後的三支柱框架——P1 到處能跑（runs anywhere）/ P2 會學習（learns）/ P3 可遠端指揮（remote command）。BIG-GOAL 的 P4（encryption-first）項目歸入 §3 安全段。

---

## §1 重點摘要（TL;DR）

v0.6.0 = rc1 的四支柱核心 + 三週的誠實收尾：E003 教練節點與 E005 萃取後端落地（後者維持 feature-gated）、E006 一鍵安裝與 30 秒 demo 基材落地，然後是 06-07 以降至本稿快照為止的一波 **29 個 first-parent 合併（共 68 個 commit，`4c213b29..4d4884ba`，2026-06-07 → 06-11；快照後至切 tag 前仍有合併持續落地，見變更紀錄待補項）**——內容幾乎全是平台正確性（Windows/Linux/Android）、#321 安全修正、與測試隔離工程。沒有新功能煙火；有的是讓既有功能在五個平台上真的能跑、且不在無認證情況下被遠端打穿。

如果你只讀一段：這個版本的價值是「誠實」。功能旗標關著的東西（cluster-heartbeat、技能記憶）在 §7 列得清清楚楚；沒驗過的安裝路徑（L1）與沒到位的硬體（Pi）也是。

---

## §2 rc1（05-25）→ 本波（06-07）之間的落地

逐項證據見 `ROADMAP-v0.6.0.md` 的 2026-06-02 實況欄。摘要：

- **E003 教練節點 SHIPPED**：`spectyn coach review --date` CLI、`daily_review.rs` 聚合器（經 E004 解密）、shame-free prompt lint、`--save` 加密寫入 `reviews/<date>.md`、Gemini→Groq→Ollama 降級鏈，外加 spec 之外的每日排程器（`coach_scheduler.rs` + `spectyn serve` 內嵌 21:00 timer）。
- **E005 萃取後端落地但 gated**：`from_daily_review.rs` 萃取器（預設編譯）+ 3 個 RPC 端點（`serve_skillbank.rs`，鎖在 `experimental-memory` 之後）。見 §7.2。
- **E006 基材落地**：`scripts/install.sh` / `install.ps1`（SHA256 先驗證後執行）、`demo-30sec-life-hello.sh`、README 頂部 30 秒走查、daemon-down 可行動錯誤提示。
- **OSS 整備副作用**：commit `9914cc6b`（05-30）刪除了整個 `docs/superpowers/runbooks/`——E007 煙霧測試 runbook 由本版重建（`docs/superpowers/runbooks/E007-release-smoke.md`）。

## §3 安全（#321 + Windows DPAPI keystore）

本版最重要的一段。外部審查 issue **#321** 對 serve 層找出的高風險項，已於 `77c359c6` / `4d4884ba` 移植到 main：

- **Fail-open → fail-closed**：`rpc_squad_dispatch` 與 `rpc_evolve_handoff` 原本在認證設定缺漏時直接放行（未認證 RCE 等級），現一律走 `require_cluster_auth_dual` 拒絕。（#321 findings #1、#2）
- **task/assign 嚴格 at-most-once 去重**：關閉重送造成的 double-spawn。（finding #5）
- **`/api/events` multipart 上限**：`MAX_EVENT_PARTS=64`、單一 part 32 MiB、總量 128 MiB——擷取端不再可被無上限上傳打掛。（bonus）
- **Onboarding 非 table 設定優雅回 500** 而非 panic。（bonus）

**Windows DPAPI / Credential Manager keystore arm**（`dcdf4810` / `bd04676b`）：Windows 上的身分種子改存 OS 憑證儲存（DPAPI 加密），附 e2e 測試 `core/tests/keystore_windows_dpapi_e2e.rs`。Windows 因此先於 macOS/iOS（仍在 keystore lineage 上，見 §7.5）拿到原生 keystore。

**已知殘留**：#321 finding #6（`rpc_tool_call` 使用 `ToolsConfig::default`，遠端 tool_call 忽略節點 `[tools]` 設定）目前只在 lineage 分支上修好，main 尚未——列入 §7.5。

## §4 P1 — 到處能跑（runs anywhere）

`4c213b29..HEAD` 中最大的一桶：讓 Windows / Linux / Android 不再是「理論上支援」。

**Windows 正確性修正波**（dirs::home_dir() 不認 `$HOME` 這一族問題的系統性收口）：
- runtime `agents.toml` home 解析改走共用的 `home_dir_lenient`（`7cd6a196` / `660455a1`）。
- providers 憑證探索 home 解析 fallback（`f643cb7e` / `aaccc0ae`）。
- skill DB 路徑改用 `dirs::home_dir()` 而非裸 `$HOME`（`6ce0b5db` / `22efdd8c`）。
- sh-based mDNS 探索在 Windows 上優雅跳過而非失敗（`10511131` / `83f37edb`）。
- 桌面 app 版本飄移收斂為單一事實來源（#306 / `b1d948c7`）。
- （家目錄解析的全面統一仍是 #322 追蹤項——本波是逐點修正，不是 resolver 統一。）

**Linux 打包與 keystore**：
- `package-linux.sh` 新增 `--rpm` 與 `--appimage` 模式（#313 / `db6eca56`），偏好 `rpmbuild`（`5b663b11`），rpm `%install` 引號加固（`776be2e5`）。
- `SPECTYN_KEYSTORE=file` 環境覆寫，讓無 Secret Service 的環境可控退回檔案儲存（#314 / `c38ade2a`）。
- 3 份 Linux backlog spec（secret-service keystore / AppImage / rpm，#307）。

**Android keystore**：
- JNI bridge + `IdentityKeystore.kt`（EncryptedSharedPreferences）乾淨落地（#309 / `ba092240`）；種子寫入改 `commit()` 確保耐久（`a5d75cdc`）。

**Mesh 強健性**：`parse_mdns_urls` 非 ASCII char-boundary panic 修正（`ddb56dbf` / `ff6c6b57`）。

## §5 P2 — 會學習（learns）

- **Onboarding 首次執行即可用**：first-run `agents.toml` 烘入一個真的解析得到的模型 + 回歸測試（#308 / `4721b2bd`）；D1 login-first 欄位補齊 + v7 效能預算（#315 / `c20f5d46`）。
- **模型解析一致性**：streaming 的第三優先序路徑改走共用 `resolve_entry_model`（`f50d9d10` / `91105efa`）——三條解析路徑不再各自為政。
- **Providers 不再 panic**：Stage-4 stub arm 回傳型別化 `Err` 而非 `unimplemented!` panic（`c813f0e8` / `297ec495`）。
- **設定診斷**：設定檔讀取/解析失敗時 stderr 警告（`493bcae9` / `114b9b3f`），且每路徑至多警告一次（`04ba71e4` / `5b37f4bc`）。
- **擷取/教練測試補強**：cuj02 食物/專注 hermetic 整合測試 + 涵蓋文件對齊至 138 案例（`5d87ab7d` / `571821db`）；cuj03 broker login token 持久化 hermetic（#316 / `25457b5d`）。

## §6 P3 — 可遠端指揮（remote command）＋ 測試/發行工程

- **serve 層加固** = §3 的 #321 全部項目（fail-closed 認證、at-most-once 派工、multipart 上限）——遠端指揮面的信任邊界就是這裡。
- **`spectyn exec --json` AgentEvent JSONL 契約**：hermetic 跨平台 schema 測試（#317 / `b545909a`）——遠端控制器消費的機器可讀介面從此有測試釘住。
- **多節點開發機具**：dev-loop / dev-cluster / local-ai runner + backlog + review 機制（scripts-only，#310 / `6727d12d`）；M1 Windows login→LLM 流程驗證紀錄（#311 / `1667c363`）。
- **serve 測試移除固定埠**：`serve_starts` 改用 OS ephemeral port（#320 / `957d23a0`）。

**測試與發行工程（橫切）**：
- `env_lock` 全面收口：所有改動 HOME 的測試持共享鎖（`c365b539` / `4d26bb75`、food 競態 `bb0ecb9c` / `e0a7e931`）；連續 6 次完整 `--lib` 綠。
- 誠實 skip 標記統一為 `SKIPPED: <test> — <reason>`（`e4e5687a` / `60c01809`）。
- Windows 測試對等性與安全閥：cli-win D22/D23/D24/D28 加固移植（#318 / `621a8041`）、D24 絕不碰操作者真實 `identity.key`（`0bdbb3e4`）、`$HOME`-redirect 測試上 `#[cfg(unix)]` 閘（#319 / `3165254a`、#317 / `78aaa783`、`1c7af082`）。

## §7 已知例外（Known Exceptions）

取代 rc1 §6。煙霧矩陣的每個合法 SKIP 代碼都必須對應到本清單中的一項（代碼對照表見 runbook §5）；反之，本清單也含不對應任何 SKIP 代碼的純記錄項（如 5–7）。

1. **cluster-heartbeat 仍 feature-gated**（`experimental-cluster-heartbeat`，預設 build 不含）：預設發行的二進位**永遠不會**把節點翻成 Unhealthy/恢復 Healthy；`cross_host_recovery.sh` 需要旗標建置的二進位。心跳狀態機的單元/整合測試在旗標 build 下全綠——這是範圍決策，不是壞掉。
2. **E005 技能庫 feature-gated 且未接線**（`experimental-memory`，預設 off）：即使旗標編譯，`spectyn serve` 也不會把 `state.skill_memory` 接成 `Some`，三個端點回 503。`spectyn skill extract --commit` CLI、F402 recall-trace 端點、與 serve 端正式接線**延後至 v0.7**（位於 lineage 管制檔案內）。FTS5 p99 < 200ms 從未實測；FTS5 用 `unicode61` tokenizer，**CJK 斷詞未測試**。萃取器本體（`from_daily_review.rs`）預設編譯且有測試。
3. **L1 安裝路徑未驗證**：`phantommesh.io` 的 curl|sh / ps1 一鍵端點需要操作者持有的 Cloudflare 憑證，切版時未開通。`install.sh` / `install.ps1` 腳本本身在 main 上且過審（SHA256 先驗後執行）；**已驗證的替代路徑**是 GitHub Releases 工件直下。60 秒全新機器安裝檢核未對 live 端點執行過。
4. **Pi 節點以艦隊替代（documented substitute）**：E001 spec 硬性要求 Pi 4 aarch64 Linux 實機；本次切版無 Pi 可用，依 roadmap 滑點規則以艦隊節點（ayaneo=Windows、WSL=Linux、acer=Android、Mac 兩標的）替代並如實記錄。aarch64-Linux-on-Pi 在切版時未測。
5. **lineage 殘留項**（Mac/codex keystore lineage，~170 commits，截至本稿未合併）：#321 finding #6（`rpc_tool_call` 的 `ToolsConfig::default`）僅 lineage 已修、main 未修；macOS/iOS Keychain 原生 arm；Android keystore arm 優雅降級 + logout 非短路清除；coach scheduler/install-schedule 整合主幹（`step3-coach-install-schedule`）。隨 lineage 合併或 v0.6.x 收。
6. **承襲 rc1 未變**：8/11 LLM 供應商回 `ConfigInvalid`；embedding 語意召回延 v0.7；iOS keychain 退回沙箱加密檔案；2 個 `service::macos` launchctl 測試 fixture 紅（執行期路徑可用）。
7. **切版前雜項**（不擋出貨但要做）：~~版本號飄移需收斂（core=`0.6.0-rc.1`、pm-types=`0.6.0`、app/src-tauri=`0.6.1`）~~ **[已解 — drift 回填 2026-06-12]** 四個 manifest 已全部收斂為 `0.6.0`：`core/Cargo.toml`、`app/package.json`、`app/src-tauri/Cargo.toml`、`app/src-tauri/tauri.conf.json` 皆 `version = "0.6.0"`；`spectyn --version` / `--version --short` 的字串唯一來源為 `env!("CARGO_PKG_VERSION")`（裁決依據：`core/src/bin/spectyn.rs:3718`/`:3722`），故實機輸出即 `0.6.0`（無 `-rc.1`、無 `0.6.1`）。舊的 `v0.6.0-rc1` tag（05-25 @ `f4a5c7e0`）仍過時，需操作者決定重切或補 rc2；epic spec 驗收勾選與 F600 計分板的中文標題漂移需在 06-13 凍結檢查前修。
8. **操作者持有項未煙測（`S-OPR`）**：(a) 教練日報的 live 推送通道（Telegram / email，SPEC-24 coach-delivery 面）需要操作者持有的 bot token／寄送憑證——煙霧矩陣只驗證 CLI markdown 輸出，live 推送在切版時未測；(b) 發行說明的「陌生人測試」需操作者安排真人執行，是切 tag 前的硬性檢核（見變更紀錄待辦），若操作者明示延期則如實記入本項。煙霧矩陣中標 `SKIP(S-OPR)` 的儲存格即對應本條。

---

## §8 升級路徑

與 rc1 §5/§8 相同（事件前向讀取、deep-link 重註冊、OAuth 惰性重加密；`git pull` → rebuild → `spectyn selftest` → 30 秒 demo），無新增遷移步驟。Windows 使用者首次啟動會把身分種子遷入 DPAPI/Credential Manager（自動，無需動作）。

## §9 統計（rc1 → v0.6.0 增量）

- **29** 個 first-parent 合併 / **68** 個 commit（`4c213b29..4d4884ba`，06-07 → 06-11），其中 14 個批次 PR（#306–#320，無 #312）+ 15 個雙閘直接合併
- **5** 項 #321 高風險安全修正移植到 main（+2 bonus 加固）
- **1** 個新原生 keystore 平台（Windows DPAPI）；Android EncryptedSharedPreferences wrapper 落地
- **2** 種新 Linux 打包格式（AppImage、rpm）
- **6** 連續完整 `cargo test --lib` 綠（env_lock 收口後、lineage 安全移植前）

---

## 變更紀錄（Changelog）

- **2026-06-12（P1-5 版本收斂 drift 回填）** — §7 條目 7 的「版本號飄移需收斂」標為已解：四個 manifest 已全部為 `0.6.0`（讀碟核實 `core/Cargo.toml`、`app/package.json`、`app/src-tauri/Cargo.toml`、`app/src-tauri/tauri.conf.json`）；以 `core/src/bin/spectyn.rs:3718`/`:3722`（`env!("CARGO_PKG_VERSION")`）為 as-built 裁決 `--version` 輸出 = `0.6.0`。core/tests 版本斷言（`cli_win.rs`/`cli_linux.rs`/`v5_smoke_linux.rs`/`v4_e2e_desktop_*.rs`）本已為結構性/容忍式（`X.Y.Z[-pre]`、semver-ish ≥2 dots），切版至 `0.6.0` 不會 false-FAIL，無需修改。
- **2026-06-11（vendor gate 修訂）** — 新增 §7 條目 8（`S-OPR` 操作者持有項），補齊 runbook §5 SKIP 代碼對照缺口；§7 前言改述對應方向；runbook 門檻算式措辭釐清為 PASS/(PASS+FAIL)。
- **2026-06-11** — 初稿。自 rc1 草稿（05-25）承接 + `4c213b29..4d4884ba` 29 合併分桶 + 已知例外清單更新。待辦：操作者審閱、陌生人測試、煙霧矩陣（`docs/superpowers/runbooks/E007-release-smoke.md`）執行結果回填、版本號收斂後 06-15 切 `v0.6.0` tag。
- **待補（切 tag 前，硬性）** — 本稿快照 `4d4884ba` 之後 main 已續有合併落地（截至 06-11 同日稍晚至少 4 筆 first-parent：`115c135e` Apple 登入 pre-1970 時鐘優雅錯誤、`2057f50d` M1 swarm node-identity 機具、`5af6b42f` Gemini 空回應/安全攔截 200 改走 failover 而非斷流、`4300ad34` 桌面 app IPC 批次），且 06-15 前預期還會更多。切 tag 前須以 `git log --first-parent 4d4884ba..<tag>` 把增量補入 §4–§6 分桶與 §9 統計，否則「29/68」即為不誠實數字。
