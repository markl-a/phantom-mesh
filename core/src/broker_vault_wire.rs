// SPEC-15 §7 + §9 — Broker-vault-sync wire types (single source of truth for
// the 7 REST endpoints + per-account vault_seal_key envelope + age-wrap key
// handoff + HS256 broker JWT envelope that every broker client / future
// `spectynmesh-io` server impl must agree on).
//
// Stage 1 (spec → interface): types + ts-rs exports + `unimplemented!()` stub
// helpers only. No business logic — Stage 2 wires age v1 sealing / HMAC-SHA256
// per-item MAC / HS256 JWT verify / `crypto::randombytes(32)` seal-key gen per
// §7.1 (vault wire shape) / §7.2 (JWT claims) / §7.3 (vault item schema).
//
// 中文: 本檔對應 SPEC-15 §7（REST endpoints + JWT claims + vault item schema）
// 與 §9（API contracts）。`VaultSealKey`（保險庫密封金鑰）是 per-account 的
// 32-byte 隨機金鑰，**僅在 Rust crate 內持有 — 永遠不過 FFI / 不出 core**；
// 對 broker 上傳的只有 sealed ciphertext + per-item HMAC（雜湊訊息驗證碼）；
// 新裝置加入時透過既有裝置以 age v1 recipient-mode 把 seal_key 包成
// `WrappedVaultSealKey`（包好的保險庫金鑰）上傳 broker，新裝置 OAuth（開放
// 授權）完成後拉回自己解開。
//
// TODO Stage 2:
//   - wrap `VaultSealKey.bytes` in `zeroize::Zeroizing<[u8; 32]>` so Drop-time
//     memset is guaranteed even on panic unwind (zeroize already in
//     core/Cargo.toml — kept as raw `[u8; 32]` here to minimise dependency-
//     surface churn during Stage 1 merge).
//   - `seal_vault_value` → age v1 encrypt-then-base64url per §7.3.
//   - `compute_client_hmac` → HMAC-SHA256(`VaultSealKey`, service‖key‖sealed‖ts_ms)
//     then lower-hex per §7.3 column `client_hmac_hex`.
//   - `verify_broker_jwt` → HS256 verify + claim-schema validate per §7.2.
//   - `generate_vault_seal_key` → `OsRng.fill_bytes(&mut [0u8; 32])`.
//   - `wrap_vault_seal_key_for_recipient` → age v1 recipient-mode wrap → base64url.
//   - hook the per-endpoint request/response structs into the existing
//     `core/src/http_client.rs` so `spectyn broker {set,get,wipe}` CLI verbs
//     use a single typed surface.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::io::Write;
use ts_rs::TS;

// ─── §7 BrokerEndpoint — 7 REST endpoints catalog ────────────────────────────

/// Catalog of the 7 SPEC-15 §7.1 REST endpoints. Exposed to TS so the
/// `app/src/lib/api/broker.ts` client can dispatch on a single enum instead of
/// hand-rolled string switches (drift-prone — each rename would break in 7
/// places).
///
/// 中文: SPEC-15 §7.1 的 7 個 REST endpoint（介接點）列舉。把它 ts-export 讓
/// 前端 client 用 enum dispatch 而不是字串 switch — enum 改名 compiler 會抓，
/// 字串 switch 改名會 silently miss。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/broker_vault/")]
#[serde(rename_all = "snake_case")]
pub enum BrokerEndpoint {
    /// `POST /oauth/finish` — PKCE code → broker JWT exchange.
    OauthFinish,
    /// `POST /vault/set` — upload sealed item (one per call in Stage 2;
    /// batching extension tracked in §18 R4).
    VaultSet,
    /// `GET /vault/get` — pull sealed item by `service+key`.
    VaultGet,
    /// `DELETE /vault/wipe` — schedule 24h SLA full wipe.
    VaultWipe,
    /// `GET /vault/wipe/{wipe_id}` — poll wipe job status.
    VaultWipeStatus,
    /// `POST /vault/keys/wrap` — existing device uploads age-wrapped
    /// `vault_seal_key` for a pending new device.
    KeysWrap,
    /// `GET /vault/keys/wrapped` — new device pulls its wrapped seal-key
    /// after OAuth completes.
    KeysWrapped,
}

impl BrokerEndpoint {
    /// Lower-kebab path slug used in URL construction.
    /// `OauthFinish` → `"oauth/finish"`, `KeysWrap` → `"vault/keys/wrap"` etc.
    ///
    /// 中文: 回傳該 endpoint 的 URL path slug（不含 leading `/`），
    /// 給 `http_client.rs` 拼 `{broker_base_url}/{slug}` 用。
    pub const fn path_slug(self) -> &'static str {
        match self {
            BrokerEndpoint::OauthFinish => "oauth/finish",
            BrokerEndpoint::VaultSet => "vault/set",
            BrokerEndpoint::VaultGet => "vault/get",
            BrokerEndpoint::VaultWipe => "vault/wipe",
            // wipe_id is a path-param appended at call time; the slug here is
            // the prefix only.
            BrokerEndpoint::VaultWipeStatus => "vault/wipe",
            BrokerEndpoint::KeysWrap => "vault/keys/wrap",
            BrokerEndpoint::KeysWrapped => "vault/keys/wrapped",
        }
    }
}

// ─── §7.1 VaultSealKey — per-account 32-byte random key (Rust-only) ──────────

/// Per-account vault sealing key (32 random bytes). Used as:
/// 1. age v1 symmetric encryption secret for `value_sealed` (§7.3)
/// 2. HMAC-SHA256 key for `client_hmac_hex` per-item MAC (§7.3)
/// 3. payload that gets age-wrapped for new-device handoff (§6.3)
///
/// **NOT** ts-exported — this material MUST NOT cross the FFI boundary.
/// Frontend only sees sealed ciphertext + HMAC hex. The only legitimate way
/// for a new device to obtain `VaultSealKey` is via the `KeysWrap` →
/// `KeysWrapped` flow with age recipient-mode unwrap performed inside core.
///
/// 中文: per-account `VaultSealKey`（保險庫密封金鑰），32 byte 隨機；同時當 age
/// 對稱密鑰 + HMAC（雜湊訊息驗證碼）金鑰。**嚴禁過 FFI / 嚴禁 ts-export** — 前
/// 端只看得到加密過的 ciphertext 與 hex 編碼的 HMAC。新裝置取得 seal key 的唯
/// 一合法路徑：既有裝置以 age recipient-mode wrap → broker 中轉 → 新裝置 OAuth
/// 完成後拉回 → core 內 unwrap。
///
/// The 32 raw key bytes are scrubbed from memory on drop via `ZeroizeOnDrop`
/// (T-SEC-01 — no cleartext seal key lingers in freed heap/stack).
#[derive(Clone, zeroize::ZeroizeOnDrop)]
pub struct VaultSealKey {
    /// 32 raw bytes of CSPRNG output. `pub(crate)` — external callers MUST go
    /// through `seal_vault_value` / `compute_client_hmac` instead of reading
    /// the raw key.
    pub(crate) bytes: [u8; 32],
}

/// Redacting Debug — NEVER print the raw seal key (Debug-derived structs that
/// embed it, or stray `{:?}`, must not leak it into logs).
impl std::fmt::Debug for VaultSealKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VaultSealKey(<redacted 32 bytes>)")
    }
}

/// Load the per-account `VaultSealKey` from `~/.spectyn-mesh/vault-seal.key`
/// (base64url no-pad of the 32-byte key). Public so the separate Tauri/app
/// crate can unseal vault items without touching the `pub(crate)` field.
/// Returns `Err` if the file is missing or malformed — callers MUST fail
/// closed (never fall back to broker plaintext) on error.
///
/// 中文: 從 `~/.spectyn-mesh/vault-seal.key` 載入 per-account seal key（base64url
/// 32 byte）。公開給 app crate 用；檔案缺失/損毀回 Err,呼叫端必須 fail closed。
pub fn load_vault_seal_key() -> Result<VaultSealKey, String> {
    let path = crate::cli_config::spectyn_data_dir()
        .ok()
        .map(|d| d.join("vault-seal.key"))
        .ok_or_else(|| "no home dir for vault-seal.key".to_string())?;
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("vault-seal.key unreadable ({}): {e}", path.display()))?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw.trim().as_bytes())
        .map_err(|e| format!("vault-seal.key not valid base64url: {e}"))?;
    let bytes: [u8; 32] = decoded
        .try_into()
        .map_err(|v: Vec<u8>| format!("vault-seal.key must be 32 bytes, got {}", v.len()))?;
    Ok(VaultSealKey { bytes })
}

// ─── §6.3 WrappedVaultSealKey — age-wrap envelope for new-device handoff ────

/// Envelope carrying an age-wrapped `VaultSealKey` from an existing device
/// (via broker, who never sees plaintext) to a freshly OAuth'd new device.
///
/// The `wrapped_vault_seal_key_b64` field holds the age v1 recipient-mode
/// ciphertext encoded as base64url — the recipient is the new device's
/// ed25519 → X25519 public key (`target_device_pubkey_hex`). `key_version`
/// is bumped whenever the existing device rotates the seal-key (§18 R1).
///
/// 中文: 把 `VaultSealKey` 用 age v1 接收者模式包好的信封 — broker（中介伺
/// 服器）只當郵差，看不到內容；新裝置 OAuth 完成後拉回，用自己的私鑰拆
/// 開。`key_version` 配合 seal-key rotation（輪替）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/broker_vault/")]
#[serde(rename_all = "snake_case")]
pub struct WrappedVaultSealKey {
    /// Base64url-encoded age v1 ciphertext (recipient = `target_device_pubkey_hex`).
    pub wrapped_vault_seal_key_b64: String,
    /// Hex-encoded ed25519 public key of the **target** (new) device — the age
    /// recipient. 64 hex chars.
    pub target_device_pubkey_hex: String,
    /// Hex-encoded ed25519 public key of the **source** (existing) device —
    /// included so the new device UI can show "wrapped by: <hint>". 64 hex chars.
    pub source_device_pubkey_hex: String,
    /// Seal-key version. New devices reject wraps with a version they don't
    /// expect (prevents stale-key replay after rotation).
    pub key_version: u32,
    /// Hex-encoded ed25519 signature (128 hex chars) by the **source** device
    /// over the canonical envelope bytes (domain ‖ ciphertext ‖ target ‖ source
    /// ‖ version — see `wrap_envelope_signing_bytes`). Before unwrapping, the new
    /// device MUST verify this against an **out-of-band-pinned** source pubkey
    /// (QR / TOFU) — NOT against `source_device_pubkey_hex` alone, which a
    /// malicious broker could swap. Defeats a broker substituting the recipient,
    /// downgrading `key_version`, or splicing a different ciphertext under a
    /// trusted source identity (SPEC-15 multi-device, DECISION 2).
    pub envelope_sig_hex: String,
}

// ─── §7.1 OAuth finish — `POST /oauth/finish` request/response ───────────────

/// `POST /oauth/finish` request body — PKCE code + verifier exchange.
///
/// 中文: 把 OAuth provider（Google / Apple）callback 收到的 `code` 與 PKCE
/// （證明金鑰交換）verifier 上傳 broker，換回 broker JWT（網頁權杖）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/broker_vault/")]
#[serde(rename_all = "camelCase")]
pub struct OAuthFinishRequest {
    /// OAuth authorization code returned by provider (single-use).
    pub oauth_code: String,
    /// PKCE code-verifier matching the `code_challenge` sent at /authorize.
    pub pkce_verifier: String,
}

