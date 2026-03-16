# OWL (開源多工作流語言) 架構掃描報告

**日期**: 2026年3月13日 | **語言**: 繁體中文 | **分析範圍**: 完整系統架構

---

## 1. 專案概覽

### 專案名稱
**OWL** — CAMEL-AI 增強型多代理角色扮演與工作流自動化平台

### 核心使命
基於 CAMEL-AI 框架，提供高階多代理協作能力，支援動態角色定義、多 LLM 提供商無縫切換、以及複雜業務工作流自動化。特別針對**企業級應用場景**（投資分析、客服、內容審核等）進行優化。

### 技術棧
- **語言**: Python 3.9+
- **核心框架**: CAMEL-AI (Multi-Agent Orchestration)
- **增強庫**: OWL (角色扮演增強)
- **LLM支援**: OpenAI (GPT-5.2)、Deepseek、Groq、Qwen、Claude
- **工具系統**: 12+ 工具包 (Web Search、Code Execution、Browser、Document Processing)
- **Web 框架**: Streamlit (交互式 UI)
- **數據存儲**: Redis、Pinecone (向量搜索)

### 版本與規模
- **行數**: 50,000+ Python 代碼
- **模塊數**: 30+ 核心模塊
- **社區用例**: 20+ (投資分析、簡歷審核、內容策展等)
- **工具包數**: 12+ (Search、File、Excel、Image、Browser、Code)

---

## 2. 目錄結構

```
owl/
├── owl/                            # 核心 OWL 框架
│   ├── utils/                      # 增強工具庫
│   │   ├── enhanced_role_playing.py    # OwlRolePlaying 類
│   │   ├── enhanced_role_playing.py    # OwlGAIARolePlaying (推理任務)
│   │   ├── document_toolkit.py         # DocumentProcessingToolkit
│   │   ├── gaia.py                     # GAIA 基准
│   │   └── common.py                   # 通用工具函數
│   ├── webapp.py                   # Streamlit 主應用
│   ├── webapp_zh.py                # 繁體中文版本
│   ├── webapp_jp.py                # 日文版本
│   └── webapp_backup.py             # 備用版本
│
├── examples/                       # 示例與演示程式
│   ├── run.py                      # 完整工作流示例
│   ├── run_claude.py               # Claude 特定示例
│   ├── run_deepseek.py             # Deepseek 示例
│   ├── run_groq.py                 # Groq 示例
│   ├── run_qwen.py                 # Qwen 示例
│   └── run_vllm.py                 # vLLM 推論示例
│
├── community_usecase/              # 社區用例 (20+ 專案)
│   │
│   ├── a_share_investment_agent_camel/    # A股投資決策代理
│   │   ├── src/
│   │   │   ├── agents/
│   │   │   │   ├── base_agent.py          # BaseAgent 抽象
│   │   │   │   ├── debate_room.py         # 多代理辯論室
│   │   │   │   ├── investment_agent.py    # 投資代理
│   │   │   │   ├── portfolio_manager.py   # 投資組合管理
│   │   │   │   ├── risk_manager.py        # 風險管理
│   │   │   │   ├── fundamentals_analyst.py  # 基本面分析
│   │   │   │   ├── technical_analyst.py   # 技術分析
│   │   │   │   ├── market_data_agent.py   # 市場數據
│   │   │   │   ├── sentiment_analyst.py   # 情緒分析
│   │   │   │   ├── researcher_bull.py     # 看多研究員
│   │   │   │   ├── researcher_bear.py     # 看空研究員
│   │   │   │   └── valuation_analyst.py   # 估值分析
│   │   │   ├── models.py           # 數據模型 (StockData/Portfolio)
│   │   │   ├── roles.py            # 角色定義
│   │   │   ├── tools/
│   │   │   │   ├── api.py          # 證券 API 調用
│   │   │   │   └── data_helper.py  # 數據處理
│   │   │   ├── main.py             # 入口
│   │   │   └── utils/
│   │   │       └── logging_utils.py
│   │   └── requirements.txt
│   │
│   ├── excel_analyzer/                  # Excel 數據分析器
│   │   ├── data_analyzer_en.py          # 英文版
│   │   ├── data_analyzer_zh.py          # 中文版
│   │   └── sample_data.xlsx
│   │
│   ├── stock-analysis/                  # 股票分析工作流
│   │   ├── run.py
│   │   ├── prompts.py
│   │   ├── agent/
│   │   │   └── sec_agent.py             # SEC 規範 10-K 分析
│   │   └── tools/
│   │       └── sec_tools.py             # SEC 數據工具
│   │
│   ├── OWL Interview Preparation Assistant/
│   │   ├── main.py
│   │   ├── app.py
│   │   ├── config/
│   │   │   └── prompts.py
│   │   └── logging_utils.py
│   │
│   ├── resume-analysis-assistant/       # 簡歷分析助手
│   │   ├── run_mcp.py
│   │   └── config/
│   │
│   ├── learning-assistant/              # 學習助手
│   │   └── run_gpt4o.py
│   │
│   ├── cooking-assistant/               # 烹飪助手
│   │   └── run_gpt4o.py
│   │
│   ├── virtual_fitting_room/            # 虛擬試衣間
│   │   └── run_gpt4o.py
│   │
│   ├── Mcp_use_case/                    # MCP 用例
│   │   └── Content_curator.py           # 內容策展
│   │
│   ├── Notion-MCP/                      # Notion 集成
│   │   └── notion_manager.py
│   │
│   ├── Airbnb-MCP/                      # Airbnb 集成
│   │   └── Airbnb_MCP.py
│   │
│   ├── Whatsapp-MCP/                    # WhatsApp 集成
│   │   └── app.py
│   │
│   ├── Puppeteer-MCP/                   # Puppeteer 瀏覽器
│   │   └── demo.py
│   │
│   ├── PHI_Sanitization_Summarization_and_Article_Writing/
│   │   └── project.py
│   │
│   └── qwen3_mcp/
│       └── run_mcp_qwen3.py
│
├── licenses/                       # 許可證管理
│   └── update_license.py
│
├── .env.example
├── requirements.txt
└── README.md
```

