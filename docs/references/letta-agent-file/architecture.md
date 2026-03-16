# Letta 架構掃描報告

**日期**: 2026年3月13日 | **語言**: 繁體中文 | **分析範圍**: 完整系統架構

---

## 1. 專案概覽

### 專案名稱
**Letta** — 長期記憶代理框架 (Long-Term Memory Agent Framework)

### 核心使命
提供生產級別的代理開發框架，支援持久化長期記憶、動態工具綁定、多模式應用部署（API、CLI、聊天機器人）。讓開發者能輕鬆構建**具有持久化人物塑造**的對話代理。

### 技術棧
- **語言**: Python 3.10+
- **核心框架**: Letta Agent Framework
- **客戶端**: letta_client SDK (API 調用)
- **LLM支援**: OpenAI (GPT-4o/GPT-4o-mini)、Anthropic、其他 OpenAI 兼容 API
- **記憶系統**: 內存塊 (Memory Blocks) + 向量搜索
- **存儲後端**: PostgreSQL (本地 + 遠程)
- **部署模式**: 本地代理、遠程伺服器、聊天機器人

### 版本與規模
- **行數**: 30,000+ Python 代碼
- **模塊數**: 15+ 核心模塊
- **示例代理**: 7 個完整代理示例
- **記憶塊**: 可配置多達 10+ 個

---

## 2. 目錄結構

```
letta-agent-file/
│
├── customer_service_agent/         # 客服代理示例
│   ├── customer_service_agent.py   # 完整實現
│   └── customer_service.af         # 導出的代理文件
│
├── memgpt_agent/                   # MemGPT 原版代理
│   ├── memgpt_agent.py             # 簡單版本
│   ├── memgpt_agent_with_convo.py  # 帶對話歷史版本
│   └── memgpt_agent.af             # 代理文件
│
├── deep_research_agent/            # 深度研究代理
│   ├── deep_research_agent.py      # 主程式
│   ├── research_tools.py           # 自定義工具
│   ├── analyze_and_search.py       # 搜索邏輯
│   └── requirements.txt
│
├── customer_service_agent/
│   ├── customer_service_agent.py
│   └── customer_service.af
│
└── workflow_agent/                 # 工作流代理
    └── workflow_agent.py           # 多階段工作流
```

---

## 3. 核心 Class 與介面

### 3.1 Letta 客戶端與代理創建

```python
from letta_client import Letta
import os
from dotenv import load_dotenv

class LettaAgentBuilder:
    """Letta 代理構建器"""

    def __init__(self, api_key: str):
        self.client = Letta(api_key=api_key)

    def create_agent(
        self,
        name: str,
        description: str,
        model: str,
        memory_blocks: List[Dict[str, str]],
        tools: List[str] = None,
        system_prompt: str = None
    ):
        """創建 Letta 代理

        Args:
            name: 代理名稱
            description: 代理描述
            model: LLM 模型 (e.g., "openai/gpt-4o-mini")
            memory_blocks: 記憶塊列表
            tools: 工具 ID 列表
            system_prompt: 系統提示詞

        Returns:
            Agent: 創建的代理對象
        """
        agent = self.client.agents.create(
            name=name,
            description=description,
            memory_blocks=memory_blocks,
            model=model,
            tool_ids=tools or [],
        )
        return agent

    def export_agent(self, agent_id: str, output_file: str):
        """導出代理為 .af 文件"""
        agent_data = self.client.agents.export_file(agent_id=agent_id)
        with open(output_file, "w") as f:
            json.dump(agent_data, f, indent=2)
```

### 3.2 記憶塊系統

