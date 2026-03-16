# Aider 深度技術分析

> 分析日期: 2026-03-12
> 專案路徑: `LLM-Cluster-Project/references/aider/`
> 目標: 從開發者角度深入解析 Aider 的架構設計，萃取可用於 clawtex-core 的模式與策略。

---

## 目錄

1. [專案結構](#1-專案結構)
2. [入口點與啟動流程](#2-入口點與啟動流程)
3. [核心架構](#3-核心架構)
   - 3.1 [Coder 類別體系](#31-coder-類別體系)
   - 3.2 [Agent 迴圈](#32-agent-迴圈)
   - 3.3 [模型整合系統](#33-模型整合系統)
   - 3.4 [編輯格式系統](#34-編輯格式系統)
4. [Git 整合](#4-git-整合)
5. [上下文管理 — Repo Map](#5-上下文管理--repo-map)
6. [多檔案協調編輯](#6-多檔案協調編輯)
7. [Linting / Testing 整合](#7-linting--testing-整合)
8. [語音輸入](#8-語音輸入)
9. [值得採納的關鍵模式](#9-值得採納的關鍵模式)
10. [與 clawtex-core 的關聯性](#10-與-clawtex-core-的關聯性)

---

## 1. 專案結構

```
aider/
  aider/                    # 核心 Python 套件
    __init__.py
    __main__.py             # python -m aider 入口
    main.py                 # CLI 啟動邏輯、配置載入、Coder 初始化
    args.py                 # configargparse 參數定義（YAML / env / CLI 三層合一）
    models.py               # 模型元數據系統（ModelSettings + Model 類別）
    repo.py                 # GitRepo 封裝（GitPython）
    repomap.py              # Repo Map 引擎（tree-sitter + PageRank）
    commands.py             # /add, /drop, /commit, /diff 等斜線命令
    io.py                   # InputOutput — prompt_toolkit 交互層
    linter.py               # tree-sitter 語法檢查 + flake8 + 自訂命令
    voice.py                # Whisper API 語音輸入
    watch.py                # FileWatcher — 監聽 AI 註解變更
    sendchat.py             # 訊息交替驗證工具
    history.py              # ChatSummary — 歷史摘要壓縮
    llm.py                  # litellm 初始化包裝
    scrape.py               # Playwright 網頁抓取
    diffs.py                # 差異顯示工具
    mdstream.py             # Markdown 串流渲染（Rich）
    prompts.py              # 全域提示詞常數
    utils.py                # 工具函式庫
    analytics.py            # Mixpanel 匿名分析
    exceptions.py           # LiteLLM 例外分類
    openrouter.py           # OpenRouter 模型管理
    reasoning_tags.py       # <reasoning> 標籤處理
    coders/                 # 編輯模式子類別（核心！）
      __init__.py           # 註冊所有 Coder 子類別
      base_coder.py         # Coder 基底類別（~2500 行，核心迴圈）
      base_prompts.py       # 基礎提示詞
      chat_chunks.py        # ChatChunks 訊息分區
      editblock_coder.py    # SEARCH/REPLACE 編輯器
      editblock_prompts.py  # SEARCH/REPLACE 提示詞
      wholefile_coder.py    # 整檔覆寫編輯器
      udiff_coder.py        # Unified Diff 編輯器
      architect_coder.py    # 架構師 → 編輯器雙層模式
      patch_coder.py        # Patch 格式編輯器
      ask_coder.py          # 純問答模式
      context_coder.py      # 上下文選擇器
      help_coder.py         # 幫助模式
      search_replace.py     # 搜尋替換核心演算法
      editor_*_coder.py     # 各種 Editor 子模式
      shell.py              # Shell 命令提示詞
    queries/                # tree-sitter 查詢檔（.scm）
      tree-sitter-language-pack/   # 30+ 語言的 tags 查詢
      tree-sitter-languages/       # 舊版語言查詢
    resources/
      model-settings.yml    # 100+ 模型的預設配置
    website/                # 文件網站內容
  benchmark/                # 效能基準測試
  tests/                    # 測試套件
  docker/                   # Docker 配置
  scripts/                  # 開發工具腳本
  pyproject.toml            # 專案配置
```

**關鍵觀察**: Aider 的核心設計圍繞 `coders/` 目錄 — 每個編輯模式都是 `Coder` 基底類別的子類別，
透過覆寫 `get_edits()` 和 `apply_edits()` 方法來實現不同的程式碼修改策略。

---

## 2. 入口點與啟動流程

### 入口點

**檔案**: `aider/main.py` + `aider/__main__.py`

```python
# aider/__main__.py
from aider.main import main
main()
```

### 啟動流程

`main()` 函式在 `aider/main.py` 中，執行以下步驟：

**Step 1 — 配置載入** (多層合併)
```python
# aider/args.py
parser = configargparse.ArgumentParser(
    default_config_files=default_config_files,
    config_file_parser_class=configargparse.YAMLConfigFileParser,
    auto_env_var_prefix="AIDER_",
)
```
配置來源的優先順序：
1. CLI 參數（最高優先）
2. `.aider.conf.yml`（專案級別）
3. `~/.aider.conf.yml`（使用者級別）
4. `AIDER_*` 環境變數
5. `.env` 檔案

**Step 2 — Git 偵測與初始化**
```python
# aider/main.py
git_root = get_git_root()
git_root = setup_git(git_root, io)  # 可能會提示建立新 repo
check_gitignore(git_root, io)       # 確保 .aider* 在 .gitignore
```

**Step 3 — 模型選擇與 Onboarding**
```python
# aider/main.py → aider/onboarding.py
main_model = models.Model(
    args.model,
    weak_model=args.weak_model,
    editor_model=args.editor_model,
)
```
如果沒有提供 API Key，會觸發 OAuth 流程引導使用者取得 OpenRouter 免費帳號。

**Step 4 — Coder 工廠建立**
```python
# aider/coders/base_coder.py :: Coder.create()
coder = Coder.create(
    main_model=main_model,
    edit_format=args.edit_format,
    io=io,
    repo=repo,
    fnames=fnames,
    auto_commits=args.auto_commits,
    auto_lint=args.auto_lint,
    auto_test=args.auto_test,
    # ... 40+ 參數
)
```

**Step 5 — 主迴圈**
```python
coder.run()  # 進入 REPL 互動迴圈
```

---

## 3. 核心架構

### 3.1 Coder 類別體系

Aider 的核心是一個精心設計的 **策略模式（Strategy Pattern）**，所有編輯模式都繼承自 `Coder` 基底類別：

```
Coder (base_coder.py, ~2500 行)
  ├── EditBlockCoder      # edit_format = "diff"      — SEARCH/REPLACE 區塊
  ├── WholeFileCoder      # edit_format = "whole"      — 整檔覆寫
  ├── UnifiedDiffCoder    # edit_format = "udiff"      — 標準 unified diff
  ├── PatchCoder          # edit_format = "patch"      — 結構化 patch
  ├── AskCoder            # edit_format = "ask"        — 純問答（不修改檔案）
  │   └── ArchitectCoder  # edit_format = "architect"  — 雙層：架構師+編輯器
  ├── ContextCoder        # edit_format = "context"    — 上下文檔案選擇
  ├── HelpCoder           # edit_format = "help"       — 使用說明
  ├── EditorEditBlockCoder    # edit_format = "editor-diff"
  ├── EditorWholeFileCoder    # edit_format = "editor-whole"
  └── EditorDiffFencedCoder   # edit_format = "editor-diff-fenced"
```

**每個子類別只需覆寫兩個方法**：

```python
# aider/coders/editblock_coder.py
class EditBlockCoder(Coder):
    edit_format = "diff"
    gpt_prompts = EditBlockPrompts()

    def get_edits(self):
        """從 LLM 回應中解析編輯操作"""
        content = self.partial_response_content
        edits = list(find_original_update_blocks(content, self.fence, ...))
        return edits

    def apply_edits(self, edits, dry_run=False):
        """將解析後的編輯套用到檔案"""
        for path, original, updated in edits:
            full_path = self.abs_root_path(path)
            content = self.io.read_text(full_path)
            new_content = do_replace(full_path, content, original, updated, self.fence)
            if new_content:
                self.io.write_text(full_path, new_content)
```

**Coder 工廠方法** — `Coder.create()` 利用 `__init__.py` 中的 `__all__` 列表動態匹配：

```python
# aider/coders/__init__.py
__all__ = [
    HelpCoder, AskCoder, Coder, EditBlockCoder, EditBlockFencedCoder,
    WholeFileCoder, PatchCoder, UnifiedDiffCoder, UnifiedDiffSimpleCoder,
    ArchitectCoder, EditorEditBlockCoder, EditorWholeFileCoder,
    EditorDiffFencedCoder, ContextCoder,
]

# aider/coders/base_coder.py :: Coder.create()
for coder in coders.__all__:
    if hasattr(coder, "edit_format") and coder.edit_format == edit_format:
        res = coder(main_model, io, **kwargs)
        return res
```

### 3.2 Agent 迴圈

核心迴圈在 `base_coder.py` 的 `run()` → `run_one()` → `send_message()` 中：

```
run()
  └── while True:
      ├── get_input()                    # prompt_toolkit 使用者輸入
      └── run_one(message, preproc)
          ├── preproc_user_input(inp)    # 斜線命令檢查、URL 偵測、檔案提及
          └── while message:             # 反射迴圈（最多 3 次）
              ├── send_message(message)
              │   ├── format_messages()  # 組裝完整提示詞
              │   │   ├── system prompt
              │   │   ├── example messages
              │   │   ├── done_messages (歷史摘要)
              │   │   ├── repo_map      ★ 倉庫地圖
              │   │   ├── read-only files
              │   │   ├── chat files (可編輯的)
              │   │   ├── cur_messages
              │   │   └── reminder prompt
              │   ├── check_tokens()     # 檢查 token 限制
              │   ├── send()             # litellm.completion()
              │   │   └── 串流或批次處理回應
              │   ├── apply_updates()    ★ 解析 + 套用編輯
              │   │   ├── get_edits()    # 子類別實作
              │   │   ├── apply_edits_dry_run()
              │   │   ├── prepare_to_edit()  # dirty commit
              │   │   └── apply_edits()  # 子類別實作
              │   ├── auto_commit()      # 自動 git commit
              │   ├── lint_edited()      # 自動 lint
              │   ├── run_shell_commands()  # 執行建議的 shell 命令
              │   └── cmd_test()         # 自動測試
              └── reflected_message      # 如果有錯誤，反射修正
```

**反射機制（Reflection）** 是 Aider 的核心創新之一：

```python
# base_coder.py :: run_one()
while message:
    self.reflected_message = None
    list(self.send_message(message))
    if not self.reflected_message:
        break
    if self.num_reflections >= self.max_reflections:  # 預設 3
        break
    self.num_reflections += 1
    message = self.reflected_message
```

反射觸發的場景：
1. **編輯格式錯誤** — SEARCH 區塊無法匹配原始碼 → 將錯誤訊息回饋給 LLM
2. **Lint 錯誤** — 套用編輯後有語法問題 → 讓 LLM 修正
3. **測試失敗** — 自動測試不通過 → 讓 LLM 修正
4. **檔案提及** — LLM 提到未加入聊天的檔案 → 提示加入

### 3.3 模型整合系統

**檔案**: `aider/models.py`

Aider 透過 **litellm** 支援 100+ 模型，配合自建的模型元數據系統：

**ModelSettings 資料類別**:
```python
@dataclass
class ModelSettings:
    name: str
    edit_format: str = "whole"          # 預設編輯格式
    weak_model_name: Optional[str]      # 用於 commit message 等低成本任務
    use_repo_map: bool = False          # 是否啟用 repo map
    lazy: bool = False                  # 是否需要反偷懶提示
    overeager: bool = False             # 是否需要精確提示
    reminder: str = "user"              # reminder 放在 system 或 user 訊息
    examples_as_sys_msg: bool = False   # 範例放在 system 訊息中
    cache_control: bool = False         # 支援 Anthropic 快取控制
    streaming: bool = True              # 是否支援串流
    editor_model_name: Optional[str]    # architect 模式的編輯器模型
    editor_edit_format: Optional[str]   # architect 模式的編輯格式
    reasoning_tag: Optional[str]        # 推理標籤名稱
    system_prompt_prefix: Optional[str] # 系統提示前綴
    extra_params: Optional[dict]        # 額外 API 參數
```

**模型設定從 YAML 載入**:
```python
# aider/models.py
MODEL_SETTINGS = []
with importlib.resources.open_text("aider.resources", "model-settings.yml") as f:
    model_settings_list = yaml.safe_load(f)
    for model_settings_dict in model_settings_list:
        MODEL_SETTINGS.append(ModelSettings(**model_settings_dict))
```

**model-settings.yml 範例**:
```yaml
- name: claude-sonnet-4-5
  edit_format: diff
  weak_model_name: claude-haiku-4-5
  use_repo_map: true
  send_undo_reply: true
  lazy: true
  reminder: user
  cache_control: true
  editor_model_name: claude-sonnet-4-5
  editor_edit_format: editor-diff

- name: gpt-4o
  edit_format: diff
  weak_model_name: gpt-4o-mini
  use_repo_map: true
  lazy: true
  reminder: sys
  examples_as_sys_msg: true
  editor_edit_format: editor-diff
```

**模型別名系統**:
```python
MODEL_ALIASES = {
    "sonnet": "claude-sonnet-4-5",
    "opus": "claude-opus-4-6",
    "4o": "gpt-4o",
    "deepseek": "deepseek/deepseek-chat",
    "gemini": "gemini/gemini-3-pro-preview",
    "flash": "gemini/gemini-flash-latest",
    # ...
}
```

**模型資訊管理器** — 三層緩存：
1. litellm 內建資訊
2. 本地 JSON 快取 (`~/.aider/caches/model_prices_and_context_window.json`)
3. OpenRouter API / 網頁抓取

**三模型架構**:
- `main_model` — 主要模型（執行編輯）
- `weak_model` — 弱模型（commit message、摘要等低成本任務）
- `editor_model` — 編輯器模型（在 architect 模式中執行實際編輯）

### 3.4 編輯格式系統

Aider 支援多種編輯格式，每種都有針對性的提示詞設計和解析邏輯：

#### SEARCH/REPLACE 格式（`diff`，最常用）

**檔案**: `aider/coders/editblock_coder.py`

LLM 輸出格式：
```
path/to/file.py
```python
<<<<<<< SEARCH
原始碼（必須精確匹配）
=======
替換後的碼
>>>>>>> REPLACE
```

**核心替換邏輯** — 多層容錯：
```python
def replace_most_similar_chunk(whole, part, replace):
    # 1. 精確匹配
    res = perfect_replace(whole_lines, part_lines, replace_lines)
    if res: return res

    # 2. 容忍前導空白差異
    res = replace_part_with_missing_leading_whitespace(...)
    if res: return res

    # 3. 處理 ... 省略符號
    res = try_dotdotdots(whole, part, replace)
    if res: return res

    # 4. 模糊匹配（已停用但保留程式碼）
    # res = replace_closest_edit_distance(...)
```

**檔名解析** — 同樣多層容錯：
```python
def find_filename(lines, fence, valid_fnames):
    # 1. 精確匹配已加入聊天的檔案
    # 2. basename 匹配
    # 3. difflib.get_close_matches 模糊匹配
    # 4. 尋找有副檔名的名稱
```

#### Unified Diff 格式（`udiff`）

**檔案**: `aider/coders/udiff_coder.py`

```diff
```diff
--- a/path/to/file.py
+++ b/path/to/file.py
@@ ... @@
-old line
+new line
 context
```

特色：分段套用（partial hunk application），如果整個 hunk 匹配失敗，
會嘗試將 hunk 拆成多個小段分別套用。

#### Whole File 格式（`whole`）

**檔案**: `aider/coders/wholefile_coder.py`

LLM 直接輸出完整的檔案內容。適合弱模型（無法穩定輸出 diff 的模型）。

#### Architect 模式

**檔案**: `aider/coders/architect_coder.py`

這是一個**雙層代理模式**：
1. 架構師模型（通常是強模型如 Claude Opus）分析需求、描述修改方案
2. 編輯器模型（可以是相同或不同的模型）將方案轉為實際的程式碼編輯

```python
class ArchitectCoder(AskCoder):
    edit_format = "architect"

    def reply_completed(self):
        content = self.partial_response_content

        # 使用編輯器模型執行實際編輯
        editor_model = self.main_model.editor_model or self.main_model
        kwargs["main_model"] = editor_model
        kwargs["edit_format"] = self.main_model.editor_edit_format

        editor_coder = Coder.create(**new_kwargs)
        editor_coder.run(with_message=content, preproc=False)
```

#### Patch 格式

**檔案**: `aider/coders/patch_coder.py`

結構化的 patch 格式，包含 `ActionType`（Add/Delete/Update）和 `Chunk` 資料結構，
使用三級模糊匹配（精確 → rstrip → strip）。

---

## 4. Git 整合

**檔案**: `aider/repo.py`

### 自動提交

Aider 的 Git 策略是 **每次成功編輯都自動提交**：

```python
# base_coder.py :: send_message()
edited = self.apply_updates()
if edited:
    saved_message = self.auto_commit(edited)

# repo.py :: commit()
def commit(self, fnames=None, context=None, message=None, aider_edits=False, coder=None):
    diffs = self.get_diffs(fnames)
    if not message:
        commit_message = self.get_commit_message(diffs, context, user_language)
    # ... 使用 weak_model 生成 commit message
    self.repo.git.commit(cmd)
```

### Commit 歸屬

精心設計的歸屬系統，支援多種模式：
- `--attribute-author` — 修改 Author 為 "UserName (aider)"
- `--attribute-committer` — 修改 Committer 為 "UserName (aider)"
- `--attribute-co-authored-by` — 加入 `Co-authored-by: aider (model) <aider@aider.chat>` 標尾

### Dirty Commit

在 Aider 修改檔案前，如果有未提交的變更，會先自動提交使用者的修改，
確保 Aider 的編輯和使用者的編輯在不同的 commit 中，方便 undo：

```python
def prepare_to_edit(self, edits):
    for edit in edits:
        path = edit[0]
        if self.repo.is_dirty(path):
            self.need_commit_before_edits.add(path)
    self.dirty_commit()
    return edits
```

### Diff 追蹤

使用 LLM 生成的 commit message 會包含上下文：
```python
def get_commit_message(self, diffs, context, user_language=None):
    content = context + "\n" + "# Diffs:\n" + diffs
    messages = [
        dict(role="system", content=system_content),
        dict(role="user", content=content),
    ]
    commit_message = model.simple_send_with_retries(messages)
```

---

## 5. 上下文管理 -- Repo Map

**檔案**: `aider/repomap.py`（785 行）

Repo Map 是 Aider 最具創新性的功能，它為 LLM 提供整個倉庫的**結構概覽**，
讓 LLM 理解哪些符號在哪些檔案中定義和引用。

### 核心流程

```
1. tree-sitter 解析每個檔案 → 提取定義(def)和引用(ref)標籤
2. 建構 NetworkX 有向圖：檔案→檔案 的引用關係
3. 執行 PageRank 排名
4. 用排名結果選擇最相關的標籤
5. 用 TreeContext 格式化輸出（只顯示相關行+上下文）
6. 二分搜尋控制 token 數量
```

### Step 1: tree-sitter 標籤提取

```python
# repomap.py
Tag = namedtuple("Tag", "rel_fname fname line name kind".split())

def get_tags_raw(self, fname, rel_fname):
    lang = filename_to_lang(fname)
    language = get_language(lang)
    parser = get_parser(lang)

    # 載入語言特定的查詢
    query_scm = get_scm_fname(lang)  # 例如 python-tags.scm
    query_scm = query_scm.read_text()

    code = self.io.read_text(fname)
    tree = parser.parse(bytes(code, "utf-8"))

    captures = self._run_captures(Query(language, query_scm), tree.root_node)

    for node, tag in all_nodes:
        if tag.startswith("name.definition."):
            kind = "def"
        elif tag.startswith("name.reference."):
            kind = "ref"
        yield Tag(rel_fname=rel_fname, fname=fname,
                  name=node.text.decode("utf-8"), kind=kind, line=node.start_point[0])
```

支援的查詢檔案位於 `aider/queries/tree-sitter-language-pack/`，涵蓋 30+ 語言：
- Python, JavaScript, TypeScript, Rust, Go, Java, C, C++, C#, Ruby, Elixir, Dart, Swift...

### Step 2-3: PageRank 排名

```python
def get_ranked_tags(self, chat_fnames, other_fnames, mentioned_fnames, mentioned_idents):
    import networkx as nx

    defines = defaultdict(set)      # 符號 → 定義所在檔案集合
    references = defaultdict(list)   # 符號 → 引用所在檔案列表
    personalization = dict()         # 個性化 PageRank 權重

    G = nx.MultiDiGraph()

    for ident in idents:
        mul = 1.0
        # 使用者提到的識別符 → 10x 權重
        if ident in mentioned_idents:
            mul *= 10
        # 蛇形/駝峰命名且長度 >= 8 → 10x（過濾掉短通用名）
        if (is_snake or is_camel) and len(ident) >= 8:
            mul *= 10
        # 私有符號 → 0.1x
        if ident.startswith("_"):
            mul *= 0.1

        for referencer, num_refs in Counter(references[ident]).items():
            for definer in definers:
                use_mul = mul
                # 正在聊天中的檔案 → 50x（確保相關檔案排在最前）
                if referencer in chat_rel_fnames:
                    use_mul *= 50
                G.add_edge(referencer, definer,
                           weight=use_mul * math.sqrt(num_refs), ident=ident)

    ranked = nx.pagerank(G, weight="weight", personalization=personalization)
```

**權重設計的巧妙之處**：
- 使用者正在編輯的檔案引用的符號得到 **50x 加權**
- 使用者在訊息中提到的識別符得到 **10x 加權**
- 有意義的命名（蛇形/駝峰且長度 >= 8）得到 **10x 加權**
- 定義超過 5 處的符號得到 **0.1x 懲罰**（過於通用）

### Step 4-5: TreeContext 格式化

使用 `grep_ast.TreeContext` 智慧顯示相關行及其語法上下文：

```python
def render_tree(self, abs_fname, rel_fname, lois):
    context = TreeContext(
        rel_fname, code,
        color=False, line_number=False,
        child_context=False, last_line=False,
        margin=0, mark_lois=False, loi_pad=0,
        show_top_of_file_parent_scope=False,
    )
    context.add_lines_of_interest(lois)
    context.add_context()
    return context.format()
```

### Step 6: 二分搜尋 Token 控制

```python
def get_ranked_tags_map_uncached(self, ...):
    # 使用二分搜尋找到最佳 token 數量
    middle = min(int(max_map_tokens // 25), num_tags)
    while lower_bound <= upper_bound:
        tree = self.to_tree(ranked_tags[:middle], chat_rel_fnames)
        num_tokens = self.token_count(tree)

        pct_err = abs(num_tokens - max_map_tokens) / max_map_tokens
        if pct_err < 0.15:  # 15% 容差
            break

        if num_tokens < max_map_tokens:
            lower_bound = middle + 1
        else:
            upper_bound = middle - 1
        middle = int((lower_bound + upper_bound) // 2)
```

### 快取機制

- **標籤快取**: `diskcache.Cache` (SQLite)，以 mtime 為鍵
- **地圖快取**: 記憶體快取，根據聊天檔案集合和提及的符號作為鍵
- **Tree 快取**: 記憶體快取，避免重複渲染

---

## 6. 多檔案協調編輯

Aider 的多檔案編輯策略：

### 檔案管理

- **Chat files** (`/add`) — 可讀寫的檔案，LLM 可以修改
- **Read-only files** (`/read-only`) — 只讀參考檔案
- **Repo map files** — 透過 repo map 提供的結構概覽

### 提示詞設計

```python
# editblock_prompts.py
main_system = """...
1. Decide if you need to propose *SEARCH/REPLACE* edits to any files
   that haven't been added to the chat.
   But if you need to propose edits to existing files not already added
   to the chat, you *MUST* tell the user their full path names and ask
   them to *add the files to the chat*.
...
"""
```

### 智慧檔案偵測

```python
# base_coder.py
def check_for_file_mentions(self, content):
    """偵測 LLM 回應中提到的檔案名稱"""
    mentioned_rel_fnames = self.get_file_mentions(content)
    for rel_fname in sorted(new_mentions):
        if self.io.confirm_ask("Add file to the chat?", subject=rel_fname):
            self.add_rel_fname(rel_fname)
            added_fnames.append(rel_fname)
    # 回傳 reflected_message 讓 LLM 重新嘗試
    return prompts.added_files.format(fnames=", ".join(added_fnames))
```

### 跨檔案編輯容錯

```python
# editblock_coder.py :: apply_edits()
if not new_content and original.strip():
    # 如果在指定檔案中找不到匹配，嘗試所有聊天中的檔案
    for full_path in self.abs_fnames:
        content = self.io.read_text(full_path)
        new_content = do_replace(full_path, content, original, updated, self.fence)
        if new_content:
            path = self.get_rel_fname(full_path)
            break
```

---

## 7. Linting / Testing 整合

**檔案**: `aider/linter.py`

### Linter 架構

```python
class Linter:
    def __init__(self, encoding="utf-8", root=None):
        self.languages = dict(python=self.py_lint)  # 內建 Python lint
        self.all_lint_cmd = None                      # 自訂全域 lint 命令

    def lint(self, fname, cmd=None):
        lang = filename_to_lang(fname)
        if callable(cmd):
            lintres = cmd(fname, rel_fname, code)
        elif cmd:
            lintres = self.run_cmd(cmd, rel_fname, code)
        else:
            lintres = basic_lint(rel_fname, code)  # tree-sitter 語法檢查
```

### 三層 Python Lint

```python
def py_lint(self, fname, rel_fname, code):
    basic_res = basic_lint(rel_fname, code)       # tree-sitter 語法錯誤
    compile_res = lint_python_compile(fname, code) # compile() 檢查
    flake_res = self.flake8_lint(rel_fname)        # flake8 致命錯誤
    # 合併結果
```

### 通用語法檢查（tree-sitter）

```python
def basic_lint(fname, code):
    """使用 tree-sitter 檢測語法錯誤"""
    parser = get_parser(lang)
    tree = parser.parse(bytes(code, "utf-8"))
    errors = traverse_tree(tree.root_node)  # 遍歷找 ERROR 節點

def traverse_tree(node):
    errors = []
    if node.type == "ERROR" or node.is_missing:
        line_no = node.start_point[0]
        errors.append(line_no)
    for child in node.children:
        errors += traverse_tree(child)
    return errors
```

### 自動 Lint/Test 流程

在 `send_message()` 中，編輯成功後：

```python
# base_coder.py :: send_message()
if edited and self.auto_lint:
    lint_errors = self.lint_edited(edited)
    self.auto_commit(edited, context="Ran the linter")
    if lint_errors:
        ok = self.io.confirm_ask("Attempt to fix lint errors?")
        if ok:
            self.reflected_message = lint_errors  # 觸發反射修正

if edited and self.auto_test:
    test_errors = self.commands.cmd_test(self.test_cmd)
    if test_errors:
        ok = self.io.confirm_ask("Attempt to fix test errors?")
        if ok:
            self.reflected_message = test_errors  # 觸發反射修正
```

### 錯誤上下文呈現

```python
def tree_context(fname, code, line_nums):
    """使用 TreeContext 顯示錯誤行及其語法上下文"""
    context = TreeContext(
        fname, code,
        color=False, line_number=True,
        mark_lois=True,      # 用 █ 標記錯誤行
        loi_pad=3,            # 前後各 3 行上下文
    )
    context.add_lines_of_interest(line_nums)
    context.add_context()
    output = f"## See relevant lines below marked with █.\n\n"
    output += context.format()
```

---

## 8. 語音輸入

**檔案**: `aider/voice.py`

### 實作細節

```python
class Voice:
    def raw_record_and_transcribe(self, history, language):
        # 使用 sounddevice 錄音
        with self.sd.InputStream(
            samplerate=sample_rate, channels=1,
            callback=self.callback, device=self.device_id
        ):
            prompt(self.get_prompt, refresh_interval=0.1)

        # 儲存為 WAV
        with sf.SoundFile(temp_wav, mode="x", ...) as file:
            while not self.q.empty():
                file.write(self.q.get())

        # 大檔案自動轉 MP3
        if file_size > 24.9 * 1024 * 1024:
            audio = AudioSegment.from_wav(temp_wav)
            audio.export(new_filename, format="mp3")

        # 使用 Whisper API 轉錄
        transcript = litellm.transcription(
            model="whisper-1", file=fh,
            prompt=history, language=language
        )
        return transcript.text
```

### 即時音量顯示

```python
def callback(self, indata, frames, time, status):
    rms = np.sqrt(np.mean(indata**2))
    self.pct = (rms - self.min_rms) / (self.max_rms - self.min_rms)

def get_prompt(self):
    cnt = int(self.pct * 10)
    bar = "░" * cnt + "█" * (num - cnt)
    return f"Recording, press ENTER when done... {dur:.1f}sec {bar}"
```

---

## 9. 值得採納的關鍵模式

### 9.1 Repo Map（倉庫地圖）

**價值**: 讓 LLM 理解整個程式碼庫的結構，而不只是使用者手動加入的檔案。

**技術棧**: tree-sitter 語法解析 + NetworkX PageRank + 二分搜尋 token 控制

**clawtex 可採用的方案**:
- 用 Rust 版 tree-sitter（`tree-sitter` crate）替代 Python 版
- PageRank 可用 `petgraph` 或簡化的自訂實作
- 在 `ai_code` tool 執行前自動生成 repo map 作為上下文

### 9.2 編輯格式的策略模式

**價值**: 不同模型適合不同的編輯格式。強模型（Claude Opus, GPT-4o）用 SEARCH/REPLACE，
弱模型用 whole file，特別強的用 architect 雙層模式。

**技術要點**:
- 每個編輯格式都有專屬的提示詞（包含範例對話）
- 提示詞中包含精確的格式規範
- 解析器有多層容錯（精確 → 空白容錯 → 模糊匹配）

### 9.3 反射修正迴圈

**價值**: 當 LLM 輸出的編輯有問題時，自動將錯誤訊息回饋給 LLM 重試。

```
使用者請求 → LLM 生成編輯 → 套用失敗
    → 錯誤訊息 → LLM 修正 → 再次套用
    → 仍然失敗 → 錯誤訊息 → LLM 再修正
    （最多 3 次反射）
```

### 9.4 自動提交策略

**價值**: 每次成功編輯都 commit，使用者可以用 `/undo` 回退。

**設計要點**:
- 編輯前先 dirty commit 使用者的變更
- 使用 weak model 生成 commit message（省錢）
- 支援多種歸屬模式（author/committer/co-authored-by）

### 9.5 模型元數據系統

**價值**: YAML 驅動的模型配置，不需要改程式碼就能支援新模型。

**設計要點**:
- 每個模型有專屬的 edit_format、repo_map、lazy 等設定
- 模型別名（`sonnet` → `claude-sonnet-4-5`）
- 三層模型架構（main/weak/editor）

### 9.6 歷史摘要壓縮

**檔案**: `aider/history.py`

**價值**: 當對話歷史超過 token 限制時，自動壓縮舊訊息。

```python
class ChatSummary:
    def summarize_real(self, messages, depth=0):
        # 二分法：保留最近的尾部，壓縮前半部
        # 遞迴壓縮直到 fit 在 max_tokens 內
```

### 9.7 AI 註解監聽（File Watcher）

**檔案**: `aider/watch.py`

**價值**: 使用者在程式碼中寫 `// AI! fix this bug` 類的註解，
Aider 自動偵測並處理。

```python
ai_comment_pattern = re.compile(
    r"(?:#|//|--|;+) *(ai\b.*|ai\b.*|.*\bai[?!]?) *$", re.IGNORECASE
)
```
- `AI!` — 請求程式碼修改
- `AI?` — 請求回答問題

---

## 10. 與 clawtex-core 的關聯性

### 10.1 ai_code 工具改進

clawtex-core 的 `src/tools/ai_code.rs` 目前的實作相對簡單。
可以從 Aider 借鑑以下模式：

| Aider 模式 | clawtex 應用 | 優先級 |
|---|---|---|
| Repo Map | `ai_code` 執行前自動生成上下文 | 高 |
| SEARCH/REPLACE 格式 | 替代目前的 `file_edit` 整行替換 | 高 |
| 反射修正 | 編輯失敗後自動重試 | 高 |
| 自動 lint | 編輯後 tree-sitter 語法檢查 | 中 |
| 模型元數據 | 從 agents.toml 讀取各模型的 edit format 設定 | 中 |
| Architect 模式 | 用 classifier 的 complex 路由實現雙層模式 | 低 |

### 10.2 具體建議

**Repo Map for Rust**:
```rust
// 可用的 crate:
// - tree-sitter (核心解析器)
// - tree-sitter-python, tree-sitter-javascript 等
// - petgraph (圖演算法，包含 PageRank)

struct RepoMap {
    root: PathBuf,
    tag_cache: HashMap<PathBuf, Vec<Tag>>,
    max_tokens: usize,
}

struct Tag {
    rel_fname: String,
    name: String,
    kind: TagKind,  // Def | Ref
    line: usize,
}

impl RepoMap {
    fn get_ranked_tags(&self, chat_files: &[PathBuf]) -> Vec<Tag> {
        // 1. tree-sitter 提取標籤
        // 2. petgraph 建圖
        // 3. PageRank 排名
        // 4. 截斷到 max_tokens
    }
}
```

**SEARCH/REPLACE for clawtex**:
```rust
// 在 ai_code tool 中使用 SEARCH/REPLACE 格式
// 讓 LLM 輸出:
//   path/to/file.rs
//   <<<<<<< SEARCH
//   old code
//   =======
//   new code
//   >>>>>>> REPLACE

fn parse_search_replace_blocks(content: &str) -> Vec<Edit> {
    // 使用 regex 解析 SEARCH/REPLACE 區塊
}

fn apply_edit(file_content: &str, search: &str, replace: &str) -> Option<String> {
    // 1. 精確匹配
    // 2. 忽略前導空白匹配
    // 3. 行尾空白容錯
}
```

**反射修正整合到 agent_runtime**:
```rust
// src/agent_runtime.rs
impl AgentRuntime {
    async fn run_with_reflection(&self, message: &str) -> Result<String> {
        let mut attempts = 0;
        let mut current_message = message.to_string();

        loop {
            let response = self.run_round(&current_message).await?;

            // 嘗試套用編輯
            match self.apply_edits(&response) {
                Ok(_) => break,
                Err(e) if attempts < 3 => {
                    current_message = format!(
                        "The edit failed: {}\nPlease fix and try again.", e
                    );
                    attempts += 1;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(response)
    }
}
```

### 10.3 模型設定整合

在 `agents.toml` 中加入 Aider 風格的模型元數據：

```toml
[model_settings.claude-sonnet-4-5]
edit_format = "search_replace"
use_repo_map = true
max_repo_map_tokens = 4096
weak_model = "claude-haiku-4-5"
lazy = true

[model_settings.ollama/qwen2.5-coder]
edit_format = "whole_file"
use_repo_map = false
```

---

## 結論

Aider 是一個精心設計的系統，其核心創新在於：

1. **Repo Map** — 用 tree-sitter + PageRank 自動選擇最相關的程式碼上下文
2. **策略模式的編輯格式** — 根據模型能力選擇最適合的編輯方式
3. **反射修正迴圈** — 自動從錯誤中恢復，大幅提高編輯成功率
4. **Git-first 設計** — 每次編輯都有 commit，可隨時回退

對 clawtex-core 最有價值的是 Repo Map 概念和 SEARCH/REPLACE 編輯格式。
這兩個模式可以顯著提升 `ai_code` 工具在多檔案編輯場景下的準確性和可靠性。
