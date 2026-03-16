# OpenHands (formerly OpenDevin) 深度技術分析

> 分析日期: 2026-03-12
> 版本: V0 Legacy + V1 Migration (V1 app_server 共存)
> 原始碼路徑: `LLM-Cluster-Project/references/openhands/`

---

## 1. 專案結構

OpenHands 是一個以 Python 撰寫的自主 AI 軟體工程師平台, 目前正從 V0 (單體架構) 遷移到 V1 (Software Agent SDK + app_server 分離架構)。所有 V0 檔案頂部都標記了 `Legacy-V0` 標籤。

### 頂層目錄樹

```
openhands/
  openhands/                  # 核心 Python 套件
    agenthub/                 # Agent 實作集合 (CodeAct, Browsing, LOC, ReadOnly, VisualBrowsing, Dummy)
    app_server/               # V1 新版應用伺服器
    controller/               # AgentController, Agent 基類, StuckDetector, Replay
    core/                     # 設定、啟動流程、主迴圈、訊息、例外
    critic/                   # 批評/評估模組
    events/                   # Action/Observation 事件系統 (核心架構)
    integrations/             # GitHub/GitLab/Bitbucket/Forgejo/Azure DevOps 整合
    io/                       # 輸入輸出工具
    linter/                   # 程式碼靜態分析
    llm/                      # LLM 抽象層 (litellm 包裝)
    mcp/                      # MCP (Model Context Protocol) 客戶端
    memory/                   # 記憶管理 + Condenser 壓縮器系統
    microagent/               # 微代理系統 (Knowledge, Repo, Task 三類)
    resolver/                 # GitHub Issue 自動解決器
    runtime/                  # 執行時環境 (Docker, Remote, Local, K8s, CLI, E2B)
    security/                 # 安全分析器 (Invariant, GraySwan, LLM-based)
    server/                   # V0 Web 伺服器
    storage/                  # 檔案儲存抽象
  enterprise/                 # 企業版功能 (Stripe, Jira, Slack, Linear, 資料庫遷移)
  containers/                 # Docker 建置腳本
  third_party/                # E2B sandbox 等第三方容器
  .openhands/                 # 專案級微代理 (documentation.md, glossary.md)
  .agents/                    # Agent 技能定義
```

### 關鍵模組依賴圖

```
core/main.py (CLI 進入點)
  --> core/setup.py (create_runtime, create_agent, create_controller)
      --> runtime/base.py (Runtime 抽象基類)
      --> controller/agent_controller.py (AgentController)
      --> controller/agent.py (Agent 抽象基類)
      --> agenthub/codeact_agent/ (CodeActAgent 實作)
      --> llm/llm.py (LLM wrapper over litellm)
      --> memory/condenser/ (歷史壓縮)
      --> events/stream.py (EventStream)
```

---

## 2. 進入點與啟動流程

### CLI 模式

檔案: `openhands/core/main.py`

```python
if __name__ == '__main__':
    args = parse_arguments()
    config: OpenHandsConfig = setup_config_from_args(args)
    task_str = read_task(args, config.cli_multiline_input)
    initial_user_action = MessageAction(content=task_str)
    sid = generate_sid(config, session_name)
    asyncio.run(
        run_controller(
            config=config,
            initial_user_action=initial_user_action,
            sid=sid,
            fake_user_response_fn=None if args.no_auto_continue else auto_continue_response,
        )
    )
```

### `run_controller` 啟動序列

1. **建立 LLM Registry** -- `create_registry_and_conversation_stats(config, sid, None)`
2. **建立 Agent** -- `create_agent(config, llm_registry)` 透過 `Agent.get_cls()` 註冊表查找
3. **建立 Runtime** -- `create_runtime(config, llm_registry, sid)` 根據 `config.runtime` 選擇實作
4. **連接 Runtime** -- `runtime.connect()` (Docker 會啟動容器、安裝插件)
5. **初始化 Repository** -- 如果設定了 `selected_repo`, clone 到 sandbox
6. **建立 Memory** -- `create_memory()` 載入微代理
7. **載入 MCP 工具** -- `add_mcp_tools_to_agent(agent, runtime, memory)`
8. **建立 Controller** -- `create_controller(agent, runtime, config, conversation_stats)`
9. **事件注入** -- 將初始 `MessageAction` 加入 EventStream
10. **主迴圈** -- `run_agent_until_done(controller, runtime, memory, end_states)` (每秒輪詢 state)

### Server 模式 (V1)

