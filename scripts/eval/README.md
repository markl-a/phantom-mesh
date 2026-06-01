# phantom-mesh 評測框架（eval harness，採用 promptfoo，offline-first 離線優先）

一套小型的 [promptfoo](https://promptfoo.dev) 評測套件，針對 phantom-mesh 的
agent（代理）/provider（供應商）層。它是 **deterministic（決定性的），且預設 OFFLINE 離線執行** —— 沒有
雲端、不需要 API key（金鑰）、無 telemetry（遙測）。這些測試案例用 promptfoo 的非 LLM
asserts（斷言）（`contains`、`regex`、`is-json`、`javascript`）來驗證 phantom *自身*的
行為（CLI 契約 + provider-route 路由解析）。

## 執行

```bash
# one-shot (CI mode, non-zero exit on failure)
bash scripts/eval/run.sh

# or directly via promptfoo
npx promptfoo eval -c scripts/eval/promptfooconfig.yaml

# watch mode (re-run on edits, local dev)
bash scripts/eval/run.sh --watch
# equivalently: npx promptfoo eval -c scripts/eval/promptfooconfig.yaml --watch

# browse the last run in the local web UI (optional, still offline)
npx promptfoo view
```

`run.sh` 會優先使用本機已安裝的 `promptfoo`（例如 `app/node_modules/.bin`），
並在找不到時退回 `npx --yes promptfoo@latest`。第一次執行 `npx` 時會
從 npm registry（套件登錄處）下載 promptfoo；但每一次評測*執行本身*都是離線的。

## 預設離線 + 隱私

- `promptfooconfig.yaml` 中設定 `sharing: false` —— 結果永遠不會上傳到
  `app.promptfoo.dev`。
- `run.sh` 會匯出 `PROMPTFOO_DISABLE_TELEMETRY=1` 與 `PROMPTFOO_DISABLE_UPDATE=1`
  —— 沒有分析數據、沒有更新檢查的網路請求。
- 預設的 provider 是一個本機 Node harness（`phantom_harness.js`），它驅動
  本機的 `phantom` CLI 以及一個純 in-process（行程內）的 route resolver（路由解析器）。**預設案例不會碰到任何
  網路、LLM 或 API key**。
- 唯一一個依賴 LLM 的案例是 **以 `PROMPTFOO_LLM=1` 作為 gate（閘門）控制，且預設會被
  skip 跳過**（當該 gate 未設定時，它的 `javascript` 斷言會回傳通過）。

## 各案例檢查的內容

| # | `vars.cmd` | 它驗證什麼 | 使用的斷言 |
|---|------------|-----------------|--------------|
| 1 | `version` | `phantom --version` 輸出一個格式正確的 `phantom <semver> (<sha>+, <target>, built <date>)` 橫幅（banner） | `contains`、`regex` |
| 2 | `help` | 頂層 `phantom --help` 揭示核心命令面（`phantom exec` / `phantom serve` / `phantom selftest`） | `contains` x3 |
| 3 | `exec-help` | `phantom exec --help` 描述 headless（無介面）CI 契約：從 `stdin` 讀取 prompt、支援 `--json` 機器可讀輸出、標記為 "Headless" | `contains`、`regex` |
| 4 | `route` | `provider:model` 規格（`groq:llama-3.1-8b-instant`）解析到被釘選的模型（`pinned=true`）—— 純運算、無 I/O | `is-json`、`javascript` |
| 5 | `route` | 裸 provider 規格（`groq`）使 `model=null`、`pinned=false`（退回到該 provider 的預設模型） | `is-json`、`javascript` |
| 6 | `version` | **[已 gate]** LLM 冒煙測試（smoke test）—— **除非設定 `PROMPTFOO_LLM=1` 否則跳過**；作為真正本機 Ollama 斷言的佔位符（placeholder） | `javascript`（自我跳過） |

案例 4 與 5 編碼了 phantom 已記載的 `provider:model` 優先序規則
（`agent.X.model` / `provider:model` 語法會釘選一個特定模型；裸 provider 則
使用其 `default_model`）。它們完全在行程內執行，零 subprocess（子行程）且
零網路，所以即使 `phantom` 二進位檔不存在它們也會通過。

## 需求

- **Node.js**（給 harness 與 promptfoo 使用）。
- 案例 1-3 需要 **`phantom` CLI 在 PATH 上**。如果 `phantom` 不在 PATH 上但位於
  `~/.cargo/bin/phantom`，`run.sh` 會自動把它加進去。你也可以用
  `PHANTOM_BIN=/path/to/phantom` 讓 harness 指向某個特定的二進位檔。如果 `phantom`
  完全不存在，案例 1-3 會報錯（清楚地回報），而案例 4-5 仍會通過。

## 啟用 LLM 案例（案例 6）

```bash
# requires a local model endpoint (e.g. Ollama at http://localhost:11434);
# still no cloud key required if you point at a local model.
PROMPTFOO_LLM=1 bash scripts/eval/run.sh
```

在依賴案例 6 之前，先把實際的本機模型 provider/斷言接進
`promptfooconfig.yaml` 裡的案例 6；預設情況下它只會重新檢查
決定性的橫幅，所以即使在沒有外部服務的情況下打開這個 gate 也是安全的。

## 檔案

- `promptfooconfig.yaml` —— 整套套件（6 個案例；5 個離線 + 1 個 gated）。
- `phantom_harness.js` —— 離線 Node provider；驅動 `phantom` CLI 與
  純 route resolver。會移除 ANSI 碼，讓斷言不受終端機（terminal）差異影響。
- `run.sh` —— CI 執行器；提供離線/隱私的環境變數防護 + 失敗時非零退出。
