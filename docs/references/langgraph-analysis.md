# LangGraph 深度技術分析

> 分析日期: 2026-03-12
> 來源: `LLM-Cluster-Project/references/langgraph/` (main branch)
> 版本: LangGraph v1.x (monorepo 架構)

---

## 目錄

1. [專案結構](#1-專案結構)
2. [入口點: 如何建立並執行 Graph](#2-入口點-如何建立並執行-graph)
3. [核心架構](#3-核心架構)
4. [Agent 模式](#4-agent-模式)
5. [工具整合](#5-工具整合)
6. [串流機制](#6-串流機制)
7. [Human-in-the-Loop](#7-human-in-the-loop)
8. [子圖 (Subgraphs)](#8-子圖-subgraphs)
9. [LangGraph Platform](#9-langgraph-platform)
10. [值得採用的關鍵模式](#10-值得採用的關鍵模式)
11. [Clawtex Hands 引擎如何採用圖模式](#11-clawtex-hands-引擎如何採用圖模式)

---

## 1. 專案結構

LangGraph 採用 **monorepo** 架構, 所有套件位於 `libs/` 目錄下:

```
langgraph/
  libs/
    langgraph/           # 核心框架 -- 圖定義、Pregel 執行引擎、Channels
    prebuilt/            # 高階 API -- create_react_agent、ToolNode
    checkpoint/          # 檢查點基底介面 + InMemorySaver
    checkpoint-postgres/ # PostgreSQL 檢查點實作
    checkpoint-sqlite/   # SQLite 檢查點實作
    checkpoint-conformance/ # 檢查點一致性測試套件
    cli/                 # CLI 工具 (Docker 部署、開發伺服器)
    sdk-py/              # Python SDK (REST API 客戶端)
    sdk-js/              # JavaScript/TypeScript SDK
  examples/              # Jupyter notebook 範例 (ReAct、RAG、多 Agent 等)
  docs/                  # 文件
```

### 核心套件依賴關係

```
checkpoint
  +-- checkpoint-postgres
  +-- checkpoint-sqlite
  +-- prebuilt
  +-- langgraph

prebuilt
  +-- langgraph

sdk-py
  +-- langgraph
  +-- cli
```

### 關鍵目錄結構 (核心 `libs/langgraph/langgraph/`)

```
langgraph/
  graph/
    __init__.py          # 公開 API: StateGraph, START, END, MessagesState, add_messages
    state.py             # StateGraph 建構器 + CompiledStateGraph
    _branch.py           # BranchSpec -- 條件邊的路由邏輯
    _node.py             # StateNodeSpec -- 節點型別定義與 Protocol
    message.py           # MessagesState, add_messages reducer, MessageGraph (已棄用)
    ui.py                # UIMessage 系統 -- push_ui_message, ui_message_reducer
  pregel/
    main.py              # Pregel 執行引擎核心 + NodeBuilder
    _loop.py             # PregelLoop -- 同步/非同步執行迴圈
    _algo.py             # 任務排程演算法 (prepare_next_tasks, apply_writes)
    _checkpoint.py       # 檢查點 CRUD 操作
    _runner.py           # PregelRunner -- 任務執行器
    _retry.py            # RetryPolicy 實作
    _io.py               # 輸入/輸出映射
    protocol.py          # PregelProtocol -- 核心介面定義 (get_state, stream...)
    remote.py            # 遠端 Pregel 代理
  channels/
    base.py              # BaseChannel 抽象基類
    last_value.py        # LastValue -- 儲存最後一個值 (預設 channel)
    binop.py             # BinaryOperatorAggregate -- 二元運算聚合
    topic.py             # Topic -- PubSub channel
    ephemeral_value.py   # EphemeralValue -- 短暫值 (不持久化)
  types.py               # 核心型別: Send, Command, interrupt, StateSnapshot, StreamMode...
  runtime.py             # Runtime 物件 -- 注入 context, store, stream_writer
  constants.py           # START, END, TAG_HIDDEN 等常量
  errors.py              # 錯誤型別定義
```

**檔案路徑:**
- 核心圖: `libs/langgraph/langgraph/graph/state.py`
- Pregel 引擎: `libs/langgraph/langgraph/pregel/main.py`
- 型別定義: `libs/langgraph/langgraph/types.py`
- 預建代理: `libs/prebuilt/langgraph/prebuilt/chat_agent_executor.py`

---

## 2. 入口點: 如何建立並執行 Graph

### 基本流程: 定義 -> 編譯 -> 執行

LangGraph 的核心使用模式是 **Builder Pattern**: 先定義圖結構, 然後 `.compile()` 成可執行物件.

```python
from typing_extensions import TypedDict, Annotated
from langgraph.graph import StateGraph, START, END
from langgraph.checkpoint.memory import InMemorySaver

# 1. 定義狀態 Schema
class State(TypedDict):
    x: int
    messages: Annotated[list, add_messages]  # 使用 reducer

# 2. 建構圖
builder = StateGraph(State)

# 3. 添加節點 (函式或 Runnable)
def my_node(state: State) -> dict:
    return {"x": state["x"] + 1}

builder.add_node(my_node)              # 自動使用函式名稱
builder.add_node("custom_name", fn)    # 自訂名稱

# 4. 定義邊
builder.add_edge(START, "my_node")     # 入口邊
builder.add_edge("my_node", END)       # 結束邊

# 5. 編譯
graph = builder.compile(
    checkpointer=InMemorySaver(),       # 啟用持久化
    interrupt_before=["my_node"],       # 設定中斷點
)

# 6. 執行
result = graph.invoke({"x": 0})
# 或串流
for chunk in graph.stream({"x": 0}, stream_mode="updates"):
    print(chunk)
```

**來源檔案:** `libs/langgraph/langgraph/graph/state.py` 第 115-200 行 (StateGraph class)

### compile() 做了什麼

`StateGraph.compile()` 方法 (第 1038-1193 行) 執行以下步驟:

1. **驗證圖結構** -- 確保所有邊的起點和終點都存在, START 有出邊
2. **準備 Channels** -- 根據 state schema 建立 channel 映射
3. **建構 CompiledStateGraph** -- 繼承自 `Pregel` 的可執行圖
4. **附加節點** -- 每個節點轉成 `PregelNode` (含 channel 讀寫邏輯)
5. **附加邊** -- 直接邊和條件邊
6. **回傳已編譯圖** -- 實作了 `Runnable` 介面

```python
# libs/langgraph/langgraph/graph/state.py 第 1038-1085 行
def compile(
    self,
    checkpointer: Checkpointer = None,
    *,
    cache: BaseCache | None = None,
    store: BaseStore | None = None,
    interrupt_before: All | list[str] | None = None,
    interrupt_after: All | list[str] | None = None,
    debug: bool = False,
    name: str | None = None,
) -> CompiledStateGraph[StateT, ContextT, InputT, OutputT]:
    # ...validate, create channels, build compiled graph...
    compiled = CompiledStateGraph(
        builder=self,
        channels={**self.channels, **self.managed, START: EphemeralValue(...)},
        input_channels=START,
        output_channels=output_channels,
        checkpointer=checkpointer,
        interrupt_before_nodes=interrupt_before,
        interrupt_after_nodes=interrupt_after,
        # ...
    )
```

---

## 3. 核心架構

### 3.1 StateGraph: 節點與邊的定義

**StateGraph** 是 LangGraph 的核心建構器. 它基於 **Pregel 演算法** (Google 的分散式圖計算模型): 節點是 Actor, 透過 Channel 通訊.

**檔案:** `libs/langgraph/langgraph/graph/state.py`

#### 節點 (Nodes)

節點簽名: `State -> Partial<State>`. 每個節點接收完整狀態, 回傳要更新的部分.

```python
# libs/langgraph/langgraph/graph/_node.py

# 節點可以接受多種簽名:
class _Node(Protocol[NodeInputT_contra]):
    def __call__(self, state: NodeInputT_contra) -> Any: ...

class _NodeWithConfig(Protocol[NodeInputT_contra]):
    def __call__(self, state: NodeInputT_contra, config: RunnableConfig) -> Any: ...

class _NodeWithWriter(Protocol[NodeInputT_contra]):
    def __call__(self, state: NodeInputT_contra, *, writer: StreamWriter) -> Any: ...

class _NodeWithRuntime(Protocol[NodeInputT_contra, ContextT]):
    def __call__(self, state: NodeInputT_contra, *, runtime: Runtime[ContextT]) -> Any: ...

# StateNode 是上述所有變體的聯合型別
StateNode: TypeAlias = (
    _Node[NodeInputT] | _NodeWithConfig[NodeInputT] | _NodeWithWriter[NodeInputT]
    | _NodeWithStore[NodeInputT] | _NodeWithRuntime[NodeInputT, ContextT]
    | Runnable[NodeInputT, Any]
)
```

每個節點被包裝成 `StateNodeSpec`:

```python
@dataclass(slots=True)
class StateNodeSpec(Generic[NodeInputT, ContextT]):
    runnable: StateNode[NodeInputT, ContextT]
    metadata: dict[str, Any] | None
    input_schema: type[NodeInputT]
    retry_policy: RetryPolicy | Sequence[RetryPolicy] | None
    cache_policy: CachePolicy | None
    ends: tuple[str, ...] | dict[str, str] | None = EMPTY_SEQ
    defer: bool = False
```

#### 邊 (Edges)

三種邊型別:

```python
# 1. 直接邊: A -> B
builder.add_edge("node_a", "node_b")

# 2. 等待邊: [A, B] -> C (等待 A 和 B 都完成後才執行 C)
builder.add_edge(["node_a", "node_b"], "node_c")

# 3. 條件邊: A -> ? (根據函式回傳值決定下一步)
builder.add_conditional_edges(
    "node_a",
    routing_function,           # (state) -> str | list[str]
    path_map={"yes": "node_b", "no": END}
)
```

#### 序列快捷方式

```python
# add_sequence -- 按順序串連節點
builder.add_sequence([node_a, node_b, node_c])
# 等同於:
# add_node(node_a); add_node(node_b); add_node(node_c)
# add_edge(node_a, node_b); add_edge(node_b, node_c)
```

### 3.2 狀態管理: Channel 系統

**LangGraph 的核心創新: 用 Channel 管理狀態流動.**

每個狀態欄位背後是一個 Channel, Channel 決定了值如何被更新.

**檔案:** `libs/langgraph/langgraph/channels/base.py`

```python
class BaseChannel(Generic[Value, Update, Checkpoint], ABC):
    """所有 Channel 的基底類別"""

    @abstractmethod
    def get(self) -> Value:
        """取得當前值"""

    @abstractmethod
    def update(self, values: Sequence[Update]) -> bool:
        """用一組更新來修改 channel 值 (多個節點的輸出在此合併)"""

    def checkpoint(self) -> Checkpoint | Any:
        """回傳可序列化的 channel 狀態 (用於持久化)"""

    @abstractmethod
    def from_checkpoint(self, checkpoint: Checkpoint | Any) -> Self:
        """從 checkpoint 恢復 channel"""
```

#### Channel 類型

| Channel 類型 | 用途 | 行為 |
|---|---|---|
| `LastValue` | 預設 -- 儲存最新值 | 只保留最後一次寫入 |
| `BinaryOperatorAggregate` | 聚合 -- 用 reducer 合併 | `value = reducer(current, update)` |
| `EphemeralValue` | 短暫值 -- 每步重置 | 執行後自動清除 |
| `Topic` | PubSub -- 多值累積 | 收集所有寫入值為列表 |
| `NamedBarrierValue` | 屏障 -- 等待多方寫入 | 等所有指定節點都寫入後才允許讀取 |

#### Reducer 機制

```python
from typing import Annotated
import operator

class State(TypedDict):
    # LastValue channel (預設 -- 無 reducer)
    current_step: str

    # BinaryOperatorAggregate channel (使用 reducer)
    messages: Annotated[list, add_messages]    # 合併訊息列表
    total: Annotated[int, operator.add]        # 累加
    items: Annotated[list, operator.add]       # 串接列表

# add_messages 是特殊 reducer (libs/langgraph/langgraph/graph/message.py):
# - 按 ID 合併 (相同 ID 的訊息被替換)
# - 支援 RemoveMessage (刪除指定 ID 的訊息)
# - 支援 REMOVE_ALL_MESSAGES (清空所有訊息)
```

**來源:** `libs/langgraph/langgraph/channels/binop.py` -- `BinaryOperatorAggregate.update()`:

```python
def update(self, values: Sequence[Value]) -> bool:
    if not values:
        return False
    if self.value is MISSING:
        self.value = values[0]
        values = values[1:]
    for value in values:
        is_overwrite, overwrite_value = _get_overwrite(value)
        if is_overwrite:
            self.value = overwrite_value  # 完全覆蓋
        else:
            self.value = self.operator(self.value, value)  # 使用 reducer
    return True
```

### 3.3 條件邊: 路由決策

**檔案:** `libs/langgraph/langgraph/graph/_branch.py`

條件邊由 `BranchSpec` 實作. 路由函式接收當前狀態, 回傳下一個節點名稱:

```python
class BranchSpec(NamedTuple):
    path: Runnable[Any, Hashable | list[Hashable]]  # 路由函式
    ends: dict[Hashable, str] | None                  # 路由對應表
    input_schema: type[Any] | None = None

    def _route(self, input, config, *, reader, writer):
        if reader:
            value = reader(config)
        else:
            value = input
        result = self.path.invoke(value, config)
        return self._finish(writer, input, result, config)

    def _finish(self, writer, input, result, config):
        if not isinstance(result, (list, tuple)):
            result = [result]
        if self.ends:
            destinations = [self.ends[r] if not isinstance(r, Send) else r for r in result]
        else:
            destinations = result
        # 寫入目標 channel, 觸發下游節點
        entries = writer(destinations, False)
        ChannelWrite.do_write(config, entries)
        return input
```

**關鍵功能: 路由函式可以回傳 `Send` 物件, 實現動態並行 fan-out:**

```python
from langgraph.types import Send

def route_to_workers(state: State) -> list[Send]:
    """將每個任務發送到不同的 worker 節點, 每個帶有自訂輸入"""
    return [
        Send("worker", {"task": task, "context": state["context"]})
        for task in state["tasks"]
    ]

builder.add_conditional_edges("planner", route_to_workers)
```

### 3.4 Pregel 執行引擎

**檔案:** `libs/langgraph/langgraph/pregel/main.py`

LangGraph 的執行模型基於 **Pregel 演算法 / BSP (Bulk Synchronous Parallel)**:

```
Pregel Algorithm:
  每一步 (superstep) 分三個階段:
    1. Plan:  決定本步要執行哪些 Actor (節點)
    2. Execute: 並行執行所有選中的 Actor
    3. Update: 用 Actor 的輸出更新 Channel

  重複直到:
    - 沒有節點需要執行
    - 達到最大步數限制 (recursion_limit)
    - 發生中斷 (interrupt)
```

**Pregel 類別** (第 332-585 行) 是核心執行器:

```python
class Pregel(PregelProtocol[StateT, ContextT, InputT, OutputT]):
    """Pregel 管理 LangGraph 應用的執行行為.

    結合 actors 和 channels 為單一應用.
    Actors 從 channels 讀取資料, 並寫入資料到 channels.
    """

    nodes: dict[str, PregelNode]
    channels: dict[str, BaseChannel | ManagedValueSpec]
    checkpointer: Checkpointer = None
    interrupt_before_nodes: All | Sequence[str]
    interrupt_after_nodes: All | Sequence[str]
    # ...
```

**PregelLoop** (第 142-300 行) 管理執行迴圈的完整生命週期:

```python
class PregelLoop:
    status: Literal["input", "pending", "done",
                     "interrupt_before", "interrupt_after", "out_of_steps"]
    step: int
    stop: int         # recursion_limit
    tasks: dict[str, PregelExecutableTask]

    # 迴圈邏輯:
    # 1. 從 checkpoint 恢復 channels
    # 2. 映射輸入到 channels
    # 3. while 有待執行任務:
    #    a. prepare_next_tasks() -- 根據 channel 版本找需執行的節點
    #    b. 檢查 interrupt_before
    #    c. 並行執行所有任務
    #    d. apply_writes() -- 將任務輸出寫入 channels
    #    e. 檢查 interrupt_after
    #    f. 儲存 checkpoint
    # 4. 讀取輸出 channels
```

### 3.5 Checkpoint 系統: 圖狀態持久化與恢復

**檔案:**
- 基底: `libs/checkpoint/langgraph/checkpoint/base/__init__.py`
- 記憶體: `libs/checkpoint/langgraph/checkpoint/memory/__init__.py`
- PostgreSQL: `libs/checkpoint-postgres/`
- SQLite: `libs/checkpoint-sqlite/`

#### Checkpoint 資料結構

```python
class Checkpoint(TypedDict):
    v: int                              # 格式版本 (當前為 1)
    id: str                             # 唯一 + 單調遞增 ID (用於排序)
    ts: str                             # ISO 8601 時間戳
    channel_values: dict[str, Any]      # 所有 channel 的值快照
    channel_versions: ChannelVersions   # 每個 channel 的版本號
    versions_seen: dict[str, ChannelVersions]  # 每個節點已看過的 channel 版本

class CheckpointMetadata(TypedDict, total=False):
    source: Literal["input", "loop", "update", "fork"]
    step: int                           # -1 表示初始輸入, 0+ 表示執行步驟
    parents: dict[str, str]             # 父 checkpoint ID (用於 namespace)
    run_id: str
```

#### BaseCheckpointSaver 介面

```python
class BaseCheckpointSaver(Generic[V]):
    serde: SerializerProtocol  # JSON+、MessagePack、或加密序列化

    def get_tuple(self, config: RunnableConfig) -> CheckpointTuple | None: ...
    def list(self, config, *, filter, before, limit) -> Iterator[CheckpointTuple]: ...
    def put(self, config, checkpoint, metadata, new_versions) -> RunnableConfig: ...
    def put_writes(self, config, writes, task_id) -> None: ...

    # 非同步版本
    async def aget_tuple(...): ...
    async def alist(...): ...
    async def aput(...): ...
    async def aput_writes(...): ...
```

#### 使用方式 -- thread_id 是核心概念

```python
from langgraph.checkpoint.memory import InMemorySaver

checkpointer = InMemorySaver()
graph = builder.compile(checkpointer=checkpointer)

# thread_id 是狀態持久化的主鍵
config = {"configurable": {"thread_id": "conversation-1"}}

# 第一次呼叫 -- 建立新 checkpoint
result1 = graph.invoke({"messages": [("user", "hi")]}, config)

# 第二次呼叫 -- 從上次 checkpoint 繼續
result2 = graph.invoke({"messages": [("user", "how are you")]}, config)

# 時間旅行 -- 取得歷史狀態
for snapshot in graph.get_state_history(config):
    print(snapshot.values, snapshot.metadata["step"])

# 狀態更新 -- 手動修改狀態
graph.update_state(config, {"messages": [...]}, as_node="agent")
```

---

## 4. Agent 模式

### 4.1 ReAct Agent

**檔案:** `libs/prebuilt/langgraph/prebuilt/chat_agent_executor.py`

`create_react_agent()` 是最常用的預建模式. 其圖結構為:

```
START -> [pre_model_hook ->] agent -> should_continue -> tools -> agent -> ...
                                   |                              |
                                   +-> [post_model_hook ->] END   |
                                                                  v
                                                             (loop back)
```

核心實作 (第 861-995 行):

```python
def create_react_agent(model, tools, *, prompt=None, ...):
    # 建構圖
    workflow = StateGraph(state_schema=state_schema)

    # 添加 LLM 呼叫節點
    workflow.add_node("agent", RunnableCallable(call_model, acall_model))

    # 添加工具執行節點
    workflow.add_node("tools", tool_node)

    # 入口
    workflow.set_entry_point("agent")

    # 條件邊: agent 之後決定是呼叫工具還是結束
    def should_continue(state):
        last_message = state["messages"][-1]
        if not isinstance(last_message, AIMessage) or not last_message.tool_calls:
            return END      # 沒有工具呼叫 -> 結束
        else:
            # v2: 用 Send 並行處理每個 tool_call
            return [
                Send("tools", ToolCallWithContext(tool_call=call, state=state))
                for call in last_message.tool_calls
            ]

    workflow.add_conditional_edges("agent", should_continue)
    workflow.add_edge("tools", "agent")  # 工具結果回到 agent

    return workflow.compile(checkpointer=checkpointer, ...)
```

**關鍵設計:**
- `version="v2"` 使用 `Send` 將每個 tool call 並行發送到獨立的 `ToolNode` 實例
- `remaining_steps` 追蹤剩餘步數, 防止無限迴圈
- `pre_model_hook` / `post_model_hook` 提供前處理/後處理擴展點

### 4.2 Plan-and-Execute

**範例:** `examples/plan-and-execute/plan-and-execute.ipynb`

此模式將規劃與執行分離:

```
planner -> executor -> re-planner -> executor -> ... -> END
```

### 4.3 多 Agent

**範例:** `examples/multi_agent/`

LangGraph 支援兩種多 Agent 模式:

1. **Supervisor**: 一個 supervisor 節點路由到多個 worker 節點
2. **Hierarchical Teams**: 巢狀的 supervisor-worker 結構

```python
# Supervisor 模式
def supervisor(state):
    # LLM 決定下一步
    response = llm.invoke(state["messages"])
    return {"next": response.next_agent}

builder.add_node("supervisor", supervisor)
builder.add_node("researcher", researcher_agent)
builder.add_node("coder", coder_agent)

builder.add_conditional_edges("supervisor", lambda s: s["next"],
    {"researcher": "researcher", "coder": "coder", "FINISH": END})
```

### 4.4 其他模式

| 模式 | 範例路徑 | 說明 |
|---|---|---|
| Reflection | `examples/reflection/reflection.ipynb` | 自我反思迴圈 |
| Reflexion | `examples/reflexion/reflexion.ipynb` | 反思 + 外部回饋 |
| LATS | `examples/lats/lats.ipynb` | Language Agent Tree Search |
| Self-RAG | `examples/rag/langgraph_self_rag.ipynb` | 自適應 RAG |
| CRAG | `examples/rag/langgraph_crag.ipynb` | Corrective RAG |
| ReWOO | `examples/rewoo/rewoo.ipynb` | Reasoning without Observation |
| LLM Compiler | `examples/llm-compiler/LLMCompiler.ipynb` | 並行函式呼叫 |

---

## 5. 工具整合

**檔案:** `libs/prebuilt/langgraph/prebuilt/tool_node.py`

### ToolNode

`ToolNode` 是 LangGraph 的工具執行器, 特色包括:

- **並行執行**: 多個 tool call 並行處理
- **錯誤處理**: 自訂錯誤訊息模板
- **狀態注入**: 透過 `InjectedState` 將圖狀態注入工具
- **Store 注入**: 透過 `InjectedStore` 將持久化存儲注入工具
- **Command 回傳**: 工具可回傳 `Command` 進行狀態更新和路由控制

```python
from langchain_core.tools import tool

@tool
def search(query: str) -> str:
    """搜尋網路"""
    return f"Results for: {query}"

# ToolNode 包裝多個工具
tool_node = ToolNode([search, calculator, weather])

# tools_condition: 標準的條件路由函式
def tools_condition(state, messages_key="messages") -> Literal["tools", "__end__"]:
    """如果最後一條 AIMessage 有 tool_calls, 路由到 'tools'; 否則結束"""
    messages = state[messages_key]
    last_message = messages[-1]
    if isinstance(last_message, AIMessage) and last_message.tool_calls:
        return "tools"
    return END
```

### ToolCallRequest -- 工具呼叫攔截器

```python
@dataclass
class ToolCallRequest:
    tool_call: ToolCall       # 工具呼叫資訊 (name, args, id)
    tool: BaseTool | None     # 工具實例
    state: Any                # 當前圖狀態
    runtime: ToolRuntime      # 執行時上下文
```

### 狀態注入模式

```python
from langgraph.prebuilt import InjectedState, InjectedStore

@tool
def my_tool(
    query: str,
    state: Annotated[dict, InjectedState],     # 注入圖狀態
    store: Annotated[BaseStore, InjectedStore], # 注入存儲
) -> str:
    context = state.get("context", "")
    # 從 store 讀取記憶
    memories = store.search(("memories",), query=query)
    return f"Context: {context}, Memories: {memories}"
```

---

## 6. 串流機制

**檔案:** `libs/langgraph/langgraph/types.py` (StreamMode 定義)

LangGraph 支援 7 種串流模式:

```python
StreamMode = Literal[
    "values",       # 每步後發送完整狀態
    "updates",      # 只發送節點的更新
    "messages",     # Token-by-token LLM 串流 + 元資料
    "custom",       # 自訂串流 (透過 StreamWriter)
    "checkpoints",  # 檢查點事件
    "tasks",        # 任務開始/完成事件
    "debug",        # 除錯事件 (checkpoints + tasks)
]
```

### 使用方式

```python
# 基本串流
for chunk in graph.stream(inputs, stream_mode="updates"):
    print(chunk)  # {"node_name": {"key": "value"}}

# 多模式串流
for chunk in graph.stream(inputs, stream_mode=["values", "messages"]):
    if chunk["type"] == "values":
        print("State:", chunk["data"])
    elif chunk["type"] == "messages":
        msg, metadata = chunk["data"]
        print(f"Token from {metadata['langgraph_node']}: {msg.content}")

# 自訂串流 (從節點內部)
def my_node(state, *, writer: StreamWriter):
    writer({"progress": 0.5})   # 發送自訂資料
    # ... 處理 ...
    writer({"progress": 1.0})
    return {"result": "done"}
```

### 串流資料型別

```python
# 每種模式對應一個 TypedDict:
class ValuesStreamPart(TypedDict):
    type: Literal["values"]
    ns: tuple[str, ...]      # namespace (子圖路徑)
    data: OutputT             # 完整狀態
    interrupts: tuple[Interrupt, ...]

class UpdatesStreamPart(TypedDict):
    type: Literal["updates"]
    ns: tuple[str, ...]
    data: dict[str, Any]     # {節點名: 輸出}

class MessagesStreamPart(TypedDict):
    type: Literal["messages"]
    ns: tuple[str, ...]
    data: tuple[AnyMessage, dict[str, Any]]  # (訊息, 元資料)

class CustomStreamPart(TypedDict):
    type: Literal["custom"]
    ns: tuple[str, ...]
    data: Any                # 使用者自訂資料
```

---

## 7. Human-in-the-Loop

LangGraph 提供兩種人機互動機制:

### 7.1 interrupt() 函式

**檔案:** `libs/langgraph/langgraph/types.py` 第 705-794 行

```python
def interrupt(value: Any) -> Any:
    """從節點內部中斷圖執行.

    - 第一次呼叫: 拋出 GraphInterrupt, 暫停執行, 將 value 傳給客戶端
    - 恢復執行時: 回傳客戶端提供的 resume 值
    - 需要啟用 checkpointer
    """
```

使用範例:

```python
def review_node(state: State):
    # 暫停執行, 請求人類審核
    answer = interrupt({
        "question": "是否批准此操作?",
        "details": state["proposed_action"]
    })
    # answer 是人類回覆的值
    if answer == "approved":
        return {"status": "approved"}
    else:
        return {"status": "rejected"}

# 客戶端恢復執行:
graph.invoke(Command(resume="approved"), config)
```

### 7.2 interrupt_before / interrupt_after

在編譯時設定自動中斷點:

```python
graph = builder.compile(
    checkpointer=InMemorySaver(),
    interrupt_before=["dangerous_node"],   # 在節點執行前中斷
    interrupt_after=["review_node"],        # 在節點執行後中斷
)

# 執行時自動暫停
result = graph.invoke(inputs, config)
# result 包含 __interrupt__ 資訊

# 查看狀態
state = graph.get_state(config)
print(state.next)  # 下一步要執行的節點

# 手動修改狀態後繼續
graph.update_state(config, {"approved": True})
graph.invoke(None, config)  # 從中斷處繼續
```

### 7.3 HumanInterrupt 結構 (prebuilt)

**檔案:** `libs/prebuilt/langgraph/prebuilt/interrupt.py`

```python
class HumanInterruptConfig(TypedDict):
    allow_ignore: bool     # 允許跳過
    allow_respond: bool    # 允許文字回覆
    allow_edit: bool       # 允許編輯
    allow_accept: bool     # 允許直接批准

class ActionRequest(TypedDict):
    action: str           # 動作名稱
    args: dict            # 動作參數

class HumanInterrupt(TypedDict):
    action_request: ActionRequest
    config: HumanInterruptConfig
    description: str | None

class HumanResponse(TypedDict):
    type: Literal["accept", "ignore", "response", "edit"]
    args: None | str | ActionRequest
```

### 7.4 Command 物件

**檔案:** `libs/langgraph/langgraph/types.py` 第 652-700 行

`Command` 是 LangGraph 中最強大的控制流原語:

```python
@dataclass
class Command(Generic[N]):
    graph: str | None = None          # None=當前圖, Command.PARENT=父圖
    update: Any | None = None          # 狀態更新
    resume: dict[str, Any] | Any = None  # 恢復中斷的值
    goto: Send | Sequence[Send | N] | N = ()  # 跳轉到指定節點

# 使用範例:
# 1. 從工具內跳轉到特定節點
return Command(goto="summarize", update={"data": result})

# 2. 從子圖發送命令到父圖
return Command(graph=Command.PARENT, update={"child_result": data})

# 3. 恢復中斷
graph.invoke(Command(resume="user input"), config)
```

---

## 8. 子圖 (Subgraphs)

LangGraph 支援將圖組合成更大的圖:

### 編譯的子圖作為節點

```python
# 定義子圖
sub_builder = StateGraph(SubState)
sub_builder.add_node("sub_a", sub_node_a)
sub_builder.add_edge(START, "sub_a")
sub_builder.add_edge("sub_a", END)
sub_graph = sub_builder.compile()

# 將子圖作為節點加入主圖
main_builder = StateGraph(MainState)
main_builder.add_node("sub_process", sub_graph)
main_builder.add_edge(START, "sub_process")
main_builder.add_edge("sub_process", END)
main_graph = main_builder.compile()
```

### Checkpointer 繼承

```python
# 子圖繼承父圖的 checkpointer
sub_graph = sub_builder.compile()  # checkpointer=None -> 繼承父圖

# 子圖使用獨立 checkpointer
sub_graph = sub_builder.compile(checkpointer=True)  # 啟用獨立持久化

# 禁用子圖的 checkpointing
sub_graph = sub_builder.compile(checkpointer=False)
```

### Namespace 隔離

子圖在 checkpoint 中使用 namespace 隔離:
- 父圖 namespace: `""`
- 子圖 namespace: `"sub_process"` (節點名稱)
- 巢狀子圖: `"sub_process|inner_sub"`

```python
# 查看子圖狀態
state = main_graph.get_state(config, subgraphs=True)
for task in state.tasks:
    if task.state:  # 子圖的 StateSnapshot
        print(task.name, task.state.values)
```

### 跨圖通訊

```python
# 子圖節點可以透過 Command.PARENT 向父圖發送更新:
def sub_node(state):
    return Command(
        graph=Command.PARENT,
        update={"parent_key": "value from child"}
    )
```

---

## 9. LangGraph Platform

### CLI 工具

**檔案:** `libs/cli/langgraph_cli/cli.py`

LangGraph CLI 提供:
- `langgraph dev` -- 啟動開發伺服器
- `langgraph build` -- 建構 Docker 映像
- `langgraph up` -- 用 Docker Compose 啟動
- `langgraph dockerfile` -- 生成 Dockerfile

### SDK (Python + JavaScript)

**Python SDK:** `libs/sdk-py/langgraph_sdk/client.py`

```python
from langgraph_sdk import get_client

client = get_client(url="http://localhost:2024")

# 核心 API:
# - Assistants: 管理 Agent 定義
# - Threads: 管理對話線程
# - Runs: 執行 Agent
# - Store: 持久化存儲
# - Cron: 排程任務

# 建立線程
thread = await client.threads.create()

# 在線程上執行
run = await client.runs.create(
    thread["thread_id"],
    assistant_id="agent",
    input={"messages": [{"role": "user", "content": "hello"}]}
)

# 串流執行
async for chunk in client.runs.stream(
    thread["thread_id"],
    assistant_id="agent",
    input={"messages": [...]},
    stream_mode="messages",
):
    print(chunk)
```

### Platform 額外功能

- **Cron Jobs**: 排程執行
- **Assistants**: Agent 版本管理
- **Threads**: 對話狀態管理
- **Store**: 跨線程持久化存儲
- **Webhooks**: 事件通知
- **Auth**: 認證與授權

---

## 10. 值得採用的關鍵模式

### 10.1 圖式工作流 vs Clawtex 線性 Phase 模型

| 特性 | LangGraph (圖) | Clawtex (線性 Phase) |
|---|---|---|
| 流程控制 | 任意有向圖, 支援迴圈、分支、並行 | 嚴格線性: Phase 1 -> 2 -> 3 -> ... |
| 條件路由 | `add_conditional_edges` + 路由函式 | 無 (Phase 順序固定) |
| 並行執行 | 原生支援 (Send, 多節點同步執行) | 無 (逐 Phase 執行) |
| 迴圈 | 自然支援 (A -> B -> A) | 不支援 |
| 錯誤恢復 | Checkpoint 恢復 + RetryPolicy | 無內建持久化 |
| 動態路由 | 節點可回傳 `Command(goto=...)` | `chain_to` 靜態指定 |

**關鍵洞見:**
- LangGraph 的圖模型天然適合 Agent 工作流: LLM 決策導致的分支、工具呼叫迴圈、多 Agent 協作
- Clawtex 的線性 Phase 模型適合確定性工作流, 但缺乏處理 LLM 非確定性輸出的能力

### 10.2 條件路由在 Phase 轉換

LangGraph 的條件邊機制可以直接應用到 Phase 轉換:

```python
# LangGraph 方式:
def route_after_research(state):
    if state["research_quality"] > 0.8:
        return "write_content"
    elif state["retries"] < 3:
        return "research"      # 迴圈重試
    else:
        return "fallback"

# Clawtex 可借鑒:
# 在 hand.toml 中支援 phase 條件轉換:
# [[phases]]
# name = "research"
# on_success = "write_content"
# on_retry = "research"       # 條件回退
# on_fail = "fallback"
# max_retries = 3
```

### 10.3 Checkpoint/Restore 對長時間工作流

LangGraph 的 Checkpoint 系統提供:

1. **失敗恢復**: 從最後成功的步驟繼續
2. **時間旅行**: 回到任何歷史狀態
3. **狀態分叉**: 從歷史點建立新分支
4. **人機互動**: 暫停/恢復跨越任意時間

**Clawtex 可借鑒:**
- 為 HandRunner 加入 checkpoint 機制
- 每個 Phase 完成後持久化狀態
- 失敗時從最後完成的 Phase 恢復
- 支援 `interrupt()` 式的人機互動

### 10.4 Channel/Reducer 模式

LangGraph 的 Reducer 是一個優雅的狀態合併策略:

```python
# 不同欄位可以有不同的合併策略:
class State(TypedDict):
    messages: Annotated[list, add_messages]     # 按 ID 合併訊息
    total_cost: Annotated[float, operator.add]  # 累加
    last_action: str                             # 覆蓋 (LastValue)
    all_sources: Annotated[list, operator.add]  # 串接列表
```

這對 Clawtex 的多 Phase 結果合併非常有用.

### 10.5 Send -- 動態並行

`Send` 允許在執行時動態決定並行度:

```python
def fan_out(state):
    return [Send("worker", {"task": t}) for t in state["tasks"]]
```

這比靜態的並行設定更靈活, 適合 Clawtex 的 cluster 任務分派.

### 10.6 Runtime 注入

LangGraph v0.6+ 的 `Runtime` 物件提供乾淨的依賴注入:

```python
@dataclass
class Context:
    user_id: str
    db_conn: Connection

def my_node(state: State, runtime: Runtime[Context]):
    user = runtime.context.user_id
    store = runtime.store
    writer = runtime.stream_writer
```

---

## 11. Clawtex Hands 引擎如何採用圖模式

### 現狀分析

Clawtex 的 Hands 引擎 (`src/hands/mod.rs`) 採用線性 Phase 模型:

```toml
# ~/.clawtex/hands/lead/hand.toml
[[phases]]
name = "research"
prompt = "..."
tools = ["web_search", "content_search"]

[[phases]]
name = "scoring"
prompt = "..."

[[phases]]
name = "output"
prompt = "..."
```

執行: Phase 1 -> Phase 2 -> Phase 3 -> (chain_to 下一個 Hand)

### 建議改進方案

#### 方案 A: 圖增強 Phase (最小改動)

保留 Phase 線性結構, 但加入條件轉換和 checkpoint:

```toml
# hand.toml 增強
[settings]
checkpoint = true              # 啟用 checkpoint

[[phases]]
name = "research"
prompt = "..."
tools = ["web_search"]
max_retries = 3
on_success = "scoring"         # 預設下一步
on_retry = "research"          # 質量不夠, 重試
on_fail = "fallback_research"  # 失敗處理

[[phases]]
name = "scoring"
prompt = "..."
condition = "research.quality > 0.7"  # 進入條件
on_success = "output"
on_low_score = "research"      # 分數太低, 回到研究
```

Rust 實作要點:

```rust
// src/hands/mod.rs 增強
struct PhaseSpec {
    name: String,
    prompt: String,
    tools: Vec<String>,
    max_retries: u32,
    transitions: HashMap<String, String>,  // 條件 -> 目標 Phase
    condition: Option<String>,             // 進入條件表達式
}

struct HandCheckpoint {
    hand_name: String,
    current_phase: String,
    phase_results: HashMap<String, serde_json::Value>,
    retry_count: u32,
    timestamp: chrono::DateTime<chrono::Utc>,
}

impl HandRunner {
    async fn run_with_checkpoint(&self, hand: &Hand, input: &str) -> Result<String> {
        // 嘗試恢復 checkpoint
        if let Some(checkpoint) = self.load_checkpoint(&hand.name).await? {
            return self.resume_from_checkpoint(hand, checkpoint).await;
        }

        let mut state = HandState::new(input);
        for phase in &hand.phases {
            // 檢查進入條件
            if let Some(cond) = &phase.condition {
                if !self.evaluate_condition(cond, &state) {
                    continue;
                }
            }

            // 執行 phase (含重試邏輯)
            let result = self.execute_phase_with_retry(phase, &mut state).await?;

            // 儲存 checkpoint
            self.save_checkpoint(&hand.name, &state).await?;

            // 條件路由
            let next = self.evaluate_transitions(&phase.transitions, &result);
            match next.as_str() {
                phase_name if hand.has_phase(phase_name) => {
                    // 跳轉到指定 phase (可能是回退)
                }
                "__end__" => break,
                _ => {} // 預設繼續下一個
            }
        }
        Ok(state.final_output())
    }
}
```

#### 方案 B: 完整圖引擎 (大改動)

引入 LangGraph 式的圖定義:

```toml
# hand.toml 圖模式
[graph]
type = "state_graph"

[graph.state]
messages = { type = "list", reducer = "append" }
research_data = { type = "object" }
score = { type = "float" }

[[graph.nodes]]
name = "research"
prompt = "..."
tools = ["web_search"]

[[graph.nodes]]
name = "scoring"
prompt = "..."

[[graph.nodes]]
name = "output"
prompt = "..."

[[graph.edges]]
from = "__start__"
to = "research"

[[graph.edges]]
from = "research"
to = "scoring"

[[graph.conditional_edges]]
from = "scoring"
function = "score_router"      # 內建路由函式
paths = { high = "output", low = "research", end = "__end__" }
```

#### 方案 C: 混合模式 (推薦)

保留 Phase 序列作為預設, 支援可選的圖增強:

```toml
[settings]
mode = "graph"  # 或 "sequential" (預設, 向後相容)
checkpoint = true

# Phase 定義 (向後相容)
[[phases]]
name = "research"
# ...

# 圖增強 (可選)
[[edges]]
from = "scoring"
to = "research"
condition = "score < 0.7"

[[edges]]
from = "scoring"
to = "output"
condition = "score >= 0.7"
```

### 具體建議清單

1. **Phase 重試 + 條件路由** (優先級: 高)
   - 在 `PhaseSpec` 加入 `max_retries` 和 `transitions`
   - 在 `HandRunner` 實作條件轉換邏輯
   - 對應 LangGraph: `add_conditional_edges`

2. **Hand Checkpoint** (優先級: 高)
   - 每個 Phase 完成後持久化狀態到 SQLite
   - 支援從中斷處恢復
   - 對應 LangGraph: `BaseCheckpointSaver`

3. **interrupt() 支援** (優先級: 中)
   - 在 Phase 內支援暫停等待人類輸入
   - 整合現有的 Telegram approval gate
   - 對應 LangGraph: `interrupt()` + `Command(resume=...)`

4. **並行 Phase** (優先級: 中)
   - 支援 `parallel: [phase_a, phase_b]` 語法
   - 用 tokio::join! 並行執行
   - 對應 LangGraph: `Send`

5. **State Reducer** (優先級: 低)
   - 為 Hand 狀態欄位定義合併策略
   - 特別是 messages 的 append 語義
   - 對應 LangGraph: `Annotated[list, operator.add]`

6. **子 Hand 組合** (優先級: 低)
   - 允許一個 Hand 作為另一個 Hand 的節點
   - 對應 LangGraph: 子圖

---

## 附錄: 關鍵檔案索引

| 功能 | 檔案路徑 |
|---|---|
| StateGraph 建構器 | `libs/langgraph/langgraph/graph/state.py` |
| Pregel 執行引擎 | `libs/langgraph/langgraph/pregel/main.py` |
| 執行迴圈 | `libs/langgraph/langgraph/pregel/_loop.py` |
| 條件邊 | `libs/langgraph/langgraph/graph/_branch.py` |
| 節點型別 | `libs/langgraph/langgraph/graph/_node.py` |
| Channel 基底 | `libs/langgraph/langgraph/channels/base.py` |
| BinaryOperatorAggregate | `libs/langgraph/langgraph/channels/binop.py` |
| 核心型別 (Send, Command, interrupt) | `libs/langgraph/langgraph/types.py` |
| Runtime 注入 | `libs/langgraph/langgraph/runtime.py` |
| add_messages reducer | `libs/langgraph/langgraph/graph/message.py` |
| create_react_agent | `libs/prebuilt/langgraph/prebuilt/chat_agent_executor.py` |
| ToolNode | `libs/prebuilt/langgraph/prebuilt/tool_node.py` |
| tools_condition | `libs/prebuilt/langgraph/prebuilt/tool_node.py` (第 1456 行) |
| HumanInterrupt | `libs/prebuilt/langgraph/prebuilt/interrupt.py` |
| UI 訊息系統 | `libs/langgraph/langgraph/graph/ui.py` |
| InMemorySaver | `libs/checkpoint/langgraph/checkpoint/memory/__init__.py` |
| BaseCheckpointSaver | `libs/checkpoint/langgraph/checkpoint/base/__init__.py` |
| Checkpoint 型別 | `libs/checkpoint/langgraph/checkpoint/base/__init__.py` (第 65 行) |
| PostgresSaver | `libs/checkpoint-postgres/langgraph/checkpoint/postgres/` |
| Python SDK | `libs/sdk-py/langgraph_sdk/client.py` |
| CLI | `libs/cli/langgraph_cli/cli.py` |
| 常量 (START, END) | `libs/langgraph/langgraph/constants.py` |

> 所有路徑相對於 `LLM-Cluster-Project/references/langgraph/`
