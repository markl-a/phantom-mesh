# Phantom Mesh App Platform — Design Specification

> **設計目標：** 將 phantom-mesh 封裝為可安裝的桌面/手機 GUI App，支援自動組網、加密金鑰管理、動態 SubAgent、插件熱更新、自然語言操控，以及跨節點記憶同步。
>
> **設計原則：** 所有模組可插拔替換（trait 抽象）。先做桌面，再做手機。

---

## 1. Plugin Bus 架構（基礎層）

所有功能模組透過統一 trait 介面掛載，可在執行時替換。

### 1.1 核心 Trait

```rust
/// 複用 health_check.rs 中的既有定義
pub use crate::health_check::HealthStatus;
// HealthStatus::Healthy | Degraded | Unhealthy

#[async_trait]
pub trait PluginModule: Send + Sync {
    fn id(&self) -> &str;
    fn version(&self) -> semver::Version;
    fn capabilities(&self) -> Vec<String>;
    async fn init(&self, ctx: &AppContext) -> Result<()>;
    async fn shutdown(&self) -> Result<()>;
    fn health(&self) -> HealthStatus; // 來自 health_check.rs
}
```

> **B1 釐清：** `HealthStatus` 使用 `health_check.rs` 中的三態 enum（Healthy / Degraded / Unhealthy）。
> `customer_health.rs` 的 `HealthGrade` 僅用於客戶健康評分，不混用。

### 1.2 Plugin Bus

```rust
pub struct PluginBus {
    modules: HashMap<String, Arc<dyn PluginModule>>,
    event_tx: broadcast::Sender<PluginEvent>,
}
```

### 1.3 模組分類

| 類型 | 模組 | 可替換範例 |
|------|------|-----------|
| Core | AgentRuntime, TaskQueue, DispatchMode | 核心不替換，可擴展 |
| Network | Iroh, libp2p, Tailscale | 任意 P2P/VPN 方案 |
| Auth | KeyVault, ClusterSync | HashiCorp Vault, AWS KMS |
| Memory | MemoryStore, ObservationalMemory | Redis, Pinecone, Qdrant |
| Provider | Ollama, Anthropic, OpenAI... | 隨時加新 Provider |
| Channel | Telegram, Slack, Discord... | 隨時加新頻道 |
| Storage | SQLite, pgvector | TiKV, DuckDB |
| UI | TauriUI, MobileUI, CLI | 前端獨立 |

### 1.4 模組間通訊

- **事件匯流排**（broadcast channel）— 解耦模組依賴
- **服務定位器**（AppContext）— 模組間透過 trait 取得服務
- **不直接互相引用** — 避免循環依賴

### 1.5 AppContext 定義

```rust
/// 服務定位器 — 仿照 AgentRuntime 的 DI 模式擴展
/// 所有 PluginModule 在 init() 時透過 AppContext 取得所需服務
pub struct AppContext {
    services: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    event_bus: Arc<broadcast::Sender<PluginEvent>>,
    config: Arc<AppConfig>,
}

impl AppContext {
    /// 註冊服務（init 階段由 PluginBus 呼叫）
    pub fn register<T: Send + Sync + 'static>(&mut self, service: Arc<T>) {
        self.services.insert(TypeId::of::<T>(), service);
    }

    /// 取得服務（模組透過 trait 型別取得）
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.services
            .get(&TypeId::of::<T>())
            .and_then(|s| s.clone().downcast::<T>().ok())
    }

    /// 訂閱事件匯流排
    pub fn subscribe(&self) -> broadcast::Receiver<PluginEvent> {
        self.event_bus.subscribe()
    }

    /// 發送事件
    pub fn emit(&self, event: PluginEvent) {
        let _ = self.event_bus.send(event);
    }
}
```

> **設計依據：** `AgentRuntime` 已使用 `Option<Arc<T>>` 注入 14 個子系統（`set_cost_tracker()` 等），
> `AppContext` 將此模式泛化為 type-keyed service locator。

### 1.6 模組初始化順序

```
Phase 1 — 基礎設施（無依賴）:
  Config → Logger → MetricsCollector

Phase 2 — 資料層:
  OptimizerStore → MemoryStore → TrajectoryLogger → RoiScheduler

Phase 3 — 安全層:
  InjectionGuard → KeyVault → PrivacyGuard

Phase 4 — 核心引擎:
  DispatchMode → ProviderRouter → ProviderCircuitBreaker → AgentRuntime

Phase 5 — 網路層:
  ServiceDiscovery(mDNS) → MeshTransport(Iroh) → ClusterHub

Phase 6 — 高階功能:
  SubAgentRunner → FeedbackLoop → EvolutionManager → NlInterface

Phase 7 — UI 層:
  WebDashboard → TauriUI / MobileUI
```

每個 Phase 完成後才啟動下一個。同一 Phase 內的模組可並行初始化。
`PluginBus.init_all()` 按此順序執行，並在失敗時回滾已初始化的模組。

### 1.7 與現有程式碼對應

- `PluginRegistry` / `plugin_loader.rs` → 擴展為 Plugin Bus
- `MemoryBackend` trait → 已是可插拔設計
- `Provider` trait → 11 個實作，完美範例

---

## 2. Desktop App（Tauri v2 + React）

### 2.1 技術棧

- **Rust 後端：** Tauri v2 + `phantom-mesh = { path = "../" }`
- **前端：** React + TypeScript
- **打包：** NSIS (Windows) / DMG (Mac) / AppImage (Linux)

### 2.2 專案結構

```
phantom-mesh-desktop/
├── src-tauri/
│   ├── src/
│   │   ├── main.rs          # Tauri 啟動 + phantom-mesh 初始化
│   │   ├── commands/         # #[tauri::command] 分組
│   │   │   ├── cluster.rs
│   │   │   ├── agent.rs
│   │   │   ├── provider.rs
│   │   │   ├── pipeline.rs
│   │   │   ├── economy.rs
│   │   │   ├── network.rs
│   │   │   ├── settings.rs
│   │   │   └── chat.rs
│   │   ├── tray.rs           # 系統托盤
│   │   └── updater.rs        # 自動更新
│   ├── Cargo.toml
│   └── tauri.conf.json
├── src/                      # React 前端
│   ├── layouts/Sidebar.tsx
│   ├── pages/                # 16 個頁面
│   │   ├── Dashboard.tsx
│   │   ├── Chat.tsx
│   │   ├── Cluster.tsx
│   │   ├── Agents.tsx
│   │   ├── Tasks.tsx
│   │   ├── Hands.tsx
│   │   ├── Providers.tsx
│   │   ├── Economy.tsx
│   │   ├── Channels.tsx
│   │   ├── Tools.tsx
│   │   ├── Memory.tsx
│   │   ├── Network.tsx
│   │   ├── Security.tsx
│   │   ├── Evolution.tsx
│   │   ├── Logs.tsx
│   │   └── Settings.tsx
│   └── components/
└── package.json
```

### 2.3 側邊欄導航分組