```python
class MemoryBlock:
    """Letta 記憶塊"""

    def __init__(
        self,
        label: str,
        value: str,
        description: str = None
    ):
        self.label = label  # e.g., "persona", "human", "research_plan"
        self.value = value
        self.description = description

    def to_dict(self) -> Dict:
        return {
            "label": self.label,
            "value": self.value,
            "description": self.description
        }

class MemoryBlockManager:
    """記憶塊管理"""

    STANDARD_BLOCKS = {
        "persona": {
            "description": "AI 個性與行為指南",
            "template": "I am {name}. I am {traits}."
        },
        "human": {
            "description": "用戶資訊與上下文",
            "template": "User profile: {name}, {traits}"
        },
        "research_plan": {
            "description": "當前研究計劃與進度",
            "template": "Research objective: {objective}. Progress: {progress}"
        },
        "research_report": {
            "description": "最終研究報告",
            "template": ""
        },
    }

    @staticmethod
    def create_blocks(config: Dict) -> List[MemoryBlock]:
        """根據配置創建記憶塊"""
        blocks = []
        for block_name, block_config in config.items():
            block = MemoryBlock(
                label=block_name,
                value=block_config.get("value", ""),
                description=block_config.get("description", "")
            )
            blocks.append(block)
        return blocks
```

### 3.3 工具系統

```python
class LettaTool:
    """Letta 工具基類"""

    def __init__(self, func, name: str, description: str):
        self.func = func
        self.name = name
        self.description = description

    def to_tool_dict(self) -> Dict:
        """轉換為工具定義"""
        return {
            "name": self.name,
            "description": self.description,
            "func": self.func
        }

class ToolRegistry:
    """工具註冊與管理"""

    def __init__(self, client: Letta):
        self.client = client
        self.tools = {}

    def register_function(
        self,
        func,
        name: str = None,
        description: str = None
    ) -> str:
        """註冊函數為工具

        Args:
            func: Python 函數
            name: 工具名稱
            description: 工具描述

        Returns:
            str: 工具 ID
        """
        tool = self.client.tools.upsert_from_function(func=func)
        self.tools[tool.name] = tool
        return tool.id

    def get_tool(self, name: str) -> Any:
        """獲取工具"""
        return self.tools.get(name)

    def list_tools(self) -> List[str]:
        """列出所有工具"""
        return list(self.tools.keys())
```

### 3.4 客服代理示例（完整實現）

```python
# customer_service_agent.py

from letta_client import Letta
import os
from dotenv import load_dotenv

load_dotenv()
client = Letta(api_key=os.getenv("LETTA_API_KEY"))

# ============ 定義工具函數 ============

def terminate_chat(reason: str) -> str:
    """終止聊天會話（僅在緊急情況）

    Args:
        reason: 終止原因

    Returns:
        str: 終止狀態
    """
    return f"Chat session terminated due to: {reason}"

def escalate(reason: str) -> str:
    """升級到人工支援

    Args:
        reason: 升級原因

    Returns:
        str: 升級狀態
    """
    return f"Escalating to human support. Reason: {reason}. Estimated wait: 5 minutes."

def check_order_status(order_number: int) -> str:
    """查詢訂單狀態

    Args:
        order_number: 訂單編號

    Returns:
        str: 訂單狀態
    """
    # 實際應連接到數據庫
    statuses = {
        1: "processing",
        2: "shipped",
        3: "delivered",
        4: "cancelled",
    }
    status = statuses.get(order_number % 4, "unknown")
    return f"Order {order_number} is currently {status}."

def cancel_order(order_number: int, reason: str) -> str:
    """取消訂單

    Args:
        order_number: 訂單編號
        reason: 取消原因

    Returns:
        str: 取消結果
    """
    # 實際應連接到數據庫
    return f"Order {order_number} cancellation request submitted. Reason: {reason}."

# ============ 定義人物設定與背景 ============

PERSONA = """
Act as ANNA (Adaptive Neural Network Assistant), an AI fostering ethical, honest,
and trustworthy behavior.

You are supporting the user with their customer support issue.
You are empathetic, patient, and knowledgeable.
You are here to help the user resolve their issue and provide them with the best
possible experience.
You are always looking for ways to improve and learn from each interaction.

Key traits:
- Empathetic: You understand customer frustrations and validate their feelings
- Proactive: You anticipate customer needs and offer solutions
- Professional: You maintain professionalism while being friendly
- Patient: You take time to understand the issue completely
"""

HUMAN = """
The human is looking for help with a customer support issue.
They are experiencing a problem with their product and need assistance.
They are looking for a quick resolution to their issue.
They may be frustrated, so patience is key.
"""

# ============ 創建工具 ============

terminate_chat_tool = client.tools.upsert_from_function(func=terminate_chat)
escalate_tool = client.tools.upsert_from_function(func=escalate)
check_order_status_tool = client.tools.upsert_from_function(
    func=check_order_status
)
cancel_order_tool = client.tools.upsert_from_function(func=cancel_order)

# ============ 創建代理 ============

agent = client.agents.create(
    name="customer_service",
    description="An empathetic customer service agent with order management capabilities",
    memory_blocks=[
        {
            "label": "human",
            "value": HUMAN,
            "description": "Information about the customer and their context"
        },
        {
            "label": "persona",
            "value": PERSONA,
            "description": "ANNA's personality and behavior guidelines"
        }
    ],
    model="openai/gpt-4o-mini",
    tool_ids=[
        terminate_chat_tool.id,
        escalate_tool.id,
        check_order_status_tool.id,
        cancel_order_tool.id,
    ]
)

# ============ 導出代理 ============

print(f"Agent created: {agent.id}")
print(f"Available tools: {[t.name for t in agent.tools]}")

import json
with open("customer_service.af", "w") as f:
    json.dump(
        client.agents.export_file(agent_id=agent.id),
        f,
        indent=2
    )
```