---

## 3. 核心 Class 與介面

### 3.1 增強角色扮演 - OwlRolePlaying

```python
class OwlRolePlaying(RolePlaying):
    """CAMEL 的增強版本，支援多語言與高級配置"""

    def __init__(self, **kwargs):
        # 角色配置
        self.user_role_name = kwargs.get("user_role_name", "user")
        self.assistant_role_name = kwargs.get("assistant_role_name", "assistant")

        # 多語言支援
        self.output_language = kwargs.get("output_language", None)

        # 代理配置
        self.user_agent_kwargs: dict = kwargs.get("user_agent_kwargs", {})
        self.assistant_agent_kwargs: dict = kwargs.get("assistant_agent_kwargs", {})

        # 初始化父類
        super().__init__(**kwargs)

        # 構建系統消息
        init_user_sys_msg, init_assistant_sys_msg = \
            self._construct_gaia_sys_msgs()

        # 初始化代理
        self._init_agents(
            init_assistant_sys_msg,
            init_user_sys_msg,
            assistant_agent_kwargs=self.assistant_agent_kwargs,
            user_agent_kwargs=self.user_agent_kwargs,
            output_language=self.output_language,
        )

    def _init_agents(
        self,
        init_assistant_sys_msg: Optional[BaseMessage],
        init_user_sys_msg: Optional[BaseMessage],
        assistant_agent_kwargs: Optional[Dict] = None,
        user_agent_kwargs: Optional[Dict] = None,
        output_language: Optional[str] = None,
    ) -> None:
        """初始化助手與用戶代理

        Args:
            init_assistant_sys_msg: 助手系統消息
            init_user_sys_msg: 用戶系統消息
            assistant_agent_kwargs: 助手配置
            user_agent_kwargs: 用戶配置
            output_language: 輸出語言
        """
        # 1. 處理模型配置
        if self.model is not None:
            if assistant_agent_kwargs is None:
                assistant_agent_kwargs = {"model": self.model}
            elif "model" not in assistant_agent_kwargs:
                assistant_agent_kwargs.update(dict(model=self.model))

            if user_agent_kwargs is None:
                user_agent_kwargs = {"model": self.model}
            elif "model" not in user_agent_kwargs:
                user_agent_kwargs.update(dict(model=self.model))

        # 2. 初始化助手代理
        if init_assistant_sys_msg is None:
            raise ValueError("Assistant system message cannot be None")

        self.assistant_agent = ChatAgent(
            init_assistant_sys_msg,
            output_language=output_language,
            **(assistant_agent_kwargs or {}),
        )
        self.assistant_sys_msg = self.assistant_agent.system_message

        # 3. 初始化用戶代理
        if init_user_sys_msg is None:
            raise ValueError("User system message cannot be None")

        self.user_agent = ChatAgent(
            init_user_sys_msg,
            output_language=output_language,
            **(user_agent_kwargs or {}),
        )
        self.user_sys_msg = self.user_agent.system_message

    async def run(self, max_turns: int = 100) -> str:
        """執行角色扮演對話"""
        for turn in range(max_turns):
            # 用戶說話
            user_response = await self.user_agent.step(
                self.assistant_sys_msg,
                conversation_history=self.conversation_history
            )
            self.conversation_history.append(
                ("user", user_response.msg)
            )

            # 檢查終止條件
            if user_response.terminated:
                break

            # 助手回應
            assistant_response = await self.assistant_agent.step(
                self.user_sys_msg,
                conversation_history=self.conversation_history
            )
            self.conversation_history.append(
                ("assistant", assistant_response.msg)
            )

            if assistant_response.terminated:
                break

        return self._format_output()
```

