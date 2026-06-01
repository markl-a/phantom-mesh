# MCP Registry（MCP 註冊中心）提交準備 — phantom-mesh

**狀態：** 草稿 — 尚未提交。最後的 `mcp-publisher publish` 步驟（見 §5）
必須由 operator（操作者，markl-a）親自執行，因為向第三方
registry（註冊中心）提交是一項 operator 決策，不是自動化動作。

**目標 registry（註冊中心）：** <https://registry.modelcontextprotocol.io>
（由 Model Context Protocol 組織維護的官方 MCP server（伺服器）註冊中心；
背後由公開的 repo（程式碼倉庫）
<https://github.com/modelcontextprotocol/registry> 支撐）。

**Namespace（命名空間）認領：** `io.github.markl-a/phantom-mesh` — 透過
GitHub OAuth（開放授權）以使用者 `markl-a` 的身分擁有（也就是公開鏡像站
<https://github.com/markl-a/phantom-mesh> 的擁有者）。

---

## 1. phantom-mesh 透過 MCP 對外暴露什麼

真實來源（source of truth）：
[`core/src/mcp.rs`](../core/src/mcp.rs) 與
[`core/src/tools/mod.rs`](../core/src/tools/mod.rs)。

實作備註：

- Transport（傳輸方式）：stdio（標準輸入輸出）（`phantom mcp` — 在
  stdin/stdout 上以換行分隔的 JSON-RPC 2.0）。也由 `phantom serve` 在
  `POST /mcp` 上透過 HTTP 對外暴露（見 `core/src/serve.rs:142`），因此同一套
  工具集可透過 `server.json` 中的 `remotes.streamable-http` 形狀存取。
- Protocol（協議）版本：`2024-11-05`。
- Capabilities（能力）：僅 `tools`（無 `resources`、無 `prompts`，
  `listChanged: false`）。
- Server（伺服器）身分：`serverInfo.name = "phantom-mesh"`，
  `serverInfo.version = CARGO_PKG_VERSION`（目前為 `0.4.0`）。
- 支援的方法：`initialize`、`notifications/initialized`、
  `tools/list`、`tools/call`、`ping`。

### 1.1 透過 `tools/list` 暴露的工具

這份清單來自 `tools::all_tool_names()`（以 `#[cfg]` 依平台閘控），再加上
由 `mcp.rs::handle` 附加的兩個分散式工具：

| 類別 | 工具名稱 |
|---|---|
| **File I/O（檔案輸入輸出）**（沙箱安全，iOS 上也包含） | `file_read`, `file_write`, `file_edit` |
| **Search（搜尋）** | `content_search`, `glob_search` |
| **Web（網路）** | `web_search`, `web_fetch`, `http_get`, `http_post` |
| **Memory（記憶體，持久化、有命名空間）** | `memory_store`, `memory_recall`, `memory_list`, `memory_delete`, `memory_search` |
| **Directory（目錄）** | `ls`, `stat` |
| **Patch（修補）** | `apply_patch` |
| **Todos（agent 內任務清單）** | `todo_add`, `todo_update`, `todo_list`, `todo_clear` |
| **Multi-file edit（多檔編輯）** | `multi_file_edit` |
| **Diff（差異比對）** | `diff_files`, `diff_strings` |
| **Interactive（互動）** | `ask_user` |
| **Subagent orchestration（子代理編排）** | `task`, `subagent`, `parallel_tasks` |
| **Cluster awareness（叢集感知，唯讀）** | `cluster_status`, `cluster_sessions`, `cluster_peers` |
| **Self-introspection（自我內省）** | `diag_read` |
| **Shell（外殼，非 iOS）** | `shell` |
| **Git（版本控制，非 iOS）** | `git_status`, `git_diff`, `git_log`, `git_commit`, `git_branch_list`, `git_checkout`, `git_show`, `git_blame`, `git_add`, `git_stash_list` |
| **Diagnostics（診斷，非 iOS）** | `cargo_check`, `cargo_test`, `tsc_check`, `run_tests` |
| **Background bash（背景 bash，非 iOS）** | `bash_run_background`, `bash_output`, `bash_kill` |
| **僅 macOS** | `spotlight_search`, `xcode_simctl` |
| **分散式（於 `mcp.rs` 附加）** | `phantom_swarm`, `phantom_evolve_distributed` |

總計：40 + 2 = 在功能完整的 macOS 主機上有 **42 個工具**；
iOS 上約 30 個；Linux/Windows 上約 40 個。

