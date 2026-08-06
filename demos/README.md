# `demos/`

旗艦級 60 秒 demo（示範影片）腳本。從 doc 28 §4 中**精選 5 個**——這些情境最能展現「spectyn-mesh 的特色形態」：跨裝置、真實硬體、有別於任何單一競品工具。

> 測試堆疊中的層級：**L4 旗艦 demo 影片**
> 參見 `goal_plan/docs/29 §3 L4` + `goal_plan/docs/28 §4`。
> 於 v0.6.0 GA（正式發行，2026-06-15）後再製作；這些是行銷素材，並非出貨阻擋項目（ship blocker）。

## 精選 5 個

| # | Demo | 為何是旗艦 |
|---|---|---|
| 01 | **Telegram → 3-machine execution** | 唯一 100% 對應 spectyn-mesh 形態的使用者故事（V3 production-default，生產環境預設） |
| 02 | **Self-hosted Apple PCC alternative** | 熱門市場：對 Apple Intelligence 失望的使用者（K5 情境） |
| 03 | **LM Studio remote model over Tailscale** | 對既有安裝使用者群來說易於上手的進入點（D6 情境） |
| 04 | **Frigate + VLM "package delivered" alert** | Homelab（家用實驗室）使用者——具體且帶情感（F3 情境） |
| 05 | **Night-shift refactor → 8 PRs at 7am** | 開發者／獨立開發者（indie hacker）市場（D5 情境） |

## 格式

每個 demo 檔案遵循以下結構：
```text
- Hook (1 sentence; what the viewer's takeaway should be)
- Cast / setup (1 paragraph; who and what's on screen)
- 60-second script (10-15 timestamped beats)
- Pre-record checklist (things to set up before pressing record)
- Post-record notes (what to highlight in caption / pinned comment)
```

## 工具

- **OBS Studio** — 錄製 + 場景切換
- **asciinema** — 用於純終端機（terminal）片段（以 130-150% 字型大小錄製）
- **Screen Studio**（Mac）或 **OBS**（Win）— 用於錄製過程中的縮放
- **DaVinci Resolve** 或 **CapCut** — 輕度剪輯（剪掉停頓、加字幕）

## 發佈目標

- YouTube（完整 60 秒）
- LinkedIn（方形裁切，60 秒）
- X / Twitter（方形裁切，60 秒 + 推文串）
- Hacker News（連到 YouTube + 200 字的脈絡說明貼文）
- spectyn-mesh-site 著陸頁（landing page）主視覺影片

## 時程

- **建立骨架（本目錄）**：🟢 現在
- **錄製**：GA 後 1 週（2026-06-16~6-22）
- **發佈**：GA 後 2 週（2026-06-23+），配合 release blog post（發行部落格文章）