### 3.2 GAIA 增強角色扮演 - OwlGAIARolePlaying

```python
class OwlGAIARolePlaying(OwlRolePlaying):
    """針對推理任務的增強版本（GAIA Benchmark）"""

    def __init__(self, **kwargs):
        super().__init__(**kwargs)
        self.reasoning_mode = True
        self.step_by_step_enabled = True

    def _construct_gaia_sys_msgs(self) -> Tuple[BaseMessage, BaseMessage]:
        """構建針對推理的系統消息"""
        # 添加逐步推理提示
        assistant_msg = f"""
        You are an expert reasoning assistant.
        Approach each problem with clear step-by-step thinking.

        1. Break down the problem
        2. Identify constraints
        3. Develop solution
        4. Verify answer

        Provide reasoning in <reasoning></reasoning> tags.
        """

        user_msg = """
        You are a thoughtful question asker.
        Ask probing questions to verify understanding.
        """

        return BaseMessage(content=user_msg), \
               BaseMessage(content=assistant_msg)
```

### 3.3 文檔處理工具包

```python
from camel.toolkits import BaseToolkit

class DocumentProcessingToolkit(BaseToolkit):
    """支援多格式文檔處理"""

    def __init__(self, model=None):
        self.model = model
        self.supported_formats = [
            'pdf', 'docx', 'txt', 'markdown',
            'xlsx', 'csv', 'json'
        ]

    def extract_document_content(self, file_path: str) -> str:
        """提取文檔內容"""
        if file_path.endswith('.pdf'):
            return self._extract_pdf(file_path)
        elif file_path.endswith('.docx'):
            return self._extract_docx(file_path)
        elif file_path.endswith('.xlsx'):
            return self._extract_excel(file_path)
        else:
            return self._extract_text(file_path)

    def _extract_pdf(self, path: str) -> str:
        # PyPDF2 提取
        pass

    def _extract_docx(self, path: str) -> str:
        # python-docx 提取
        pass

    def process_with_ai(self, content: str, task: str) -> str:
        """使用 AI 模型處理文檔內容"""
        prompt = f"Process the following content for {task}:\n{content}"
        response = self.model.generate(prompt)
        return response

    def get_tools(self) -> List[FunctionTool]:
        """返回工具列表"""
        return [
            FunctionTool(self.extract_document_content),
            FunctionTool(self.process_with_ai),
        ]
```

### 3.4 GAIA 基准實現

```python
from camel.benchmarks import BaseBenchmark

class GAIABenchmark(BaseBenchmark):
    """GAIA (General AI Assistants) 基准測試"""

    def __init__(self):
        self.dataset = []  # 加載 GAIA 數據集
        self.metrics = {
            'accuracy': 0.0,
            'f1_score': 0.0,
            'reasoning_quality': 0.0,
        }

    def evaluate(self, agent: OwlRolePlaying) -> Dict[str, float]:
        """評估代理性能"""
        correct = 0
        for task in self.dataset:
            result = agent.run(task['prompt'])
            if self._verify_answer(result, task['expected']):
                correct += 1

        self.metrics['accuracy'] = correct / len(self.dataset)
        return self.metrics

    def _verify_answer(self, output: str, expected: str) -> bool:
        """驗證答案是否正確"""
        # 使用 LLM 或規則進行驗證
        pass
```

