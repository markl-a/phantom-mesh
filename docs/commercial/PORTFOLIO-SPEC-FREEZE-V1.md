# 作品集 Repo 規格凍結 v1（Portfolio Repo Spec Freeze v1）

**生效期間（Effective）**：2026-05-01 → 2026-05-15（OSS 開源發佈）
**狀態（Status）**：🟡 草稿（DRAFT）— 依 2026-05-01 與使用者逐步走查（walk-through）後鎖定；
一旦 5/8 的彩排（dress rehearsal）確認每個 repo（程式碼倉庫）的
demo（示範）路徑能即時運行，即升級為 FROZEN（凍結）。

本文件記錄與 spectyn-mesh 一同隨 5/9 面試作品集 + 5/15 OSS 開源發佈出貨的
**5 個衛星 repo（satellite repos，圍繞主專案的附屬倉庫）** 的凍結合約（freeze contract）。
每一節都刻意保持簡短（10–20 行）：
深入內容存放於各 repo 自己的 README + INTERVIEW-TALK-TRACK.md。

---

## 如何閱讀本文件

每一節針對凍結期間回答四個問題：

1. **5/9 demo 角色** — 面試官在 14 分鐘流程中看到什麼
2. **5/15 OSS 開源發佈狀態** — 公開可見性（public visibility）+ 授權條款（license）+ 所需的最後一次
   commit（提交）
3. **5/1 → 5/15 期間凍結** — 沒有例外 PR（pull request，合併請求）就絕對
   不得變更的路徑／檔案
4. **spectyn-mesh 依賴關係** — 它是否呼叫／執行／嵌入 spectyn-mesh，
   或只是並列的姊妹 demo（sibling demo）

外加一行 **狀態（5/1）**（附驗證命令），以及若有任何未解決事項時的
**風險（risk）** 一行。

---

## Repo 1：spectyn-secops 🔒（Trend Micro 主攻方向）

- **5/9 demo 角色**：demo 流程的第 3 幕（Act 3，3 分鐘）。從
  Mac TerminalShell（終端機外殼）shell out（外殼呼叫）：`cd ~/.../spectyn-secops && make demo-mock`。
  1 秒內 → `reports/runs/<ts>/` 產生 8 個產出物（artefacts）（incident-report.md +
  pentest-report.md + 6 個結構化 JSON）。打開 `incident-report.md`，
  展示 red-4（紅隊 4 agent）+ blue-4（藍隊 4 agent）流水線輸出，順帶提及 ETHICS.md。
- **5/15 OSS 開源發佈狀態**：5/8 從 PRIVATE（私有）翻為公開；
  MIT 授權；釘選（pinned）於 github.com/markl-a 個人檔案頁。
- **5/1 → 5/15 期間凍結**：
  - Red 4 agents（recon → vuln-scan → exploit-suggest → pentest-report）
  - Blue 4 agents（log-anomaly → alert-triage → threat-correlate → incident-report）
  - 模擬資料（Mock data）：`lab/mocks/{recon-juice-shop.json, vuln-scan-juice-shop.json, attack-log.txt}`
  - `ETHICS.md`（面試的安全網）
  - `docs/INTERVIEW-TALK-TRACK.md`
  - `make demo-mock` / `make test` / `make lint` 目標（targets）
- **spectyn-mesh 依賴關係**：**無（NONE）**（姊妹 demo，不是消費者）。
  兩個專案共用 AI-agent 流水線模式；spectyn-mesh 是
  執行時環境（runtime），spectyn-secops 是應用程式。它們在 demo 中並排運行，
  而非疊成一個技術棧（stack）。
- **狀態（5/1）**：✅ commit `51220f8`，`make demo-mock` 綠燈，
  7/7 測試通過，`make lint` 乾淨。
- **風險**：🔴 commit `3abf406` + `0d5c714` 在 git 歷史中夾帶真實 API 金鑰（API keys）。
  **公開翻轉前（5/8）務必執行 `git filter-repo`**，
  否則唯一的補救方式就是在供應商端撤銷公開金鑰 + filter-repo + force-push（強制推送）。

### 1.x 檔案樹 + 執行流程深入剖析（5/1 新增）

