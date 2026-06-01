# Demo 01 — Telegram → 三台機器執行

**長度（Length）**：60 秒
**情境來源（Scenario source）**：doc 28 T1.1 → T2.10 + doc 30 §4 旗艦案例 #1
**狀態（Status）**：🟡 等 GA + v0.6.0 V3 production-default（正式環境預設）

## 鉤子（Hook，給字幕 + 置頂留言用的一句話）

> "From your phone — one message — three machines coordinate — result back in 30 sec. No CLI, no SSH, no cloud bill."

## 演員陣容 / 場景設定（Cast / setup）

- **Phone（手機）**：iPhone，Telegram 開著，與 `@my_phantom_mesh_bot` 對話
- **node-a**（主舞台 desktop，桌機）：TUI（文字使用者介面）視覺化任務跑;靠左半邊 screen（畫面）
- **node-c**（右上 picture-in-picture，子母畫面）：跑 LLM（大型語言模型，MLX）;右上小視窗
- **node-b**（右下 picture-in-picture，子母畫面）：跑 lint（程式碼檢查）;右下小視窗
- 4 個畫面用 OBS scene（OBS 場景）排,中央 phone screen mirror（手機畫面鏡射）

## 60 秒腳本（60-second script）

| 時間 | 螢幕上的畫面 | 旁白 / 字幕 |
|---|---|---|
| 0:00-0:03 | 標題:"phantom-mesh — one message, three machines" | — |
| 0:03-0:08 | 手機特寫:Telegram 開新訊息 | "I'm at a coffee shop." |
| 0:08-0:15 | 手機:打字「summarise the README, lint the Rust code, draft a PR」+ 送出 | （顯示文字氣泡 fade in，淡入）|
| 0:15-0:20 | 切到 node-a:TUI 跳出 `[ToolCall: read_file(README.md)]` | "node-a reads the README." |
| 0:20-0:25 | 切到 node-c:TUI 跳出 `[Route: provider=mlx]` + streaming text（串流文字） | "node-c runs the LLM locally, on Apple Silicon." |
| 0:25-0:35 | 切到 node-b:terminal（終端機）跑 `cargo clippy`,output 流出 | "node-b runs the linter." |
| 0:35-0:45 | 切到 node-a:`[Tool: git_create_pr]` + 印出 URL | "All three machines feed back to a coordinator." |
| 0:45-0:53 | 切回手機:Telegram bot 回 message:"PR #42 ready: https://..." | "30 seconds later — done." |
| 0:53-0:60 | 結尾卡片（end card）:phantom-mesh logo + URL + GitHub stars badge（星星數徽章） | "phantom-mesh.io" |

## 錄製前檢查清單（Pre-record checklist）

- [ ] 3 台機器都在 Tailscale 上,用 `phantom peer list` 確認 peer list（節點清單）
- [ ] Telegram bot allowlist（允許清單）包含示範用 Telegram account
- [ ] node-c MLX 跑著（`phantom mlx serve`）+ node-b phantom 跑著
- [ ] node-a 顯示乾淨的 TUI（`/clear`）,sidebar（側邊欄）開
- [ ] OBS 4-scene 排好（phone mirror / node-a / node-c PiP / node-b PiP）
- [ ] 示範用 prompt（提示詞）預先在手機 Telegram draft（草稿）好,不打錯

## 錄製後備註（Post-record notes，字幕 / 置頂）

- 主賣點:**no cloud bill**（沒有雲端帳單;三台都是自己的機器）
- 對比:不像 Zapier / n8n 要走 SaaS（軟體即服務）
- 強調:三台用 Tailscale,**沒開 public port**（公開連接埠）（隱私 + 安全）
- 連結:GitHub repo / 安裝指令一行

## 散播管道（Distribution）

- YouTube + LinkedIn（60s 完整）
- X（60s + 3-tweet thread，三則推文串）
- HN（連到 YouTube + 200 字背景）