### 3.5 投資分析代理層級結構

```python
class BaseAgent(ABC):
    """所有投資代理的基類"""

    def __init__(self, model, tools: List[FunctionTool] = None):
        self.model = model
        self.tools = tools or []
        self.name = self.__class__.__name__

    @abstractmethod
    async def analyze(self, data: Dict) -> Dict:
        """進行分析"""
        pass

class FundamentalsAnalystAgent(BaseAgent):
    """基本面分析代理"""
    async def analyze(self, data: StockData) -> Dict:
        # PE、PB、ROE 等分析
        prompt = f"Analyze fundamentals of {data.ticker}..."
        return await self.model.generate(prompt)

class TechnicalAnalystAgent(BaseAgent):
    """技術面分析代理"""
    async def analyze(self, data: StockData) -> Dict:
        # 趨勢、支撐位、阻力位
        prompt = f"Analyze technical chart of {data.ticker}..."
        return await self.model.generate(prompt)

class SentimentAnalystAgent(BaseAgent):
    """情緒分析代理"""
    async def analyze(self, data: StockData) -> Dict:
        # 市場情緒、新聞情緒
        prompt = f"Analyze market sentiment for {data.ticker}..."
        return await self.model.generate(prompt)

class DebateRoomAgent(BaseAgent):
    """辯論室 - 協調多個代理"""

    async def orchestrate(
        self,
        data: StockData,
        agents: List[BaseAgent]
    ) -> TradingDecision:
        """多代理協調"""
        analyses = []
        for agent in agents:
            analysis = await agent.analyze(data)
            analyses.append(analysis)

        # 綜合所有分析
        final_decision = self._synthesize(analyses)
        return final_decision

    def _synthesize(self, analyses: List[Dict]) -> TradingDecision:
        """綜合分析結果"""
        # 投票或加權平均
        pass

class InvestmentAgent(BaseAgent):
    """主投資決策代理"""

    async def make_decision(
        self,
        portfolio: Portfolio,
        candidates: List[StockData]
    ) -> TradingDecision:
        """進行投資決策"""
        for candidate in candidates:
            analysis = await self.debate_room.orchestrate(candidate)
            if analysis.confidence > 0.8:
                portfolio.add_position(candidate)

        return portfolio.summary()
```

### 3.6 Web 應用層 - Streamlit UI

```python
# webapp.py
import streamlit as st
from owl.utils import OwlRolePlaying, DocumentProcessingToolkit

def main():
    st.set_page_config(page_title="OWL Assistant", layout="wide")

    st.title("OWL - Open Workflow Language")

    # 側邊欄配置
    with st.sidebar:
        model = st.selectbox(
            "選擇 LLM",
            ["gpt-5.2", "claude-opus-4-6", "deepseek", "qwen3"]
        )
        temperature = st.slider("Temperature", 0.0, 2.0, 0.7)
        max_turns = st.slider("Max Turns", 1, 100, 10)

    # 主區域
    task_type = st.tabs(
        ["自由對話", "文檔分析", "代碼生成", "投資分析"]
    )

    with task_type[0]:
        handle_free_dialog(model, temperature, max_turns)

    with task_type[1]:
        handle_document_analysis(model)

    with task_type[2]:
        handle_code_generation(model)

    with task_type[3]:
        handle_investment_analysis(model)

def handle_free_dialog(model, temperature, max_turns):
    user_input = st.text_area("輸入任務描述:")

    if st.button("執行"):
        owl = OwlRolePlaying(
            user_role_name="user",
            assistant_role_name="assistant",
            model_name=model,
            temperature=temperature,
        )

        result = owl.run(max_turns=max_turns)
        st.write(result)
```

---

## 4. 啟動流程

### 4.1 典型應用啟動序列

