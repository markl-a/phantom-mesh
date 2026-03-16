# Claude Code Bridge (CCB) 深度技術分析

> 分析對象：`references/claude_code_bridge/` (by bfly123)
> 分析日期：2026-03-12
> 分析目的：為 clawtex-core 叢集與多 agent 架構提供參考

---

## 1. 專案結構

### 語言與技術棧

CCB 是一個**純 Python 專案**，無外部框架依賴（除了 watchdog 為可選依賴），完全使用 Python 標準庫實現。支援 tmux 與 WezTerm 兩種終端後端。

### 目錄樹

```
claude_code_bridge/
├── bin/                          # CLI 入口腳本（shell 指令）
│   ├── cask, cpend, cping        # Codex: ask/pend/ping
│   ├── gask, gpend, gping        # Gemini: ask/pend/ping
│   ├── lask, lpend, lping        # Claude: ask/pend/ping
│   ├── oask, opend, oping        # OpenCode: ask/pend/ping
│   ├── dask, dpend, dping        # Droid: ask/pend/ping
│   ├── ccb-ping, ccb-cleanup     # 系統工具
│   ├── ccb-arch, ccb-web         # 架構 + Web UI
│   ├── autonew, maild            # 自動建立 / 郵件守護
│   └── ctx-transfer              # 上下文轉移
├── lib/                          # 核心程式庫
│   ├── askd/                     # 統一 Ask 守護程式（核心）
│   │   ├── __init__.py
│   │   ├── daemon.py             # UnifiedAskDaemon 主迴圈
│   │   ├── registry.py           # ProviderRegistry
│   │   └── adapters/             # 各 Provider 適配器
│   │       ├── base.py           # BaseProviderAdapter（抽象介面）
│   │       ├── claude.py         # ClaudeAdapter
│   │       ├── codex.py          # CodexAdapter
│   │       ├── gemini.py         # GeminiAdapter
│   │       ├── droid.py          # DroidAdapter
│   │       ├── opencode.py       # OpenCodeAdapter
│   │       ├── copilot.py        # CopilotAdapter
│   │       ├── codebuddy.py      # CodeBuddyAdapter
│   │       └── qwen.py           # QwenAdapter
│   ├── terminal.py               # TerminalBackend 抽象 + TmuxBackend + WeztermBackend
│   ├── claude_comm.py            # Claude 通訊（JSONL 日誌讀取 + 終端注入）
│   ├── codex_comm.py             # Codex 通訊
│   ├── gemini_comm.py            # Gemini 通訊
│   ├── droid_comm.py             # Droid 通訊
│   ├── opencode_comm.py          # OpenCode 通訊
│   ├── ccb_protocol.py           # 請求/完成協議 (CCB_REQ_ID / CCB_DONE)
│   ├── ccb_config.py             # BackendEnv 配置（Windows/WSL 偵測）
│   ├── pane_registry.py          # 面板註冊表（持久化 JSON）
│   ├── session_utils.py          # Session 檔案工具
│   ├── claude_session_resolver.py # Claude Session 多源解析器
│   ├── caskd_session.py          # Codex 專案 Session（ensure_pane + 自癒）
│   ├── laskd_session.py          # Claude 專案 Session
│   ├── gaskd_session.py          # Gemini 專案 Session
│   ├── providers.py              # Provider/Daemon Spec 定義 + 多實例工具
│   ├── worker_pool.py            # PerSessionWorkerPool（執行緒池）
│   ├── project_id.py             # 專案 ID 計算
│   ├── codex_dual_bridge.py      # Claude-Codex 雙向橋接
│   ├── process_lock.py           # 程序鎖
│   ├── web/                      # Web UI
│   ├── mail/                     # 郵件守護
│   └── memory/                   # 記憶體模組
├── mcp/
│   └── ccb-delegation/
│       └── server.py             # MCP stdio 跨 Provider 委派伺服器
├── claude_skills/                # Claude Code 技能定義
│   ├── ask/                      # 非同步委派
│   ├── pend/                     # 取回結果
│   ├── cping/                    # Ping 健康檢查
│   ├── tp/                       # Task Plan（協作設計）
│   ├── tr/                       # Task Run（步驟執行）
│   ├── all-plan/                 # 全流程協作規劃
│   ├── review/                   # 審核評分
│   ├── file-op/                  # 檔案操作委派
│   ├── continue/                 # 接續執行
│   ├── autonew/                  # 自動建立 Session
│   └── mounted/                  # 掛載狀態
├── codex_skills/                 # Codex CLI 技能
├── droid_skills/                 # Droid CLI 技能
├── config/                       # 配置範本
├── docs/                         # 架構文件
│   ├── memory-first-agent-architecture.md
│   └── caskd-wezterm-daemon-plan.md
└── test/                         # 測試套件
```

