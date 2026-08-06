# Spectyn Mesh — 工具參考手冊

本文件是 spectyn-mesh 核心內部執行的每個代理（agent）所有可用工具的權威參考。撰寫方式力求讓另一個 AI 代理能準確理解每個工具的功能、可接受的參數，以及如何有效地串接（chain）多個工具。

---

## 總覽表

| 工具 | 模組 | 說明 |
|---|---|---|
| `shell` | shell | 執行 shell 命令（含安全過濾），回傳 stdout/stderr 與結束碼（exit code） |
| `file_read` | file | 讀取檔案內容，可選擇依行範圍切片 |
| `file_write` | file | 以任意內容寫入（或建立）檔案 |
| `file_edit` | file | 在檔案中替換一個精確比對的字串 |
| `content_search` | search | 使用 ripgrep 搜尋檔案內容（正規表示式／字面值，附上下文行） |
| `glob_search` | search | 以名稱 glob 樣式尋找檔案 |
| `web_search` | web | 透過 Brave API 或 DuckDuckGo 後備方案搜尋網路 |
| `memory_store` | memory | 將鍵值對（key-value）持久化儲存到磁碟 |
| `memory_recall` | memory | 從磁碟依鍵（key）取回值 |
| `memory_list` | memory | 列出所有已儲存的記憶項目（可選依命名空間 namespace 篩選） |
| `memory_delete` | memory | 依鍵刪除單一記憶項目 |
| `memory_search` | memory | 以子字串搜尋記憶值 |
| `git_status` | git | 顯示精簡（short）格式的 git 工作樹（working-tree）狀態 |
| `git_diff` | git | 顯示 git diff 統計（已暫存或未暫存，可選針對單一檔案） |
| `git_log` | git | 顯示近期 git 提交（commit，單行格式） |
| `git_commit` | git | 以訊息建立一次 git 提交 |
| `git_add` | git | 將一個或多個檔案暫存（stage）以供下次提交 |
| `git_push` | git | 將提交推送（push）到遠端（需要 SPECTYN_AUTO_APPROVE=1） |
| `git_reset` | git | 重設工作樹（soft/hard） |
| `git_blame` | git | 顯示檔案每一行的作者歸屬 |
| `git_show` | git | 顯示特定提交的詳細資訊 |
| `git_branch_list` | git | 列出本地（或全部）分支及詳細資訊 |
| `git_checkout` | git | 切換分支或建立新分支 |
| `git_stash_list` | git | 列出儲存庫中所有暫存堆疊（stash） |
| `fetch` | fetch | 抓取一個 URL 並回傳清理後的文字（移除 HTML、美化 JSON） |
| `http_get` | http_client | 原始 HTTP GET 請求；回傳狀態與內文（body） |
| `http_post` | http_client | 以 JSON 或文字內文發出原始 HTTP POST 請求 |
| `ls` | ls | 列出目錄內容（精簡或詳細格式，可選樹狀檢視） |
| `ls_stat` | ls | 取得單一檔案／目錄的詳細資訊（大小、權限、時間戳、行數） |
| `list_files` | fs | 遞迴列出目錄下的檔案，可選名稱篩選 |
| `list_dir` | fs | 列出單一目錄層級及大小 |
| `create_dir` | fs | 建立目錄（含上層目錄） |
| `rename_file` | fs | 重新命名／移動檔案（需要 SPECTYN_AUTO_APPROVE=1） |
| `delete_file` | fs | 刪除單一檔案（10 MB 大小安全防護） |
| `patch` | patch | 將統一差異格式（unified diff）修補檔套用到一個或多個檔案 |
| `multi_file_edit` | multi_edit | 跨多個檔案以原子（atomic）方式套用多個精確字串替換 |
| `diff_files` | diff_view | 產生兩個檔案間的統一差異 |
| `diff_strings` | diff_view | 產生兩個字串間的統一差異 |
| `cargo_check` | diagnostic | 執行 `cargo check` 並摘要錯誤／警告 |
| `cargo_test` | diagnostic | 執行 `cargo test` 並摘要結果 |
| `tsc_check` | diagnostic | 執行 TypeScript 編譯器型別檢查（不產出檔案） |
| `run_tests` | diagnostic | 執行任意測試命令並回傳輸出 |
| `task_add` | task | 將一項任務加入工作階段（session）任務清單 |
| `task_update` | task | 更新任務的狀態 |
| `task_list` | task | 列出任務（可選依狀態篩選） |
| `task_clear` | task | 移除任務（全部，或僅已完成） |
| `shell_bg` | shell | 啟動一個長時間執行的背景工作（立即回傳 PID） |
| `shell_bg_check` | shell | 檢查背景工作的狀態 |

---

## 工具參考

---

### `shell`

執行一個 shell 命令並回傳 stdout、stderr 與結束碼。支援複合命令（`&&`、`||`、`;`）。某些具破壞性的樣式需要 `SPECTYN_AUTO_APPROVE=1`。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `command` | string | 是 | — | Shell 命令。支援 `&&`、`||`、`;` 複合運算子（最多 10 段）。`$(...)` 與反引號替換會被封鎖。 |
| `timeout_secs` | integer | 否 | 30 | 最大執行時間（秒，上限 300）。 |
| `cwd` | string | 否 | 目前目錄 | 工作目錄；必須存在。 |
| `env` | object | 否 | `{}` | 額外環境變數，會與目前環境合併。 |
| `stdin` | string | 否 | — | 透過管道（pipe）送入命令 stdin 的文字。 |

