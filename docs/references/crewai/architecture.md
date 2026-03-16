# CrewAI 架構文檔

## 專案概覽

CrewAI 是一個獨立的 Python 多代理編排框架，完全不依賴 LangChain。設計宗旨是提供高效能、低延遲的多代理系統構建平台，適合從簡單任務到複雜企業級場景的各種需求。

**核心特性**：
- 輕量級、獨立框架，零 LangChain 依賴
- 雙軌道架構：Crews（自主型代理隊伍）和 Flows（事件驅動工作流）
- 110,000+ 認證開發者社群
- 企業級功能（AMP Suite：可觀測性、安全、自動化）

**適用場景**：
- 多代理協作系統
- 自動化業務流程
- 複雜任務分解與執行
- 實時代理監控與控制

## 目錄結構

```
crewai/
├── lib/
│   ├── crewai-files/              # 文件處理子庫
│   │   ├── src/crewai_files/
│   │   │   ├── cache/             # P0 文件快取層
│   │   │   ├── core/              # P0 類型與解析
│   │   │   ├── formatting/        # P1 多供應商格式化（Anthropic/OpenAI/Gemini/Bedrock）
│   │   │   ├── processing/        # P1 文件約束與驗證
│   │   │   ├── resolution/        # P1 URL 解析器
│   │   │   └── uploaders/         # P1 供應商文件上傳適配器
│   │   └── tests/
│   │
│   └── crewai-tools/              # 工具庫（24+ 工具集成）
│       ├── src/crewai_tools/
│       │   ├── adapters/          # P0 MCP/RAG/Zapier 適配器
│       │   ├── aws/               # P1 AWS Bedrock/S3/Knowledge Base
│       │   ├── rag/               # P1 RAG 管道（loader/chunker/embedding）
│       │   └── [tools]/           # P1 個別工具實現
│       └── tests/
│
├── docs/                          # 官方文檔（多語言）
│   ├── en/concepts/               # P0 核心概念文檔
│   ├── en/api-reference/          # P0 API 參考
│   └── en/enterprise/             # P1 企業功能
│
├── src/crewai/                    # 核心框架（未顯示但推斷結構）
│   ├── crew.py                    # P0 Crew 編排
│   ├── agent.py                   # P0 Agent 實現
│   ├── task.py                    # P0 Task 定義
│   ├── flow.py                    # P0 Flow 引擎（新）
│   ├── llm/                       # P0 LLM 供應商集成
│   ├── memory/                    # P0 短期 + 長期記憶
│   ├── tools/                     # P0 工具系統
│   ├── knowledge/                 # P0 知識庫集成
│   ├── telemetry/                 # P1 可觀測性（遙測）
│   └── processes/                 # P1 執行策略
│
├── tests/                         # 單元與集成測試
├── pyproject.toml                 # 專案配置（Python 3.10+）
└── Makefile                       # 開發命令
```

## 核心 Class/Type

### Crew（代理隊伍）

```python
class Crew:
    """多個 Agent 的協調容器，支持自主決策與任務委派"""

    # 初始化參數
    agents: list[Agent]              # P0 執行者
    tasks: list[Task]                # P0 任務列表
    process: Process                 # P0 執行流程 (sequential/hierarchical/consensus)
    memory: Memory                   # P0 共享記憶體
    manager_llm: LLM | None          # P0 經理 LLM（用於 hierarchical）

    # 主要方法
    def kickoff(inputs: dict) -> str  # P0 啟動執行 → 傳回最終結果
    def kickoff_async(inputs) -> str  # P1 非同步執行
    def kickoff_for_each(inputs) -> list  # P1 批量執行

    # 執行狀態
    id: UUID                         # P0 Crew ID（唯一標識）
    created_at: datetime             # P0 創建時間

    # 內部狀態機
    _execution_state: dict           # P0 當前執行狀態
    _used_agent_ids: set             # P0 已用過的代理 ID
```

### Agent（智能代理）

```python
class Agent:
    """單個決策執行單位，具有工具與記憶"""

    # 核心配置
    role: str                        # P0 職位描述（"數據分析師"）
    goal: str                        # P0 代理目標
    backstory: str                   # P0 背景故事（上下文）
    llm: LLM                         # P0 底層語言模型
    tools: list[Tool]                # P0 可用工具

    # 行為控制
    max_iter: int                    # P0 最多迭代次數（預設 15）
    max_execution_time: int          # P0 最長執行時間（秒）
    allow_code_execution: bool       # P1 允許代碼執行
    allow_delegation: bool           # P1 允許委派任務給其他代理

    # 記憶與上下文
    memory: Memory | None            # P0 個人記憶
    system_template: str             # P0 系統提示模板

    # 主要方法
    def execute_task(task: Task) -> str  # P0 執行單個任務
    def get_tools_from_llm() -> list     # P1 動態工具發現
```

### Task（執行任務）