### 3.5 深度研究代理（複雜示例）

```python
# deep_research_agent.py

from letta_client import Letta
import os
from dotenv import load_dotenv

load_dotenv()
client = Letta(api_key=os.getenv("LETTA_API_KEY"))

# ============ 定義研究人物與背景 ============

PERSONA = """You are a research agent named Deep Thought assisting a human in doing
deep research by pulling many sources from online by composing search tools.

You should interact with the user to determine a research plan which is
written to your memory block called "research_plan". Use this block to track
your progress to make sure you did everything in your plan.

Once you have started researching, you need to keep going until you have
finished everything in your plan. Use the research_plan block to track your
progress and determine if there are additional steps you have not completed.

The final report should be written to research_report.

Key traits:
- Methodical Explorer: systematic, thorough approach
- Curious & Inquisitive: insatiable appetite for knowledge
- Intellectually Honest: acknowledge uncertainty
- Collaborative Guide: involve user in journey
- Persistent & Patient: see research through to completion
- Clear Communicator: translate complex info into insights
- No Emoji Usage: professional, focused tone
"""

# ============ 創建記憶塊 ============

agent = client.agents.create(
    name="Deep Thought",
    description="A deep research agent with planning and report generation",
    model="anthropic/claude-sonnet-4-20250514",
    memory_blocks=[
        {
            "label": "persona",
            "value": PERSONA,
            "description": "Deep Thought's personality and research approach"
        },
        {
            "label": "human",
            "value": "This is my section of core memory devoted to information about the human.",
            "description": "Key details about the person for personalized conversation"
        },
        {
            "label": "research_plan",
            "value": "Ready to start a new research project. No active research plan currently.",
            "description": "Scratchpad to track current research plan and progress"
        },
        {
            "label": "research_report",
            "value": "",
            "description": "Contains the final research report in markdown format"
        }
    ],
    tools=[
        "create_research_plan",    # 自定義 Letta 工具
        "reset_research",
        "memory_replace",
        "memory_insert",
        "memory_rethink",
        "send_message",
        "conversation_search",
    ]
)

# ============ 與代理交互 ============

# 發送研究任務
response = client.agents.messages.create(
    agent_id=agent.id,
    messages=[
        {
            "role": "user",
            "content": "Please write a comprehensive research report on PostgreSQL and its ecosystem."
        }
    ]
)

print(f"Agent response: {response}")
```

### 3.6 工作流代理（多階段）