**安全注意事項：**
- 硬性封鎖樣式（一律拒絕）：`rm -rf /`、`rm -rf ~`、`sudo rm`、`sudo dd`、`mkfs`、`dd if=/dev/zero of=/dev/`、`:(){:|:&};:`、`chmod -R 777 /`、`curl | sh` 等。
- 需要 `SPECTYN_AUTO_APPROVE=1` 的樣式：`rm `、`sudo `、`kill `、`pkill `、`git reset --hard`、`git clean `、`chmod `、`chown `、`DROP TABLE`、`curl `、`wget `、`nc `，以及把 `mv`／`cp` 指向絕對路徑。
- `$(...)` 子 shell（subshell）替換與反引號展開一律封鎖。

**輸出格式：**
- 僅 stdout：原始文字 + `[exit code: N]`
- 同時有 stdout 與 stderr：`STDOUT:\n<text>\nSTDERR:\n<text>\n[exit code: N]`
- 僅 stderr：`STDERR:\n<text>\n[exit code: N]`
- 輸出於 20,000 字元處截斷。

**範例：**
```json
{"command": "cargo build --release", "cwd": "/workspace/myproject", "timeout_secs": 120}
```

**含 env 與 stdin 的範例：**
```json
{"command": "cat", "stdin": "hello world", "env": {"DEBUG": "1"}}
```

**回傳：** 合併的 stdout/stderr，並附上結束碼。

---

### `shell_bg`

在背景啟動一個長時間執行的命令，不等待其結束。立即回傳 PID。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `command` | string | 是 | — | 要在背景執行的命令。 |
| `label` | string | 否 | 同 command | 供追蹤用的人類可讀標籤。 |

**範例：**
```json
{"command": "sleep 600", "label": "keep-alive"}
```

**回傳：** `Job started: PID=12345 label='keep-alive'\nUse shell with command 'kill 12345' to stop...`

---

### `shell_bg_check`

檢查由 `shell_bg` 追蹤的背景工作狀態。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `pid` | integer | 否 | — | 要檢查的特定 PID。若省略，則列出所有追蹤中的工作。 |

**範例：**
```json
{"pid": 12345}
```

**回傳：** `PID 12345 (keep-alive): running` 或 `finished`。

---

### `file_read`

讀取檔案內容。二進位檔案會被偵測並回報而不予解碼。輸出於 100,000 字元處截斷。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 是 | — | 絕對或相對檔案路徑。 |
| `offset` | integer | 否 | 1 | 起始行，以 1 為起點。與 `limit` 搭配用於視窗式讀取。 |
| `limit` | integer | 否 | 全部 | 回傳的最大行數。 |
| `start_line` | integer | 否 | — | 舊版（legacy）：以 1 為起點的起始行（建議改用 `offset`）。 |
| `end_line` | integer | 否 | — | 舊版：以 1 為起點、含端點的結束行（建議改用 `offset`+`limit`）。 |
| `show_line_numbers` | boolean | 否 | false | 若為 true，每行前綴其行號。 |

當提供 `offset` 或 `limit` 時，回應會包含類似 `[Lines 10-59 of 300]` 的標頭，且每行皆前綴其行號。

**範例 — 讀取整個檔案：**
```json
{"path": "src/main.rs"}
```

**範例 — 讀取第 100-149 行：**
```json
{"path": "src/main.rs", "offset": 100, "limit": 50}
```

**範例 — 含行號讀取：**
```json
{"path": "Cargo.toml", "show_line_numbers": true}
```

**回傳：** 檔案文字（可能帶行號前綴），或 `[binary file, N bytes]`，或錯誤字串。

---

### `file_write`

將內容寫入檔案，預設會建立缺少的上層目錄。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 是 | — | 目標檔案路徑。 |
| `content` | string | 是 | `""` | 要寫入的完整內容（覆寫既有檔案）。 |
| `create_dirs` | boolean | 否 | true | 若為 `true`，自動建立缺少的上層目錄。 |

**範例：**
```json
{"path": "src/config.rs", "content": "pub const VERSION: &str = \"1.0\";\n"}
```

**回傳：** `Written N bytes to <path>` 或錯誤字串。

---

### `file_edit`

替換檔案中的一個精確字串。`old_string` 必須恰好比對一次（預設）。使用 `replace_all` 替換每一處出現。使用 `line_range` 限制搜尋範圍。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 是 | — | 要編輯的檔案。 |
| `old_string` | string | 是 | — | 要尋找的精確文字。除非 `replace_all` 為 true，否則必須恰好比對一次。 |
| `new_string` | string | 否 | `""` | 替換文字。 |
| `replace_all` | boolean | 否 | false | 若為 true，替換每一處出現。 |
| `line_range` | object | 否 | — | 限定搜尋範圍：`{"start": N, "end": M}`（以 1 為起點、含端點）。 |

**錯誤：**
- `old_string not found` — 文字不存在；回應會包含搜尋範圍的 200 字元預覽。
- `old_string appears N times` — 當 `replace_all` 為 false 而存在多筆比對時；回應會列出行號。

**範例 — 單一替換：**
```json
{"path": "src/lib.rs", "old_string": "fn old_name(", "new_string": "fn new_name("}
```

**範例 — 在範圍內全部替換：**
```json
{
  "path": "src/config.rs",
  "old_string": "TODO",
  "new_string": "DONE",
  "replace_all": true,
  "line_range": {"start": 10, "end": 50}
}
```

**回傳：** 成功時：`Edited <path> successfully.\n\nDiff:\n<mini-diff>`（或對於 `replace_all`，回傳 `Edited <path> (N occurrence(s) replaced).`）。

