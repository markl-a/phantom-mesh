# Aider 架構文檔

## 專案概覽

Aider 是一款 **Python 開發的 AI 配對編程工具**，為開發者提供終端環境中的 AI 編程協助。與其他 coding agent 不同，Aider 採用交互式對話模式，讓開發者在終端中與 LLM 進行即時協作。

**核心定位**：
- 終端優先的 AI 配對編程工具
- 支援多種編輯格式（Whole File、Edit Blocks、Unified Diff 等）
- 實時 Git 整合與 Repository 理解
- 支援 40+ 個 LLM 提供商（OpenAI、Claude、Gemini、Ollama 等）

**技術棧**：
- 語言：Python 3.x
- UI 框架：prompt_toolkit（終端 UI）
- LLM 集成：LiteLLM（統一的 LLM 接口）
- 版本控制：GitPython
- 配置：YAML/TOML

---

## 目錄結構

```
aider/
├── aider/
│   ├── main.py                    # 主入口點，CLI 啟動邏輯
│   ├── llm.py                     # LLM 配置與初始化
│   ├── models.py                  # 模型信息管理
│   ├── coders/                    # 核心編碼引擎（P0）
│   │   ├── base_coder.py          # Coder 基類，統一接口
│   │   ├── editblock_coder.py     # Edit Block 編輯格式
│   │   ├── wholefile_coder.py     # Whole File 編輯格式
│   │   ├── udiff_coder.py         # Unified Diff 格式
│   │   ├── patch_coder.py         # Patch 格式
│   │   ├── architect_coder.py     # 架構規劃層
│   │   ├── *_prompts.py           # 各類型 Coder 的 system prompt
│   │   └── search_replace.py      # 搜尋替換引擎
│   ├── repo.py                    # Git 倉庫管理
│   ├── repomap.py                 # Repository 代碼地圖生成
│   ├── commands.py                # CLI 命令處理
│   ├── io.py                      # 輸入/輸出管理
│   ├── linter.py                  # 程式碼 linter 整合
│   ├── history.py                 # 對話歷史與摘要
│   ├── analytics.py               # 使用統計與遠程報告
│   ├── utils.py                   # 工具函數
│   ├── prompts.py                 # prompt 範本與生成
│   ├── watch.py                   # 文件變化監視
│   ├── voice.py                   # 語音輸入支援（P2）
│   └── gui.py                     # GUI 模式（早期）
├── benchmark/                      # 性能評測套件
├── tests/                          # 單元測試與集成測試
└── scripts/                        # 工具與自動化腳本
```

---

## 核心 Trait/Struct

### 1. BaseCoder 類（基類）

```python
class Coder:
    """編碼器基類，定義統一的編碼接口"""

    # 狀態屬性
    abs_fnames: List[str]           # 被編輯文件的絕對路徑
    abs_read_only_fnames: List[str] # 只讀文件列表
    repo: GitRepo                   # Git 倉庫對象

    # 編輯狀態
    edit_format: str                # 編輯格式（editblock、wholefile 等）
    num_malformed_responses: int    # 格式錯誤響應計數
    num_reflections: int            # 反思迭代次數

    # LLM 交互
    message_cost: float             # 當前消息成本
    temperature: float              # 溫度參數

    # 主要方法
    def run(self, user_messages)    # 運行編碼循環
    def get_messages(self)          # 構建提交給 LLM 的消息
    def parse_response(response)    # 解析 LLM 響應
    def apply_edits(edits)          # 應用編輯到文件
```

### 2. 子 Coder 類型（多態設計）

- **EditBlockCoder** - 使用 `<aider>` 標籤塊編輯
- **WholeFileCoder** - 替換整個文件
- **UDiffCoder** - 使用 Unified Diff 格式
- **PatchCoder** - 使用 Patch 文件格式
- **ArchitectCoder** - 高層架構規劃（分離關注點）

每個 Coder 子類重寫：
- `get_edit_format()`
- `parse_response()`
- `apply_edits()`

### 3. GitRepo 類