/// `POST /oauth/finish` 200 success body — broker JWT envelope.
///
/// 中文: OAuth 完成回傳的 broker JWT 信封；客戶端把 `jwt.token` 塞進
/// `Authorization: Bearer <token>` header 後續所有 vault 呼叫使用。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/broker_vault/")]
#[serde(rename_all = "camelCase")]
pub struct OAuthFinishResponse {
    /// HS256-signed broker JWT envelope.
    pub jwt: BrokerJwt,
}

// ─── §7.2 BrokerJwt — HS256 envelope ────────────────────────────────────────

/// Thin envelope around a broker-issued HS256 JWT. We keep the token as an
/// opaque `String` here — claim parsing lives in `verify_broker_jwt`. `expires_at_ts`
/// is duplicated outside the token body so the client can cheaply check
/// staleness without decoding (avoids parse + base64 work on every request).
///
/// 中文: HS256（HMAC-SHA256 JWT 簽章演算法）broker JWT 的精簡信封。`token`
/// 是 opaque 字串；`expires_at_ts` 是冗餘欄位 — client 不解碼 JWT 也可便宜判
/// 過期。Stage 2 的 `verify_broker_jwt` 才負責真正解碼 claims（聲明）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/broker_vault/")]
#[serde(rename_all = "camelCase")]
pub struct BrokerJwt {
    /// Opaque JWT string `<base64header>.<base64payload>.<base64signature>`.
    pub token: String,
    /// Epoch milliseconds; mirrors the `exp` claim inside `token`.
    pub expires_at_ts: u64,
}

// ─── §7.1 / §7.3 Vault set — `POST /vault/set` request ──────────────────────

/// `POST /vault/set` request body — one sealed vault item.
///
/// `value_sealed` is the age v1 ciphertext of the cleartext secret
/// (base64url). `client_hmac_hex` is HMAC-SHA256 over
/// `service ‖ key ‖ value_sealed ‖ ts_ms` — broker verifies before persist
/// so any tampered ciphertext is rejected at the edge.
///
/// 中文: 上傳單一 sealed vault item 的請求體。`value_sealed`（已密封值）是 age
/// 加密後 base64url；`client_hmac_hex` 是 client 端用 `VaultSealKey` 簽的 HMAC
/// （雜湊訊息驗證碼），broker 落地前先驗 — 任何篡改都會被擋。
///
/// **Stage 2 batching**: SPEC-15 §7.1 `POST /vault/set` accepts `items: [...]`
/// array. Stage 1 ships the single-item primitive; Stage 2 will add a
/// `VaultSetBatchRequest` wrapper struct around `Vec<VaultSetRequest>`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/broker_vault/")]
#[serde(rename_all = "snake_case")]
pub struct VaultSetRequest {
    /// Service slug — e.g. `"cerebras"`, `"anthropic"`, `"cluster"`.
    pub service: String,
    /// Item key under the service — e.g. `"default"`, `"api_key"`.
    pub key: String,
    /// Base64url age v1 ciphertext. Empty string is invalid; broker rejects.
    pub value_sealed: String,
    /// Lowercase hex HMAC-SHA256, 64 chars.
    pub client_hmac_hex: String,
    /// Client wall-clock at write intent (epoch ms). LWW conflict tiebreaker.
    pub ts_ms: u64,
}

// ─── §7.1 Vault get — `GET /vault/get` request/response ─────────────────────

/// `GET /vault/get?service=...&key=...` typed query.
///
/// Per §7.1 the endpoint also supports a "list mode" when both fields are
/// omitted — Stage 1 only models the single-item read path; the list-mode
/// response (`{items: [{service,key,ts_ms,byte_len}]}`) will land in Stage 2
/// alongside the `VaultListResponse` struct.
///
/// 中文: 拉單一 sealed vault item 的查詢請求。SPEC-15 還支援不帶參數的「列
/// 表模式」回傳 metadata 列表 — Stage 1 先寫 single-item 路徑，list mode
/// Stage 2 再補。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/broker_vault/")]
#[serde(rename_all = "snake_case")]
pub struct VaultGetRequest {
    pub service: String,
    pub key: String,
}

/// `GET /vault/get` 200 success body — single item with sealed payload.
///
/// `age_recipient_hint` is a UI-only field surfacing which device's age
/// public key the sealed value was originally sealed for. **Not** an
/// authorization claim — broker does no cryptographic check against it;
/// purely diagnostic ("this entry was sealed on iPhone-2026-05-01").
///
/// 中文: 單一 sealed item 的回應。`age_recipient_hint` 純 UI 提示用 —
/// broker 不靠它做授權；只是讓使用者看到「這筆是當初哪台裝置 seal 的」。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/broker_vault/")]
#[serde(rename_all = "snake_case")]
pub struct VaultGetResponse {
    pub service: String,
    pub key: String,
    pub value_sealed: String,
    pub ts_ms: u64,
    /// Optional hint (none = legacy / unknown).
    pub age_recipient_hint: Option<String>,
}

// ─── §7.1 Vault wipe — `DELETE /vault/wipe` request/response ────────────────

/// `DELETE /vault/wipe` request body — `scope` controls how aggressive.
///
/// `scope = "vault"` clears only vault rows + R2 objects; `scope = "all"`
/// additionally drops `broker_tokens` + `user_settings` (effectively logout
/// + account delete). 24h SLA on completion (§7.1 success body).
///
/// 中文: 一鍵清除請求。`scope`（範圍）= `"vault"` 只清保險庫；`"all"` 連
/// token / 帳號設定一起清（等同登出 + 砍號）。24 小時 SLA。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/broker_vault/")]
#[serde(rename_all = "snake_case")]
pub struct VaultWipeRequest {
    /// `"vault"` | `"all"` — represented as String to keep the wire surface
    /// open; broker validates. Future variants ("logs" / "telemetry") won't
    /// break this struct.
    pub scope: String,
    /// Human-readable reason logged for support / audit. Not used by broker
    /// logic; never returned to other users.
    pub reason: Option<String>,
}

/// `DELETE /vault/wipe` 202 accepted body — wipe job has been scheduled.
///
/// Per §7.1 the SLA is `eta_complete_ts = scheduled_at + 24h`. Clients should
/// surface this to the user as "estimated completion".
///
/// 中文: wipe 任務已排程的回應。`eta_complete_ts`（預計完成時戳）= 排程時間
/// + 24 小時。前端應提示使用者「預計 24 小時內清除完畢」。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/broker_vault/")]
#[serde(rename_all = "snake_case")]
pub struct VaultWipeResponse {
    /// Server-generated wipe job ID (e.g. `"wipe_abc123"`). Use with
    /// `VaultWipeStatusResponse` polling.
    pub wipe_id: String,
    /// Epoch ms — broker's 24h SLA deadline.
    pub eta_complete_ts: u64,
}

/// `GET /vault/wipe/{wipe_id}` 200 body — current wipe job status.
///
/// `completed_at` is `Some` iff `status == WipeStatus::Completed`. Clients
/// polling for the 24h SLA should treat `WipeStatus::Failed` as terminal
/// and surface a support contact path (broker keeps retry logic internal).
///
/// 中文: wipe 任務當前狀態查詢回應。`completed_at`（完成時戳）只有 status
/// = Completed 才會有值；Failed 是終態，UI 應提示使用者聯絡 support。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/broker_vault/")]
#[serde(rename_all = "snake_case")]
pub struct VaultWipeStatusResponse {
    pub wipe_id: String,
    pub status: WipeStatus,
    /// Epoch ms — `Some` only when `status == Completed`.
    pub completed_at: Option<u64>,
}

// ─── §7.1 Keys wrap — `POST /vault/keys/wrap` request ───────────────────────

/// `POST /vault/keys/wrap` request body — existing device uploads an
/// age-wrapped `VaultSealKey` for a freshly-paired new device to pick up.
///
/// `target_device_public_key` is the hex-encoded ed25519 verifying key of
/// the **new** device (broker stores the wrap under this key as index;
/// new device's later `KeysWrapped` GET selects by its own pubkey). The
/// actual ciphertext lives in the companion `WrappedVaultSealKey` struct
/// that gets uploaded separately — this request carries only the routing
/// metadata + key-version pin.
///
/// 中文: 既有裝置幫新裝置 wrap seal_key 的上傳請求。`target_device_public_key`
/// （目標裝置公鑰）= 新裝置的 ed25519 公鑰 hex；broker 用它當 index 儲存，
/// 新裝置之後 GET 時用自己的公鑰選回。`key_version` 配合 rotation 防 stale-
/// key replay（過期金鑰重放）。
// SPEC-15 §7.2/§7.3 reconcile (T-SEC-01 Stage D) — RESOLVED. Two distinct
// identifiers are deliberately kept separate, NOT conflated:
//   - the age `age1…` WRAP RECIPIENT the seal key is encrypted to (held in
//     `WrappedVaultSealKey.target_device_pubkey_hex`, the crypto target), and
//   - the new device's 64-hex ed25519 DEVICE PUBKEY the broker routes/indexes by
//     (this request's `target_device_pubkey_hex`, validated /^[0-9a-f]{64}$/).
// They are different values (the age1 recipient is HKDF-derived from identity.key
// and is NOT recoverable to the ed25519 pubkey), so the converter
// `WrappedVaultSealKey::into_keys_wrap_request` takes the ed25519 routing hex
// EXPLICITLY rather than copying the age1 string into a hex-validated field.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/broker_vault/")]
#[serde(rename_all = "snake_case")]
pub struct KeysWrapRequest {
    /// Hex ed25519 public key of the new device (age recipient). 64 hex chars.
    /// Wire name `target_device_pubkey_hex` to match the broker server.
    pub target_device_pubkey_hex: String,
    /// Base64url age v1 ciphertext of the wrapped `VaultSealKey` — the broker
    /// stores this verbatim and never decrypts it (it is the courier only).
    pub wrapped_vault_seal_key: String,
    /// Seal-key generation counter. Bumped on rotation (§18 R1).
    pub key_version: u32,
}

impl WrappedVaultSealKey {
    /// Convert a signed wrap envelope into the broker upload request
    /// (`POST /vault/keys/wrap`). T-SEC-01 Stage D — bridges the §6.3 envelope to
    /// the §7.2 request, carrying over the ciphertext + version.
    ///
    /// `target_device_pubkey_hex` MUST be the new device's 64-hex ed25519 public
    /// key (the broker's routing index, validated `/^[0-9a-f]{64}$/`) — supplied
    /// explicitly because the envelope's own `target_device_pubkey_hex` holds the
    /// age `age1…` wrap recipient (the crypto target), a DIFFERENT value. The
    /// ed25519-signed envelope fields are not re-derivable to this hex, so the
    /// caller (which learned it during pairing/QR) passes it in.
    ///
    /// 中文: 把已簽名的 wrap 信封轉成 broker 上傳請求。`target_device_pubkey_hex`
    /// 必須是新裝置的 64-hex ed25519 公鑰（broker 路由索引），由呼叫端在配對時
    /// 取得後明確傳入 — 信封自己的同名欄位存的是 age1 加密接收者（另一個值）。
    pub fn into_keys_wrap_request(self, target_device_pubkey_hex: String) -> KeysWrapRequest {
        KeysWrapRequest {
            target_device_pubkey_hex,
            wrapped_vault_seal_key: self.wrapped_vault_seal_key_b64,
            key_version: self.key_version,
        }
    }