---

### `content_search`

使用 ripgrep 搜尋檔案內容（若未安裝 rg 則退回 grep）。回傳比對的行，附上檔案路徑與上下文。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `pattern` | string | 是 | — | 正規表示式或字面值搜尋樣式（最多 500 字元）。 |
| `path` | string | 否 | `.` | 要搜尋的目錄或檔案。 |
| `context_lines` | integer | 否 | 2 | 每筆比對前後顯示的上下文行數。 |
| `file_type` | string | 否 | — | 依檔案類型篩選（不含點），例如 `"rs"`、`"ts"`、`"py"`。使用 ripgrep 的 `-t` 旗標。 |
| `case_sensitive` | boolean | 否 | false | 若為 true，執行區分大小寫的搜尋。 |
| `max_results` | integer | 否 | 50 | 回傳的最大比對行數。 |

**安全性：** path 引數會經過驗證 — `..` 路徑穿越（traversal）與 shell 注入字元（`;`、`|`、`&`、`$`、`` ` ``、`>`、`<`）會被拒絕。

**範例 — 找出某函式的所有用法：**
```json
{"pattern": "fn send_message", "path": "core/src", "file_type": "rs"}
```

**範例 — 區分大小寫的搜尋：**
```json
{"pattern": "TODO", "path": ".", "case_sensitive": true, "context_lines": 0}
```

**回傳：** ripgrep 輸出（file:line:content 行，以 `--` 區塊（hunk）分隔符分開）或 `No matches found`。

---

### `glob_search`

尋找符合 glob 樣式的檔案。使用 ripgrep `--files` 以求速度（自動排除 `.git/`、`node_modules/`、`target/`）；退回時改用 `find`。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `pattern` | string | 是 | — | Glob 樣式，例如 `"**/*.rs"`、`"src/**/*.ts"`、`"*.toml"`（最多 200 字元）。 |
| `path` | string | 否 | `.` | 要搜尋的基底目錄。 |
| `exclude` | array of strings | 否 | `[]` | 額外要排除的 glob 樣式，例如 `["tests/**", "*.lock"]`。 |
| `max_results` | integer | 否 | 200 | 回傳的最大檔案數。 |

**範例 — 找出所有 Rust 原始檔：**
```json
{"pattern": "**/*.rs", "path": "core/src"}
```

**範例 — 找出 TypeScript 檔案，排除測試：**
```json
{"pattern": "**/*.ts", "path": "app/src", "exclude": ["**/*.test.ts"]}
```

**回傳：** 排序後的比對檔案路徑清單，每行一筆；若達到上限則附上截斷通知。

---

### `web_search`

搜尋網路。當 `agents.toml` 中設定了 `brave_search_api_key` 時使用 Brave Search API；否則退回 DuckDuckGo Instant Answer API，再退回 DuckDuckGo HTML 搜尋。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `query` | string | 是 | — | 搜尋查詢。 |
| `num_results` | integer | 否 | 5 | 回傳的結果數（最多 10）。 |

**範例：**
```json
{"query": "Rust async tokio tutorial 2024", "num_results": 5}
```

**回傳：** 編號清單：`[N] Title\n    URL: ...\n    Snippet: ...` 或 `No results for: <query>`。

---

### `memory_store`

將一組鍵值字串對持久化儲存到 `~/.spectyn-mesh/memory.json`。支援帶命名空間的鍵。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `key` | string | 是 | — | 鍵名。 |
| `value` | string | 是 | — | 要儲存的值。 |
| `namespace` | string | 否 | — | 前綴；儲存的鍵會變成 `{namespace}/{key}`。 |

**範例：**
```json
{"key": "project_root", "value": "/workspace/myproject", "namespace": "agent_a"}
```

**回傳：** `Stored: agent_a/project_root = /workspace/myproject`

---

### `memory_recall`

依鍵取回已儲存的值。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `key` | string | 是 | — | 要查詢的鍵。 |
| `namespace` | string | 否 | — | 命名空間前綴（必須與 `memory_store` 所用相符）。 |

**範例：**
```json
{"key": "project_root", "namespace": "agent_a"}
```

**回傳：** 已儲存的值字串，或 `No memory found for key: <key>`。

---

### `memory_list`

列出所有已儲存的鍵與截斷後的值（值於 50 字元處截斷）。可選依命名空間篩選。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `namespace` | string | 否 | — | 若提供，僅列出完整名稱以 `{namespace}/` 開頭的鍵。 |

**範例 — 列出全部：**
```json
{}
```

**範例 — 列出單一命名空間：**
```json
{"namespace": "agent_a"}
```

**回傳：** 排序後的 `key: value…` 行，或 `No memory entries stored.`

---

### `memory_delete`

依鍵刪除單一記憶項目。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `key` | string | 是 | — | 要刪除的鍵。 |
| `namespace` | string | 否 | — | 命名空間前綴。 |

**範例：**
```json
{"key": "project_root", "namespace": "agent_a"}
```

**回傳：** `deleted` 或 `key not found`。

---

### `memory_search`

以子字串比對（不分大小寫）搜尋記憶值。可選限制於某命名空間。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `query` | string | 是 | — | 要在值中搜尋的子字串。 |
| `namespace` | string | 否 | — | 將搜尋限制於此命名空間。 |

**範例：**
```json
{"query": "workspace", "namespace": "agent_a"}
```

**回傳：** 比對項目的排序後 `key: value` 行，或 `No memory entries matching '<query>'.`

---

### `git_status`

以精簡格式顯示 git 工作樹狀態（`git status --short`）。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 否 | `.` | Git 儲存庫路徑。 |

**範例：**
```json
{"path": "/workspace/myproject"}
```

**回傳：** 精簡格式的狀態行（例如 `M src/main.rs`、` M Cargo.lock`）或 `Working tree clean`。

---

### `git_diff`

顯示 git diff 統計（每個檔案的新增／刪除行數）。可針對已暫存的變更或特定檔案。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 否 | `.` | 儲存庫目錄。 |
| `cached` | boolean | 否 | false | 若為 true，顯示已暫存（cached）的 diff。 |
| `file` | string | 否 | — | 將 diff 限制於此檔案。 |

**範例 — 未暫存的變更：**
```json
{"path": "."}
```

**範例 — 單一檔案的已暫存 diff：**
```json
{"cached": true, "file": "src/agent.rs"}
```

**回傳：** `git diff --stat` 的輸出。

---

### `git_log`

以單行格式顯示近期提交（`git log --oneline -N`）。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 否 | `.` | 儲存庫目錄。 |
| `n` | integer | 否 | 10 | 要顯示的提交數。 |

**範例：**
```json
{"n": 5}
```

**回傳：** 提交雜湊（hash）與訊息，每行一筆。

---

### `git_commit`

建立一次 git 提交。你必須先暫存檔案（使用 `git_add` 或以 `shell` 執行 `git add`）。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `message` | string | 是 | — | 提交訊息（最多 1000 字元；`$(...)` 與反引號會被封鎖）。 |
| `path` | string | 否 | `.` | 儲存庫目錄。 |

**範例：**
```json
{"message": "fix: correct off-by-one in line range calculation"}
```

**回傳：** `git commit -m` 的合併 stdout/stderr。

---

### `git_add`

將一個或多個檔案暫存以供下次提交（`git add -- <files>`）。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `files` | array of strings | 是 | — | 要暫存的檔案路徑清單。不可為空。 |
| `path` | string | 否 | `.` | 儲存庫目錄。 |

**範例：**
```json
{"files": ["src/agent.rs", "src/session.rs"], "path": "/workspace/myproject"}
```

**回傳：** `Staged N file(s): file1, file2` 或錯誤字串。

---

### `git_push`

將提交推送到遠端儲存庫。**需要 `SPECTYN_AUTO_APPROVE=1`。**

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 否 | `.` | 儲存庫目錄。 |
| `remote` | string | 否 | `origin` | 遠端名稱。 |
| `branch` | string | 否 | `HEAD` | 要推送的分支參照（ref）。 |

**範例：**
```json
{"remote": "origin", "branch": "main"}
```

**回傳：** `git push` 的合併 stdout/stderr，若未自動核准則回傳 `APPROVAL_REQUIRED: ...`。

---

### `git_reset`

重設工作樹。Hard 重設**需要 `SPECTYN_AUTO_APPROVE=1`**。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `mode` | string | 否 | `soft` | 重設模式：`"soft"`、`"mixed"` 或 `"hard"`。 |
| `path` | string | 否 | `.` | 儲存庫目錄。 |

**範例：**
```json
{"mode": "soft"}
```

**回傳：** `Reset complete.` 或錯誤／需核准字串。

---

### `git_blame`

顯示誰最後修改了檔案的每一行。輸出截斷至 100 行。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 是 | — | 要做 blame 的「檔案」（非儲存庫目錄）。 |
| `repo` | string | 否 | `.` | 儲存庫目錄。 |

**範例：**
```json
{"path": "core/src/agent.rs", "repo": "/workspace/myproject"}
```

**回傳：** `git blame` 輸出（至多 100 行），若更長則附 `... (output truncated to 100 lines)` 通知。

---

### `git_show`

顯示某次提交的詳細資訊：統計，並可選顯示完整 diff。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `ref_` | string | 否 | `HEAD` | 提交參照（hash、tag、分支等）。 |
| `stat_only` | boolean | 否 | false | 若為 true，僅顯示 `--stat` 輸出（不含完整 diff）。 |
| `path` | string | 否 | `.` | 儲存庫目錄。 |

**範例 — 顯示最後一次提交及其 diff：**
```json
{"ref_": "HEAD"}
```

**範例 — 僅顯示特定提交的統計：**
```json
{"ref_": "abc1234", "stat_only": true}
```

**回傳：** `git show` 的輸出。

---

### `git_branch_list`

列出分支及詳細資訊（最後一次提交的 hash 與訊息）。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 否 | `.` | 儲存庫目錄。 |
| `remote` | boolean | 否 | false | 若為 true，包含遠端追蹤（remote-tracking）分支（`git branch -av`）。 |

**範例：**
```json
{"remote": true}
```

**回傳：** 分支清單輸出或 `No branches found.`

---

### `git_checkout`

切換到既有分支或建立新分支。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `branch` | string | 是 | — | 目標分支名稱。 |
| `create` | boolean | 否 | false | 若為 true，建立該分支（`git checkout -b`）。 |
| `path` | string | 否 | `.` | 儲存庫目錄。 |

**範例 — 切換分支：**
```json
{"branch": "feature/new-tool"}
```

**範例 — 建立並切換：**
```json
{"branch": "feature/new-tool", "create": true}
```

**回傳：** `git checkout` 的合併 stdout/stderr。

---

### `git_stash_list`

列出儲存庫中所有暫存堆疊（stash）。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 否 | `.` | 儲存庫目錄。 |

**範例：**
```json
{}
```

**回傳：** `git stash list` 輸出或 `No stashes.`

---

### `fetch`

抓取一個 URL 並以可讀文字回傳其內容。對 HTML 而言，會移除 script/style/nav/footer/header 標籤、移除註解、將標題與區塊元素轉為換行，並解碼 HTML 實體（entity）。JSON 回應會被美化（pretty-print）。僅接受 `http://` 與 `https://`；私有／回送（loopback）IP 會被封鎖。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `url` | string | 是 | — | 必須以 `http://` 或 `https://` 開頭。最大長度 2000 字元。 |
| `timeout_secs` | integer | 否 | 15 | 請求逾時（秒）。 |
| `max_length` | integer | 否 | 8000 | 回傳的最大字元數（硬上限 50,000）。超出部分以 `[... truncated]` 取代。 |
| `raw` | boolean | 否 | false | 若為 true，回傳未經移除處理的原始 HTML／文字。 |
| `selector` | string | 否 | — | HTML 標籤名稱提示（例如 `"article"`、`"main"`），將擷取範圍縮小至第一個符合的元素。 |

