# 文件設定稽核報告 / Doc-Setup Audit — 2026-06-19

> **唯讀（READ-ONLY）稽核**，涵蓋主 repo 加 10 個衛星專案。範圍：僅限文件設定
> （README / ROADMAP / docs/INDEX / docs/_archive / 狀態去重 / 連結完整性 / 計數漂移（drift）/
> LICENSE+CONTRIBUTING / 雙語 / 暫存檔）。本次稽核未修改任何程式碼或文件；本檔案為
> **未追蹤（untracked）／未提交（not committed）**。
>
> 背景脈絡：一次 KB 整併（consolidation）剛落地於這 10 個衛星專案（每個專案現在應具備 ROADMAP.md =
> 狀態唯一真實來源（SSOT）、docs/INDEX.md = 單一導覽、含 tombstone 的 docs/_archive/、以及一份連結至
> ROADMAP 而本身不含狀態清單的 README）。主 repo 先前即已完成整併。數個衛星專案正在進行中，正陸續加入
> `ROADMAP.zh-TW.md` 與 `docs/OSS-LANDSCAPE-AND-DIRECTION.md` —— **缺少這兩者並非缺陷。**

---

## 摘要矩陣

圖例：✅ 良好 · ⚠️ 輕微／進行中 · ❌ 缺失或損壞 · README-clean = README 將狀態交由 ROADMAP（不重複列出狀態清單）。

| Project | README | ROADMAP | docs/INDEX | _archive | README-clean | Issues |
|---|---|---|---|---|---|---|
| spectyn-mesh (main) | ✅ | n/a (uses EXECUTION-PLAN/status.md) | ✅ (+zh-TW, CI doc-lint) | ✅ | ✅ | 0 (已整併) |
| spectyn-companion | ✅ | ✅ | ✅ | ✅ | ✅ | 1 |
| spectyn-finance | ✅ | ✅ | ✅ | ✅ | ✅ | 2 |
| spectyn-training | ✅ | ✅ | ✅ | ✅ | ✅ | 1 |
| spectyn-quant | ✅ | ✅ | ✅ | ✅ | ✅ | 2 |
| spectyn-enterprise | ✅ | ✅ | ✅ | ✅ | ✅ | 1 |
| spectyn-ai-feed | ✅ | ✅ | ✅ | ✅ | ✅ | 2 |
| spectyn-flow | ✅ | ✅ | ✅ | ✅ | ✅ | 2 |
| spectyn-secops | ✅ | ✅ | ✅ | ✅ | ✅ | 3 |
| spectyn-secure-connector | ✅ | ✅ | ✅ | ✅ | ✅ | 3 |
| spectyn-tutor | ✅ | ✅ | ✅ | ✅ | ✅ | 3 |

**重點摘要：** 此次整併紮實。每個衛星專案都正確接好了 README→ROADMAP(SSOT)→INDEX→_archive
的模式。各處皆**未發現損壞的 INDEX 連結、未發現放錯位置的日期化開發紀錄（dev-log），也未發現顯著的雙語
漂移（drift）**。剩餘問題都很小：全艦隊普遍缺少 `CONTRIBUTING.md`、一個缺少的
`LICENSE`（tutor）加上一個已宣告卻不存在的 LICENSE（quant）、幾個陳舊計數、幾個 orphan／未追蹤的
已完成文件，以及一些散落的暫存檔。

**完全乾淨（0 個可處理問題）：1 / 11**（僅主 repo）。
**需要修正：10 / 11** —— 但全部都是低風險的收尾打磨；沒有任何一項會破壞導覽。

---

## 各專案稽核結果

### spectyn-mesh (main) — 乾淨
已完成整併：`docs/INDEX.md` + `docs/INDEX.zh-TW.md` 雙語導覽、治理金字塔（governance pyramid）、CI
doc-lint（`scripts/check-doc-tree.ps1`）、含 tombstone README 的 `docs/_archive/`。`git status` 中眾多根目錄的
`TASK-*.md` 是本次工作階段的指揮（conductor）暫存（父 repo），不在衛星模式的範圍內。
無需處理。

### spectyn-companion — 1 個問題
- ❌ **根目錄缺少 `CONTRIBUTING.md`**。
- ⚠️（進行中）`ROADMAP.zh-TW.md` + `docs/OSS-LANDSCAPE-AND-DIRECTION.md` 為**未追蹤**；一旦提交，請加入 INDEX 列。
- 已驗證乾淨：README 將狀態交由 ROADMAP（`README.md:36-38`）；「5 insight modules」與 `insight_modules/` 相符（剛好 5 個）；無損壞連結；tombstone 正確；zh-TW 長度相當，且自我宣告非 SSOT。
- 可選打磨：README quickstart 漏列已出貨的子指令 `anomaly-alerts` / `ingest-output` / `ingest-health`（低估了功能）。