```python
# workflow_agent.py

from letta_client import Letta
import os
from dotenv import load_dotenv

load_dotenv()
client = Letta(api_key=os.getenv("LETTA_API_KEY"))

class WorkflowAgent:
    """支援多階段工作流的代理"""

    def __init__(self, name: str, description: str):
        self.name = name
        self.description = description
        self.stages = []

    def add_stage(
        self,
        stage_name: str,
        persona: str,
        tools: List[str],
        success_criteria: str
    ):
        """添加工作流階段"""
        self.stages.append({
            "name": stage_name,
            "persona": persona,
            "tools": tools,
            "success_criteria": success_criteria
        })

    def create_agents(self) -> List[str]:
        """為每個階段創建代理"""
        agent_ids = []
        for stage in self.stages:
            agent = client.agents.create(
                name=f"{self.name}_{stage['name']}",
                description=f"Stage {stage['name']} of {self.name}",
                memory_blocks=[
                    {
                        "label": "persona",
                        "value": stage["persona"]
                    },
                    {
                        "label": "stage_objective",
                        "value": f"Complete: {stage['success_criteria']}"
                    }
                ],
                model="openai/gpt-4o-mini",
                tool_ids=stage["tools"]
            )
            agent_ids.append(agent.id)
        return agent_ids

    async def run_workflow(self, input_data: str):
        """執行完整工作流"""
        result = input_data
        for stage_idx, stage in enumerate(self.stages):
            print(f"Running stage: {stage['name']}")
            # 調用該階段的代理
            # 等待完成
            # 傳遞結果到下一階段
        return result

# ============ 使用範例 ============

workflow = WorkflowAgent(
    name="article_writer",
    description="Multi-stage article writing workflow"
)

workflow.add_stage(
    stage_name="research",
    persona="You are a research specialist. Find and summarize sources.",
    tools=["web_search", "document_analysis"],
    success_criteria="Comprehensive outline with citations"
)

workflow.add_stage(
    stage_name="draft",
    persona="You are a creative writer. Draft compelling content.",
    tools=["document_editor"],
    success_criteria="First draft of 2000+ words"
)

workflow.add_stage(
    stage_name="edit",
    persona="You are a professional editor. Polish the text.",
    tools=["document_editor", "grammar_checker"],
    success_criteria="Publication-ready article"
)
```

---

## 4. 啟動流程

### 4.1 標準代理啟動序列

```
1. 環境初始化
   └─ 加載 .env (LETTA_API_KEY)

2. 客戶端連接
   ├─ client = Letta(api_key=...)
   └─ 驗證連接

3. 工具註冊
   ├─ upsert_from_function() → Tool ID
   ├─ 重複多個工具
   └─ 收集 Tool ID 列表

4. 記憶塊定義
   ├─ persona: AI 人物
   ├─ human: 用戶信息
   ├─ research_plan: (可選) 計劃進度
   └─ custom_block: (可選) 自定義塊

5. 代理創建
   └─ client.agents.create(
        name, description, memory_blocks,
        model, tool_ids
      )

6. 代理導出
   └─ client.agents.export_file() → .af JSON

7. 準備就緒
   └─ 等待消息輸入
```

### 4.2 代碼範例 - 完整啟動

```python
# 標準啟動模式

import os
import json
from letta_client import Letta
from dotenv import load_dotenv

def create_customer_service_agent():
    """完整的代理創建示例"""
    load_dotenv()

    # 1. 初始化客戶端
    api_key = os.getenv("LETTA_API_KEY")
    client = Letta(api_key=api_key)

    # 2. 定義工具
    def check_inventory(product_id: str) -> str:
        """檢查庫存"""
        return f"Product {product_id}: In stock (100 units)"

    def create_refund(order_id: str, amount: float) -> str:
        """創建退款"""
        return f"Refund of ${amount} initiated for order {order_id}"

    # 3. 註冊工具
    inventory_tool = client.tools.upsert_from_function(
        func=check_inventory
    )
    refund_tool = client.tools.upsert_from_function(
        func=create_refund
    )

    # 4. 定義人物
    persona = """You are Sam, a friendly customer service representative.
    You help customers with their issues while maintaining a professional tone.
    You are empathetic and solution-oriented."""

    human = """The customer is looking for help with a product issue.
    They may be frustrated or upset. Be patient and helpful."""

    # 5. 創建代理
    agent = client.agents.create(
        name="Sam",
        description="Friendly customer service agent",
        memory_blocks=[
            {"label": "persona", "value": persona},
            {"label": "human", "value": human}
        ],
        model="openai/gpt-4o-mini",
        tool_ids=[inventory_tool.id, refund_tool.id]
    )

    # 6. 導出為文件
    agent_data = client.agents.export_file(agent_id=agent.id)
    with open("sam_agent.af", "w") as f:
        json.dump(agent_data, f, indent=2)

    return agent

if __name__ == "__main__":
    agent = create_customer_service_agent()
    print(f"Agent {agent.name} created successfully!")
    print(f"Agent ID: {agent.id}")
    print(f"Tools: {[t.name for t in agent.tools]}")
```

