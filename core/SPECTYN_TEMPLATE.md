# 專案：{PROJECT_NAME}

## 概覽（Overview）

{簡述這個專案做什麼、解決什麼問題。
請包含：語言/執行環境（runtime，執行環境）、主要進入點（entry point，程式啟動入口）、以及核心使用情境。}

範例：
> my-service 是一個 Rust HTTP API，負責處理來自 GitHub 的 webhook（網路鉤子）事件，並
> 將其分派到內部的 worker queue（工作佇列）。它以單一執行檔（binary）的形式在
> Linux 上運行，透過一個 TOML 檔案進行設定。

## 建置與測試（Build & Test）

```bash
# Compile-check (run after every source edit — fast, no linking)
{check command}
# Example: cargo check  |  tsc --noEmit  |  pylint src/

# Full build
{build command}
# Example: cargo build  |  npm run build  |  go build ./...

# Release / production build
{release build command}
# Example: cargo build --release  |  npm run build -- --mode production

# Run all tests
{test command}
# Example: cargo test  |  npm test  |  pytest  |  go test ./...

# Lint / format check
{lint command}
# Example: cargo clippy  |  npm run lint  |  ruff check .
```

**規則：** 每次修改程式碼後，務必執行 `{check command}`，並確認
其結束碼（exit code）為 0 之後才能提交（commit）。

## 關鍵檔案（Key Files）

```
{entry_point}               — {description, e.g. "main server entry point"}
{config_file}               — {description, e.g. "project config loaded at startup"}
{core_module}               — {description, e.g. "primary business logic"}
{test_directory}/           — {description, e.g. "integration and unit tests"}
{docs_directory}/           — {description, e.g. "architecture and deployment docs"}
```

範例：
```
src/main.rs                 — Axum HTTP server; defines routes in build_router()
src/lib.rs                  — Public API surface, re-exports, shared state
src/config.rs               — Config structs loaded from agents.toml
src/handlers/               — One file per route group
Cargo.toml                  — Package manifest; add dependencies here
tests/                      — Integration tests
docs/                       — Architecture and deployment documentation
```

## 代理指示（Agent Instructions）

- 每次修改程式碼後務必執行 `{check command}`，以便及早抓到錯誤。
- 編輯檔案前先讀取它——絕不要猜測檔案內容。
- 修改既有檔案時，使用精確字串替換（exact-string replacement）編輯，而非整份重寫。
- 為新功能撰寫測試，並在提交前執行 `{test command}`。
- 為提交挑選特定檔案進行 stage（暫存）——不要使用 `git commit -am` 或 `git add .`。
- 使用 conventional commit（慣例式提交）訊息：`feat:`、`fix:`、`chore:`、`docs:`、`refactor:`。
- 先搜尋再假設——編輯前先用內容搜尋或 glob（檔名萬用字元）搜尋來定位定義與使用處。
- {project-specific rule 1}
- {project-specific rule 2}

## 架構（Architecture）

{用 3 到 8 個項目符號或一張簡短的示意圖描述高階結構。
聚焦於各元件之間的邊界，而非實作細節。}

範例：

```
HTTP Layer (handlers/)
    |
    v
Service Layer (services/)      — business logic, no HTTP types
    |
    v
Storage Layer (db/ / store/)   — database, cache, file persistence
```

關鍵設計決策：
- {Decision 1 and why it was made}
- {Decision 2 and why it was made}
- {Decision 3 and why it was made}

## 設定（Configuration）

設定檔位置與格式：
```
{path to config file}       — {description}
{path to example/template}  — copy this to get started
```

重要設定欄位：
- `{field}` — {what it does, required vs optional}
- `{field}` — {what it does, required vs optional}

**安全性：** {關於密鑰（secret）處理的說明——例如「API 金鑰請使用環境變數，絕不要把密鑰提交進設定檔。」}

## 新增 {Components}（Adding New {Components}）

當你要新增一個 {tool / route / module / service}（工具／路由／模組／服務）時：

1. 建立 `{path}/{name}.{ext}` 並寫入實作。
2. 在 `{registration file}` 中註冊它——{specific instructions}。
3. 在 `{test location}` 加上測試。
4. 執行 `{check command}` 與 `{test command}` 來驗證。
5. {Any other required steps}

**陷阱（Gotcha）：** {新增這類元件時的常見錯誤，以及如何避免。}

## 已知陷阱（Known Gotchas）

- {Gotcha 1}：{explanation and how to avoid or fix it}
- {Gotcha 2}：{explanation and how to avoid or fix it}
- {Gotcha 3}：{explanation and how to avoid or fix it}

範例：
- **狀態必須可 Clone（State must be Clone）：** `AppState` 是以傳值（by value）方式傳給 handler 的；每個欄位
  都必須實作 `Clone`（可變狀態請用 `Arc<Mutex<_>>` 包起來）。
- **註冊表的兩端都要改（Both sides of the registry）：** 新增工具時，dispatch table（分派表）
  與 schema definitions（結構定義）都要更新——少改任一邊都會導致無聲的失敗。
- **密鑰用環境變數（Env vars for secrets）：** 絕不要把 API 金鑰寫死在設定檔裡；一律使用
  環境變數參照。

## 測試策略（Testing Strategy）

- {單元測試位置以及它們涵蓋的範圍}
- {整合測試位置以及它們涵蓋的範圍}
- {如何執行測試的子集以快速迭代}
- {提交 PR（pull request，合併請求）前所需的任何手動測試步驟}

範例：
- 單元測試：`src/**/*_test.{ext}` — 純邏輯，無 I/O
- 整合測試：`tests/` — 含真實資料庫（DB）的完整堆疊（需先執行 `docker compose up`）
- 快速迭代：`{test command} -- {module_name}` 可只執行單一模組的測試
- 提交 PR 前：執行完整測試套件，並檢查沒有 snapshot（快照）檔案被意外更動

## 安全性注意事項（Security Notes）

- {密鑰處理政策}
- {驗證／存取控制（auth / access control）注意事項}
- {程式碼中任何已知的敏感區域}
- {關於哪些內容不應記錄（log）的指引}
