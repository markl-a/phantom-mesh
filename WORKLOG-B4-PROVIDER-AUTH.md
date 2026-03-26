# B.4 Provider Auth + Tier Router — 工作日誌

> 開始日期：2026-03-22
> Spec：`docs/superpowers/specs/2026-03-21-phantom-mesh-app-platform-design.md` §B.4
> Prompt：`../../PROMPT-B4.md`

---

## 里程碑進度

| # | 里程碑 | 狀態 | 完成時間 | 備註 |
|---|--------|------|----------|------|
| M4.1 | KeyVault — Encrypted Key Storage | ✅ 完成 | 2026-03-22 | 10 tests pass |
| M4.2 | Four-Tier Provider Routing | ✅ 完成 | 2026-03-22 | 12 tests pass |
| M4.3 | SubscriptionPacer | ✅ 完成 | 2026-03-22 | 10 tests pass |
| M4.4 | Local LLM Latency Probe | ✅ 完成 | 2026-03-22 | 9 tests pass |
| M4.5 | Cluster Key Sync (X25519) | ✅ 完成 | 2026-03-22 | 8 tests pass |

---

## 工作記錄

### 2026-03-22

#### M4.1 KeyVault — Encrypted Key Storage ✅
- [x] `src/security/key_vault.rs` — KeyStore trait + LocalKeyVault impl
- [x] Added `argon2`, `aes-gcm` crates to Cargo.toml
- [x] KeyStore trait: async store_key, get_key, list_keys, delete_key
- [x] KeyMeta + KeyPermission structs (provider, models, budget, nodes)
- [x] LocalKeyVault: Argon2id password → master key, AES-256-GCM per-key encryption
- [x] Lock/unlock state management with key zeroing
- [x] 10 unit tests: roundtrip, wrong password, lock/unlock, list, delete, duplicates, permissions
- [x] Export from `src/security/mod.rs`
- [x] `cargo check` ✅ + 10/10 tests pass

#### M4.2 Four-Tier Provider Routing ✅
- [x] `src/providers/tier.rs` — ProviderTier enum + TierRouter logic
- [x] LocalSpeed enum: Fast/Medium/Slow/Unknown with `from_latency_ms()`
- [x] ProviderTier: Local(1), FreeApi(2), Subscription(3), PayAsYouGo(4)
- [x] TierRouter: sorted providers, `best_providers()` with 3 routing modes
- [x] Circuit breaker integration: `set_available()`, `set_tripped()`
- [x] 12 unit tests: routing modes, priority, circuit breaker skip, empty/all-tripped
- [x] Export from `src/providers/mod.rs`
- [x] `cargo check` ✅ + 12/12 tests pass

#### M4.3 SubscriptionPacer ✅
- [x] `src/providers/subscription_pacer.rs` — daily quota management
- [x] `daily_allowance()` = remaining / days_left (clamps to min 1 day)
- [x] `can_use_today()`, `is_exhausted()`, `record_usage()`, `utilization()`
- [x] `reset_daily()` / `reset_cycle()` for daily/billing-cycle resets
- [x] Deterministic `_at(now)` methods for testability
- [x] Fixed: test race condition with `Utc::now()` drift
- [x] 10 unit tests: normal/last-day/exceeded/partial/zero-quota/past-reset
- [x] `cargo check` ✅ + 10/10 tests pass

#### M4.4 Local LLM Latency Probe ✅
- [x] `src/providers/local_probe.rs` — LocalProbe + LocalProbeManager
- [x] LocalProbe: provider_name, endpoint, last_latency_ms, speed, staleness check
- [x] LocalProbeManager: register/unregister, probe_all, overall_speed, stale_providers
- [x] `probe_endpoint()` — Ollama-compatible GET /api/tags health check
- [x] 9 unit tests: update, mark_failed, staleness, classification, manager ops
- [x] `cargo check` ✅ + 9/9 tests pass

#### M4.5 Cluster Key Sync (X25519) ✅
- [x] `src/security/key_sync.rs` — KeySyncServer + KeySyncClient
- [x] Added `x25519-dalek` crate to Cargo.toml
- [x] X25519 ephemeral key exchange → SHA-256 derived session key
- [x] Envelope encryption: session key encrypts API keys via AES-256-GCM
- [x] KeySyncServer: initiate_exchange, encrypt_key_for_worker, revoke_key
- [x] KeySyncClient: begin_exchange, complete_exchange, process_message
- [x] Key revocation: server marks revoked, client deletes + blocks
- [x] 8 unit tests: full roundtrip, revocation, no-session, wrong-key, multi-key
- [x] `cargo check` ✅ + 8/8 tests pass

#### 三方審查 + 修復 ✅
- [x] 三方審查: Codex(GPT-5.4) 1.4/5, Gemini 3.2/5, Claude subagent 3.5/5
- [x] 共識 13 項問題修復 (5 agent 並行):
  - key_vault.rs: constant-time 比較(subtle)、zeroize、password_hash 清除、test leak
  - key_sync.rs: HKDF 取代 SHA-256、不回傳 session key、加 KeyPermission、zeroize + Drop
  - local_probe.rs: saturating_sub、失敗更新 timestamp、重用 Client
  - subscription_pacer.rs: saturating_add、ceiling division
  - tier.rs: serde default available=true
  - Cargo.toml: 加 zeroize、subtle、hkdf crates
- [x] `cargo check` ✅ + 3717/3717 tests pass ✅