**倉庫結構（Repository layout）**：
```
spectyn-secops/
├─ Makefile                    # 6 targets: demo / demo-mock / lab-up / lab-down / test / lint
├─ docker-compose.yml          # OWASP Juice Shop + DVWA + Metasploitable + Kali (live mode only)
├─ ETHICS.md                   # 80-line legal/scope doc — interview safety net
├─ LICENSE                     # MIT
├─ README.md                   # public face
├─ requirements-dev.txt        # pytest, ruff, mypy
│
├─ agents/                     # 8 spectyn-mesh TOML agent configs (NOT Python)
│  ├─ red/
│  │  ├─ recon.toml            # nmap_runner + http_probe + dns_enum + file_write
│  │  ├─ vuln-scan.toml        # nuclei_runner + nmap_runner + file_write
│  │  ├─ exploit-suggest.toml  # text-only LLM, no shell
│  │  └─ pentest-report.toml   # text-only LLM, markdown writer
│  └─ blue/
│     ├─ log-anomaly.toml      # log_ingest + file_write
│     ├─ alert-triage.toml     # text-only triage + dedup
│     ├─ threat-correlate.toml # kill-chain reconstruction
│     └─ incident-report.toml  # text-only markdown writer
│
├─ scenarios/
│  ├─ run_kill_chain.py        # orchestrator (mock + live)
│  └─ full-kill-chain.md       # narrative version of the pipeline
│
├─ tools/                      # 3 wrappers callable by agents
│  ├─ nmap_runner.py
│  ├─ nuclei_runner.py
│  └─ log_ingest.py
│
├─ lab/
│  └─ mocks/                   # 3 canned datasets used by --mock
│     ├─ recon-juice-shop.json     # nmap output, 1 open port (3000), Express+AngularJS+jQuery
│     ├─ vuln-scan-juice-shop.json # nuclei findings (parsed JSON)
│     └─ attack-log.txt            # synthetic Apache + sshd + audit logs covering kill chain
│
├─ tests/                      # 2 files, 7 test functions
│  ├─ test_log_anomaly.py
│  └─ test_nmap_runner.py
│
├─ scripts/lint.py             # custom lint runner (TOML + Python syntax + JSON validity)
├─ docs/
│  ├─ ARCHITECTURE.md          # layered ASCII diagram + design rationale
│  └─ INTERVIEW-TALK-TRACK.md  # 30-sec elevator + 5 likely Qs + answers
└─ reports/runs/<YYYY-MM-DD-HHMM>/  # 8 artefacts per run (see below)
```

**`make demo-mock` 端對端追蹤（end-to-end trace）**（1.0 秒實際耗時（wall-clock）；具決定性（deterministic））：

1. `make demo-mock` → `python3 scenarios/run_kill_chain.py --target juice-shop --mock`
2. 腳本建立 `reports/runs/<UTC-timestamp>/` 並把 `REPO_ROOT` 插入 `sys.path`
3. **紅隊流水線（Red pipeline）**（mock 模式下不用 LLM — 使用模板（templates））：
   - `red-recon`：讀取 `lab/mocks/recon-juice-shop.json` → 產出 `recon.json`（1 個開放埠、3 個端點、0 個子網域）
   - `red-vuln-scan`：讀取 `lab/mocks/vuln-scan-juice-shop.json` → 產出 `vuln-scan.json`（5 項發現）
   - `red-exploit-suggest`：從 vuln-scan.json 合成 POC（概念驗證）文字 → `exploit-suggestions.md`
   - `red-pentest-report`：合併所有紅隊輸出 → `pentest-report.md`
4. **藍隊流水線（Blue pipeline）**（可平行化，但 mock 模式為循序執行）：
   - `blue-log-anomaly`：讀取 `lab/mocks/attack-log.txt` → 產出 `alerts.jsonl`（21 筆原始告警）
   - `blue-alert-triage`：分類 + 去重 → `triage-queue.jsonl`（分為 5 組：2 P1 / 2 P2 / 1 P3）
   - `blue-threat-correlate`：從分類結果重建 kill-chain（攻擊鏈）→ `kill-chains.jsonl`（1 名攻擊者）
   - `blue-incident-report`：合併所有藍隊輸出 → `incident-report.md`
5. 輸出：`reports/runs/<ts>/` 中 8 個產出物。合計：約 28 KB markdown + 約 12 KB JSONL。

**7 個測試**（橫跨 2 個檔案，依 `make test` 全數通過）：
- `test_nmap_runner.py`：parse_nmap_output / handle_no_hosts / closed_port_filtering / unknown_service_field
- `test_log_anomaly.py`：detect_brute_force / detect_sqli_pattern / dedup_within_window

**實機模式邊界（Live mode boundary）**（不在 5/9 demo 內）：
- `make demo`（無 --mock）需先執行 `make lab-up` → `docker compose up` 啟動 4 個容器
- `--use-llm` 旗標會呼叫位於 `localhost:7878` 的 spectyn-mesh `spectyn serve` 來進行 LLM 驅動的報告撰寫
- 5/9 demo 我們**一律**執行 `make demo-mock`。實機模式只是面試問答時的素材。

**刻意粗糙／未建置（Intentionally crude / not built）**：
- mock 中的 LLM 模式 = 帶有決定性替換的模板（無實際 LLM 呼叫）。快速、可重複、demo 不需 API 金鑰。
- `dns_enum` 工具在 recon.toml 中有宣告，但在 mock 模式永遠回傳 `[]`（實驗室沒有真實 DNS）
- Alert-triage 使用啟發式正則（heuristic regex），不是 ML 分類器 — 刻意如此以利於檢視友善的 demo
- 各次執行之間無持久化（persistence）— `reports/runs/` 目錄會累積，但屬唯讀產出物