### 程式碼規模估算

- `lib/` 核心：約 60+ Python 模組
- 8 個 Provider 適配器
- 2 個終端後端 (tmux / WezTerm)
- 12+ Claude/Codex/Droid 技能
- MCP 委派伺服器
- Web UI + Mail 守護

---

## 2. 入口點

### 守護程式入口

核心守護程式是 `UnifiedAskDaemon`（位於 `lib/askd/daemon.py`），它統一了所有原先獨立的守護程式（caskd、gaskd、oaskd、laskd、daskd）：

```python
# lib/askd/daemon.py
class UnifiedAskDaemon:
    """
    Unified daemon server for all AI providers.
    Handles requests for codex, gemini, opencode, droid, and claude
    in a single process with per-provider worker pools.
    """
    def __init__(self, host="127.0.0.1", port=0, *, registry=None):
        self.registry = registry or ProviderRegistry()
        self.pool = _UnifiedWorkerPool(self.registry)

    def serve_forever(self) -> int:
        self.registry.start_all()
        server = AskDaemonServer(
            spec=ASKD_SPEC,
            request_handler=self._handle_request,
            ...
        )
        return server.serve_forever()
```

### CLI 入口

`bin/` 下的腳本是使用者直接呼叫的 CLI 指令：

| 指令 | Provider | 功能 |
|------|----------|------|
| `cask` / `cpend` / `cping` | Codex | 發送/取回/Ping |
| `gask` / `gpend` / `gping` | Gemini | 發送/取回/Ping |
| `lask` / `lpend` / `lping` | Claude | 發送/取回/Ping |
| `oask` / `opend` / `oping` | OpenCode | 發送/取回/Ping |
| `dask` / `dpend` / `dping` | Droid | 發送/取回/Ping |

### Skill 入口

Claude Code 內部透過技能系統（`claude_skills/`）驅動跨 Provider 協作：

- `/ask gemini "問題"` -- 非同步委派至 Gemini
- `/tp` -- 啟動協作設計流程（designer + inspiration + reviewer）
- `/tr` -- 執行當前步驟
- `/all-plan` -- 完整協作規劃

---

## 3. 核心架構

### 3.1 多實例 Provider 管理

CCB 最核心的設計是**統一 Provider 適配器架構**。每個 AI Provider（Claude、Codex、Gemini、OpenCode、Droid、Copilot、CodeBuddy、Qwen）都實作同一個抽象介面：

```python
# lib/askd/adapters/base.py
class BaseProviderAdapter(ABC):
    @property
    @abstractmethod
    def key(self) -> str: ...           # 如 'codex', 'gemini', 'claude'

    @property
    @abstractmethod
    def session_filename(self) -> str: ...  # 如 '.codex-session'

    @abstractmethod
    def load_session(self, work_dir: Path, instance: Optional[str] = None) -> Optional[Any]: ...

    @abstractmethod
    def compute_session_key(self, session: Any, instance: Optional[str] = None) -> str: ...

    @abstractmethod
    def handle_task(self, task: QueuedTask) -> ProviderResult: ...

    def on_start(self) -> None: pass
    def on_stop(self) -> None: pass
```

**多實例支援**是一個重要特性。透過 `parse_qualified_provider()` 與 `make_qualified_key()` 函式，同一個 Provider 可以同時運行多個實例：

