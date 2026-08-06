# 權限 DSL（領域特定語言）

spectyn-mesh 內建一套 Claude-Code 風格的 **Tool(specifier)**（工具（指定符））規則語法，
用來把關工具執行。透過 `agents.toml` 裡的 `[permissions]` 設定；
由代理執行期（agent runtime）的工具派發路徑強制執行。

真相來源（source of truth）：[`core/src/permission.rs`](../core/src/permission.rs)。
6 個 fuzz（模糊測試）守護解析器（parser）與引擎，避免遭遇輸入時崩潰（panic-on-input）
（`fuzz_parse_rule_never_panics` 等）。

---

## 快速參考

```toml
[permissions]
deny  = ["Read(./.env)", "Read(./secrets/*)", "WebFetch(domain:badsite.com)"]
ask   = ["Bash", "Edit"]
allow = ["Bash(git status)", "Bash(cargo check)", "Read(./README.md)", "100:Bash(git push origin feature/*)"]
```

三個清單 → 三種動作。區塊為空／缺失 ⇒ 全部允許（allow-all，沿用舊版
預設）。只要有任一規則存在 ⇒ 未命中的呼叫一律落入 **Ask（詢問）**。

---

## 語法

```text
RULE        := [PRIORITY ":"] TOOL [ "(" SPECIFIER ")" ]
PRIORITY    := signed integer (default 0; higher beats lower)
TOOL        := PascalCase name | snake_case name | "*"
SPECIFIER   := tool-specific glob/string (see below)
```

| 形式 | 範例 | 效果 |
|---|---|---|
| 純工具名稱 | `Bash` | 命中每一次 shell（命令列）呼叫 |
| 工具 + 指定符 | `Bash(npm run *)` | 命中 `command` 與 `npm run *` 以 glob（萬用字元比對）相等的 shell 呼叫 |
| 萬用工具 | `*` | 命中每一次工具呼叫 |
| 優先級前綴 | `100:Bash(git status)` | 與 `Bash(git status)` 相同的比對，但 priority=100 |

### 工具名稱別名（與 Claude Code 對齊）

| 你寫的 | spectyn 命中的 |
|---|---|
| `Bash` 或 `Shell` | `shell` |
| `Read` | `file_read` |
| `Write` | `file_write` |
| `Edit` | `file_edit`、`file_write`、`multi_file_edit`、`apply_patch`（編輯家族合併） |
| `WebFetch` | `web_fetch` |
| `WebSearch` | `web_search` |
| 其他任何名稱 | 原樣透傳——若你偏好可直接寫 `shell` |

### 各工具的指定符形狀

| 工具 | 指定符語法 | 比對對象 |
|---|---|---|
| `Bash` / `Shell` | `cmd-pattern with *` | `command` 引數；`*` 命中任意字元 |
| `Read` / `Write` / `Edit` | `path-glob with *` | `path` 引數 |
| `WebFetch` | `domain:host.com`（或僅 `host.com`） | URL 主機（host）（子網域比對：`github.com` ⇒ 也包含 `api.github.com`） |
| 其他任何名稱 | 後備（fallback）：對 `path` / `cmd` / `url` / 序列化後的引數做子字串比對 | 第一個非空者勝出 |

指定符是**錨定的（anchored）**（必須命中整個引數）——但網域比對例外，
它會感知主機後綴（host-suffix-aware）。

---

## 評估順序

規則依 `(user_priority, action_precedence)` **遞減**排序：

1. **數值優先級較高者勝出**。`100:Bash(git status)` 勝過
   預設優先級的 `Bash`。
2. 在**相同優先級**之間：deny > ask > allow。
3. **首次命中者勝出（first match wins）**——規則清單依排序後的順序由上而下
   掃描；第一個命中該 (tool, args) 組合的規則
   產生決策。
4. **無命中** ⇒ 若引擎含有任何規則，則落入
   `Decision::Ask`。若引擎為空，則落入
   `Decision::Allow`（保留舊版 `SPECTYN_PERM=allow` 行為）。

這與 Claude Code 文件記載的順序一致（deny → ask → allow，
首次命中者勝出），**外加**透過優先級欄位提供的逃生口（escape-hatch）。
沒有優先級你就無法在允許 `git status` 的同時封鎖其他每一個
shell 呼叫——spectyn 的引擎特別為此案例加入了優先級。

---

## Bash 加固：重導向／串接自動降級

Bash 是最危險的工具介面——單一個重導向（`>`、
`>>`、`|`、`<`）或串接運算子（`;`、`&&`、`||`）就能把一個「已允許」的
命令變成可能洩漏檔案的東西。spectyn 的引擎
在命中的命令含有上述任一運算子時，會**自動把** `Allow` 決策
降級為 `Ask`。

```toml
[permissions]
allow = ["Bash(cat *)"]
```

```bash
# Allowed:  cat README.md           → Decision::Allow
# Allowed:  cat ./.env              → Decision::Allow  (path-glob ignored at this level — see Read rules)
# DOWNGRADED: cat secrets > /tmp/x  → Decision::Ask    (redirect detected)
# DOWNGRADED: cat README.md | wc    → Decision::Ask    (pipe detected)
# DOWNGRADED: cat a; rm b           → Decision::Ask    (chain detected)
```