V1 使用 `openhands/app_server/` 中的 FastAPI 應用, 配合 `v1_router.py` 路由。V0 伺服器在 `openhands/server/` 中。

---

## 3. 核心架構

### 3.1 Agent 迴圈: AgentController

檔案: `openhands/controller/agent_controller.py`

AgentController 是整個系統的心臟, 負責:

- **狀態管理**: 透過 `State` + `StateTracker` 追蹤所有歷史事件
- **步驟執行**: `_step()` 方法驅動 Agent 的單步推理
- **事件監聽**: 訂閱 `EventStream`, 收到事件時觸發處理
- **卡住偵測**: 內建 `StuckDetector` 偵測迴圈
- **Agent 委託**: 支援 `AgentDelegateAction` 將任務委派給子 Agent
- **安全分析**: 可選的 `SecurityAnalyzer` 對每個 Action 評估風險
- **預算控制**: `max_iterations` 和 `max_budget_per_task` 雙重限制

```python
class AgentController:
    def __init__(self, agent, event_stream, conversation_stats, ...):
        self._stuck_detector = StuckDetector(self.state)
        self.event_stream.subscribe(EventStreamSubscriber.AGENT_CONTROLLER, self.on_event, self.id)
        self._add_system_message()  # 將系統提示加入事件流

    async def _step(self):
        # 1. 呼叫 agent.step(state) 取得 Action
        # 2. 安全分析 (如果啟用)
        # 3. 確認模式 (如果啟用)
        # 4. 將 Action 加入 EventStream
        # 5. Runtime 執行 Action 並產生 Observation
        # 6. StuckDetector 檢查是否卡住
```

### 3.2 Agent 抽象基類

檔案: `openhands/controller/agent.py`

```python
class Agent(ABC):
    _registry: dict[str, type['Agent']] = {}  # 全域註冊表

    def __init__(self, config: AgentConfig, llm_registry: LLMRegistry):
        self.llm = llm_registry.get_llm_from_agent_config('agent', config)
        self.mcp_tools: dict[str, ChatCompletionToolParam] = {}
        self.tools: list = []

    @abstractmethod
    def step(self, state: State) -> Action:
        """核心: 根據當前狀態決定下一步行動"""

    @classmethod
    def register(cls, name: str, agent_cls: type['Agent']) -> None:
        cls._registry[name] = agent_cls

    def set_mcp_tools(self, mcp_tools: list[dict]) -> None:
        """動態注入 MCP 工具到 Agent"""
```

**設計亮點**: Agent 使用靜態 `_registry` 字典實現插件式註冊, 透過 `import openhands.agenthub` 自動觸發所有 Agent 的註冊。

### 3.3 CodeActAgent (主要 Agent 實作)

檔案: `openhands/agenthub/codeact_agent/codeact_agent.py`

CodeActAgent 是 OpenHands 的核心 Agent, 實作了 CodeAct 論文的理念: 將所有行動統一為程式碼行動空間。

```python
class CodeActAgent(Agent):
    VERSION = '2.2'
    sandbox_plugins = [AgentSkillsRequirement(), JupyterRequirement()]

    def __init__(self, config, llm_registry):
        self.tools = self._get_tools()  # 動態組裝工具列表
        self.conversation_memory = ConversationMemory(self.config, self.prompt_manager)
        self.condenser = Condenser.from_config(self.config.condenser, llm_registry)

    def step(self, state: State) -> Action:
        # 1. 檢查 pending_actions 佇列
        if self.pending_actions:
            return self.pending_actions.popleft()

        # 2. Condenser 壓縮歷史
        match self.condenser.condensed_history(state):
            case View(events=events):
                condensed_history = events
            case Condensation(action=condensation_action):
                return condensation_action  # 直接回傳壓縮動作

        # 3. 組裝訊息
        messages = self._get_messages(condensed_history, initial_user_message, ...)

        # 4. LLM 呼叫
        response = self.llm.completion(messages=messages, tools=self.tools)

        # 5. 解析回應為 Actions
        actions = self.response_to_actions(response)
        for action in actions:
            self.pending_actions.append(action)
        return self.pending_actions.popleft()

    def _get_tools(self):
        """根據設定動態組裝工具列表"""
        tools = []
        if self.config.enable_cmd: tools.append(create_cmd_run_tool(...))
        if self.config.enable_think: tools.append(ThinkTool)
        if self.config.enable_finish: tools.append(FinishTool)
        if self.config.enable_browsing: tools.append(BrowserTool)
        if self.config.enable_jupyter: tools.append(IPythonTool)
        if self.config.enable_editor: tools.append(create_str_replace_editor_tool(...))
        # ...更多工具
        return tools
```

