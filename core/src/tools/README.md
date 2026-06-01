# `core/src/tools/` — 工具註冊表（Tool Registry）

這個目錄收納了 agent loop（代理迴圈）能呼叫的每一個**工具（tool）**：行程內
（in-process）內建工具（檔案/shell/web/git/搜尋/記憶體/…），以及讓
外部 MCP 伺服器與未來外掛（plugin）能透過同一個介面呈現工具的鷹架（scaffolding）。

如果 LLM（大型語言模型）發出名為 `"shell"`、`"file_read"`、`"git_status"`
等的工具呼叫，就會落在這裡。這份 README 說明檔案佈局、dispatch（派發）流程、
[`trait_def.rs`](trait_def.rs) 中的 `Tool` trait 契約，以及如何新增一個
工具。

---

## 目前 dispatch 如何運作

現行（live）的註冊表**不是**一份 `dyn Tool` 物件清單（目前還不是 — 見
[鷹架路徑 vs. 現行路徑](#鷹架路徑-vs-現行路徑)）。它是
[`mod.rs`](mod.rs) 中的三個自由函式（free function）：

| 函式 | 用途 |
| --- | --- |
| `all_tool_names() -> Vec<&'static str>` | 每一個已註冊的工具名稱。支撐 `/tools`、help 輸出，以及現行工具包裝（live tool wrapping）。受平台閘控（platform-gated，見 [平台閘控](#平台閘控)）。 |
| `execute(name, args, config) -> String` | 派發器（dispatcher）。MCP 優先（MCP-first），接著是一個大的 `match name { … }`，將呼叫路由到同層模組（sibling module）中的某個自由函式。 |
| `schema(name) -> Option<Value>` | 回傳 OpenAI 風格（OpenAI-style）的 `{"type":"function","function":{…}}` 信封（envelope），會被拼接（splice）進 LLM 請求的 `tools=[…]` 欄位。 |

`execute` 路由順序：

1. **MCP 優先。** 若某個 MCP（Model Context Protocol — model 上下文協議）伺服器
   以 `<server>_<tool>` 前綴註冊了一個工具，呼叫就會送往那裡。
2. **內建 match。** 否則 `match name` 分支會轉發給其所屬的
   模組（例如 `"shell" => shell::run(args)`）。

要註冊一個新的內建工具，三處都得動到：把名稱加進
`all_tool_names()`、在 `execute()` 加一個 `match` 分支、在
`schema()` 加一個 `match` 分支。（逐步教學見 [新增一個工具](#新增一個工具)。）

---

## `Tool` trait（[`trait_def.rs`](trait_def.rs)）

`trait_def.rs` 定義了**未來的外掛介面（plugin surface）**。每一個具名工具 —
內建、MCP 或第三方外掛 — 都能被呈現為單一的
`&dyn Tool`，因此 agent loop 不論來源為何都能對同一種形狀（shape）溝通。

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    /// Name the LLM calls. Built-ins use bare names ("shell", "file_read");
    /// MCP tools are namespaced "<server>_<tool>" to avoid collisions.
    fn name(&self) -> &str;

    /// OpenAI-style {"type":"function","function":{…}} envelope, or None
    /// for internal-only tools that should not be advertised to the model.
    fn schema(&self) -> Option<Value>;

    /// Invoke. Tools should be cancel-aware (a CancellationToken in the
    /// context is a planned extension).
    async fn call(&self, args: &Value, ctx: &ToolContext<'_>) -> String;
}
```

`ToolContext<'a>` 是交給每次呼叫的唯讀（read-only）上下文。今天它
帶有 `config: &ToolsConfig`；可以新增欄位而不破壞既有實作（impl），
因為工具是依名稱借用（borrow by name）的。

### 兩個參考實作（reference impl）

- **`BuiltinTool`** 包裝一個內建名稱，並在內部呼叫
  `tools::execute(name, args, config)` — 即原本的舊有路徑（legacy path）— 因此行為
  與 match 派發完全相同。它的 `schema()` 委派給 `super::schema`。
- **`McpToolWrapper`** 包裝一個帶前綴的外部工具。它的 `call()` 透過
  `mcp_client::global()` 派發，並在註冊表缺失或沒有 client 匹配前綴時，
  回傳清楚的錯誤字串（絕不 panic）。

### 建立現行清單

```rust
pub async fn live_tools() -> Vec<Box<dyn Tool>>
```

把 `all_tool_names()` 的每個名稱包進 `BuiltinTool`，接著把
`mcp_client::global().tool_defs()` 的每個 MCP 工具以 `McpToolWrapper` 附加上去。未來的
外掛來源也會在此附加。以 `Box<dyn Tool>` 回傳，因此呼叫者永遠不必
區分來源。

### 鷹架路徑 vs. 現行路徑

這個 trait 刻意是一個**鷹架（scaffold）**。`execute()` 已經是 MCP 優先、
接著內建的路由方式，所以 trait 的存在並不改變端對端（end-to-end）行為。這個
trait 為未來的工作（外掛載入器、hook 生命週期、跨對等節點（cross-peer）工具路由）
提供一個單一形狀作為目標。把那個巨大的 `match` 遷移為完整的 `dyn Tool`
派發，是一項刻意的後續工作（follow-up）— 兩者同時做會帶來大量
侵入式變動（invasive churn），而使用者可見的價值卻很有限。設計參考自 Codex 的
`ToolHandler` 與 claurst-rust 的 `Tool` trait。

---

## 模組目錄（Module catalogue）

### 工具實作

| 模組 | 暴露的工具 | 功能 |
| --- | --- | --- |
| [`file.rs`](file.rs) | `file_read`、`file_write`、`file_edit` | 讀取 / 寫入 / 精準編輯（surgical-edit）檔案。沙箱安全（Sandbox-safe，透過 `safe_path` 可在 iOS app 容器內運作）。 |
| [`multi_edit.rs`](multi_edit.rs) | `multi_file_edit` | 一次呼叫即跨多個檔案套用多項編輯。 |
| [`patch.rs`](patch.rs) | `apply_patch` | 套用 unified-diff 風格的修補（patch）。 |
| [`search.rs`](search.rs) | `content_search`、`glob_search` | Ripgrep 風格的內容搜尋與 glob 檔案匹配。 |
| [`ls.rs`](ls.rs) | `ls`、`stat` | 列出目錄；對路徑做 stat。 |
| [`shell.rs`](shell.rs) | `shell` | 執行一個 shell 命令。會 spawn 子行程（Subprocess-spawning）⇒ 在 iOS 上被移除。 |
| [`bash_bg.rs`](bash_bg.rs) | `bash_run_background`、`bash_output`、`bash_kill` | 長時間執行的背景行程：啟動、輪詢（poll）輸出、終止（kill）。受 iOS 閘控（iOS-gated）。 |
| [`git.rs`](git.rs) | `git_status`、`git_diff`、`git_log`、`git_commit`、`git_branch_list`、`git_checkout`、`git_show`、`git_blame`、`git_add`、`git_stash_list` | 透過 `git` 二進位檔執行 Git 操作。受 iOS 閘控。 |
| [`diagnostic.rs`](diagnostic.rs) | `cargo_check`、`cargo_test`、`tsc_check`、`run_tests` | 建置 / 型別檢查（typecheck）/ 測試執行器。需要工具鏈（toolchain）⇒ 受 iOS 閘控。 |
| [`web.rs`](web.rs) | `web_search` | 網路搜尋（使用來自 `ToolsConfig` 的供應商設定）。 |
| [`web_fetch.rs`](web_fetch.rs) | `web_fetch` | 抓取一個 URL 並把 HTML 轉換成文字。 |
| [`http_client.rs`](http_client.rs) | `http_get`、`http_post` | 原始（raw）HTTP 請求。 |
| [`fetch.rs`](fetch.rs) | （fetch 工具的輔助程式） | 共用的 fetch 實作 / 字元數上限。 |
| [`memory.rs`](memory.rs) | `memory_store`、`memory_recall`、`memory_list`、`memory_delete`、`memory_search` | Agent 的長期記憶層。 |
| [`todo.rs`](todo.rs) | `todo_add`、`todo_update`、`todo_list`、`todo_clear` | Agent 內部的 TODO 清單。 |
| [`diff_view.rs`](diff_view.rs) | `diff_files`、`diff_strings` | 產生檔案之間 / 字串之間的差異（diff）。 |
| [`ask_user.rs`](ask_user.rs) | `ask_user` | 暫停 agent 並向人類提問。 |
| [`subagent.rs`](subagent.rs) | `task`、`subagent`、`parallel_tasks` | 生成（spawn）另一個已設定的 agent；`parallel_tasks` 會扇出（fan out）。 |
| [`cluster.rs`](cluster.rs) | `cluster_status`、`cluster_sessions`、`cluster_peers` | 唯讀的「誰可連線 / 在線」資訊，讓 agent 可為 `task`/`parallel_tasks` 挑選一個 `node:` 目標。 |
| [`diag.rs`](diag.rs) | `diag_read` | 自我內省（Self-introspection）— 讀取 phantom 自身的診斷狀態。 |
| [`image_gen.rs`](image_gen.rs) | `image_generate` | 生成影像。 |
| [`video_gen.rs`](video_gen.rs) | `video_generate` | 生成影片。 |
| [`music_gen.rs`](music_gen.rs) | `music_generate` | 生成音樂 / 音訊。 |
| [`computer_use_win.rs`](computer_use_win.rs) | `screen_capture`、`mouse_click`、`keystroke` | 操控真實桌面。**僅限 Windows**（`#[cfg(target_os = "windows")]`）。 |
| [`spotlight.rs`](spotlight.rs) | `spotlight_search` | Spotlight 索引搜尋。**僅限 macOS**。 |
| [`xcode.rs`](xcode.rs) | `xcode_simctl` | 透過 `simctl` 操控 iOS 模擬器（Simulator）。**僅限 macOS**。 |

### Trait + 管路（plumbing）

| 模組 | 角色 |
| --- | --- |
| [`trait_def.rs`](trait_def.rs) | `Tool` trait、`ToolContext`、`BuiltinTool`、`McpToolWrapper`、`live_tools()`。 |
| [`mod.rs`](mod.rs) | 模組宣告 + 註冊表函式 `all_tool_names`、`execute`、`schema`，外加 `truncate`/`floor_char_boundary` 輸出輔助程式。 |

### 共用的安全 / 驗證輔助程式（本身不是工具）

這些模組是被上述工具*所*呼叫，用來強制執行安全政策（security policy）— 它們
不暴露任何自己的工具名稱：

| 模組 | 角色 |
| --- | --- |
| [`urlguard.rs`](urlguard.rs) | 為所有對外 HTTP 工具提供共用的 SSRF（server-side request forgery，伺服器端請求偽造）防護 — 封鎖 loopback（回送）/ 私有 IP / link-local（鏈結本地）主機。`PHANTOM_FETCH_ALLOW_LOCAL=1` 可在經過稽核的工作流程中選擇加入（opt in）本地主機。 |
| [`env_filter.rs`](env_filter.rs) | 過濾合併進工具所生成子行程（例如 `shell`、`bash_run_background`）的 `env: { K: V }` 對應表，以封鎖動態連結器注入（dynamic-linker injection）（`LD_*`、`DYLD_*`、…）。 |
| [`validate.rs`](validate.rs) | 為 `shell`/`bash_bg`/`git`/`search` 提供共用的輸入驗證 — 包含黑名單繞過（blocklist-bypass）、git 選項注入（option-injection），以及 `rg`/`grep` 旗標注入（flag-injection）的防禦。 |
| [`fs.rs`](fs.rs) | 檔案系統輔助程式（路徑安全的列舉），由 file/ls 工具共用。 |

---

## 平台閘控

`all_tool_names()`、`execute()` 與 `schema()` 全都使用 `#[cfg(target_os = …)]`，
因此註冊表會因平台而異：

- **iOS** — 移除每一個需要 `fork`/`exec` 的工具（iOS 沙箱禁止
  這類操作）：`shell`、所有 `git_*`、`cargo_*`/`tsc_check`/`run_tests` 診斷工具，
  以及 `bash_*`。mesh（網狀網路）的 `required_caps` 過濾器會改把這些任務路由到一個
  非 iOS 的對等節點（peer）。
- **Windows** — 新增 `screen_capture`、`mouse_click`、`keystroke`。
- **macOS** — 新增 `spotlight_search`、`xcode_simctl`。

> 注意：`vec!` 不會像 `match` 分支那樣對行內元素（inline element）尊重
> `#[cfg(...)]`，因此平台專屬的名稱是在基礎 `vec!`
> 建好之後才被 `v.push(...)` 加入的。新增受閘控工具時請維持這個模式。

---

## 新增一個工具

要新增一個名為 `my_tool` 的內建工具：

1. **撰寫實作。** 新增 `my_tool.rs`（或在既有的
   同層檔案中加一個函式），帶有一個 `async fn run(args: &Value) -> String`。若它會接觸網路或
   生成行程，請重用 `urlguard` / `env_filter` / `validate`。
2. **宣告模組** 於 [`mod.rs`](mod.rs)：`pub mod my_tool;`（若它是平台專屬的，
   就加上 `#[cfg(target_os = …)]`）。
3. **註冊名稱** 於 `all_tool_names()` — 放在基礎 `vec!` 中，或作為
   受閘控的 `v.push("my_tool")`。
4. **新增派發分支** 於 `execute()`：`"my_tool" => my_tool::run(args).await,`
   （以 `#[cfg(...)]` 閘控，使其與註冊一致）。
5. **新增 schema 分支** 於 `schema()`，回傳 OpenAI 風格的 function
   信封，讓模型知道參數。只有對於不應對外公告（advertise）的內部專用工具，
   才回傳 `None`。
6. **留意輸出大小。** 用 `tools::truncate(...)` 包裝大型結果，以免單一
   工具呼叫撐爆上下文視窗（context window）。

一旦某個新工具進入 `all_tool_names()`，它就會自動透過 `live_tools()`（經由
`BuiltinTool`）浮現 — 無需變更 `trait_def.rs`。

> `trait_def.rs` 中的參考實作（`BuiltinTool`、`McpToolWrapper`）有針對
> 經 `execute` 的往返（round-trip）、MCP 錯誤路徑的訊息傳遞，以及
> `Box<dyn Tool>` 的物件安全性（object safety）做單元測試。重構此 trait 時請保持那些測試為綠燈。