```python
class Task:
    """Crew 中的單個可執行工作單位"""

    # 任務定義
    description: str                 # P0 任務描述
    expected_output: str             # P0 期望輸出格式
    assigned_to: Agent               # P0 分配的代理

    # 執行控制
    async_execution: bool            # P1 非同步執行
    context: list[Task]              # P0 依賴的上游任務
    output_file: str | None          # P1 輸出文件路徑

    # 評估
    tools: list[Tool] | None         # P0 工具列表（覆蓋代理工具）
    callback: Callable | None        # P1 完成回調

    # 主要方法
    def execute(agent: Agent) -> TaskOutput  # P0 由代理執行
    def increment_attempts() -> None         # P0 增加嘗試次數
```

### Flow（事件驅動工作流）

```python
class Flow:
    """企業級事件驅動編排系統，單次 LLM 調用進行精確控制"""

    # 流程定義
    name: str                        # P0 流程名稱
    description: str                 # P0 流程描述

    # 步驟與連接
    _steps: dict[str, Step]          # P0 步驟字典
    _edges: list[tuple]              # P0 邊連接

    # 主要方法
    def add_step(name: str, fn: Callable) -> Flow  # P0 添加步驟
    def run(input: dict) -> dict                   # P0 執行流程
    def run_async(input: dict) -> dict             # P1 非同步執行

    # 條件控制
    def add_condition(source, target, condition)   # P1 條件分支

    # 狀態管理
    _state: dict                     # P0 流程狀態
    _execution_id: UUID              # P0 執行 ID
```

### LLM（語言模型）

```python
class LLM:
    """統一 LLM 供應商介面"""

    # 配置
    model: str                       # P0 模型名稱
    temperature: float               # P0 創意度（0-1）
    max_tokens: int | None           # P0 最大令牌
    api_key: str | None              # P0 API 密鑰
    base_url: str | None             # P1 自訂基礎 URL

    # 供應商列表
    provider: str                    # P0 "openai" / "anthropic" / "gemini" / "ollama" / ...

    # 主要方法
    def call(messages: list[Message]) -> str  # P0 調用 LLM
    def stream(messages) -> Iterator[str]     # P1 流式回應
```

### Memory（記憶系統）

```python
class Memory:
    """多層記憶體架構"""

    # 短期記憶（當前任務）
    short_term: list[Message]        # P0 對話歷史

    # 長期記憶（跨會話）
    long_term: dict                  # P0 長期知識

    # 向量記憶（語義搜索）
    vector_store: VectorStore        # P1 嵌入向量儲存

    # 主要方法
    def recall(query: str) -> list[str]        # P0 回想相關上下文
    def remember(content: str) -> None         # P0 存儲新信息
    def clear() -> None                        # P0 清除臨時記憶
```

### Tool（工具系統）

```python
class Tool:
    """代理可使用的原子能力單位"""

    # 工具元數據
    name: str                        # P0 工具名稱
    description: str                 # P0 功能描述
    args_schema: dict                # P0 參數JSON Schema

    # 執行邏輯
    func: Callable                   # P0 實現函數

    # 高級功能
    cache_function: bool             # P1 結果快取

    # 主要方法
    def execute(args: dict) -> str               # P0 執行工具
    def get_tool_schema() -> dict                # P0 獲取 Schema
```

### Process（執行策略）

```python
class Process(Enum):
    """Crew 執行策略"""

    SEQUENTIAL = "sequential"         # P0 順序執行
    HIERARCHICAL = "hierarchical"     # P0 層級審查（manager_llm）
    CONSENSUS = "consensus"           # P1 共識投票
```

## 啟動流程

```
1. 用戶代碼初始化
   └─> Crew(agents=[...], tasks=[...], process=...)
   └─> crew.kickoff(inputs={"topic": "..."})

2. Crew 啟動序列
   ├─> 驗證代理與任務綁定
   ├─> 初始化共享記憶體 (short_term + long_term)
   ├─> 根據 process 類型選擇執行引擎
   └─> 啟動任務執行循環

3. 任務執行（Sequential 為例）
   Loop: for task in tasks:
   ├─> task.context 任務已完成？是 → 執行
   ├─> Agent 接收任務
   │   ├─> 構建系統提示 (role + goal + backstory)
   │   ├─> 添加工具列表與可用資源
   │   ├─> 調用 LLM 生成思維鏈
   │   ├─> LLM 輸出工具調用指令？
   │   │   ├─> 是 → 執行工具 → 反饋結果
   │   │   └─> 否 → 返回最終答案
   │   └─> 重複直到達到 max_iter 或生成最終答案
   ├─> 將任務輸出存儲到共享記憶 (context)
   └─> 下一個任務使用前任務的輸出

4. 內部 LLM 調用語法（tool_use）
   ├─> CrewAI 使用 XML 工具調用格式
   ├─> 示例：
   │   <function_calls>
   │   <invoke name="web_search">
   │   <parameter name="query">...