```
📊 儀表板
💬 對話（自然語言介面）

▸ 集群
   節點管理 / Agent 監控 / 網路拓撲

▸ 執行
   任務佇列 / Hand・Pipeline / 排程

▸ Provider
   API 金鑰（四層分級）/ 用量監控

▸ 經濟
   成本追蹤 / 收益報表 / ROI 分析

▸ 通訊
   Telegram / 其他頻道

▸ 工具 & 技能
   工具管理 / 技能市場

▸ 系統
   安全・審計 / 記憶 / 日誌 / 設定
```

### 2.4 系統托盤

```
右鍵選單:
├── 開啟主介面
├── 集群狀態: N/N 節點在線
├── ─────────────
├── 暫停 / 恢復 Agent
├── ─────────────
├── 檢查更新
└── 結束
```

### 2.5 啟動流程

1. Tauri 啟動
2. 載入 phantom-mesh（PluginBus.init_all()）
3. 啟動 Main Agent
4. 啟動 Auto-Networking（Iroh broadcast + mDNS fallback）
5. 最小化到系統托盤
6. 背景：自動更新檢查（每 6 小時）
7. 背景：SubAgent 依資源伸縮

### 2.6 必要 Tauri 插件

| 插件 | 用途 |
|------|------|
| tauri-plugin-updater | 自動更新 + 進度回報 |
| tauri-plugin-single-instance | 防止開兩個 App |
| tauri-plugin-notification | 系統推播通知 |
| tauri-plugin-store | 輕量 key-value 設定持久化 |
| tauri-plugin-biometric | 指紋/Face ID 解鎖 KeyVault |
| tauri-plugin-deep-link | `phantom-mesh://` 協定處理 |
| tauri-plugin-process | 重啟/退出控制 |

### 2.7 自動更新

- **更新源：** GitHub Releases 或 CrabNebula Cloud（CDN）
- **檢查頻率：** 每 6 小時
- **流程：** 靜默下載 → 通知用戶 → 確認 → 重啟更新
- **簽章：** Ed25519 必須驗證
- **回滾：** 保留前一版本，更新失敗自動回滾

---

## 3. Auto-Networking（三層自動組網）

### 3.1 架構

```
┌─────────────────────────────────────────────┐
│            NetworkManager (Plugin)           │
├─────────┬──────────────┬────────────────────┤
│ Layer 1 │   Layer 2    │     Layer 3        │
│  mDNS   │  Iroh/VPN    │   Relay            │
│ (LAN)   │  (跨網段)    │   (保底)           │
└────┬────┴──────┬───────┴────────┬───────────┘
  同網段 <1ms  跨網段 5-20ms   打洞失敗 50-100ms
```

### 3.2 Layer 1: mDNS / LAN 發現

```rust
#[async_trait]
pub trait ServiceDiscovery: Send + Sync {
    async fn announce(&self, node: &NodeInfo) -> Result<()>;
    async fn discover(&self, timeout: Duration) -> Result<Vec<NodeInfo>>;
    async fn stop(&self) -> Result<()>;
}
```

- 廣播 `_phantom-mesh._tcp.local`
- 同網段零配置自動發現
- 每 30 秒 heartbeat

### 3.3 Layer 2: Iroh（預設）/ Tailscale（可插拔替代）

**Iroh（推薦預設）：**

```rust
#[async_trait]
pub trait MeshTransport: Send + Sync {
    async fn connect(&self, peer: &PeerId) -> Result<Connection>;
    async fn peers(&self) -> Result<Vec<PeerInfo>>;
    async fn status(&self) -> Result<TransportStatus>;
}

pub struct IrohTransport { /* ... */ }
// QUIC 傳輸，~90% NAT 穿透成功率
// 內建 relay 服務，零配置
// 用加密 key 做節點身份（不是 IP）
```

**可插拔替代：**
- `TailscaleTransport` — 偵測本機 Tailscale，透過 Tailscale IP 連線
- `Libp2pTransport` — 完整 DHT + GossipSub（需要進階功能時）

### 3.4 Layer 3: Cloud Relay（保底）

- Iroh 內建 relay 服務（不需自建）
- 端對端加密（relay 無法讀取內容）
- 僅在 mDNS + Iroh 打洞都失敗時啟用

### 3.5 自動選路

```
NetworkManager.connect(target):
  1. mDNS 發現列表？→ 直連 LAN IP
  2. Iroh peer 列表？→ QUIC 直連/relay
  3. Tailscale peer？ → Tailscale IP
  4. 都沒有？→ Cloud Relay
  5. 連線建立後快取路由
  6. 定期探測，更快路由恢復時自動切換
```

### 3.6 新節點加入流程

```
首次啟動:
  1. mDNS 廣播 → 發現同網段 Hub → 自動註冊
  2. 沒發現？→ Iroh peer discovery → 找 Hub → 註冊
  3. 還是沒有？→ UI：輸入 Hub 地址 / 掃 QR Code
  4. 註冊成功 → Hub 下發加密金鑰 → 開始接任務
```

### 3.7 可插拔 trait

三層都是 trait 抽象，可替換為任何 P2P/VPN 方案。

---

## 4. Mobile App（React Native + Rust Core）

### 4.1 架構

```
React Native (UI)
      │
  Bridge Layer
  Phase 1: HTTP API (localhost)
  Phase 2: UniFFI (Rust → Kotlin/Swift)
      │
  phantom-mesh (精簡版)
  Main Agent + SubAgent (1-2個)
```

### 4.2 兩階段策略

| 階段 | 方式 | 說明 |
|------|------|------|
| Phase 1 | HTTP API | 內嵌 HTTP server，RN 透過 localhost 呼叫。複用 web_dashboard API |
| Phase 2 | UniFFI | uniffi-bindgen-react-native 產生 Turbo Module。效能更好 |

Bridge 層是可插拔 trait，切換無痛。

### 4.3 頁面設計

```
底部 Tab Bar: 首頁 | 任務 | 💬 | 集群 | 更多

首頁: 集群摘要、本機 Agent 狀態、今日收益、最近告警
任務: 執行中 Hand/Pipeline、手動觸發、排程、歷史
💬:   自然語言操作介面
集群: 節點列表、網路拓撲、Provider 狀態
更多: Provider 金鑰、頻道設定、技能市場、記憶、安全、設定、日誌
```

### 4.4 推播通知

任務完成/失敗、節點離線、預算告警、收益里程碑、安全事件、可用更新。

### 4.5 資源感知

- 電池 > 20% 才接任務
- WiFi 優先（行動數據可開關）
- 背景模式降低 SubAgent 數量
- 溫度過高自動暫停

### 4.6 自動更新

- JS 層：Expo OTA 即時推送
- Rust 層：需重新發版

### 4.7 與現有 App 關係

增量升級現有 `mobile/phantom-mesh-worker-app/`，不重寫。

---

## 5. Provider Auth（四層分級 + 加密金鑰管理）

### 5.1 四層 Provider 分級