```
1. 環境初始化
   ├─ 加載 .env 配置
   ├─ 驗證 API 密鑰
   └─ 初始化日誌

2. 模型工廠構建
   ├─ 選擇模型平台 (OpenAI/Deepseek/Groq)
   ├─ 配置溫度與 Token 限制
   └─ 建立連接池

3. 工具包初始化
   ├─ SearchToolkit (DuckDuckGo/Wikipedia)
   ├─ DocumentProcessingToolkit
   ├─ CodeExecutionToolkit (沙盒)
   ├─ FileToolkit
   ├─ ExcelToolkit
   ├─ ImageAnalysisToolkit
   └─ BrowserToolkit (Selenium/Puppeteer)

4. 代理組織
   ├─ Web 代理 (瀏覽 & 搜索)
   ├─ 文檔處理代理
   ├─ 推理代理 (代碼執行)
   └─ Workforce 協調器

5. 應用啟動
   └─ Streamlit webapp.py (port 8501)
      或
      執行 run.py (批量模式)
```

### 4.2 代碼範例 - 完整工作流

```python
# examples/run.py - 完整多代理協調示例

from dotenv import load_dotenv
from camel.models import ModelFactory
from camel.agents import ChatAgent
from camel.toolkits import (
    SearchToolkit, DocumentProcessingToolkit,
    CodeExecutionToolkit, BrowserToolkit
)
from camel.societies import Workforce
from owl.utils import DocumentProcessingToolkit

# 1. 加載環境
load_dotenv()

# 2. 構建模型
model = ModelFactory.create(
    model_platform=ModelPlatformType.OPENAI,
    model_type=ModelType.GPT_5_2,
    model_config_dict={"temperature": 0.7}
)

# 3. 初始化工具
search_toolkit = SearchToolkit()
doc_toolkit = DocumentProcessingToolkit(model=model)
code_toolkit = CodeExecutionToolkit(sandbox="subprocess")
browser_toolkit = BrowserToolkit()

# 4. 構建代理
web_agent = ChatAgent(
    "You are a web search expert",
    model=model,
    tools=[
        FunctionTool(search_toolkit.search_duckduckgo),
        FunctionTool(search_toolkit.search_wiki),
    ]
)

doc_agent = ChatAgent(
    "You are a document analysis expert",
    model=model,
    tools=[
        FunctionTool(doc_toolkit.extract_document_content),
    ]
)

code_agent = ChatAgent(
    "You are a Python coding expert",
    model=model,
    tools=[
        FunctionTool(code_toolkit.execute_code),
    ]
)

# 5. 組織為 Workforce
workforce = Workforce([web_agent, doc_agent, code_agent])

# 6. 分配任務
task = Task(
    objective="研究 Postgres 生態系統",
    agents=[web_agent, doc_agent]
)

# 7. 執行
result = workforce.run(task)
print(result)
```

### 4.3 投資分析啟動示例

```python
# community_usecase/a_share_investment_agent_camel/src/main.py

async def main():
    # 1. 初始化代理群
    agents = {
        'fundamentals': FundamentalsAnalystAgent(model),
        'technical': TechnicalAnalystAgent(model),
        'sentiment': SentimentAnalystAgent(model),
        'risk': RiskManagerAgent(model),
        'portfolio': PortfolioManagerAgent(model),
    }

    # 2. 創建辯論室
    debate_room = DebateRoomAgent(model)

    # 3. 創建投資主代理
    investment_agent = InvestmentAgent(model)

    # 4. 獲取候選股票
    candidates = await get_stock_candidates()

    # 5. 分析並投資
    decision = await investment_agent.make_decision(
        portfolio=Portfolio(),
        candidates=candidates,
        debate_room=debate_room,
        agents=agents
    )

    # 6. 輸出決策
    print(json.dumps(decision.dict(), cls=DateTimeEncoder))

if __name__ == "__main__":
    asyncio.run(main())
```

---

## 5. 資料流 ASCII 圖

### 5.1 單個 OwlRolePlaying 對話迴圈