```python
# lib/providers.py
def parse_qualified_provider(key: str) -> tuple[str, str | None]:
    """Parse 'codex:auth' -> ('codex', 'auth'); 'codex' -> ('codex', None)."""
    parts = key.split(":", 1)
    base = parts[0].strip()
    instance = parts[1].strip() if len(parts) > 1 and parts[1].strip() else None
    return (base, instance)

def session_filename_for_instance(base_filename: str, instance: str | None) -> str:
    """'.codex-session' + 'auth' -> '.codex-auth-session'"""
    if not instance:
        return base_filename
    if base_filename.endswith("-session"):
        prefix = base_filename[:-len("-session")]
        return f"{prefix}-{instance}-session"
    return f"{base_filename}-{instance}"
```

這允許用戶同時運行：
- `codex:auth` -- 負責認證模組的 Codex 實例
- `codex:payment` -- 負責支付模組的 Codex 實例
- 每個實例有各自獨立的 Session 檔案與 pane

### 3.2 橋接通訊機制

CCB 的跨 Provider 通訊建立在三個層次上：

#### 層次 1：終端注入（Terminal Injection）

所有通訊的底層都是通過**終端面板注入文字**。CCB 不透過 API 直接呼叫 AI，而是將提示文字注入到 AI CLI 工具運行的終端面板中：

```python
# lib/terminal.py
class TerminalBackend(ABC):
    @abstractmethod
    def send_text(self, pane_id: str, text: str) -> None: ...
    @abstractmethod
    def is_alive(self, pane_id: str) -> bool: ...
    @abstractmethod
    def create_pane(self, cmd: str, cwd: str, ...) -> str: ...
```

tmux 實作使用 `load-buffer` + `paste-buffer` 機制處理多行文字：

```python
# lib/terminal.py -- TmuxBackend.send_text()
buffer_name = f"ccb-tb-{os.getpid()}-{int(time.time() * 1000)}-{random.randint(1000, 9999)}"
self._tmux_run(["load-buffer", "-b", buffer_name, "-"], input_bytes=sanitized.encode("utf-8"))
self._tmux_run(["paste-buffer", "-p", "-t", pane_id, "-b", buffer_name])
self._tmux_run(["send-keys", "-t", pane_id, "Enter"])
```

#### 層次 2：協議標記（Protocol Markers）

CCB 定義了一套基於文字標記的協議來追蹤請求-回應對應關係：

```python
# lib/ccb_protocol.py
REQ_ID_PREFIX = "CCB_REQ_ID:"
BEGIN_PREFIX = "CCB_BEGIN:"
DONE_PREFIX = "CCB_DONE:"

def wrap_codex_prompt(message: str, req_id: str) -> str:
    return (
        f"{REQ_ID_PREFIX} {req_id}\n\n"
        f"{message}\n\n"
        "IMPORTANT:\n"
        "- Reply normally.\n"
        "- End your reply with this exact final line (verbatim, on its own line):\n"
        f"{DONE_PREFIX} {req_id}\n"
    )
```

每個請求都帶有唯一 `req_id`（格式：`YYYYMMDD-HHMMSS-mmm-PID-counter`），AI 被指示在回覆末尾附上 `CCB_DONE: <req_id>` 標記。CCB 持續輪詢日誌檔等待此標記出現。

#### 層次 3：日誌讀取（Log-based Response Reading）

CCB 不直接讀取終端輸出，而是讀取 AI CLI 的**結構化 Session 日誌檔案**（JSONL 格式）：

```python
# lib/claude_comm.py
class ClaudeLogReader:
    """Reads Claude session logs from ~/.claude/projects/<key>"""

    def _read_new_messages(self, session: Path, state: dict):
        # 增量讀取：使用 offset 追蹤已讀位置
        offset = int(state.get("offset") or 0)
        with session.open("rb") as handle:
            handle.seek(offset)
            data = handle.read()
        # 解析 JSONL 行
        for raw in lines:
            entry = json.loads(line.decode("utf-8", errors="replace"))
            msg = _extract_message(entry, "assistant")
```

這個設計非常聰明：
- 利用 Claude Code / Codex / Gemini 自帶的 JSONL 日誌
- 增量讀取（offset-based）避免重複處理
- 支援子代理日誌（`subagents/` 目錄）
- 支援 `sessions-index.json` 快速定位最新 Session