| Tier | 名稱 | 成本 | 速度 | 品質 | 策略 |
|------|------|------|------|------|------|
| 1 | 本地 (Ollama, LM Studio) | 免費 | 依硬體 | 中 | 常態使用（響應 < 500ms 時） |
| 2 | 免費 API (Gemini Free, Groq Free) | 免費 | 快 | 低 | 量大使用 |
| 3 | 訂閱制 (Claude Max, ChatGPT Pro) | 已付月費 | 快 | 高 | 截止線前平均分配 |
| 4 | 按量付費 (Anthropic API, OpenAI API) | 按 token | 快 | 高 | 非不得已不用 |

### 5.2 本地 LLM 動態排序

```
每 10 分鐘探測延遲:
  < 500ms  → Tier 1（排在免費 API 前）
  500ms-3s → 跟免費 API 並列
  > 3s     → 排在免費 API 後
```

### 5.3 訂閱制配額平均分配

```rust
pub struct SubscriptionPacer {
    pub total_quota: u64,
    pub used_quota: u64,
    pub reset_at: DateTime<Utc>,
}

impl SubscriptionPacer {
    /// 計算今日可用額度 — 剩餘額度 / 剩餘天數
    pub fn daily_allowance(&self) -> u64 {
        let remaining = self.total_quota.saturating_sub(self.used_quota);
        let days_left = days_until(self.reset_at).max(1);
        remaining / days_left
    }

    /// 今日是否還有餘額
    pub fn can_use_today(&self, used_today: u64) -> bool {
        used_today < self.daily_allowance()
    }
}
```

當日額度用完 → 切到其他 Tier。昨天少用的額度動態補回。

### 5.4 路由優先級（動態）

```
本地快: 本地 → 免費 → 訂閱(當日額) → 按量
本地中: 免費 ↔ 本地 → 訂閱(當日額) → 按量
本地慢: 免費 → 本地 → 訂閱(當日額) → 按量
```

### 5.5 可插拔 LLM Gateway

**TensorZero**（Rust 原生，<1ms P99 延遲）作為可插拔 Provider 路由實作之一：
- 自動 Fallback
- A/B 測試
- Bandit 算法自動路由最佳 model variant

### 5.6 加密金鑰管理

```
主密碼 → Argon2id 衍生 → AES-256-GCM Master Key
  → 每個 API key 獨立加密 → SQLite encrypted blob

解鎖方式:
  桌面: 主密碼 或 生物辨識 (tauri-plugin-biometric)
  手機: 指紋 / Face ID
```

### 5.7 集群金鑰同步

```
Hub → Worker 單向分發:
  1. X25519 key exchange 產生 session key
  2. 用 session key 加密 API keys（envelope encryption）
  3. TLS 傳輸（第二層加密）
  4. 依 Worker 能力分配不同 key

撤銷:
  Hub 輪替 key → 推送新 key
  Worker 離線 > 24h → 標記過期
  Worker 移除 → 清除本地金鑰
```

### 5.8 權限控制

```rust
pub struct KeyPermission {
    pub provider: String,
    pub allowed_models: Vec<String>,
    pub daily_budget_usd: f64,
    pub allowed_nodes: Vec<String>,
}
```

### 5.9 可插拔 trait

```rust
#[async_trait]
pub trait KeyStore: Send + Sync {
    async fn store_key(&self, provider: &str, name: &str, key: &str) -> Result<()>;
    async fn get_key(&self, provider: &str, name: &str) -> Result<String>;
    async fn list_keys(&self, provider: &str) -> Result<Vec<KeyMeta>>;
    async fn delete_key(&self, provider: &str, name: &str) -> Result<()>;
    async fn test_key(&self, provider: &str, key: &str) -> Result<KeyTestResult>;
}
// 預設: LocalKeyVault | 可替換: SystemKeychain, HashiCorpVault, AwsKms
```

---

## 6. SubAgent 模型（動態伸縮）

### 6.1 節點模型

每台機器：1 Main Agent（常駐）+ N SubAgent（按需建立、任務完成即銷毀）。

### 6.2 Main Agent vs SubAgent

| | Main Agent | SubAgent |
|---|-----------|----------|
| 生命週期 | App 啟動 → 關閉 | 任務建立 → 完成銷毀 |
| 職責 | 管理、排程、監控 | 單一任務執行 |
| 狀態 | 持有全局狀態 | 只拿任務所需上下文 |
| 數量 | 固定 1 | 0 ~ N，動態 |

### 6.3 資源感知伸縮

```rust
pub struct ScalingPolicy {
    pub max_subagents: usize,
    pub cpu_threshold: f32,     // 0.80
    pub ram_threshold: f32,     // 0.85
    pub battery_min: f32,       // 0.20 (手機)
    pub cooldown_secs: u64,     // 30
}
```

| 裝置 | max_subagents |
|------|---------------|
| 桌面高效能 | 8 |
| 桌面一般 | 4 |
| 手機前景 | 2 |
| 手機背景 | 1 |
| Linux Server | 16 |

### 6.4 SubAgent 生命週期

1. Main Agent 決定建立 → 分配資源配額
2. 注入上下文（Hand 定義 + 相關記憶 + Provider 金鑰）
3. 獨立執行任務（工具呼叫、多輪對話、受 guardrail 約束）
4. 完成 → TaskReport 回報 Main Agent
5. Main Agent 記錄 trajectory + 精煉記憶 + 更新經濟數據
6. SubAgent 銷毀，釋放資源

### 6.5 可插拔 trait

```rust
#[async_trait]
pub trait SubAgentRunner: Send + Sync {
    async fn spawn(&self, task: AgentTask, ctx: SubAgentContext) -> Result<SubAgentHandle>;
    async fn list_running(&self) -> Vec<SubAgentStatus>;
    async fn kill(&self, id: &str) -> Result<()>;
    fn capacity(&self) -> SubAgentCapacity;
}
// 預設: LocalSubAgentRunner | 可替換: DockerRunner, WasmRunner
```

### 6.6 與現有模組整合

- `agent_runtime.rs` → SubAgent 共用工具呼叫邏輯
- `task_queue.rs` → Main Agent 管理本地 queue
- `task_preemption.rs` → 高優先級搶佔
- `node_scoring.rs` → SubAgent 空閒數作為評分因子
- `watchdog.rs` → 監控 SubAgent 健康

---

## 7. Evolution Layer（自動進化系統）

### 7.1 三種可進化單元

| 單元 | 定義 | 熱更新 |
|------|------|--------|
| Skill | 單一能力/知識包（TOML + prompt） | ✅ 即時載入 |
| Plugin | 功能模組（WASM via Extism） | ✅ 重新載入 |
| Hand | 多階段工作流程（hand.toml） | ✅ 即時載入 |

### 7.2 Plugin 執行引擎：Extism（基於 Wasmtime）

```
Extism 優勢（vs 原生 Wasmtime）:
├── 簡化 Host Function 綁定
├── 持久化記憶體（Plugin 重載不丟狀態）
├── HTTP 存取控制
├── 執行時間/資源限制
├── 多語言 PDK（Rust/Go/JS/Python 都能寫 Plugin）
└── 熱重載支援
```