### 3.4 事件系統 (Action / Observation)

這是 OpenHands 最核心的架構設計: **一切皆事件**。

#### Event 基類

檔案: `openhands/events/event.py`

```python
@dataclass
class Event:
    INVALID_ID = -1
    # 屬性: id, timestamp, source (AGENT/USER/ENVIRONMENT), cause, timeout
    # 元資料: llm_metrics, tool_call_metadata, response_id
```

#### Actions (Agent 的行動)

檔案: `openhands/events/action/`

| Action 類型 | 檔案 | 說明 |
|---|---|---|
| `CmdRunAction` | `commands.py` | 執行 bash 指令 (支援 tmux 持久會話) |
| `IPythonRunCellAction` | `commands.py` | 執行 IPython/Jupyter 程式碼 |
| `FileReadAction` | `files.py` | 讀取檔案 (支援行範圍) |
| `FileWriteAction` | `files.py` | 寫入檔案 |
| `FileEditAction` | `files.py` | 編輯檔案 (OH_ACI str_replace 或 LLM-based) |
| `BrowseURLAction` | `browse.py` | 瀏覽網址 |
| `BrowseInteractiveAction` | `browse.py` | 互動式瀏覽器操作 |
| `AgentDelegateAction` | `agent.py` | 委派任務給子 Agent |
| `AgentFinishAction` | `agent.py` | Agent 完成任務 |
| `AgentThinkAction` | `agent.py` | Agent 內部思考 |
| `MessageAction` | `message.py` | 使用者/Agent 訊息 |
| `MCPAction` | `mcp.py` | MCP 工具呼叫 |
| `RecallAction` | `agent.py` | 記憶召回 |
| `TaskTrackingAction` | `agent.py` | 任務追蹤 |
| `LoopRecoveryAction` | `agent.py` | 迴圈恢復 |

每個 Action 都有:
- `runnable: ClassVar[bool]` -- 是否可被 Runtime 執行
- `confirmation_state` -- 確認狀態 (CONFIRMED/REJECTED/AWAITING)
- `security_risk` -- 安全風險等級 (LOW/MEDIUM/HIGH/UNKNOWN)
- `thought` -- Agent 的推理過程

#### Observations (環境的回饋)

檔案: `openhands/events/observation/`

| Observation 類型 | 說明 |
|---|---|
| `CmdOutputObservation` | bash 指令輸出 (含 exit_code) |
| `IPythonRunCellObservation` | IPython 執行結果 |
| `FileReadObservation` | 檔案讀取內容 |
| `FileWriteObservation` | 寫入確認 |
| `FileEditObservation` | 編輯結果 (含 diff) |
| `BrowserOutputObservation` | 瀏覽器輸出 (DOM + 截圖) |
| `ErrorObservation` | 錯誤回報 |
| `AgentDelegateObservation` | 子 Agent 回傳結果 |
| `AgentStateChangedObservation` | Agent 狀態變更通知 |
| `MCPObservation` | MCP 工具回傳 |
| `LoopDetectionObservation` | 迴圈偵測結果 |
| `AgentCondensationObservation` | 歷史壓縮事件 |

### 3.5 EventStream

檔案: `openhands/events/stream.py`

EventStream 是事件的中央匯流排, 結合了持久化儲存和發布/訂閱模式:

```python
class EventStream(EventStore):
    def __init__(self, sid, file_store, user_id=None):
        self._subscribers = {}       # 訂閱者回呼函式
        self._queue = queue.Queue()  # 事件佇列
        self._queue_thread = threading.Thread(target=self._run_queue_loop)
        self.secrets = {}            # 秘密遮蔽

    def add_event(self, event, source):
        # 1. 設定 timestamp, source, id
        # 2. 序列化並遮蔽秘密
        # 3. 寫入 file_store (持久化)
        # 4. 放入佇列通知訂閱者

    def subscribe(self, subscriber_id, callback, callback_id):
        # 每個訂閱者有獨立的 ThreadPoolExecutor

    def _replace_secrets(self, data):
        # 自動將 secrets 替換為 '<secret_hidden>'
```

**設計亮點**:
- 事件持久化到 FileStore (可以是本地檔案或雲端儲存)
- 秘密自動遮蔽 -- 防止 API key 等敏感資訊洩漏到事件流
- 頁面快取 -- 批量讀取事件以提升效能
- 每個訂閱者有獨立的執行緒池, 避免阻塞