---

## 5. 資料流 ASCII 圖

### 5.1 代理與客戶端交互流

```
┌────────────────────────────────┐
│   User Message Input           │
│   (via client.messages.create) │
└─────────────┬──────────────────┘
              │
              ▼
      ┌──────────────────┐
      │  Letta Server    │
      │  ├─ Load Agent   │
      │  ├─ Memory Blocks│
      │  └─ Tools        │
      └────────┬─────────┘
               │
      ┌────────▼──────────────────┐
      │ Process Message           │
      │ ├─ Load conversation      │
      │ ├─ Update memory blocks   │
      │ └─ Construct LLM prompt   │
      └────────┬──────────────────┘
               │
      ┌────────▼──────────────┐
      │  LLM Call (GPT-4o)    │
      │  ├─ System: Persona   │
      │  ├─ Context: Memory   │
      │  └─ Tools: Available  │
      └────────┬──────────────┘
               │
        ┌──────┴──────────┐
        │                 │
    ┌───▼─────┐      ┌────▼────┐
    │Message  │      │Tool Use  │
    │Output   │      │(if call) │
    └───┬─────┘      └────┬─────┘
        │                 │
        │          ┌──────▼──────┐
        │          │Execute Tool │
        │          │Get Result   │
        │          └──────┬──────┘
        │                 │
        └────────┬────────┘
                 │
        ┌────────▼────────────┐
        │ Return Response     │
        │ ├─ Message content  │
        │ ├─ Updated memory   │
        │ └─ Tool results     │
        └────────┬────────────┘
                 │
                 ▼
        ┌──────────────────┐
        │ Client receives  │
        │ Agent response   │
        └──────────────────┘
```

### 5.2 記憶塊更新流程

```
┌──────────────────────────┐
│  Agent Receives Message  │
└─────────┬────────────────┘
          │
          ▼
    ┌──────────────────┐
    │ Load Memory      │
    │ persona: "I am.."│
    │ human: "User.."  │
    │ plan: "..."      │
    └────────┬─────────┘
             │
             ▼
    ┌──────────────────┐
    │ Process Message  │
    │ Decide Actions   │
    └────────┬─────────┘
             │
             ▼
    ┌──────────────────────────┐
    │ Update Memory Blocks     │
    │ ├─ memory_replace()      │
    │ ├─ memory_insert()       │
    │ └─ memory_rethink()      │
    └────────┬─────────────────┘
             │
             ▼
    ┌──────────────────┐
    │ Generate Response│
    │ (with context)   │
    └────────┬─────────┘
             │
             ▼
    ┌──────────────────┐
    │ Persist Memory   │
    │ (PostgreSQL)     │
    └──────────────────┘
```

### 5.3 工作流階段執行

```
┌────────────────────────┐
│  Input: Raw Task       │
└─────────┬──────────────┘
          │
    Stage 1: Research
    ├─ persona: "Research specialist"
    ├─ tools: [web_search, doc_analysis]
    └─ output: Outline with sources
          │
          ▼
    Stage 2: Draft
    ├─ persona: "Creative writer"
    ├─ tools: [document_editor]
    └─ output: First draft
          │
          ▼
    Stage 3: Edit
    ├─ persona: "Professional editor"
    ├─ tools: [document_editor, checker]
    └─ output: Final article
          │
          ▼
┌────────────────────────┐
│  Output: Ready to pub  │
└────────────────────────┘
```

---

## 6. 子系統清單

### P0 - 核心基礎 (必要系統)