**支援的內容類型：** `text/html`、`text/plain`、`application/json`。其他內容類型會回傳錯誤。

**封鎖：** `localhost`、`::1`、`127.x.x.x`、`10.x.x.x`、`172.16-31.x.x`、`192.168.x.x`、`169.254.x.x`。

**範例 — 抓取並清理文件頁面：**
```json
{"url": "https://docs.rs/tokio/latest/tokio/", "selector": "main", "max_length": 10000}
```

**範例 — 抓取原始 JSON API：**
```json
{"url": "https://api.github.com/repos/rust-lang/rust/releases/latest", "max_length": 5000}
```

**回傳：** 對 HTML 為 `Title: ...\nURL: ...\n---\n<cleaned content>`，或美化後的 JSON，或錯誤字串。

---

### `http_get`

原始 HTTP GET 請求。不做 HTML 清理 — 回傳狀態碼、Content-Type 標頭與原始內文。內文於 8000 字元處截斷。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `url` | string | 是 | — | 目標 URL。 |
| `timeout_secs` | integer | 否 | 30 | 請求逾時。 |
| `headers` | object | 否 | `{}` | 以鍵值對形式的額外 HTTP 標頭。 |

**範例：**
```json
{
  "url": "https://api.example.com/status",
  "headers": {"Authorization": "Bearer <token>", "Accept": "application/json"}
}
```