---

## 4. Runtime: 沙盒隔離與程式碼執行

### Runtime 架構

檔案: `openhands/runtime/base.py`

```python
class Runtime(FileEditRuntimeMixin):
    """抽象基類, 提供:
    - Bash shell 存取
    - 瀏覽器互動
    - 檔案系統操作
    - Git 操作
    - 環境變數管理
    """

    def __init__(self, config, event_stream, llm_registry, sid, plugins, ...):
        self.git_handler = GitHandler(...)
        self.event_stream = event_stream
        event_stream.subscribe(EventStreamSubscriber.RUNTIME, self.on_event, self.sid)
        # 載入 plugins (AgentSkills, Jupyter, VSCode)
        self.security_analyzer = None  # 可選安全分析器
```

### Runtime 實作類別

| 實作 | 檔案路徑 | 說明 |
|---|---|---|
| DockerRuntime | `runtime/impl/docker/docker_runtime.py` | **主要實作**: Docker 容器隔離 |
| RemoteRuntime | `runtime/impl/remote/remote_runtime.py` | 遠端執行環境 |
| LocalRuntime | `runtime/impl/local/local_runtime.py` | 本地執行 (開發用) |
| KubernetesRuntime | `runtime/impl/kubernetes/kubernetes_runtime.py` | K8s 執行環境 |
| CLIRuntime | `runtime/impl/cli/cli_runtime.py` | CLI 執行時 |

### DockerRuntime 核心設計

```python
class DockerRuntime(ActionExecutionClient):
    """
    1. 基於使用者指定的 base image 建置 runtime image
    2. 在 Docker 容器內啟動 Action Execution Server (FastAPI)
    3. 透過 HTTP 將 Action 發送到容器內執行
    4. 容器內的 server 回傳 Observation
    """
    def __init__(self, config, event_stream, ...):
        self.docker_client = self._init_docker_client()
        self.runtime_builder = DockerRuntimeBuilder(self.docker_client)
        # 分配 port: 執行 server (30000-39999), VSCode (40000-49999), App (50000-59999)
```

### Action Execution Server (容器內)

檔案: `openhands/runtime/action_execution_server.py`

這是一個 **在 Docker 容器內部運行** 的 FastAPI 伺服器:

```python
# 容器內部執行的 server
from fastapi import FastAPI
from openhands_aci.editor.editor import OHEditor  # str_replace_editor

# 接收 Action, 執行, 回傳 Observation
# 管理: BashSession, BrowserEnv, JupyterPlugin, MCPProxyManager
# 安全: SESSION_API_KEY 驗證
```

**核心流程**:
```
Agent --> Action --> [HTTP] --> Action Execution Server (容器內)
                                 |
                                 +--> BashSession (tmux)
                                 +--> BrowserEnv (Playwright)
                                 +--> JupyterPlugin (IPython kernel)
                                 +--> OHEditor (str_replace)
                                 |
Action Execution Server --> [HTTP] --> Observation --> AgentController
```

---

## 5. LLM 整合

### LLM 類別

檔案: `openhands/llm/llm.py`

OpenHands 使用 **litellm** 作為統一的 LLM 提供者介面:

```python
class LLM(RetryMixin, DebugMixin):
    def __init__(self, config: LLMConfig, service_id, metrics=None, retry_listener=None):
        self.config = copy.deepcopy(config)
        self.metrics = Metrics(model_name=config.model)

        # 處理各種模型特性:
        # - openhands/ 前綴 → litellm_proxy
        # - Gemini 2.5 Pro: 特殊 thinking budget
        # - Claude Opus 4.1: 停用 extended thinking
        # - Claude Opus 4.6, Sonnet 4.x: 不能同時用 temperature + top_p
        # - Azure: 使用 max_tokens 而非 max_completion_tokens
        # - Bedrock: AWS 認證

        self._completion = partial(litellm_completion, model=config.model, ...)

        @self.retry_decorator(num_retries=config.num_retries, ...)
        def wrapper(*args, **kwargs):
            # Mock function calling (非原生 FC 的模型)
            # 記錄 latency, cost, token usage
            # 日誌記錄
            return resp

    def is_function_calling_active(self) -> bool:
        """檢查模型是否支援原生 function calling"""

    def get_token_count(self, messages) -> int:
        """使用 litellm.token_counter 計算 token 數"""

    def _completion_cost(self, response) -> float:
        """計算 completion 費用並累加到 metrics"""
```

### LLM Registry