Schema（結構描述）位於 `core/src/tools/mod.rs::schema()`，採用 OpenAI
function-calling（函式呼叫）封套形狀（`{type:"function", function:{name,
description, parameters}}`）；`core/src/mcp.rs:212` 中的 `to_mcp_tool()` 會在
`tools/list` 時把它們重新包裝成 MCP 的 `{name, description, inputSchema}`
形狀。有一個測試（`all_tools_have_mcp_schema`）保證每個已註冊的工具都有
schema，因此隨著我們新增工具，回傳給 registry 的清單仍會保持準確。

### 1.2 phantom-mesh（目前）尚未暴露什麼

- 沒有 MCP **resources（資源）**（無 `resources/list`、無 `resources/read`）。
- 沒有 MCP **prompts（提示）**（無 `prompts/list`）。
- 沒有 `notifications/tools/list_changed` — 工具表面在行程啟動時即固定。

在 `server.json._meta` 中誠實地列出這些，可避免消費端（consumer）期待
我們不提供的功能。

---

## 2. Registry 的提交流程（已於 2026-05-15 驗證）

來源：<https://github.com/modelcontextprotocol/registry/tree/main/docs>。

**它_不是_以 PR（拉取請求）為基礎的提交。** 此 registry 是一個由 API
支撐的服務。你執行一個 CLI（`mcp-publisher`），它會：

1. 為你進行身分驗證（GitHub OAuth device flow（裝置流程）是最簡單的途徑）。
2. 在本地依照已發布的 schema 驗證你的 `server.json`。
3. 驗證 `server.json.name` 中的 **namespace（命名空間）** 與已驗證的身分相符
   （例如 `io.github.markl-a/...` 需要以 `markl-a` 身分登入 GitHub）。
4. 在適用時驗證 **套件擁有權**（例如 `npm` 套件的 `package.json` 中必須
   含有 `"mcpName"`；`mcpb` 發行版必須位於你所掌控的 GitHub repo 中）。
5. 把 manifest（清單）以 POST 送至 registry API。

沒有人工審核佇列、也沒有公開的 SLA（服務水準協議）— 一旦擁有權檢查通過，
被接受的提交會在數分鐘內出現在公開清單中。也沒有正式文件化的審查政策；
此 registry 依賴 namespace 擁有權 + 受限的套件 registry 來源來防止冒名。

### 2.1 允許的套件來源（registry 端的允許清單）

| `registryType` | 允許的 `registryBaseUrl` |
|---|---|
| `npm`    | 僅 `https://registry.npmjs.org` |
| `pypi`   | 僅 `https://pypi.org` |
| `nuget`  | 僅 `https://api.nuget.org/v3/index.json` |
| `oci`    | Docker Hub、GHCR、Quay.io、GAR、ACR、MCR |
| `mcpb`   | 僅 GitHub Releases 與 GitLab Releases |

phantom-mesh 目前並未發布至 npm/PyPI/NuGet 任何一者。目前可行的途徑為：

- **`mcpb` + GitHub release** 交叉編譯後的二進位封存檔
  （這是建議的近期途徑）。
- **`oci`** 當（若）有一個 `phantom mcp` Docker 映像被推送至 GHCR 時。
- 供自行架設 `phantom serve` 的使用者使用的 **`remotes`** 項目。

### 2.2 Metadata（中繼資料）上限

`server.json` 上的 `_meta` 欄位會在伺服器端被過濾。只有
`_meta["io.modelcontextprotocol.registry/publisher-provided"]` 會被保留。
該子物件的 JSON 上限為 **4 KB**；超量的提交會失敗。

### 2.3 Schema 參考

`$schema = "https://static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json"`

在發布前使用 `mcp-publisher validate server.json` 來捕捉 schema 問題。

---

## 3. phantom-mesh 的 `server.json` 草稿

草稿位於 [`docs/mcp-registry/server.json`](mcp-registry/server.json)。
關鍵抉擇：

- `name = "io.github.markl-a/phantom-mesh"` — 以使用者 `markl-a` 透過
  GitHub OAuth 認領該 namespace。
- `version = "0.4.0"` — 與 `core/Cargo.toml::version` 一致。每次發行時
  同步遞增。
- `repository.url = "https://github.com/markl-a/phantom-mesh"` —
  公開鏡像站，**不是**私有 repo。Registry 會驗證該 URL 可連線。
- `repository.subfolder = "core"` — phantom-mesh 是工作區（workspace）中
  位於 `core/` 之下的 Rust crate（套件箱）。