**回傳：** `HTTP 200 OK\nContent-Type: application/json\n---\n<body>` 或 `ERROR: HTTP 404 Not Found\nURL: ...`

---

### `http_post`

原始 HTTP POST 請求。送出 JSON 內文或純文字內文。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `url` | string | 是 | — | 目標 URL。 |
| `body` | any JSON value | 否 | — | JSON 內文（自動設定 `Content-Type: application/json`）。 |
| `body_text` | string | 否 | — | 純文字內文（設定 `Content-Type: text/plain`）。若 `body` 與 `body_text` 皆缺，則送出空內文。 |
| `timeout_secs` | integer | 否 | 30 | 請求逾時。 |
| `headers` | object | 否 | `{}` | 額外 HTTP 標頭。 |

**範例 — JSON 內文：**
```json
{
  "url": "https://api.example.com/create",
  "body": {"name": "agent-x", "role": "worker"},
  "headers": {"Authorization": "Bearer <token>"}
}
```

**範例 — 純文字內文：**
```json
{"url": "https://webhook.example.com/ping", "body_text": "hello"}
```

**回傳：** 與 `http_get` 相同格式。

---

### `ls`

列出目錄內容。先列目錄（依字母順序），再列檔案（依字母順序）。支援詳細格式與樹狀檢視。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 否 | `.` | 要列出的目錄。 |
| `long` | boolean | 否 | false | 若為 true，顯示權限、大小與修改日期（如 `ls -l`）。 |
| `tree` | boolean | 否 | false | 若為 true，呈現最多 3 層深的樹狀檢視。 |
| `hidden` | boolean | 否 | false | 若為 true，包含隱藏檔案與目錄（名稱以 `.` 開頭者）。 |
| `max_entries` | integer | 否 | 200 | 最多顯示的項目數；若超出則附上截斷通知。 |

**範例 — 簡單列表：**
```json
{"path": "core/src"}
```

**範例 — 詳細格式：**
```json
{"path": "core/src", "long": true}
```

**範例 — 樹狀檢視：**
```json
{"path": "core", "tree": true}
```

**回傳：** 項目名稱（目錄附上 `/` 後綴）。詳細格式欄位：`permissions  size  date  name`。樹狀格式使用 Unicode 製表（box-drawing）字元。

---

### `ls_stat`

取得單一檔案或目錄的詳細中繼資料（metadata）。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 是 | — | 檔案或目錄路徑。 |

**範例：**
```json
{"path": "core/src/agent.rs"}
```

**回傳：**
```
Path:     /abs/path/to/file.rs
Type:     file
Size:     4096 bytes (4.0 KB)
Modified: 2026-04-24 10:30:00 UTC
Created:  2026-01-01 00:00:00 UTC
Perms:    644
Lines:    120
```
（行數僅對小於 1 MB 的文字檔顯示。）

---

### `list_files`

遞迴列出目錄下的所有檔案（最多 15 層深，最多 500 筆結果）。自動略過 `node_modules`、`.git`、`target`、`.next`、`dist`、`__pycache__`、`.cache`。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 否 | `.` | 基底目錄。 |
| `pattern` | string | 否 | `""` | 簡單名稱篩選。支援 `prefix*`、`*suffix`、`*middle*` 或完全比對。 |