檔案: `openhands/llm/llm_registry.py`

```python
class LLMRegistry:
    """LLM 實例的集中管理器, 支援:
    - 依 service_id 快取和復用 LLM 實例
    - 從 AgentConfig 取得對應的 LLM
    - Retry 監聽器
    """
    def __init__(self, config: OpenHandsConfig):
        self.service_to_llm: dict[str, LLM] = {}
        self.active_agent_llm = self.get_llm('agent', llm_config)
```

### 函式呼叫轉換

檔案: `openhands/llm/fn_call_converter.py`

對於不支援原生 function calling 的模型, OpenHands 提供了:
- `convert_fncall_messages_to_non_fncall_messages()` -- 將工具定義嵌入 prompt
- `convert_non_fncall_messages_to_fncall_messages()` -- 將文字回應解析為工具呼叫

### 支援的模型特性

檔案: `openhands/llm/model_features.py`

針對不同模型的特性偵測:
- `supports_function_calling`
- `supports_vision`
- `supports_prompt_cache`
- `supports_reasoning_effort`
- `supports_stop_words`

---

## 6. 瀏覽器自動化

### BrowserEnv

檔案: `openhands/runtime/browser/browser_env.py`

OpenHands 使用 **BrowserGym** (基於 Playwright) 進行瀏覽器自動化:

```python
class BrowserEnv:
    def __init__(self, browsergym_eval_env=None):
        # 使用 multiprocessing 隔離瀏覽器進程
        self.browser_side, self.agent_side = multiprocessing.Pipe()
        self.process = multiprocessing.Process(target=self.browser_process)

    def browser_process(self):
        # 在獨立進程中:
        env = gym.make('browsergym/openended', headless=True, ...)
        obs, info = env.reset()

        while should_continue():
            # 接收 action, 執行 env.step(action)
            # 回傳: text_content (HTML→markdown), screenshot (base64),
            #        set_of_marks (SoM overlay), AXTree
            obs['text_content'] = html_text_converter.handle(html_str)
            obs['set_of_marks'] = overlay_som(obs['screenshot'], ...)

    def step(self, action_str, timeout=120) -> dict:
        """透過 Pipe 發送 action 到瀏覽器進程"""

    def close(self):
        """安全關閉: SHUTDOWN 訊號 → join → terminate → kill"""
```

**設計亮點**:
- 瀏覽器在獨立進程中運行, 透過 `multiprocessing.Pipe` 通訊
- 自動將 DOM 轉為可讀文字 (html2text)
- Set-of-Marks (SoM) 技術: 在截圖上標記可互動元素
- 支援 WebArena, MiniWoB, VisualWebArena 等評估環境

---

## 7. Microagent 系統

### 類型

檔案: `openhands/microagent/microagent.py`

微代理 (Microagents) 是 OpenHands 的提示增強系統, 分為三類:

```python
class BaseMicroagent(BaseModel):
    name: str
    content: str          # Markdown 內容
    metadata: MicroagentMetadata
    source: str           # 檔案路徑
    type: MicroagentType  # KNOWLEDGE / REPO_KNOWLEDGE / TASK

class KnowledgeMicroagent(BaseMicroagent):
    """關鍵字觸發的知識增強"""
    def match_trigger(self, message: str) -> str | None:
        # 當使用者訊息包含 trigger 關鍵字時啟動
        # 例: triggers: ["documentation", "docs"]

class RepoMicroagent(BaseMicroagent):
    """倉庫特定指令, 永遠載入"""
    # 從 .openhands/microagents/ 目錄載入
    # 也支援 .cursorrules, AGENTS.md

class TaskMicroagent(KnowledgeMicroagent):
    """任務型微代理, 需要使用者輸入"""
    # 觸發格式: /{agent_name}
    # 支援 ${variable_name} 變數
```

### 載入機制

```python
def load_microagents_from_dir(microagent_dir):
    """
    1. 掃描 .openhands/microagents/ 下所有 .md 檔案
    2. 也檢查倉庫根目錄的 .cursorrules 和 AGENTS.md
    3. 解析 frontmatter 中的 metadata (name, triggers, type, inputs)
    4. 自動推斷類型: 有 inputs → TASK, 有 triggers → KNOWLEDGE, 否則 → REPO
    """
```

### 實際範例

```markdown
---
name: documentation
type: knowledge
version: 1.0.0
agent: CodeActAgent
triggers:
- documentation
- docs
---

# Documentation Guidelines
All documentation must be grounded in fact...
```

---

