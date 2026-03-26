# B.2 Desktop App (Tauri v2 + React) — 工作日誌

> 開始日期：2026-03-21
> Spec：`docs/superpowers/specs/2026-03-21-phantom-mesh-app-platform-design.md` §2 + 附錄 B.2
> 專案目錄：`LLM-Cluster-Project/phantom-mesh-desktop/`

---

## 里程碑進度

| # | 里程碑 | 狀態 | 完成時間 | 備註 |
|---|--------|------|----------|------|
| M2.1 | Tauri 骨架 (`cargo tauri dev` 開啟空視窗) | ✅ 完成 | 2026-03-21 23:15 | App 可啟動，視窗 + 托盤 + 16 頁 sidebar |
| M2.2 | phantom-mesh 嵌入 (daemon 在 App 內啟動) | ✅ 完成 | 2026-03-21 23:45 | daemon.rs 進程管理，auto-start，auto-detect binary |
| M2.3 | 基本 UI (Dashboard + Chat + 側邊欄) | ✅ 完成 | 2026-03-21 23:15 | Dashboard 有 stat cards、Chat 有輸入框、sidebar 7 分組 |
| M2.4 | 完整 16 頁面 + tauri::command 對接 | ✅ 完成 | 2026-03-21 23:45 | 16 頁面全部有實際 UI，16 commands (13 HTTP + 3 daemon) |
| M2.5 | 系統托盤 + 自動更新 | ✅ 完成 | 2026-03-21 | 托盤三選單 + daemon 生命週期 + updater.rs (check + install) |

---

## 工作記錄

### 2026-03-21

#### 環境檢查 ✅
- [x] Node.js v24.4.1
- [x] npm 11.4.2 / pnpm 10.28.2
- [x] Tauri CLI — 背景安裝中 (`cargo install tauri-cli`)
- [x] WebView2 146.0.3856.62
- [x] Rust 1.93.1 / Cargo 1.93.1

#### M2.1 Tauri 骨架 — 進行中
- [x] `phantom-mesh-desktop/` 目錄建立
- [x] `src-tauri/Cargo.toml` — Tauri v2 + 6 plugins + reqwest + tokio
- [x] `src-tauri/build.rs`
- [x] `src-tauri/tauri.conf.json` — 1280x800 視窗、系統托盤
- [x] `src-tauri/src/main.rs` — plugin 註冊、系統托盤（zh-TW）、16 commands、auto-start daemon
- [x] `src-tauri/src/daemon.rs` — DaemonState + start/stop/status + auto-detect binary
- [x] `src-tauri/src/commands/` — 5 模組（health, cluster, agent, provider, settings）
- [x] `package.json` — React 19 + Vite 6 + TailwindCSS 3 + react-router-dom 7
- [x] `vite.config.ts` + `tsconfig.json` + `postcss.config.js` + `tailwind.config.js`
- [x] `index.html` — zh-TW, dark mode
- [x] `src/main.tsx` + `src/App.tsx` — sidebar 導航 + 16 路由
- [x] `src/pages/*.tsx` — 16 個頁面全部完成（含 mock data）
- [x] `src/lib/api.ts` + `src/hooks/useApi.ts` — Tauri invoke 封裝
- [x] `pnpm install` 安裝 node_modules（hoisted mode for exFAT）
- [x] `cargo run` 驗證可啟動 ✅
- [x] `npx vite build` 前端 production build ✅ (289KB JS, 14KB CSS)
- [x] `cargo check` Rust 編譯通過 ✅

#### M2.2 Daemon 嵌入
- [x] `src-tauri/src/daemon.rs` — DaemonState, find_binary(), start/stop/status commands
- [x] Auto-detect binary: same dir → ../../phantom-mesh/target/{release,debug}/ → PATH
- [x] Auto-start on app launch (when auto_start=true)
- [x] Auto-kill on tray quit
- [x] Health check with retry (5 attempts, 500ms interval)

#### M2.5 Updater + Step 1 完成
- [x] `src-tauri/src/updater.rs` — check_for_updates + install_update commands
- [x] `src-tauri/src/commands/tasks.rs` — get_task_history (GET /task/history)
- [x] `src-tauri/src/commands/security.rs` — get_audit_log (GET /audit, 支援 risk_level + limit)
- [x] `src-tauri/src/commands/memory.rs` — get_memory_observations + get_memory_stats + search_memory
- [x] `src-tauri/src/commands/provider.rs` — 新增 get_provider_health (GET /api/providers/health)
- [x] `src-tauri/src/commands/health.rs` — 新增 get_estop_status (GET /estop)
- [x] `src-tauri/src/commands/mod.rs` — 新增 tasks, security, memory 模組宣告
- [x] `src-tauri/src/main.rs` — 註冊全部 22 commands + updater plugin
- [x] `src/pages/Evolution.tsx` — Skills/Plugins/Adaptation 三分頁 + mock data
- [x] `src/pages/Logs.tsx` — Terminal-style 日誌檢視器 + level/module filter
- [x] Step 1 合併驗證: cargo check ✅ + tsc --noEmit ✅

