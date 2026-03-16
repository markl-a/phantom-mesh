# LangGraph 架構文檔

## 專案概覽

LangGraph 是 LangChain 公司開發的低階可編程狀態機框架，專為長期運行的有狀態代理設計。它採用圖論（Graph Theory）模式，將複雜工作流表示為節點和邊的有向圖，支持動態狀態管理、檢查點持久化和人類介入。

**核心特性**：
- 低階編程模型：完全控制節點和狀態轉換
- 檢查點持久化：自動保存執行狀態，支持失敗恢復
- 人類介入（HITL）：在執行中暫停、檢查、修改狀態
- LangSmith 集成：完整可觀測性與調試
- 多語言支持：Python 和 JavaScript/TypeScript

**適用場景**：
- 複雜狀態機工作流
- 需要人機互動的自動化
- 長期運行的代理系統
- 需要細粒度控制的決策引擎

## 目錄結構

```
langgraph/
├── libs/
│   ├── checkpoint/                # P0 檢查點基礎介面
│   │   ├── src/langgraph/checkpoint/
│   │   │   ├── base.py            # P0 BaseCheckpointSaver 抽象類
│   │   │   ├── protocol.py        # P0 CheckpointStorage 協議
│   │   │   └── types.py           # P0 配置類型（Checkpoint, Config）
│   │
│   ├── checkpoint-sqlite/         # P1 SQLite 檢查點實現
│   │   ├── langgraph/checkpoint/sqlite/
│   │   │   └── aiosqlite.py       # P1 非同步 SQLite 驅動
│   │
│   ├── checkpoint-postgres/       # P1 PostgreSQL 檢查點實現
│   │   ├── langgraph/checkpoint/postgres/
│   │   │   └── asyncpg.py         # P1 非同步 PostgreSQL 驅動
│   │
│   ├── langgraph/                 # P0 核心框架（主庫）
│   │   ├── src/langgraph/
│   │   │   ├── graph/
│   │   │   │   ├── graph.py       # P0 Graph 基類
│   │   │   │   ├── state.py       # P0 StateGraph 狀態管理
│   │   │   │   ├── message_graph.py # P1 MessageGraph（對話）
│   │   │   │   └── types.py       # P0 Node, Edge, Compiled 類型
│   │   │   │
│   │   │   ├── pregel/            # P0 圖編譯與執行引擎
│   │   │   │   ├── pregel.py      # P0 Pregel 編譯器
│   │   │   │   ├── runner.py      # P0 異步執行器
│   │   │   │   └── io.py          # P1 I/O 流
│   │   │   │
│   │   │   ├── channels/          # P0 狀態通道系統
│   │   │   │   ├── base.py        # P0 BaseChannel 抽象
│   │   │   │   ├── in_memory.py   # P0 InMemoryChannel
│   │   │   │   └── broadcast.py   # P1 廣播通道
│   │   │   │
│   │   │   ├── constants.py       # P0 CONFIG_KEYS, START/END 常數
│   │   │   ├── errors.py          # P0 GraphRecursionError 等
│   │   │   └── types.py           # P0 RunnableConfig, RetryPolicy
│   │   │
│   │   └── tests/
│   │
│   ├── prebuilt/                  # P1 高階 API（代理工具包）
│   │   ├── src/langgraph/prebuilt/
│   │   │   ├── chat_agent_executor.py  # P1 create_react_agent()
│   │   │   └── agent_executor.py       # P1 create_agent_executor()
│   │
│   ├── cli/                       # P1 命令行工具
│   │   ├── langgraph/cli/
│   │   │   ├── graph.py           # P1 圖驗證與類型檢查
│   │   │   └── server.py          # P1 本地開發服務器
│   │
│   ├── sdk-py/                    # P1 Python REST API SDK
│   │   ├── langgraph_sdk/
│   │   │   ├── client.py          # P1 LangGraphClient
│   │   │   └── types.py           # P1 Run, Config 類型
│   │
│   └── sdk-js/                    # P1 JavaScript/TypeScript SDK
│       ├── sdk-js/src/
│       │   ├── client.ts          # P1 LangGraphClient
│       │   └── types.ts           # P1 Run, Config 類型
│
├── examples/                      # 示例應用
│   ├── chatbot-simulation-evaluation/    # P1 聊天機器人評估
│   ├── multi_agent/                     # P1 多代理協作
│   ├── rag/                             # P1 RAG 應用
│   ├── tool-calling.ipynb               # P1 工具調用示例
│   └── ...
│
├── docs/                          # 官方文檔
├── pyproject.toml                 # 依賴配置
└── Makefile                       # 開發命令
```