## 8. 安全 / 沙盒

### 多層安全設計

**第一層: Runtime 隔離**
- DockerRuntime: 程式碼在 Docker 容器內執行
- RemoteRuntime: 程式碼在遠端沙盒執行
- E2B Runtime: 使用 E2B 提供的安全沙盒

**第二層: SecurityAnalyzer**

檔案: `openhands/security/analyzer.py`

```python
class SecurityAnalyzer:
    async def security_risk(self, action: Action) -> ActionSecurityRisk:
        """評估 Action 的安全風險等級"""
        # 子類別實作: InvariantAnalyzer, GraySwanAnalyzer, LLMRiskAnalyzer

    def set_event_stream(self, event_stream):
        """存取對話歷史以做上下文分析"""
```

**第三層: Action 風險標記**

每個可執行的 Action 都帶有 `security_risk` 屬性:
```python
class ActionSecurityRisk(int, Enum):
    UNKNOWN = -1
    LOW = 0
    MEDIUM = 1
    HIGH = 2
```

**第四層: 確認模式**

AgentController 支援 `confirmation_mode`, 高風險 Action 需要人類確認:
```python
class ActionConfirmationStatus(str, Enum):
    CONFIRMED = 'confirmed'
    REJECTED = 'rejected'
    AWAITING_CONFIRMATION = 'awaiting_confirmation'
```

**第五層: 秘密遮蔽**

EventStream 自動將秘密替換為 `<secret_hidden>`:
```python
def _replace_secrets(self, data):
    for key in data:
        if isinstance(data[key], str):
            for secret in self.secrets.values():
                data[key] = data[key].replace(secret, '<secret_hidden>')
```

---

## 9. 上下文管理 (Condenser 系統)

### 架構

檔案: `openhands/memory/condenser/condenser.py`

Condenser 是 OpenHands 的記憶壓縮系統, 負責將冗長的事件歷史壓縮到 LLM 的上下文窗口內:

```python
class Condenser(ABC):
    """抽象壓縮器介面
    回傳 View (壓縮後的事件列表) 或 Condensation (壓縮動作)
    """
    @abstractmethod
    def condense(self, view: View) -> View | Condensation:
        pass

    def condensed_history(self, state: State) -> View | Condensation:
        with self.metadata_batch(state):
            return self.condense(state.view)

class RollingCondenser(Condenser, ABC):
    """滾動壓縮器: 當歷史超過閾值時自動壓縮"""
    @abstractmethod
    def should_condense(self, view: View) -> bool:
        pass
    @abstractmethod
    def get_condensation(self, view: View) -> Condensation:
        pass
```

### 壓縮器實作

| 壓縮器 | 說明 |
|---|---|
| `NoOpCondenser` | 不壓縮, 原樣回傳 |
| `RecentEventsCondenser` | 只保留最近 N 個事件 |
| `ConversationWindowCondenser` | 滑動窗口壓縮 |
| `ObservationMaskingCondenser` | 遮蔽舊的 Observation 內容 |
| `BrowserOutputCondenser` | 壓縮瀏覽器輸出 |
| `LLMSummarizingCondenser` | 使用 LLM 摘要歷史 |
| `LLMAttentionCondenser` | LLM 判斷哪些事件重要 |
| `AmortizedForgettingCondenser` | 漸進式遺忘 |
| `StructuredSummaryCondenser` | 結構化摘要 |
| `CondenserPipeline` | 多個壓縮器串聯 |

### 在 CodeActAgent 中的使用

```python
# CodeActAgent.step() 中:
match self.condenser.condensed_history(state):
    case View(events=events, forgotten_event_ids=forgotten_ids):
        condensed_history = events  # 使用壓縮後的歷史

    case Condensation(action=condensation_action):
        return condensation_action  # 回傳壓縮動作, Controller 會立即再呼叫 step()
```

---

## 10. StuckDetector (卡住偵測)

檔案: `openhands/controller/stuck.py`

OpenHands 有非常精細的卡住偵測機制, 偵測 5 種迴圈模式:

```python
class StuckDetector:
    def is_stuck(self, headless_mode=True) -> bool:
        # Scenario 1: 重複相同的 Action + Observation (4次)
        # Scenario 2: 重複相同 Action + Error (3次)
        # Scenario 3: Agent 自言自語 (3次相同 MessageAction)
        # Scenario 4: A-B-A-B 交替模式 (6步)
        # Scenario 5: Context window 錯誤迴圈 (10次 condensation events)

    def _eq_no_pid(self, obj1, obj2):
        """比較事件時忽略 PID (process ID) 差異"""
```