**面試問答整合（Interview Q&A integration）**（`docs/INTERVIEW-TALK-TRACK.md`）：
- 30 秒電梯簡報（elevator pitch）已備妥
- 預先備答：「這合法嗎？」/「這跟 <廠商 X> 有何不同？」/「LLM 實際貢獻了什麼？」/「為何用 TOML 做 agent 設定？」/「這能擴展嗎？」
- 每個問題都有預寫好的 3 段式答覆

---

## Repo 2：spectyn-mobile 📱（Google Pixel 登月計畫）

- **5/9 demo 角色**：demo 流程的第 4 幕（Act 4，1 分鐘）。從
  Mac shell out：`cd ~/.../spectyn-mobile && make demo-mock`。1 秒內 → 60 格
  矩陣（12 情境 × 5 裝置）。結果：48 通過 / 8 警告 / 4 失敗
  = 80%。展示一個具體的失敗敘事（例如「Pixel Fold 上的
  RTL email 對齊問題」）。
- **5/15 OSS 開源發佈狀態**：5/8 從 PRIVATE 翻為公開；
  MIT 授權；釘選。
- **5/1 → 5/15 期間凍結**：
  - 流水線：Generator → Explorer × Verifier（每格）→ Reporter
  - 5 個模擬模組：network / battery / locale / accessibility / lifecycle
  - `lab/emulator-matrix.yml`（60 格設定）
  - `docs/{ARCHITECTURE.md, SIMULATION-ENGINE.md, INTERVIEW-TALK-TRACK.md}`
  - `make demo-mock`（mock 驅動；`make demo` 用於實機 ADB，屬選用
    且不在 5/9 demo 內）
- **spectyn-mesh 依賴關係**：**無（NONE）**（姊妹 demo，與 #1 相同）。
  在 v0.2 可以接上，跨 mesh 分派裝置測試格
  （例如 ROG Phone 跑 `lifecycle.py`、Mipad 跑 `accessibility.py`），
  但**不**在 5/9 範圍內。
- **狀態（5/1）**：✅ commit `eeb2db2` + README 潤飾 `85ad7b4`，
  `make demo-mock` 綠燈，14/14 測試，`make lint` 乾淨（PyYAML 深層
  解析）。
- **風險**：實機模式（真實 Android SDK + uiautomator2 + AVD）尚
  未測試。若面試官要求「給我看實機跑一次」，誠實的
  退路是：「實機模式已接好但本衝刺（sprint）未實際操作；
  mock 模式是設計的主要呈現面。」

### 2.x 檔案樹 + 執行流程深入剖析（5/1 新增）

**倉庫結構（Repository layout）**：
```
spectyn-mobile/
├─ Makefile                   # 6 targets: demo / demo-mock / emulator-up/down/status / test / lint
├─ LICENSE                    # MIT
├─ README.md                  # public face
├─ requirements-dev.txt       # pytest, pyyaml
│
├─ agents/                    # 4 spectyn-mesh TOML agent configs
│  ├─ generator.toml          # user-story → 8-15 scenario variants (locale/a11y/network/lifecycle/permission/low-resource)
│  ├─ explorer.toml           # walk one (scenario × device) cell, capture screenshots + step transcript
│  ├─ verifier.toml           # judge cell pass/warn/fail with reasoning
│  └─ reporter.toml           # aggregate 60 cell results → matrix-report.md
│
├─ scenarios/
│  └─ run_matrix.py           # orchestrator (mock + live)
│
├─ simulation/                # 5 Python modules — one per failure axis
│  ├─ network.py              # 3g/edge/loss5/high_latency/offline_mid_flow profiles via tc + iptables
│  ├─ battery.py              # set battery level + thermal state via dumpsys
│  ├─ locale.py               # ar-EG, ja-JP, zh-TW, font scale, dark mode, TalkBack
│  ├─ accessibility.py        # font scale 130/175%, TalkBack focus order, contrast
│  └─ lifecycle.py            # background → foreground, rotation, incoming-call sim
│
├─ android/
│  ├─ emulator-matrix.yml     # 5 device configs (Pixel 8 Pro / Pixel 6 / Fold / Tablet / Nexus 5X)
│  └─ start-matrix.sh         # boots all 5 emulators in parallel
│
├─ tests/
│  ├─ test_runner.py          # orchestration tests
│  ├─ test_simulation.py      # simulation module unit tests (14 total)
│  ├─ mocks/                  # canned cell-result data for --mock
│  └─ signup-flow.story.md    # the demo's user-story input
│
├─ scripts/lint.py            # custom lint
├─ tools/                     # callable by agents (limited set for mobile)
└─ docs/
   ├─ ARCHITECTURE.md
   ├─ SIMULATION-ENGINE.md
   └─ INTERVIEW-TALK-TRACK.md

reports/runs/<YYYY-MM-DD-HHMM>/  # 3 artefacts per run
```

**`make demo-mock` 端對端追蹤**（1.0 秒實際耗時；具決定性的 60 格）：