**範例 — 所有檔案：**
```json
{"path": "core/src"}
```

**範例 — 僅 Rust 檔案：**
```json
{"path": "core/src", "pattern": "*.rs"}
```

**回傳：** 每行一個檔案路徑，或 `No files found`。

---

### `list_dir`

列出單一目錄層級，附上項目名稱與大小／類型註記。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 否 | `.` | 要列出的目錄。路徑穿越（`..`）會被封鎖。 |

**範例：**
```json
{"path": "core/src/tools"}
```

**回傳：** 類似 `agent.rs (8192 bytes)`、`tools/ (dir)` 的行，依字母順序排序。於 10,000 字元處截斷。

---

### `create_dir`

建立目錄及所有必要的上層目錄（等同 `mkdir -p`）。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 是 | — | 要建立的目錄路徑。路徑穿越（`..`）會被封鎖。 |

**範例：**
```json
{"path": "core/src/tools/new_module"}
```

**回傳：** `Created directory: <abs-path>` 或錯誤字串。

---

### `rename_file`

重新命名或移動檔案。**需要 `SPECTYN_AUTO_APPROVE=1`。**

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `src` | string | 是 | — | 來源檔案路徑。 |
| `dst` | string | 是 | — | 目標路徑。 |

任一引數中的路徑穿越（`..`）皆會被封鎖。

**範例：**
```json
{"src": "core/src/old_name.rs", "dst": "core/src/new_name.rs"}
```

**回傳：** `Renamed: <src> -> <dst>` 或 `APPROVAL_REQUIRED: ...`

---

### `delete_file`

刪除單一檔案。拒絕刪除目錄或大於 10 MB 的檔案。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 是 | — | 要刪除的檔案。路徑穿越（`..`）會被封鎖。 |

**範例：**
```json
{"path": "core/src/old_module.rs"}
```

**回傳：** `Deleted: <path>` 或錯誤字串。

---

### `patch`

將統一差異格式修補檔（例如來自 `git diff` 或 `diff_files`）套用到一個或多個檔案。寫入前會驗證區塊（hunk）的上下文。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `patch` | string | 是 | — | 完整的統一差異文字（可含多個 `--- / +++` 檔案區段與多個 `@@` 區塊）。 |
| `base_dir` | string | 否 | 目前目錄 | 用於解析 diff 中相對檔案路徑的基底目錄。 |
| `dry_run` | boolean | 否 | false | 若為 true，描述將會變更的內容但不修改任何檔案。 |

**修補檔格式：** 由 `git diff`、`diff -u` 或 `diff_files` 產生的標準統一差異。支援 `+++ b/path` 與 `+++ path` 前綴。

**範例：**
```json
{
  "patch": "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,3 @@\n fn main() {\n-    println!(\"hello\");\n+    println!(\"world\");\n }\n",
  "base_dir": "/workspace/myproject"
}
```

**回傳：** `Applied N hunk(s) to M file(s). Modified: file1, file2\n\nPatched <path> (N hunks)` 或逐檔的錯誤細節。失敗時會回報區塊上下文不符的細節。

---

### `multi_file_edit`

在單一原子操作中跨多個檔案套用多個精確字串替換。所有替換會先經驗證；若任一驗證失敗，則不更動任何檔案。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `edits` | array of objects | 是 | — | 編輯規格清單。每個物件須有 `path`（string）、`old_string`（string）與 `new_string`（string）。 |
| `dry_run` | boolean | 否 | false | 若為 true，回報將會變更的內容但不修改檔案。 |

每個 `old_string` 必須在其檔案中**恰好比對一次** — 零次或多次比對皆會導致整批失敗且不寫入。

**範例：**
```json
{
  "edits": [
    {"path": "src/agent.rs", "old_string": "version = \"1.0\"", "new_string": "version = \"2.0\""},
    {"path": "Cargo.toml", "old_string": "version = \"1.0\"", "new_string": "version = \"2.0\""}
  ]
}
```

**回傳：** `Applied N edit(s):\n  <path>: replaced '<old>' → '<new>'\n  ...` 或 `Validation failed — no changes were made:\nERROR: ...`

---

### `diff_files`