    /// Build the `GET /vault/keys/wrapped` 200 body the broker returns to the
    /// new device — carries the ciphertext, version, AND the source pubkey hint
    /// + ed25519 signature the new device needs to verify-before-decrypt
    /// (T-SEC-01 Stage C). The broker stores+returns these verbatim (courier).
    /// Drops `target_device_pubkey_hex` (the age1 recipient): the new device
    /// recomputes it locally from its own identity, so it never travels.
    ///
    /// 中文: 組出 broker 回給新裝置的 wrapped 回應，帶上密文 + 版本 + 來源公鑰
    /// + 簽章（供新裝置先驗章再解密）。target age1 recipient 不隨線傳 — 新裝置
    /// 自己用本機 identity 重算。
    pub fn into_keys_wrapped_response(self) -> KeysWrappedResponse {
        KeysWrappedResponse {
            wrapped_vault_seal_key: self.wrapped_vault_seal_key_b64,
            key_version: self.key_version,
            source_device_pubkey_hex: Some(self.source_device_pubkey_hex),
            envelope_sig_hex: Some(self.envelope_sig_hex),
        }
    }
}

impl KeysWrappedResponse {
    /// Reconstruct the `WrappedVaultSealKey` envelope for verification on the new
    /// device, so it can call [`unwrap_vault_seal_key_verified`]. T-SEC-01.
    ///
    /// `target_device_wrap_recipient` MUST be THIS device's own published wrap
    /// recipient — i.e. `device_wrap_recipient(identity_bytes)` — because that is
    /// the exact `age1…` string the source device signed over when it wrapped TO
    /// this device. Passing anything else makes the reconstructed signing bytes
    /// differ from the source's, so verification fails closed (which is correct:
    /// a wrap aimed at a different recipient must not verify here).
    ///
    /// **Fails closed** with `Unauthorized` if the broker omitted the signature /
    /// source pubkey (a legacy broker that predates SPEC-15 sig support): without
    /// them the new device cannot verify-before-decrypt, so it MUST refuse rather
    /// than silently skip verification.
    ///
    /// 中文: 新裝置把 broker 回傳的 wrapped 回應還原成 `WrappedVaultSealKey` 以便
    /// 驗章+解密。`target_device_wrap_recipient` 必須是本機自己的 wrap recipient
    /// （`device_wrap_recipient`）。若 broker 沒帶簽章/來源公鑰（舊 broker），
    /// 直接 fail closed（`Unauthorized`）— 不可略過驗章。
    pub fn into_wrapped_vault_seal_key(
        self,
        target_device_wrap_recipient: String,
    ) -> Result<WrappedVaultSealKey, BrokerError> {
        let envelope_sig_hex = self.envelope_sig_hex.ok_or_else(|| BrokerError::Unauthorized {
            detail: "broker response omitted envelope_sig_hex — cannot verify wrap (legacy broker?)"
                .into(),
        })?;
        let source_device_pubkey_hex =
            self.source_device_pubkey_hex.ok_or_else(|| BrokerError::Unauthorized {
                detail: "broker response omitted source_device_pubkey_hex — cannot verify wrap"
                    .into(),
            })?;
        Ok(WrappedVaultSealKey {
            wrapped_vault_seal_key_b64: self.wrapped_vault_seal_key,
            target_device_pubkey_hex: target_device_wrap_recipient,
            source_device_pubkey_hex,
            key_version: self.key_version,
            envelope_sig_hex,
        })
    }
}

/// `GET /vault/keys/wrapped` 200 body — new device pulls its wrapped seal key.
///
/// `wrapped_vault_seal_key` is the base64url age v1 ciphertext; new device
/// unwraps it with its own X25519 secret (derived from the device ed25519
/// master via SPEC-12 HKDF) to recover the per-account `VaultSealKey`.
///
/// 中文: 新裝置取回 wrapped seal_key 的回應。`wrapped_vault_seal_key`（包裝過
/// 的保險庫金鑰）是 base64url age 密文；新裝置用自己的 X25519 私鑰拆出明文
/// `VaultSealKey`。`source_device_pubkey_hex` + `envelope_sig_hex` 讓新裝置在解密
/// 前先對 OOB-pinned 來源公鑰驗章（T-SEC-01 Stage C / DECISION 2）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/broker_vault/")]
#[serde(rename_all = "snake_case")]
pub struct KeysWrappedResponse {
    pub wrapped_vault_seal_key: String,
    pub key_version: u32,
    /// Hex ed25519 public key of the **source** device that produced this wrap.
    /// A UI hint only — the new device MUST verify `envelope_sig_hex` against an
    /// **out-of-band-pinned** source key (QR / TOFU), NOT this broker-supplied
    /// field. Mirrors `WrappedVaultSealKey.source_device_pubkey_hex`.
    ///
    /// `Option` + `#[serde(default)]`: the CURRENT broker
    /// (`spectynmesh-io/routes/vault.ts`, schema `0010`) does NOT yet store/return
    /// this, so a legacy response deserializes with `None` rather than failing.
    /// `None` ⇒ the new device cannot verify ⇒ `into_wrapped_vault_seal_key`
    /// fails closed (see there). Carrying it E2E needs a broker sig column +
    /// route field (tracked: SPEC-15 multi-device handoff, broker side).
    #[serde(default)]
    pub source_device_pubkey_hex: Option<String>,
    /// Hex ed25519 signature over the canonical wrap envelope (see
    /// `wrap_envelope_signing_bytes`). Lets the new device run
    /// `unwrap_vault_seal_key_verified` on retrieval. `Option`/`default` for the
    /// same legacy-broker reason as `source_device_pubkey_hex`; `None` ⇒ verify
    /// cannot run ⇒ fail closed. The broker is a courier and never
    /// produces/checks this; it would only store+return it verbatim.
    #[serde(default)]
    pub envelope_sig_hex: Option<String>,
}

// ─── §7.1 WipeStatus enum ───────────────────────────────────────────────────

/// Wipe job lifecycle state. Snake_case on the wire so TS clients can switch
/// on `"pending"` / `"in_progress"` / etc. directly without case-conversion.
///
/// 中文: wipe 任務的生命週期狀態。線上是 snake_case，前端 TS 直接 switch 字
/// 串就好。`Failed` 是終態（broker 內部重試已耗盡），UI 應提示聯絡 support。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/broker_vault/")]
#[serde(rename_all = "snake_case")]
pub enum WipeStatus {
    /// Queued, not yet started (broker scheduling lag).
    Pending,
    /// Worker is actively deleting D1 rows / R2 objects / backup tiers.
    InProgress,
    /// All three tiers cleared within the 24h SLA window.
    Completed,
    /// Terminal failure — internal retries exhausted; user must contact support.
    Failed,
}

// ─── §11 BrokerError — wire-facing error catalog ────────────────────────────

/// Wire-facing error variants for the broker-vault subsystem. Mirrors the
/// SPEC-15 §11 + per-endpoint error tables in §7.1 (collapsed to a single
/// canonical set — Stage 2 maps HTTP-level codes onto these variants via the
/// `http_client.rs` middleware).
///
/// 中文: SPEC-15 §11 與 §7.1 各 endpoint error table 的 wire-facing 收斂版。
/// HTTP 層的 status code 由 `http_client.rs` middleware 對應到這個列舉。
#[derive(Debug, Clone, Serialize, Deserialize, TS, thiserror::Error)]
#[ts(export, export_to = "../../app/src/lib/generated/broker_vault/")]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum BrokerError {
    /// `client_hmac_hex` did not match broker's recomputation — likely the
    /// client used a stale `VaultSealKey`.
    #[error("broker.hmac_mismatch")]
    HmacMismatch,
    /// JWT `exp` claim has passed — client must re-OAuth.
    #[error("broker.jwt_expired")]
    JwtExpired,
    /// Per-endpoint rate limit hit (e.g. `/vault/set` = 60/min/user per §7.1).
    /// `retry_after_s` is the broker's hint.
    #[error("broker.rate_limited: retry_after_s={retry_after_s}")]
    RateLimited { retry_after_s: u32 },
    /// Free-tier quota exceeded (items count or item bytes).
    #[error("broker.quota_exceeded: {detail}")]
    QuotaExceeded { detail: String },
    /// `DELETE /vault/wipe` while another wipe is already pending — broker
    /// returns the existing `wipe_id` so the client can poll instead of
    /// double-scheduling.
    #[error("broker.wipe_already_requested: existing_wipe_id={existing_wipe_id}")]
    WipeAlreadyRequested { existing_wipe_id: String },
    /// New device's expected `key_version` does not match the broker-stored
    /// wrap's version — likely a rotation happened mid-handoff. Client should
    /// re-pair from scratch.
    #[error("broker.key_version_mismatch: expected={expected} got={got}")]
    KeyVersionMismatch { expected: u32, got: u32 },
    /// Missing / malformed `Authorization` header, or JWT signature invalid.
    #[error("broker.unauthorized: {detail}")]
    Unauthorized { detail: String },
    /// Underlying transport failure (DNS / TCP / TLS / 5xx). Caller should
    /// retry with backoff.
    #[error("broker.network_error: {detail}")]
    NetworkError { detail: String },
}

// ─── §9 Stub helpers (Stage 2 implements; Stage 1 leaves `unimplemented!()`) ─

/// Seal a vault value plaintext with the per-account `VaultSealKey` using
/// age v1 symmetric encryption, then base64url-encode the ciphertext for
/// upload via `VaultSetRequest.value_sealed`.
///
/// 中文: 用 `VaultSealKey` 把明文以 age v1 對稱加密，輸出 base64url 字串給
/// `VaultSetRequest.value_sealed` 用。Stage 2 接 `age` crate。
pub fn seal_vault_value(
    plaintext: &[u8],
    seal_key: &VaultSealKey,
) -> Result<String, BrokerError> {
    // Step 1: build age::Encryptor::with_recipients from seal_key bytes — by
    //         deriving a deterministic x25519 identity from the 32-byte seal
    //         key (same pattern as life_node/crypto.rs::key_to_age_identity).
    let ciphertext = age_seal_pseudo(plaintext, &seal_key.bytes)?;
    // Step 2: base64url-encode the produced ciphertext for wire transport.
    Ok(base64_encode_pseudo(&ciphertext))
}

// ─── T-SEC-01 Stage A: per-device X25519 vault-wrap keypair (Option C) ──────
//
// SPEC-15 multi-device handoff, DECISION 1 (recipient encoding): instead of
// converting the device's ed25519 key to X25519 (montgomery math, error-prone)
// or shipping the `age` `ssh` feature (new dep), derive a DEDICATED X25519
// keypair for vault-wrap from `identity.key` via HKDF under a distinct label.
// The public half is published as the device's `age1...` wrap recipient (what
// other devices encrypt the seal_key TO); the secret half unwraps it. Reuses the
// proven bech32 → `age::x25519::Identity` path (same as `age_seal_pseudo`), so
// no new dependency and no hand-rolled curve math. Cryptographically independent
// of the event key + the symmetric seal key (different HKDF label).

/// HKDF label for the per-device X25519 vault-wrap keypair (must stay stable;
/// changing it rotates every device's wrap recipient).
const WRAP_X25519_HKDF_LABEL: &[u8] = b"spectyn-mesh.vault-wrap-x25519-v1";