```python
class GitRepo:
    """版本控制管理"""

    root: Path                      # 倉庫根目錄
    repo: git.Repo                  # GitPython Repo 對象

    def get_files()                 # 獲取追蹤文件列表
    def get_changed_files()         # 變動文件
    def write_gitignore()           # 生成 .gitignore
    def get_git_log()               # 獲取 commit 歷史
```

### 4. RepoMap 類（Repository 智能索引）

```python
class RepoMap:
    """構建代碼文件的結構索引"""

    def get_relevant_files(query)   # 根據查詢找出相關文件
    def get_tokens(fnames)          # 計算文件的 token 數
    def get_summary()               # 生成倉庫摘要
```

### 5. ChatSummary 類

```python
class ChatSummary:
    """對話歷史摘要與管理"""

    def save_message()              # 保存消息到歷史
    def get_summary()               # 提取關鍵信息摘要
    def load_previous()             # 加載之前的對話
```

---

## 啟動流程

### 1. CLI 入口 (`main.py`)

```
aider (CLI)
  → get_parser()
    - 解析命令行參數
    - 配置 LLM 提供商、模型、文件列表
  → check_config_files()
    - 驗證配置文件（YAML/TOML）
  → InputOutput 初始化
    - 設置終端 UI（prompt_toolkit）
  → 選擇 Coder 類型
    - 根據配置選擇適當的 Coder 子類
  → 進入交互循環
```

### 2. Coder 初始化流程

```
Coder.__init__(io, model, fnames)
  → 初始化 GitRepo
  → 構建 RepoMap
  → 編譯 system prompt
    - 加載 base_prompts
    - 添加文件上下文
    - 注入編輯格式說明
  → 設置編輯格式解析器
```

### 3. 核心編碼循環

```
run(user_message)
  → get_messages()
    - 構建消息列表（系統 + 歷史 + 當前）
    - 計算 token 數，應用上下文管理
  → call_llm()
    - 通過 LiteLLM 調用 LLM API
    - 接收流式響應
  → parse_response()
    - 提取編輯塊/Diff
    - 驗證語法
  → apply_edits()
    - 寫入文件
    - 運行 linter/測試（如啟用）
  → commit_with_aider()
    - 如果成功，自動 git commit
  → 返回用戶進行確認
```

---

## 資料流 ASCII 圖

```
┌─────────────────────────────────────────────────────────────┐
│                    終端 UI (prompt_toolkit)                 │
│                                                              │
│  User Input → [Command Parser] → [File Manager] → Output  │
└──────────────────┬──────────────────────────────────────────┘
                   │
                   ↓
        ┌──────────────────────┐
        │   Coder Instance      │
        │  (多態：Edit/Whole)   │
        │                      │
        │  - Message Builder   │
        │  - Response Parser   │
        │  - Edit Applier      │
        └──────────┬───────────┘
                   │
        ┌──────────┴──────────┐
        ↓                     ↓
    ┌────────────┐      ┌──────────────┐
    │  GitRepo   │      │   RepoMap    │
    │            │      │              │
    │  - Files   │      │  - Code Map  │
    │  - History │      │  - Search    │
    │  - Commit  │      │  - Summary   │
    └──────┬─────┘      └──────┬───────┘
           │                   │
           └───────────┬───────┘
                       ↓
        ┌──────────────────────────┐
        │     LiteLLM Interface    │
        │                          │
        │  [OpenAI] [Claude]       │
        │  [Gemini] [Ollama]  ...  │
        └──────────┬───────────────┘
                   │
                   ↓
        ┌──────────────────────────┐
        │    外部 LLM API          │
        │                          │
        │  Chat Completion         │
        │  Streaming Response      │
        └──────────────────────────┘
```

---

## 子系統清單

### P0 - 核心功能（必須）