### 7.3 Plugin 分發：OCI Registry

```
分發渠道:
├── 官方: GitHub Container Registry / Docker Hub
├── 自架: Harbor / 任何 OCI 相容 registry
└── 離線: 本地 .wasm 檔案

流程: oras pull → 驗證 SHA-256 + Ed25519 簽章 → 解壓 → 熱載入
```

### 7.4 自動更新

```rust
pub struct EvolutionConfig {
    pub auto_check_interval_secs: u64,  // 21600 (6h)
    pub auto_install_minor: bool,        // true
    pub auto_install_major: bool,        // false (需確認)
    pub registries: Vec<String>,
}
```

### 7.5 自動安裝技能（Auto Skill Install）

```rust
/// 任務需要某能力但本機缺少時，自動搜尋+安裝
pub struct AutoSkillInstaller {
    registry: Arc<dyn PackageRegistry>,
    config: AutoInstallConfig,
}

pub struct AutoInstallConfig {
    pub enabled: bool,              // 預設 true
    pub auto_install_verified: bool, // 官方/驗證過的 → 自動安裝
    pub auto_install_community: bool,// 社群來源 → 預設需確認
    pub max_auto_installs_per_day: u32, // 10
}

impl AutoSkillInstaller {
    /// Main Agent 在分配任務前呼叫
    pub async fn ensure_capability(
        &self,
        required: &str,       // e.g. "image_generation"
        node_caps: &[String], // 本機已有能力
    ) -> Result<CapabilityResult> {
        if node_caps.contains(&required.to_string()) {
            return Ok(CapabilityResult::AlreadyInstalled);
        }

        // 搜尋 registry
        let candidates = self.registry.fetch_index().await?
            .search_by_capability(required);

        if candidates.is_empty() {
            return Ok(CapabilityResult::NotAvailable);
        }

        let best = candidates.first().unwrap();

        // 驗證來源決定是否自動安裝
        if best.verified && self.config.auto_install_verified {
            self.install(best).await?;
            Ok(CapabilityResult::AutoInstalled(best.id.clone()))
        } else {
            Ok(CapabilityResult::NeedsApproval(best.clone()))
        }
    }
}

pub enum CapabilityResult {
    AlreadyInstalled,
    AutoInstalled(String),
    NeedsApproval(PackageInfo),
    NotAvailable,
}
```

### 7.6 架構自動調適（Auto Architecture Adaptation）

```rust
/// 根據歷史執行數據，自動調整系統配置
pub struct ArchitectureAdaptor {
    resource_monitor: Arc<ResourceMonitor>,
    roi_scheduler: Arc<RoiScheduler>,
    config: AdaptationConfig,
}

pub struct AdaptationConfig {
    pub analysis_interval_secs: u64,  // 3600 (1 小時)
    pub auto_apply_safe: bool,        // Safe 等級自動套用
    pub auto_apply_normal: bool,      // Normal 等級需確認（預設 false）
}

pub enum Adaptation {
    // Safe — 自動套用
    AdjustScaling { node: NodeId, new_max: usize, reason: String },
    ReorderProviderTier { tier: u8, new_order: Vec<String>, reason: String },

    // Normal — 需確認
    RebalanceTasks { from: NodeId, to: Vec<NodeId>, reason: String },
    InstallCapability { name: String, reason: String },
    SwitchClusterProfile { from: String, to: String, reason: String },

    // Dangerous — 必須確認
    RemoveNode { node: NodeId, reason: String },
    DisableProvider { provider: String, reason: String },
}

impl Adaptation {
    pub fn risk(&self) -> OperationRisk {
        match self {
            Self::AdjustScaling { .. } | Self::ReorderProviderTier { .. } => OperationRisk::Safe,
            Self::RebalanceTasks { .. } | Self::InstallCapability { .. }
            | Self::SwitchClusterProfile { .. } => OperationRisk::Normal,
            Self::RemoveNode { .. } | Self::DisableProvider { .. } => OperationRisk::Dangerous,
        }
    }
}
```

**自動調適觸發範例：**

| 偵測到的模式 | 調適動作 | 等級 |
|-------------|---------|------|
| SubAgent 經常觸達上限 | AdjustScaling +2 | Safe |
| 本地 LLM 延遲升高 | ReorderProviderTier | Safe |
| 節點 A 忙、B 閒 | RebalanceTasks | Normal |
| 任務需要缺少的能力 | InstallCapability | Normal |
| 任務類型改變 | SwitchClusterProfile | Normal |
| 節點持續離線 | RemoveNode | Dangerous |
| Provider 持續失敗 | DisableProvider（`CircuitBreaker` 已做） | Dangerous |

### 7.7 集群同步

Hub 安裝新 Skill/Plugin/Hand → 廣播 `SkillSync(SkillManifest)` → Worker 檢查需求 → 按需下載安裝。

### 7.8 安全

- SHA-256 checksum + Ed25519 簽章
- WASM 沙箱（Extism/Wasmtime，無法存取檔案系統）
- 社群來源標記「未驗證」
- 可設定白名單

### 7.9 Plugin 介面標準：MCP

Plugin 暴露功能透過 MCP 介面，與 AI agent 生態系統互通。

### 7.10 可插拔 trait

```rust
#[async_trait]
pub trait PackageRegistry: Send + Sync {
    async fn fetch_index(&self) -> Result<RegistryIndex>;
    async fn download(&self, id: &str, version: &str) -> Result<Vec<u8>>;
    async fn verify(&self, data: &[u8], checksum: &str) -> Result<bool>;
}
// 預設: OciRegistry | 可替換: HttpRegistry, LocalRegistry, IpfsRegistry
```

---

## 8. 自然語言介面（NL Interface）

### 8.1 架構

```
Chat UI → NL Interpreter → phantom-mesh 所有模組
           ├── Rule Engine（快速，90% 指令）
           └── LLM Fallback（複雜/模糊指令）
```

### 8.2 意圖分類

| 類別 | 範例 | 動作 |
|------|------|------|
| 查詢狀態 | 「集群狀態如何？」 | 讀取模組數據 |
| 執行任務 | 「執行 freelancer hand」 | 觸發 Hand/Pipeline |
| 管理操作 | 「暫停節點 B」 | 集群管理 API |
| 設定變更 | 「每日預算改成 $3」 | 更新設定 |
| 知識問答 | 「freelancer hand 做什麼的？」 | 查記憶+文件 |
| 複合指令 | 「檢查所有節點，有問題的暫停」 | 多步驟 |

### 8.3 ParsedIntent 定義