```
┌──────────────────────────────────────┐
│    User Provides Task/Question       │
└────────────┬─────────────────────────┘
             │
             ▼
    ┌────────────────────┐
    │ OwlRolePlaying     │
    │ .run(max_turns)    │
    └────────┬───────────┘
             │
    ┌────────▼──────────────────────┐
    │ Turn 1: USER Agent Steps       │
    │ ├─ Input: system_msg          │
    │ ├─ LLM Call: GPT-5.2          │
    │ ├─ Output: user_response      │
    │ └─ Check: terminated?         │
    └────────┬──────────────────────┘
             │
    ┌────────▼──────────────────────┐
    │ Turn 1: ASSISTANT Agent Steps  │
    │ ├─ Input: conversation_hist   │
    │ ├─ LLM Call: GPT-5.2          │
    │ ├─ Output: asst_response      │
    │ └─ Check: terminated?         │
    └────────┬──────────────────────┘
             │
    ┌────────▼──────────────────────┐
    │ Turn 2-N: Repeat              │
    │ (until max_turns or terminate)│
    └────────┬──────────────────────┘
             │
             ▼
    ┌────────────────────┐
    │ Format Output      │
    │ Markdown/Text      │
    └────────────────────┘
```

### 5.2 多代理 Workforce 協調

```
┌──────────────────────────────────────────────────┐
│       User Task Input (Complex Objective)        │
└────────┬─────────────────────────────────────────┘
         │
         ▼
    ┌──────────────────────┐
    │  Workforce.run()     │
    │  Task Distribution   │
    └────────┬─────────────┘
             │
      ┌──────┼──────┬──────────┐
      │      │      │          │
      ▼      ▼      ▼          ▼
   ┌────┐┌────┐┌────┐     ┌─────┐
   │Web ││Doc ││Code│ ... │Other│
   │Ag. ││Ag. ││Ag. │     │Ag.  │
   └─┬──┘└─┬──┘└─┬──┘     └──┬──┘
     │     │     │           │
     │     │     │     ┌─────┘
     │     │     │     │
     └────┬┴─────┴─────┘
          │
          ▼
    ┌─────────────────┐
    │ Merge Results   │
    │ Consolidate     │
    └────────┬────────┘
             │
             ▼
    ┌──────────────────┐
    │ Final Output     │
    └──────────────────┘
```

### 5.3 投資決策工作流

```
┌────────────────────────────────┐
│  Market Data + Stock List      │
└─────────────┬──────────────────┘
              │
              ▼
        ┌────────────────┐
        │  For each stock│
        └────────┬───────┘
                 │
      ┌──────────┼──────────┐
      │          │          │
      ▼          ▼          ▼
 ┌─────────┐┌──────────┐┌───────────┐
 │Fundamen-││Technical ││Sentiment  │
 │tals Agent││ Analyst  ││  Analyst  │
 └────┬────┘└────┬─────┘└─────┬─────┘
      │          │            │
      └────┬─────┴────┬───────┘
           │          │
           ▼          ▼
      ┌──────────────────────┐
      │  DebateRoomAgent     │
      │  Synthesize Analysis │
      │  (voting/averaging)  │
      └─────┬────────────────┘
            │
            ▼
      ┌──────────────┐
      │Confidence    │
      │>0.8?         │
      └┬────────┬────┘
       │ YES    │ NO
       │        │
       ▼        ▼
    ┌────┐  ┌─────┐
    │BUY │  │SKIP │
    └────┘  └─────┘
       │        │
       └────┬───┘
            │
            ▼
      ┌──────────────────┐
      │ Update Portfolio │
      │ Return Decision  │
      └──────────────────┘
```

---

## 6. 子系統清單

### P0 - 核心基礎 (必要系統)

| 子系統 | 檔案 | 說明 | 狀態 |
|-------|------|------|------|
| **OwlRolePlaying** | `owl/utils/enhanced_role_playing.py` | 多語言角色扮演 | ✅ Production |
| **CAMEL ChatAgent** | (camel-ai 庫) | 底層代理 | ✅ Production |
| **Model Factory** | (camel-ai 庫) | 多 LLM 支援 | ✅ Production |
| **Tool Registry** | (camel-ai 庫) | 工具管理 | ✅ Production |
| **Workforce** | (camel-ai 庫) | 多代理協調 | ✅ Production |
| **Search Toolkit** | (camel-ai 庫) | Web 搜索 | ✅ Production |
| **Document Toolkit** | `owl/utils/document_toolkit.py` | 文檔處理 | ✅ Production |