### spectyn-finance — 2 個問題
- ❌ **缺少 `CONTRIBUTING.md`**。
- ⚠️ **repo 根目錄有 4 個散落的 `TASK-*.md` 暫存檔**（未追蹤），應加以封存／移動或 gitignore：
  `TASK-finance-account-cli.md`、`TASK-finance-recurring-persist.md`、`TASK-networth-cli.z13.md`、
  `TASK-price-hike-report-event.z13.md`。account-cli / networth-cli 描述的工作在 ROADMAP 中已標示為出貨 → 陳舊。
- 外觀問題：`docs/INDEX.md:12` 寫著 `_archive/` 是「Empty today」，但其現已含有一個 `README.md` tombstone。
- 已驗證乾淨：README→ROADMAP（`README.md:12,22-25`）；ROADMAP 含 commit hash；所有 `cli.py` 指令皆有文件；無計數漂移；zh-TW 交由英文版本。

### spectyn-training — 1 個問題
- ❌ **缺少 `CONTRIBUTING.md`** —— 唯一缺口。
- 已驗證乾淨：README→ROADMAP（`README.md:43-46`）；「7 stdlib-only modules」（`README.md:108`）與原始碼完全相符；根目錄的 `DESIGN.md` 是刻意保留的正式（canonical）文件（正確地未被封存），且由 INDEX 連結；`docs/OSS-LANDSCAPE-AND-DIRECTION.md` 已存在。

### spectyn-quant — 2 個問題
- ❌ **缺少 `LICENSE`**，儘管 `pyproject.toml:10` 宣告了 `license = { text = "Apache-2.0" }` —— 已宣告卻不存在的 license 檔案是優先缺口。
- ❌ **缺少 `CONTRIBUTING.md`**。
- ⚠️（進行中）`ROADMAP.zh-TW.md` + `docs/OSS-LANDSCAPE-AND-DIRECTION.md` 未追蹤；落地後請在 INDEX 加入 zh-TW 列。
- 已驗證乾淨：README→ROADMAP 去重 + 連結；INDEX 連結皆可解析；計數相符（3 個 CLI 子指令 `backtest/paper/import-csv`、1 個策略 `sma_cross`）；zh-TW 狀態行與英文相符，無漂移。

### spectyn-enterprise — 1 個問題
- ❌ **缺少 `CONTRIBUTING.md`** —— 唯一缺口（LICENSE 已存在）。
- 已驗證乾淨：README 在 5 處將狀態交由他處；`ask`/`status` 與 `code_qa/cli.py` 相符；**所有 INDEX 連結皆可解析**；疑似暫存的檔案（`ldap-activation-spec.md`、`saml-oidc-spec.md`、`apple-silicon-ha-deploy.md`、`vpn-mesh-demo.md`、`05-spectyn-enterprise.md`）全部合法、非陳舊、且由 INDEX 正確連結；7-connector 清單與文件相符（mes/erp 確認為 0-LOC 佔位（placeholder））。

### spectyn-ai-feed — 2 個問題
- ❌ **缺少 `CONTRIBUTING.md`**（LICENSE 已存在）。
- ⚠️ **`docs/AI-SOURCES-CURATED.md` 既未追蹤又為 orphan** —— 未由 `docs/INDEX.md` 連結。請選擇追蹤 + 加入 INDEX 列，或移除。（內容與 `feeds.toml` 不衝突 —— 它是更廣泛的人類參考用策展。）
- 已驗證乾淨（注意 —— 已更正一個子代理（sub-agent）的誤報）：`sources/feeds.toml` 中的 feed 計數為 **14**（第 8 行的某個 `[[feed]]` token 是註解），這與 `README.md:75`（「14 RSS sources」）及 `ROADMAP.md:27`（「14 feeds」）**相符** —— **無 feed 計數漂移。** README→ROADMAP 紀律良好；所有 INDEX 連結皆可解析；所有有文件的 CLI 指令都是真實的 argparse 進入點；無 zh-TW 檔案（無雙語漂移）。（回報的 103→104 測試計數差異至多是 ±1 的外觀問題；不具關鍵性。）

### spectyn-flow — 2 個問題
- ❌ **缺少 `CONTRIBUTING.md`**（只有 vendored 的 subtree 才有一份）。
- ⚠️ **`docs/demo.cast` 為 orphan** 的 asciinema cast —— 未被任何文件引用／不在 INDEX 中。請接入、移動或移除。
- ⚠️（進行中）`ROADMAP.zh-TW.md` + `docs/OSS-LANDSCAPE-AND-DIRECTION.md` 未追蹤。
- 外觀問題：根目錄的 `spectyn_flow.egg-info/` 是建置產物（gitignore 候選）。
- 已驗證乾淨：根目錄的 `DESIGN.md` 是一份在用、被引用的設計文件（正確地未被封存）；INDEX 連結皆可解析；無指令漂移。