- 一個 `mcpb` 類型的 `packages[0]` 項目，指向尚未發布的 GitHub release
  tarball（壓縮封存檔）。
  - `transport.type = "stdio"`、`runtimeHint = "phantom"`、
    `packageArguments = [{type:"positional", value:"mcp"}]`。
  - `fileSha256` 是一個 `TODO_FILL_AFTER_RELEASE_BUILD` 佔位符 —
    必須是實際上傳至 release 的 `.mcpb` 產物的 SHA-256。若雜湊值與 GitHub
    提供的不符，`mcp-publisher publish` 會拒絕該 manifest。
- 一個 `streamable-http` 類型的 `remotes[0]` 項目，帶有樣板化的
  `url = "https://{host}/mcp"`，供自行架設的 `phantom serve` 使用。
- `_meta.io.modelcontextprotocol.registry/publisher-provided` 攜帶
  類別標籤、工具數量、授權條款、支援的平台與亮點 — 遠低於 4 KB 上限。

---

## 4. Operator 在發布_之前_必須完成的先決條件

這些不在本 PR 的範圍內。在此列出是為了讓 operator 知道需要事先備齊什麼：

1. **發布一個公開的 GitHub release。** 撰寫此文時，`markl-a/phantom-mesh`
   尚無任何 release。本次提交參照了
   `releases/download/v0.4.0/phantom-mcpb-v0.4.0.mcpb`，它必須在
   `mcp-publisher publish` 成功之前確實存在。

   `.mcpb`（MCP Bundle，MCP 套裝包）tarball 的建議結構：

   ```
   phantom-mcpb-v0.4.0.mcpb     # gzip tar of:
     phantom-mesh-aarch64-apple-darwin/phantom
     phantom-mesh-x86_64-apple-darwin/phantom
     phantom-mesh-x86_64-unknown-linux-gnu/phantom
     phantom-mesh-aarch64-unknown-linux-gnu/phantom
     phantom-mesh-x86_64-pc-windows-msvc/phantom.exe
     manifest.json   # mcpb metadata (see mcpb spec)
   ```

   擷取 SHA-256 並把它修補進 `server.json.packages[0].fileSha256`。

2. **決定 namespace。** `io.github.markl-a/phantom-mesh` 是目前的草稿。
   若專案日後擁有自己的 GitHub org（組織）
   （例如 `phantom-mesh-org`），namespace 會變成
   `io.github.phantom-mesh-org/phantom-mesh`，且 GitHub 認證必須以該 org
   成員的身分進行。

3. **（選用）新增一個 `oci` 套件**，一旦有 `phantom mcp` 的 Docker 映像
   發布至 GHCR。草圖：

   ```jsonc
   {
     "registryType": "oci",
     "registryBaseUrl": "https://ghcr.io",
     "identifier": "ghcr.io/markl-a/phantom-mesh:0.4.0",
     "transport": { "type": "stdio" },
     "packageArguments": [
       { "type": "positional", "value": "mcp" }
     ]
   }
   ```

4. **（選用）README 擁有權標記。** 對於非 npm 套件，registry 目前要求
   發布者在 repo README 的某處放入一行像是
   `mcp-name: io.github.markl-a/phantom-mesh`，好讓擁有權檢查有東西可
   grep（比對）。在發布前把它加進公開鏡像站 `markl-a/phantom-mesh` 的
   `README.md`。

---

## 5. 逐步提交說明

在 §4 步驟完成後，從公開的 `markl-a/phantom-mesh` 檢出（checkout）執行這些
（**不要**從這個私有 repo 執行）。

### 5.1 安裝 `mcp-publisher`

**macOS / Linux（curl）：**
```bash
curl -L \
  "https://github.com/modelcontextprotocol/registry/releases/latest/download/mcp-publisher_$(uname -s | tr '[:upper:]' '[:lower:]')_$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/').tar.gz" \
  | tar xz mcp-publisher
sudo mv mcp-publisher /usr/local/bin/
```

**macOS（Homebrew）：**
```bash
brew install mcp-publisher
```

**Windows：** 從 <https://github.com/modelcontextprotocol/registry/releases/latest>
下載 `mcp-publisher_windows_amd64.tar.gz`，並把 `mcp-publisher.exe` 解壓縮到
你的 PATH（路徑）上。

### 5.2 以 `markl-a` 身分驗證

```bash
mcp-publisher login github
```

這會印出一個裝置碼（device code）與一個 URL（`https://github.com/login/device`）。
貼上該碼，授權「MCP Registry Publisher」OAuth 應用程式，CLI 便會在
`~/.config/mcp-publisher/token.json` 之下取得一個 token（權杖）。