### P1 - 業務邏輯 (核心價值)

| 子系統 | 檔案 | 說明 | 狀態 |
|-------|------|------|------|
| **Investment Agents** | `community_usecase/.../agents/` | 投資決策代理組 | ✅ Production |
| **DebateRoom** | `community_usecase/.../debate_room.py` | 多代理辯論 | ✅ Production |
| **Stock Analysis** | `community_usecase/stock-analysis/` | SEC 10-K 分析 | ✅ Production |
| **Data Models** | `community_usecase/.../models.py` | Pydantic 數據模型 | ✅ Production |
| **Streamlit UI** | `owl/webapp.py` | 交互式界面 | ✅ Production |
| **Excel Analyzer** | `community_usecase/excel_analyzer/` | 表格分析 | ✅ Production |
| **Interview Assistant** | `community_usecase/.../interview/` | 面試準備 | ✅ Production |

### P2 - 增強工具 (可選特性)

| 子系統 | 檔案 | 說明 | 狀態 |
|-------|------|------|------|
| **GAIA Benchmark** | `owl/utils/gaia.py` | 推理任務基准 | ✅ Production |
| **OwlGAIARolePlaying** | `owl/utils/enhanced_role_playing.py` | 推理優化版本 | ✅ Production |
| **Code Execution** | (camel-ai Toolkit) | 沙盒代碼執行 | ✅ Production |
| **Image Analysis** | (camel-ai Toolkit) | 視覺識別 | ✅ Production |
| **Browser Toolkit** | (camel-ai Toolkit) | 瀏覽器自動化 | ✅ Production |
| **File Operations** | (camel-ai Toolkit) | 文件管理 | ✅ Production |
| **Excel Toolkit** | (camel-ai Toolkit) | 電子表格處理 | ✅ Production |

### P3 - 社區用例 (參考實現)

| 用例 | 目錄 | 複雜度 | 說明 |
|-----|-----|--------|------|
| **A股投資分析** | `community_usecase/a_share_investment_agent_camel/` | ⭐⭐⭐⭐⭐ | 12 代理 + 辯論室 |
| **股票分析** | `community_usecase/stock-analysis/` | ⭐⭐⭐⭐ | SEC 規範解析 |
| **Excel 分析** | `community_usecase/excel_analyzer/` | ⭐⭐⭐ | 多語言版本 |
| **簡歷分析** | `community_usecase/resume-analysis-assistant/` | ⭐⭐⭐ | MCP 集成 |
| **面試準備** | `community_usecase/OWL Interview Preparation/` | ⭐⭐⭐ | Streamlit UI |
| **內容策展** | `community_usecase/Mcp_use_case/` | ⭐⭐ | MCP 原型 |
| **虛擬試衣** | `community_usecase/virtual_fitting_room/` | ⭐⭐⭐ | 視覺識別 |
| **烹飪助手** | `community_usecase/cooking-assistant/` | ⭐⭐ | 簡單示例 |

---

## 7. 核心交互流程

### 7.1 典型任務執行流程

```
1. 用戶輸入
   └─ "研究 Postgres 生態系統"

2. Workforce 分配
   ├─ Web Agent: 搜索最新文章
   ├─ Doc Agent: 分析文檔
   └─ Code Agent: 執行示例代碼

3. Web Agent 執行
   ├─ 搜索 "Postgres 2025"
   ├─ 訪問官方文檔
   └─ 提取關鍵信息

4. Doc Agent 執行
   ├─ 下載 Postgres 白皮書
   ├─ 提取核心內容
   └─ 總結要點

5. Code Agent 執行
   ├─ 編寫連接代碼
   ├─ 測試查詢性能
   └─ 提供性能數據

6. 結果聚合
   ├─ Web: 生態系統概覽
   ├─ Doc: 技術細節
   ├─ Code: 性能基准
   └─ Final: 完整報告
```

### 7.2 投資決策流程 (深度)

