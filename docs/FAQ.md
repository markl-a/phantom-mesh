# Phantom Mesh -- Frequently Asked Questions

## 1. What is Phantom Mesh?

Phantom Mesh is a local-first AI knowledge engine that captures, structures, and recalls your professional knowledge. It uses LLMs to extract structured experience nodes from freeform text -- meeting notes, retrospectives, technical decisions -- and builds a searchable knowledge graph on your machine. Over time it tracks your growth and generates deliverables like PRDs, memos, and sprint plans grounded in your accumulated experience.

## 2. Why local-first?

Running inference locally means your professional knowledge never leaves your machine. There is no cloud dependency, no subscription, and no risk of a third party training on your proprietary notes. You also get predictable latency and zero per-token costs once the hardware is in place.

## 3. What LLMs are supported?

Phantom Mesh uses Ollama as its default inference backend, so any model Ollama supports will work. The recommended default is `gemma3:12b` for general tasks and `nomic-embed-text` for embeddings. You can override the model on any CLI command with the `--model` flag or set a default in `~/.phantom-mesh/config.json`.

## 4. What are the hardware requirements?

You need a machine with at least 16 GB of RAM to run a 7B-parameter model comfortably through Ollama. For the recommended 12B model, 16-24 GB of RAM and a discrete GPU with 8+ GB VRAM will give the best experience. Storage requirements are modest -- the SQLite + ChromaDB knowledge base stays small unless you ingest thousands of documents.

## 5. Can I use it without a GPU?

Yes. Ollama supports CPU-only inference, so PPI will run on any machine that meets the RAM requirements. Responses will be slower without a GPU, but the system is fully functional. Devices with NPUs (like Qualcomm Snapdragon X) can also accelerate inference via the `local` agent.

## 6. How do I set up multiple devices?

Install Ollama on each machine, then connect them over your LAN or a Tailscale mesh VPN. Set the `CLAWTEX_CLUSTER_NODES` environment variable or edit `~/.phantom-mesh/config.json` to list each node's address. Phantom Mesh's agent dispatcher will route tasks to the optimal node automatically. See the guides in the `docs/` folder for device-specific setup instructions.

## 7. Is my data private?

Yes. In the default local-only configuration all data stays on your machine (or your private cluster). The knowledge graph is stored in a local SQLite database and ChromaDB vector store. No telemetry is sent and no external API is called unless you explicitly configure a cloud LLM provider.

## 8. How does it compare to cloud alternatives like Notion AI?

Unlike Notion AI or similar SaaS tools, PPI keeps all data on your hardware -- there is no vendor lock-in and no recurring fee. You own the models and the data. The trade-off is that you need to manage your own infrastructure (Ollama, hardware), but Phantom Mesh's CLI and multi-device cluster make that straightforward.

## 9. Can I use it for my company?

Yes. Phantom Mesh is licensed under Apache 2.0, which permits commercial use. Corporate mode provides six built-in templates for generating professional deliverables. For team-wide deployment, each user can run their own instance or share a cluster over a private network.

## 10. What is corporate mode?

Corporate mode is a set of built-in templates that produce polished professional deliverables from your knowledge base. You can generate PRDs, memos, sprint plans, and more with the `phantom-mesh compile --template <name> --query "..."` command. Each template draws on your stored experience nodes to ground the output in real context.

## 11. How do I contribute?

Fork the repository, create a feature branch, and ensure all 154 tests pass by running `python scripts/test_all.py`. Keep commits focused on one logical change and open a pull request with a clear description. Bug reports and feature requests can be filed as GitHub issues.

## 12. What is on the roadmap?

Planned work includes expanding the multi-device cluster to additional hardware, adding a Gradio web UI, deeper Telegram bot integration for mobile access, improved growth analytics, and support for more embedding models. Check the GitHub issues and project board for the latest priorities.

## 13. Does it work on Windows, Mac, and Linux?

Yes. Phantom Mesh runs anywhere Python 3.11+ and Ollama are available, which covers Windows, macOS (Intel and Apple Silicon), and Linux. The multi-device cluster can mix operating systems freely -- for example, a Windows GPU workstation alongside a Mac M1 and a Linux server.

## 14. Can I use cloud LLMs instead of Ollama?

Yes. Phantom Mesh's architecture supports cloud LLM providers as a fallback or primary backend. The `scout` agent is specifically designed for cloud API access. You can configure providers like Groq, OpenRouter, Gemini, and others through OpenClaw or by setting the appropriate environment variables. Note that using cloud providers means your queries will leave your machine.

## 15. How do I export my data?

The knowledge graph lives in a standard SQLite database (default location `~/.phantom-mesh/ppi.db`) and an optional ChromaDB vector store. You can query the SQLite file directly with any SQLite client, back it up by copying the file, or use `phantom-mesh search` to extract specific nodes. There is no proprietary format -- your data is always portable.

## 中文對照

這份 FAQ 的意思如下：

- Phantom Mesh 是一套 local-first 的 AI 知識引擎，會把你的筆記、會議紀錄、技術決策等自由文字整理成可搜尋的知識圖譜，並據此生成 PRD、備忘錄、衝刺計畫等交付物。
- 它強調 local-first，是因為資料預設不離開你的機器；沒有雲端依賴、沒有訂閱費，也能避免第三方拿你的內容去訓練。
- 預設推理後端是 Ollama，所以 Ollama 能跑的模型理論上都能接。文件建議一般任務用 `gemma3:12b`，embedding 用 `nomic-embed-text`。
- 硬體方面，7B 模型至少要 16GB RAM 才比較舒服；建議的 12B 模型則最好有 16 到 24GB RAM，加上一張 8GB 以上 VRAM 的 GPU。
- 沒有 GPU 也能用，只是速度會慢。若裝置有 NPU，也可以透過 `local` agent 做在地推理。
- 多裝置架設方式是：每台機器都裝 Ollama，再用 LAN 或 Tailscale 串起來，然後把節點位址填進 `CLAWTEX_CLUSTER_NODES` 或 `config.json`。
- 預設情況下資料是私有的，知識圖譜存放在本機 SQLite 與 ChromaDB；只有你主動設定雲端模型時，查詢內容才會離開本機。
- 跟 Notion AI 這類 SaaS 相比，Phantom Mesh 的差異是資料與模型都由你自己掌控，代價是你要自己維護 Ollama 與硬體。
- 公司也可以使用，因為授權是 Apache 2.0，允許商業用途；團隊可以各自跑自己的實例，或透過私有網路共享叢集。
- 所謂 corporate mode，是一組內建模板，能用知識庫內容產出 PRD、memo、sprint plan 等比較正式的文件。
- 貢獻方式是 fork、開 branch、跑完整測試，再送 PR；問題與需求則可提 GitHub issue。
- 路線圖包含更多硬體節點、Gradio Web UI、Telegram 更深整合、成長分析，以及更多 embedding 模型支援。
- 它支援 Windows、macOS（含 Apple Silicon）與 Linux，只要能跑 Python 3.11+ 和 Ollama 即可；叢集也可以混合作業系統。
- 也可以改用雲端 LLM。`scout` agent 就是為雲端 API 設計的，但這代表你的查詢會離開本機。
- 匯出資料很直接，因為核心資料就存在 SQLite 與可選的 ChromaDB 裡，沒有專有格式鎖定問題。
