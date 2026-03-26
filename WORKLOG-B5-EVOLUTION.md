# B.5 Evolution Layer + Auto-Install — 工作日誌

> 開始日期：2026-03-22
> Spec：`docs/superpowers/specs/2026-03-21-phantom-mesh-app-platform-design.md` §B.5
> Prompt：`../../PROMPT-B5.md`

---

## 里程碑進度

| # | 里程碑 | 狀態 | 完成時間 | 備註 |
|---|--------|------|----------|------|
| M5.1 | PackageRegistry + HTTP Registry | ✅ 完成 | 2026-03-22 | 21 tests pass |
| M5.2 | AutoSkillInstaller | ✅ 完成 | 2026-03-22 | 10 tests pass |
| M5.3 | ArchitectureAdaptor | ✅ 完成 | 2026-03-22 | 14 tests pass |
| M5.4 | Evolution Manager | ✅ 完成 | 2026-03-22 | 3 tests pass |
| M5.5 | Cluster Capability Sync | ✅ 完成 | 2026-03-22 | 7 tests pass |

---

## 工作記錄

### 2026-03-22

#### M5.1 PackageRegistry + HTTP Registry ✅
- [x] `src/evolution/mod.rs` — module declarations + re-exports
- [x] `src/evolution/registry.rs` — PackageRegistry trait + HttpRegistry + LocalRegistry
- [x] PackageRegistry trait: async fetch_index, download, verify
- [x] RegistryIndex: search_by_capability, search_by_name, get_package
- [x] HttpRegistry: JSON index fetch, package download, SHA-256 verify
- [x] LocalRegistry: filesystem-based offline mode
- [x] verify_sha256 utility function
- [x] 20 unit tests: SHA-256 verify, index search, local registry I/O
- [x] Export from lib.rs (`pub mod evolution;`)
- [x] `cargo check` ✅ + 20/20 tests pass

#### M5.2 AutoSkillInstaller ✅
- [x] `src/evolution/auto_installer.rs` — capability-driven auto-install
- [x] AutoInstallConfig: enabled, auto_install_verified, auto_install_community, max_per_day
- [x] CapabilityResult enum: AlreadyInstalled, AutoInstalled, NeedsApproval, NotAvailable
- [x] AutoSkillInstaller: ensure_capability, reset_daily, installs_today, installed_packages
- [x] Policy: verified auto-install, community gate, daily limit counter
- [x] 8 unit tests: already installed, auto-install verified/community, needs approval, not available, daily limit, disabled, reset
- [x] `cargo check` ✅ + 8/8 tests pass

#### M5.3 ArchitectureAdaptor ✅
- [x] `src/evolution/architecture_adaptor.rs` — auto-adjustment of system config
- [x] AdaptationRisk enum: Safe, Normal, Dangerous
- [x] Adaptation enum: 7 variants (AdjustScaling, ReorderProviderTier, RebalanceTasks, InstallCapability, SwitchClusterProfile, RemoveNode, DisableProvider)
- [x] SystemMetrics: provider latencies, failures, node task counts, success rates, missing capabilities, local latency trend
- [x] ArchitectureAdaptor: analyze (5 patterns), pending_approvals, approve/reject
- [x] Pattern detection: failure rate, latency trend, load imbalance, missing capabilities, high load scaling
- [x] Auto-apply Safe, queue Normal/Dangerous for approval
- [x] 10 unit tests: risk classification, each pattern, approve/reject, healthy system, auto-applied vs queued
- [x] `cargo check` ✅ + 10/10 tests pass

#### M5.4 Evolution Manager ✅
- [x] `src/evolution/manager.rs` — orchestrates all evolution subsystems
- [x] EvolutionConfig: auto_check_interval_secs, auto_install_minor/major, registries
- [x] EvolutionStatus: last_check, pending/applied adaptations, installed_today, enabled
- [x] EvolutionManager: evolution_cycle, ensure_capability, status, approve/reject_adaptation
- [x] Integration with AutoSkillInstaller + ArchitectureAdaptor
- [x] 3 unit tests: cycle, status, approve/reject via manager
- [x] `cargo check` ✅ + 3/3 tests pass

#### M5.5 Cluster Capability Sync ✅
- [x] `src/evolution/cluster_sync.rs` — Hub→Worker capability broadcast
- [x] CapabilitySyncMessage enum: Announce, SkillSync, ConfigSync, RequestSync, SyncResponse
- [x] NodeManifest: node_id, capabilities (HashSet), installed_packages (HashSet), last_sync_at
- [x] CapabilitySyncManager: process_message, diff_for_node, broadcast_install, known_nodes
- [x] Diff-based sync: only sync what's missing
- [x] 7 unit tests: announce, request sync, diff, broadcast, multiple nodes, hub capabilities, config/skill sync passthrough
- [x] `cargo check` ✅ + 7/7 tests pass

#### 整體驗證
- [x] `cargo check` ✅
- [x] 全部 evolution 測試: 47/47 pass
- [x] 全部測試: 3764/3764 pass (原 3717 + 47 evolution)

#### 三方審查 + 修復 ✅
- [x] 三方審查: Codex 1.6/5, Gemini 3.4/5 (Claude subagent 超時)
- [x] 共識 9 項問題修復 (5 agent 並行):
  - registry.rs: path traversal 驗證 (`..` / `/` / `\` 檢查)
  - auto_installer.rs: session dedupe 改存 `(id, Vec<capability>)` tuple
  - auto_installer.rs: CAS loop 取代 load/fetch_add 的 race condition
  - auto_installer.rs: verified-but-disabled → NeedsApproval 而非 NotAvailable
  - architecture_adaptor.rs: analyze() 加 adaptation_key() 去重
  - architecture_adaptor.rs: 負載不均衡 min=0 case 處理
  - architecture_adaptor.rs: approve/reject 加 is_none() 冪等檢查
  - manager.rs: last_check_at 改用 AtomicU64，evolution_cycle 更新時間戳
  - cluster_sync.rs: RequestSync.current_capabilities → current_packages 語意修正
- [x] `cargo check` ✅ + 54/54 evolution tests pass + 3771/3771 全部測試 pass ✅