### spectyn-secops — 3 個問題
- ❌ **缺少 `CONTRIBUTING.md`**。
- ⚠️ **`ROADMAP.zh-TW.md` 未追蹤** —— 已完成的雙語伴隨文件，應予提交。
- ⚠️ **`docs/OSS-LANDSCAPE-AND-DIRECTION.md` 既未追蹤又不在 `docs/INDEX.md` 中** —— orphan；請提交 + 加入 INDEX 列。
- 外觀問題：`docs/L2-INTEGRATION-PLAN.md` 標頭寫著「target completion 5/14 evening」（看起來陳舊的日期；結構上沒問題，已正確地作為設計參考被索引）。
- 已驗證乾淨：README→ROADMAP；封存整潔度良好（COMPLETION-PLAN / L2-TODO / STATUS 全在 `_archive/`）；無指令／模組漂移。

### spectyn-secure-connector — 3 個問題
- ❌ **缺少 `CONTRIBUTING.md`**（LICENSE 已存在，Apache-2.0）。
- ⚠️ **陳舊的測試計數：文件寫 112，實際 113。** `ROADMAP.md:9`（「112 tests passing in CI」）與 `ROADMAP.zh-TW.md:38` —— 已在追蹤的測試檔（`compliance_checker/tests`、`mcp_bridge/tests`、`phi_redactor/tests`、`secops_simulator/tests`）中驗證共有 113 個 `def test_` 函式。
- ⚠️ **repo 根目錄有 2 個未追蹤的暫存任務筆記**（未 gitignore）：`TASK-mcp-real-tools.z13.md`、`TASK-mcp-inbound-injection-gate.z13.md` —— 請封存或忽略。
- 已驗證乾淨：README→ROADMAP；INDEX 連結皆可解析；封存 tombstone 正確。

### spectyn-tutor — 3 個問題
- ❌ **缺少 `LICENSE`** —— 在 AGPL 父專案之下完全沒有 license 檔案。是全艦隊中最嚴重的單一缺口。
- ❌ **缺少 `CONTRIBUTING.md`**。
- ⚠️ **`personalized/` 未追蹤暫存目錄不在 `.gitignore` 中** —— 內容標示為「不要 commit」，卻依賴於永不被 `git add` → 有意外提交的風險。請加入 `.gitignore`。
- 注意：`docs/2026-06-18-spectyn-tutor-design.md` 雖有日期，但它是由 README + INDEX 引用的**在用設計 SSOT** —— **正確地未被封存；請勿將其移至 _archive。**
- 外觀問題：某則 commit 訊息聲稱知識「2→20」；ROADMAP/原始碼則為 19（以 ROADMAP 為準）。

---

## 建議先修 / Prioritized "fix first"（最高價值、最低風險者優先）

1. **spectyn-tutor：加入 `LICENSE`**（AGPL，以與父專案一致）。法律／OSS 曝險最高；零風險；約 1 分鐘。
2. **spectyn-quant：加入 `LICENSE`** —— `pyproject.toml` 已宣告 Apache-2.0 但檔案不存在。已宣告卻缺失的 license 是真實的封裝／法律缺口；放入標準的 Apache-2.0 文字即可。
3. **spectyn-tutor：將 `personalized/` 加入 `.gitignore`** —— 防止意外提交明確標示「不要 commit」的個人資料。極小、純安全性。
4. **全艦隊：為所有 10 個衛星專案加入一份 `CONTRIBUTING.md`**（目前每個都缺）。使用單一統一範本；這是全艦隊唯一一致的缺口。批次處理。
5. **spectyn-secops + spectyn-quant + spectyn-companion + spectyn-flow：提交已完成的 `ROADMAP.zh-TW.md` / `docs/OSS-LANDSCAPE-AND-DIRECTION.md` 並加入其 INDEX 列**（並修正 secops/ai-feed 的 `OSS-LANDSCAPE` orphan 連結）。乾淨地收尾進行中的雙語／landscape 工作。

次要整理（便宜，可一併處理）：修正 `spectyn-secure-connector` 測試計數 112→113
（`ROADMAP.md:9`、`ROADMAP.zh-TW.md:38`）；移動／忽略 finance 與 secure-connector 根目錄散落的
`TASK-*.md` 暫存；處理 orphan 資產（`spectyn-flow/docs/demo.cast`、`spectyn-ai-feed/docs/AI-SOURCES-CURATED.md`）；
更正 `spectyn-finance/docs/INDEX.md:12` 的「_archive Empty today」。

---
*Generated read-only 2026-06-19. File intentionally left untracked.*