## 核心 Class/Type

### Graph（圖基類）

```python
class Graph:
    """有向圖結構的基礎表示"""

    # 圖結構
    _nodes: dict[str, Runnable]      # P0 節點 ID → 執行函數
    _edges: list[tuple[str, str]]    # P0 邊列表 (source, target)

    # 主要方法
    def add_node(name: str, action: Callable) -> None  # P0 添加節點
    def add_edge(source: str, target: str) -> None      # P0 添加邊
    def compile() -> CompiledGraph                      # P0 編譯為可執行圖
```

### StateGraph（狀態圖）

```python
class StateGraph(Graph):
    """帶狀態傳遞的有向圖，主流工作流框架"""

    # 狀態定義
    _schema: TypedDict                # P0 狀態 TypedDict 定義
    _config_schema: TypedDict | None  # P0 動態配置 schema

    # 狀態減速器
    _state_reducers: dict             # P0 state_key → reducer 函數
                                      #    (reducer: Callable[[Any, Any], Any])

    # 編譯控制
    _debug: bool                      # P1 調試模式
    _interrupt_after: list[str]       # P1 中斷點列表
    _interrupt_before: list[str]      # P1 中斷前點

    # 主要方法
    def add_node(name, action) -> StateGraph         # P0 添加節點
    def add_edge(source, target) -> StateGraph       # P0 添加邊
    def add_conditional_edges(                       # P0 條件邊
        source: str,
        condition: Callable[[State], str | list[str]]
    ) -> StateGraph

    def compile(
        checkpointer: BaseCheckpointSaver | None = None,  # P1
        interrupt_after: list[str] | None = None,         # P1
        interrupt_before: list[str] | None = None,        # P1
        debug: bool = False                                # P1
    ) -> CompiledGraph                                # P0 編譯

    # 配置方法
    def set_entry_point(name: str) -> StateGraph     # P0 入口點
    def set_finish_point(name: str) -> StateGraph    # P0 終點
```

### CompiledGraph（編譯後的可執行圖）

```python
class CompiledGraph:
    """可執行的編譯圖，包含所有驗證和優化"""

    # 執行引擎
    _pregel: Pregel                   # P0 底層執行引擎

    # 狀態
    _state: TypedDict                 # P0 狀態 schema

    # 持久化
    _checkpointer: BaseCheckpointSaver | None  # P1 檢查點保存器

    # 主要方法
    def invoke(
        input: dict,
        config: RunnableConfig | None = None,      # P0/P1
        **kwargs
    ) -> dict                                      # P0 同步執行

    def stream(
        input: dict,
        config: RunnableConfig | None = None
    ) -> Iterator[tuple[str, dict]]               # P1 流式執行

    async def ainvoke(input, config=None) -> dict  # P1 非同步執行

    def get_state(config: RunnableConfig) -> State  # P1 獲取當前狀態

    def update_state(
        config: RunnableConfig,
        values: dict,
        as_node: str = ""
    ) -> dict                                      # P1 手動更新狀態
```

### Pregel（圖編譯引擎）

```python
class Pregel:
    """基於 Google Pregel 論文的並行圖執行引擎"""

    # 圖拓撲
    _graph: dict[str, Node]           # P0 編譯後的圖
    _edges: list[Edge]                # P0 邊連接

    # 狀態管理
    _channels: dict[str, Channel]     # P0 狀態通道
    _step: int                        # P0 當前步驟計數

    # 主要方法
    def invoke(state: dict) -> dict               # P0 執行
    def stream(state: dict) -> Iterator[dict]    # P1 流執行
    async def astream(state: dict)                # P1 非同步流

    # 內部
    def _next_step(state: dict) -> dict          # P0 下一步計算
    def _run_node(node: Node, state: dict)       # P0 執行單個節點
```