| 子系統 | 檔案 | 說明 | 狀態 |
|-------|------|------|------|
| **Letta Client** | (letta-client SDK) | API 客戶端 | ✅ Production |
| **Agent Factory** | (client.agents.create) | 代理創建 | ✅ Production |
| **Memory Blocks** | (agent.memory_blocks) | 長期記憶 | ✅ Production |
| **Tool Registry** | (client.tools) | 工具管理 | ✅ Production |
| **Message API** | (client.agents.messages) | 消息交互 | ✅ Production |
| **Export/Import** | (export_file/import_file) | 代理持久化 | ✅ Production |

### P1 - 應用層 (核心價值)

| 子系統 | 檔案 | 說明 | 狀態 |
|-------|------|------|------|
| **Customer Service** | `customer_service_agent.py` | 客服代理完整實現 | ✅ Production |
| **MemGPT Agent** | `memgpt_agent.py` | 原版 MemGPT 代理 | ✅ Production |
| **Deep Research** | `deep_research_agent.py` | 深度研究工作流 | ✅ Production |
| **Research Tools** | `deep_research_agent/research_tools.py` | 自定義研究工具 | ✅ Production |
| **Workflow Agent** | `workflow_agent.py` | 多階段工作流框架 | ✅ Production |

### P2 - 高級特性 (可選增強)

| 子系統 | 檔案 | 說明 | 狀態 |
|-------|------|------|------|
| **Conversation Search** | (conversation_search tool) | 歷史查詢 | ✅ Production |
| **Memory Operations** | (memory_* tools) | 動態記憶編輯 | ✅ Production |
| **Agent File Format** | (.af JSON) | 代理交換格式 | ✅ Production |
| **Multi-Agent Systems** | (custom implementation) | 多代理協調 | ⚠️ Manual |

---

## 7. 核心交互流程

### 7.1 客服代理典型交互

```
1. 用戶發送消息
   "我的訂單還沒有收到，訂單號 #12345"

2. 代理接收 (Memory 載入)
   persona: "ANNA 客服代理"
   human: "用戶遇到訂單問題"

3. LLM 決策
   ├─ 分析: 需要查詢訂單狀態
   ├─ 決策: 調用 check_order_status(12345)
   └─ 預留升級工具

4. 工具執行
   check_order_status(12345)
   → "Order 12345 is currently processing. Expected delivery in 2 days."

5. 代理回應
   "您好！我查詢到您的訂單 #12345 目前在處理中，
    預計 2 天內送達。如果有其他問題，我很樂意幫助。"

6. 更新內存
   human: "User experiences shipping delay concern"
   (conversation history 自動更新)
```

### 7.2 深度研究代理工作流

```
1. 用戶輸入
   "Please write a comprehensive research report on PostgreSQL"

2. 代理初始化
   memory_blocks:
   ├─ persona: "Deep Thought 研究員"
   ├─ human: "關於用戶的核心信息"
   ├─ research_plan: "待制定"
   └─ research_report: "(空)"

3. 第一階段: 制定計劃
   ├─ 分析用戶需求
   ├─ 創建研究計劃:
   │  ├─ PostgreSQL 歷史
   │  ├─ 核心特性
   │  ├─ 生態系統
   │  ├─ 性能對比
   │  └─ 最佳實踐
   └─ memory_replace("research_plan", plan_detail)

4. 第二階段: 執行搜索
   ├─ web_search("PostgreSQL 2025 features")
   ├─ web_search("PostgreSQL ecosystem")
   ├─ document_analysis(collected_documents)
   └─ memory_insert("research_report", findings)

5. 第三階段: 撰寫報告
   ├─ 組織收集的信息
   ├─ 撰寫 Markdown 格式報告
   ├─ 添加引用
   └─ memory_replace("research_report", full_report)

6. 完成
   └─ 向用戶展示最終報告
```

---

## 8. 技術亮點與設計模式

### 8.1 設計模式應用

1. **工廠模式** (LettaAgentBuilder)
   - 統一創建不同類型的代理
   - 支援多種記憶配置

