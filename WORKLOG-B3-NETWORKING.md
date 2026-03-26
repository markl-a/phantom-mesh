# B.3 Auto-Networking (三層自動組網) — 工作日誌

> 開始日期：2026-03-22
> Spec：`docs/superpowers/specs/2026-03-21-phantom-mesh-app-platform-design.md` §B.3
> Prompt：`../../PROMPT-B3.md`

---

## 里程碑進度

| # | 里程碑 | 狀態 | 完成時間 | 備註 |
|---|--------|------|----------|------|
| M3.1 | Module Structure + Traits | ✅ 完成 | 2026-03-22 00:30 | 5 files, 15 tests pass |
| M3.2 | mDNS Discovery 實作 | ✅ 完成 | 2026-03-22 01:00 | mdns-sd + flume, 18 tests pass |
| M3.3 | Route Manager + Fallback | ✅ 完成 | 2026-03-22 01:15 | transport query + static routes + 23 tests |
| M3.4 | 整合現有 Cluster | ✅ 完成 | 2026-03-22 02:00 | AppState + mDNS init + 3 API endpoints + auto-register sync |
| M3.5 | Iroh Transport (Trait + Stub) | ✅ 完成 | 2026-03-22 | stub + 3 tests, cargo check ✅ |
| M3.6 | Desktop UI 更新 | ✅ 完成 | 2026-03-22 | 3 Tauri commands + Network.tsx enhanced + auto-refresh |
| M3.7 | 最終驗證 | ✅ 完成 | 2026-03-22 | 三方驗證×2 全通過 |

---

## 工作記錄

### 2026-03-22 (Ralph Loop 自動執行)

#### M3.1 Module Structure + Traits ✅
- [x] `src/networking/mod.rs` — 模組宣告 + re-exports
- [x] `src/networking/discovery.rs` — ServiceDiscovery trait, DiscoveredNode, ConnectionLayer enum
- [x] `src/networking/transport.rs` — MeshTransport trait, PeerId, PeerInfo, TransportChannel
- [x] `src/networking/route_manager.rs` — RouteManager (cache + best_route + all_routes + refresh loop)
- [x] `src/networking/mdns.rs` — MdnsDiscovery struct (functional, pending real mdns-sd integration)
- [x] `src/networking/iroh_transport.rs` — IrohTransport stub
- [x] `pub mod networking;` added to lib.rs
- [x] `cargo check` ✅ + 15/15 tests pass
- 注意: exFAT file lock 問題需用 `CARGO_TARGET_DIR=target5 cargo test` 繞過

#### M3.4 Integration with Existing Cluster ✅
- [x] Added `route_manager: Option<Arc<RouteManager>>` to AppState
- [x] Networking init block: creates MdnsDiscovery, starts browsing, creates RouteManager
- [x] Auto-register sync loop: discovered nodes → ClusterRegistry every 15s
- [x] RouteManager refresh loop (30s): evict stale, re-probe discoveries
- [x] 3 new API endpoints: `/networking/discovered`, `/networking/routes`, `/networking/status`
- [x] Fixed: `port` scope, `ServiceDiscovery` trait import, router state type ordering
- [x] `cargo check` ✅ + 23/23 networking tests pass

#### M3.5 Iroh Transport (Trait + Stub) ✅
- [x] `src/networking/iroh_transport.rs` — IrohTransport stub with listen/stop/connect/peers
- [x] 3 unit tests: listen_and_stop, connect_fails, peers_empty
- [x] Registered in RouteManager in main.rs networking init block
- [x] `cargo check` ✅ + 23/23 tests pass (兩輪驗證)

#### M3.6 Desktop UI — Network Page Enhancement ✅
- [x] `phantom-mesh-desktop/src-tauri/src/commands/networking.rs` — 3 Tauri commands:
  - `get_network_discovery` → GET /networking/discovered
  - `get_network_routes` → GET /networking/routes
  - `get_network_status` → GET /networking/status
- [x] `commands/mod.rs` — added `pub mod networking;`
- [x] `main.rs` — registered 3 networking commands in invoke_handler
- [x] `src/pages/Network.tsx` — enhanced:
  - Fetches real discovery data via `get_network_discovery` + `get_network_status`
  - Shows RouteManager status bar (enabled/disabled, backend counts, known routes)
  - Auto-refresh every 10s with cleanup on unmount
  - `normalizeLayer()` maps backend layer names (mDNS/LAN/QUIC/HTTP) to UI layers
  - Graceful fallback to mock data when daemon offline
- [x] 三方驗證×2:
  - `cargo check` (src-tauri) ✅✅
  - `npx tsc --noEmit` ✅✅
  - `npx vite build` ✅✅

#### M3.7 最終驗證 ✅
- [x] `phantom-mesh`: cargo check ✅✅ + 23/23 networking tests ✅✅
- [x] `phantom-mesh-desktop/src-tauri`: cargo check ✅✅
- [x] `phantom-mesh-desktop`: tsc --noEmit ✅✅ + vite build ✅✅
- [x] 所有三方驗證×2 通過，B.3 完成

#### 三方審查後修復 (Codex GPT-5.4 發現的問題) ✅
- [x] **Fix #1**: IPv6 URL bracket — `DiscoveredNode::http_url()` 改為 `http://[::1]:7878` 格式
- [x] **Fix #2**: mDNS callback 鎖風險 — `add_node()` 先 drop nodes write lock 再觸發 callback
- [x] **Fix #3**: 背景 task shutdown handle — `networking_tasks: Arc<Mutex<Vec<JoinHandle>>>` 收集 handles
- [x] **Fix #4**: API JSON 統一 — `/discovered` 改回 `{ "nodes": [...] }` 物件格式; layer 用 serde enum
- [x] **Fix #5**: best_route 優先級 — discovery 先於 cache，mDNS 正確壓過 HTTP cache
- [x] **Fix #6**: Network.tsx in-flight guard — `fetchingRef` 防止 setInterval 慢請求堆積
- [x] URL parsing — `url::Url::parse()` 取代字串切割，正確處理 IPv6/bracket
- [x] 新增 IPv6 test (`discovered_node_http_url_ipv6`)
- [x] 修正 `route_priority_mdns_over_http` test 預期值
- [x] 三方驗證×2: cargo check ✅✅ + 24/24 tests ✅✅ + tsc ✅✅ + vite build ✅✅