使用 Myers diff 演算法產生兩個檔案間的統一差異。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path_a` | string | 是 | — | 第一個檔案路徑（顯示為 `a/path_a`）。 |
| `path_b` | string | 是 | — | 第二個檔案路徑（顯示為 `b/path_b`）。 |
| `context_lines` | integer | 否 | 3 | 每個變更區塊周圍的上下文行數。 |

**範例：**
```json
{"path_a": "src/agent.rs.bak", "path_b": "src/agent.rs", "context_lines": 5}
```

**回傳：** 統一差異文字（`--- a/...\n+++ b/...\n@@ ... @@\n...`）或 `Files are identical`。於 5000 字元處截斷。

---

### `diff_strings`

產生兩個字串間的統一差異（不讀取檔案）。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `a` | string | 是 | — | 第一個字串。 |
| `b` | string | 是 | — | 第二個字串。 |
| `label_a` | string | 否 | `"a"` | diff 標頭中第一個字串的標籤。 |
| `label_b` | string | 否 | `"b"` | diff 標頭中第二個字串的標籤。 |
| `context_lines` | integer | 否 | 3 | 每個變更區塊周圍的上下文行數。 |

**範例：**
```json
{
  "a": "fn foo() {}\n",
  "b": "fn bar() {}\n",
  "label_a": "before",
  "label_b": "after"
}
```

**回傳：** 與 `diff_files` 相同格式。

---

### `cargo_check`

執行 `cargo check --message-format=short` 並摘要結果。若不支援該旗標則退回純 `cargo check`。逾時：120 秒。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 否 | `.` | 含有 `Cargo.toml` 的目錄（manifest 目錄）。 |
| `package` | string | 否 | — | 要檢查的特定套件名稱（傳入 `--package <name>`）。 |

**範例：**
```json
{"path": "core"}
```

**回傳：** 成功時：`✓ cargo check passed (N warnings)`。失敗時：`cargo check failed:\n<error lines>`（於 5000 字元處截斷）。

---

### `cargo_test`

以 `--nocapture` 執行 `cargo test` 並摘要通過／失敗數。逾時：120 秒。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 否 | `.` | Manifest 目錄。 |
| `filter` | string | 否 | — | 測試名稱篩選（子字串比對，作為位置引數傳給 cargo test）。 |
| `package` | string | 否 | — | 要測試的特定套件。 |

**範例 — 執行所有測試：**
```json
{"path": "core"}
```

**範例 — 執行特定測試：**
```json
{"path": "core", "filter": "test_parse_compound"}
```

**回傳：** `N passed, M failed` 摘要。失敗時，列出失敗的測試名稱與輸出（於 3000 字元處截斷）。

---

### `tsc_check`

以型別檢查模式（`--noEmit`）執行 TypeScript 編譯器。先嘗試 `tsc`，再退回 `npx tsc`。逾時：120 秒。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `path` | string | 否 | `.` | 專案目錄（`tsconfig.json` 所在處）。 |
| `config` | string | 否 | — | 明確的 tsconfig 路徑（傳入 `--project <config>`）。 |

**範例：**
```json
{"path": "app"}
```

**回傳：** `✓ TypeScript check passed` 或 `TypeScript errors:\n<error lines>`（於 5000 字元處截斷）。

---

### `run_tests`

執行任意測試命令（例如 `pytest`、`jest`、`go test ./...`）並回傳輸出。逾時：120 秒。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `command` | string | 是 | — | 完整測試命令，例如 `"pytest tests/ -v"`。 |
| `path` | string | 否 | `.` | 命令的工作目錄。 |

**範例：**
```json
{"command": "pytest tests/ -x --tb=short", "path": "/workspace/myproject"}
```

**回傳：** `Tests completed successfully.\n<output>` 或 `Tests finished with failures.\n<output>`（於 5000 字元處截斷）。

---

### `task_add`

將一項狀態為 `todo` 的新任務加入工作階段任務清單。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `description` | string | 是 | — | 人類可讀的任務說明。 |
| `session` | string | 否 | `"default"` | 工作階段名稱（任務按工作階段儲存於 `~/.spectyn-mesh/tasks/<session>.json`）。 |

**範例：**
```json
{"description": "Refactor authentication module", "session": "sprint-42"}
```

**回傳：** `Added task #N: <description>`

---

### `task_update`

依 ID 更新任務的狀態。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `id` | integer | 是 | — | 任務 ID（由 `task_add` 或 `task_list` 回傳）。 |
| `status` | string | 是 | — | 新狀態：`"todo"`、`"in_progress"` 或 `"done"`。 |
| `session` | string | 否 | `"default"` | 工作階段名稱。 |

**範例：**
```json
{"id": 3, "status": "in_progress", "session": "sprint-42"}
```

**回傳：** `Task #3 marked as in_progress` 或 `Error: task #3 not found`。

---

### `task_list`

列出某工作階段的任務，可選依狀態篩選。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `session` | string | 否 | `"default"` | 工作階段名稱。 |
| `status_filter` | string | 否 | — | 篩選為 `"todo"`、`"in_progress"` 或 `"done"`。 |

**範例 — 列出所有任務：**
```json
{"session": "sprint-42"}
```

**範例 — 僅列出未完成任務：**
```json
{"session": "sprint-42", "status_filter": "todo"}
```

**回傳：** `Tasks (N total, M done):\n  #1 [todo] description\n  #2 [done] description\n...` 或 `No tasks found.`

---

### `task_clear`

從工作階段清單移除任務。

| 參數 | 型別 | 必填 | 預設 | 說明 |
|---|---|---|---|---|
| `session` | string | 否 | `"default"` | 工作階段名稱。 |
| `done_only` | boolean | 否 | false | 若為 true，僅移除狀態為 `"done"` 的任務；否則移除全部任務。 |

**範例 — 清除已完成任務：**
```json
{"session": "sprint-42", "done_only": true}
```

**回傳：** `Cleared N task(s)`

---

## 最佳實務

### 選對讀取工具

| 目標 | 偏好工具 |
|---|---|
| 讀取特定檔案 | `file_read` |
| 依名稱尋找檔案 | `glob_search` |
| 依內容尋找檔案 | `content_search` |
| 瀏覽目錄樹 | 帶 `tree: true` 的 `ls` |
| 遞迴列出所有檔案 | `list_files` |
| 檢查檔案中繼資料 | `ls_stat` |

### 安全地編輯檔案

1. 在編輯檔案前一律先以 `file_read` 讀取，以了解目前內容並挑選唯一的 `old_string`。
2. 對於精準變更，偏好 `file_edit` 而非 `file_write` — 它會保留替換區域以外的一切，並產生 diff 供驗證。
3. 當你需要跨多個檔案以原子方式套用協調一致的變更時（例如重新命名一個在多處使用的函式），使用 `multi_file_edit`。
4. 當你已有完整的統一差異時（例如來自程式碼審查或先前的 `diff_files` 呼叫），使用 `patch`。
5. 只有在你確定每一處出現都應變更時，才在 `file_edit` 中使用 `replace_all: true`。否則請提供更多周邊上下文，使 `old_string` 唯一。