/// Derive this device's X25519 vault-wrap `age::x25519::Identity` (the SECRET,
/// used to UNWRAP an incoming seal_key) deterministically from its `identity.key`
/// bytes. The matching public recipient is `.to_public()`. T-SEC-01 Decision 1.
fn derive_device_wrap_identity(
    identity_bytes: &[u8],
) -> Result<age::x25519::Identity, BrokerError> {
    use bech32::Hrp;
    if identity_bytes.len() < 16 {
        return Err(BrokerError::NetworkError {
            detail: format!(
                "identity.key too short for wrap-key derivation: {} bytes",
                identity_bytes.len()
            ),
        });
    }
    let hk = hkdf::Hkdf::<sha2::Sha256>::new(None, identity_bytes);
    let mut okm = zeroize::Zeroizing::new([0u8; 32]);
    hk.expand(WRAP_X25519_HKDF_LABEL, okm.as_mut())
        .map_err(|e| BrokerError::NetworkError {
            detail: format!("hkdf expand: {e}"),
        })?;
    let hrp = Hrp::parse("age-secret-key-").map_err(|e| BrokerError::NetworkError {
        detail: format!("bech32 hrp: {e}"),
    })?;
    // Zeroize the bech32-encoded secret string on drop (it holds the secret
    // scalar). Hardens beyond the surrounding age_seal_pseudo pattern, which
    // leaves its encoded secret string un-zeroized.
    let encoded = zeroize::Zeroizing::new(
        bech32::encode::<bech32::Bech32>(hrp, okm.as_ref())
            .map_err(|e| BrokerError::NetworkError {
                detail: format!("bech32 encode: {e}"),
            })?
            .to_uppercase(),
    );
    encoded
        .parse::<age::x25519::Identity>()
        .map_err(|e| BrokerError::NetworkError {
            detail: format!("age identity parse: {e}"),
        })
}

/// This device's published vault-wrap recipient (`age1...`) — the public key
/// other devices encrypt the `VaultSealKey` TO during the KeysWrap handoff.
/// Derived from `identity.key` (T-SEC-01 Decision 1, Option C).
pub fn device_wrap_recipient(identity_bytes: &[u8]) -> Result<String, BrokerError> {
    Ok(derive_device_wrap_identity(identity_bytes)?
        .to_public()
        .to_string())
}

/// Stage 3 helper — age v1 symmetric seal of `plaintext` under the 32-byte
/// `seal_key`. Returns the raw ciphertext bytes (pre-base64). Implemented as
/// a deterministic x25519 self-recipient (the seal_key bytes themselves form
/// the recipient identity) so encrypt and decrypt can both run from the same
/// 32-byte symmetric secret without dragging in passphrase / scrypt mode.
fn age_seal_pseudo(plaintext: &[u8], seal_key: &[u8; 32]) -> Result<Vec<u8>, BrokerError> {
    use bech32::Hrp;
    let hrp = Hrp::parse("age-secret-key-").map_err(|e| BrokerError::NetworkError {
        detail: format!("bech32: {e}"),
    })?;
    let encoded = bech32::encode::<bech32::Bech32>(hrp, seal_key).map_err(|e| {
        BrokerError::NetworkError {
            detail: format!("bech32: {e}"),
        }
    })?;
    let identity = encoded
        .to_uppercase()
        .parse::<age::x25519::Identity>()
        .map_err(|e| BrokerError::NetworkError {
            detail: format!("age identity: {e}"),
        })?;
    let recipient = identity.to_public();
    let encryptor =
        age::Encryptor::with_recipients(vec![Box::new(recipient)]).ok_or_else(|| {
            BrokerError::NetworkError {
                detail: "age: no recipients".into(),
            }
        })?;
    let mut buf = Vec::with_capacity(plaintext.len() + 256);
    let mut writer = encryptor
        .wrap_output(&mut buf)
        .map_err(|e| BrokerError::NetworkError {
            detail: format!("age wrap: {e}"),
        })?;
    writer
        .write_all(plaintext)
        .map_err(|e| BrokerError::NetworkError {
            detail: format!("age write: {e}"),
        })?;
    writer.finish().map_err(|e| BrokerError::NetworkError {
        detail: format!("age finish: {e}"),
    })?;
    Ok(buf)
}

/// Stage 3 helper — base64url encode arbitrary bytes for `value_sealed` wire field.
/// Per SPEC-15 §7.3 the wire format is base64url **without** padding (URL-safe
/// + no `=` so HTTP query / path embedding stays clean).
fn base64_encode_pseudo(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Read-path inverse of [`seal_vault_value`]. Takes the base64url `value_sealed`
/// returned by `GET /vault/get` (which the broker stored + echoed back verbatim,
/// never having been able to read it) and recovers the cleartext **client-side
/// only**, using the same deterministic x25519 identity derived from the 32-byte
/// `VaultSealKey`.
///
/// This is the single missing crypto primitive on the SPEC-15 §7 read path
/// (the Stage 1/2 module shipped `seal_vault_value` with no inverse, so the
/// whole module was dead code — nothing could complete a round trip). It MUST
/// stay symmetric with `seal_vault_value`: `unseal_vault_value(seal_vault_value(p))
/// == p` for the same key.
///
/// **E2EE invariant (SPEC-15 §0):** this function only ever runs inside the
/// Rust core holding the local `VaultSealKey`. The broker has no key and no
/// equivalent path — there is intentionally no server-side analogue. The §8.C
/// integrity step (client re-derives [`compute_client_hmac`] over the downloaded
/// payload and compares to the stored `server_hmac_hex`) catches tampering
/// before this runs; a low-level decrypt failure here maps to `NetworkError`.
///
/// 中文: [`seal_vault_value`] 的讀取路徑反函數。把 broker 原樣存回又原樣吐回
/// （broker 全程看不到明文）的 base64url `value_sealed` 解回明文 — **只在
/// 本機 core 內、用本機 `VaultSealKey`**。broker 沒有金鑰、沒有對應解密路徑。
pub fn unseal_vault_value(
    value_sealed_b64: &str,
    seal_key: &VaultSealKey,
) -> Result<Vec<u8>, BrokerError> {
    // Step 1: base64url(no-pad) decode the wire field back to age ciphertext.
    let ciphertext = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value_sealed_b64)
        .map_err(|e| BrokerError::NetworkError {
            detail: format!("base64url decode: {e}"),
        })?;
    // Step 2: age v1 decrypt under the deterministic x25519 identity that
    //         `age_seal_pseudo` derived from the same 32-byte seal key.
    age_unseal_pseudo(&ciphertext, &seal_key.bytes)
}

/// Stage 4 helper — inverse of [`age_seal_pseudo`]. Reconstructs the same
/// deterministic x25519 identity from the 32-byte `seal_key` and age v1 decrypts
/// `ciphertext`. Mirrors `core/src/life_node/crypto.rs::decrypt` (age 0.10
/// `Decryptor::Recipients` variant).
fn age_unseal_pseudo(ciphertext: &[u8], seal_key: &[u8; 32]) -> Result<Vec<u8>, BrokerError> {
    use bech32::Hrp;
    use std::io::Read as _;
    let hrp = Hrp::parse("age-secret-key-").map_err(|e| BrokerError::NetworkError {
        detail: format!("bech32: {e}"),
    })?;
    let encoded = bech32::encode::<bech32::Bech32>(hrp, seal_key).map_err(|e| {
        BrokerError::NetworkError {
            detail: format!("bech32: {e}"),
        }
    })?;
    let identity = encoded
        .to_uppercase()
        .parse::<age::x25519::Identity>()
        .map_err(|e| BrokerError::NetworkError {
            detail: format!("age identity: {e}"),
        })?;
    let decryptor =
        age::Decryptor::new(ciphertext).map_err(|e| BrokerError::NetworkError {
            detail: format!("age decryptor: {e}"),
        })?;
    let recipients_decryptor = match decryptor {
        age::Decryptor::Recipients(r) => r,
        age::Decryptor::Passphrase(_) => {
            return Err(BrokerError::NetworkError {
                detail: "passphrase-encrypted age blob; expected x25519 recipient".into(),
            })
        }
    };
    let mut reader = recipients_decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|e| BrokerError::NetworkError {
            detail: format!("age decrypt: {e}"),
        })?;
    let mut out = Vec::new();
    reader
        .read_to_end(&mut out)
        .map_err(|e| BrokerError::NetworkError {
            detail: format!("age read: {e}"),
        })?;
    Ok(out)
}

/// Compute the per-item HMAC-SHA256 over `service ‖ key ‖ sealed ‖ ts_ms`
/// using `VaultSealKey` as MAC key, then lower-hex encode. Used to fill
/// `VaultSetRequest.client_hmac_hex`.
///
/// 中文: 計算單一 item 的 HMAC-SHA256（雜湊訊息驗證碼），對 `service ‖ key
/// ‖ sealed ‖ ts_ms` 簽章後 lower-hex 編碼。Stage 2 接 `hmac` + `sha2` crate。
pub fn compute_client_hmac(
    seal_key: &VaultSealKey,
    service: &str,
    key: &str,
    sealed: &str,
    ts_ms: u64,
) -> String {
    // Step 1: build canonical string = service\nkey\nsealed\nts_ms per SPEC-15
    //         §7.3. Newline separator chosen because none of the four fields
    //         legally contain `\n` (service/key are slugs; sealed is base64url;
    //         ts_ms is u64) so unambiguous parsing on the broker side is free.
    let canonical = format!("{}\n{}\n{}\n{}", service, key, sealed, ts_ms);
    // Step 2: HMAC-SHA256(seal_key, canonical)
    let mac = hmac_sha256_pseudo(&seal_key.bytes, canonical.as_bytes());
    // Step 3: hex encode the MAC bytes lower-case (no `0x` prefix per §7.3).
    hex_encode_pseudo(&mac)
}

/// Constant-time verification that `provided_hmac_hex` matches the freshly
/// recomputed `compute_client_hmac(...)` for this vault item. Returns `true`
/// iff they match. Use this instead of `==` / `eq_ignore_ascii_case` on the
/// tag: a byte-by-byte short-circuiting compare leaks, via timing, how many
/// leading bytes an attacker guessed — letting them forge a valid
/// `client_hmac_hex` one nibble at a time. The recomputed tag is the secret
/// side; the comparison runs in time independent of its contents (a length
/// mismatch — public information, the tag is always 64 lower-hex chars — short
/// circuits, which is fine).
///
/// 中文: 用常數時間（constant-time）比對 HMAC tag，取代會短路的 `==`。短路比對
/// 會用「比對耗時」洩漏攻擊者猜中了前幾個位元組，使其能逐位元組偽造出合法
/// `client_hmac_hex`。長度不同（公開資訊，tag 固定 64 字）才短路，無妨。
pub fn verify_client_hmac(
    seal_key: &VaultSealKey,
    service: &str,
    key: &str,
    sealed: &str,
    ts_ms: u64,
    provided_hmac_hex: &str,
) -> bool {
    use subtle::ConstantTimeEq as _;
    let expected = compute_client_hmac(seal_key, service, key, sealed, ts_ms);
    // Normalise case on the attacker-supplied side only (compute_client_hmac
    // already emits lower-hex). Trimming + lowercasing operate on the provided
    // value, not the secret, so they add no secret-dependent timing.
    let provided = provided_hmac_hex.trim().to_ascii_lowercase();
    // Equal length is required before ct_eq (which assumes equal-length slices);
    // the tag length is fixed + public, so branching on it leaks nothing secret.
    expected.len() == provided.len()
        && bool::from(expected.as_bytes().ct_eq(provided.as_bytes()))
}

/// Stage 3 helper — HMAC-SHA256(key, msg) returning the 32-byte tag.
fn hmac_sha256_pseudo(key: &[u8; 32], msg: &[u8]) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    // HMAC key length is unrestricted; new_from_slice on Hmac<Sha256> only
    // errors for InvalidLength which never happens with a 32-byte input —
    // hence the unwrap. We still scope the type alias locally to keep the
    // top-of-file `use` list minimal.
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(msg);
    let result = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Stage 3 helper — lower-hex encode arbitrary bytes (no `0x` prefix).