**設計啟發**:
- 多場景偵測比單一策略更可靠
- 交互模式下只檢查最後一次使用者訊息之後的歷史
- `_eq_no_pid` 的設計很巧妙: 比較 Action 內容而非物件相等

---

## 11. MCP (Model Context Protocol) 整合

檔案: `openhands/mcp/__init__.py`

```python
# MCP 工具鏈:
# 1. create_mcp_clients() -- 從設定建立 MCP 客戶端
# 2. fetch_mcp_tools_from_config() -- 取得可用工具列表
# 3. convert_mcp_clients_to_tools() -- 轉為 ChatCompletionToolParam 格式
# 4. add_mcp_tools_to_agent() -- 注入到 Agent
# 5. call_tool_mcp() -- 執行 MCP 工具呼叫
```

Runtime 內部使用 `MCPProxyManager` 在沙盒中代理 MCP 連線。

---

## 12. OpenHands Resolver (Issue 解決器)

檔案: `openhands/resolver/`

Resolver 是一個獨立的子系統, 能自動解決 GitHub Issue:

```
resolver/
  resolve_issue.py        -- 主入口
  issue_resolver.py       -- 解決邏輯
  issue_handler_factory.py -- Issue 處理器工廠
  send_pull_request.py    -- 自動建立 PR
  patching/               -- Patch 應用
```

工作流程:
1. 讀取 Issue 內容
2. 呼叫 Agent 分析和修復
3. 生成 patch
4. 自動提交 PR

---

## 13. 值得借鏡的關鍵設計模式

### 13.1 Action/Observation 事件模式

OpenHands 的核心設計: **Agent 產出 Action, 環境回傳 Observation**。這種對稱設計有幾個優點:

- **解耦**: Agent 不直接接觸執行環境
- **可序列化**: 所有事件都可持久化和重放
- **可追蹤**: 完整的行為軌跡 (trajectory)
- **可測試**: Mock Action/Observation 即可測試 Agent

**clawtex-core 對應**: 目前 clawtex 的 tool 系統是 `ToolInput -> ToolOutput` 模式, 可以考慮在 agent_runtime 層面加入類似的事件流。

### 13.2 Runtime 沙盒架構

```
Host Machine (Agent + Controller)
    |
    | HTTP API
    v
Docker Container (Action Execution Server)
    |
    +-- BashSession (tmux)
    +-- BrowserEnv (Playwright, 獨立進程)
    +-- JupyterPlugin (IPython kernel)
    +-- MCP Proxy
```

**關鍵優勢**:
- Agent 程式碼與執行環境完全隔離
- 使用 HTTP API 通訊而非直接函式呼叫
- 容器可以安全地被銷毀和重建
- 支援遠端執行 (RemoteRuntime)

**clawtex-core 對應**: 目前 clawtex 的 shell tool 直接在主進程中執行子程序。可以參考 OpenHands 的 Docker sandbox 設計, 將 tool 執行隔離到容器中。`src/sandbox/docker.rs` 已有相關基礎。

### 13.3 Condenser 記憶壓縮

OpenHands 的 Condenser 系統比簡單的 "砍掉舊訊息" 更精緻:

- **Pipeline**: 多個壓縮器串聯 (先遮蔽 Observation, 再 LLM 摘要)
- **Condensation 協議**: 壓縮器可以回傳一個 "壓縮動作" 讓 Controller 處理
- **元資料追蹤**: 每次壓縮都記錄 metadata 供診斷

**clawtex-core 對應**: `src/context_compactor.rs` 有 Light/Medium/Aggressive 三級壓縮, 但缺乏 pipeline 組合和 LLM-based 摘要。可以參考 OpenHands 的 `CondenserPipeline` 設計。

### 13.4 Microagent 知識注入

微代理系統讓 OpenHands 能依據上下文動態注入專業知識:

- **Knowledge**: 關鍵字觸發 (例如提到 "React" 時注入 React 最佳實踐)
- **Repo**: 倉庫特定指令 (永遠載入)
- **Task**: 互動式任務範本 (需要使用者輸入變數)

**clawtex-core 對應**: clawtex 的 `skills` 系統類似但更簡單。可以參考 OpenHands 的 trigger 機制, 在 agent_runtime 中根據使用者訊息內容動態注入相關知識。

### 13.5 StuckDetector 迴圈偵測

5 種場景的偵測比 clawtex 的 `loop_detection.rs` (GenericRepeat, PingPong, StaleResult) 更全面:

