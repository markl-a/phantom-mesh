# Demo 05 — 夜班重構 → 早上 7 點生出 8 個 PR

**Length（長度）**: 60 秒
**Scenario source（情境來源）**: doc 28 D5 + doc 30 §4 旗艦 #5
**Status（狀態）**: 🟡 v0.6.0(`phantom autoevolve schedule` 已 ship,但要拍 demo 等夜間 footage（影像素材）)

## Hook（開場鉤子）

> "You go to sleep. Your gaming PC stays up, refactoring your repo. 7am — your phone buzzes — '8 PRs ready for review.'"

## Cast / setup（演員／場景設定）
- **Time-lapse**: 夜間室內,gaming PC 螢幕亮,monitor 顯示 cargo + git 滾動
- **iPhone**: 早上鬧鐘 → notification preview「8 PRs ready」
- **GitHub 介面**: PR list,8 個 PR 都有 phantom 簽名 + 通過 CI
- **Clock**: 動畫,從 23:00 → 02:00 → 05:00 → 07:00

## 60 秒腳本

| 時間 | 畫面上呈現的內容 | 旁白（Voiceover） |
|---|---|---|
| 0:00-0:05 | 真人在床邊放手機 → 關燈 → "23:00" | "23:00. You go to sleep." |
| 0:05-0:12 | Time-lapse:phantom autoevolve TUI 滾動,終端不斷有任務跑出 | "Your gaming PC keeps working." |
| 0:12-0:22 | 鏡頭拉近 terminal:`phantom autoevolve --watch --target check --max-rounds 5` 跑著,每 hour 一輪 | "Every hour: cargo check → if red, try to fix → if green, refactor or test." |
| 0:22-0:32 | GitHub:PR list 數字從 0 ↑ 4 ↑ 8(time-lapse)| "PRs created automatically — only when tests stay green." |
| 0:32-0:42 | 切到 07:00 鬧鐘 → 手機通知:「📊 8 PRs ready · 6 green · 2 need review」+ digest mail summary | "Morning digest on your phone." |
| 0:42-0:53 | 點 GitHub PR — 看到 phantom 寫的 PR 描述 + diff + 全部 CI 綠 | "Each PR has a clear description, a clean diff, and CI passing." |
| 0:53-0:60 | End card:"sleep is when you ship the most" + phantom-mesh logo | — |

## 預錄製檢查清單
- [ ] node-a / gaming PC 上 `phantom autoevolve schedule install --interval 3600`
- [ ] `EVOLVE-GOALS.md` 預先寫 5-10 個小目標(refactor X / add test for Y)
- [ ] git 設好 push 權限 + PR 模板(phantom 用得到)
- [ ] 至少 2-3 天連跑 + GitHub 累積 PR 紀錄
- [ ] Time-lapse 拍 1-2 晚(GoPro / phone tripod)

## 錄製後備註
- 主對手:Cursor / Copilot 等需要人在電腦前才動
- 主對手:雲端 SaaS agent(Devin / Aider remote)價格高
- 強調:**你的硬體閒置時做事**(電費比 SaaS 便宜)
- 強調:**綠 CI 才 PR**(不會塞垃圾 PR)
- 受眾:indie hacker / solo dev / 副業開源項目維護者

## 字幕中的誠實聲明（caveat）
- "PRs still need human review before merge. phantom drafts; you decide."
- "Not every task is autoevolve-able — works best for refactor, test coverage, small features."
