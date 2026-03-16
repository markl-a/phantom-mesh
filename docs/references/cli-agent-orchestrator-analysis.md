# CLI Agent Orchestrator (CAO) - AWS Labs 深度技術分析

> **專案來源**: https://github.com/awslabs/cli-agent-orchestrator
> **版本**: v1.1.0
> **授權**: Apache-2.0
> **分析日期**: 2026-03-12
> **分析者**: Claude Opus 4.6 for clawtex-core 參考架構

---

## 目錄

1. [專案結構](#1-專案結構)
2. [進入點與 CLI 命令](#2-進入點與-cli-命令)
3. [核心架構](#3-核心架構)
4. [多 Agent 協調機制](#4-多-agent-協調機制)
5. [終端管理系統](#5-終端管理系統)
6. [錯誤處理與容錯機制](#6-錯誤處理與容錯機制)
7. [值得採用的設計模式](#7-值得採用的設計模式)
8. [與 clawtex-core 的整合可能性](#8-與-clawtex-core-的整合可能性)

---

## 1. 專案結構

### 1.1 目錄樹總覽

```
cli-agent-orchestrator/
├── pyproject.toml                    # 建置設定、三個入口點
├── cliff.toml                        # changelog 生成設定
├── .tmux.conf                        # tmux 設定
├── src/cli_agent_orchestrator/
│   ├── __init__.py
│   ├── constants.py                  # 全域常數（路徑、埠號、Provider 列表）
│   ├── agent_store/                  # 內建 Agent 設定檔（Markdown + Frontmatter）
│   │   ├── code_supervisor.md
│   │   ├── developer.md
│   │   └── reviewer.md
│   ├── api/
│   │   └── main.py                   # FastAPI HTTP 伺服器（cao-server）
│   ├── cli/
│   │   ├── main.py                   # Click CLI 群組（cao 命令）
│   │   └── commands/
│   │       ├── launch.py             # cao launch — 建立 tmux session
│   │       ├── init.py               # cao init — 初始化設定
│   │       ├── install.py            # cao install — 安裝 provider
│   │       ├── shutdown.py           # cao shutdown — 關閉所有 session
│   │       ├── flow.py               # cao flow — 排程管理
│   │       ├── mcp_server.py         # cao mcp-server — 啟動 MCP server
│   │       └── info.py               # cao info — 顯示系統資訊
│   ├── clients/
│   │   ├── tmux.py                   # TmuxClient singleton（libtmux 封裝）
│   │   └── database.py              # SQLAlchemy SQLite 資料層
│   ├── mcp_server/
│   │   ├── server.py                 # FastMCP server（handoff, assign, send_message）
│   │   ├── models.py                 # HandoffResult 模型
│   │   └── utils.py                  # 資料庫查詢工具
│   ├── models/
│   │   ├── terminal.py               # Terminal, TerminalStatus 模型
│   │   ├── session.py                # Session, SessionStatus 模型
│   │   ├── provider.py               # ProviderType 列舉
│   │   ├── agent_profile.py          # AgentProfile, McpServer 模型
│   │   ├── flow.py                   # Flow 模型（排程）
│   │   ├── inbox.py                  # InboxMessage 模型
│   │   ├── kiro_agent.py             # Kiro Agent 特定模型
│   │   └── q_agent.py                # Q CLI 特定模型
│   ├── providers/
│   │   ├── base.py                   # BaseProvider 抽象基底類別
│   │   ├── manager.py                # ProviderManager singleton
│   │   ├── claude_code.py            # ClaudeCodeProvider
│   │   ├── codex.py                  # CodexProvider
│   │   ├── kiro_cli.py               # KiroCliProvider
│   │   ├── q_cli.py                  # QCliProvider
│   │   └── gemini_cli.py             # GeminiCliProvider
│   ├── services/
│   │   ├── terminal_service.py       # Terminal 生命週期管理
│   │   ├── session_service.py        # Session 管理
│   │   ├── inbox_service.py          # Agent 間訊息佇列
│   │   ├── flow_service.py           # 排程工作流
│   │   └── cleanup_service.py        # 資料清理
│   └── utils/
│       ├── terminal.py               # ID 生成、狀態輪詢工具
│       ├── agent_profiles.py         # Agent 設定檔載入
│       ├── template.py               # 模板渲染
│       └── logging.py                # 日誌設定
├── test/                             # 測試（unit, integration, e2e, provider）
├── examples/                         # 使用範例（assign, codex-basic, flow, cross-provider）
└── docs/                             # 文件（API, Provider 指南）
```

### 1.2 依賴總覽

```toml
# pyproject.toml — 核心依賴
dependencies = [
    "fastapi>=0.104.0",       # HTTP API 伺服器
    "fastmcp>=2.14.0",        # MCP server 框架
    "mcp>=1.23.0",            # MCP 協議
    "pydantic>=2.10.6",       # 資料驗證
    "apscheduler>=3.10.4",    # Cron 排程
    "sqlalchemy>=2.0.0",      # ORM (SQLite)
    "uvicorn[standard]>=0.24.0",  # ASGI server
    "libtmux>=0.51.0",        # tmux Python 綁定
    "click>=8.0.0",           # CLI 框架
    "python-frontmatter>=1.1.0",  # Markdown frontmatter 解析
    "watchdog==6.0.0",        # 檔案系統監視
    "requests>=2.32.0",       # HTTP client
]
```

**關鍵依賴**: `libtmux` 是整個系統的基石 -- 所有 CLI Agent 都透過 tmux session/window 來管理。

### 1.3 三個進入點

```toml
[project.scripts]
"cao" = "cli_agent_orchestrator.cli.main:cli"              # CLI 工具
"cao-server" = "cli_agent_orchestrator.api.main:main"      # FastAPI 伺服器
"cao-mcp-server" = "cli_agent_orchestrator.mcp_server.server:main"  # MCP server
```

---

## 2. 進入點與 CLI 命令

### 2.1 CLI 主入口 (`cao`)

**檔案**: `src/cli_agent_orchestrator/cli/main.py`

```python
@click.group()
def cli():
    """CLI Agent Orchestrator."""

# 註冊子命令
cli.add_command(launch)    # 建立新 session 並啟動 agent
cli.add_command(init)      # 初始化專案設定
cli.add_command(install)   # 安裝 provider CLI 工具
cli.add_command(shutdown)  # 關閉所有 CAO session
cli.add_command(flow)      # 排程管理
cli.add_command(mcp_server)# 啟動 MCP server
cli.add_command(info)      # 顯示系統資訊
```

### 2.2 launch 命令 — 核心啟動流程

**檔案**: `src/cli_agent_orchestrator/cli/commands/launch.py`

```python
@click.command()
@click.option("--agents", required=True, help="Agent profile to launch")
@click.option("--provider", default=DEFAULT_PROVIDER)
@click.option("--session-name", help="自訂 session 名稱")
@click.option("--headless", is_flag=True, help="背景模式")
@click.option("--yolo", is_flag=True, help="跳過工作區信任確認")
def launch(agents, session_name, headless, provider, yolo):
```

**啟動流程**:
1. 驗證 provider 類型
2. 工作區信任確認（`--yolo` 可跳過）
3. 呼叫 `cao-server` API 建立 session
4. 若非 headless 模式，則 `tmux attach-session`

### 2.3 API 伺服器 (`cao-server`)

**檔案**: `src/cli_agent_orchestrator/api/main.py`

FastAPI 應用程式，提供完整的 REST API：

| 端點 | 方法 | 功能 |
|------|------|------|
| `/health` | GET | 健康檢查 |
| `/sessions` | POST | 建立新 session + terminal |
| `/sessions` | GET | 列出所有 session |
| `/sessions/{name}` | GET/DELETE | 取得/刪除 session |
| `/sessions/{name}/terminals` | POST/GET | 在 session 中建立/列出 terminal |
| `/terminals/{id}` | GET/DELETE | 取得/刪除 terminal |
| `/terminals/{id}/input` | POST | 傳送輸入至 agent |
| `/terminals/{id}/output` | GET | 取得 agent 輸出 |
| `/terminals/{id}/exit` | POST | 發送退出命令 |
| `/terminals/{id}/working-directory` | GET | 取得工作目錄 |
| `/terminals/{id}/inbox/messages` | POST/GET | 收件匣訊息管理 |

**生命週期管理** (lifespan):
```python
@asynccontextmanager
async def lifespan(app: FastAPI):
    init_db()                           # 初始化 SQLite
    asyncio.create_task(cleanup_old_data)  # 清理舊資料
    daemon_task = asyncio.create_task(flow_daemon())  # 排程守護程序
    inbox_observer = PollingObserver()   # 收件匣監視
    inbox_observer.schedule(LogFileHandler(), TERMINAL_LOG_DIR)
    inbox_observer.start()
    yield
    inbox_observer.stop()
    daemon_task.cancel()
```

---

## 3. 核心架構

### 3.1 Provider 系統 — CLI Agent 抽象層

這是整個 CAO 最精妙的設計。每個外部 CLI agent（Claude Code、Codex、Kiro、Q CLI、Gemini CLI）都被封裝為一個 `BaseProvider` 的實作。

#### 3.1.1 BaseProvider 抽象介面

**檔案**: `src/cli_agent_orchestrator/providers/base.py`

```python
class BaseProvider(ABC):
    def __init__(self, terminal_id: str, session_name: str, window_name: str):
        self.terminal_id = terminal_id
        self.session_name = session_name
        self.window_name = window_name
        self._status = TerminalStatus.IDLE

    @property
    def paste_enter_count(self) -> int:
        """貼上文字後需要按幾次 Enter。預設 2 次（TUI 多行模式）。"""
        return 2

    @abstractmethod
    def initialize(self) -> bool:
        """初始化 CLI 工具（例如啟動 claude 命令）"""

    @abstractmethod
    def get_status(self, tail_lines: Optional[int] = None) -> TerminalStatus:
        """透過分析終端輸出判斷目前狀態"""

    @abstractmethod
    def get_idle_pattern_for_log(self) -> str:
        """取得 log 檔案中的 IDLE 模式（用於快速偵測）"""

    @abstractmethod
    def extract_last_message_from_script(self, script_output: str) -> str:
        """從終端輸出中提取最後一則訊息"""

    @abstractmethod
    def exit_cli(self) -> str:
        """取得退出命令"""

    @abstractmethod
    def cleanup(self) -> None:
        """清理資源"""

    def mark_input_received(self) -> None:
        """通知 provider 已收到外部輸入（用於狀態偵測調整）"""
```

#### 3.1.2 ProviderType 列舉

**檔案**: `src/cli_agent_orchestrator/models/provider.py`

```python
class ProviderType(str, Enum):
    Q_CLI = "q_cli"
    KIRO_CLI = "kiro_cli"
    CLAUDE_CODE = "claude_code"
    CODEX = "codex"
    GEMINI_CLI = "gemini_cli"
```

#### 3.1.3 TerminalStatus 狀態機

**檔案**: `src/cli_agent_orchestrator/models/terminal.py`

```python
class TerminalStatus(str, Enum):
    IDLE = "idle"                        # 等待輸入
    PROCESSING = "processing"            # 正在處理
    COMPLETED = "completed"              # 已完成回應
    WAITING_USER_ANSWER = "waiting_user_answer"  # 等待使用者確認
    ERROR = "error"                      # 錯誤狀態
```

#### 3.1.4 五大 Provider 實作對比

| Provider | 檔案 | 啟動命令 | IDLE 偵測 | 回應提取 | 退出命令 | 特殊處理 |
|----------|------|----------|-----------|----------|----------|----------|
| **ClaudeCodeProvider** | `claude_code.py` | `claude --dangerously-skip-permissions` | `[>❯]\s` prompt | `⏺` 標記 | `/exit` | 工作區信任對話框自動確認 |
| **CodexProvider** | `codex.py` | `codex --no-alt-screen --disable shell_snapshot` | `(?:❯\|›\|codex>)` | User/Assistant 前綴 | `/exit` | TUI footer 截止計算、暖機命令 |
| **KiroCliProvider** | `kiro_cli.py` | `kiro-cli chat --agent <profile>` | `[profile] >` prompt | 綠色箭頭 `>` 標記 | `/exit` | 權限提示偵測（y/n/t） |
| **QCliProvider** | `q_cli.py` | `q chat --agent <profile>` | `[profile] >` prompt | 綠色箭頭 `>` 標記 | `/exit` | 與 Kiro 類似但獨立實作 |
| **GeminiCliProvider** | `gemini_cli.py` | `gemini --yolo --sandbox false` | `* Type your message` | `✦` 前綴行 | `C-d` (Ctrl+D) | GEMINI.md 注入、MCP 設定寫入 settings.json、Ink TUI spinner 偵測 |

#### 3.1.5 ClaudeCodeProvider 深度分析

**檔案**: `src/cli_agent_orchestrator/providers/claude_code.py`

這是對 clawtex-core 最相關的 provider。其狀態偵測基於正規表達式解析終端 ANSI 輸出：

```python
# 正規表達式模式
ANSI_CODE_PATTERN = r"\x1b\[[0-9;]*m"
RESPONSE_PATTERN = r"⏺(?:\x1b\[[0-9;]*m)*\s+"   # Claude 回應標記
PROCESSING_PATTERN = r"[✶✢✽✻✳].*…"              # 思考中 spinner
IDLE_PROMPT_PATTERN = r"[>❯][\s\xa0]"             # 閒置提示
TRUST_PROMPT_PATTERN = r"Yes, I trust this folder" # 信任對話框

def get_status(self, tail_lines=None) -> TerminalStatus:
    output = tmux_client.get_history(self.session_name, self.window_name, tail_lines)
    if not output:
        return TerminalStatus.ERROR
    # 優先順序：PROCESSING > WAITING_USER_ANSWER > COMPLETED > IDLE > ERROR
    if re.search(PROCESSING_PATTERN, output):
        return TerminalStatus.PROCESSING
    if re.search(WAITING_USER_ANSWER_PATTERN, output) and not TRUST_PROMPT:
        return TerminalStatus.WAITING_USER_ANSWER
    if re.search(RESPONSE_PATTERN, output) and re.search(IDLE_PROMPT_PATTERN, output):
        return TerminalStatus.COMPLETED
    if re.search(IDLE_PROMPT_PATTERN, output):
        return TerminalStatus.IDLE
    return TerminalStatus.ERROR
```

**初始化流程**:
1. `wait_for_shell()` — 等待 shell 就緒
2. 建構含 `--dangerously-skip-permissions` 和可選 `--append-system-prompt`、`--mcp-config` 的命令
3. `send_keys()` 透過 tmux 傳送命令
4. `_handle_trust_prompt()` — 自動確認工作區信任
5. `wait_until_status(IDLE | COMPLETED)` — 等待 Claude Code 就緒

#### 3.1.6 GeminiCliProvider 深度分析 — 最複雜的實作

**檔案**: `src/cli_agent_orchestrator/providers/gemini_cli.py` (685 行)

Gemini CLI 是最複雜的 provider，因為：
- **Ink TUI 行為**: idle prompt 在處理過程中也持續可見
- **Processing Spinner**: 需要偵測 Braille dots (`⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`)
- **系統提示注入**: 寫入 `GEMINI.md` 檔案 + `-i` 旗標
- **MCP 設定**: 直接修改 `~/.gemini/settings.json`
- **清理**: 需要恢復原始 GEMINI.md、移除 MCP server 註冊

```python
# Gemini 的 _received_input_after_init 機制
# 解決 -i 旗標處理後狀態偵測的問題
def mark_input_received(self) -> None:
    self._received_input_after_init = True

def get_status(self, tail_lines=None) -> TerminalStatus:
    # ...
    if has_idle_prompt:
        has_spinner = any(re.search(PROCESSING_SPINNER_PATTERN, line) for line in bottom_lines)
        if has_spinner and not (has_query and has_response):
            return TerminalStatus.PROCESSING
        if has_query and has_response:
            # 關鍵：初始化完成後、收到外部輸入前，回傳 IDLE 而非 COMPLETED
            if self._initialized and self._uses_prompt_interactive and not self._received_input_after_init:
                return TerminalStatus.IDLE
            return TerminalStatus.COMPLETED
        return TerminalStatus.IDLE
```

#### 3.1.7 ProviderManager — Provider 工廠與生命週期

**檔案**: `src/cli_agent_orchestrator/providers/manager.py`

```python
class ProviderManager:
    def __init__(self):
        self._providers: Dict[str, BaseProvider] = {}  # terminal_id -> provider

    def create_provider(self, provider_type, terminal_id, tmux_session, tmux_window, agent_profile=None) -> BaseProvider:
        """工廠方法：根據 provider_type 建立對應的 provider 實例"""
        if provider_type == ProviderType.CLAUDE_CODE.value:
            provider = ClaudeCodeProvider(terminal_id, tmux_session, tmux_window, agent_profile)
        elif provider_type == ProviderType.CODEX.value:
            provider = CodexProvider(terminal_id, tmux_session, tmux_window, agent_profile)
        # ... 其他 provider
        self._providers[terminal_id] = provider
        return provider

    def get_provider(self, terminal_id) -> Optional[BaseProvider]:
        """取得 provider，若不存在則從資料庫元資料重建"""
        provider = self._providers.get(terminal_id)
        if provider:
            return provider
        metadata = get_terminal_metadata(terminal_id)  # 從 SQLite 查詢
        return self.create_provider(...)  # 隨需重建

# 模組級 singleton
provider_manager = ProviderManager()
```

### 3.2 Tmux 整合

#### 3.2.1 TmuxClient 封裝

**檔案**: `src/cli_agent_orchestrator/clients/tmux.py`

`TmuxClient` 是 `libtmux` 的簡化封裝，以模組級 singleton 模式提供：

```python
class TmuxClient:
    def __init__(self):
        self.server = libtmux.Server()

    def create_session(self, session_name, window_name, terminal_id, working_directory=None) -> str:
        """建立 detached tmux session"""
        environment = os.environ.copy()
        environment["CAO_TERMINAL_ID"] = terminal_id  # 注入環境變數
        session = self.server.new_session(
            session_name=session_name,
            window_name=window_name,
            start_directory=working_directory,
            detach=True,
            environment=environment,
        )

    def send_keys(self, session_name, window_name, keys, enter_count=1):
        """透過 paste-buffer 傳送文字（避免逐字輸入與特殊字元問題）"""
        buf_name = f"cao_{uuid.uuid4().hex[:8]}"
        subprocess.run(["tmux", "load-buffer", "-b", buf_name, "-"], input=keys.encode())
        subprocess.run(["tmux", "paste-buffer", "-p", "-b", buf_name, "-t", target])
        time.sleep(0.3)  # 等待 TUI 處理 bracketed paste
        for i in range(enter_count):
            if i > 0:
                time.sleep(0.5)  # Enter 間的延遲
            subprocess.run(["tmux", "send-keys", "-t", target, "Enter"])

    def get_history(self, session_name, window_name, tail_lines=None) -> str:
        """透過 capture-pane 取得終端歷史（含 ANSI 跳脫序列）"""
        result = pane.cmd("capture-pane", "-e", "-p", "-S", f"-{lines}")
        return "\n".join(result.stdout)

    def pipe_pane(self, session_name, window_name, file_path):
        """將 pane 輸出串流至檔案（用於 inbox 監視）"""
        pane.cmd("pipe-pane", "-o", f"cat >> {file_path}")
```

**關鍵設計決策**:
- 使用 `paste-buffer -p`（bracketed paste mode）而非 `send-keys` 傳送文字，避免 TUI 熱鍵問題
- Enter 按鍵間需要 `time.sleep(0.3-0.5)` 延遲，因為 Ink 等 TUI 框架需要時間處理
- `capture-pane -e` 保留 ANSI 跳脫序列，provider 可用於狀態偵測
- `pipe-pane` 將輸出串流至 log 檔案，配合 watchdog 實現非同步訊息遞送

#### 3.2.2 安全性 — 路徑驗證

```python
_BLOCKED_DIRECTORIES = frozenset({
    "/", "/bin", "/sbin", "/usr/bin", "/usr/sbin", "/etc", "/var",
    "/tmp", "/dev", "/proc", "/sys", "/root", "/boot", "/lib", "/lib64",
    "/private/etc", "/private/var", "/private/tmp",
})

def _resolve_and_validate_working_directory(self, working_directory):
    real_path = os.path.realpath(os.path.abspath(working_directory))
    if real_path in self._BLOCKED_DIRECTORIES:
        raise ValueError(f"Working directory not allowed: {real_path}")
```

### 3.3 任務路由

CAO 的任務路由不是基於模型能力的智慧路由，而是基於 **Agent Profile** 的角色分配：

#### 3.3.1 Agent Profile 系統

**檔案**: `src/cli_agent_orchestrator/models/agent_profile.py`

```python
class AgentProfile(BaseModel):
    name: str
    description: str
    provider: Optional[str] = None  # Provider 覆寫
    system_prompt: Optional[str] = None
    mcpServers: Optional[Dict[str, Any]] = None
    tools: Optional[List[str]] = None
    # ... Q CLI 特有欄位
```

Agent Profile 以 Markdown + Frontmatter 格式儲存：

```markdown
---
name: code_supervisor
description: Coding Supervisor Agent in a multi-agent system
mcpServers:
  cao-mcp-server:
    type: stdio
    command: uvx
    args: ["--from", "git+https://...", "cao-mcp-server"]
---
# CODING SUPERVISOR AGENT
You are the Coding Supervisor Agent...
```

**載入順序** (`utils/agent_profiles.py`):
1. 本地 Agent Store: `~/.aws/cli-agent-orchestrator/agent-store/{name}.md`
2. 內建 Agent Store: `src/cli_agent_orchestrator/agent_store/{name}.md`

#### 3.3.2 Provider 解析

```python
def resolve_provider(agent_profile_name, fallback_provider) -> str:
    """從 agent profile 解析 provider，若未指定則使用 fallback"""
    profile = load_agent_profile(agent_profile_name)
    if profile.provider and profile.provider in PROVIDERS:
        return profile.provider
    return fallback_provider
```

### 3.4 狀態監控

#### 3.4.1 輪詢式狀態偵測

每個 Provider 透過分析 tmux `capture-pane` 輸出中的正規表達式模式來偵測狀態。這是一種**被動觀察**的方法，無需修改被管理的 CLI agent。

```python
# utils/terminal.py
def wait_until_status(provider_instance, target_status, timeout=30.0, polling_interval=1.0) -> bool:
    """輪詢 provider 狀態直到達到目標或逾時"""
    targets = target_status if isinstance(target_status, set) else {target_status}
    start_time = time.time()
    while time.time() - start_time < timeout:
        status = provider_instance.get_status()
        if status in targets:
            return True
        time.sleep(polling_interval)
    return False
```

#### 3.4.2 Inbox 監視（watchdog + pipe-pane）

**檔案**: `src/cli_agent_orchestrator/services/inbox_service.py`

```python
class LogFileHandler(FileSystemEventHandler):
    """監視終端 log 檔案變更，自動遞送佇列中的訊息"""

    def on_modified(self, event):
        if event.src_path.endswith(".log"):
            terminal_id = Path(event.src_path).stem
            self._handle_log_change(terminal_id)

    def _handle_log_change(self, terminal_id):
        messages = get_pending_messages(terminal_id, limit=1)
        if not messages:
            return
        # 快速預檢：log 尾端是否有 idle 模式
        if not _has_idle_pattern(terminal_id):
            return
        # 完整狀態檢查 + 訊息遞送
        check_and_send_pending_messages(terminal_id)
```

**兩階段偵測最佳化**:
1. **快速預檢**: 讀取 log 檔案尾端（`tail -n 100`），用 provider 的 `get_idle_pattern_for_log()` 做字串匹配
2. **完整檢查**: 只有預檢通過才呼叫 `provider.get_status()`（需要 tmux `capture-pane`，較昂貴）

---

## 4. 多 Agent 協調機制

### 4.1 MCP Server — 協調核心

**檔案**: `src/cli_agent_orchestrator/mcp_server/server.py`

CAO 的多 Agent 協調透過 MCP（Model Context Protocol）server 實現，提供三個 MCP 工具：

#### 4.1.1 `handoff` — 同步委派（阻塞式）

```python
async def _handoff_impl(agent_profile, message, timeout=600, working_directory=None) -> HandoffResult:
    """
    流程：
    1. 建立新 terminal（與呼叫者同一 tmux session）
    2. 等待 terminal 就緒（IDLE/COMPLETED）
    3. 傳送任務訊息
    4. 輪詢等待完成（COMPLETED 狀態）
    5. 提取回應輸出
    6. 發送退出命令清理 terminal
    """
    terminal_id, provider = _create_terminal(agent_profile, working_directory)

    # 等待就緒（最多 120 秒）
    wait_until_terminal_status(terminal_id, {TerminalStatus.IDLE, TerminalStatus.COMPLETED}, timeout=120.0)
    await asyncio.sleep(2)  # 額外等待

    # Codex 需要特殊的 handoff 上下文前綴
    if provider == "codex":
        handoff_message = f"[CAO Handoff] Supervisor terminal ID: {supervisor_id}. " \
                         f"This is a blocking handoff...\n\n{message}"
    else:
        handoff_message = message

    _send_direct_input(terminal_id, handoff_message)

    # 等待完成（最多 timeout 秒）
    wait_until_terminal_status(terminal_id, TerminalStatus.COMPLETED, timeout=timeout)

    # 提取最後回應
    response = requests.get(f"{API_BASE_URL}/terminals/{terminal_id}/output", params={"mode": "last"})
    output = response.json()["output"]

    # 清理
    requests.post(f"{API_BASE_URL}/terminals/{terminal_id}/exit")

    return HandoffResult(success=True, output=output, terminal_id=terminal_id)
```

#### 4.1.2 `assign` — 非同步委派（非阻塞式）

```python
def _assign_impl(agent_profile, message, working_directory=None) -> Dict:
    """
    與 handoff 不同：
    - 建立 terminal 後立即傳送訊息
    - 不等待完成
    - 回傳 terminal_id 讓呼叫者可以稍後查詢
    """
    terminal_id, _ = _create_terminal(agent_profile, working_directory)
    _send_direct_input(terminal_id, message)
    return {"success": True, "terminal_id": terminal_id}
```

#### 4.1.3 `send_message` — Agent 間訊息傳遞

```python
@mcp.tool()
async def send_message(receiver_id: str, message: str) -> Dict:
    """發送訊息至另一個 terminal 的收件匣（IDLE 時自動遞送）"""
    return _send_to_inbox(receiver_id, message)
```

### 4.2 Supervisor-Worker 模式

CAO 的多 Agent 協調基於 **Supervisor-Worker** 階層模式：

```
使用者 → cao launch --agents code_supervisor
         ↓
    [code_supervisor] tmux window
         ↓ (使用 handoff MCP 工具)
    ├── [developer] tmux window    ← handoff("developer", "寫一個 REST API")
    └── [reviewer] tmux window     ← handoff("reviewer", "審查以下程式碼...")
```

**Supervisor Agent Profile** 的關鍵規則：
```markdown
1. NEVER write code directly yourself
2. ALWAYS assign actual coding work to the Developer Agent
3. ALWAYS assign code reviews to the Code Reviewer Agent
```

### 4.3 CAO_TERMINAL_ID 環境變數機制

每個 terminal 建立時都會注入 `CAO_TERMINAL_ID` 環境變數：

```python
# tmux.py — session 建立時注入
environment = os.environ.copy()
environment["CAO_TERMINAL_ID"] = terminal_id

# MCP server 中用於識別呼叫者
current_terminal_id = os.environ.get("CAO_TERMINAL_ID")
```

這個機制讓每個 Agent 都能：
1. 知道自己的 terminal ID
2. 在 `assign` 模式中指示 worker 回傳結果至正確的 terminal
3. MCP server 繼承此 ID 以識別當前 session

### 4.4 Flow — 排程工作流

**檔案**: `src/cli_agent_orchestrator/services/flow_service.py`

Flow 是 CAO 的 cron-style 排程系統：

```markdown
---
name: morning-trivia
schedule: "0 9 * * *"
agent_profile: developer
provider: claude_code
script: "./check_something.sh"
---

Generate a morning trivia question about {{topic}} with difficulty {{difficulty}}.
```

**執行流程**:
1. Flow daemon 每 60 秒檢查 `next_run <= now` 的 flow
2. 執行可選的 script（回傳 JSON `{execute: bool, output: {}}`）
3. 渲染 prompt template（替換 `{{variable}}`）
4. 建立新 session + terminal
5. 傳送渲染後的 prompt

---

## 5. 終端管理系統

### 5.1 Terminal 生命週期

**檔案**: `src/cli_agent_orchestrator/services/terminal_service.py`

```
create_terminal()
  ├── generate_terminal_id()           # UUID hex[:8]
  ├── generate_window_name(profile)    # "{profile}-{uuid[:4]}"
  ├── tmux_client.create_session()     # 或 create_window()
  │   └── 注入 CAO_TERMINAL_ID 環境變數
  ├── db_create_terminal()             # SQLite 儲存元資料
  ├── provider_manager.create_provider()
  │   └── provider.initialize()        # 啟動 CLI 工具
  └── tmux_client.pipe_pane()          # 開始 log 串流

send_input(terminal_id, message)
  ├── get_terminal_metadata()          # 從 DB 查詢 session/window
  ├── tmux_client.send_keys()          # paste-buffer + Enter
  └── provider.mark_input_received()   # 通知 provider

get_output(terminal_id, mode)
  ├── tmux_client.get_history()        # capture-pane
  └── provider.extract_last_message()  # 僅 LAST 模式

delete_terminal(terminal_id)
  ├── tmux_client.stop_pipe_pane()     # 停止 log 串流
  ├── provider_manager.cleanup_provider()
  └── db_delete_terminal()
```

### 5.2 Session 階層

```
Session (tmux session, 前綴 "cao-")
├── Terminal 1 (tmux window "developer-a1b2")
│   └── Provider (ClaudeCodeProvider)
├── Terminal 2 (tmux window "reviewer-c3d4")
│   └── Provider (ClaudeCodeProvider)
└── Terminal 3 (tmux window "analyst-e5f6")
    └── Provider (GeminiCliProvider)  ← 可混合不同 provider
```

### 5.3 資料層

**檔案**: `src/cli_agent_orchestrator/clients/database.py`

SQLite 資料庫（`~/.aws/cli-agent-orchestrator/db/cli-agent-orchestrator.db`）包含三張表：

| 表名 | 用途 | 主鍵 |
|------|------|------|
| `terminals` | Terminal 元資料 | `id` (hex8) |
| `inbox` | Agent 間訊息 | `id` (自增) |
| `flows` | 排程工作流 | `name` |

### 5.4 Cleanup Service

**檔案**: `src/cli_agent_orchestrator/services/cleanup_service.py`

```python
def cleanup_old_data():
    """清理超過 RETENTION_DAYS (14天) 的資料"""
    # 刪除舊 terminal 記錄
    # 刪除舊 inbox 訊息
    # 刪除舊 terminal log 檔案
    # 刪除舊 server log 檔案
```

---

## 6. 錯誤處理與容錯機制

### 6.1 Provider 初始化容錯

每個 provider 的 `initialize()` 都有多層容錯：

```python
# 1. Shell 就緒等待（10 秒逾時）
if not wait_for_shell(tmux_client, session_name, window_name, timeout=10.0):
    raise TimeoutError("Shell initialization timed out")

# 2. 暖機命令（Codex、Gemini 需要）
tmux_client.send_keys(session_name, window_name, "echo ready")
time.sleep(2.0)

# 3. 工作區信任對話框處理（20 秒逾時）
self._handle_trust_prompt(timeout=20.0)

# 4. 等待 CLI 就緒（30-240 秒逾時，視 provider 而定）
if not wait_until_status(self, {TerminalStatus.IDLE, TerminalStatus.COMPLETED}, timeout=30.0):
    raise TimeoutError("CLI initialization timed out")
```

### 6.2 Handoff 逾時處理

```python
# MCP server — handoff 有兩層逾時
# 第一層：等待 terminal 就緒（120 秒）
if not wait_until_terminal_status(terminal_id, {IDLE, COMPLETED}, timeout=120.0):
    return HandoffResult(success=False, message="Terminal did not reach ready status")

# 第二層：等待任務完成（預設 600 秒，最多 3600 秒）
if not wait_until_terminal_status(terminal_id, COMPLETED, timeout=timeout):
    return HandoffResult(success=False, message=f"Handoff timed out after {timeout} seconds")
```

### 6.3 Terminal 建立失敗回復

```python
def create_terminal(...) -> Terminal:
    try:
        # ... 建立流程
    except Exception as e:
        # 清理已建立的資源
        try:
            provider_manager.cleanup_provider(terminal_id)
        except Exception:
            pass
        if new_session and session_name:
            try:
                tmux_client.kill_session(session_name)
            except:
                pass
        raise
```

### 6.4 Inbox 訊息遞送容錯

```python
# api/main.py — 訊息建立後嘗試立即遞送
try:
    inbox_service.check_and_send_pending_messages(receiver_id)
except Exception as e:
    logger.warning(f"Immediate delivery attempt failed: {e}")
    # 不影響 API 回應 — 訊息已持久化，watchdog 會稍後重試
```

### 6.5 Codex 特殊的 Handoff 訊息前綴

```python
if provider == "codex":
    handoff_message = (
        f"[CAO Handoff] Supervisor terminal ID: {supervisor_id}. "
        "This is a blocking handoff -- the orchestrator will automatically "
        "capture your response when you finish. Complete the task and output "
        "your results directly. Do NOT use send_message to notify the supervisor "
        "unless explicitly needed -- just do the work and present your deliverables.\n\n"
        f"{message}"
    )
```

這解決了 Codex agent 會嘗試透過 `send_message` 回傳結果（而非直接輸出）的問題。

---

## 7. 值得採用的設計模式

### 7.1 CLI Agent 抽象層（Provider Interface）

**核心概念**: 將每個外部 CLI agent 封裝為實作共同介面的 provider，透過終端輸出的正規表達式分析來偵測狀態。

**優點**:
- 無需修改被管理的 CLI 工具
- 新增 agent 只需實作 6 個方法
- 狀態偵測與業務邏輯完全分離

**適用場景 for clawtex-core**:
```rust
// 概念性 Rust trait
trait CliAgentProvider {
    fn initialize(&self, session: &TmuxSession) -> Result<()>;
    fn get_status(&self, output: &str) -> TerminalStatus;
    fn extract_response(&self, output: &str) -> Result<String>;
    fn exit_command(&self) -> &str;
    fn idle_pattern(&self) -> &str;
    fn cleanup(&self) -> Result<()>;
}
```

### 7.2 Tmux-Based Agent Management

**核心概念**: 每個 agent 獨佔一個 tmux window，透過 `paste-buffer -p`（bracketed paste）傳送輸入，透過 `capture-pane -e` 讀取輸出。

**關鍵技術細節**:
- `paste-buffer -p` 避免 TUI 熱鍵攔截
- Enter 按鍵間需要 0.3-0.5 秒延遲
- 不同 CLI 需要不同次數的 Enter（`paste_enter_count` 屬性）
- `pipe-pane` 串流至 log 檔案實現非同步事件偵測

### 7.3 Two-Stage Idle Detection

**核心概念**: 先用便宜的 log 檔案尾端檢查做預篩，通過後才用昂貴的 tmux capture-pane 做完整狀態偵測。

```
log file (pipe-pane) → fast regex check → if match → capture-pane → full status detection
```

### 7.4 Handoff vs Assign 雙模式協調

| 特性 | Handoff（同步） | Assign（非同步） |
|------|-----------------|-----------------|
| 阻塞 | 是 | 否 |
| 取得回應 | 自動 | 需要 send_message 回傳 |
| Terminal 清理 | 自動 | 手動 |
| 適用場景 | 確定性任務 | 長時間任務、平行處理 |

### 7.5 Agent Profile as Markdown

使用 Markdown + Frontmatter 格式定義 agent：
- 前置元資料包含名稱、描述、provider、MCP server 設定
- Markdown 正文作為系統提示
- 可本地覆寫內建設定
- 每個 provider 用不同方式注入系統提示（`--append-system-prompt`、`-c developer_instructions`、`-i`、`GEMINI.md`）

### 7.6 環境變數傳遞鏈

```
cao launch
  → tmux session (CAO_TERMINAL_ID=abc123)
    → CLI agent (claude/codex/gemini)
      → MCP server subprocess (CAO_TERMINAL_ID=abc123)
        → _create_terminal() 知道呼叫者是誰
```

---

## 8. 與 clawtex-core 的整合可能性

### 8.1 直接適用的模式

#### 8.1.1 外部 CLI Agent 協調器

clawtex-core 可以實作一個新的 tool 或 provider 層，用於協調外部 CLI agent：

```rust
// src/tools/cli_orchestrator.rs
pub struct CliOrchestratorTool {
    tmux_client: TmuxClient,
    providers: HashMap<String, Box<dyn CliAgentProvider>>,
}

impl Tool for CliOrchestratorTool {
    fn name(&self) -> &str { "cli_orchestrate" }

    async fn execute(&self, params: Value) -> Result<Value> {
        let agent_type = params["agent"].as_str().unwrap(); // "claude_code", "codex", "gemini"
        let task = params["task"].as_str().unwrap();

        // 1. 建立 tmux window
        let session = self.tmux_client.create_window(agent_type)?;

        // 2. 啟動 CLI agent
        let provider = self.providers.get(agent_type)?;
        provider.initialize(&session)?;

        // 3. 傳送任務
        session.send_keys(task)?;

        // 4. 等待完成
        loop {
            let output = session.capture_pane()?;
            match provider.get_status(&output) {
                TerminalStatus::Completed => break,
                TerminalStatus::Error => return Err(...),
                _ => tokio::time::sleep(Duration::from_secs(1)).await,
            }
        }

        // 5. 提取回應
        let response = provider.extract_response(&session.capture_pane()?)?;
        session.cleanup()?;

        Ok(json!({"result": response}))
    }
}
```

#### 8.1.2 Cluster Worker 的 CLI Agent 模式

clawtex-core 的 cluster worker 可以使用 CAO 的模式來管理本地 CLI agent：

```toml
# agents.toml
[cluster.worker]
type = "cli_agent"
provider = "claude_code"
tmux_session = "clawtex-workers"
max_concurrent = 3  # 同時最多 3 個 tmux window
```

### 8.2 架構差異與取捨

| 維度 | CAO | clawtex-core |
|------|-----|-------------|
| 語言 | Python | Rust |
| Agent 管理 | tmux session/window | 直接 API 呼叫 |
| 通訊 | MCP over stdio | Telegram Bot API / HTTP |
| 狀態偵測 | regex 解析終端輸出 | API 回應 |
| Provider 模型 | CLI 工具抽象 | LLM API 抽象 |
| 排程 | APScheduler (cron) | 自建 cron 系統 |
| 資料庫 | SQLite (SQLAlchemy) | SQLite (rusqlite) |

### 8.3 具體整合建議

#### 建議 1: 新增 `CliAgentProvider` trait

在 clawtex-core 的 `src/providers/` 中新增一個全新的 provider 類型，將外部 CLI agent 視為一種「LLM provider」：

```rust
// src/providers/cli_agent.rs
pub struct CliAgentProvider {
    agent_type: CliAgentType,  // ClaudeCode, Codex, GeminiCli
    tmux_session: String,
    window_pool: Vec<TmuxWindow>,
}

impl Provider for CliAgentProvider {
    async fn complete(&self, messages: &[Message]) -> Result<String> {
        let window = self.acquire_window().await?;
        let task = messages.last().unwrap().content.clone();
        window.send_input(&task).await?;
        window.wait_for_completion(Duration::from_secs(600)).await?;
        let response = window.extract_response().await?;
        self.release_window(window).await;
        Ok(response)
    }
}
```

#### 建議 2: delegate_to_cli tool

新增一個 tool 讓 clawtex agent 可以委派工作給外部 CLI agent：

```rust
// src/tools/delegate_cli.rs
// 使用者透過 Telegram: "用 Claude Code 幫我重構這個檔案"
// agent 呼叫: delegate_to_cli(agent="claude_code", task="重構 src/main.rs")
```

#### 建議 3: 借鑑 inbox 監視模式

CAO 的 `pipe-pane` + `watchdog` 模式可以用來監視任何長時間運行的 CLI 工具，不僅限於 AI agent。clawtex-core 的 `shell` tool 可以借鑑此模式來改善長命令的輸出捕獲。

### 8.4 不建議採用的部分

1. **純 Python 正規表達式狀態偵測**: 維護成本高，每次 CLI 更新都可能需要調整 regex。建議優先使用 `--json` 或 structured output 模式（如果 CLI 支援）。

2. **`time.sleep()` 輪詢**: CAO 大量使用 `time.sleep()` 輪詢狀態，在 Rust 中應改用 `tokio::select!` + channel 或 `inotify`/`kqueue` 事件驅動。

3. **Module-level Singleton**: CAO 使用 Python module-level singleton（`tmux_client = TmuxClient()`、`provider_manager = ProviderManager()`），在 Rust 中應使用 `Arc<Mutex<>>` 或 `OnceCell`。

4. **同一進程內的 HTTP API 自呼叫**: MCP server 透過 HTTP 呼叫同一個 `cao-server` 的 API 端點，這增加了不必要的網路開銷。在 Rust 中應直接呼叫函式。

---

## 附錄 A: 關鍵檔案索引

| 檔案路徑 | 行數 | 功能 |
|----------|------|------|
| `src/cli_agent_orchestrator/providers/base.py` | 147 | Provider 抽象基底類別 |
| `src/cli_agent_orchestrator/providers/claude_code.py` | 253 | Claude Code provider |
| `src/cli_agent_orchestrator/providers/codex.py` | 475 | Codex provider |
| `src/cli_agent_orchestrator/providers/kiro_cli.py` | 265 | Kiro CLI provider |
| `src/cli_agent_orchestrator/providers/q_cli.py` | 170 | Q CLI provider |
| `src/cli_agent_orchestrator/providers/gemini_cli.py` | 685 | Gemini CLI provider（最複雜） |
| `src/cli_agent_orchestrator/providers/manager.py` | 116 | Provider 工廠 + 生命週期 |
| `src/cli_agent_orchestrator/clients/tmux.py` | 468 | tmux 封裝（libtmux） |
| `src/cli_agent_orchestrator/clients/database.py` | 369 | SQLAlchemy 資料層 |
| `src/cli_agent_orchestrator/services/terminal_service.py` | 331 | Terminal CRUD + I/O |
| `src/cli_agent_orchestrator/services/session_service.py` | 90 | Session 管理 |
| `src/cli_agent_orchestrator/services/inbox_service.py` | 152 | Agent 間訊息 + watchdog |
| `src/cli_agent_orchestrator/services/flow_service.py` | 226 | 排程工作流 |
| `src/cli_agent_orchestrator/mcp_server/server.py` | 477 | MCP server（handoff, assign, send_message） |
| `src/cli_agent_orchestrator/api/main.py` | 440 | FastAPI HTTP 伺服器 |
| `src/cli_agent_orchestrator/cli/main.py` | 31 | CLI 入口 |
| `src/cli_agent_orchestrator/cli/commands/launch.py` | 78 | launch 命令 |
| `src/cli_agent_orchestrator/models/terminal.py` | 37 | Terminal + TerminalStatus 模型 |
| `src/cli_agent_orchestrator/models/agent_profile.py` | 37 | AgentProfile 模型 |
| `src/cli_agent_orchestrator/utils/terminal.py` | 118 | 狀態輪詢、ID 生成 |
| `src/cli_agent_orchestrator/utils/agent_profiles.py` | 82 | Agent 設定檔載入 |
| `src/cli_agent_orchestrator/constants.py` | 96 | 全域常數 |

## 附錄 B: 架構流程圖

```
                    使用者
                      │
                      ▼
              ┌──────────────┐
              │   cao CLI    │  cao launch --agents code_supervisor
              └──────┬───────┘
                     │ HTTP POST /sessions
                     ▼
              ┌──────────────┐
              │  cao-server  │  FastAPI (port 9889)
              │  (FastAPI)   │
              └──────┬───────┘
                     │
          ┌──────────┼──────────┐
          ▼          ▼          ▼
    ┌──────────┐ ┌────────┐ ┌──────────┐
    │ Terminal  │ │  DB    │ │ Inbox    │
    │ Service  │ │(SQLite)│ │ Service  │
    └────┬─────┘ └────────┘ └────┬─────┘
         │                       │
         ▼                       ▼
    ┌──────────┐           ┌──────────┐
    │ Provider │           │ Watchdog │ (PollingObserver)
    │ Manager  │           │ LogFile  │
    └────┬─────┘           │ Handler  │
         │                 └──────────┘
         ▼
    ┌──────────┐
    │  Tmux    │  libtmux singleton
    │  Client  │
    └────┬─────┘
         │
         ▼
    ┌─────────────────────────────────────────────┐
    │              tmux server                     │
    │  ┌─────────────────────────────────────┐    │
    │  │  cao-session-abc123                  │    │
    │  │  ┌───────────┐  ┌───────────┐       │    │
    │  │  │supervisor │  │developer  │       │    │
    │  │  │ (claude)  │  │ (claude)  │       │    │
    │  │  │           │  │           │       │    │
    │  │  │ MCP tools:│  │           │       │    │
    │  │  │ -handoff  │──→           │       │    │
    │  │  │ -assign   │  │           │       │    │
    │  │  │ -send_msg │  │           │       │    │
    │  │  └───────────┘  └───────────┘       │    │
    │  └─────────────────────────────────────┘    │
    └─────────────────────────────────────────────┘
```

## 附錄 C: 狀態偵測模式對照表

| Provider | IDLE Pattern | PROCESSING Pattern | COMPLETED Pattern | EXIT Command |
|----------|-------------|-------------------|-------------------|--------------|
| Claude Code | `[>❯]\s` | `[✶✢✽✻✳].*…` | `⏺` + idle prompt | `/exit` |
| Codex | `(?:❯\|›\|codex>)` | `thinking\|working\|...` 或 TUI spinner | assistant marker + idle | `/exit` |
| Kiro CLI | `[profile] >` | 無 idle prompt | green arrow `>` + idle | `/exit` |
| Q CLI | `[profile] >` | 無 idle prompt | green arrow `>` + idle | `/exit` |
| Gemini CLI | `* Type your message` | Braille spinner `⠋⠙⠹⠸...` | `✦` response + idle | `C-d` |