```rust
/// NL 解析後的結構化意圖
pub struct ParsedIntent {
    pub category: IntentCategory,
    pub action: String,           // e.g. "pause_node", "run_hand", "query_status"
    pub targets: Vec<String>,     // e.g. ["node-b"], ["freelancer"]
    pub params: HashMap<String, serde_json::Value>,
    pub risk: OperationRisk,
    pub confidence: f32,          // 0.0-1.0
    pub source: ParseSource,      // RuleEngine | LlmFallback
}

pub enum IntentCategory {
    QueryStatus,
    ExecuteTask,
    ManageCluster,
    ChangeConfig,
    KnowledgeQuery,
    Composite(Vec<ParsedIntent>), // 複合指令拆分
}

pub enum ParseSource { RuleEngine, LlmFallback }
```

### 8.4 兩層解析

1. **Rule Engine**（正則 + 關鍵字，支援中英文）— 零延遲、零成本
2. **LLM Fallback**（本地 Ollama 優先）— 處理複雜/模糊語句

#### NlRule 定義與 Rule Engine 具體規則範例

```rust
/// NL 操作風險等級（與 audit_log.rs 的 RiskLevel 不同，此為操作語義層級）
/// audit_log::RiskLevel 用於審計日誌（Low/Medium/High/Critical）
/// OperationRisk 用於 NL 意圖判斷（Safe/Normal/Dangerous）
pub enum OperationRisk {
    Safe,       // 查詢類 → 直接執行
    Normal,     // 執行任務 → 執行+回報
    Dangerous,  // 刪除/停止/修改 → 需用戶確認
}

/// 自然語言規則定義
pub struct NlRule {
    pub pattern: &'static str,       // 正則表達式（編譯為 Regex）
    pub intent: &'static str,        // 意圖動作名稱
    pub target_group: Option<usize>, // 正則 capture group 索引（提取目標）
    pub category: IntentCategoryKind,// 簡單枚舉（不含 Composite）
    pub risk: OperationRisk,
}

/// 規則層面的意圖分類（不含 Composite，可用於靜態定義）
#[derive(Clone, Copy)]
pub enum IntentCategoryKind {
    QueryStatus, ExecuteTask, ManageCluster, ChangeConfig, KnowledgeQuery,
}

/// 內建規則（中英文雙語）— 用 lazy_static 初始化編譯後的 Regex
static RULES: Lazy<Vec<CompiledRule>> = Lazy::new(|| {
    vec![
        // 查詢狀態
        rule(r"(?i)(集群|cluster)\s*(狀態|status|怎麼樣|如何)",
             "query_status", None, IntentCategoryKind::QueryStatus, OperationRisk::Safe),
        // 執行 Hand
        rule(r"(?i)(執行|run|跑|start)\s+(\w+)\s*(hand)?",
             "run_hand", Some(2), IntentCategoryKind::ExecuteTask, OperationRisk::Normal),
        // 暫停節點
        rule(r"(?i)(暫停|pause|停止|stop)\s*(節點|node)\s*(\w+)",
             "pause_node", Some(3), IntentCategoryKind::ManageCluster, OperationRisk::Dangerous),
        // 修改預算
        rule(r"(?i)(預算|budget)\s*(改|set|調)\s*\$?(\d+\.?\d*)",
             "set_budget", Some(3), IntentCategoryKind::ChangeConfig, OperationRisk::Dangerous),
        // 查看收益
        rule(r"(?i)(今[日天]|today|收益|revenue|賺了|earned)",
             "query_revenue", None, IntentCategoryKind::QueryStatus, OperationRisk::Safe),
    ]
});
```

> **設計決策：**
> - `OperationRisk` 與 `audit_log.rs` 的 `RiskLevel`（Low/Medium/High/Critical）分開命名，避免衝突。
> - `IntentCategoryKind`（無 Composite）用於規則靜態定義；`IntentCategory`（含 Composite）用於解析後的 `ParsedIntent`。
> - 規則用 `Lazy<Vec<CompiledRule>>` 而非 `const`，因 `Regex` 需要堆分配。

### 8.5 Invariant Check（形式化驗證）

LLM 產生的系統指令必須通過結構化驗證才執行：
- 節點存在性檢查
- 資源充足性檢查
- 正在執行的任務衝突檢查
- 驗證失敗 → 回覆原因並提供選項

### 8.6 對話記憶

使用 `ConversationStore` 維持上下文 + `ObservationalMemory` 壓縮舊對話。

### 8.7 安全模型

```
NL 輸入安全管線:
  1. InjectionGuard.check(input)  ← 複用既有 injection_guard.rs
     → Suspicious(High) → 攔截，回覆警告
     → Suspicious(Medium) → 標記，仍解析但加審計日誌
     → Safe → 繼續
  2. Rule Engine / LLM 解析
  3. ParsedIntent → Invariant Check（8.5 節）
  4. OperationRisk::Dangerous → 用戶確認
  5. 執行
```

> **複用：** `InjectionGuard` 已有 18 種偵測模式（`injection_guard.rs`），
> 涵蓋 SystemOverride、RoleManipulation、EncodingBypass、DataExfiltration、
> Jailbreak、MarkupInjection、MultiLang、FinancialManipulation、DangerousInstruction。
> NL 介面直接複用，不另建。

### 8.8 可插拔 trait

```rust
#[async_trait]
pub trait NlParser: Send + Sync {
    async fn parse(&self, input: &str, context: &ChatContext) -> Result<Vec<ParsedIntent>>;
}
// 預設: RuleFirstParser（Rule Engine + LLM Fallback）
// 可替換: LlmOnlyParser, CustomParser

#[async_trait]
pub trait IntentExecutor: Send + Sync {
    async fn execute(&self, intent: &ParsedIntent, ctx: &AppContext) -> Result<ExecutionResult>;
}
// 預設: DefaultIntentExecutor

#[async_trait]
pub trait ResponseGenerator: Send + Sync {
    async fn generate(&self, result: &ExecutionResult, lang: &str) -> Result<String>;
}
// 預設: TemplateResponseGenerator | 可替換: LlmResponseGenerator
```

---

## 9. 資源監控（CPU / RAM / GPU / NPU / Custom）

### 9.1 可插拔計算單元

```rust
#[async_trait]
pub trait ComputeUnit: Send + Sync {
    fn unit_type(&self) -> ComputeType;
    fn name(&self) -> &str;
    async fn usage(&self) -> Result<UsageSnapshot>;
    fn capabilities(&self) -> Vec<String>;
}

pub enum ComputeType {
    Cpu, Gpu, Npu, Ram, Vram,
    Custom(String), // 未來: FPGA, TPU, 專用 AI 加速卡
}

pub struct UsageSnapshot {
    pub utilization_pct: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub temperature_c: Option<f32>,
    pub power_watts: Option<f32>,
    pub clock_mhz: Option<u32>,
}
```

### 9.2 自動偵測

啟動時掃描：CPU（sysinfo）、RAM、GPU（nvml/rocm-smi/Metal）、NPU（openvino）、VRAM。透過 Plugin 註冊新計算單元。

### 9.3 ResourceMonitor 聚合器

