> ⚠️ STALE (pre-pivot) — 本檔用「knowledge-engine / PRD generator」框架,已被現行 4-pillar Life/Work 方向(superpowers/BIG-GOAL.md)取代。

# Spectyn Mesh -- 常見問題

## 1. 什麼是 Spectyn Mesh？

Spectyn Mesh 是一套 local-first（本機優先）的 AI 知識引擎，能捕捉、結構化並回憶你的專業知識。它運用 LLM（大型語言模型）從自由格式文字中萃取出結構化的經驗節點（experience node）——例如會議筆記、回顧檢討、技術決策——並在你的機器上建立可搜尋的知識圖譜（knowledge graph）。隨著時間累積，它會追蹤你的成長，並根據你累積的經驗生成 PRD（產品需求文件）、memo（備忘錄）與 sprint plan（衝刺計畫）等交付物。

## 2. 為什麼採用 local-first（本機優先）？

在本機執行推論（inference）代表你的專業知識永遠不離開你的機器。沒有雲端依賴、沒有訂閱費，也沒有第三方拿你的專有筆記去訓練的風險。一旦硬體就緒，你還能獲得可預期的延遲，以及零的逐 token（詞元）成本。

## 3. 支援哪些 LLM（大型語言模型）？

Spectyn Mesh 預設使用 Ollama 作為推論後端（backend），因此任何 Ollama 支援的模型都能使用。建議的預設值是一般任務用 `gemma3:12b`、embedding（嵌入向量）用 `nomic-embed-text`。你可以在任何 CLI（命令列介面）指令上用 `--model` 旗標覆寫模型，或在 `~/.spectyn-mesh/config.json` 中設定預設值。

## 4. 硬體需求是什麼？

要透過 Ollama 順暢地執行一個 7B（70 億參數）的模型，你的機器至少需要 16 GB 的 RAM（記憶體）。若使用建議的 12B 模型，搭配 16 至 24 GB 的 RAM，以及一張具備 8 GB 以上 VRAM（顯示記憶體）的獨立 GPU（圖形處理器），會有最佳體驗。儲存需求並不高——除非你匯入數千份文件，否則 SQLite 加 ChromaDB 的知識庫都會維持得很小。

## 5. 沒有 GPU 也能用嗎？

可以。Ollama 支援純 CPU（中央處理器）推論，所以只要機器符合 RAM 需求，PPI 就能執行。沒有 GPU 時回應會比較慢，但系統功能完全正常。具備 NPU（神經網路處理器）的裝置（例如 Qualcomm Snapdragon X）也能透過 `local` agent（在地代理）加速推論。

## 6. 如何設定多裝置？

在每台機器上安裝 Ollama，然後透過你的 LAN（區域網路）或 Tailscale mesh VPN（網狀虛擬私人網路）將它們連接起來。設定 `CLAWTEX_CLUSTER_NODES` 環境變數，或編輯 `~/.spectyn-mesh/config.json` 以列出每個節點的位址。Spectyn Mesh 的 agent dispatcher（代理派工器）會自動把任務路由到最佳節點。裝置專屬的設定說明請參閱 `docs/` 資料夾中的指南。

## 7. 我的資料是私密的嗎？

是的。在預設的純本機（local-only）設定下，所有資料都留在你的機器上（或你的私有叢集中）。知識圖譜儲存在本機的 SQLite 資料庫與 ChromaDB 向量儲存中。除非你明確設定雲端 LLM 供應商，否則不會送出任何遙測資料，也不會呼叫任何外部 API。

## 8. 它與 Notion AI 之類的雲端替代方案相比如何？

不同於 Notion AI 或類似的 SaaS（軟體即服務）工具，PPI 把所有資料保留在你的硬體上——沒有供應商鎖定（vendor lock-in），也沒有經常性費用。模型與資料都歸你所有。代價是你需要自行管理基礎設施（Ollama、硬體），但 Spectyn Mesh 的 CLI 與多裝置叢集讓這件事變得很直接。

## 9. 我可以在公司使用它嗎？

可以。Spectyn Mesh 採用 Apache 2.0 授權，允許商業用途。Corporate mode（企業模式）提供六個內建模板，用於生成專業交付物。若要進行全團隊部署，每位使用者都可以執行自己的實例，或透過私有網路共享一個叢集。

## 10. 什麼是 corporate mode（企業模式）？

Corporate mode（企業模式）是一組內建模板，能從你的知識庫產出精緻的專業交付物。你可以用 `spectyn-mesh compile --template <name> --query "..."` 指令生成 PRD（產品需求文件）、memo（備忘錄）、sprint plan（衝刺計畫）等。每個模板都會擷取你儲存的經驗節點，讓輸出立基於真實情境。