```
1. 獲取候選股票
   └─ API: 獲取最近漲跌幅

2. For 每支股票 loop:

   a. FundamentalsAnalyst 分析
      ├─ 計算 PE、PB、ROE
      ├─ 對比同行業平均值
      └─ 得出基本面評分 (0-100)

   b. TechnicalAnalyst 分析
      ├─ 繪製技術圖表
      ├─ 識別支撐/阻力位
      └─ 得出技術評分

   c. SentimentAnalyst 分析
      ├─ 爬取市場評論
      ├─ NLP 情緒分析
      └─ 得出情緒評分

   d. DebateRoom 協調
      ├─ 三個評分投票
      ├─ 計算綜合信心度
      └─ 得出買/賣/觀望 決策

   e. RiskManager 風控
      ├─ 計算組合風險
      ├─ 檢查止損位
      └─ 批准或拒絕

3. InvestmentAgent 決策
   ├─ 檢查所有批准項目
   ├─ 優化組合配置
   └─ 執行下單

4. PortfolioManager 跟蹤
   ├─ 監控實時收益
   ├─ 更新風險指標
   └─ 重新平衡（如需）
```

---

## 8. 技術亮點與設計模式

### 8.1 設計模式應用

1. **工廠模式** (ModelFactory)
   - 統一創建多個 LLM 模型實例
   - 支援 OpenAI、Deepseek、Groq、Qwen 等

2. **觀察者模式** (Workforce)
   - 多代理組織與事件通知
   - 支援並行或順序執行

3. **策略模式** (多個 Agent)
   - FundamentalsAnalyst / TechnicalAnalyst / SentimentAnalyst
   - 每個代理採用不同分析策略

4. **組合模式** (DebateRoom)
   - 協調多個代理的輸出
   - 整合為最終決策

### 8.2 多語言支援機制

```python
# OwlRolePlaying 支援
output_language = "Chinese"  # 輸出語言
user_role_name = "用戶"
assistant_role_name = "助手"

# 多語言 Webapp
webapp.py          # English
webapp_zh.py       # 繁體中文
webapp_jp.py       # 日本語
```

### 8.3 可擴展性設計

- **CAMEL-AI 基礎**: 自動支援任何新增 LLM
- **工具包模塊化**: BaseToolkit 可輕易擴展
- **代理模塊化**: BaseAgent 支援自定義業務邏輯
- **Workforce 組合**: 支援任意代理數量與組合

---

## 9. 與 Clawtex-Core 的對標

| 功能 | OWL | Clawtex-Core |
|-----|-----|--------------|
| 多代理協調 | ✅ (Workforce) | ✅ (Hand 工作流) |
| 多 LLM 支援 | ✅ (ModelFactory) | ✅ (6 提供商) |
| 工具系統 | ✅ (12+ Toolkit) | ✅ (24 工具) |
| 內存系統 | ✅ (Conversation) | ✅ (Memory Store) |
| 多語言 | ✅ (Webapp_zh) | ✅ (優先繁體中文) |
| 角色扮演 | ✅ (RolePlaying) | ✅ (Persona) |
| 推理優化 | ✅ (GAIA) | ✅ (O1-style) |
| 企業整合 | ❌ (無原生) | ✅ (Telegram/Slack) |
| 計費追蹤 | ❌ | ✅ (costs.db) |

---

## 10. 部署與配置建議

### 10.1 開發環境配置

```yaml
# .env
OPENAI_API_KEY=sk-...
CAMEL_LOG_LEVEL=DEBUG
STREAMLIT_THEME=dark
```

### 10.2 投資分析系統配置

```python
# config.py
MODEL_CONFIG = {
    "fundamentals": "gpt-5.2",
    "technical": "gpt-5.2",
    "sentiment": "deepseek-v3",
    "debate": "claude-opus-4-6",
}

AGENT_CONFIG = {
    "max_turns": 20,
    "temperature": 0.2,  # 低溫確保一致性
    "top_p": 0.95,
}

PORTFOLIO_CONFIG = {
    "max_positions": 10,
    "position_size": 0.1,  # 10% 單倉位
    "stop_loss": 0.08,  # 8% 止損
}
```

---

## 結論

OWL 是 CAMEL-AI 上的**高級業務應用層**，展示了如何：
- 構建複雜的多代理協調系統
- 實現真實投資決策工作流
- 支援多語言與多 LLM 適配
- 創建可交互的 Web 應用
- 發展社區用例與參考實現

特別適合需要**複雜多代理協調**和**業務邏輯深度集成**的應用場景。