被引號包住的運算子會被**尊重**——`echo 'a > b'` 不會觸發。
實作：`bash_has_redirect_or_chain()` 逐字元走訪命令，
正確處理單引號、雙引號與反斜線轉義（backslash escapes）。

若你刻意想允許重導向（例如 `Bash(tee
./logs/*.log)`），請使用更精確的樣式 + 提高優先級：
```toml
allow = ["100:Bash(tee ./logs/*)"]
```
重導向降級規則只套用在*命中的*命令上，因此
精確樣式能在高優先級下重新選擇加入（opt back in）。

---

## 靜態封鎖的工具

一個**無指定符**的 `Deny` 規則（例如 `WebFetch`）表示「此工具
被全面封鎖——在任何引數下都永不可呼叫」。引擎透過
`Engine::statically_denied_tools()` 暴露這些工具。代理執行期的
`run_with_callbacks_gated()` 會把它們從 LLM（大型語言模型）的工具清單
結構（schema）中**過濾掉**，因此模型永遠不會提出一個它無法執行的工具。

```toml
[permissions]
deny = ["WebFetch", "WebSearch"]
```
⇒ LLM 根本不知道 `web_fetch` 與 `web_search` 存在。不必每回合
做「allow」+「deny」的拉鋸戲——模型維持在範圍內。

條件式封鎖（例如 `Bash(rm -rf *)`）**不會**靜態
排除 `Bash`——它們只在命中的引數上觸發，因此該工具仍留在
結構（schema）中。

---

## 範例——常見策略

### 「個人開發模式」（最寬鬆）

```toml
[permissions]
deny = ["Read(./.env)", "Read(./secrets/*)"]
allow = ["*"]
```
除了碰觸機密外允許一切。無任何提示。

### 「生產謹慎」（寫入前一律詢問）

```toml
[permissions]
deny  = ["Read(./.env)", "Bash(rm -rf *)"]
ask   = ["Edit", "Write", "Bash"]
allow = [
  "Bash(git status)", "Bash(git diff)", "Bash(git log *)",
  "Bash(cargo check)", "Bash(cargo build)", "Bash(cargo test)",
  "Read(./*)",
]
```
唯讀操作 + 安全的 git／cargo 命令靜默執行。任何
會變更檔案或執行新穎 shell 的呼叫都會提示。

### 「CI 自動封鎖 shell」

```toml
[permissions]
deny  = ["Bash"]
ask   = []
allow = ["Read(./*)", "Edit(./src/**)"]
```
最緊的沙箱（sandbox）：完全沒有 shell，可讀取原始碼，僅能編輯
`./src/` 底下的檔案。

---

## 診斷

`spectyn doctor` 包含一個 `[permissions]` 區段，顯示：
- 已解析的規則數量
- 靜態封鎖的工具清單（那些 LLM 看不到的工具）
- 每條規則的解析錯誤（若有）

```
permissions
  ✓ [permissions]: 4 rules parsed (2 deny, 1 ask, 1 allow)
  ✓ statically denied: web_fetch (will be hidden from LLM tool list)
```

若 `spectyn doctor` 顯示 `parse error: unterminated specifier in rule
"Bash(unclosed"` 之類訊息，問題的那一行會被原樣指名，讓你能
修正 `agents.toml` 後重跑。

---

## 舊版 `SPECTYN_PERM` 環境變數

當引擎回傳 `Decision::Ask`（無規則命中，預設狀態）時，
DSL 之前的行為被保留為後備（fallback）：

| 環境變數值 | 效果 |
|---|---|
| `allow`（預設） | 引擎 `Ask` ⇒ 允許 |
| `ask` | 引擎 `Ask` ⇒ 互動式 y/n 提示 |
| `deny` | 引擎 `Ask` ⇒ 拒絕 |
| `diff` | 引擎 `Ask` ⇒ 對 file_edit 渲染統一差異（unified diff），然後提示 |

一旦你的 `[permissions]` 規則涵蓋了真實案例，就把
`SPECTYN_PERM=ask` 設好，讓未命中的呼叫跳出提示而非
靜默允許——那正是新策略漏洞浮現之處。

---

## 實作指標

- 解析器：`permission::parse_rule(s, action)` → `Vec<Rule>`（當別名
  展開時一次回傳多條規則，例如 `Edit(...)` 為 4 個編輯家族工具
  回傳 4 條規則）。
- 引擎：`permission::Engine::from_lists(deny, ask, allow)` →
  `Engine`；內部自行排序；`engine.evaluate(tool, args)` →
  `Decision`。
- 輔助函式：`wildcard_match(pat, text)`、`bash_segments(cmd)`、
  `bash_has_redirect_or_chain(cmd)`、`host_matches(host, url)`。
- 測試：`core/src/permission.rs` 中有 26 個單元測試 + 6 個 fuzz。