## 11. 我要如何貢獻？

Fork（分叉）儲存庫、建立一個 feature branch（功能分支），並透過執行 `python scripts/test_all.py` 確保全部 154 個測試都通過。讓每次 commit（提交）聚焦於一個邏輯變更，並開啟一個附帶清楚描述的 pull request。錯誤回報與功能請求可以用 GitHub issue 提出。

## 12. roadmap（藍圖）上有什麼？

規劃中的工作包括：將多裝置叢集擴展到更多硬體、新增 Gradio web UI（網頁使用者介面）、為行動裝置存取加深 Telegram bot（機器人）整合、改善成長分析，以及支援更多 embedding（嵌入向量）模型。最新的優先順序請查看 GitHub issue 與專案看板。

## 13. 它能在 Windows、Mac 與 Linux 上運作嗎？

可以。只要有 Python 3.11+ 與 Ollama，Spectyn Mesh 就能在任何地方執行，這涵蓋 Windows、macOS（Intel 與 Apple Silicon）以及 Linux。多裝置叢集可以自由混搭作業系統——例如一台 Windows GPU 工作站搭配一台 Mac M1 與一台 Linux 伺服器。

## 14. 我可以用雲端 LLM（大型語言模型）取代 Ollama 嗎？

可以。Spectyn Mesh 的架構支援以雲端 LLM 供應商作為備援（fallback）或主要後端。`scout` agent（偵察代理）就是專為雲端 API 存取設計的。你可以透過遠端控制設定或設定適當的環境變數，來設定 Groq、OpenRouter、Gemini 等供應商。請注意，使用雲端供應商代表你的查詢會離開你的機器。

## 15. 我要如何匯出資料？

知識圖譜存放在一個標準的 SQLite 資料庫（預設位置 `~/.spectyn-mesh/ppi.db`）與一個可選的 ChromaDB 向量儲存中。你可以用任何 SQLite 用戶端直接查詢該 SQLite 檔案、透過複製檔案來備份，或用 `spectyn-mesh search` 來萃取特定節點。沒有任何專有格式——你的資料永遠可攜。

## 中文對照

這份 FAQ 的意思如下：

- Spectyn Mesh 是一套 local-first 的 AI 知識引擎，會把你的筆記、會議紀錄、技術決策等自由文字整理成可搜尋的知識圖譜，並據此生成 PRD、備忘錄、衝刺計畫等交付物。
- 它強調 local-first，是因為資料預設不離開你的機器；沒有雲端依賴、沒有訂閱費，也能避免第三方拿你的內容去訓練。
- 預設推理後端是 Ollama，所以 Ollama 能跑的模型理論上都能接。文件建議一般任務用 `gemma3:12b`，embedding 用 `nomic-embed-text`。
- 硬體方面，7B 模型至少要 16GB RAM 才比較舒服；建議的 12B 模型則最好有 16 到 24GB RAM，加上一張 8GB 以上 VRAM 的 GPU。
- 沒有 GPU 也能用，只是速度會慢。若裝置有 NPU，也可以透過 `local` agent 做在地推理。
- 多裝置架設方式是：每台機器都裝 Ollama，再用 LAN 或 Tailscale 串起來，然後把節點位址填進 `CLAWTEX_CLUSTER_NODES` 或 `config.json`。
- 預設情況下資料是私有的，知識圖譜存放在本機 SQLite 與 ChromaDB；只有你主動設定雲端模型時，查詢內容才會離開本機。
- 跟 Notion AI 這類 SaaS 相比，Spectyn Mesh 的差異是資料與模型都由你自己掌控，代價是你要自己維護 Ollama 與硬體。
- 公司也可以使用，因為授權是 Apache 2.0，允許商業用途；團隊可以各自跑自己的實例，或透過私有網路共享叢集。
- 所謂 corporate mode，是一組內建模板，能用知識庫內容產出 PRD、memo、sprint plan 等比較正式的文件。
- 貢獻方式是 fork、開 branch、跑完整測試，再送 PR；問題與需求則可提 GitHub issue。
- 路線圖包含更多硬體節點、Gradio Web UI、Telegram 更深整合、成長分析，以及更多 embedding 模型支援。
- 它支援 Windows、macOS（含 Apple Silicon）與 Linux，只要能跑 Python 3.11+ 和 Ollama 即可；叢集也可以混合作業系統。
- 也可以改用雲端 LLM。`scout` agent 就是為雲端 API 設計的，但這代表你的查詢會離開本機。
- 匯出資料很直接，因為核心資料就存在 SQLite 與可選的 ChromaDB 裡，沒有專有格式鎖定問題。
