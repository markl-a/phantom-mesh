# 架構參考文件

Phantom Mesh 各子系統的架構文件 — 包含目的、關鍵檔案、資料流（data flow）、
擴充點（extension point）與測試。在修改某子系統前，請先從這裡開始了解它如何運作。

## 執行時核心（Runtime core）
- [agent-runtime](agent-runtime.md) — 多供應商（multi-provider）SSE（伺服器推送事件）代理迴圈、工具派發（tool dispatch）、壓縮（compaction）。
- [auth-security-gate](auth-security-gate.md) — 工具/RPC 的預設安全（secure-by-default）認證閘門（auth gate）。
- [provider-routing](provider-routing.md) — LLM（大型語言模型）供應商選擇、備援（fallback）、重試（retry）。
- [cli-phantom](cli-phantom.md) — `phantom` CLI 介面。
- [mcp-server](mcp-server.md) — 對外提供工具的 Model Context Protocol（模型上下文協議）伺服器。

## 叢集與分散式（Cluster & distribution）
- [cluster-dispatch](cluster-dispatch.md) — 跨主機（cross-host）任務派發 + 能力路由（capability routing）。
- [capabilities-routing](capabilities-routing.md) — 能力探索（capability discovery）與以能力路由的任務。
- [broker-vault](broker-vault.md) — broker（中介伺服器）JWT（JSON Web Token，網頁權杖） + 各使用者加密金鑰 vault（保險庫）。

## 儲存與加密（Storage & crypto）
- [at-rest-crypto-storage](at-rest-crypto-storage.md) — HKDF（雜湊金鑰衍生函數）/age/HMAC（雜湊訊息驗證碼）的靜態加密（at-rest encryption）。
- [event-storage](event-storage.md) — 裝置端事件儲存（event store） + FTS5（全文檢索第 5 版）。

## 生活軌道（Life Track，擷取 → 教練）
- [capture-wires](capture-wires.md) — 專注 / 飲食 / 習慣的擷取管線（capture pipeline）。
- [coach-daily-review](coach-daily-review.md) — 教練引擎（coach engine） + 每日回顧（daily review）。
- [evolve-goals](evolve-goals.md) — 目標演化（goal evolution） + 檢查點（checkpoint）。

## 技能與通道（Skills & channels）
- [skills](skills.md) — 技能擷取/策展（curation）迴圈。
- [channels-telegram](channels-telegram.md) — Telegram 機器人通道（bot channel）。

## 應用程式與平台（App & platform）
- [onboarding-llm-providers](onboarding-llm-providers.md) — 初次登入 → LLM 設定 → 使用確認流程；供應商分類（雲端共享 / 本機訂閱-CLI / 本地伺服器）與每機優先序。**內部文件（含灰色訂閱策略 + ToS）— 不進 public sync。**
- [app-tauri-frontend](app-tauri-frontend.md) — Tauri 桌面/行動端前端。
- [i18n-localization](i18n-localization.md) — 在地化（localization，en / zh-TW）。

## 品質（Quality）
- [selftest-harness](selftest-harness.md) — `phantom selftest` + `scripts/selftest.d/`。

> 這些文件由原始碼產生並維持其準確性 — 若某個檔案搬移或某條流程改變了，
> 請在同一個 PR 中更新對應的文件。