```rust
/// 聚合所有 ComputeUnit 的系統級資源監控
pub struct ResourceMonitor {
    units: Vec<Arc<dyn ComputeUnit>>,
    poll_interval: Duration,
}

impl ResourceMonitor {
    pub fn new(poll_interval: Duration) -> Self {
        Self { units: vec![], poll_interval }
    }

    /// 啟動時自動偵測並註冊所有可用計算單元
    pub async fn auto_detect(&mut self) -> Result<()> {
        // CPU (sysinfo) — 必定存在
        self.units.push(Arc::new(CpuUnit::detect()?));
        // RAM — 必定存在
        self.units.push(Arc::new(RamUnit::detect()?));
        // GPU — 選擇性 (nvml / rocm-smi / Metal)
        if let Ok(gpu) = GpuUnit::detect() { self.units.push(Arc::new(gpu)); }
        // NPU — 選擇性 (openvino)
        if let Ok(npu) = NpuUnit::detect() { self.units.push(Arc::new(npu)); }
        Ok(())
    }

    /// 註冊自訂計算單元（Plugin 擴展用）
    pub fn register(&mut self, unit: Arc<dyn ComputeUnit>) {
        self.units.push(unit);
    }

    /// 取得所有單元的即時快照
    pub async fn snapshot(&self) -> Vec<(ComputeType, UsageSnapshot)> { /* ... */ }

    /// 判斷是否超過伸縮閾值
    pub async fn exceeds_threshold(&self, policy: &ScalingPolicy) -> bool { /* ... */ }
}
```

### 9.4 對任務路由影響

需要 GPU 的任務 → 找有 GPU 的節點；VRAM 不夠 → 用外部 API；可拆分 → 量化模型降需求。

---

## 10. 集群交互（五層可插拔）

### 10.1 五層堆疊

```
Layer 5: Strategy — 任務怎麼分配？
  BroadcastOffer | DirectAssign | AuctionBased | PipelineChain

Layer 4: Coordination — 誰做決策？
  SingleCoordinator | ConsensusGroup | Autonomous | Hierarchical

Layer 3: Routing — 任務送到哪？
  ScoreBased | CostAware | LatencyFirst | AffinityBased

Layer 2: Sync — 狀態怎麼同步？
  HeartbeatPull | EventPush | CrdtMerge | GossipProtocol

Layer 1: Transport — 怎麼傳資料？
  HTTP REST | WebSocket | QUIC (Iroh) | gRPC
```

### 10.2 Profile 組合

```rust
pub struct ClusterProfile {
    pub name: String,
    pub transport: Arc<dyn ClusterTransport>,
    pub sync: Arc<dyn StateSync>,
    pub routing: Arc<dyn TaskRouter>,
    pub coordination: Arc<dyn CoordinationMode>,
    pub strategy: Arc<dyn DispatchStrategy>,
}
```

可按任務類型動態選 profile（收益類 → CostAware + Auction，品質類 → ScoreBased + DirectAssign，流水線 → PipelineChain + Coordinator）。

### 10.3 協調者選舉（去單點化）

Coordinator 心跳超時 30 秒 → Bully Algorithm 選舉 → 最高分節點成為新 Coordinator。

```
選舉分數 = uptime_hours * 0.3
         + (1.0 - cpu_usage) * 0.2
         + available_ram_gb * 0.2
         + gpu_count * 0.15
         + success_rate * 0.15
```

行動裝置（DeviceType::Mobile）不參與選舉。分數相同取 NodeId 字典序最大。

### 10.4 訊息類型

```rust
pub enum ClusterMessage {
    NodeAnnounce(NodeInfo),
    NodeGoodbye(NodeId),
    Heartbeat(HeartbeatData),
    TaskOffer(TaskDescriptor),
    TaskAccept(TaskId, NodeId),
    TaskResult(TaskId, TaskOutput),
    TaskCancel(TaskId),
    ResourceQuery(ResourceFilter),
    ResourceReport(NodeResources),
    KeySync(EncryptedKeyBundle),
    SkillSync(SkillManifest),
    ConfigSync(ConfigDelta),
    ElectCoordinator(ElectionBallot),
    CoordinatorAnnounce(NodeId),
}
```

### 10.5 SubAgent 通訊用 A2A，工具整合用 MCP

- A2A: Agent 之間發現能力 + 協商任務
- MCP: Agent 呼叫外部工具/服務
- Plugin 暴露功能也用 MCP（生態互通）

### 10.6 向下相容

保留現有 HTTP API（register, heartbeat, poll, result），新節點用 WebSocket/QUIC mesh。Coordinator 橋接兩種協定。

---

## 11. 記憶體架構（Memory Architecture）

### 11.1 三層記憶體

| 層級 | 說明 | 生命週期 |
|------|------|---------|
| Cluster Memory | 全集群共享知識 | 持久 |
| Node Memory | Main Agent 持有（Semantic/Episodic/Procedural/Observational） | 持久 |
| SubAgent Memory | Working Memory + Task Output | 任務結束後精煉合併 |

### 11.2 SubAgent → Node Memory 合併

SubAgent 完成 → 產生 TaskReport → Main Agent 精煉合併：

| 來源 | 目標 | 說明 |
|------|------|------|
| 擷取的知識 | Semantic Memory | KnowledgeCapturer 提取 problem/decision/result/lesson |
| 多輪對話 | Observational Memory | ObservationalMemory 壓縮（3-40x） |
| 執行結果 | Episodic Memory | 時間+任務+結果+品質+成本 |
| 失敗教訓 | Procedural Memory | 錯誤原因+避免方法 |
| Working Memory | 丟棄 | 中間步驟不保留 |

```rust
#[async_trait]
pub trait MemoryMerger: Send + Sync {
    async fn merge(&self, report: &TaskReport, node_memory: &MemoryStore) -> Result<MergeResult>;
}
// 預設: DefaultMemoryMerger | 可替換: LlmMemoryMerger
```

### 11.3 節點間記憶同步

| 場景 | 策略 | 說明 |
|------|------|------|
| 桌面 ↔ 桌面 | Full | 完整雙向同步 |
| 桌面 → 手機 | Selective | 只推 Semantic + Procedural |
| 手機 → 桌面 | Selective | 知識上傳回主節點 |
| 新節點 | ReadOnly | 先唯讀，信任建立後升級 |

### 11.4 衝突解決

**策略：Vector Clock + Keep-Both（非 LWW）**

```
衝突偵測: Vector clock 判斷因果關係
  → 一方先於另一方 → 保留較新的（因果序，非時間戳）
  → 並行修改（互相無因果） → 兩條都保留，標記來源節點
  → 定期由 Coordinator 觸發記憶整理（合併重複、刪除過期）
```

> **釐清：** 不使用 Last-Write-Wins（LWW 在分散式系統中可能丟失知識）。
> 並行衝突一律 keep-both，避免靜默覆蓋。整理合併由 `MemoryCompactor` 處理。

### 11.5 SubAgent 啟動時記憶注入

Main Agent 從 Node Memory `recall_tiered()` 查詢相關記憶，注入 SubAgent 上下文。優先 Semantic + Procedural tier。

### 11.6 可插拔 trait