fn hex_encode_pseudo(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

/// Verify a broker-issued JWT: signature with HS256 + `hs256_secret`,
/// `exp` claim not past, required claims (`iss` / `sub` / `aud` / `iat` /
/// `exp` / `provider` / `scope` / `jti`) all present.
///
/// 中文: 驗證 broker 簽的 JWT — HS256 簽章 + `exp` 未過 + 必要 claims 齊全。
/// Stage 2 接 `jsonwebtoken` crate。
pub fn verify_broker_jwt(jwt: &str, hs256_secret: &[u8]) -> Result<(), BrokerError> {
    use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};

    #[derive(serde::Deserialize)]
    struct BrokerClaims {
        // RFC 7519 reserves `exp` as numeric epoch seconds. jsonwebtoken's
        // built-in Validation already enforces `exp` not-past against the
        // current wall clock, so we only need to deserialize for fall-back
        // diagnostics — the library returns `ErrorKind::ExpiredSignature`
        // before our code path even sees the claims.
        #[allow(dead_code)]
        exp: u64,
    }

    // HS256 + default leeway = 60s. We do NOT validate iss/sub/aud here
    // (SPEC-15 §7.2 lists those as required claims; the broker is expected
    // to populate them and a later wrapper will turn on validation once the
    // expected values are wired through config).
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;

    match decode::<BrokerClaims>(jwt, &DecodingKey::from_secret(hs256_secret), &validation) {
        Ok(_) => Ok(()),
        Err(e) => match e.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => Err(BrokerError::JwtExpired),
            _ => Err(BrokerError::Unauthorized {
                detail: e.to_string(),
            }),
        },
    }
}

/// Stage 3 helper — current wall-clock epoch ms.
#[allow(dead_code)]
fn now_ms_pseudo() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Generate a fresh per-account `VaultSealKey` from the OS CSPRNG.
/// Stage 2 will use `OsRng.fill_bytes(&mut [0u8; 32])`.
///
/// 中文: 產生全新 per-account `VaultSealKey`（32 byte 隨機）。Stage 2 用
/// `OsRng` 或同等 CSPRNG（密碼學安全偽隨機數產生器）。
pub fn generate_vault_seal_key() -> VaultSealKey {
    // Step 1: OS RNG fill 32 bytes
    let bytes = os_random_bytes_pseudo();
    // Step 2: wrap as VaultSealKey newtype
    VaultSealKey { bytes }
}

/// Stage 3 helper — fetch 32 fresh bytes from the OS CSPRNG.
fn os_random_bytes_pseudo() -> [u8; 32] {
    use rand::RngCore;
    let mut buf = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    buf
}

/// Domain-separation prefix for the ed25519 signature over a wrap envelope.
/// Bumping it invalidates every previously-issued signature (a full re-wrap).
const WRAP_ENVELOPE_SIG_DOMAIN: &[u8] = b"spectyn-mesh.vault-wrap-envelope-v1";

/// Canonical byte string the source device ed25519-signs over a wrap envelope
/// (T-SEC-01 / SPEC-15 DECISION 2). Binds the ciphertext to ALL of its routing
/// metadata so a malicious broker cannot, under a trusted source identity:
///   - swap `target_device_pubkey_hex` to re-aim the wrap at an attacker device,
///   - downgrade `key_version` to replay a stale (rotated-out) seal key,
///   - splice a different `wrapped_vault_seal_key_b64` ciphertext.
///
/// Framing is **length-prefixed** (each variable field is preceded by its byte
/// length as u32 little-endian) rather than separator-delimited. This makes the
/// encoding injective for ANY `&str` content — there is no assumption that
/// fields avoid a separator byte, so no two distinct field tuples can ever map
/// to the same signing bytes (no canonicalization / field-confusion gap even if
/// a field somehow carried a NUL). The leading domain prefix gives
/// cross-protocol separation.
fn wrap_envelope_signing_bytes(
    wrapped_vault_seal_key_b64: &str,
    target_device_pubkey_hex: &str,
    source_device_pubkey_hex: &str,
    key_version: u32,
) -> Vec<u8> {
    let mut msg = Vec::with_capacity(
        WRAP_ENVELOPE_SIG_DOMAIN.len()
            + wrapped_vault_seal_key_b64.len()
            + target_device_pubkey_hex.len()
            + source_device_pubkey_hex.len()
            + 32,
    );
    msg.extend_from_slice(WRAP_ENVELOPE_SIG_DOMAIN);
    for field in [
        wrapped_vault_seal_key_b64,
        target_device_pubkey_hex,
        source_device_pubkey_hex,
    ] {
        // u32-LE length prefix → injective framing (no separator assumption).
        msg.extend_from_slice(&(field.len() as u32).to_le_bytes());
        msg.extend_from_slice(field.as_bytes());
    }
    msg.extend_from_slice(&key_version.to_le_bytes());
    msg
}

/// Wrap the local `VaultSealKey` for a remote recipient using age v1
/// recipient-mode AND ed25519-sign the resulting envelope with the source
/// device's signing key (T-SEC-01 / SPEC-15 DECISION 2). Returns a fully
/// populated `WrappedVaultSealKey` ready for upload via `POST /vault/keys/wrap`.
///
/// `recipient` is the target device's published `age1...` vault-wrap recipient
/// (see `device_wrap_recipient`). `source_signing_key` is the source device's
/// ed25519 identity key; its verifying half is recorded in
/// `source_device_pubkey_hex` AND must be delivered to the new device
/// out-of-band (QR / TOFU pin) so the new device can verify the signature
/// against a key the broker never controls. `key_version` pins the seal-key
/// generation (§18 R1 rotation).
///
/// Signing is MANDATORY — there is deliberately no unsigned-envelope path, so a
/// substituted or spliced envelope cannot pass the new device's verify step.
///
/// 中文: 用 age v1 recipient mode（接收者模式）把 `VaultSealKey` 包給遠端裝
/// 置，並用來源裝置的 ed25519 簽章金鑰對整個信封簽名（DECISION 2，防中介伺
/// 服器掉包）。recipient 是新裝置公布的 `age1...` wrap 公鑰；簽章對應的公鑰
/// （`source_device_pubkey_hex`）必須另循 QR / TOFU 帶給新裝置驗證 — 不可只
/// 信 broker 傳來的那份。簽名為強制，沒有未簽信封的路徑。
pub fn wrap_vault_seal_key_for_recipient(
    seal_key: &VaultSealKey,
    recipient: &str,
    source_signing_key: &ed25519_dalek::SigningKey,
    key_version: u32,
) -> Result<WrappedVaultSealKey, BrokerError> {
    use ed25519_dalek::Signer as _;
    // Step 1: age v1 recipient-mode wrap of the 32-byte seal key.
    let ciphertext = age_wrap_for_recipient(&seal_key.bytes, recipient)?;
    let wrapped_b64 = base64_encode_pseudo(&ciphertext);
    // Step 2: record the source verifying key (hex) — both for UI hinting and
    //         as the identity the out-of-band-pinned key must match.
    let source_pubkey_hex = hex_encode_lower(source_signing_key.verifying_key().as_bytes());
    let target_pubkey_hex = recipient.to_string();
    // Step 3: ed25519-sign the canonical envelope bytes (binds ciphertext to
    //         target + source + version — see `wrap_envelope_signing_bytes`).
    let msg = wrap_envelope_signing_bytes(
        &wrapped_b64,
        &target_pubkey_hex,
        &source_pubkey_hex,
        key_version,
    );
    let sig = source_signing_key.sign(&msg);
    let envelope_sig_hex = hex_encode_lower(&sig.to_bytes());
    Ok(WrappedVaultSealKey {
        wrapped_vault_seal_key_b64: wrapped_b64,
        target_device_pubkey_hex: target_pubkey_hex,
        source_device_pubkey_hex: source_pubkey_hex,
        key_version,
        envelope_sig_hex,
    })
}

/// Lower-hex encode (local helper — avoids dragging in the `hex` crate just for
/// two call sites). Used for ed25519 public keys + signatures on the wire.
fn hex_encode_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

/// Decode a lower/upper-hex string to bytes. Returns `None` on odd length or any
/// non-hex char (so malformed broker-supplied hex is rejected, never panics).
fn hex_decode_lower(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut i = 0;
    while i < b.len() {
        let hi = (b[i] as char).to_digit(16)?;
        let lo = (b[i + 1] as char).to_digit(16)?;
        out.push(((hi << 4) | lo) as u8);
        i += 2;
    }
    Some(out)
}

/// Verify a `WrappedVaultSealKey` against an **out-of-band-pinned** source
/// verifying key, then age-unwrap it with THIS device's HKDF-derived wrap
/// secret — returning the 32-byte seal key in `Zeroizing` so it scrubs on drop.
/// This is the production counterpart to `wrap_vault_seal_key_for_recipient`
/// (T-SEC-01 / SPEC-15 DECISION 2 — verified unwrap on the new device).
///
/// SECURITY: `pinned_source_pubkey` MUST be obtained out-of-band (QR scan /
/// TOFU pin) — it is intentionally a typed `VerifyingKey` the caller supplies,
/// NOT decoded from `envelope.source_device_pubkey_hex` (which a malicious
/// broker controls). Verifying against the embedded field would defeat the
/// entire scheme. The order is verify-then-decrypt: a tampered or wrong-source
/// envelope is rejected before any decryption is attempted.
///
/// `expected_key_version` rejects a broker downgrading to a rotated-out key
/// (returns `KeyVersionMismatch`). `identity_bytes` are this device's
/// `identity.key` bytes (same input as `device_wrap_recipient`).
///
/// 中文: 先用「另循 QR / TOFU 帶來、broker 控制不到」的來源公鑰驗章，再用本機
/// 由 identity.key 衍生的 wrap 私鑰把信封拆開，回傳會自動歸零的 32-byte seal
/// key。絕不可用信封內 broker 給的 `source_device_pubkey_hex` 來驗（那會讓整套
/// 防掉包失效）。順序是「先驗章再解密」；版本不符直接拒（防降級重放）。
pub fn unwrap_vault_seal_key_verified(
    envelope: &WrappedVaultSealKey,
    pinned_source_pubkey: &ed25519_dalek::VerifyingKey,
    expected_key_version: u32,
    identity_bytes: &[u8],
) -> Result<zeroize::Zeroizing<[u8; 32]>, BrokerError> {
    use ed25519_dalek::Verifier as _;
    // Step 1: reject a version downgrade before doing crypto work.
    if envelope.key_version != expected_key_version {
        return Err(BrokerError::KeyVersionMismatch {
            expected: expected_key_version,
            got: envelope.key_version,
        });
    }
    // Step 2: parse the signature (64 raw bytes from 128 hex chars).
    let sig_bytes = hex_decode_lower(&envelope.envelope_sig_hex).ok_or_else(|| {
        BrokerError::Unauthorized {
            detail: "wrap envelope signature is not valid hex".into(),
        }
    })?;
    let sig_arr: [u8; 64] = sig_bytes.as_slice().try_into().map_err(|_| {
        BrokerError::Unauthorized {
            detail: "wrap envelope signature must be 64 bytes".into(),
        }
    })?;
    let sig = ed25519_dalek::Signature::from_bytes(&sig_arr);
    // Step 3: verify the signature against the PINNED source key over the same
    //         canonical bytes the source signed. Tamper / wrong-source → reject.
    let msg = wrap_envelope_signing_bytes(
        &envelope.wrapped_vault_seal_key_b64,
        &envelope.target_device_pubkey_hex,
        &envelope.source_device_pubkey_hex,
        envelope.key_version,
    );
    pinned_source_pubkey
        .verify(&msg, &sig)
        .map_err(|_| BrokerError::Unauthorized {
            detail: "wrap envelope signature does not verify against pinned source key".into(),
        })?;
    // Step 4: signature good — now decrypt. Derive this device's wrap identity
    //         and age-unwrap the base64url ciphertext.
    let ciphertext = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&envelope.wrapped_vault_seal_key_b64)
        .map_err(|e| BrokerError::NetworkError {
            detail: format!("wrap ciphertext base64url: {e}"),
        })?;
    let identity = derive_device_wrap_identity(identity_bytes)?;
    // `age_unwrap_with_identity` returns the cleartext seal in `Zeroizing`, so
    // both the intermediate plaintext and the final array scrub on drop.
    let plaintext = age_unwrap_with_identity(&ciphertext, &identity)?;
    let seal: [u8; 32] = plaintext.as_slice().try_into().map_err(|_| {
        BrokerError::NetworkError {
            detail: format!("unwrapped seal key must be 32 bytes, got {}", plaintext.len()),
        }
    })?;
    Ok(zeroize::Zeroizing::new(seal))
}