### 5.3 把草稿 manifest 複製進公開 repo

從公開的檢出：

```bash
# in github.com/markl-a/phantom-mesh checkout
curl -L https://raw.githubusercontent.com/markl-a/phantom-mesh/feat/mcp-registry/docs/mcp-registry/server.json \
  -o server.json
```

…或從本 PR 中的 `docs/mcp-registry/server.json` 手動複製。

### 5.4 修補 SHA-256 + 版本

在執行 release 建置之後：

```bash
sha256sum phantom-mcpb-v0.4.0.mcpb
# paste the hex into server.json.packages[0].fileSha256
```

若你把 `core/Cargo.toml` 升到 0.4.0 以上，也要一併遞增
`server.json.version` 與 `identifier` URL。

### 5.5 在本地驗證（免費、無副作用）

```bash
mcp-publisher validate server.json
```

修正回報的每一個錯誤。常見的有：

- `$schema mismatch` — schema URL 被升版了；從文件頁面複製最新的。
- `registryType not in allow-list` — 使用了自訂的 registry URL。
- `fileSha256 mismatch` — 重建 release tarball 後重新計算雜湊。
- `_meta size > 4096 bytes` — 精簡 `highlights` 陣列。

### 5.6 發布

```bash
mcp-publisher publish server.json
```

CLI 會印出新 registry 項目的正規（canonical）URL。預期不到一分鐘即可完成。
該項目會出現在：

`https://registry.modelcontextprotocol.io/v0/servers/io.github.markl-a/phantom-mesh`

### 5.7 （未來）遞增版本

對於每一個後續的 phantom-mesh release：

```bash
# bump server.json version + identifier + fileSha256
mcp-publisher publish server.json
```

發布器會附加一個新的版本項目；舊版本仍可透過 API 查詢。

### 5.8 （未來）棄用 / 撤回

```bash
mcp-publisher status deprecated --message "Use 0.5.x — 0.4.0 has a known mcp crash"
# or
mcp-publisher status deleted --all-versions
```

---

## 6. 審核者期待與 SLA

**沒有人工審核佇列，也沒有公開的 SLA**。Registry 依自動化的 namespace +
套件擁有權檢查運作；若這些通過，清單會在約 1 分鐘內上線。維護者也未文件化
任何審查或下架 SLA。

社群支援管道（依 registry 的 `docs/README.md`）：

- GitHub Discussions：<https://github.com/modelcontextprotocol/registry/discussions>
- Discord：從 MCP org 的 README 連出
- 錯誤回報：<https://github.com/modelcontextprotocol/registry/issues>

Registry API 本身處於 **v0.1 凍結（freeze）** 狀態（於 2025-10-24 宣告，
穩定性承諾：自凍結起一個月）。把 `2025-12-11` schema URL 當作目前的固定版本
（pin）— 若已過了數個月，在發布前重新確認。

---

## 7. 待解問題 / 留給 operator 的決策

這些並非本 PR（僅為準備）的阻擋因素，但在 `mcp-publisher publish` 執行前
會需要一個是/否的答覆：

1. **以 v0.4.0 作為首個清單，還是等 v0.5.0？**
   v0.5.0 是已宣布的發行（依 v0.5.0 發行計畫備忘錄）。今天就列出 v0.4.0
   能讓 phantom-mesh 更早進入 PulseMCP 式的索引；等一個標籤則意味著第一印象
   是打磨過的那個。

2. **`io.github.markl-a/phantom-mesh` 是正確的 namespace 嗎，還是我們現在
   就想搬到一個 org，以避免日後一次性的改名？**

3. **是否在 mcpb tarball 之外也提供一個 OCI 映像？** 沒有它，
   「用 Docker 執行」的情境就需要使用者在本地自行建置。

4. **`_meta.publisher-provided` 裡要保留什麼？** 目前草稿使用約 600 位元組；
   上限是 4 KB。若 registry 日後有呈現這些內容，還有空間可放截圖 URL、
   一個示範 GIF，或一個 `pricing: "free, self-hosted"` 旗標。

---

## 8. 本 PR 中的檔案

| 路徑 | 用途 |
|---|---|
| `docs/mcp-registry-submission.md`（本檔） | 提交準備備忘錄 |
| `docs/mcp-registry/server.json` | 草稿 `server.json`，待 operator 填入 SHA-256 並發布 release 後即可用於 `mcp-publisher publish` |

無原始碼變更。尚未進行任何 registry 提交。