| 子系統 | 文件 | 功能 | 狀態 |
|--------|------|------|------|
| **Coder 引擎** | `base_coder.py` + 子類 | 編輯格式解析、應用 | ✅ 成熟 |
| **LLM 整合** | `llm.py`、`models.py` | LiteLLM 封裝、模型管理 | ✅ 成熟 |
| **Git 管理** | `repo.py` | 版本控制、commit | ✅ 成熟 |
| **終端 UI** | `io.py`、`gui.py` | prompt_toolkit 集成 | ✅ 成熟 |
| **Prompt 引擎** | `prompts.py`、`*_prompts.py` | system prompt 生成 | ✅ 成熟 |
| **檔案操作** | `commands.py` | 檔案添加/移除、監視 | ✅ 成熟 |
| **RepoMap** | `repomap.py` | 代碼智能索引 | ✅ 成熟 |

### P1 - 增強功能（重要）

| 子系統 | 文件 | 功能 | 狀態 |
|--------|------|------|------|
| **Linter 整合** | `linter.py` | 代碼品質檢查、自動修復 | ✅ 成熟 |
| **對話歷史** | `history.py` | 會話管理、摘要生成 | ✅ 成熟 |
| **多編輯格式** | `*_coder.py` 家族 | 支援 5+ 種編輯方式 | ✅ 成熟 |
| **文件監視** | `watch.py` | 熱重載檔案變化 | ✅ 成熟 |
| **分析統計** | `analytics.py` | 使用數據收集 | ✅ 成熟 |
| **架構層** | `architect_coder.py` | 高層規劃分離 | ✅ 成熟 |

### P2 - 實驗功能（可選）

| 子系統 | 文件 | 功能 | 狀態 |
|--------|------|------|------|
| **語音輸入** | `voice.py` | 語音轉文字 | 🔄 開發中 |
| **GUI 模式** | `gui.py` | 圖形界面 | 🔄 早期 |
| **Web 集成** | N/A | HTTP API 服務 | ❌ 計畫中 |
| **自適應提示** | N/A | 動態 prompt 優化 | 🔄 研究中 |

---

## 關鍵設計模式

### 1. 編碼器多態設計

```python
# 統一的編碼器接口
class Coder:
    def parse_response(self, response): raise NotImplementedError()
    def apply_edits(self, edits): raise NotImplementedError()

# 多個子類實現不同的編輯策略
class EditBlockCoder(Coder): ...   # <aider>...</aider>
class WholeFileCoder(Coder): ...   # 整個文件替換
class UDiffCoder(Coder): ...       # diff 格式
```

**優勢**：易於添加新的編輯格式，不破壞現有邏輯

### 2. 懶加載與流式處理

```python
# LLM 響應流式處理
for chunk in llm.stream(messages):
    yield chunk  # 即時將結果返回給用戶
    parse_incremental(chunk)  # 漸進式解析
```

### 3. Repository 智能索引

RepoMap 根據用戶輸入的查詢，動態計算與之相關的代碼上下文，減少 token 使用。

### 4. 編輯驗證反思迴路

如果編輯失敗（Linter 錯誤、語法錯誤），自動進入反思迴路，LLM 修復問題。

---

## 與 Clawtex 的差異對比

| 特性 | Aider | Clawtex |
|------|-------|---------|
| **語言** | Python | Rust |
| **UI** | 終端（TUI） | Telegram/Web |
| **編輯方式** | 多種格式選擇 | JSON DSL |
| **對話方式** | 交互式（命令行） | 非同步（Agent 自主） |
| **模型支援** | 40+ 提供商 | 12+ 提供商 |
| **編排** | 簡單循環 | Hands 工作流引擎 |
| **數據庫** | 檔案系統 | SQLite/多 DB |

---

## 性能特點

- **快速啟動**：Python 直譯，無編譯延遲
- **流式處理**：即時流式響應，無需等待完整回覆
- **上下文管理**：RepoMap 自動優化相關文件選擇
- **增量編輯**：Edit Block 格式只更新必要部分，減少 token 消耗

---

## 總結

Aider 是一款成熟的 **終端優先 AI 編程工具**，以其多樣的編輯格式和靈活的編碼器設計而聞名。相比 Cline（IDE 集成）和 OpenCode（TUI），Aider 更注重於**與開發者的交互式配對編程體驗**，而非完全自主的代理行為。其架構的核心強度在於多態的編碼器設計和智能的倉庫上下文管理。