#### 層次 4：MCP 委派伺服器

對於無法直接使用 CLI 的場景，CCB 提供 MCP stdio 伺服器（位於 `mcp/ccb-delegation/server.py`）：

```python
# mcp/ccb-delegation/server.py
TOOL_DEFS = []
for provider in ("codex", "gemini", "claude", "opencode"):
    TOOL_DEFS.append({
        "name": f"ccb_ask_{provider}",
        "description": f"Submit a background request to {provider} (CCB).",
        "inputSchema": _ask_schema(),
    })
```

這讓任何支援 MCP 的 AI 工具都能透過 JSON-RPC 2.0 發送跨 Provider 請求。

### 3.3 面板管理（Pane Management）

CCB 對終端面板的管理極為精細，包含三個核心機制：

#### 面板註冊表（Pane Registry）

位於 `~/.ccb/run/ccb-session-*.json`，每個 CCB Session 維護一個 JSON 檔案，記錄所有 Provider 的面板映射：

```python
# lib/pane_registry.py
REGISTRY_TTL_SECONDS = 7 * 24 * 60 * 60  # 7天過期

def upsert_registry(record: dict) -> bool:
    # 結構化的 providers map
    data["providers"] = {
        "claude": { "pane_id": "%12", "pane_title_marker": "CCB_claude_xxx" },
        "codex": { "pane_id": "%13", ... },
        "gemini": { "pane_id": "%14", ... },
    }
    data["updated_at"] = int(time.time())
    atomic_write_text(path, json.dumps(data))
```

#### 面板健康檢查與自動修復

`ensure_pane()` 方法實作了多層面板恢復策略：

```python
# lib/caskd_session.py -- CodexProjectSession.ensure_pane()
def ensure_pane(self) -> Tuple[bool, str]:
    backend = self.backend()

    # 策略 1：直接檢查 pane_id 是否存活
    if pane_id and backend.is_alive(pane_id):
        return True, pane_id

    # 策略 2：透過 title marker 重新定位面板
    if marker and callable(resolver):
        resolved = resolver(marker)
        if resolved and backend.is_alive(str(resolved)):
            self.data["pane_id"] = str(resolved)  # 更新 stale ID
            self._write_back()
            return True, str(resolved)

    # 策略 3：tmux 自癒 -- 對死亡面板執行 respawn
    if self.terminal == "tmux":
        start_cmd = self.start_cmd
        if start_cmd and callable(respawn):
            respawn(str(target), cmd=start_cmd, cwd=self.work_dir, remain_on_exit=True)
            if backend.is_alive(str(target)):
                return True, str(target)
```

#### Title Marker 機制

每個面板被分配一個唯一的 title marker（如 `CCB_codex_abc123`），即使 tmux pane ID 因為重啟而改變，CCB 仍可通過 marker 重新定位：

```python
# lib/terminal.py -- TmuxBackend.find_pane_by_title_marker()
def find_pane_by_title_marker(self, marker: str) -> Optional[str]:
    cp = self._tmux_run(["list-panes", "-a", "-F", "#{pane_id}\t#{pane_title}"], capture=True)
    for line in (cp.stdout or "").splitlines():
        pid, title = line.split("\t", 1)
        if (title or "").startswith(marker):
            if self._looks_like_pane_id(pid):
                return pid
    return None
```

### 3.4 Stale ID 處理

Stale ID 是 CCB 面臨的核心難題之一。當 tmux pane 被殺死並重建時，舊的 `%xx` pane ID 失效。CCB 通過多層策略處理：

#### A. 環境變數 `$TMUX_PANE` 的 stale 偵測

```python
# lib/terminal.py -- TmuxBackend.get_current_pane_id()
def get_current_pane_id(self) -> str:
    # $TMUX_PANE 可能過時（面板被殺/取代後）
    env_pane = (os.environ.get("TMUX_PANE") or "").strip()
    if self._looks_like_pane_id(env_pane) and self.pane_exists(env_pane):
        return env_pane  # 只在面板確實存在時才信任

    # 退路：查詢 tmux 當前焦點面板
    cp = self._tmux_run(["display-message", "-p", "#{pane_id}"], capture=True)
    ...
```