1. `make demo-mock` → `python3 scenarios/run_matrix.py --story tests/signup-flow.story.md --mock`
2. **Generator** 讀取 `signup-flow.story.md`（1 行使用者目標：「使用者用 email + 密碼註冊」）→ 產出 12 個情境為 `scenarios.json`。涵蓋軸向：快樂路徑（happy path，1）、語系（locale，3：RTL ar-EG、CJK ja-JP、字元密集 zh-TW）、無障礙（accessibility，3：字型 130%/175%、TalkBack）、網路（network，3：3g、封包遺失 5%、流程中途離線）、權限拒絕（permission denials，1）、低資源（low-resource，1）。
3. **矩陣展開（Matrix expansion）**：12 情境 × 5 裝置（Pixel 8 Pro/6/Fold/Tablet/Nexus 5X）= 60 格。
4. **Explorer × Verifier（每格，可平行，mock 模式為循序）**：
   - Mock 模式：每格從 `tests/mocks/<scenario_id>-<device_id>.json` 讀取罐裝追蹤（canned trace）
   - 實機模式：explorer 透過 uiautomator2 驅動模擬器；verifier 依故事預期判斷輸出
   - 輸出：`cell-results.jsonl`（60 行，每行為 `{scenario, device, status: pass|warn|fail, reasoning, first_failing_step?}`）
5. **Reporter** 彙整 → `matrix-report.md`：
   - 執行摘要（Executive summary）：48/60 通過 / 8/60 警告 / 4/60 失敗
   - 主要失敗表（4 個具體 bug 敘事 — 這些是 demo 的精華）
   - 依模擬軸向劃分的失敗：network 10/2/3、battery 5/0/0、locale 10/5/0、a11y 4/0/1
6. 輸出：`reports/runs/<ts>/` 中 3 個產出物：`cell-results.jsonl`、`matrix-report.md`、`scenarios.json`。

**4 個 demo 失敗敘事**（5/9 demo 精華）：

| 情境 | 裝置 | 失敗步驟 | Bug 敘事 |
|---|---|---|---|
| 字型縮放 175% | Nexus 5X · API 28 | 填入 email + 密碼 | Email 標籤與輸入框碰撞；兩者皆被裁切 |
| 步驟 4 中途離線 | Pixel 8 Pro · API 35 | 應看到錯誤或重試 | UI 凍結 30 秒以上，未浮現任何錯誤/重試 |
| 步驟 4 中途離線 | Pixel 6 · API 34 | 同上 | 相同的退化問題 |
| 步驟 4 中途離線 | Pixel Fold · API 34 | 同上 | 相同的退化問題 |

→ **Demo 講稿（talk track）**：「矩陣浮現出一個清楚的模式：在 3 款最新的 Pixel 裝置上，流程中途離線都未被處理，而較舊的 Nexus 5X 則有不同的失敗模式（無障礙版面被裁切）。兩者都是生產團隊會出貨卻漏掉的真實 bug 類別。」

**14 個測試**（橫跨 2 個檔案，依 `make test` 全數通過）：
- `test_runner.py`：情境展開、格排程、mock 資料解析、報告生成
- `test_simulation.py`：5 個模擬模組各有 2 個單元測試 + 邊界案例

**實機模式邊界**（不在 5/9 demo 內）：
- `make demo`（無 --mock）需先執行 `make emulator-up` → `bash android/start-matrix.sh` 啟動 5 個模擬器
- 真實的 `simulation/*.py` 透過 `adb shell` + `tc qdisc` + `dumpsys` 觸及模擬器
- 14 分鐘 demo 不跑實機模式

**刻意粗糙／未建置**：
- Mock 格追蹤是合成的 — 沒有真實截圖
- Generator 是單次提示（single-shot prompt），不是多輪 agent（可以更豐富）
- 5/9 demo 中沒有實際的受測 app — 故事檔描述的是通用的註冊流程
- 真實裝置支援已記錄於 SIMULATION-ENGINE.md，但使用 mitmproxy 代理而非本地 tc

**面試問答整合**（`docs/INTERVIEW-TALK-TRACK.md`）：
- 30 秒電梯簡報：「60 格裝置矩陣浮現出 4 種會出貨到生產環境的不同 bug 類別」
- 預先備答：「為何用 mock 模式？」/「真實裝置流水線？」/「這如何擴展到 1000 台裝置？」/「為何 generator 產出 12 個而非 N 個？」/「agent 能共享情境狀態嗎？」

---

## Repo 3：My-AI-Learning-Notes 📚（持續學習的佐證）

- **5/9 demo 角色**：僅作品集提及 — 釘選於
  `github.com/markl-a` 頁面的 repo；不在實機 demo 中打開。
  點進去的面試官會看到中文 AI 工程師學習路徑，
  其中第 9 章（「面試準備與職業發展」）將 spectyn-mesh 整合為
  案例研究（case study）。
- **5/15 OSS 開源發佈狀態**：已公開，17⭐，無需變更。
- **5/1 → 5/15 期間凍結**：**整個 repo**。不要編輯任何檔案。
  platform/macos session 上的 pre-commit hook（提交前掛鉤）會阻擋對
  `spectyn-mesh` 以外 repo 的意外寫入（已透過位於
  不同工作目錄 + 不同 git remote 而強制執行）。