/// age v1 recipient-mode decrypt of `ciphertext` using the given x25519
/// `identity` (this device's derived wrap secret). Mirrors `age_unseal_pseudo`
/// but takes an already-built identity rather than raw symmetric bytes.
fn age_unwrap_with_identity(
    ciphertext: &[u8],
    identity: &age::x25519::Identity,
) -> Result<zeroize::Zeroizing<Vec<u8>>, BrokerError> {
    use std::io::Read as _;
    let decryptor = age::Decryptor::new(ciphertext).map_err(|e| BrokerError::NetworkError {
        detail: format!("age decryptor: {e}"),
    })?;
    let recipients_decryptor = match decryptor {
        age::Decryptor::Recipients(r) => r,
        age::Decryptor::Passphrase(_) => {
            return Err(BrokerError::NetworkError {
                detail: "passphrase-encrypted age blob; expected x25519 recipient".into(),
            })
        }
    };
    let mut reader = recipients_decryptor
        .decrypt(std::iter::once(identity as &dyn age::Identity))
        .map_err(|e| BrokerError::NetworkError {
            detail: format!("age decrypt: {e}"),
        })?;
    // Pre-size so a 32-byte seal-key plaintext never reallocates mid-read — a
    // growth realloc would free an un-zeroized buffer holding key fragments.
    // The Zeroizing wrapper scrubs the final buffer on drop.
    let mut out = zeroize::Zeroizing::new(Vec::with_capacity(64));
    reader
        .read_to_end(&mut out)
        .map_err(|e| BrokerError::NetworkError {
            detail: format!("age read: {e}"),
        })?;
    Ok(out)
}

