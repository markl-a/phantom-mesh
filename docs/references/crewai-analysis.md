# CrewAI 深度技術分析

> **分析日期**: 2026-03-13
> **CrewAI 版本**: 1.10.2a1
> **原始碼路徑**: `references/crewai/lib/crewai/src/crewai/`
> **目的**: 從開發者角度深入分析 CrewAI 多代理框架的架構設計，供 clawtex-core 參考

---

## 目錄

1. [專案結構](#1-專案結構)
2. [進入點與啟動流程](#2-進入點與啟動流程)
3. [核心架構](#3-核心架構)
   - [Crew 團隊定義](#31-crew-團隊定義)
   - [Agent 代理配置](#32-agent-代理配置)
   - [Task 任務定義](#33-task-任務定義)
   - [Process 執行流程](#34-process-執行流程)
4. [工具系統](#4-工具系統)
5. [記憶系統](#5-記憶系統)
6. [委派機制](#6-委派機制)
7. [人類介入迴路](#7-人類介入迴路)
8. [輸出解析與結構化輸出](#8-輸出解析與結構化輸出)
9. [值得採納的設計模式](#9-值得採納的設計模式)
10. [clawtex Hands 可借鑑之處](#10-clawtex-hands-可借鑑之處)

---

## 1. 專案結構

### 1.1 目錄總覽

CrewAI 以 monorepo 結構管理，核心程式碼位於 `lib/crewai/src/crewai/`。

```
lib/crewai/src/crewai/
  __init__.py              # 頂層匯出: Agent, Crew, Task, Process, Flow, Memory
  crew.py                  # Crew 類別 — 團隊編排的核心 (~1600 行)
  task.py                  # Task 類別 — 任務定義與執行 (~800 行)
  process.py               # Process 列舉: sequential / hierarchical
  llm.py                   # LLM 封裝層
  agent/
    core.py                # Agent 主類別 (~700 行)
    utils.py               # 代理工具函式
  agents/
    agent_builder/
      base_agent.py        # BaseAgent 抽象基底 (~300 行)
      base_agent_executor_mixin.py
    crew_agent_executor.py # CrewAgentExecutor — 代理執行迴圈
    parser.py              # AgentAction / AgentFinish 解析
  crews/
    crew_output.py         # CrewOutput 結果封裝
    utils.py               # prepare_kickoff, prepare_task_execution
  tasks/
    conditional_task.py    # ConditionalTask — 條件式任務
    task_output.py         # TaskOutput 結果封裝
    output_format.py       # OutputFormat 列舉 (RAW/JSON/PYDANTIC)
    llm_guardrail.py       # LLM 防護欄
  tools/
    base_tool.py           # BaseTool 抽象基底 + @tool 裝飾器
    agent_tools/
      agent_tools.py       # AgentTools 管理器
      delegate_work_tool.py # DelegateWorkTool
      ask_question_tool.py  # AskQuestionTool
      base_agent_tools.py   # BaseAgentTool
    memory_tools.py        # RecallMemoryTool / RememberTool
  memory/
    unified_memory.py      # Memory 統一記憶類別 (~876 行)
    memory_scope.py        # MemoryScope / MemorySlice
    types.py               # MemoryRecord, MemoryMatch, MemoryConfig
    analyze.py             # LLM 記憶分析
    recall_flow.py         # RecallFlow — 深度回憶流
    encoding_flow.py       # EncodingFlow — 編碼存儲流
    storage/
      backend.py           # StorageBackend 抽象介面
      lancedb_storage.py   # LanceDB 向量存儲
  flow/
    flow.py                # Flow 類別 — 事件驅動工作流
    flow_wrappers.py       # @start, @listen, @router 裝飾器
    persistence/           # 流程持久化
  knowledge/
    knowledge.py           # Knowledge 知識庫
    source/                # 知識來源介面
  events/
    event_bus.py           # 全域事件匯流排
    event_listener.py      # EventListener
    types/                 # 各類事件定義
  core/
    providers/
      human_input.py       # HumanInputProvider Protocol
  utilities/
    converter.py           # Converter — 結構化輸出轉換
    prompts.py             # Prompts — 提示詞生成
    i18n.py                # 國際化
    planning_handler.py    # CrewPlanner — 預先規劃
  translations/
    en.json                # 英文提示詞模板
  security/                # Fingerprint, SecurityConfig
  mcp/                     # MCP 工具整合
  a2a/                     # Agent-to-Agent 跨代理通訊
```

**工具套件** (`lib/crewai-tools/src/crewai_tools/`): 獨立套件，包含 70+ 工具實作，包括 Brave Search、Firecrawl、Selenium、PDF Search、Code Interpreter、Vision 等。

### 1.2 關鍵程式碼統計

| 模組 | 約略行數 | 角色 |
|------|---------|------|
| `crew.py` | ~1600 | 團隊編排核心 |
| `agent/core.py` | ~700 | 代理配置與執行 |
| `task.py` | ~800 | 任務定義與生命週期 |
| `memory/unified_memory.py` | ~876 | 統一記憶系統 |
| `agents/crew_agent_executor.py` | ~500 | 代理執行迴圈 |
| `tools/base_tool.py` | ~588 | 工具抽象基底 |
| `translations/en.json` | ~82 | 提示詞模板 |

---

## 2. 進入點與啟動流程

### 2.1 初始化流程

CrewAI 的啟動始於使用者建構 `Crew` 物件：

```python
from crewai import Agent, Task, Crew, Process

# 1. 定義 Agent
researcher = Agent(
    role="Research Analyst",
    goal="Find and analyze market trends",
    backstory="You are an experienced market research analyst...",
    tools=[search_tool],
    allow_delegation=True,
    verbose=True,
)

# 2. 定義 Task
research_task = Task(
    description="Research the latest AI market trends",
    expected_output="A detailed report on current AI trends",
    agent=researcher,
    output_json=MarketReport,   # 可選: 結構化輸出
    human_input=True,           # 可選: 人類審核
)

# 3. 組建 Crew
crew = Crew(
    agents=[researcher, writer],
    tasks=[research_task, writing_task],
    process=Process.sequential,
    memory=True,                # 啟用統一記憶
    verbose=True,
)

# 4. 啟動
result = crew.kickoff(inputs={"topic": "AI trends 2026"})
```

**檔案**: `lib/crewai/src/crewai/crew.py` 第 129-305 行

### 2.2 Crew Model Validators 初始化鏈

`Crew` 繼承 `FlowTrackable` 和 Pydantic `BaseModel`。建構時觸發一連串 `@model_validator(mode="after")`:

```
Crew.__init__()
  -> set_private_attrs()      # 初始化 CacheHandler, Logger, RPMController
  -> create_crew_memory()     # 若 memory=True, 建立 Memory() 統一記憶
  -> create_crew_knowledge()  # 若有 knowledge_sources, 建立 Knowledge
  -> check_manager_llm()      # hierarchical 流程須有 manager_llm 或 manager_agent
  -> check_config()           # 從 config dict 設定 agents/tasks
  -> validate_tasks()         # sequential 流程中每個 task 必須有 agent
  -> validate_end_with_at_most_one_async_task()
  -> validate_must_have_non_conditional_task()
  -> validate_first_task()    # 第一個 task 不可為 ConditionalTask
```

**檔案**: `lib/crewai/src/crewai/crew.py` 第 331-499 行

### 2.3 kickoff() 主執行流程

```python
def kickoff(self, inputs=None, input_files=None):
    # 1. 若啟用串流, 包裝為 StreamingContext
    if self.stream:
        return CrewStreamingOutput(...)

    # 2. 設定 OpenTelemetry baggage
    baggage_ctx = baggage.set_baggage("crew_context", ...)

    # 3. 準備執行 (插值變數, 設定 agents, 規劃)
    inputs = prepare_kickoff(self, inputs, input_files)

    # 4. 依 process 類型分流
    if self.process == Process.sequential:
        result = self._run_sequential_process()
    elif self.process == Process.hierarchical:
        result = self._run_hierarchical_process()

    # 5. 執行 after_kickoff_callbacks
    for callback in self.after_kickoff_callbacks:
        result = callback(result)

    # 6. 計算使用量指標
    self.usage_metrics = self.calculate_usage_metrics()

    # 7. 清理: drain memory writes, clear files
    if self._memory is not None:
        self._memory.drain_writes()
    return result
```

**檔案**: `lib/crewai/src/crewai/crew.py` 第 668-747 行

---

## 3. 核心架構

### 3.1 Crew 團隊定義

`Crew` 是 CrewAI 的頂層編排單元，管理一組 Agent 和 Task 的協作。

**核心欄位**:

```python
class Crew(FlowTrackable, BaseModel):
    name: str | None = "crew"
    agents: list[BaseAgent] = []
    tasks: list[Task] = []
    process: Process = Process.sequential
    memory: bool | Any = False           # True=預設 Memory(), 或傳入自訂實例
    cache: bool = True
    verbose: bool = False
    manager_llm: str | BaseLLM | None    # hierarchical 模式的管理者 LLM
    manager_agent: BaseAgent | None       # 或自訂管理者 Agent
    planning: bool = False                # 啟用前置規劃
    planning_llm: str | BaseLLM | None
    stream: bool = False
    knowledge: Knowledge | None = None
    embedder: EmbedderConfig | None       # 記憶/知識的向量化配置
    # 回呼
    before_kickoff_callbacks: list[Callable]
    after_kickoff_callbacks: list[Callable]
    step_callback: Any | None
    task_callback: Any | None
```

**檔案**: `lib/crewai/src/crewai/crew.py` 第 129-305 行

**關鍵設計決策**:
- Crew 不直接執行任務，而是透過 `_execute_tasks()` 迴圈逐一驅動 Task 執行
- Memory 為全 Crew 共享，注入到每個 Agent 的執行環境中
- 支援 `before_kickoff_callbacks` / `after_kickoff_callbacks` 對輸入輸出做攔截處理

### 3.2 Agent 代理配置

`Agent` 繼承 `BaseAgent`，是 CrewAI 中角色扮演的核心單元。

**BaseAgent 欄位** (`agents/agent_builder/base_agent.py`):

```python
class BaseAgent(BaseModel, ABC):
    id: UUID4
    role: str               # 角色 (如 "Research Analyst")
    goal: str               # 目標 (如 "Find market trends")
    backstory: str          # 背景故事 (人格設定)
    config: dict | None
    cache: bool = True
    verbose: bool = False
    max_rpm: int | None
    allow_delegation: bool = False
    tools: list[BaseTool] | None = []
    max_iter: int = 25
    crew: Any | None        # 所屬 Crew 的反向引用
    i18n: I18N
    knowledge: Knowledge | None
    knowledge_sources: list[BaseKnowledgeSource] | None
    memory: Any | None      # Agent 級別的 Memory 實例
```

**Agent 擴展欄位** (`agent/core.py`):

```python
class Agent(BaseAgent):
    llm: str | BaseLLM | Any              # 語言模型
    function_calling_llm: str | BaseLLM   # 工具呼叫專用 LLM
    max_execution_time: int | None        # 最大執行時間 (秒)
    allow_code_execution: bool = False
    code_execution_mode: "safe" | "unsafe"  # Docker 或直接執行
    respect_context_window: bool = True   # 自動摘要保持在視窗內
    max_retry_limit: int = 2
    reasoning: bool = False               # 啟用思考/計劃
    max_reasoning_attempts: int | None
    use_system_prompt: bool = True
    inject_date: bool = False
    guardrail: GuardrailType | None       # Agent 級別防護欄
    a2a: A2AConfig | None                 # Agent-to-Agent 遠端委派
    executor_class: type = CrewAgentExecutor  # 可替換的執行器
```

**檔案**: `lib/crewai/src/crewai/agent/core.py` 第 107-245 行

**角色扮演的提示詞生成**:

Agent 的 `role`, `goal`, `backstory` 會被注入到系統提示詞中:

```json
"role_playing": "You are {role}. {backstory}\nYour personal goal is: {goal}"
```

這三個欄位構成了 CrewAI 角色扮演設計的基石。

**檔案**: `lib/crewai/src/crewai/translations/en.json` 第 11 行

### 3.3 Task 任務定義

`Task` 是 CrewAI 中工作的基本單位，定義了「做什麼」和「預期產出」。

**核心欄位**:

```python
class Task(BaseModel):
    description: str          # 任務描述
    expected_output: str      # 預期輸出格式
    agent: BaseAgent | None   # 負責的 Agent (sequential 必須)
    context: list[Task] | None  # 依賴的前置任務 (顯式 context)
    async_execution: bool = False
    tools: list[BaseTool] | None = []
    human_input: bool = False      # 是否需要人類審核
    # 結構化輸出
    output_json: type[BaseModel] | None
    output_pydantic: type[BaseModel] | None
    response_model: type[BaseModel] | None  # 原生 provider 結構化輸出
    output_file: str | None
    # 防護欄
    guardrail: GuardrailType | None
    guardrails: GuardrailsType | None
    guardrail_max_retries: int = 3
    # 中繼資料
    input_files: dict[str, FileInput] = {}
    callback: Any | None
    markdown: bool = False
```

**檔案**: `lib/crewai/src/crewai/task.py` 第 85-230 行

**Task 的 `context` 機制**:

每個 Task 可以透過 `context` 欄位顯式依賴前置 Task 的輸出。在執行時, Crew 會將所有 context Task 的 `TaskOutput.raw` 串接作為額外 context 傳入。若未設定 `context`，sequential 模式下會自動將前一個 task 的輸出作為 context。

**Task 執行核心** (`_execute_core`):

```python
def _execute_core(self, agent, context, tools):
    # 1. 設定 current_task_id (ContextVar)
    task_id_token = set_current_task_id(str(self.id))
    # 2. 存儲 input_files
    self._store_input_files()
    # 3. 委派給 Agent 執行
    result = agent.execute_task(task=self, context=context, tools=tools)
    # 4. 建構 TaskOutput
    task_output = TaskOutput(raw=raw, pydantic=..., json_dict=..., agent=agent.role)
    # 5. 執行 guardrail 驗證
    if self._guardrails:
        for guardrail in self._guardrails:
            task_output = self._invoke_guardrail_function(task_output, agent, tools, guardrail)
    # 6. 回呼
    if self.callback:
        self.callback(task_output)
    # 7. 輸出到檔案
    if self.output_file:
        self._save_file(content)
    return task_output
```

**檔案**: `lib/crewai/src/crewai/task.py` 第 673-770 行

### 3.4 Process 執行流程

#### 3.4.1 Sequential (循序執行)

```python
class Process(str, Enum):
    sequential = "sequential"
    hierarchical = "hierarchical"
```

**Sequential 流程**:

```python
def _run_sequential_process(self) -> CrewOutput:
    return self._execute_tasks(self.tasks)

def _execute_tasks(self, tasks, start_index=0, was_replayed=False):
    task_outputs = []
    futures = []         # 非同步任務的 Future 集合
    last_sync_output = None

    for task_index, task in enumerate(tasks):
        # 1. 準備執行環境 (agent, tools, 跳過已執行的)
        exec_data, task_outputs, last_sync_output = prepare_task_execution(...)
        if exec_data.should_skip:
            continue

        # 2. 處理 ConditionalTask
        if isinstance(task, ConditionalTask):
            skipped = self._handle_conditional_task(task, task_outputs, ...)
            if skipped:
                task_outputs.append(skipped)
                continue

        # 3. 非同步執行
        if task.async_execution:
            context = self._get_context(task, [last_sync_output] if last_sync_output else [])
            future = task.execute_async(agent=exec_data.agent, context=context, tools=exec_data.tools)
            futures.append((task, future, task_index))
        # 4. 同步執行
        else:
            # 先收集之前的非同步結果
            if futures:
                task_outputs = self._process_async_tasks(futures)
                futures.clear()
            context = self._get_context(task, task_outputs)
            task_output = task.execute_sync(agent=exec_data.agent, context=context, tools=exec_data.tools)
            task_outputs.append(task_output)

    # 5. 收尾: 收集殘餘的非同步結果
    if futures:
        task_outputs = self._process_async_tasks(futures)

    return self._create_crew_output(task_outputs)
```

**檔案**: `lib/crewai/src/crewai/crew.py` 第 1139-1245 行

**context 傳遞機制**: `_get_context()` 會收集任務 `task.context` 中指定的前置任務輸出，或在 sequential 模式下自動使用所有已完成任務的輸出作為 context。

#### 3.4.2 Hierarchical (層級式執行)

```python
def _run_hierarchical_process(self) -> CrewOutput:
    # 1. 建立管理者 Agent
    self._create_manager_agent()
    # 2. 用管理者驅動所有任務
    return self._execute_tasks(self.tasks)

def _create_manager_agent(self):
    if self.manager_agent is not None:
        # 使用自訂管理者
        self.manager_agent.allow_delegation = True
        manager = self.manager_agent
        manager.tools = []  # 管理者不應有工具
    else:
        # 自動建立管理者
        manager = Agent(
            role="Crew Manager",
            goal="Manage the team to complete the task in the best way possible.",
            backstory="You are a seasoned manager with a knack for getting the best...",
            tools=AgentTools(agents=self.agents).tools(),  # 注入委派工具
            allow_delegation=True,
            llm=self.manager_llm,
        )
    self.manager_agent = manager
```

**檔案**: `lib/crewai/src/crewai/crew.py` 第 1143-1173 行

**層級式與循序式的關鍵差異**:

| 面向 | Sequential | Hierarchical |
|------|-----------|-------------|
| 任務分配 | 每個 Task 預先綁定 Agent | 管理者決定由誰執行 |
| Agent 選取 | `task.agent` | `self.manager_agent` |
| 工具注入 | Agent 自身工具 + delegation tools | 管理者的 delegation tools |
| Task 必須有 agent | 是 | 否 (管理者代為分配) |

在 hierarchical 模式下, `_get_agent_to_use(task)` 永遠返回 `self.manager_agent`, 管理者透過 `DelegateWorkTool` 和 `AskQuestionTool` 來指揮其他 Agent。

```python
def _get_agent_to_use(self, task: Task) -> BaseAgent | None:
    if self.process == Process.hierarchical:
        return self.manager_agent
    return task.agent
```

**檔案**: `lib/crewai/src/crewai/crew.py` 第 1332-1335 行

---

## 4. 工具系統

### 4.1 BaseTool 基底類別

所有工具繼承自 `BaseTool`，使用 Pydantic BaseModel 定義。

```python
class BaseTool(BaseModel, ABC):
    name: str                        # 工具名稱
    description: str                 # 工具說明 (自動加入 JSON Schema)
    args_schema: type[BaseModel]     # 參數 Schema (自動從 _run 簽名推斷)
    cache_function: Callable         # 快取判斷函式
    result_as_answer: bool = False   # 工具結果是否直接作為最終答案
    max_usage_count: int | None      # 使用次數上限

    @abstractmethod
    def _run(self, *args, **kwargs) -> Any:
        """同步執行 — 子類必須實作"""

    async def _arun(self, *args, **kwargs) -> Any:
        """非同步執行 — 可選覆寫"""
        raise NotImplementedError

    def run(self, *args, **kwargs) -> Any:
        """公開介面: 驗證參數 -> 執行 -> 計數"""
        kwargs = self._validate_kwargs(kwargs)
        result = self._run(*args, **kwargs)
        self.current_usage_count += 1
        return result
```

**檔案**: `lib/crewai/src/crewai/tools/base_tool.py` 第 55-260 行

### 4.2 @tool 裝飾器

CrewAI 提供便利的 `@tool` 裝飾器從函式建立工具:

```python
from crewai.tools import tool

@tool
def search_web(query: str) -> str:
    """Search the web for information."""
    return perform_search(query)

# 自訂名稱
@tool("My Custom Tool")
def my_tool(input: str) -> str:
    """Custom tool description."""
    return process(input)

# 帶選項
@tool(result_as_answer=True, max_usage_count=3)
def final_answer_tool(data: str) -> str:
    """Generate the final answer."""
    return generate(data)
```

**檔案**: `lib/crewai/src/crewai/tools/base_tool.py` 第 486-587 行

### 4.3 工具自動描述生成

`BaseTool` 在初始化時自動生成包含 JSON Schema 的描述:

```python
def _generate_description(self) -> None:
    schema = generate_model_description(self.args_schema)
    args_json = json.dumps(schema["json_schema"]["schema"], indent=2)
    self.description = (
        f"Tool Name: {sanitize_tool_name(self.name)}\n"
        f"Tool Arguments: {args_json}\n"
        f"Tool Description: {self.description}"
    )
```

### 4.4 工具注入到 Agent

Crew 在準備每個任務時, 透過 `_prepare_tools()` 動態組裝工具集:

```python
def _prepare_tools(self, agent, task, tools):
    # 1. 委派工具 (allow_delegation=True)
    if agent.allow_delegation:
        if self.process == Process.hierarchical:
            tools = self._update_manager_tools(task, tools)
        else:
            tools = self._add_delegation_tools(task, tools)

    # 2. 程式碼執行工具 (allow_code_execution=True)
    if agent.allow_code_execution:
        tools = self._add_code_execution_tools(agent, tools)

    # 3. 多模態工具 (multimodal=True, 且 LLM 不原生支援)
    if agent.multimodal and not agent.llm.supports_multimodal():
        tools = self._add_multimodal_tools(agent, tools)

    # 4. Platform 工具 (CrewAI Enterprise apps)
    if agent.apps:
        tools = self._add_platform_tools(task, tools)

    # 5. MCP 工具
    if agent.mcps:
        tools = self._add_mcp_tools(task, tools)

    # 6. 記憶工具 (agent.memory 或 crew._memory)
    resolved_memory = getattr(agent, "memory", None) or self._memory
    if resolved_memory is not None:
        tools = self._add_memory_tools(tools, resolved_memory)

    # 7. 檔案工具 (input_files)
    files = get_all_files(self.id, task.id)
    if files:
        tools = self._add_file_tools(tools, files)

    return tools
```

**檔案**: `lib/crewai/src/crewai/crew.py` 第 1263-1330 行

**關鍵洞察**: 工具不是靜態綁定的，而是在每個任務執行前動態組裝。這允許同一個 Agent 在不同任務中使用不同的工具集。

---

## 5. 記憶系統

### 5.1 統一記憶架構 (Unified Memory)

CrewAI v1.10 採用統一記憶 (`Memory`)，取代了先前的 ShortTermMemory / LongTermMemory / EntityMemory 分離架構。

```python
class Memory(BaseModel):
    """統一記憶: 獨立運作, LLM 分析, 智慧回憶流"""

    llm: BaseLLM | str = "gpt-4o-mini"          # 分析用 LLM
    storage: StorageBackend | str = "lancedb"     # 向量存儲後端
    embedder: Any = None                          # 嵌入模型 (預設 OpenAI)
    # 評分權重
    recency_weight: float = 0.3
    semantic_weight: float = 0.5
    importance_weight: float = 0.2
    recency_half_life_days: int = 30
    # 整合 (consolidation)
    consolidation_threshold: float = 0.85
    consolidation_limit: int = 5
    # 回憶信心度
    confidence_threshold_high: float = 0.8
    confidence_threshold_low: float = 0.5
    exploration_budget: int = 1
    # 功能
    read_only: bool = False
```

**檔案**: `lib/crewai/src/crewai/memory/unified_memory.py` 第 55-128 行

### 5.2 記憶存儲 (remember)

```python
def remember(self, content, scope=None, categories=None, metadata=None,
             importance=None, source=None, private=False, agent_role=None):
    """存儲單一記憶 (同步)"""
    # 1. 發出 MemorySaveStartedEvent
    # 2. 透過 EncodingFlow 編碼
    #    - LLM 推斷 scope, categories, importance (若未提供)
    #    - 嵌入文本向量
    #    - 整合檢查: 若與現有記憶相似度 > 0.85, 合併而非重複存儲
    # 3. 存入 LanceDB
    # 4. 發出 MemorySaveCompletedEvent
    return record

def remember_many(self, contents, ...):
    """批次存儲 (非阻塞, 背景執行)"""
    self._submit_save(self._background_encode_batch, contents, ...)
    return []  # 不等待完成
```

### 5.3 記憶回憶 (recall)

```python
def recall(self, query, scope=None, categories=None, limit=10,
           depth="deep", source=None, include_private=False):
    """回憶相關記憶"""
    # 讀取屏障: 等待所有背景寫入完成
    self.drain_writes()

    if depth == "shallow":
        # 直接向量搜尋
        embedding = embed_text(self._embedder, query)
        raw = self._storage.search(embedding, scope_prefix=scope, ...)
        # 計算複合分數: recency * 0.3 + semantic * 0.5 + importance * 0.2
        results = [MemoryMatch(record=r, score=composite, match_reasons=reasons) ...]

    else:  # depth == "deep"
        # RecallFlow: LLM 驅動的智慧回憶
        # 1. LLM 分析查詢 -> 關鍵字, 範圍建議, 複雜度
        # 2. 生成 1-3 個子查詢
        # 3. 平行搜尋多個 scope
        # 4. 信心度路由: 高信心直接返回, 低信心深入探索
        flow = RecallFlow(storage=..., llm=..., embedder=..., config=...)
        flow.kickoff(inputs={"query": query, ...})
        results = flow.state.final_results

    return results
```

**檔案**: `lib/crewai/src/crewai/memory/unified_memory.py` 第 543-672 行

### 5.4 記憶在 Agent 執行中的使用

Agent 執行任務時, 記憶會在兩個地方介入:

**1. 執行前 — 自動回憶**:
```python
# agent/core.py execute_task()
if self._is_any_available_memory():
    unified_memory = getattr(self, "memory", None) or getattr(self.crew, "_memory", None)
    if unified_memory is not None:
        matches = unified_memory.recall(task.description, limit=5)
        if matches:
            memory = "Relevant memories:\n" + "\n".join(m.format() for m in matches)
        task_prompt += self.i18n.slice("memory").format(memory=memory)
```

**2. 執行中 — 主動工具**:

Agent 可以使用 `RecallMemoryTool` 和 `RememberTool` 主動搜尋和存儲記憶。

**記憶提示詞** (`translations/en.json`):
```json
"memory": "# Memories from past conversations:\n{memory}\n\n
IMPORTANT: The memories above are an automatic selection and may be INCOMPLETE.
If the task involves counting, listing, or summing items, you MUST use the
Search memory tool with several different queries..."
```

### 5.5 記憶範圍 (MemoryScope / MemorySlice)

```python
# 建立範圍視圖
agent_memory = memory.scope("/agents/researcher")
agent_memory.remember("Found 3 emerging trends in AI sector")

# 建立多範圍切片
project_view = memory.slice(
    scopes=["/project/alpha", "/project/beta"],
    categories=["findings", "decisions"],
    read_only=True,
)
results = project_view.recall("key decisions", limit=5)
```

**檔案**: `lib/crewai/src/crewai/memory/memory_scope.py`

---

## 6. 委派機制

### 6.1 委派工具

CrewAI 的委派是透過兩個特殊工具實現的:

**DelegateWorkTool**: 將完整任務委派給其他 Agent

```python
class DelegateWorkToolSchema(BaseModel):
    task: str       # 要委派的任務描述
    context: str    # 任務背景
    coworker: str   # 目標 Agent 的 role 名稱

class DelegateWorkTool(BaseAgentTool):
    name: str = "Delegate work to coworker"
    args_schema = DelegateWorkToolSchema

    def _run(self, task, context, coworker=None, **kwargs):
        coworker = self._get_coworker(coworker, **kwargs)
        return self._execute(coworker, task, context)
```

**AskQuestionTool**: 向其他 Agent 詢問問題

```python
class AskQuestionToolSchema(BaseModel):
    question: str   # 要問的問題
    context: str    # 問題背景
    coworker: str   # 目標 Agent 的 role 名稱

class AskQuestionTool(BaseAgentTool):
    name: str = "Ask question to coworker"
```

**檔案**: `lib/crewai/src/crewai/tools/agent_tools/delegate_work_tool.py`

### 6.2 委派執行流程

`BaseAgentTool._execute()` 方法是委派的核心:

```python
def _execute(self, agent_name, task, context=None):
    # 1. 清理/正規化 agent 名稱 (大小寫不敏感, 去除引號)
    sanitized_name = self.sanitize_agent_name(agent_name)

    # 2. 從 self.agents 列表中查找匹配的 Agent
    agent = [a for a in self.agents
             if self.sanitize_agent_name(a.role) == sanitized_name]

    if not agent:
        return "Error: coworker not found..."

    # 3. 建立臨時 Task 並委派執行
    selected_agent = agent[0]
    task_with_assigned_agent = Task(
        description=task,
        agent=selected_agent,
        expected_output=selected_agent.i18n.slice("manager_request"),
        i18n=selected_agent.i18n,
    )
    return selected_agent.execute_task(task_with_assigned_agent, context)
```

**檔案**: `lib/crewai/src/crewai/tools/agent_tools/base_agent_tools.py` 第 51-134 行

**關鍵洞察**:
- 委派是透過建立臨時 Task 物件來實現的
- 被委派的 Agent 會完整執行其 ReAct 迴圈
- 委派結果以字串返回給委派者
- 支援多層委派 (Agent A -> Agent B -> Agent C)
- `allow_delegation` 旗標控制哪些 Agent 可以委派

### 6.3 委派在不同 Process 中的行為

| Process | 委派行為 |
|---------|---------|
| Sequential | 每個 Agent 可委派給 Crew 中其他 Agent |
| Hierarchical | 管理者透過 DelegateWorkTool 指揮所有 Agent |

在 Hierarchical 模式下, 管理者 Agent 的工具集透過 `AgentTools(agents=self.agents).tools()` 自動建立, 包含指向所有工作 Agent 的委派/詢問工具。

```python
class AgentTools:
    def __init__(self, agents, i18n=None):
        self.agents = agents

    def tools(self):
        coworkers = ", ".join([f"{agent.role}" for agent in self.agents])
        return [
            DelegateWorkTool(agents=self.agents, description=..., coworkers=coworkers),
            AskQuestionTool(agents=self.agents, description=..., coworkers=coworkers),
        ]
```

**委派工具的描述模板**:
```json
"delegate_work": "Delegate a specific task to one of the following coworkers: {coworkers}
The input should be the coworker, the task, and ALL necessary context...
they know nothing about the task, so share absolutely everything you know"
```

---

## 7. 人類介入迴路

### 7.1 Task 級別的 Human Input

在 Task 上設定 `human_input=True`, 執行完成後會暫停等待人類回饋:

```python
task = Task(
    description="Write a marketing report",
    expected_output="A comprehensive report",
    agent=writer,
    human_input=True,  # 啟用人類審核
)
```

### 7.2 HumanInputProvider Protocol

CrewAI 使用 Provider 模式處理人類輸入, 允許替換不同的輸入來源:

```python
@runtime_checkable
class HumanInputProvider(Protocol):
    def setup_messages(self, context: ExecutorContext) -> bool: ...
    def post_setup_messages(self, context: ExecutorContext) -> None: ...
    def handle_feedback(self, formatted_answer, context) -> AgentFinish: ...
    async def handle_feedback_async(self, formatted_answer, context) -> AgentFinish: ...
```

**檔案**: `lib/crewai/src/crewai/core/providers/human_input.py` 第 59-131 行

### 7.3 預設同步回饋流程

```python
class SyncHumanInputProvider(HumanInputProvider):
    def handle_feedback(self, formatted_answer, context):
        # 1. 顯示 Rich Panel 提示使用者
        feedback = self._prompt_input(context.crew)

        # 2. 訓練模式: 單次回饋
        if context._is_training_mode():
            return self._handle_training_feedback(formatted_answer, feedback, context)

        # 3. 一般模式: 迴圈直到滿意
        return self._handle_regular_feedback(formatted_answer, feedback, context)

    def _handle_regular_feedback(self, current_answer, initial_feedback, context):
        feedback = initial_feedback
        answer = current_answer
        while context.ask_for_human_input:
            if feedback.strip() == "":
                # 空輸入 = 接受結果
                context.ask_for_human_input = False
            else:
                # 將回饋加入對話, 重新執行 Agent 迴圈
                context.messages.append(context._format_feedback_message(feedback))
                answer = context._invoke_loop()
                feedback = self._prompt_input(context.crew)
        return answer
```

**回饋提示詞**:
```json
"human_feedback": "You got human feedback on your work, re-evaluate it and
give a new Final Answer when ready.\n {human_feedback}"
```

### 7.4 ContextVar 注入

Provider 透過 `ContextVar` 管理, 允許在不同上下文中使用不同的 Provider:

```python
_provider: ContextVar[HumanInputProvider | None] = ContextVar("human_input_provider", default=None)

def get_provider() -> HumanInputProvider:
    provider = _provider.get()
    if provider is None:
        return SyncHumanInputProvider()
    return provider

def set_provider(provider: HumanInputProvider) -> Token:
    return _provider.set(provider)
```

**檔案**: `lib/crewai/src/crewai/core/providers/human_input.py` 第 451-490 行

---

## 8. 輸出解析與結構化輸出

### 8.1 三種輸出格式

```python
class OutputFormat(str, Enum):
    RAW = "raw"
    JSON = "json"
    PYDANTIC = "pydantic"
```

Task 定義時選擇輸出格式:
```python
# JSON 輸出
task = Task(..., output_json=MarketReport)
# Pydantic 輸出
task = Task(..., output_pydantic=MarketReport)
# 原生 response_model (使用 provider 的結構化輸出功能)
task = Task(..., response_model=MarketReport)
```

### 8.2 Converter 轉換器

`Converter` 類別負責將 Agent 的原始文字輸出轉換為結構化格式:

```python
class Converter(OutputConverter):
    def to_pydantic(self, current_attempt=1):
        if self.llm.supports_function_calling():
            # 使用 LLM 的原生 function calling
            response = self.llm.call(
                messages=[
                    {"role": "system", "content": self.instructions},
                    {"role": "user", "content": self.text},
                ],
                response_model=self.model,
            )
            result = self.model.model_validate_json(response)
        else:
            # 降級: 要求 LLM 輸出 JSON, 再解析
            response = self.llm.call([...])
            try:
                result = self.model.model_validate_json(response)
            except ValidationError:
                # 嘗試從部分 JSON 中提取
                result = handle_partial_json(result=response, model=self.model)
        return result

    def to_json(self, current_attempt=1):
        # 類似邏輯, 但返回 dict
```

**檔案**: `lib/crewai/src/crewai/utilities/converter.py` 第 41-180 行

### 8.3 TaskOutput 與 CrewOutput

**TaskOutput** — 單一任務的結果:
```python
class TaskOutput(BaseModel):
    description: str
    name: str | None
    expected_output: str | None
    raw: str                              # 原始文字輸出
    pydantic: BaseModel | None            # Pydantic 模型實例
    json_dict: dict[str, Any] | None      # JSON 字典
    agent: str                            # 執行者角色
    output_format: OutputFormat
    messages: list[LLMMessage]            # 對話歷史
```

**CrewOutput** — 整個 Crew 的結果:
```python
class CrewOutput(BaseModel):
    raw: str                              # 最後一個任務的原始輸出
    pydantic: BaseModel | None
    json_dict: dict[str, Any] | None
    tasks_output: list[TaskOutput]        # 所有任務的輸出
    token_usage: UsageMetrics             # Token 使用統計
```

### 8.4 Guardrail 防護欄

Task 支援 Guardrail 驗證, 確保輸出符合要求:

```python
# 函式型 Guardrail
def validate_report(output: TaskOutput) -> tuple[bool, str]:
    if len(output.raw) < 100:
        return (False, "Report too short, must be at least 100 words")
    return (True, output.raw)

task = Task(..., guardrail=validate_report, guardrail_max_retries=3)

# 字串型 Guardrail (LLM 判斷)
task = Task(..., guardrail="Ensure the output contains at least 3 market trends")

# 多重 Guardrail
task = Task(..., guardrails=[validate_format, validate_content, "Must be professional tone"])
```

驗證失敗時, Agent 會收到錯誤訊息重新執行:
```json
"validation_error": "Previous attempt failed validation: {guardrail_result_error}\n
Previous result: {task_output}\n
Try again, making sure to address the validation error."
```

---

## 9. 值得採納的設計模式

### 9.1 角色扮演代理設計 (Role-Based Agent Design)

CrewAI 的 `role / goal / backstory` 三元組是其最核心的設計模式:

```python
Agent(
    role="Senior Data Analyst",           # 角色 — 決定能力和觀點
    goal="Analyze data to find insights", # 目標 — 驅動行為方向
    backstory="10 years experience...",   # 背景 — 提供人格深度
)
```

**為何有效**: LLM 在角色扮演時表現更佳。`backstory` 不只是裝飾, 它為 LLM 提供了解決問題的思維框架。

**clawtex 可借鑑**:
- 在 `agents.toml` 中為每個 agent 增加 `role`, `goal`, `backstory` 欄位
- 在系統提示詞中注入: `"You are {role}. {backstory}\nYour personal goal is: {goal}"`
- 這比目前簡單的 `instructions` 字串更結構化

### 9.2 流程編排模式 (Process Orchestration)

**Sequential**: 適合線性流程, 每個步驟明確
- 前一步的輸出自動成為下一步的 context
- 可插入 `ConditionalTask` 實現分支邏輯
- 非同步任務允許並行處理

**Hierarchical**: 適合複雜問題, 需要動態調度
- 管理者 Agent 根據任務內容決定委派給誰
- 管理者透過工具 (而非硬編碼邏輯) 進行委派
- 這使得調度邏輯由 LLM 驅動, 更靈活

**clawtex 可借鑑**:
- Hands 目前的 phases 是純 sequential, 可增加 `conditional` phase 類型
- 考慮新增 `hierarchical` Hands 模式, 用 master agent 動態分配 phases

### 9.3 工具即委派 (Tools as Delegation)

CrewAI 將 inter-agent 通訊實作為工具, 而非特殊的訊息傳遞機制:

```
Agent A (執行中)
  -> 使用 DelegateWorkTool(coworker="Agent B", task="...", context="...")
    -> 建立臨時 Task
    -> Agent B 完整執行 ReAct 迴圈
    -> 返回字串結果
  -> Agent A 繼續執行
```

**為何有效**: 委派和其他工具使用相同的機制, LLM 不需要學習特殊的委派語法, 只需要選擇一個工具即可。

**clawtex 可借鑑**:
- 目前的 `delegate` 工具可以增強, 仿照 CrewAI 的 `DelegateWorkTool` 設計
- 增加 `AskQuestion` 變體, 專門用於 Agent 間的資訊查詢

### 9.4 統一記憶與背景寫入

CrewAI 的記憶系統有幾個精妙設計:

1. **背景寫入**: `remember_many()` 非阻塞, 使用 `ThreadPoolExecutor` 在背景執行
2. **讀取屏障**: `recall()` 前先 `drain_writes()`, 確保一致性
3. **LLM 分析**: 存儲時 LLM 自動推斷 scope, categories, importance
4. **深度回憶**: `RecallFlow` 使用 LLM 將查詢分解為子查詢, 多角度搜尋
5. **整合去重**: 新記憶與現有記憶相似度超過 0.85 時自動合併

**clawtex 可借鑑**:
- 目前的 `memory_store` / `memory_recall` 工具較為簡單
- 可增加: 自動重要性評分、scope 階層、背景寫入、consolidation

### 9.5 Guardrail 驗證閘門

CrewAI 的 Guardrail 機制允許在任務輸出和代理輸出上設置驗證:

```python
# 三種 Guardrail 類型:
# 1. 函式型: 精確的程式驗證
# 2. 字串型: LLM 驅動的語義驗證
# 3. 多重: 組合多個驗證器

task = Task(
    guardrails=[
        validate_json_format,           # 函式型
        "Must contain 3+ data points",  # 字串型
    ],
    guardrail_max_retries=3,
)
```

**clawtex 可借鑑**:
- Hands 的 phases 可增加 `guardrail` 欄位
- 支援 Lua/Python 函式驗證 + LLM 語義驗證

### 9.6 ConditionalTask 分支

```python
from crewai.tasks import ConditionalTask

conditional = ConditionalTask(
    description="Write detailed analysis",
    expected_output="Detailed report",
    agent=analyst,
    condition=lambda output: output.raw.count("critical") >= 2,  # 前一步包含 2+ 個 "critical"
)
```

**clawtex 可借鑑**: Hands 的 phases 可增加 `condition` 欄位, 根據前一 phase 的輸出決定是否跳過。

---

## 10. clawtex Hands 可借鑑之處

### 10.1 Phase-to-Task 對映

| CrewAI 概念 | clawtex Hands 概念 | 差異與改進機會 |
|------------|-------------------|-------------|
| `Crew` | `Hand` | Hand 包含 settings/phases, Crew 包含 agents/tasks |
| `Task` | `Phase` | Phase 缺乏顯式 `expected_output` |
| `Agent` | agent (全域) | CrewAI Agent 有 role/goal/backstory, clawtex agent 較扁平 |
| `Process.sequential` | phases 順序執行 | 相同概念 |
| `Process.hierarchical` | 無對應 | **機會: 新增 master-agent 驅動的動態 phase 分配** |
| `ConditionalTask` | 無對應 | **機會: phase 條件跳過** |
| `task.context` | phase 間自動傳遞 | CrewAI 允許顯式跨 task 依賴 |
| `task.guardrail` | 無對應 | **機會: phase 輸出驗證** |
| `task.human_input` | approval gate (全域) | CrewAI 更細粒度, 可 per-task 設定 |

### 10.2 建議改進 Hands 的具體方案

#### 方案一: 增加 Phase 的 expected_output

目前 Hand TOML:
```toml
[[phases]]
name = "research"
prompt = "Research the market..."
tools = ["web_search", "http_request"]
```

建議增加:
```toml
[[phases]]
name = "research"
prompt = "Research the market for {topic}"
expected_output = "A structured report with 3+ data points"
condition = "previous.contains('critical')"  # 可選: 條件執行
guardrail = "Must include sources"           # 可選: 驗證
human_review = true                          # 可選: 人類審核此 phase
```

#### 方案二: 角色設定注入 Agent

在 `agents.toml` 中:
```toml
[agents.researcher]
role = "Market Research Analyst"
goal = "Find actionable market intelligence"
backstory = "10 years in tech market analysis, specializing in emerging trends"
instructions = "..."  # 保留原有指令
model = "..."
```

在系統提示詞中自動合成:
```
You are Market Research Analyst. 10 years in tech market analysis...
Your personal goal is: Find actionable market intelligence

{original instructions}
```

#### 方案三: Hierarchical Hand 模式

新增 `mode = "hierarchical"` 到 hand.toml:
```toml
[settings]
mode = "hierarchical"     # 預設 "sequential"
manager_agent = "master"  # 管理者 agent

[[phases]]
name = "analyze_market"
# 無需指定 agent — 管理者動態分配
prompt = "Comprehensive market analysis"
```

在此模式下, master agent 會收到所有 phases 的描述, 並決定用哪個 agent 執行每個 phase。

#### 方案四: 統一記憶增強

將 CrewAI 的 Unified Memory 概念引入 clawtex:

```rust
// src/memory_unified.rs
pub struct UnifiedMemory {
    storage: Box<dyn VectorStorage>,  // LanceDB or SQLite + pgvector
    embedder: Box<dyn Embedder>,
    llm: Arc<dyn Provider>,           // 用於分析
    config: MemoryConfig,
    write_pool: tokio::sync::Semaphore,
}

impl UnifiedMemory {
    pub async fn remember(&self, content: &str, scope: Option<&str>) -> MemoryRecord {
        // LLM 自動推斷 scope, categories, importance
        // 整合去重
        // 非同步存儲
    }

    pub async fn recall(&self, query: &str, depth: RecallDepth) -> Vec<MemoryMatch> {
        // drain pending writes
        // shallow: 直接向量搜尋
        // deep: LLM 驅動的多子查詢搜尋
    }
}
```

#### 方案五: 委派工具增強

將 `delegate` 工具拆分為兩個:

```rust
// delegate_work — 完整任務委派
// args: { agent: "researcher", task: "Find 3 trends", context: "..." }
// -> 建立臨時 task, 被委派 agent 完整執行

// ask_question — 輕量問答
// args: { agent: "analyst", question: "What's the GDP?", context: "..." }
// -> 被問 agent 只回答問題, 不執行完整任務
```

---

## 附錄 A: CrewAI 事件系統

CrewAI 使用全域事件匯流排 (`crewai_event_bus`) 進行跨模組通訊:

```python
# 事件發出
crewai_event_bus.emit(self, TaskStartedEvent(context=context, task=task))

# 事件類型
CrewKickoffStartedEvent / CompletedEvent / FailedEvent
TaskStartedEvent / CompletedEvent / FailedEvent
MemoryQueryStartedEvent / CompletedEvent / FailedEvent
MemorySaveStartedEvent / CompletedEvent / FailedEvent
LiteAgentExecutionStartedEvent / CompletedEvent / ErrorEvent
KnowledgeQueryStartedEvent / CompletedEvent / FailedEvent
FlowCreatedEvent / StartedEvent / FinishedEvent / PausedEvent
```

clawtex 已有 `agent_events.rs` 事件系統, 可參考 CrewAI 的粒度來擴展事件類型。

## 附錄 B: 提示詞工程要點

CrewAI 的 ReAct 格式提示詞 (`translations/en.json`) 有幾個值得注意的設計:

1. **嚴格格式要求**: `Thought: -> Action: -> Action Input: -> Observation:` 循環
2. **最終答案格式**: `Thought: I now know the final answer\nFinal Answer: ...`
3. **強制完成機制**: 超過 max_iter 後注入 `"Now it's time you MUST give your absolute best final answer"`
4. **錯誤恢復**: 工具呼叫失敗時注入具體錯誤訊息, Agent 可自行調整
5. **記憶提示**: 提醒 Agent 自動回憶可能不完整, 需主動搜尋

```json
"force_final_answer": "Now it's time you MUST give your absolute best final answer.
You'll ignore all previous instructions, stop using any tools,
and just return your absolute BEST Final answer."
```

---

> **結論**: CrewAI 的核心價值在於將多代理協作抽象為 Crew/Agent/Task 三層結構, 以 role-playing 驅動 Agent 行為, 以 Process 定義編排策略, 以工具化的委派實現 Agent 間通訊。clawtex-core 的 Hands 系統可以在保持 TOML 宣告式設計優勢的同時, 借鑑 CrewAI 的角色設定、條件分支、輸出驗證和統一記憶設計, 大幅提升編排能力。
