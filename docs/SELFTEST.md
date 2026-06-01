# 自我測試套件（Self-Test Suite）

一套即插即用、登錄表（registry）風格的測試框架。**每個新功能都附帶一個位於
`scripts/selftest.d/` 底下的檔案**；協調器（orchestrator，`scripts/selftest.sh`）
會自動探索（auto-discover）到它。輸出預設為人類可讀格式，需要時也可輸出機器可讀
（JSON）格式，因此開發者與 LLM 代理（agent，如 Claude Code、phantom 本身、CI）
都能用同樣的方式執行它。

## 執行它

共有三個進入點（entry point）——它們做的事完全相同，挑一個符合你情境的就好：

```bash
# 1. Native subcommand (works anywhere phantom is on PATH)
phantom selftest                          # text, all features
phantom selftest --json --out r.json      # JSON report
phantom selftest --feature mcp            # just one feature
phantom selftest --p0-only                # CI smoke run
phantom selftest --list                   # show registered features

# 2. make targets (in the repo)
make selftest
make selftest-json
make selftest-list

# 3. The script directly (when you don't have a phantom binary handy)
scripts/selftest.sh --json --out a.json

# Env knobs (apply to all three)
PHANTOM_BIN=/path/to/phantom phantom selftest
COORD=http://10.0.0.5:7878   phantom selftest
PHANTOM_SELFTEST_SCRIPT=/abs/path/selftest.sh phantom selftest   # script override
PHANTOM_BASH=/path/to/bash.exe phantom selftest                  # bash override (Windows)
```

## 平台支援

| 平台 | 狀態 |
|---|---|
| macOS    | ✅ 一等公民（first-class）—— bash 與 `python3` 預設隨附 |
| Linux    | ✅ 一等公民（first-class）—— bash 與 `python3` 預設隨附 |
| Windows + Git Bash | ✅ —— `phantom selftest` 會自動尋得 `bash.exe`。當缺少 `python3` 時，純 bash 的 JSON 建構器會接手執行，因此無需額外安裝。`jq` 為選用（僅在消費 JSON 時需要）。 |
| Windows + WSL | ✅ —— 與 Linux 相同的一等公民體驗 |
| Windows native（cmd / PowerShell，無 Git/WSL） | ❌ —— 沒有 bash。`phantom selftest` 會印出一次性的安裝提示，指向 `https://git-scm.com/download/win`。 |

`phantom selftest` 這個 Rust shim（薄墊片）在 Windows 上會以下列順序探測 bash：

1. `$PHANTOM_BASH`（若已設定；檔案不存在 → 以清楚的錯誤訊息 exit 2）
2. `$PATH` 上的 `bash` / `bash.exe`
3. `C:\Program Files\Git\bin\bash.exe`
4. `C:\Program Files (x86)\Git\bin\bash.exe`
5. `%LOCALAPPDATA%\Programs\Git\bin\bash.exe`（每使用者的 Git 安裝）

`phantom selftest` 是一個輕薄的 Rust shim：它會在 repo 中定位 `scripts/selftest.sh`
（cwd → 從 cwd 往上層尋找 → 從二進位檔往上層尋找 →
`~/.phantom-mesh/scripts/selftest.sh`），然後 exec bash 並轉發你所有的引數。
因此一個驅動 phantom 的 LLM 代理 —— 或 phantom 透過自己的 `shell` 工具驅動自己 ——
只需要知道 `phantom selftest` 即可。

## 結束碼（Exit codes）

| 代碼 | 意義 |
|------|---------|
| 0    | 無 P0 失敗 |
| 1    | 至少一個 P0 測試失敗 |
| 2    | 協調器錯誤（缺少 `selftest.d/`、引數錯誤） |

## LLM 代理應如何消費輸出（自我除錯迴圈）

本套件的設計讓代理 —— Claude Code、phantom 本身或 CI —— 能在沒有人類介入的情況下
執行、診斷並修復。