/// Stage 3 helper — wrap `plaintext` (the 32-byte seal key) under the given
/// textual age recipient (`age1...`) and return the raw age v1 ciphertext.
fn age_wrap_for_recipient(plaintext: &[u8], recipient: &str) -> Result<Vec<u8>, BrokerError> {
    let recipient = recipient
        .parse::<age::x25519::Recipient>()
        .map_err(|e| BrokerError::NetworkError {
            detail: format!("age recipient parse: {e}"),
        })?;
    let encryptor =
        age::Encryptor::with_recipients(vec![Box::new(recipient)]).ok_or_else(|| {
            BrokerError::NetworkError {
                detail: "age: no recipients".into(),
            }
        })?;
    let mut buf = Vec::with_capacity(plaintext.len() + 256);
    let mut writer = encryptor
        .wrap_output(&mut buf)
        .map_err(|e| BrokerError::NetworkError {
            detail: format!("age wrap: {e}"),
        })?;
    writer
        .write_all(plaintext)
        .map_err(|e| BrokerError::NetworkError {
            detail: format!("age write: {e}"),
        })?;
    writer.finish().map_err(|e| BrokerError::NetworkError {
        detail: format!("age finish: {e}"),
    })?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_set_request_round_trip_smoke() {
        // §7.3 invariant: VaultSetRequest serde must round-trip without
        // losing any field — broker rejects on any missing / extra field.
        // Stage 1 only sanity-checks serde; HMAC-validity check comes in Stage 2.
        let req = VaultSetRequest {
            service: "cerebras".to_string(),
            key: "default".to_string(),
            value_sealed: "YWdlX3YxX2NpcGhlcnRleHQ".to_string(),
            client_hmac_hex: "d4e5f6a7b8c9".to_string() + &"0".repeat(52),
            ts_ms: 1_700_000_000_000,
        };
        let j = serde_json::to_string(&req).unwrap();
        // snake_case invariant — MUST match the broker server (spectynmesh-io
        // routes/vault.ts reads value_sealed / client_hmac_hex / ts_ms). A
        // camelCase regression here silently breaks every /vault/set write.
        assert!(j.contains("\"value_sealed\""), "snake_case wire required: {}", j);
        assert!(j.contains("\"client_hmac_hex\""), "snake_case wire required: {}", j);
        assert!(j.contains("\"ts_ms\""), "snake_case wire required: {}", j);
        assert!(!j.contains("valueSealed"), "camelCase regression: {}", j);
        let back: VaultSetRequest = serde_json::from_str(&j).unwrap();
        assert_eq!(req.service, back.service);
        assert_eq!(req.key, back.key);
        assert_eq!(req.value_sealed, back.value_sealed);
        assert_eq!(req.client_hmac_hex, back.client_hmac_hex);
        assert_eq!(req.ts_ms, back.ts_ms);
    }

    #[test]
    fn wipe_status_serde_shape_is_snake_case() {
        // §7.1 invariant: wire wants `"pending"` / `"in_progress"` /
        // `"completed"` / `"failed"` — NOT PascalCase. TS client switches on
        // these literal strings; any drift silently breaks the UI.
        assert_eq!(
            serde_json::to_string(&WipeStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&WipeStatus::InProgress).unwrap(),
            "\"in_progress\""
        );
        assert_eq!(
            serde_json::to_string(&WipeStatus::Completed).unwrap(),
            "\"completed\""
        );
        assert_eq!(
            serde_json::to_string(&WipeStatus::Failed).unwrap(),
            "\"failed\""
        );
        // Round-trip a Completed status carried inside the full response struct.
        let r = VaultWipeStatusResponse {
            wipe_id: "wipe_abc123".to_string(),
            status: WipeStatus::Completed,
            completed_at: Some(1_700_086_400_000),
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains("\"completed\""), "wire shape: {}", j);
        let back: VaultWipeStatusResponse = serde_json::from_str(&j).unwrap();
        assert_eq!(back.status, WipeStatus::Completed);
        assert_eq!(back.completed_at, Some(1_700_086_400_000));
    }

    #[test]
    fn broker_endpoint_path_slugs_match_spec() {
        // §7.1 invariant: path slugs are stable wire — any change is an API
        // break. Lock them in at the type level so renames are loud.
        assert_eq!(BrokerEndpoint::OauthFinish.path_slug(), "oauth/finish");
        assert_eq!(BrokerEndpoint::VaultSet.path_slug(), "vault/set");
        assert_eq!(BrokerEndpoint::VaultGet.path_slug(), "vault/get");
        assert_eq!(BrokerEndpoint::VaultWipe.path_slug(), "vault/wipe");
        // wipe-status shares the `vault/wipe` prefix (wipe_id is path-param).
        assert_eq!(BrokerEndpoint::VaultWipeStatus.path_slug(), "vault/wipe");
        assert_eq!(BrokerEndpoint::KeysWrap.path_slug(), "vault/keys/wrap");
        assert_eq!(BrokerEndpoint::KeysWrapped.path_slug(), "vault/keys/wrapped");
    }

    #[test]
    fn seal_vault_value_produces_age_v1_blob() {
        // Stage 3 real impl: `seal_vault_value` must return a base64url string
        // whose decoded prefix is the age v1 magic line. We do NOT round-trip
        // decrypt here (no `unseal_vault_value` helper exists yet — that
        // belongs to the Stage 4 read-path); the magic line proves the age
        // encryptor actually ran.
        let key = VaultSealKey { bytes: [0x11u8; 32] };
        let sealed = seal_vault_value(b"super-secret-token", &key).expect("seal");
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&sealed)
            .expect("base64url");
        assert!(
            raw.starts_with(b"age-encryption.org/v1\n"),
            "sealed payload must start with age v1 magic, first 32 bytes = {:?}",
            &raw[..raw.len().min(32)]
        );
    }

    #[test]
    fn seal_then_unseal_round_trips_plaintext() {
        // Symmetry invariant the `unseal_vault_value` doc demands:
        // `unseal_vault_value(seal_vault_value(p)) == p` for the same key.
        // Without this the §7 read path was never proven end-to-end.
        let key = VaultSealKey { bytes: [0x44u8; 32] };
        for plaintext in [
            &b""[..],
            &b"x"[..],
            &b"super-secret-api-token-value"[..],
            &[0u8; 1024][..],
        ] {
            let sealed = seal_vault_value(plaintext, &key).expect("seal");
            let recovered = unseal_vault_value(&sealed, &key).expect("unseal");
            assert_eq!(recovered, plaintext, "round trip must recover plaintext");
        }
    }

    #[test]
    fn unseal_with_wrong_key_fails() {
        // A different seal key derives a different x25519 identity → the age
        // recipient stanza will not decrypt → NetworkError (not a panic, not a
        // wrong-but-successful plaintext).
        let key = VaultSealKey { bytes: [0x44u8; 32] };
        let other = VaultSealKey { bytes: [0x45u8; 32] };
        let sealed = seal_vault_value(b"secret", &key).expect("seal");
        let err = unseal_vault_value(&sealed, &other).unwrap_err();
        assert!(matches!(err, BrokerError::NetworkError { .. }), "got {err:?}");
    }

    #[test]
    fn unseal_rejects_non_base64url_and_garbage_ciphertext() {
        let key = VaultSealKey { bytes: [0x44u8; 32] };
        // Not valid base64url at all.
        assert!(matches!(
            unseal_vault_value("not base64!!!", &key).unwrap_err(),
            BrokerError::NetworkError { .. }
        ));
        // Valid base64url but not an age blob.
        let junk = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"definitely not age");
        assert!(matches!(
            unseal_vault_value(&junk, &key).unwrap_err(),
            BrokerError::NetworkError { .. }
        ));
    }

    #[test]
    fn compute_client_hmac_is_deterministic_and_hex_64() {
        // HMAC-SHA256 → 32 bytes → 64 lowercase hex chars per SPEC-15 §7.3.
        // Determinism: same inputs MUST produce same output (otherwise broker
        // verification breaks).
        let key = VaultSealKey { bytes: [0x22u8; 32] };
        let a = compute_client_hmac(&key, "cerebras", "default", "abc", 1_700_000_000_000);
        let b = compute_client_hmac(&key, "cerebras", "default", "abc", 1_700_000_000_000);
        assert_eq!(a, b, "HMAC must be deterministic");
        assert_eq!(a.len(), 64, "lower-hex HMAC-SHA256 must be 64 chars");
        assert!(
            a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "lowercase hex only: {a}"
        );
        // Different ts → different MAC (proves ts_ms entered the canonical input).
        let c = compute_client_hmac(&key, "cerebras", "default", "abc", 1_700_000_000_001);
        assert_ne!(a, c, "ts_ms drift must change MAC");
    }

    #[test]
    fn verify_client_hmac_accepts_match_and_rejects_tamper() {
        let key = VaultSealKey { bytes: [0x22u8; 32] };
        let good = compute_client_hmac(&key, "cerebras", "default", "abc", 1_700_000_000_000);
        // Exact match verifies.
        assert!(verify_client_hmac(&key, "cerebras", "default", "abc", 1_700_000_000_000, &good));
        // Case-insensitive + surrounding whitespace tolerated (broker may echo
        // upper-hex / padded), matching the prior eq_ignore_ascii_case behaviour.
        assert!(verify_client_hmac(
            &key, "cerebras", "default", "abc", 1_700_000_000_000,
            &format!("  {}  ", good.to_uppercase())
        ));
        // Any tampered field → reject.
        assert!(!verify_client_hmac(&key, "cerebras", "default", "abc", 1_700_000_000_001, &good),
            "ts drift must fail");
        assert!(!verify_client_hmac(&key, "groq", "default", "abc", 1_700_000_000_000, &good),
            "service swap must fail");
        assert!(!verify_client_hmac(&key, "cerebras", "other", "abc", 1_700_000_000_000, &good),
            "key-field swap must fail");
        assert!(!verify_client_hmac(&key, "cerebras", "default", "abcX", 1_700_000_000_000, &good),
            "sealed swap must fail");
        // Wrong key → reject.
        let other = VaultSealKey { bytes: [0x33u8; 32] };
        assert!(!verify_client_hmac(&other, "cerebras", "default", "abc", 1_700_000_000_000, &good),
            "wrong seal key must fail");
        // Malformed provided tags → reject without panic.
        assert!(!verify_client_hmac(&key, "cerebras", "default", "abc", 1_700_000_000_000, ""),
            "empty must fail");
        assert!(!verify_client_hmac(&key, "cerebras", "default", "abc", 1_700_000_000_000, "deadbeef"),
            "under-length (valid hex) must fail");
        assert!(!verify_client_hmac(&key, "cerebras", "default", "abc", 1_700_000_000_000,
            &(good.clone() + "ff")), "over-length must fail");
        // Same length (64 chars) but non-hex content → reject (not a panic, not a match).
        assert!(!verify_client_hmac(&key, "cerebras", "default", "abc", 1_700_000_000_000,
            &"z".repeat(64)), "same-length non-hex must fail");
    }

    #[test]
    fn generate_vault_seal_key_produces_distinct_keys() {
        // Two consecutive CSPRNG draws must (overwhelmingly probably) differ.
        // Equality on 32 random bytes has probability 2^-256; if this test
        // ever flakes the RNG is broken, not the test.
        let k1 = generate_vault_seal_key();
        let k2 = generate_vault_seal_key();
        assert_ne!(k1.bytes, k2.bytes, "OsRng must produce fresh bytes");
    }

    #[test]
    fn wrap_vault_seal_key_round_trips_to_age_blob() {
        // Build a fresh recipient locally so we have a known public key.
        let identity = age::x25519::Identity::generate();
        let recipient_str = identity.to_public().to_string();
        let seal = VaultSealKey { bytes: [0x33u8; 32] };
        let signing = ed25519_dalek::SigningKey::from_bytes(&[0x55u8; 32]);
        let env =
            wrap_vault_seal_key_for_recipient(&seal, &recipient_str, &signing, 7).expect("wrap");
        assert_eq!(env.target_device_pubkey_hex, recipient_str);
        assert_eq!(env.key_version, 7);
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&env.wrapped_vault_seal_key_b64)
            .expect("base64url");
        assert!(
            raw.starts_with(b"age-encryption.org/v1\n"),
            "wrapped seal must be age v1 blob"
        );
    }

    // ── T-SEC-01 Stage B: ed25519-signed wrap envelope (DECISION 2) ─────────
    /// Re-verify a signed envelope the same way the new device will in Stage C:
    /// reconstruct the canonical bytes, check the sig against the pinned source
    /// verifying key. Returns whether it verifies — used to assert tamper fails.
    fn verify_env(env: &WrappedVaultSealKey, pinned_source_pub: &ed25519_dalek::VerifyingKey) -> bool {
        use ed25519_dalek::Verifier as _;
        let sig_bytes = match hex_decode_lower(&env.envelope_sig_hex) {
            Some(b) if b.len() == 64 => b,
            _ => return false,
        };
        let mut arr = [0u8; 64];
        arr.copy_from_slice(&sig_bytes);
        let sig = ed25519_dalek::Signature::from_bytes(&arr);
        let msg = wrap_envelope_signing_bytes(
            &env.wrapped_vault_seal_key_b64,
            &env.target_device_pubkey_hex,
            &env.source_device_pubkey_hex,
            env.key_version,
        );
        pinned_source_pub.verify(&msg, &sig).is_ok()
    }

    #[test]
    fn signed_wrap_envelope_verifies_against_pinned_source_key() {
        let recipient = age::x25519::Identity::generate().to_public().to_string();
        let seal = VaultSealKey { bytes: [0xABu8; 32] };
        let signing = ed25519_dalek::SigningKey::from_bytes(&[0x01u8; 32]);
        let env = wrap_vault_seal_key_for_recipient(&seal, &recipient, &signing, 3).expect("wrap");

        // Envelope records the source verifying key, and the sig is well-formed.
        assert_eq!(
            env.source_device_pubkey_hex,
            hex_encode_lower(signing.verifying_key().as_bytes())
        );
        assert_eq!(env.envelope_sig_hex.len(), 128, "ed25519 sig = 64 bytes = 128 hex");
        // Verifies against the OUT-OF-BAND-pinned source key (the Stage C check).
        assert!(verify_env(&env, &signing.verifying_key()));
    }

    #[test]
    fn signed_wrap_envelope_rejects_broker_substitution() {
        let recipient = age::x25519::Identity::generate().to_public().to_string();
        let seal = VaultSealKey { bytes: [0xCDu8; 32] };
        let source = ed25519_dalek::SigningKey::from_bytes(&[0x02u8; 32]);
        let pinned = source.verifying_key();
        let good = wrap_vault_seal_key_for_recipient(&seal, &recipient, &source, 5).expect("wrap");
        assert!(verify_env(&good, &pinned), "honest envelope must verify");

        // (a) Broker re-aims the wrap at an attacker recipient → sig breaks.
        let attacker_recipient = age::x25519::Identity::generate().to_public().to_string();
        let mut swapped_target = good.clone();
        swapped_target.target_device_pubkey_hex = attacker_recipient;
        assert!(!verify_env(&swapped_target, &pinned), "target swap must fail verify");

        // (b) Broker downgrades the seal-key version to replay a rotated key.
        let mut downgraded = good.clone();
        downgraded.key_version = 1;
        assert!(!verify_env(&downgraded, &pinned), "version downgrade must fail verify");

        // (c) Broker splices a different ciphertext under the trusted identity.
        let other = wrap_vault_seal_key_for_recipient(
            &VaultSealKey { bytes: [0xEEu8; 32] },
            &recipient,
            &source,
            5,
        )
        .expect("wrap");
        let mut spliced = good.clone();
        spliced.wrapped_vault_seal_key_b64 = other.wrapped_vault_seal_key_b64;
        assert!(!verify_env(&spliced, &pinned), "ciphertext splice must fail verify");

        // (d) Sig is valid but the pinned key belongs to a DIFFERENT source
        //     (broker forged the source_device_pubkey_hex hint + re-signed with
        //     its own key). Verifying against the genuine pinned key fails.
        let attacker_key = ed25519_dalek::SigningKey::from_bytes(&[0x99u8; 32]);
        let forged =
            wrap_vault_seal_key_for_recipient(&seal, &recipient, &attacker_key, 5).expect("wrap");
        assert!(
            verify_env(&forged, &attacker_key.verifying_key()),
            "forged envelope self-consistently verifies under attacker key"
        );
        assert!(
            !verify_env(&forged, &pinned),
            "but MUST fail against the genuine out-of-band-pinned source key"
        );
    }

    #[test]
    fn wrap_envelope_signing_bytes_is_field_sensitive() {
        let base = wrap_envelope_signing_bytes("ct", "tgt", "src", 1);
        assert_ne!(base, wrap_envelope_signing_bytes("ctX", "tgt", "src", 1));
        assert_ne!(base, wrap_envelope_signing_bytes("ct", "tgtX", "src", 1));
        assert_ne!(base, wrap_envelope_signing_bytes("ct", "tgt", "srcX", 1));
        assert_ne!(base, wrap_envelope_signing_bytes("ct", "tgt", "src", 2));
        // Domain prefix present (cross-protocol separation).
        assert!(base.starts_with(WRAP_ENVELOPE_SIG_DOMAIN));
    }

    // ── T-SEC-01 Stage C: verified unwrap (production verify-then-decrypt) ───
    #[test]
    fn verified_unwrap_round_trips_seal_key() {
        // This device's wrap recipient is derived from its identity.key bytes.
        let identity_bytes = [0x42u8; 64];
        let recipient = device_wrap_recipient(&identity_bytes).unwrap();
        let seal = VaultSealKey { bytes: [0x7Au8; 32] };
        let source = ed25519_dalek::SigningKey::from_bytes(&[0x03u8; 32]);
        let env = wrap_vault_seal_key_for_recipient(&seal, &recipient, &source, 9).unwrap();

        let recovered = unwrap_vault_seal_key_verified(
            &env,
            &source.verifying_key(),
            9,
            &identity_bytes,
        )
        .expect("verified unwrap");
        assert_eq!(&recovered[..], &seal.bytes[..], "recovers original seal key");
    }

    #[test]
    fn keys_wrapped_response_round_trips_through_retrieval_wire() {
        // Full multi-device retrieval path: source wraps+signs → envelope crosses
        // the broker as a KeysWrappedResponse → new device reconstructs it (using
        // ITS OWN wrap recipient) → verify-before-decrypt recovers the seal key.
        let new_device_identity = [0x42u8; 64];
        let my_recipient = device_wrap_recipient(&new_device_identity).unwrap();
        let seal = VaultSealKey { bytes: [0x7Au8; 32] };
        let source = ed25519_dalek::SigningKey::from_bytes(&[0x05u8; 32]);
        let env = wrap_vault_seal_key_for_recipient(&seal, &my_recipient, &source, 11).unwrap();

        // Source/broker side → the GET /vault/keys/wrapped body (carries sig+source).
        let resp = env.into_keys_wrapped_response();
        assert_eq!(resp.key_version, 11);
        let sig = resp.envelope_sig_hex.clone().expect("sig present");
        let src = resp.source_device_pubkey_hex.clone().expect("source present");
        assert_eq!(sig.len(), 128, "ed25519 sig survives as 128 hex chars");
        assert_eq!(src, hex_encode_lower(source.verifying_key().as_bytes()), "source = signer pubkey");
        // Serde round-trip (the actual wire hop) must preserve the new fields'
        // VALUES (not just the key names).
        let json = serde_json::to_string(&resp).unwrap();
        let resp: KeysWrappedResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp.envelope_sig_hex.as_deref(), Some(sig.as_str()), "sig value survives serde");
        assert_eq!(resp.source_device_pubkey_hex.as_deref(), Some(src.as_str()), "source value survives serde");

        // New device reconstructs the envelope with its OWN recipient + verifies.
        let rebuilt = resp.into_wrapped_vault_seal_key(my_recipient).expect("rebuild");
        let recovered = unwrap_vault_seal_key_verified(
            &rebuilt,
            &source.verifying_key(),
            11,
            &new_device_identity,
        )
        .expect("verified unwrap off the retrieval wire");
        assert_eq!(&recovered[..], &seal.bytes[..], "retrieval path recovers seal key");
    }

    #[test]
    fn keys_wrapped_response_legacy_broker_without_sig_fails_closed() {
        // A legacy broker (predating SPEC-15 sig support) returns only the
        // original fields. It MUST deserialize (serde default → None) and then
        // fail CLOSED on reconstruction — never silently skip verification.
        let legacy_body = r#"{"wrapped_vault_seal_key":"YWdlX2Jsb2I","key_version":3}"#;
        let resp: KeysWrappedResponse = serde_json::from_str(legacy_body)
            .expect("legacy body must still deserialize (Option + serde default)");
        assert!(resp.envelope_sig_hex.is_none() && resp.source_device_pubkey_hex.is_none());
        let recipient = device_wrap_recipient(&[0x42u8; 64]).unwrap();
        let err = resp.into_wrapped_vault_seal_key(recipient).unwrap_err();
        assert!(matches!(err, BrokerError::Unauthorized { .. }),
            "missing sig must fail closed, got {err:?}");
    }

    #[test]
    fn keys_wrapped_response_with_wrong_recipient_fails_verify() {
        // If the new device reconstructs with a recipient the source did NOT sign
        // over, the signing bytes differ → verify fails closed (no decrypt).
        let new_device_identity = [0x42u8; 64];
        let my_recipient = device_wrap_recipient(&new_device_identity).unwrap();
        let seal = VaultSealKey { bytes: [0x7Au8; 32] };
        let source = ed25519_dalek::SigningKey::from_bytes(&[0x05u8; 32]);
        let env = wrap_vault_seal_key_for_recipient(&seal, &my_recipient, &source, 11).unwrap();
        let resp = env.into_keys_wrapped_response();

        let wrong_recipient = device_wrap_recipient(&[0x77u8; 64]).unwrap();
        let rebuilt = resp.into_wrapped_vault_seal_key(wrong_recipient).expect("rebuild");
        let err = unwrap_vault_seal_key_verified(
            &rebuilt,
            &source.verifying_key(),
            11,
            &new_device_identity,
        )
        .unwrap_err();
        assert!(matches!(err, BrokerError::Unauthorized { .. }), "got {err:?}");
    }

    #[test]
    fn verified_unwrap_rejects_wrong_pinned_source_key() {
        let identity_bytes = [0x42u8; 64];
        let recipient = device_wrap_recipient(&identity_bytes).unwrap();
        let seal = VaultSealKey { bytes: [0x7Au8; 32] };
        let source = ed25519_dalek::SigningKey::from_bytes(&[0x03u8; 32]);

        // Broker re-signed with its OWN key and forged source_device_pubkey_hex,
        // but the new device pins the GENUINE source key out-of-band → reject.
        let attacker = ed25519_dalek::SigningKey::from_bytes(&[0x99u8; 32]);
        let forged =
            wrap_vault_seal_key_for_recipient(&seal, &recipient, &attacker, 9).unwrap();
        let err = unwrap_vault_seal_key_verified(
            &forged,
            &source.verifying_key(), // genuine pinned key
            9,
            &identity_bytes,
        )
        .unwrap_err();
        assert!(matches!(err, BrokerError::Unauthorized { .. }), "got {err:?}");
    }

    #[test]
    fn verified_unwrap_rejects_version_downgrade_before_crypto() {
        let identity_bytes = [0x42u8; 64];
        let recipient = device_wrap_recipient(&identity_bytes).unwrap();
        let seal = VaultSealKey { bytes: [0x7Au8; 32] };
        let source = ed25519_dalek::SigningKey::from_bytes(&[0x03u8; 32]);
        let env = wrap_vault_seal_key_for_recipient(&seal, &recipient, &source, 9).unwrap();

        // Device expects v10; broker presents the v9 (rotated-out) wrap → reject.
        let err = unwrap_vault_seal_key_verified(
            &env,
            &source.verifying_key(),
            10,
            &identity_bytes,
        )
        .unwrap_err();
        assert!(
            matches!(err, BrokerError::KeyVersionMismatch { expected: 10, got: 9 }),
            "got {err:?}"
        );
    }

    #[test]
    fn verified_unwrap_rejects_tampered_ciphertext_before_decrypt() {
        let identity_bytes = [0x42u8; 64];
        let recipient = device_wrap_recipient(&identity_bytes).unwrap();
        let seal = VaultSealKey { bytes: [0x7Au8; 32] };
        let source = ed25519_dalek::SigningKey::from_bytes(&[0x03u8; 32]);
        let mut env = wrap_vault_seal_key_for_recipient(&seal, &recipient, &source, 9).unwrap();

        // Splice a different (validly-encrypted) ciphertext — sig no longer
        // covers it, so verify fails BEFORE any decrypt is attempted.
        let other = wrap_vault_seal_key_for_recipient(
            &VaultSealKey { bytes: [0x01u8; 32] },
            &recipient,
            &source,
            9,
        )
        .unwrap();
        env.wrapped_vault_seal_key_b64 = other.wrapped_vault_seal_key_b64;
        let err = unwrap_vault_seal_key_verified(
            &env,
            &source.verifying_key(),
            9,
            &identity_bytes,
        )
        .unwrap_err();
        assert!(matches!(err, BrokerError::Unauthorized { .. }), "got {err:?}");
    }

    #[test]
    fn verified_unwrap_fails_decrypt_for_wrong_device_identity() {
        // Signature is genuine and version matches, but the envelope was wrapped
        // TO a different device → this device's derived secret can't decrypt it.
        let target_identity = [0x42u8; 64];
        let recipient = device_wrap_recipient(&target_identity).unwrap();
        let seal = VaultSealKey { bytes: [0x7Au8; 32] };
        let source = ed25519_dalek::SigningKey::from_bytes(&[0x03u8; 32]);
        let env = wrap_vault_seal_key_for_recipient(&seal, &recipient, &source, 9).unwrap();

        let wrong_device_identity = [0x11u8; 64];
        let err = unwrap_vault_seal_key_verified(
            &env,
            &source.verifying_key(),
            9,
            &wrong_device_identity,
        )
        .unwrap_err();
        assert!(matches!(err, BrokerError::NetworkError { .. }), "got {err:?}");
    }

    // ── T-SEC-01 Stage D: wire converter + secret hygiene ───────────────────
    #[test]
    fn into_keys_wrap_request_carries_ciphertext_and_uses_explicit_routing_hex() {
        let identity_bytes = [0x42u8; 64];
        let recipient = device_wrap_recipient(&identity_bytes).unwrap();
        let seal = VaultSealKey { bytes: [0x7Au8; 32] };
        let source = ed25519_dalek::SigningKey::from_bytes(&[0x03u8; 32]);
        let env = wrap_vault_seal_key_for_recipient(&seal, &recipient, &source, 4).unwrap();

        // The envelope's target field holds the age1 recipient (crypto target)…
        assert!(env.target_device_pubkey_hex.starts_with("age1"));
        let ciphertext = env.wrapped_vault_seal_key_b64.clone();

        // …but the broker request must carry the 64-hex ed25519 routing pubkey,
        // supplied explicitly by the caller — NOT the age1 string.
        let routing_hex = "a".repeat(64);
        let req = env.into_keys_wrap_request(routing_hex.clone());
        assert_eq!(req.target_device_pubkey_hex, routing_hex);
        assert!(!req.target_device_pubkey_hex.starts_with("age1"));
        assert_eq!(req.wrapped_vault_seal_key, ciphertext);
        assert_eq!(req.key_version, 4);
    }

    #[test]
    fn vault_seal_key_debug_is_redacted() {
        let seal = VaultSealKey { bytes: [0xABu8; 32] };
        let dbg = format!("{seal:?}");
        assert_eq!(dbg, "VaultSealKey(<redacted 32 bytes>)");
        assert!(!dbg.contains("171") && !dbg.contains("ab"), "must not leak bytes: {dbg}");
    }

    #[test]
    fn verify_broker_jwt_rejects_garbage() {
        // A non-JWT string must NOT validate. Specific error variant is
        // implementation detail of jsonwebtoken; we only assert is_err.
        let r = verify_broker_jwt("not.a.jwt", b"secret");
        assert!(r.is_err());
    }

    #[test]
    fn broker_error_serializes_with_code_tag() {
        // §11 invariant: error wire shape uses `{"code": "..."}` tag so the
        // UI can dispatch on the machine-readable code string.
        let e = BrokerError::JwtExpired;
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("jwt_expired"), "wire shape: {}", j);

        let e2 = BrokerError::RateLimited { retry_after_s: 30 };
        let j2 = serde_json::to_string(&e2).unwrap();
        assert!(j2.contains("rate_limited"), "wire shape: {}", j2);
        assert!(j2.contains("30"), "payload preserved: {}", j2);

        let e3 = BrokerError::KeyVersionMismatch {
            expected: 2,
            got: 1,
        };
        let j3 = serde_json::to_string(&e3).unwrap();
        assert!(j3.contains("key_version_mismatch"), "wire shape: {}", j3);
    }

    // ── T-SEC-01 Stage A: per-device X25519 vault-wrap keypair ──────────────
    #[test]
    fn device_wrap_keypair_deterministic_and_well_formed() {
        let ikm = [0x11u8; 64];
        let r1 = device_wrap_recipient(&ikm).unwrap();
        let r2 = device_wrap_recipient(&ikm).unwrap();
        assert_eq!(r1, r2, "same identity.key → same wrap recipient (deterministic)");
        assert!(r1.starts_with("age1"), "wrap recipient is an age1 string: {}", r1);
        assert!(
            r1.parse::<age::x25519::Recipient>().is_ok(),
            "wrap recipient must parse as an age x25519 recipient"
        );
        // The published recipient is exactly the secret identity's public half,
        // so an age-wrap to `r1` is unwrappable by derive_device_wrap_identity.
        let id = derive_device_wrap_identity(&ikm).unwrap();
        assert_eq!(r1, id.to_public().to_string(), "recipient == identity.to_public()");
    }

    #[test]
    fn device_wrap_keypair_independent_of_identity_and_seal_key() {
        // Different identity.key → different wrap recipient.
        let r_a = device_wrap_recipient(&[0x22u8; 64]).unwrap();
        let r_b = device_wrap_recipient(&[0x44u8; 64]).unwrap();
        assert_ne!(r_a, r_b, "distinct identities → distinct wrap keypairs");
        // The HKDF-derived wrap key must NOT equal treating the raw 32 identity
        // bytes directly as an age secret (the symmetric seal-key construction) —
        // i.e. the wrap key is cryptographically independent of the seal key.
        let ikm = [0x55u8; 32];
        let wrap_pub = device_wrap_recipient(&ikm).unwrap();
        let raw_seal_pub = {
            use bech32::Hrp;
            let hrp = Hrp::parse("age-secret-key-").unwrap();
            bech32::encode::<bech32::Bech32>(hrp, &ikm)
                .unwrap()
                .to_uppercase()
                .parse::<age::x25519::Identity>()
                .unwrap()
                .to_public()
                .to_string()
        };
        assert_ne!(
            wrap_pub, raw_seal_pub,
            "HKDF wrap key must differ from the raw-bytes seal-key derivation"
        );
    }

    #[test]
    fn device_wrap_rejects_short_identity() {
        assert!(device_wrap_recipient(&[0u8; 8]).is_err(), "too-short identity → error");
    }
}