### 使用 `line_range` 限定編輯範圍

當某字串在檔案中多次出現時，以 `line_range` 縮小搜尋：
```json
{
  "path": "src/config.rs",
  "old_string": "let timeout = 30;",
  "new_string": "let timeout = 60;",
  "line_range": {"start": 45, "end": 55}
}
```

### Git 工作流程

典型的提交循環：
1. 以 `file_edit` / `file_write` / `multi_file_edit` 做變更。
2. 以 `cargo_check` 或 `tsc_check` 驗證。
3. 以 `git_add` 暫存。
4. 以 `git_commit` 提交。

### 記憶命名空間

使用命名空間以避免代理或工作階段間的鍵衝突：
```json
{"key": "task_queue", "value": "[1,2,3]", "namespace": "agent_coordinator"}
```

以相同命名空間取回：
```json
{"key": "task_queue", "namespace": "agent_coordinator"}
```

### Shell 安全

- 有專用工具時（`git_add`、`file_write` 等），偏好它們而非 `shell` — 它們有內建驗證並產生結構化輸出。
- 當你必須以 `shell` 執行破壞性命令時，在環境中設定 `SPECTYN_AUTO_APPROVE=1`，否則該命令會回傳 `APPROVAL_REQUIRED`。
- 使用 `cwd` 而非 `cd && ...` 串接 — 反正工作目錄在每次 shell 呼叫間都會重設。
- 使用 `env` 傳遞機密，而非將其嵌入命令字串中。

### 抓取網路內容

- 對人類可讀的頁面（文件、部落格文章）使用 `fetch` — 它會移除導覽外框（chrome）並回傳乾淨文字。
- 對需要原始 JSON 或特定標頭的結構化 API 呼叫，使用 `http_get` / `http_post`。
- 若 `fetch` 發生截斷，提高 `max_length`（最多 50,000）或以 `selector` 縮小範圍。
- `fetch` 會封鎖私有／回送 IP；對內部服務改用 `http_get`（它沒有 IP 過濾）。

---

## 工具串接範例

### 範例 1：找到一個函式、讀取上下文、編輯它

```
1. content_search: {"pattern": "fn authenticate", "path": "core/src", "file_type": "rs"}
   → core/src/auth.rs:42: pub fn authenticate(

2. file_read: {"path": "core/src/auth.rs", "offset": 38, "limit": 25}
   → 讀取第 38–62 行以了解函式簽章與函式本體

3. file_edit: {
     "path": "core/src/auth.rs",
     "old_string": "pub fn authenticate(token: &str) -> bool {",
     "new_string": "pub fn authenticate(token: &str, timeout_ms: u64) -> bool {"
   }

4. cargo_check: {"path": "core"}
   → 驗證該編輯可編譯
```

### 範例 2：從 diff 套用修補檔，然後提交

```
1. diff_files: {"path_a": "src/config.rs.orig", "path_b": "src/config.rs"}
   → 產生統一差異

2. patch: {"patch": "<diff text>", "base_dir": "/workspace"}
   → 套用該 diff

3. git_add: {"files": ["src/config.rs"]}

4. git_commit: {"message": "chore: update config defaults"}
```

### 範例 3：研究 → 抓取文件 → 實作

```
1. web_search: {"query": "tokio spawn_blocking documentation"}
   → [1] spawn_blocking in tokio::task — URL: https://docs.rs/tokio/...

2. fetch: {"url": "https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html", "selector": "main"}
   → 清理後的文件文字

3. file_read: {"path": "core/src/agent.rs", "offset": 1, "limit": 30}
   → 檢查既有的 import

4. file_edit: { ... 加入 spawn_blocking 用法 ... }

5. cargo_check: {"path": "core"}
```

### 範例 4：含驗證的多檔重新命名

```
1. content_search: {"pattern": "JobStore", "path": "core/src"}
   → 列出每個使用 JobStore 的檔案

2. multi_file_edit: {
     "edits": [
       {"path": "core/src/session.rs", "old_string": "JobStore", "new_string": "TaskStore"},
       {"path": "core/src/lib.rs",     "old_string": "JobStore", "new_string": "TaskStore"},
       {"path": "core/src/agent.rs",   "old_string": "JobStore", "new_string": "TaskStore"}
     ]
   }
   → 原子操作：全部成功或全部不更動

3. cargo_check: {"path": "core"}
```

### 範例 5：以任務追蹤長時間工作

```
1. task_add: {"description": "Implement streaming tool", "session": "sprint-5"}
   → Added task #1

2. task_update: {"id": 1, "status": "in_progress", "session": "sprint-5"}

3. ... 進行實作工作 ...

4. cargo_test: {"path": "core", "filter": "streaming"}
   → 3 passed, 0 failed

5. task_update: {"id": 1, "status": "done", "session": "sprint-5"}

6. task_list: {"session": "sprint-5"}
   → Tasks (1 total, 1 done): #1 [done] Implement streaming tool
```

### 範例 6：背景工作與狀態檢查

```
1. shell_bg: {"command": "cargo build --release", "label": "release-build"}
   → Job started: PID=45678 label='release-build'

2. ... 進行其他工作 ...

3. shell_bg_check: {"pid": 45678}
   → PID 45678 (release-build): finished

4. shell: {"command": "ls -lh target/release/spectyn-agent"}
   → -rwxr-xr-x 1 user group 12M Apr 24 10:30 target/release/spectyn-agent
```
