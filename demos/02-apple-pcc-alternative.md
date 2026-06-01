# Demo 02 — 自架 Apple PCC 替代方案

**長度**：60 秒
**情境來源**：doc 28 K5 + doc 30 §4 旗艦 #2
**狀態**：🟡 v0.6.0/v0.7.0（iOS app + Mac MLX 都得在）

## 鉤子（Hook）

> "Apple Intelligence sent your prompt to Private Cloud Compute. Want it to go to YOUR Mac Studio instead? phantom-mesh, 60 seconds."

## 演員 / 佈置（Cast / setup）
- **iPhone**：shortcut（捷徑）/ app 介面;近拍長 prompt（提示詞）
- **網路圖**：動畫,iPhone → 「Apple PCC」(劃 X)→「m1 at home」(箭頭)
- **m1**：TUI（文字使用者介面）跑著,MLX 接收 prompt + 跑大模型,終端流出
- **隱私強調**：Wireshark / Little Snitch 顯示「**0 packets to apple.com**（0 個封包送往 apple.com）」

## 60 秒腳本

| 時間 | 畫面內容 | 旁白 |
|---|---|---|
| 0:00-0:05 | Apple Intelligence 喚醒動畫,prompt 飛向「Apple」雲端 | "Apple decides when to send your prompt to their cloud." |
| 0:05-0:10 | Wireshark 視窗：TLS to *.apple.com 高亮 | "Even if you trust Apple — sometimes you want it home." |
| 0:10-0:18 | iPhone：長 prompt(看得到內容,真實隱私敏感)— 配 iOS Shortcuts 或 phantom iOS app | "Type the same prompt..." |
| 0:18-0:25 | 網路圖：箭頭從 iPhone → Tailscale → 家裡 m1 | "Routed via Tailscale to your home Mac Studio." |
| 0:25-0:40 | 切到 m1：TUI 顯示 prompt 收到,MLX 7B/70B model load + streaming response（串流回應） | "MLX runs Llama 3 70B locally. No bill, no log." |
| 0:40-0:50 | iPhone：回應 stream 回來 | "Result lands on your phone." |
| 0:50-0:57 | Little Snitch：**"0 connections to apple.com" highlight** | "And nothing — nothing — left your home." |
| 0:57-0:60 | 結尾卡（End card）：phantom-mesh logo + "Your PCC, your rules." | — |

## 預先錄製檢查清單

- [ ] m1 上 `phantom mlx pull llama3-70b-instruct` 完成
- [ ] m1 上 `phantom mlx serve --model llama3-70b-instruct` 跑著
- [ ] m1 在 Tailscale,iPhone 也是
- [ ] iOS Shortcut(或 phantom iOS app v0.6.0+)設好,觸發 HTTPS POST 到 m1
- [ ] Wireshark / Little Snitch 開著,filter `apple.com` 之類
- [ ] 示範用 prompt 寫好(夠長、夠真實,e.g. 醫療諮詢 / 法律問題)

## 錄製後筆記

- 主對手：Apple Intelligence + 默默被詬病的「不知道何時送 PCC」
- 強調：**all-local except Tailscale relay（除了 Tailscale relay 中繼之外全部在地端,可選 DERP-free 設定）**
- 對 privacy-conscious（重視隱私者）/ lawyer（律師）/ journalist（記者）/ doctor（醫師）行銷強
- 連到 §28 K5 + doc 30 §6 iOS section

## 字幕中的注意事項（誠實）

- "v0.6.0 needs an iOS Shortcut as the bridge; the dedicated iOS app ships in v0.7.0."