#### M2.4 頁面充實
- [x] Agents.tsx — 5 agent cards + execute modal
- [x] Tasks.tsx — task queue table + filter bar
- [x] Providers.tsx — 4-tier provider cards + API key management
- [x] Channels.tsx — 4 channel rows + inline config
- [x] Memory.tsx — 3-layer memory + 4 tabs + search
- [x] Network.tsx — 3-layer network + 8 node topology
- [x] Security.tsx — audit log table + risk filter + stats

---

## 建立的檔案清單

### src-tauri/ (Rust 後端)
```
src-tauri/Cargo.toml
src-tauri/build.rs
src-tauri/tauri.conf.json
src-tauri/src/main.rs
src-tauri/src/commands/mod.rs
src-tauri/src/commands/health.rs
src-tauri/src/commands/cluster.rs
src-tauri/src/commands/agent.rs
src-tauri/src/commands/provider.rs
src-tauri/src/commands/settings.rs
```

### src/ (React 前端)
```
package.json
tsconfig.json
vite.config.ts
postcss.config.js
tailwind.config.js
index.html
src/main.tsx
src/index.css
src/App.tsx
src/lib/api.ts
src/hooks/useApi.ts
src/pages/Dashboard.tsx
src/pages/Chat.tsx
src/pages/Cluster.tsx
src/pages/Agents.tsx
src/pages/Tasks.tsx
src/pages/Hands.tsx
src/pages/Providers.tsx
src/pages/Economy.tsx
src/pages/Channels.tsx
src/pages/Tools.tsx
src/pages/Memory.tsx
src/pages/Network.tsx
src/pages/Security.tsx
src/pages/Evolution.tsx
src/pages/Logs.tsx
src/pages/Settings.tsx
```

### Tauri Commands 對應表
| Command | Module | API 端點 |
|---------|--------|----------|
| get_health | health.rs | GET /health |
| get_dashboard_status | health.rs | GET /api/dashboard/status |
| get_cluster_status | cluster.rs | GET /cluster/status |
| get_cluster_workers | cluster.rs | GET /cluster/workers |
| get_cluster_scores | cluster.rs | GET /cluster/scores |
| run_agent | agent.rs | POST /agent/:name/run |
| run_hand | agent.rs | POST /hand/:name/run |
| get_costs | provider.rs | GET /costs |
| get_revenue | provider.rs | GET /revenue |
| get_tools | provider.rs | GET /tools |
| get_hands | provider.rs | GET /hands |
| get_config | settings.rs | (local) |
| set_config | settings.rs | (local) |

---

## 第一波：完成 B.2（進行中）

### 執行計畫
```
Step 1 (平行): A1 新 commands + A4 stub 頁面 + A5 updater
Step 2 (平行): A2 頁面組1 + A3 頁面組2 (依賴 A1)
Step 3: 合併 → cargo check + tsc → 啟動測試
Step 4: 三方 AI 審查 (Claude Code / Codex / Gemini)
Step 5: 修復審查發現的問題
```

### Step 1 進度
| Agent | 任務 | 狀態 |
|-------|------|------|
| A1 | 新增 7 Tauri commands (Rust) | ✅ 完成 |
| A4 | Evolution + Logs 頁面 | ✅ 完成 |
| A5 | Updater 接線 | ✅ 完成 |

Step 1 合併驗證: `cargo check` ✅ + `tsc --noEmit` ✅ (2026-03-21)

### Step 2 進度
| Agent | 任務 | 狀態 |
|-------|------|------|
| A2 | Agents/Tasks/Providers 接 API | ✅ 完成 |
| A3 | Channels/Memory/Network/Security 接 API | ✅ 完成 |

Step 2 合併驗證: `cargo check` ✅ + `tsc --noEmit` ✅ + `vite build` ✅ (325KB JS, 16KB CSS)