- **spectyn-mesh 依賴關係**：**無（NONE）**（這是關於學習的
  文件，不被 spectyn-mesh 消費）。
- **狀態（5/1）**：✅ commit `3d26b3b`（spectyn hook 章節於
  2026-04-29 落地）。17⭐ 基準。README 的「持續學習」角度在面試上
  表現良好。
- **風險**：零。這是唯讀素材。

### 3.x 倉庫內容地圖（5/1 新增）

**頂層章節結構**（深度 2-3，從遠端 fetch）：

```
1.從AI到LLM基礎/
  ├─ 1.Machine_Learning數學基礎/   data structures, linear algebra, probability
  ├─ 2.AI_Intro/                  Python basics, env setup, Colab tutorials
  ├─ 3.ML_&_Data_Analysis/         feature engineering, ML algorithms, evaluation
  ├─ 4.DL/                        TensorFlow, Keras, PyTorch, YOLO, SAM2
  └─ 5.Best_Practices_and_MLOps/   ETL, MLflow, monitoring

2.深入LLM模型工程與LLM運維/
  ├─ GaLore_Demo/                 memory-efficient training, medical chat
  ├─ Quantization/                GGUF, AWQ, fp8 implementations
  └─ LLM_Inference_Optimization/   vLLM, Ollama, TensorRT-LLM

3.LLM應用工程/
  ├─ 1.LangchainDemos/            chains, agents, LangGraph, LangSmith
  ├─ 2.LLM_API_Calls/             OpenAI, Claude, Gemini integrations
  ├─ 3.RAG_&_Vector_DB/            embedding, retrieval, reranking
  └─ 4.Agent_&_Workflows/          multi-step reasoning, tool use

4.相關的更新Blog/                 2024-2025 trend posts, case studies
5.AI研究前沿_2024-2025/           50+ papers with code, frontier algorithms
6.DeepLearning.ai短課程/          official course transcripts and impl
9.面試準備與職業發展/              ⭐ spectyn-mesh 開發案例 (見下)

docs/, benchmarks/, exercises/    supporting + benchmarks + practice
```

**第 9 章 — 面試案例研究整合**（commit `3d26b3b` 讓
該章節落地；出自 agent 的稽核）：

該章節明確引用 spectyn-mesh 開發情境，作為
真實世界的深度講稿：

- **多 agent 協調（Multi-agent coordination）** — 供應商故障轉移（provider fallback）、成本追蹤
- **串流 SSE 解析（Streaming SSE parsing）** — 處理即時串流回應
- **跨平台建置鏈（Cross-platform build chain）** — Mac/Linux/Windows/Android/iOS CI-CD

這些是針對面試「深度技術問題」環節的預備答覆。
每一項都對應到面試官可驗證的實際 spectyn-mesh git commit。

**Spectyn-mesh hook 章節**（已驗證，README 開頭）：
```markdown
## 🔗 spectyn-mesh 生態系（知識庫 + 面試訓練）

本知識庫同時是 [spectyn-mesh](https://github.com/markl-a/spectyn-mesh) 生態系的
**學習路徑與面試準備教材**。spectyn-mesh 是自託管多 agent AI runtime，跨
Mac / Linux / Windows / Android / iOS 五平台。
```

連結目標已驗證：`github.com/markl-a/spectyn-mesh`（badge + inline 連結皆存在）

**面試深入探討的推薦章節**（當面試官問
「給我看其中一個的細節」時）：

1. **Ch.1-§4 `4.DL`** — PyTorch 張量運算、分散式訓練（DDP/FSDP）、`torch.compile`
2. **Ch.2 `GaLore_Demo`** — 記憶體高效訓練 vs LoRA/QLoRA 取捨
3. **Ch.3-§3 `RAG_&_Vector_DB`** — 生產級 RAG 模式、embedding 選型、reranking
4. **Ch.3-§4 `Agent_&_Workflows`** — 多 agent 協調、錯誤復原、成本最佳化
5. **Ch.9 面試準備與職業發展** — 已驗證的 spectyn-mesh 情境（後設迴圈，meta-loop）
6. **Ch.5 AI研究前沿** — 前沿論文（GaLore、SAM2、多模態 RAG）

---

## Repo 4：Automation_with_AI ⚙️（RPA + AIOps + MLOps）

- **5/9 demo 角色**：僅作品集提及。若面試官點進去，
  README 標題是「RPA + AIOps + MLOps 整併」；橫幅
  醒目標示它吸收了獨立 AIOps repo 的角色。
- **5/15 OSS 開源發佈狀態**：已公開；釘選；除 spectyn hook 章節
  外無需變更。
- **5/1 → 5/15 期間凍結**：**整個 repo**（與 Repo 3 相同姿態）。
  會很想再加更多潤飾；忍住 — 深度問題用「現有內容是這些」
  回答，會比「讓我給你看我昨天加了什麼」更好。
- **spectyn-mesh 依賴關係**：spectyn-mesh README 在
  生態系表格中引用此 repo（6 個衛星之一）。雙向連結，無
  程式碼耦合。