### Channel（狀態通道）

```python
class BaseChannel:
    """狀態通道抽象，實現狀態的讀寫與消費"""

    # 狀態
    _value: Any                       # P0 當前值

    # 主要方法
    def put(value: Any) -> None                  # P0 寫入值
    def get() -> Any                             # P0 讀取值
    def update(update: Any) -> None              # P0 應用更新


class InMemoryChannel(BaseChannel):
    """內存通道（預設）"""
    # 簡單 dict 後端，不支持檢查點


class BinaryOperatorChannel(BaseChannel):
    """應用二進制操作符的通道（例如 list.append）"""

    def __init__(self, operator: Callable):
        self.operator = operator    # P0 reduce operator
```

### RunnableConfig（執行配置）

```python
class RunnableConfig(TypedDict, total=False):
    """執行時配置字典"""

    # 識別符
    run_id: str                       # P0 執行 ID (UUID)
    thread_id: str                    # P1 線程 ID（用於檢查點）

    # 中斷點
    interrupt_after: list[str]        # P1 執行後中斷
    interrupt_before: list[str]       # P1 執行前中斷

    # 其他
    max_concurrency: int              # P1 最大並發度
    timeout: float                    # P1 超時秒數
    recursion_limit: int              # P1 遞迴限制
```

### Checkpoint（檢查點）

```python
@dataclass
class Checkpoint:
    """一個執行時間點的狀態快照"""

    # 標識符
    thread_id: str                    # P1 線程 ID
    checkpoint_id: str                # P1 檢查點 ID (UUID)
    parent_checkpoint_id: str | None  # P1 父檢查點 ID（用於分支）

    # 狀態快照
    values: dict                      # P1 狀態值字典
    metadata: dict                    # P1 元數據（時間戳等）
    created_at: datetime              # P1 創建時間


class BaseCheckpointSaver(ABC):
    """檢查點持久化抽象"""

    @abstractmethod
    async def put(checkpoint: Checkpoint) -> None      # P1 保存

    @abstractmethod
    async def get(
        thread_id: str,
        checkpoint_id: str = None
    ) -> Checkpoint | None                            # P1 讀取

    @abstractmethod
    async def list(thread_id: str) -> list[Checkpoint]  # P1 列出所有
```

## 啟動流程

```
1. 圖構建階段（設計時）
   ├─> 定義 State TypedDict
   │   state = {
   │       "messages": [Message],
   │       "current_node": str,
   │       "user_input": str
   │   }
   ├─> 創建 StateGraph(state)
   ├─> 添加節點：graph.add_node("node_a", func_a)
   ├─> 添加邊：graph.add_edge("node_a", "node_b")
   ├─> 條件邊：graph.add_conditional_edges(
   │       "node_a",
   │       lambda state: "node_b" if condition else "node_c"
   │   )
   └─> 設置入/出點：graph.set_entry_point("node_a")
                   graph.set_finish_point("node_z")

2. 圖編譯階段（runtime 之前）
   ├─> app = graph.compile(
   │       checkpointer=SqliteSaver(),
   │       interrupt_before=["node_manual"],
   │       interrupt_after=["node_decision"]
   │   )
   ├─> 驗證圖拓撲（檢查懸掛節點）
   ├─> 初始化 Pregel 執行引擎
   └─> 如提供 checkpointer，初始化檢查點系統

3. 執行階段（運行時）
   Loop: while not finished:
   ├─> 當前節點 = entry_point / 條件邊結果
   ├─> 讀取狀態通道（get 所有輸入）
   ├─> 調用節點函數
   │   node_fn(state) → dict | None
   │   - 如返回 None，保持狀態
   │   - 如返回 dict，應用 reducer 更新
   ├─> 檢查檢查點中斷？
   │   ├─> interrupt_after：保存檢查點 → 暫停
   │   └─> 可調用 get_state() / update_state() 恢復
   ├─> 執行下一條邊
   │   ├─> 無條件邊→直接跳轉
   │   └─> 條件邊→執行條件函數獲取目標
   ├─> 如目標是 END，循環終止
   └─> 返回最終狀態

4. 狀態管理（Reducer 系統）
   ├─> 預設 reducer 是 overwrite（新值覆蓋舊值）
   ├─> 自訂 reducer 例子：
   │   graph.add_node("append_msg", append_fn, )
   │   state_reducers["messages"] = lambda old, new: old + new
   ├─> 執行步驟中應用 reducer：
   │   old_state["messages"] += node_output["messages"]
   └─> 修改後的狀態傳遞給下一個節點

5. 檢查點與恢復
   ├─> 執行時保存檢查點：
   │   checkpointer.put(Checkpoint(
   │       thread_id="user_123",
   │       checkpoint_id=uuid,
   │       values=current_state
   │   ))
   ├─> 恢復執行：
   │   app.invoke(
   │       input=None,  # 使用已保存的狀態
   │       config={"thread_id": "user_123", "checkpoint_id": last_id}
   │   )
   └─> 從檢查點繼續（無需重新運行前面的步驟）

6. 人類介入（HITL）流程
   ├─> 在中斷點暫停執行
   ├─> 人類審查狀態：state = app.get_state(config)
   ├─> 人類修改狀態：app.update_state(config, {"user_approved": True})
   └─> 繼續執行：app.invoke(config=config)
```

