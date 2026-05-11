# Phantom Mesh Troubleshooting Guide

## 1. Ollama Not Running / Connection Refused
**Symptom:** `ConnectionError: Ollama unavailable at http://localhost:11434`
**Cause:** Ollama service is not running, or bound to a different address/port.
**Solution:**
```bash
ollama serve                                    # start the service
curl http://localhost:11434/api/tags            # verify it responds
export CLAWTEX_OLLAMA_URL=http://192.168.1.50:11434 # if using a custom URL
```

## 2. Model Not Found
**Symptom:** `model 'gemma3:12b' not found` or empty `/api/tags` response.
**Cause:** The model has not been pulled to the local Ollama instance.
**Solution:**
```bash
ollama list                          # see what is installed
ollama pull gemma3:12b               # pull the missing model
export CLAWTEX_DEFAULT_MODEL=llama3.1:8b # or override to an available model
```

## 3. Slow Inference / High Latency
**Symptom:** Responses take 30+ seconds; `latency_ms` over 5000 in dispatch output.
**Cause:** Model running on CPU, oversized for available VRAM, or network latency to remote nodes.
**Solution:**
```bash
curl http://localhost:11434/api/ps              # check loaded models and VRAM
export CLAWTEX_DEFAULT_MODEL=llama3.1:8b            # switch to a smaller model
python -m phantom-mesh.agent_dispatcher --v2 general     # inspect per-node latency
```

## 4. Out of Memory Errors
**Symptom:** Ollama crashes, `CUDA out of memory`, or system freeze during inference.
**Cause:** Model exceeds GPU VRAM, or multiple models loaded at once.
**Solution:**
```bash
curl http://localhost:11434/api/ps   # see loaded models
ollama stop                          # unload all models
ollama pull gemma3:12b-q4_K_M       # use a quantized variant
export CLAWTEX_MAX_PARALLEL_TASKS=1      # limit concurrent requests
```

## 5. Embedding Failures
**Symptom:** `ConnectionError` in `EmbeddingEngine.embed()`, or health check shows 0% embedding coverage.
**Cause:** Ollama is down or the `nomic-embed-text` model is not installed.
**Solution:**
```bash
ollama pull nomic-embed-text
curl -X POST http://localhost:11434/api/embed \
  -d '{"model":"nomic-embed-text","input":"test"}'
python -m phantom-mesh.health                 # check embedding coverage
export CLAWTEX_EMBED_MODEL=nomic-embed-text
```

## 6. Cluster Node Unreachable
**Symptom:** Health report shows `node=DOWN`; `smart_dispatch()` finds no online nodes.
**Cause:** Remote Ollama not running, firewall blocking port 11434, or wrong node URL.
**Solution:**
```bash
curl http://<node-ip>:11434/api/tags                # test direct connectivity
OLLAMA_HOST=0.0.0.0 ollama serve                    # bind to all interfaces on remote
export CLAWTEX_CLUSTER_NODES="m1=http://100.64.0.2:11434,acer=http://100.64.0.3:11434"
```
Nodes can also be set in `~/.phantom-mesh/config.json` under the `cluster_nodes` key.

## 7. Tailscale VPN Connectivity
**Symptom:** Nodes reachable on LAN but not via Tailscale IPs (100.x.x.x).
**Cause:** Tailscale not running, peer not authorized, or firewall blocking UDP/41641.
**Solution:**
```bash
tailscale status                             # check peer list
tailscale up                                 # re-authenticate if expired
tailscale ping <peer-hostname>               # test mesh connectivity
curl http://100.64.0.2:11434/api/tags        # verify Ollama over Tailscale
```

## 8. Database Locked Errors
**Symptom:** `sqlite3.OperationalError: database is locked` during writes.
**Cause:** Multiple Phantom Mesh processes writing to `~/.phantom-mesh/knowledge.db`, or stale lock from a crash.
**Solution:**
```bash
# Find competing processes
tasklist | findstr python          # Windows
ps aux | grep ppi                  # Linux / macOS
# Kill stale processes, then retry
# Integrity check
python -c "import sqlite3; c=sqlite3.connect('$HOME/.phantom-mesh/knowledge.db'); print(c.execute('PRAGMA integrity_check').fetchone())"
# Back up if corrupted
cp ~/.phantom-mesh/knowledge.db ~/.phantom-mesh/knowledge.db.bak
```