- **狀態（5/1）**：✅ commit `70bab0a`。
- **風險**：🟡 commit 歷史中可見一日傾倒（1-day-dump）的痕跡。對照核實（face-check）
  風險為中：一位資深開發者若看到其他 repo 整年的一致提交節奏，
  卻在此處看到單一批量傾倒，可能會注意到。誠實的答覆
  已備妥：「這是一個 AI 生成的框架起手包，整併了我用過的
  模式；`My-AI-Learning-Notes` 中更深入的程式碼才是
  手寫的學習素材。」不要過度推銷；引導回到
  Repo 1-3，那裡的工作可驗證。

### 4.x 倉庫結構 + 對照核實評估（5/1 新增）

**頂層結構**（出自 agent 的深入剖析）：

```
ai_automation_framework/   main package — 27-module core, real architectural depth
  ├─ core/                 27 modules: config, task queue, circuit breaker, DI
  ├─ llm/                  10 modules: OpenAI, Anthropic, Gemini, Ollama clients
  ├─ agents/               8 modules: BaseAgent, memory (SQLite/Redis), state
  ├─ rag/                  7 modules: retriever, embeddings, vector stores
  ├─ tools/                17 modules: email, web scraping, database, scheduler,
  │                        DevOps tools, cloud integration, webhooks
  ├─ workflows/            5 modules: Chain, Pipeline, DAG execution
  ├─ integrations/         15 modules: Zapier, n8n, Airflow, Kubernetes, etc.
  └─ plugins/              3 modules: custom provider support

examples/                  5-level progressive examples
  ├─ 1_basics/             LLM API calls, streaming, simple chat
  ├─ 2_intermediate/       RAG, function calling, chains
  ├─ 3_advanced/           multi-agent, autonomous, collaboration
  ├─ 4_automation/         email, web scraping, DB ops
  └─ 5_aidev/              code review, debug assistant, test generation

tests/                     20+ test files
docs/                      API docs, architecture guides, best practices
deployment/                Docker, Kubernetes, CI-CD configs
```

**Spectyn-mesh hook 章節**（已驗證，commit `70bab0a`）：

```markdown
## 🔗 spectyn-mesh ecosystem

This repository is the **applied automation + AIOps + MLOps layer** of the
[spectyn-mesh](https://github.com/markl-a/spectyn-mesh) ecosystem ...

In other words: spectyn-mesh is the **engine**; this repo is the **toolbox**
that engine reaches into when an automation task needs to do real work.
```

連結目標已驗證。

**對照核實稽核發現**（agent 去尋找「AI 批量傾倒」
訊號；以下是誠實的結果）：

🔴 **發現的紅旗（Red flags）**：

1. **單一近期 commit**（`70bab0a`）— 暗示批量初始化
2. **測試鷹架模式**：20+ 個測試檔，命名一致
   （`test_core.py`、`test_tools_x.py`），斷言（assertion）變化極少
3. **基於 print 的「測試」**：例如 `tests/test_tool_system.py` 用 `print()`
   來驗證而非 `pytest.assert*` — 業餘品質的氣味
4. **生產程式碼中的除錯 print**：`graphql_api.py`、
   `websocket_server.py` 夾帶未清除的 `print()` 陳述句（不在
   logging 框架內）
5. **註解密度**：僅有中文註解，複雜邏輯處稀疏

🟢 **找到可辯護的實質內容（Defensible substance）**：

1. **27 模組核心**，含 DI（依賴注入）、circuit breaker（斷路器）、task queue（任務佇列）— 真實的
   架構骨架，不是單檔傾倒
2. **17 個生產級工具類別**（email/db/sched/devops/cloud）—
   廣度超出單一 `pytest fixture` 所能偽造
3. **5 級漸進式範例**（基礎 → 自動化 → AI 開發）—
   刻意安排的教學結構
4. **全程使用 Pydantic 模型** — 適當的型別化結構（typed schemas），不是鬆散的 dict
5. **非同步核心（Async core）** — 非阻塞原語（primitives），不是玩具程式碼

**#4 的面試講稿**（備妥的答覆，出自 agent 的
建議）：

> 「這是我曾維護的 AIOps repo 的快速原型整併。其架構
> （27 模組核心、17 個工具類別）是刻意且真實的；部分測試
> 是為 MVP（最小可行產品）搭的鷹架 — 在任何生產使用前我會
> 手動重寫那些基於 print 的測試。如果你想看有可驗證歷史的
> 手寫程式碼，請看 spectyn-secops（Repo 1）或 My-AI-Learning-Notes（Repo 3）。」

這是**誠實且預先準備好的** — 面試官問「你是一天內寫完
這個的嗎？」答覆就是「鷹架是的，架構不是，這是驗證方式」。

---

## Repo 5：Data-Analysis-with-Chatbots 📊（資料科學層）

- **5/9 demo 角色**：僅作品集提及。README 的
  「資料科學分析層」訴求 + 分群（clustering）/ RFM / CLV 元件。