#### B. 註冊表的 TTL 過期機制

```python
# lib/pane_registry.py
def _is_stale(updated_at: int, now: Optional[int] = None) -> bool:
    if updated_at <= 0:
        return True
    now_ts = int(time.time()) if now is None else int(now)
    return (now_ts - updated_at) > REGISTRY_TTL_SECONDS  # 7天
```

#### C. Codex 日誌的 stale 偵測與切換

```python
# lib/askd/adapters/codex.py -- CodexAdapter.handle_task()
# 等待回應時檢測：如果沒有 anchor 且沒有 chunks，可能日誌已過時
if (not anchor_seen) and (not chunks):
    if now - started_at >= stale_grace_s:
        latest_log = _scan_latest_any_log(Path(session.work_dir))
        if latest_log and latest_log != current_log:
            if _is_log_stale(current_log, latest_log, stale_threshold_s):
                # 切換到新日誌
                reader = CodexLogReader(log_path=latest_log, ...)
                state = reader.capture_state()
                session.update_codex_log_binding(log_path=str(latest_log), ...)
```

#### D. Session 路徑的一致性修復

```python
# lib/claude_session_resolver.py
def _normalize_session_binding(data: dict, work_dir: Path) -> None:
    sid = str(data.get("claude_session_id") or "").strip()
    path = Path(data.get("claude_session_path") or "")

    if path and path.exists():
        if sid and path.stem != sid:
            # ID 與路徑不匹配 -> 嘗試用 ID 找到正確路徑
            candidate = _session_path_from_id(sid, work_dir)
            if candidate and candidate.exists():
                data["claude_session_path"] = str(candidate)
            else:
                data["claude_session_id"] = path.stem
```

### 3.5 Worker Pool 與任務排程

```python
# lib/worker_pool.py
class BaseSessionWorker(threading.Thread, Generic[TaskT, ResultT]):
    """每個 Session 一個 Worker 執行緒"""
    def run(self):
        while not self._stop_event.is_set():
            task = self._q.get(timeout=0.2)
            if hasattr(task, 'cancelled') and task.cancelled:
                task.done_event.set()
                continue
            task.result = self._handle_task(task)
            task.done_event.set()

class PerSessionWorkerPool(Generic[WorkerT]):
    """按 Session 隔離的 Worker 池"""
    def get_or_create(self, session_key, factory):
        with self._lock:
            worker = self._workers.get(session_key)
            if worker is not None and not worker.is_alive():
                # Worker 死亡 -> 移除並重建
                self._workers.pop(session_key, None)
                worker = None
            if worker is None:
                worker = factory(session_key)
                self._workers[session_key] = worker
                worker.start()
        return worker
```

這保證了同一個 Session 的請求是序列化處理的，不會並行衝突。

### 3.6 角色系統與協作流程

CCB 定義了抽象角色（不綁定特定 Provider）：

```markdown
<!-- .clinerules -->
| Role | Provider | Description |
|------|----------|-------------|
| designer | claude | Primary planner and architect |
| inspiration | gemini | Creative brainstorming (unreliable, never blindly follow) |
| reviewer | codex | Scored quality gate (Rubrics) |
| executor | claude | Code implementation |
```

協作流程（`/all-plan`）：
1. 需求澄清（5 維度）
2. `inspiration`（Gemini）腦力激盪
3. `designer`（Claude）獨立制定計畫
4. `reviewer`（Codex）以 Rubric 評分（需 >= 7.0）
5. 自動修正迴圈（最多 3 輪）

---

## 4. 用例與解決的問題

### 4.1 多 AI CLI 統一管理

**問題**：開發者需要同時使用 Claude Code、Codex CLI、Gemini CLI 等多個 AI 工具，但各自獨立，無法協作。

**解決方案**：CCB 提供統一的守護程式，在 tmux 中管理多個面板，每個面板運行不同的 AI CLI。通過文字注入和日誌讀取實現透明通訊。

### 4.2 跨 Agent 委派

**問題**：Claude 想要讓 Codex 執行某個任務、或讓 Gemini 提供創意想法。