```rust
#[async_trait]
pub trait MemorySync: Send + Sync {
    async fn push(&self, entries: &[MemoryEntry], target: &NodeId) -> Result<()>;
    async fn pull(&self, source: &NodeId, since: DateTime<Utc>) -> Result<Vec<MemoryEntry>>;
    async fn resolve_conflicts(&self, local: &MemoryEntry, remote: &MemoryEntry) -> MergeDecision;
}
// 預設: CrdtMemorySync | 可替換: CoordinatorSync

#[async_trait]
pub trait MemoryCompactor: Send + Sync {
    async fn compact(&self, store: &MemoryStore) -> Result<CompactionReport>;
}
// 預設: ObservationalCompactor | 可替換: LlmCompactor
```

---

## 12. 錯誤處理策略（Error Handling）

### 12.1 統一錯誤型別

```rust
/// 系統級錯誤 — 所有模組使用 thiserror 衍生
#[derive(Debug, thiserror::Error)]
pub enum PhantomMeshError {
    #[error("Plugin {0}: {1}")]
    Plugin(String, String),
    #[error("Network: {0}")]
    Network(#[from] NetworkError),
    #[error("Provider {provider}: {message}")]
    Provider { provider: String, message: String },
    #[error("Storage: {0}")]
    Storage(#[from] StorageError),
    #[error("Auth: {0}")]
    Auth(String),
    #[error("Agent: {0}")]
    Agent(String),
    #[error("Config: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, PhantomMeshError>;
```

### 12.2 分層恢復策略

| 層級 | 策略 | 範例 |
|------|------|------|
| Provider 失敗 | CircuitBreaker → 自動降級到下一 Tier | Anthropic 502 → Ollama |
| 網路斷線 | Layer 自動切換 + 本地佇列 | Iroh 斷 → mDNS / Relay |
| SubAgent 崩潰 | Watchdog 偵測 → Main Agent 重新派工 | Panic → 新 SubAgent |
| Plugin crash | WASM 沙箱隔離 → 卸載 + 告警 | OOM → 重載 |
| 節點離線 | Coordinator 重新分配任務 | 心跳超時 30s |
| 資料庫鎖 | 重試 3 次 + exponential backoff | SQLite BUSY |

### 12.3 離線模式

```
網路完全斷線時:
  1. 本地 LLM 繼續運作（Tier 1）
  2. 任務結果暫存本地佇列
  3. 網路恢復後自動同步（TaskSync + MemorySync）
  4. 集群狀態標記為 Degraded（非 Unhealthy）
  5. 離線超過 24h → KeyVault 金鑰標記過期
```

---

## 13. 測試策略（Testing Strategy）

### 13.1 測試金字塔

| 層級 | 對象 | 工具 | 覆蓋率目標 |
|------|------|------|-----------|
| 單元測試 | Trait impl, 純函數 | `#[test]`, `#[tokio::test]` | 80% |
| 整合測試 | 模組互動, Plugin Bus | `tests/` 目錄 | 關鍵路徑 |
| 端對端測試 | CLI 指令, API 端點 | `assert_cmd`, 自訂 test harness | 核心流程 |
| 屬性測試 | 序列化/衝突解決 | `proptest` | 邊界條件 |
| 壓力測試 | SubAgent 伸縮, 網路 | `criterion`, `tokio-console` | 效能底線 |

### 13.2 各子系統測試重點

| 子系統 | 測試重點 |
|--------|---------|
| Plugin Bus | init 順序、shutdown 回滾、事件匯流排 |
| Networking | mDNS 發現、NAT 穿透模擬、Layer 切換 |
| Provider Tier | 動態排序、配額分配、降級觸發 |
| SubAgent | spawn/kill 生命週期、資源限制觸發伸縮 |
| NL Interface | Rule Engine 匹配（中英文）、InjectionGuard 攔截 |
| Memory | 合併正確性、衝突解決、跨節點同步 |
| Cluster | Coordinator 選舉、Profile 切換、向下相容 |
| Auto-Install | 能力搜尋、驗證+安裝流程、日限制 |

### 13.3 模擬框架

```rust
/// 可插拔 trait 自然支援 mock 測試
struct MockTransport { /* ... */ }
impl MeshTransport for MockTransport { /* ... */ }

struct MockProvider { /* ... */ }
impl Provider for MockProvider { /* ... */ }

// 既有模式：tests/engine_integration.rs 已驗證
// OptimizerStore + Governor + RoiGate + FeedbackLoop 閉環
```

---

## 附錄 A：技術棧總覽

| 領域 | 技術選擇 | 備選 |
|------|---------|------|
| 桌面框架 | Tauri v2 + React | — |
| 手機框架 | React Native (Expo) | — |
| Rust↔Mobile | UniFFI (Phase 2) | HTTP API (Phase 1) |
| P2P 網路 | Iroh (QUIC) | libp2p, Tailscale/Headscale |
| Plugin 引擎 | Extism (Wasmtime) | 原生 Wasmtime |
| Plugin 分發 | OCI Registry | HTTP + registry.json |
| LLM Gateway | ProviderRouter + TensorZero (可選) | — |
| 加密 | Argon2id + AES-256-GCM | — |
| 金鑰交換 | X25519 | — |
| Agent 通訊 | A2A Protocol | — |
| 工具整合 | MCP Protocol | — |
| 記憶儲存 | SQLite + Ollama embeddings | pgvector |
| 記憶同步 | CRDT | Coordinator-based |
| 資源監控 | sysinfo + nvml + openvino | — |

## 附錄 B：設計順序、里程碑與子系統拆分

> **前提：** 所有階段均建立在現有 `phantom-mesh`（108 模組、195+ .rs 檔、~125K LOC）之上，
> 不另起新專案。Desktop App 為新建同層目錄 `phantom-mesh-desktop/`，其餘為擴展既有模組。

本設計涵蓋 6 個獨立子系統，建議按此順序實作：

### B.1 階段 1：Plugin Bus + AppContext

**基礎架構層，所有後續模組的載體。擴展現有 `plugin_loader.rs` + `PluginRegistry`。**

| 里程碑 | 驗收標準 |
|--------|---------|
| M1.1 AppContext 可註冊/取得服務 | 單元測試通過：`register<T>` + `get<T>` 往返 |
| M1.2 PluginModule trait + PluginBus | 至少 3 個既有模組改寫為 PluginModule，`init_all()` 按 7-Phase 順序啟動 |
| M1.3 事件匯流排 | 模組間透過 `PluginEvent` 通訊，`emit()` → `subscribe()` 可接收 |
| M1.4 shutdown 回滾 | 模擬中途失敗，已初始化模組正確 `shutdown()` |
| **階段完成** | `cargo test` 全過，daemon 可透過 PluginBus 正常啟動 |

### B.2 階段 2：Desktop App（Tauri v2 + React）

**核心載體。新建 `phantom-mesh-desktop/`，`phantom-mesh = { path = "../" }`，複用 `web_dashboard.rs` API。**