```bash
scripts/selftest.sh --json --out /tmp/r.json   # exit 0 = green / 1 = P0 red

# 1. Triage — what failed?
jq '.summary'                                                 /tmp/r.json

# 2. For each failure, get THREE things needed to debug:
#    - the exact shell command to re-run just that check (repro)
#    - a path to a file with full stdout+stderr+exit code (artifact)
#    - source paths to grep first (hints, declared in feature meta)
jq -r '
  .features[] as $f
  | $f.tests[]
  | select(.status=="fail")
  | "── \($f.name)/\(.name)\nrepro:    \(.repro // "n/a")\nartifact: \(.artifact // "n/a")\nhints:    \($f.hints | join(" "))\n"
' /tmp/r.json

# 3. Re-run a single failing check directly (faster than the whole suite):
scripts/selftest.sh --feature <feature-name>

# 4. Read the artifact for full output:
cat <artifact-path>          # contains: command, cwd, date, stdout+stderr, exit
```

JSON 的結構是穩定的。失敗的列總是包含 `repro`，並且（對於輔助函式
`t_run`/`t_check` 或任何設定了 `T_ARTIFACT` 的測試）包含一個 `artifact`。
每個功能都附帶一個 `hints` 陣列 —— 代理應該優先 grep 的路徑。

```json
{
  "phantom_version": "phantom 0.x.y (abc1234, ...)",
  "started_at": "2026-05-05T12:00:00Z",
  "duration_s": 14,
  "artifacts_dir": "test-results/selftest-20260505T120000Z",
  "summary": {"pass": 27, "fail": 0, "skip": 3, "p0_failures": 0},
  "features": [
    {
      "name": "binary",
      "priority": "P0",
      "requires": "",
      "description": "...",
      "hints": ["core/src/bin/phantom.rs", "core/src/main.rs"],
      "file": "00-binary.sh",
      "tests": [
        {
          "name": "phantom --version",
          "status": "pass",
          "detail": "phantom 0.5.1 (abc1234, macos-aarch64, ...)",
          "repro":  "/Users/me/.cargo/bin/phantom --version | grep -qE '^phantom [0-9]+\\.[0-9]+'",
          "artifact": "test-results/selftest-.../binary/phantom-version.log"
        }
      ]
    }
  ]
}
```

### 執行產物（artifacts）目錄

每次執行都會建立 `test-results/selftest-<utc-timestamp>/`，內含：

- `selftest.log`  —— TSV 格式：`feature\tstatus\tname\tdetail\trepro\tartifact`
- `<feature>/<test-slug>.log` —— 任何使用 `t_run` / `t_check` 或設定了
  `T_ARTIFACT` 的檢查的完整 stdout+stderr+exit。標頭（header）會記錄命令、
  cwd 與時間戳記，因此代理可以忠實地重新執行。

舊的執行目錄會自動修剪（pruned，保留最近 10 個）。

## 為新功能新增一個自我測試（60 秒）

1. 挑一個數字前綴與一個簡短名稱。慣例如下：
   - `00-09` 啟動引導（bootstrap，binary、version）
   - `10-29` 核心 CLI（doctor、run、init）
   - `30-49` 網路介面（serve、mcp、mesh）
   - `50-69` 平台整合（snapshot-mac、launchd、systemd）
   - `70-89` 重量級 / 選用（mlx、autoevolve 完整週期）
   - `90+`   實驗性

2. `cp scripts/selftest.d/_template.sh scripts/selftest.d/35-myfeature.sh`

3. 填入 `selftest_feature_meta` 與 `selftest_run`。只使用這些輔助函式（helper）：

   | 輔助函式 | 用途 |
   |---|---|
   | `t_pass <name> [detail]` | 成功 |
   | `t_fail <name> [detail]` | 失敗 |
   | `t_skip <name> [reason]` | 在此主機上已知無關 |
   | `t_run   <name> <argv...>` | 執行 argv，完整輸出 → artifact，記錄 repro |
   | `t_check <name> "<shell>"` | 執行 shell 字串，完整輸出 → artifact，repro = 該字串 |
   | `t_have <cmd>`           | 述詞（predicate）：命令是否在 PATH 上？ |
   | `t_http <url> [code]`    | 述詞（predicate）：HTTP 是否回傳預期的代碼？ |

   當被測試的對象是一個 shell 命令時，**永遠優先使用 `t_check` / `t_run` 而非手動的
   `t_pass`/`t_fail`**。它們會自動把 stdout+stderr 擷取到
   `$SELFTEST_ARTIFACTS/<slug>.log`，並把命令存進 `repro`
   欄位，而這正是 LLM 代理用來重新執行並只除錯該檢查的依據。

   若要手動使用 `t_pass`/`t_fail`（例如你計算出了一個值），請在呼叫前緊接著設定
   `T_REPRO=<cmd>` 與／或 `T_ARTIFACT=<path>` ——
   它們在呼叫後會被自動清除。