- **Context window 錯誤迴圈** -- clawtex 沒有這個場景
- **PID 無關比較** -- 忽略程序 ID 差異的精確比較
- **互動模式感知** -- 只檢查最後一次使用者訊息之後的歷史

---

## 14. 與 clawtex-core 的對照與移植建議

### 架構對比

| 維度 | OpenHands | clawtex-core |
|---|---|---|
| 語言 | Python | Rust |
| 核心迴圈 | AgentController + step() | agent_runtime.rs + run_streaming() |
| 事件系統 | EventStream (Action/Observation) | 無統一事件流, tool 直接回傳 |
| LLM 抽象 | litellm (統一介面) | Provider trait (手動實作) |
| 沙盒 | Docker/Remote/K8s/E2B | 基礎 Docker sandbox |
| 記憶管理 | Condenser pipeline (10種策略) | ContextCompactor (3級) |
| 瀏覽器 | BrowserGym + Playwright (進程隔離) | Playwright Python 子進程 |
| MCP | stdio + SSE 客戶端 | stdio 客戶端 |
| 安全 | SecurityAnalyzer + 確認模式 + 秘密遮蔽 | approval.rs + secrets.rs |
| 微代理 | Knowledge/Repo/Task (Markdown + frontmatter) | skills.rs (簡化版) |
| 卡住偵測 | 5 種場景 | 3 種場景 |

### 建議移植的功能

1. **EventStream 模式** -- 在 clawtex-core 中建立統一的事件流, 讓所有 Action 和 Result 都可追蹤、持久化、重放。這是 OpenHands 最有價值的架構設計。

2. **Condenser Pipeline** -- 將 `context_compactor.rs` 升級為可組合的壓縮器管線, 加入 LLM-based 摘要和 Observation 遮蔽。

3. **Runtime HTTP API 模式** -- 將 tool 執行隔離到 Docker 容器中, 透過 HTTP API 通訊。clawtex 的 `src/sandbox/docker.rs` 已有基礎, 可以參考 OpenHands 的 action_execution_server 設計。

4. **Microagent 關鍵字觸發** -- 在 skills 系統中加入 trigger 機制, 根據使用者訊息內容自動注入相關知識。

5. **Context Window 錯誤迴圈偵測** -- 在 `loop_detection.rs` 中新增第四種場景: 偵測 context compaction 的無限迴圈。

6. **函式呼叫模擬** -- 對於不支援 function calling 的本地模型 (例如 Ollama 上的某些模型), 參考 OpenHands 的 `fn_call_converter.py` 實作 prompt-based 的 tool calling。

---

## 附錄: 關鍵檔案路徑索引

| 功能 | 路徑 |
|---|---|
| CLI 進入點 | `openhands/core/main.py` |
| Agent 基類 | `openhands/controller/agent.py` |
| AgentController | `openhands/controller/agent_controller.py` |
| CodeActAgent | `openhands/agenthub/codeact_agent/codeact_agent.py` |
| 工具定義 | `openhands/agenthub/codeact_agent/tools/` |
| 函式呼叫解析 | `openhands/agenthub/codeact_agent/function_calling.py` |
| Event 基類 | `openhands/events/event.py` |
| Action 定義 | `openhands/events/action/` |
| Observation 定義 | `openhands/events/observation/` |
| EventStream | `openhands/events/stream.py` |
| Runtime 基類 | `openhands/runtime/base.py` |
| Docker Runtime | `openhands/runtime/impl/docker/docker_runtime.py` |
| Action Execution Server | `openhands/runtime/action_execution_server.py` |
| 瀏覽器環境 | `openhands/runtime/browser/browser_env.py` |
| LLM 包裝 | `openhands/llm/llm.py` |
| LLM Registry | `openhands/llm/llm_registry.py` |
| 模型特性 | `openhands/llm/model_features.py` |
| 函式呼叫轉換 | `openhands/llm/fn_call_converter.py` |
| Condenser 基類 | `openhands/memory/condenser/condenser.py` |
| Condenser 實作 | `openhands/memory/condenser/impl/` |
| 微代理系統 | `openhands/microagent/microagent.py` |
| 安全分析器 | `openhands/security/analyzer.py` |
| 卡住偵測 | `openhands/controller/stuck.py` |
| MCP 整合 | `openhands/mcp/` |
| Issue 解決器 | `openhands/resolver/` |
| 設定系統 | `openhands/core/config/` |
| V1 App Server | `openhands/app_server/` |
