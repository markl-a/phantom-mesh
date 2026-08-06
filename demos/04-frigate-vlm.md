# Demo 04 — Frigate + VLM "Package Delivered" Smart Alert

**Length**: 60 seconds
**Scenario source**: doc 28 F3 + doc 30 §4 flagship #4
**Status**: 🔴 v0.7.0+(需 vision provider — 目前 12 provider 都是 text-only)

## Hook

> "Your security camera saw 'person'. But was it a delivery guy? Your kid? An intruder? spectyn-mesh asks a vision LLM — locally — and only pings you when it matters."

## Cast / setup
- **門口攝影機**(IP cam / 環門口或入口的視角)
- **Frigate NVR**(Linux box / Synology / Pi):跑著,偵測到 person
- **node-a / GPU 機**:跑 spectyn + vision provider(Pixtral / GPT-4V / Claude Vision)
- **iPhone**:接收 APNs / Pushover / Telegram 通知

## 60-second script

| Time | What's on screen | Voiceover |
|---|---|---|
| 0:00-0:05 | 門口攝影機:有人走過鏡頭(快遞員,手抱包裹)| "Frigate sees: person." |
| 0:05-0:12 | Frigate 截圖 → 送到 spectyn-mesh(箭頭動畫) | "It asks spectyn-mesh: 'who is this, and why?'" |
| 0:12-0:25 | node-a TUI:`[ToolCall: vlm_describe(image=...)]` → `[ToolResult: a delivery person carrying a brown box, walking toward the door]` | "Spectyn routes the image to a vision LLM, running locally on the gaming PC." |
| 0:25-0:35 | 邏輯判斷:`if "carrying" in desc: alert("Package delivered")` 走過 → 沒按門鈴 → 30s 後沒人 → "package likely dropped" | "It applies your rules. Carrying box + no doorbell + still 30s = package dropped." |
| 0:35-0:48 | iPhone 跳通知:「📦 包裹送達」+ 截圖縮圖 | "Phone alert. Just the once." |
| 0:48-0:55 | 反例對比:同一鏡頭路過鄰居 → 沒通知(Spectyn 不認識為 delivery) | "Your kid walking past? No alert." |
| 0:55-0:60 | End card + 「contrast with: dumb motion alerts spam」 | — |

## Pre-record checklist
- [ ] Frigate 跑著,接 cam stream,person detection 開
- [ ] spectyn 加 vision provider(等 v0.7.0)
- [ ] spectyn 收 Frigate webhook(自寫 small Python 接 Frigate MQTT → call spectyn)
- [ ] 預先拍好「快遞員」+「家人路過」兩段 footage
- [ ] iPhone 通知 channel 設好(Pushover / Telegram / APNs)

## Post-record notes
- 主對手:Frigate **沒有** VLM integration(只是 detection model)
- 主對手:Ring / Nest 這類雲端 + 廣告 → 用戶反感
- 強調:**all-local pipeline**(cam → Frigate → spectyn → 通知都不出家)
- 受眾:Frigate 已有 50k+ self-hoster

## v0.7.0 依賴
- 至少 1 個 vision provider 接好(Claude Vision / GPT-4V / Pixtral 或本機 Llava)
- spectyn-mesh/core/src/providers/vision.rs(待建)
- HA / MQTT integration(可選但加分)

## 不要做的(誠實 caveat)
- **不要說「100% 沒漏報」** — VLM 還是會偶爾錯
- **不要說「比 Ring 安全」** — Ring 在 hardware 端,不同 layer
