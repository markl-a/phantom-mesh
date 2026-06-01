# 為 Phantom Mesh 做出貢獻

感謝你有意願為 Phantom Mesh 做出貢獻！

在進行架構（architecture）或產品定調（product-shaping）的變更之前，請先閱讀：

- `AGENTS.md`
- `docs/ARCHITECTURE-FREEZE.md`
- `docs/ACTIVE-STATUS.md`

## 開發環境設定

### 先決條件（Prerequisites）

- **Rust 穩定版工具鏈（stable toolchain）**（透過 [rustup](https://rustup.rs/) 安裝）
- **SQLite**（已隨 Rust crate 一併打包，無需額外安裝）
- **選用**：[Ollama](https://ollama.ai) 用於本機 LLM（大型語言模型）測試

### 建置（Building）

```bash
cd core
cargo build
```

### 執行測試

```bash
cd core
cargo test --lib          # Unit tests (~2,700 tests, ~55s)
cargo test --lib -q       # Quiet mode
cargo test --lib "module" # Run tests for a specific module
```

### 執行常駐程式（Daemon）

```bash
cd core
cargo run -- health              # Check system health
cargo run -- init                # First-time setup
cargo run -- daemon              # Start daemon on port 7878
cargo run -- run "hello world"   # Single prompt execution
```

### 自我測試（`phantom selftest`）

`phantom selftest` 會執行內建的端對端（end-to-end）冒煙檢查（smoke checks）。每項功能
都在 `scripts/selftest.d/` 之下隨附自己的檢查檔，因此在開啟 PR（Pull Request，拉取請求）之前，
這是確認全新建置行為正確的最快方式。

```bash
cd core
cargo run -- selftest            # Run the full self-test suite

# Or, with an installed binary on your PATH:
phantom selftest
```

當你新增一項功能時，請在 `scripts/selftest.d/` 加入一個對應的檢查檔
（複製 `scripts/selftest.d/_template.sh` 作為起點），
讓該功能持續被測試套件（suite）涵蓋。

## 本機開發工作流程

以下所有內容都只需在一台僅安裝 Rust 工具鏈的機器上執行——
不需要叢集（cluster）、網路或外部帳號。典型的
PR 前迴圈如下：

```bash
cd core

# 1. Build the core crate
cargo build

# 2. Format — keep diffs minimal
cargo fmt --all

# 3. Lint — must be clean before a PR
cargo clippy -- -D warnings

# 4. Run the test suite (unit tests are the quickest signal)
cargo test --lib

# 5. End-to-end smoke check
cargo run -- selftest
```

若 `cargo fmt`、`cargo clippy -- -D warnings`、`cargo test --lib` 與
`cargo run -- selftest` 全部通過（綠燈），你的變更就可以推送了。這些
與 CI（持續整合，Continuous Integration）所執行的檢查相同，因此本機乾淨地跑過通常意味著 PR 也會是綠燈。

## 專案結構

```
phantom-mesh/
├── core/                  # Rust daemon (main codebase)
│   ├── src/
│   │   ├── lib.rs         # Library entry point
│   │   ├── main.rs        # CLI wrapper
│   │   ├── runtime.rs     # PhantomMesh::init() API
│   │   ├── agent_runtime.rs  # Multi-round tool-calling loop
│   │   ├── events/        # DomainEvent spine + persistence
│   │   ├── providers/     # LLM providers (Ollama, Claude, OpenAI, etc.)
│   │   ├── tools/         # Agent tools (shell, file, web search, etc.)
│   │   └── ...
│   └── tests/             # Integration tests
├── crates/
│   └── pm-types/          # Shared type definitions
├── app/                   # Tauri v2 desktop app (React)
└── .github/workflows/     # CI pipelines
```

## 程式碼風格

- 遵循標準 Rust 慣例（`cargo clippy` 必須通過）
- 單元測試（unit tests）放在同一個檔案內的 `#[cfg(test)] mod tests` 之下
- 整合測試（integration tests）放在 `core/tests/`
- 所有公開的 enum（列舉型別）都使用 `#[non_exhaustive]`

## Pull Request 流程

1. Fork（分叉複製）這個儲存庫
2. 從 `main` 建立一個功能分支（feature branch）
3. 進行變更並附帶測試
4. 確保 `cargo test --lib` 通過
5. 確保 `cargo clippy -- -D warnings` 通過
6. 提交一個附有清楚說明的 pull request

### 分支與提交（Commit）慣例

- **分支名稱**以類型前綴描述變更，例如
  `feat/ollama-streaming`、`fix/event-replay-offset`、`docs/contributing`、
  `chore/dep-bump`。
- **提交（Commits）**遵循 [Conventional Commits](https://www.conventionalcommits.org/)（約定式提交）：
  `type(scope): summary`。常見類型有 `feat`、`fix`、`docs`、`test`、
  `refactor` 與 `chore`。範例：`fix(events): correct replay offset`。
- 讓每個 PR 聚焦於單一邏輯變更，且絕對不要提交
  機密（secrets）、API 金鑰或個人資料——在範例與測試中
  請使用佔位字串（placeholders），例如 `your-api-key` 或 `127.0.0.1`。

## 授權條款（License）

提交貢獻即表示你同意你的貢獻將以 MIT OR Apache-2.0 授權條款進行授權。