#### Step 2 變更詳情
- Agents.tsx: get_cluster_workers on mount, run_agent 修正為 { name, input } 簽名
- Tasks.tsx: get_task_history on mount, best-effort field mapping, refresh button
- Providers.tsx: get_provider_health on mount, 動態 JSON 解析, refresh button
- Channels.tsx: get_health 檢查連線狀態, 狀態 badge (連線中/離線模式)
- Memory.tsx: get_memory_stats + get_memory_observations 並行載入, search_memory API 搜尋
- Network.tsx: get_cluster_workers 動態拓撲, parseNetworkNode 多欄位映射
- Security.tsx: get_audit_log 伺服器端篩選 (risk_level + limit), parseAuditEvent 結果映射
- 所有 7 頁面: loading spinner + error banner + 離線模式 fallback + retry button

### Step 4-5: 三方 AI 審查 + 修復
#### Claude Code 審查結果
發現 4 Critical + 7 Important + 6 Suggestion 問題

**已修復 Critical:**
- C1: CSP 從 null 改為限制性策略 (self + 127.0.0.1 + github.com)
- C3: Chat.tsx run_agent 參數修正 { prompt } → { name: "master", input: text }

**已修復 Important:**
- I1: Dashboard.tsx `any` 型別 → DashboardStatus interface + string|number
- I2: 所有 command 檔改用共用 HttpClient (reqwest 連線池)
- I3: security.rs + memory.rs 改用 reqwest `.query()` 取代手動 format! URL 拼接
- I4: 所有 command 新增 `.error_for_status()` 在 JSON 解析前
- I6: daemon.rs + main.rs 所有 `.lock().unwrap()` → `.unwrap_or_else(|e| e.into_inner())`

**待處理 (pre-release):**
- C2: Updater pubkey 空值 — 需 `cargo tauri signer generate` 產生金鑰
- C4: auth_key 預設空值 + config 持久化 (需實作 tauri-plugin-store 整合)
- I5: 7 頁面重複 loading/error 模式 → 可抽出共用 hook (下個迭代)
- I7: Channels 頁面無真實資料 API (待 B.3 Auto-Networking)

驗證: `cargo check` ✅ + `tsc --noEmit` ✅ + `vite build` ✅ (325KB JS, 16KB CSS)

#### Codex CLI 審查結果
發現 0 Critical + 9 Important + 5 Suggestion 問題

**已修復 Important:**
- Chat.tsx `invoke<string>` 改為處理 JSON Value 回傳 (提取 output/result/message 欄位)
- daemon.rs stdout/stderr `Stdio::piped()` → `Stdio::null()` (消除管道死鎖風險)
- useApi.ts hook 依賴陣列加入 `JSON.stringify(args)` (修正 args 變更不 refetch)
- Memory.tsx `parseMemoryEntry` index 參數修正 (0 → 實際 index)
- Memory.tsx search 清空時恢復 baseline 資料
- Security/Network/Providers 頁面: API 回傳空陣列時不再偽裝為 mock 資料

**待處理 (pre-release):**
- Provider API key 不應直接傳至 renderer (需 backend-only 遮罩)
- daemon.rs 可考慮用 Tauri Sidecar 取代手動 binary 搜尋 (production)

#### Gemini CLI 審查結果
發現 2 Critical + 4 Important + 4 Suggestion 問題

**與 Claude/Codex 重疊:**
- Critical 1: Updater pubkey 空值 (同 Claude C2)
- Critical 2: Chat.tsx 型別不匹配 (同 Codex，已修復)
- Important 2: Config 持久化 TODO (同 Claude C4)

**新發現:**
- Important 1: 建議用 Tauri Sidecar 取代手動 binary discovery (同 Codex)
- Important 3: useApi.ts args 未在 dependency array (已修復)
- Important 4: daemon.rs check_health 每次 new reqwest::Client (已簡化為 reqwest::get)
- Suggestion: CSP `unsafe-inline` 可 production 加固
- Suggestion: daemon 應監測 parent PID 防殭屍 (app crash 後 daemon 殘留)

驗證 (Step 5 全部修復後): `cargo check` ✅ + `tsc --noEmit` ✅ + `vite build` ✅ (325KB JS, 16KB CSS)

### 新增 Commands 清單
| Command | Module | API 端點 |
|---------|--------|----------|
| get_task_history | tasks.rs | GET /task/history |
| get_provider_health | provider.rs | GET /api/providers/health |
| get_audit_log | security.rs | GET /audit |
| get_memory_observations | memory.rs | GET /memory/observations |
| get_memory_stats | memory.rs | GET /memory/observations/stats |
| search_memory | memory.rs | GET /memory/observations?query= |
| get_estop_status | health.rs | GET /estop |