## 資料流 ASCII 圖

### 簡單線性流

```
Input
  ↓
[Entry Point]
  ↓
[Node A] → {state update via reducer}
  ↓
[Condition?] → Yes: [Node B] → [End]
            → No:  [Node C] → [End]
```

### 並行執行示例

```
Input
  ↓
[Start Node]
  ├──→ [Node A (research)]
  │      ↓
  │    [Collect results]
  │      ↓
  └──→ [Node B (analysis)] ⟵── [merger node]
         ↓
      [End]
```

### 檢查點與恢復

```
Run 1:
  Input → [Node A] → INTERRUPT (checkpoint saved)

Run 2:
  Restore from checkpoint
    ↓
  [Continue from Node B]
    ↓
  [Node C]
    ↓
  Final Output
```

## 子系統清單

### P0（核心必需）

| 子系統 | 模塊 | 責任 |
|--------|------|------|
| **圖構建** | `graph/state.py` | StateGraph 定義與拓撲建立 |
| **編譯引擎** | `pregel/pregel.py` | 圖拓撲驗證與編譯 |
| **執行器** | `pregel/runner.py` | 節點執行與狀態更新迴圈 |
| **狀態通道** | `channels/in_memory.py` | InMemoryChannel 狀態讀寫 |
| **類型系統** | `types.py` | RunnableConfig, Node, Edge |
| **常數** | `constants.py` | START, END, CONFIG_KEYS |

### P1（企業級功能）

| 子系統 | 模塊 | 責任 |
|--------|------|------|
| **檢查點** | `checkpoint/base.py` | 狀態持久化與恢復 |
| **SQLite** | `checkpoint-sqlite/` | 本地 SQLite 檢查點 |
| **PostgreSQL** | `checkpoint-postgres/` | 生產級數據庫檢查點 |
| **中斷機制** | `pregel/runner.py` | interrupt_before/after 實現 |
| **流式輸出** | `pregel/io.py` | stream() 與 astream() 實現 |
| **預建代理** | `prebuilt/chat_agent_executor.py` | create_react_agent() 高階 API |
| **CLI** | `cli/server.py` | 本地開發服務器 |
| **SDK** | `sdk-py/`, `sdk-js/` | REST API 客戶端 |
| **LangSmith 集成** | （外部） | 追蹤與可觀測性 |

### P2（未來擴展）

| 功能 | 說明 |
|------|------|
| **動態子圖** | 運行時動態生成子圖 |
| **分佈式執行** | 跨機器節點執行 |
| **狀態壓縮** | 檢查點大小優化 |
| **自適應併發** | 根據負載調整並發度 |
| **多模態狀態** | 支持視頻、音頻等富媒體 |