| 里程碑 | 驗收標準 |
|--------|---------|
| M2.1 Tauri 骨架 | `phantom-mesh-desktop/` 建立，`cargo tauri dev` 可開啟空視窗 |
| M2.2 phantom-mesh 嵌入 | Tauri 後端直接 import phantom-mesh，daemon 在 App 內啟動 |
| M2.3 基本 UI（Dashboard + Chat） | React 前端載入，側邊欄導航，Dashboard 顯示即時狀態 |
| M2.4 完整 16 頁面 | 所有頁面可導航，`#[tauri::command]` 對接 phantom-mesh 各模組 |
| M2.5 系統托盤 + 自動更新 | 最小化到托盤、右鍵選單、Ed25519 簽章更新流程 |
| **階段完成** | Windows 可安裝執行，全部功能透過 GUI 可操作 |

### B.3 階段 3：Auto-Networking（三層組網）

**擴展現有 `cluster_hub.rs` / `cluster_worker.rs`，保留 HTTP API 向下相容。**

| 里程碑 | 驗收標準 |
|--------|---------|
| M3.1 mDNS 發現 | 同網段兩台機器自動發現對方，< 5 秒 |
| M3.2 Iroh 連線 | 跨網段兩節點透過 Iroh QUIC 建立連線，NAT 穿透 |
| M3.3 自動選路 | NetworkManager 按 LAN → Iroh → Relay 順序嘗試，連線成功快取路由 |
| M3.4 新節點自動加入 | 啟動 App → mDNS/Iroh 找到 Hub → 自動註冊 → 開始接任務 |
| M3.5 Tailscale 可插拔替代 | `TailscaleTransport` 實作 `MeshTransport` trait，可切換 |
| **階段完成** | 2+ 節點跨網段自動組網、任務可跨節點分派 |

### B.4 階段 4：Provider Auth + Tier Router

**擴展現有 `ProviderRouter` + `provider_budget.rs` + `circuit_breaker.rs`。**

| 里程碑 | 驗收標準 |
|--------|---------|
| M4.1 KeyVault 加密存取 | API Key 以 AES-256-GCM 加密存入 SQLite，生物辨識解鎖 |
| M4.2 四層分級路由 | 本地/免費/訂閱/按量四 tier，動態排序運作 |
| M4.3 SubscriptionPacer | 訂閱制截止線前平均分配，每日額度動態計算 |
| M4.4 本地 LLM 延遲排序 | 每 10 分鐘探測，< 500ms 排 Tier 1，> 3s 降到免費 API 後 |
| M4.5 集群金鑰同步 | Hub → Worker X25519 key exchange + 加密分發 |
| **階段完成** | Provider 路由按四層分級自動選擇，金鑰加密同步到所有節點 |

### B.5 階段 5：Evolution Layer + Auto-Install

**擴展現有 `plugin_loader.rs`，加入 Extism + OCI + AutoInstall。**

| 里程碑 | 驗收標準 |
|--------|---------|
| M5.1 Extism Plugin 載入 | WASM Plugin 可載入執行，透過 MCP 暴露功能 |
| M5.2 OCI Registry 拉取 | `oras pull` + SHA-256 驗證 + Ed25519 簽章 → 熱載入 |
| M5.3 AutoSkillInstaller | 任務需要缺少能力 → 搜尋 registry → 驗證過的自動安裝 |
| M5.4 ArchitectureAdaptor | Safe 級調適自動套用（伸縮、Provider 排序），Normal/Dangerous 需確認 |
| M5.5 集群同步 | Hub 安裝新 Skill → 廣播 → Worker 自動下載安裝 |
| **階段完成** | 技能可自動安裝、架構可自動調適、集群同步更新 |

### B.6 階段 6：Mobile App

**增量升級現有 `mobile/phantom-mesh-worker-app/`，不重寫。**

| 里程碑 | 驗收標準 |
|--------|---------|
| M6.1 Phase 1 HTTP Bridge | React Native 透過 localhost HTTP API 呼叫內嵌 phantom-mesh |
| M6.2 5 頁面 UI | 首頁/任務/💬/集群/更多 Tab Bar，基本功能可用 |
| M6.3 Main Agent + SubAgent | 手機運行 1 Main Agent + 1-2 SubAgent，資源感知伸縮 |
| M6.4 NL 介面 | 聊天視窗輸入自然語言 → Rule Engine/LLM 解析 → 執行 |
| M6.5 推播通知 + 自動更新 | 任務完成/節點離線/預算告警推播，Expo OTA 更新 |
| M6.6 Phase 2 UniFFI（選配） | uniffi-bindgen-react-native 取代 HTTP，效能提升 |
| **階段完成** | 手機 App 可安裝，加入集群，接任務，自然語言操控 |

### B.7 跨階段功能嵌入時機

| 跨階段功能 | 階段 1 | 階段 2 | 階段 3 | 階段 4 | 階段 5 | 階段 6 |
|-----------|--------|--------|--------|--------|--------|--------|
| SubAgent 模型 | trait 定義 | UI 監控頁 | 跨節點分派 | — | 自動伸縮 | 手機版 |
| NL 介面 | — | Chat 頁面 | — | — | — | 手機聊天 |
| 記憶架構 | MemoryStore 插入 Bus | Memory 頁面 | 跨節點同步 | — | — | 精簡版 |
| 集群五層交互 | trait 定義 | 集群頁面 | Transport 層 | — | Profile 切換 | Mobile Profile |

每個子系統走獨立的 spec → plan → implementation 循環。

## 附錄 C：與現有程式碼的整合點

| 現有模組 | 整合方式 |
|---------|---------|
| `PluginRegistry` + `plugin_loader.rs` | 擴展為 Plugin Bus |
| `MemoryBackend` trait | 直接作為可插拔範例 |
| `Provider` trait (11+ 實作) | 加入 Tier 感知路由 |
| `ProviderRouter` | 加入四層分級 + TensorZero 可選 |
| `provider_budget.rs` | 擴展為四 tier 預算管理 |
| `circuit_breaker.rs` | 某 tier 斷路自動切下一 tier |
| `budget_downgrade.rs` | 擴展為 tier 自動降級 |
| `cluster_hub.rs` / `cluster_worker.rs` | 保留 HTTP API，加 WebSocket/QUIC mesh |
| `node_scoring.rs` | 加入 SubAgent 空閒數 + GPU/NPU 評分 |
| `agent_runtime.rs` | SubAgent 共用工具呼叫邏輯 |
| `task_preemption.rs` | 高優先級搶佔 SubAgent |
| `trajectory.rs` | SubAgent 執行記錄 |
| `a2a.rs` | SubAgent 間通訊 |
| `mcp.rs` | Plugin 暴露功能介面 |
| `web_dashboard.rs` | API 直接複用到 Tauri/Mobile |
| `worker_installer.rs` | 擴展為 App 安裝引導 |
| `multi_tenant.rs` | 租戶金鑰存入 KeyVault |
| `mobile/phantom-mesh-worker-app/` | 增量升級，不重寫 |