- **5/15 OSS 開源發佈狀態**：已公開；釘選；無變更。
- **5/1 → 5/15 期間凍結**：**整個 repo**，僅有一個小小
  例外 — 若 README 的 `examples/spectyn-telemetry/ (in progress)`
  說明文字**具誤導性**（面試官可能問「給我看那個進行中的
  部分」），那**一行** README 可透過 `[fix-docs]` commit 緩和為「規劃於 v0.2」。
- **spectyn-mesh 依賴關係**：規劃中（PLANNED）— `examples/spectyn-telemetry/`
  意在消費 spectyn-mesh agent 日誌以進行分群分析。
  **不在 5/9 範圍。不在 5/15 OSS 開源發佈範圍。** 屬 v0.2 工作項目。
- **狀態（5/1）**：✅ commit `eb1c60e`（spectyn hook 章節）。
- **風險**：🟡 README「in progress」那行是個陷阱。要嘛在 5/8
  前緩和（5 分鐘 PR），要嘛備妥答覆：「遙測（telemetry）
  消費是 v0.2 計畫；v0.1.0 出貨的是分群原語，
  而非 spectyn 整合。」

### 5.x 倉庫結構 — 發現的規模（5/1 新增）

**agent 的稽核揭露的內容遠多於記憶所記錄的**：
此 repo 除了分群 / RFM / CLV 元件之外，還承載著
**橫跨 17 個 ML 領域的 2000 個 Kaggle 解法**。對資料科學面試而言，
那是重大的可信度資產，但**同時**也是對照核實的責任
（若面試官鑽進某個解法卻發現它語無倫次，可信度就會反轉）。

**頂層結構**：

```
src/data_analysis_chatbots/  main package
  ├─ config_loader.py        YAML config
  ├─ data_loader.py          CSV / Kaggle dataset loaders
  ├─ data_downloader.py      Kaggle API auto-download
  ├─ preprocessing/          cleaning, normalization, feature extraction
  ├─ clustering/             K-Means, DBSCAN, GMM, Hierarchical impl
  ├─ visualization/          Plotly, Matplotlib dashboards
  └─ marketing/              RFM, CLV prediction, segmentation

kaggle_solutions/            ⭐ 2000 solutions across 17 ML domains
  ├─ 01_structured_data/        (112)  tabular ML, classification
  ├─ 02_time_series/            (128)  ARIMA, Prophet, LSTM
  ├─ 03_nlp/                    (112)  text classification, NER, sentiment
  ├─ 04_recommendation/         (116)  collaborative + content-based filtering
  ├─ 05_computer_vision/        (110)  classification, detection
  ├─ 06_clustering/             (120)  K-Means variants, density-based
  ├─ 07_special_domains/        (125)  healthcare, finance, geo
  ├─ 08_deep_learning/          (125)  CNN, RNN, attention, transformers
  ├─ 09_audio_signal/           (120)  spectrogram, speech, music
  ├─ 10_anomaly_detection/      (119)  Isolation Forest, LOF, autoencoders
  ├─ 11_graph_networks/         (119)  GCN, GraphSAGE, knowledge graphs
  ├─ 12_geospatial/             (118)  geo clustering, mapping
  ├─ 13_feature_engineering/    (123)  selection, creation, scaling
  ├─ 14_ensemble_methods/       (123)  Random Forest, XGBoost, voting
  ├─ 15_bayesian_methods/       (118)  Bayesian opt + inference
  ├─ 16_optimization/           (118)  scheduling, resource allocation
  └─ 17_multimodal/             (111)  CLIP, multimodal fusion

notebooks/                   3 interactive Jupyter walkthrough notebooks
docs/                        5 markdown guides (cleaning, segmentation, RFM, marketing)
config/, data/, models/, outputs/, scripts/, tests/
```

**Spectyn-mesh hook 章節**（已驗證，commit `eb1c60e`，README 開頭）：

```markdown
## 🔗 spectyn-mesh ecosystem

This repository is the **data-science & analytics layer** of the
[spectyn-mesh](https://github.com/markl-a/spectyn-mesh) ecosystem ...

**Two roles in the ecosystem:**

1. **Stand-alone**: end-to-end customer analytics framework as documented
   below — clustering algorithms (K-Means, DBSCAN, GMM, Hierarchical), CLV
   prediction, RFM segmentation, AI-assisted interpretation.
2. **Spectyn telemetry analysis**: the same algorithms applied to
   spectyn-mesh's own agent execution logs — clustering 10K+ executions
   to surface prompt-failure patterns, provider-specific performance,
   and cost outliers. See `examples/spectyn-telemetry/` (in progress).
```

連結目標已驗證。

**「in progress」陷阱 — 確切提及處**：

| 檔案 | 行 | 文字 |
|---|---|---|
| `README.md` | **26** | `See examples/spectyn-telemetry/ (in progress).` |

僅提及一次。`examples/spectyn-telemetry/` 目錄目前
尚不存在於磁碟上（僅佔位）。

**5/8 前的兩條修正路徑**（擇一）：