7. **在你的功能 meta 中加上 `hints=`** —— 以空白分隔的來源路徑，當此功能損壞時
   代理應優先 grep 這些路徑。範例：
   `hints=core/src/serve.rs core/src/main.rs`。這些會傳播到 JSON
   報告中，讓 LLM 有一組起始路徑而無需猜測。

4. 若你的功能需要協調器無法推斷的前置條件（daemon（常駐服務）正在執行、
   模型檔案存在、對等節點可達），請定義
   `selftest_requires` —— 回傳非零值並在 stderr 上附一行原因，
   即可乾淨地跳過整個功能。

5. `chmod +x scripts/selftest.d/35-myfeature.sh && make selftest-list`
   以確認它被偵測到，接著 `make selftest` 來執行。

### 風格規則

- **不要 echo PASS/FAIL。** 永遠使用 `t_pass / t_fail / t_skip`，這樣 JSON
  報告才看得到你那一列。
- **一行一個斷言（assertion）。** 不要把「功能 X 的所有東西都正常」綁成
  一個巨大的測試 —— 五個小斷言能告訴你_是什麼_壞了。
- **預設冪等（idempotent）且唯讀。** 自我測試必須能安全地在使用者的活機器上
  執行。若某個檢查會寫入狀態，請在同一個函式內復原，或把它移到由
  像 `PHANTOM_SELFTEST_DESTRUCTIVE=1` 這類環境變數把關的
  `requires=destructive` 優先級 P2 檔案中。
- **廉價。** 目標是整個套件在一台溫機（warm）筆電上於 30 秒內完成。把昂貴的
  檢查（完整 evolve 週期、MLX 推論）移到 P2 並以
  `selftest_requires` 守護它們。
- **挑對優先級。** P0 = 出貨阻擋者（ship-blocker），P1 = 在任何健康安裝上
  預期應通過，P2 = 有更好、視環境而定。

### 什麼該放進套件、什麼該用 `cargo test`

| | self-test | cargo test |
|---|---|---|
| 目標 | **已安裝的** `phantom` 二進位檔、**執行中的** daemon、真實網路 | Rust 單元 / 整合測試 |
| 由誰執行 | 使用者、Claude、CI 煙霧測試（smoke）任務 | 開發迴圈、CI 建置任務 |
| 斷言風格 | 結束碼、HTTP、JSON 鍵、是否存在區段標籤 | `assert_eq!` |

如果你在測試純 Rust 邏輯，請寫 `cargo test`。如果你在測試
「已安裝的二進位檔是否仍產生正確的 `phantom doctor` 輸出」，
那就是一個自我測試功能。

## 實作範例：為新的 `phantom backup` 命令新增自我測試

```bash
# scripts/selftest.d/45-backup.sh
selftest_feature_meta() {
  echo "name=backup"
  echo "priority=P1"
  echo "requires="
  echo "description=phantom backup create + list round-trip on a temp dir"
}

selftest_run() {
  dest="$TMP/backup-test"
  mkdir -p "$dest"
  if "$PHANTOM" backup create --dest "$dest" --dry-run >"$TMP/bk.out" 2>&1; then
    t_pass "backup create --dry-run" "$(head -1 $TMP/bk.out)"
  else
    t_fail "backup create --dry-run" "$(tail -1 $TMP/bk.out)"
    return
  fi

  out="$("$PHANTOM" backup list --json 2>/dev/null)"
  if echo "$out" | jq -e '. | length >= 0' >/dev/null 2>&1; then
    t_pass "backup list returns JSON" ""
  else
    t_fail "backup list returns JSON" "not valid JSON"
  fi
}
```

這就是整套儀式：一個檔案、兩個函式，自動接入
`make selftest`、`make selftest-json`、JSON 報告與 CI。