## 9. Import Errors After Install
**Symptom:** `ModuleNotFoundError: No module named 'phantom-mesh'` or `ImportError`.
**Cause:** Package not installed in active environment, or Python < 3.11.
**Solution:**
```bash
python --version                     # must be 3.11+
pip install -e .                     # install in dev mode from project root
python -c "from phantom-mesh.config import config; print(config.to_dict())"
# Activate venv first if applicable:
.venv\Scripts\activate               # Windows
source .venv/bin/activate            # Linux / macOS
```

## 10. ChromaDB Optional Dependency
**Symptom:** Health check shows `chromadb: down -- chromadb not installed`.
**Cause:** ChromaDB is an optional dependency (`pip install -e ".[vector]"`).
**Solution:**
```bash
pip install -e ".[vector]"           # or: pip install "chromadb>=0.5.0"
python -c "import chromadb; print(chromadb.__version__)"
# Sync ChromaDB with existing SQLite knowledge base
python -c "
from phantom-mesh.knowledge_store import KnowledgeStore
from phantom-mesh.chroma_store import ChromaStore
print(ChromaStore().sync_from_sqlite(KnowledgeStore()))
"
```
Phantom Mesh works without ChromaDB -- semantic search falls back to brute-force cosine similarity over the SQLite embeddings table. Install it only when you need faster vector search at scale.

---

## Quick Diagnostics
```bash
python -m phantom-mesh.health          # human-readable status of all components
python -m phantom-mesh.health --json   # machine-readable JSON report
```

## 中文對照

這份排錯指南列出最常見的 10 類問題與處理方式：

- 如果出現 `ConnectionError` 或連不上 `http://localhost:11434`，代表 Ollama 沒啟動，或埠號／位址不對。先把 `ollama serve` 跑起來，再用 `curl` 驗證。
- 如果看到 `model not found`，代表本地 Ollama 還沒把對應模型拉下來，先用 `ollama list` 看現況，再用 `ollama pull` 補齊。
- 如果推理很慢，通常是模型太大、在 CPU 上跑，或遠端節點延遲高。可先檢查已載入模型、VRAM 使用量與各節點延遲，再改用較小模型。
- 如果遇到記憶體不足、`CUDA out of memory` 或整機卡死，通常是模型超過顯卡 VRAM，或同時載入太多模型。做法是卸載模型、改用量化版，並降低並行數。
- 如果 embedding 失敗，多半是 Ollama 沒開或 `nomic-embed-text` 未安裝。先 pull 該模型，再手動呼叫 `/api/embed` 驗證。
- 如果 cluster node 顯示離線，先檢查遠端 Ollama 是否有啟動、是否綁在 `0.0.0.0`、防火牆是否允許 `11434`，以及節點 URL 是否正確。
- 如果 Tailscale 節點互相 ping 不到，通常是 Tailscale 沒連線、授權過期，或被防火牆擋住。用 `tailscale status`、`tailscale up`、`tailscale ping` 逐步確認。
- 如果 SQLite 報 `database is locked`，表示有多個程序同時寫入，或 crash 後留下殘留鎖。先找出競爭程序、停止它們，再做 integrity check 與備份。
- 如果安裝後出現 `ModuleNotFoundError`，通常是虛擬環境沒啟用、套件沒裝進當前 Python，或版本低於 3.11。重新啟用 venv 並用 `pip install -e .` 安裝。
- 如果 ChromaDB 顯示未安裝，代表你少裝了 vector extra。用 `pip install -e ".[vector]"` 即可；不裝也能用，只是語意搜尋會退回 SQLite 上的 brute-force cosine similarity。

最後的 Quick Diagnostics 是一個總入口。`python -m phantom-mesh.health` 會輸出人類可讀的系統狀態，`--json` 則適合交給自動化或其他工具做機器解析。