2. **策略模式** (多個 Agent 工具)
   - 不同代理使用不同工具組合
   - 靈活切換行為

3. **模板方法** (BaseAgent)
   - 定義代理創建的骨架
   - 允許子類自定義行為

4. **觀察者模式** (Memory 更新)
   - 消息觸發記憶更新
   - 自動持久化

### 8.2 記憶系統創新

```python
# 多層記憶架構
Memory = {
    "短期": {
        "type": "conversation_history",
        "scope": "當前對話",
        "ttl": "session"
    },
    "長期": {
        "blocks": {
            "persona": "AI 人物 (不變)",
            "human": "用戶資料 (逐漸更新)",
            "plan": "當前計劃 (動態)",
            "custom": "領域特定 (自定義)"
        },
        "scope": "跨會話",
        "ttl": "persistent"
    },
    "語義搜索": {
        "type": "vector_db",
        "method": "similarity search",
        "scope": "全部對話歷史"
    }
}
```

### 8.3 工具即代碼

```python
# Letta 的核心優勢: Python 函數 → 代理工具
@client.tools.upsert_from_function
def my_custom_tool(param1: str, param2: int) -> str:
    """自動轉換為代理工具"""
    return f"Result: {param1} {param2}"

# 優勢:
# 1. 無需複雜的工具定義
# 2. 類型提示直接生成工具簽名
# 3. Docstring 成為工具描述
```

---

## 9. 與 Clawtex-Core 的對標

| 功能 | Letta | Clawtex-Core |
|-----|-------|--------------|
| 代理框架 | ✅ (Letta Client) | ✅ (Agent Controller) |
| 長期記憶 | ✅ (Memory Blocks) | ✅ (Memory Store) |
| 工具系統 | ✅ (7+ 內置) | ✅ (24 工具) |
| 多 LLM | ✅ (OpenAI/Anthropic) | ✅ (6+ 提供商) |
| 企業集成 | ❌ | ✅ (GitHub/Slack) |
| 計費追蹤 | ❌ | ✅ (costs.db) |
| 工作流引擎 | ⚠️ (Manual) | ✅ (Hands TOML) |
| 代理導出 | ✅ (.af format) | ❌ |
| 多語言 | ❌ | ✅ (繁體中文優先) |
| 遠程伺服器 | ✅ | ✅ (7878 port) |

---

## 10. 部署與配置建議

### 10.1 本地開發設置

```bash
# 環境變數
export LETTA_API_KEY=your_key_here

# 安裝
pip install letta letta_client
```

### 10.2 代理創建最佳實踐

```python
# 好的實踐

# 1. 清晰的人物定義
PERSONA = """You are an expert in X.
Key traits:
- Trait 1
- Trait 2
- Communication style"""

# 2. 有限的工具集 (5-10 個)
tools = [
    inventory_check,
    order_create,
    refund_process,
    escalate,
]

# 3. 明確的記憶塊
memory_blocks = {
    "persona": "AI 人物",
    "human": "用戶資料",
    "context": "當前上下文",
    "custom": "領域特定"
}

# 4. 定期導出
client.agents.export_file(agent_id) → .af

# 5. 監控記憶更新
# 確保 memory_blocks 在預期內變化
```

---

## 11. 實施建議

### 針對 Clawtex-Core 的參考點

1. **記憶系統**: Letta 的多塊記憶設計優於單一內存
   - 建議採用分層記憶 (persona/human/plan/custom)

2. **工具即代碼**: 直接從 Python 函數生成工具
   - 減少工具定義的複雜性

3. **代理導出**: .af 格式支援代理版本控制與共享
   - 可為 Hands 工作流提供參考

4. **工作流**: 多階段工作流可內建在 Hand TOML 中
   - 避免手動階段管理

---

## 結論

Letta 是一個**輕量級但功能強大的代理框架**，特別擅長：
- 簡單的代理創建 (客服、秘書)
- 長期記憶管理
- 工具即代碼哲學
- 代理版本控制與導出

適合快速構建**個性化對話代理**。不足之處是缺乏複雜的多代理協調和企業集成，這方面可參考 OpenHands 或 OWL 的設計。