**A) 建立目錄存根（stub）** — 建立 `examples/spectyn-telemetry/README.md`
（約 50 行）附具體路線圖：spectyn-mesh 日誌分群、成本
離群偵測、提示失敗模式挖掘。耗時 30 分鐘。
*優點*：把「進行中」的含糊揮手變成具體的 v0.2 計畫
*缺點*：又多一個要維護的檔案；過度承諾的風險

**B) 緩和 README 那行** — 在 `README.md:26` 把「in progress」→「planned for v0.2」。
5 分鐘 PR。*優點*：零新承諾。*缺點*：
略顯不夠企圖心。

**建議：A**（以清楚的 v0.2 範圍建立存根），因為 2000 個
Kaggle 解法的故事已證明「此人能大量出貨」— 一個
50 行、預先回答「你會如何分析 spectyn 日誌？」的
spectyn-telemetry README 是面試精華。

**#5 的面試講稿**（深度問題 —「你有 2000 個
Kaggle 解法，帶我走過其中一個」）：

> 「挑一個領域 — `06_clustering/` 有 120 個解法。全部
> 2000 個的共通模式是：每個解法都是結構相同（載入 → EDA → 預處理 → 建模 → 評估）的
> 自包含 notebook。我在轉向資料科學的過程中把它們建成
> 一個學習晶格（learning lattice）。`src/data_analysis_chatbots/clustering/` 中的分群
> 演算法是那些探索 notebook 的整併、生產級實作。
> RFM 和 CLV 是從 `04_recommendation/` 的兔子洞中
> 衍生出的行銷領域延伸。」

若面試官進一步深挖，引導到憑直覺手工打造的
特定 notebook（`notebooks/` 中的 3 個）。

---

## 橫切風險（Cross-cutting risks，影響 ≥ 2 個 repo）

| # | 風險 | 影響 | 緩解截止日 |
|---|---|---|---|
| 1 | git 歷史中的真實 API 金鑰 | spectyn-secops (#3abf406, #0d5c714) | **5/8** — 公開翻轉前執行 `git filter-repo` + `git push --force`；在供應商端撤銷金鑰 |
| 2 | 一日傾倒對照核實 | Automation_with_AI (#4) | 5/9 — 寫好誠實答覆；引導至可驗證的 repo |
| 3 | README 中的「進行中」承諾 | Data-Analysis-with-Chatbots (#5) | 5/8 — 緩和為「planned for v0.2」 |
| 4 | spectyn-mesh 生態系表格漂移 | 全部 5 個 repo | 5/9 早晨起飛前檢查：`curl github.com/markl-a/spectyn-mesh/blob/master/README.md` 並 grep 全部 5 個 repo 名稱 |

---

## 此凍結**不**涵蓋的範圍

- **建立 repo** — 5/15 前不建立新 repo
- **重新命名 repo** — 名稱已釘選；重新命名會破壞連結
- **Star/watch 計數** — 隨時間自然累積，不做遊戲化（gamification）
- **GitHub Pages / 實機部署** — 在 v0.2 前不在範圍內
- **對 repo 3/4/5 的實質程式碼新增** — 明確凍結
  （僅針對 #5 的誤導性那行動 `[fix-docs]`）

---

## 作品集 repo 的解凍時程

- **5/8** — 為 demo 擷取最終狀態（不再變更）
- **5/9 09:00** — 起飛前驗證每個 repo 的 demo 路徑為綠燈
- **5/9 — 面試** — 凍結
- **5/10–14** — 緩衝期；僅限關鍵安全修正（例如
  若延誤則進行 filter-repo 清理）
- **5/15** — 公開翻轉 + OSS 開源發佈；凍結結束；v0.2 作品集
  計畫啟動

---

## 驗證命令（依需求執行）

```bash
# Repo 1: spectyn-secops
cd ~/path/to/spectyn-secops
git log --oneline -1                    # → 51220f8 expected
make demo-mock | tail -3                # → reports/runs/<ts>/ written
make test                                # → 7/7 passing
make lint                                # → clean

# Repo 2: spectyn-mobile
cd ~/path/to/spectyn-mobile
git log --oneline -1                    # → eeb2db2 expected
make demo-mock | tail -3                # → 48p/8w/4f
make test                                # → 14/14 passing

# Repos 3-5: read-only, just confirm last hash + public visibility
gh repo view markl-a/My-AI-Learning-Notes --json visibility,stargazerCount
gh repo view markl-a/Automation_with_AI --json visibility,stargazerCount
gh repo view markl-a/Data-Analysis-with-Chatbots --json visibility,stargazerCount

# Cross-cutting: spectyn-mesh README still lists all 5 in Ecosystem
grep -E "spectyn-secops|spectyn-mobile|My-AI-Learning-Notes|Automation_with_AI|Data-Analysis-with-Chatbots" \
   ~/path/to/spectyn-mesh/README.md
```

---

**審查者（Reviewers）**（本文件合併至 platform/macos 前）：
- 本 session（撰寫者）
- 以下之一：subagent / codex / gemini / opencode（獨立閱讀，
  與本規格（PORTFOLIO-SPEC-FREEZE-V1）§8 相同的 MULTI-AGENT-QA workflow）