**解決方案**：`/ask codex "重構這段程式碼"` -- Claude 通過技能系統將任務委派給 Codex，等待結果後繼續。

### 4.3 任務計劃與執行分離

**問題**：複雜任務需要規劃、審核、執行的流水線。

**解決方案**：
- `/tp` (Task Plan) 生成結構化計劃（`todo.md` + `state.json` + `plan_log.md`）
- `/tr` (Task Run) 按步驟執行，支持跨 context window 恢復
- Designer/Reviewer 分離確保品質

### 4.4 Session 生存性

**問題**：tmux pane 重啟、AI CLI crash 後，整個 Session 鏈斷裂。

**解決方案**：Title Marker 重定位 + respawn 自癒 + 日誌 stale 偵測與自動切換。

### 4.5 Memory-First 架構

CCB 提出了完整的 Memory-First Agent 架構設計（`docs/memory-first-agent-architecture.md`）：

- **A 角色 (Memory Keeper)**：維護長期記憶
- **B 角色 (Context Builder)**：組裝短期記憶 + 上下文
- **C 角色 (Executor)**：無狀態執行
- **T 角色 (Task Tracker)**：中期任務記憶（跨 window）

---

## 5. 值得採用的關鍵模式

### 5.1 Provider Adapter 模式

CCB 的 `BaseProviderAdapter` 是一個乾淨的 Provider 抽象。關鍵方法：

- `load_session()` -- 加載 Provider 的 Session 狀態
- `compute_session_key()` -- 計算路由鍵（確保專案+Provider 隔離）
- `handle_task()` -- 處理請求的完整生命週期
- `on_start()` / `on_stop()` -- 生命週期鉤子

**clawtex 可參考之處**：clawtex 的 `src/providers/` 已有 Provider trait，但 CCB 的多實例支援（`codex:auth`）可以強化 clawtex 的 Provider 分片能力。

### 5.2 Title Marker + Registry 的 Pane 管理

CCB 用 title marker 作為面板的「穩定識別符」，pane ID 只是「當前綁定」。這個 indirection layer 解決了 tmux pane ID 不穩定的問題。

**程式碼模式**：
```
pane_title_marker (穩定) --> pane_id (不穩定, 可重解析)
                         --> 註冊表 JSON (持久化)
                         --> ensure_pane() (自癒)
```

### 5.3 文字協議標記（CCB_REQ_ID / CCB_DONE）

這是一個極簡但實用的請求追蹤方案。不需要修改 AI CLI 的程式碼，只需在提示詞中嵌入標記，讓 AI 在回覆中附上完成標記。

**clawtex 可參考之處**：clawtex 的 `delegate_to_provider` 工具可以借鑑此模式，實現跨 Provider 的非同步任務追蹤。

### 5.4 Per-Session Worker Pool

每個 Session 獨立的 Worker 執行緒確保了：
- 同一 Session 的請求序列化處理
- 不同 Session 可並行
- Worker 死亡自動重建

```python
class PerSessionWorkerPool:
    def get_or_create(self, session_key, factory):
        # 自動偵測死亡 worker 並重建
        if worker is not None and not worker.is_alive():
            self._workers.pop(session_key, None)
            worker = None
```

### 5.5 增量日誌讀取

基於 offset 的增量讀取模式，避免重複解析整個日誌檔案：

```python
state = {"session_path": session, "offset": new_offset, "carry": carry}
# carry 保存不完整的最後一行，下次繼續拼接
```

### 5.6 角色抽象化

角色（designer/inspiration/reviewer/executor）與具體 Provider 解耦。切換 Provider 只需修改角色映射表，不需改動技能邏輯。

---

## 6. 與 clawtex-core 的關聯性分析

### 6.1 叢集系統參考價值

