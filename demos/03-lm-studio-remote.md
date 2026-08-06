# Demo 03 — LM Studio Sees Home GPU Over Tailscale

**Length**: 60 seconds
**Scenario source**: doc 28 D6 + doc 30 §4 flagship #3
**Status**: 🟢 大部分今天可錄(MacBook 連回家 GPU 鐵桿可用)

## Hook

> "Your MacBook at a café, your home 4090 sitting idle. With spectyn-mesh, LM Studio sees the 70B model running at home — zero port forwarding."

## Cast / setup
- **MacBook**(咖啡店):LM Studio 視窗,model picker
- **node-a / GPU rig**(家裡):4090 跑 Llama 70B via Ollama / vLLM behind spectyn
- **Speed indicator**:每秒 token 數浮在 LM Studio 上(15+ tok/s)
- **Tailscale 狀態圖**:右下角小視窗

## 60-second script

| Time | What's on screen | Voiceover |
|---|---|---|
| 0:00-0:05 | MacBook 在咖啡店桌上,LM Studio 開著,model picker 空 | "LM Studio. Local models. Nice." |
| 0:05-0:10 | 點 model picker 下拉 — 預設只有 ~7B Mac MLX 跑得動 | "But your MacBook can't run 70B." |
| 0:10-0:18 | 顯示「+ Add remote provider」→ 貼一行 URL `https://node-a.tailnet.ts.net:7878/v1` | "Add spectyn-mesh as a provider." |
| 0:18-0:25 | model picker 重新整理 → 出現「Llama-3-70B-instruct (home/node-a)」 | "Your home 4090 is now a remote." |
| 0:25-0:40 | 選 70B → 開 chat → 輸入問題 → streaming 流出 | "First token in 1.2 seconds. Tailscale handles routing." |
| 0:40-0:50 | 切到 node-a 桌面:`nvidia-smi` 顯示 GPU 拉到 95% util | "Your 4090 doing the work — your battery isn't." |
| 0:50-0:57 | Tailscale 視窗:「encrypted, peer-to-peer, no Cloud」icon | "End-to-end encrypted. No public port. No proxy." |
| 0:57-0:60 | End card:spectyn-mesh + "your hardware, anywhere" | — |

## Pre-record checklist
- [ ] node-a 跑 `spectyn serve --port 7878` + 後面接 Ollama 或 vLLM 跑 70B
- [ ] MacBook 在不同網路(用手機分享網路也行,模擬「咖啡店」)
- [ ] Tailscale 兩台都連上同 tailnet
- [ ] LM Studio remote provider 設定預先測過,確保 model picker 出得來
- [ ] 切到 node-a 的 cut 用 OBS 預存好(避免直播切換失敗)

## Post-record notes
- 主對手:**ChatGPT Mac app**(只能雲)+ **LM Studio 純本機**(被 RAM 限制)
- 強調:**no port forwarding 是 Tailscale 的 magic**,spectyn 是 model 路由
- 強調:**OpenAI-compatible API**(LM Studio 看得到 = 其他工具也看得到)
- 受眾:LM Studio 已有 100k+ 用戶,直接接到他們

## v0.7.0 follow-up
- 後續可拍「同樣 setup 跑 vision model」(等 vision provider PR)