| CCB 概念 | clawtex 對應 | 參考價值 |
|----------|-------------|---------|
| `UnifiedAskDaemon` | `cluster_hub.rs` (ClusterHub) | CCB 的單程序多 Provider 守護模式比 clawtex 的 HTTP-based hub 更輕量 |
| `PerSessionWorkerPool` | Worker 排程 | clawtex 可借鑑 Per-Session 序列化模式避免同一 agent 的並行衝突 |
| `pane_registry.py` | Worker 註冊 | CCB 的 JSON 檔案註冊表比 clawtex 的 heartbeat 機制更簡單 |
| `ensure_pane()` 自癒 | Worker 重連 | clawtex 可加入 Worker 自動恢復機制 |
| `ProviderAdapter` | Provider trait | 非常相似，CCB 的多實例（`codex:auth`）設計可擴展到 clawtex |

### 6.2 多 Agent 協調參考價值

| CCB 概念 | clawtex 對應 | 參考價值 |
|----------|-------------|---------|
| `CCB_REQ_ID/CCB_DONE` 協議 | `delegate` tool | clawtex 的 delegate 可加入標記追蹤機制 |
| 角色系統 | Hands 角色 | CCB 的角色抽象值得採用；clawtex 的 Hands 已有類似概念 |
| `/tp` + `/tr` 流程 | Hands 多階段 | CCB 的任務計劃持久化（`state.json`）比 clawtex 的 Hands 更結構化 |
| Memory-First 架構 | `memory_store/recall` | CCB 的三層記憶模型（L1/L2/L3）比 clawtex 的單層 SQLite 更完善 |
| MCP 委派伺服器 | MCP Client | clawtex 是 MCP Client；CCB 同時也是 MCP Server |

### 6.3 具體可整合的功能

#### A. 終端注入橋接

clawtex 可以增加一個「終端橋接模式」，讓 Worker 不需要 HTTP API，而是通過 tmux 面板注入文字來控制本地的 Claude Code / Codex 實例。這對 AYANEO NPU 等受限設備特別有用。

#### B. Stale Session 自動恢復

clawtex 的 `cluster_worker.rs` 可以參考 CCB 的 `ensure_pane()` 模式，增加：
- Worker 連線斷開後自動重連
- heartbeat 失敗後的自動重啟
- Session 狀態持久化與恢復

#### C. 請求追蹤標記

clawtex 的 `delegate_to_provider` 工具可以借鑑 `CCB_REQ_ID` / `CCB_DONE` 模式，實現：
- 非同步任務提交（`ask`）
- 結果輪詢/等待（`pend`）
- 健康檢查（`ping`）

#### D. 多實例 Provider

clawtex 目前的 Provider 是一對一映射。可以參考 CCB 的 `codex:auth` 模式，允許同一個 Provider 同時運行多個實例（如 `ollama:code-review`、`ollama:generation`），各自有不同的模型配置。

---

## 附錄：關鍵檔案路徑索引

| 功能 | 檔案路徑 |
|------|---------|
| 統一守護程式 | `lib/askd/daemon.py` |
| Provider 註冊表 | `lib/askd/registry.py` |
| Provider 適配器介面 | `lib/askd/adapters/base.py` |
| Claude 適配器 | `lib/askd/adapters/claude.py` |
| Codex 適配器 | `lib/askd/adapters/codex.py` |
| Gemini 適配器 | `lib/askd/adapters/gemini.py` |
| 終端後端抽象 | `lib/terminal.py` |
| Claude 通訊 | `lib/claude_comm.py` |
| 面板註冊表 | `lib/pane_registry.py` |
| Session 工具 | `lib/session_utils.py` |
| Session 解析器 | `lib/claude_session_resolver.py` |
| 協議定義 | `lib/ccb_protocol.py` |
| Provider Spec | `lib/providers.py` |
| Worker Pool | `lib/worker_pool.py` |
| Codex Session（ensure_pane 自癒） | `lib/caskd_session.py` |
| MCP 委派伺服器 | `mcp/ccb-delegation/server.py` |
| 雙向橋接 | `lib/codex_dual_bridge.py` |
| Memory-First 架構文件 | `docs/memory-first-agent-architecture.md` |
| 角色定義 | `.clinerules` |
| 協作規劃技能 | `claude_skills/all-plan/` |
| 任務計劃技能 | `claude_skills/tp/` |
| 任務執行技能 | `claude_skills/tr/` |

> 所有路徑相對於 `references/claude_code_bridge/`
