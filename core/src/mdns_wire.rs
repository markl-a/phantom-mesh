// SPEC-11 §7 — mDNS discovery wire types (single source of truth for the
// `_phantom-mesh._tcp.local.` service: TXT records schema + Rust API surface +
// `mdns:peer` event payload shared with SPEC-17).
//
// Stage 4 (mdns-sd live — all helpers real): the TXT-parsing pipeline
// (`parse_txt` / `require_v_equals_1` / `extract_*` / `parse_peer_os`) and
// the `sha256_hex` truncate helper from Stage 3 stay backed by `std` +
// `sha2` + `hex`. The five previously-pseudocode `mdns-sd`-dependent helpers
// (`build_service_info`, `ensure_service_daemon`, `mdns_register`,
// `mdns_browse`, `dispatch_browse_events`) now use `mdns-sd` 0.13
// `ServiceDaemon` + `ServiceInfo` per §7.3 lifecycle: one shared daemon
// guarded by `std::sync::OnceLock`, register/browse the standard
// `_phantom-mesh._tcp.local.` service, and forward `ServiceEvent` →
// `DiscoveryEvent` through a `std::thread` (mdns-sd is sync, not tokio).
//
// 中文: 本檔對應 SPEC-11 §7（資料模型）。`PeerAdvertisement` 是 mDNS TXT 記錄
// （TXT records，多播 DNS 文字記錄）的 Rust 表達；`PeerOs` enum 用 lower-case
// serde（`mac` / `win` / `linux` / `ios` / `android`）以對齊 spec §7.2 的 wire
// 值。`cluster_id`（叢集識別碼）的 16-hex hash 與 `pubkey fingerprint`（公鑰
// 指紋）的 8-hex short 形式，是 §7.2 規定的「同 LAN 可看見的雜湊而非 secret」
// 設計 — 反推不可行，因此可安全廣播。
//
// TODO post-Stage-4:
//   - wire `dispatch_browse_events` to push `DiscoveryEvent` through a real
//     `crossbeam::channel::Sender` / `tokio::sync::mpsc::Sender` instead of
//     dropping events (current scaffold spawns a thread that consumes events
//     but has no caller-supplied sink — the SPEC-17 §572 Tauri event-bus
//     adapter is the next layer up and lives outside this wire module).
//   - emit `DiscoveryEventPayload` through the Tauri event bus key `mdns:peer`
//     so SPEC-17 §572 contract is fulfilled (mobile-only; desktop uses pure
//     Rust callback per §7.3).

use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use ts_rs::TS;

// ─── §7.2 PeerOs — operating-system enum (lowercase serde per spec) ──────────

/// Operating system tag broadcast as the TXT `os` record. The serde
/// representation MUST be lowercase (`mac` / `win` / `linux` / `ios` /
/// `android`) per §7.2 — any change is a wire-break.
///
/// 中文: 廣播的作業系統標籤。serde 一律小寫，符合 §7.2 表格的 wire 值
/// （`mac`/`win`/`linux`/`ios`/`android`）— 改任何一個字串都是破壞性變更。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/mdns/")]
#[serde(rename_all = "lowercase")]
pub enum PeerOs {
    /// macOS desktop (also covers Mac Catalyst builds on Apple Silicon).
    Mac,
    /// Windows desktop (10 / 11; mDNS via Bonjour service if installed).
    Win,
    /// Linux desktop (avahi-daemon required — see §11 `MDNS_DAEMON_MISSING`).
    Linux,
    /// iOS / iPadOS (uses `NWBrowser`; subject to Local Network privacy prompt).
    Ios,
    /// Android (uses `NsdManager`; requires `CHANGE_WIFI_MULTICAST_STATE`).
    Android,
}

impl PeerOs {
    /// Stable lowercase wire string for the TXT `os` record. Mirrors the
    /// serde representation — keep them in lock-step.
    ///
    /// 中文: TXT `os` 欄位的 wire 字串；與 serde rename_all 保持一致。
    pub const fn wire_str(self) -> &'static str {
        match self {
            PeerOs::Mac => "mac",
            PeerOs::Win => "win",
            PeerOs::Linux => "linux",
            PeerOs::Ios => "ios",
            PeerOs::Android => "android",
        }
    }
}

// ─── §7.2 PeerAdvertisement — the 6 TXT records + SRV/A material ─────────────

/// Single-source-of-truth Rust struct for a mDNS-advertised phantom-mesh peer.
/// Combines the 6 TXT records from §7.2 (`v`, `pf`, `cl`, `ca`, `os`, `na`)
/// with the SRV port and the A/AAAA addresses resolved by `mdns-sd`.
///
/// 中文: mDNS 廣播 peer 的 Rust 結構。前 6 欄是 §7.2 TXT records；`port` 來自
/// SRV 記錄、`addrs` 來自 A/AAAA 記錄。整包 TXT 總長 ≤ 400 bytes（§7.2 警示），
/// 超過會在 Stage 2 由 `parse_txt_records` 回 `MDNS_TXT_OVERSIZE`。
///
/// Wire alignment: Rust / Swift / Kotlin 三平台欄位順序與型別對齊見 §7.7
/// round-trip 表 — 改任何一欄都要同步動三邊 binding。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/mdns/")]
#[serde(rename_all = "camelCase")]
pub struct PeerAdvertisement {
    /// TXT `v` — protocol version (v0.6.0 = `1`).
    pub v: u8,
    /// TXT `pf` — 8-hex pubkey fingerprint = `hex(SHA-256(pubkey)[..4])`.
    /// **Distinct** from the 12-hex SPEC-12 `IdentityPublic.fingerprint`:
    /// different truncation, different purpose (mDNS dedup vs UI display).
    pub pf: String,
    /// TXT `cl` — 16-hex cluster hash = `hex(SHA-256(cluster_secret)[..8])`.
    /// Hashed, not encrypted; LAN-visible but secret cannot be recovered.
    pub cl: String,
    /// TXT `ca` — capability tags, lexicographically sorted, joined on `,`.
    /// Total TXT must stay ≤ 400 bytes per §7.2 — `ca` is the main growth axis.
    pub ca: Vec<String>,
    /// TXT `os` — operating system enum, lowercase wire string.
    pub os: PeerOs,
    /// TXT `na` — UTF-8 nickname (≤ 63 bytes per RFC 6763).
    pub na: String,
    /// SRV record port; default `7878` (shared with `phantom serve`).
    pub port: u16,
    /// Resolved A / AAAA addresses (ts-rs has a built-in `IpAddr` impl that
    /// renders as the string representation on the TS side).
    pub addrs: Vec<IpAddr>,
}

// ─── §7.3 / SPEC-17 §572 — DiscoveryEvent (Rust) + `mdns:peer` wire payload ─

/// Discovery-event kind tag. Wire form is snake_case for the JS event bus
/// (Tauri `emit("mdns:peer", { kind: "peer_added", ... })`) per SPEC-17.
///
/// 中文: discovery 事件種類。serde snake_case，對應 Tauri event bus 的 JSON。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/mdns/")]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryEventKind {
    /// A new peer matching the expected `cl` hash was just discovered.
    PeerAdded,
    /// A previously-known peer sent goodbye (TTL=0) or timed out (120s).
    PeerRemoved,
    /// The browser successfully bound and started its search loop.
    SearchStarted,
    /// The browser stopped (clean stop, not error — see `MdnsError` for that).
    SearchStopped,
}

/// Rust-internal discovery event. The desktop callback in §7.3 receives this
/// variant directly; on mobile it is flattened into `DiscoveryEventPayload`
/// before crossing the Tauri FFI boundary.
///
/// 中文: Rust 內部 discovery 事件。desktop callback 直接收；mobile 會先攤平成
/// `DiscoveryEventPayload`（JSON-shaped）再過 Tauri event bus。
#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    /// New peer found and `cl` filter matched.
    PeerAdded(PeerAdvertisement),
    /// Peer goodbye / TTL expiry — only the instance name is known at this point.
    PeerRemoved { instance_name: String },
    /// Browser entered `running` state per §8 state machine.
    SearchStarted,
    /// Browser exited cleanly.
    SearchStopped,
}

/// Wire payload for the `mdns:peer` Tauri event (SPEC-17 §572). Mobile-only:
/// desktop consumers should subscribe to the Rust callback in §7.3 instead.
///
/// `peer` is `Some` only for `PeerAdded`; `instance_name` is `Some` only for
/// `PeerRemoved`; both are `None` for start / stop events. This shape keeps
/// the TS side as a single discriminated union over `kind`.
///
/// 中文: `mdns:peer` Tauri 事件的 wire payload — 只給 mobile 用。desktop 用
/// §7.3 Rust callback 直接收 enum。`peer` 只在 PeerAdded 時填、`instance_name`
/// 只在 PeerRemoved 時填；其餘事件兩者都 None — TS 端就是一個依 `kind` 分支
/// 的 discriminated union（區分聯合型別）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/mdns/")]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryEventPayload {
    /// Which lifecycle event this is.
    pub kind: DiscoveryEventKind,
    /// Full advertisement for `PeerAdded`; `None` otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer: Option<PeerAdvertisement>,
    /// Service-instance name for `PeerRemoved`; `None` otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_name: Option<String>,
}

// ─── §11 MdnsError — error catalog mirror ────────────────────────────────────

/// Wire-facing error variants for the mDNS discovery subsystem. Mirrors the
/// SPEC-11 §11 error catalog one-to-one; the `code` field is the
/// machine-readable string the UI dispatches on.
///
/// 中文: SPEC-11 §11 error catalog 的 wire-facing 鏡像。`code` tag 是 UI 用
/// 機器可讀的字串去 dispatch error 處理流程；user-copy 不在這邊，由 UI 層
/// 依 `code` 去查 i18n string table。
#[derive(Debug, Clone, Serialize, Deserialize, TS, thiserror::Error)]
#[ts(export, export_to = "../../app/src/lib/generated/mdns/")]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum MdnsError {
    /// iOS Local Network privacy prompt was denied. iOS-only — desktop and
    /// Android never raise this. Recovery: deep-link to Settings or fall back
    /// to manual URL paste.
    #[error("mdns.permission_denied: {detail}")]
    PermissionDenied { detail: String },
    /// UDP 5353 already bound (another mDNS responder is running, e.g. avahi
    /// already on, Bonjour conflict). Retryable after 5s backoff.
    #[error("mdns.bind_fail: {detail}")]
    BindFail { detail: String },
    /// Linux only — `avahi-daemon` not installed / not running.
    #[error("mdns.daemon_missing: {detail}")]
    DaemonMissing { detail: String },
    /// Our own TXT records exceeded the single-packet ceiling (usually `ca`
    /// capability tag list grew too long). User must shorten capabilities.
    #[error("mdns.txt_oversize: {total_bytes} bytes")]
    TxtOversize { total_bytes: usize },
    /// TXT records arrived but couldn't be parsed into `PeerAdvertisement`
    /// (missing required key, wrong type, invalid `os` enum, etc.). Silently
    /// dropped by `cluster_filter.rs` per §8 to avoid noisy logs.
    #[error("mdns.txt_parse_error: {detail}")]
    TxtParseError { detail: String },
}

// ─── Stage 2 helpers — pseudocode bodies (Stage 3 fills inner _pseudo fns) ───
//
// Per docs/superpowers/SPEC-TO-CODE-PLAYBOOK.md:
//   Stage 2 = function body shows what it WILL do via comments + nested
//   unimplemented!() inner helpers. Reader can audit the algorithm flow
//   without trusting any platform-mDNS implementation. Stage 3 swaps the
//   `_pseudo` helpers for real mdns-sd / sha2 calls (added then).

/// Start advertising this peer on `_phantom-mesh._tcp.local.` per §7.1.
///
/// Stage 2 will wire `mdns_sd::ServiceDaemon::register` with the TXT records
/// composed from `ad`'s fields per §7.2 schema, the SRV port from `ad.port`,
/// and the A/AAAA records from `ad.addrs`.
///
/// 中文: 開始廣播此 peer。Stage 2 接 `mdns-sd` ServiceDaemon，TXT/SRV/A 記錄
/// 全部從 `ad` 結構派生。Lifecycle 由 daemon 持有（§7.3）。
pub fn start_advertiser(ad: &PeerAdvertisement) -> Result<(), MdnsError> {
    // Step 1: build the mdns-sd `ServiceInfo` from the `PeerAdvertisement`.
    //         The 6 TXT pairs (v/pf/cl/ca/os/na) come from `ad`'s typed
    //         fields; `ca` is joined comma-separated per §7.2; `os` uses
    //         the lowercase `wire_str()`. SRV port + A/AAAA addrs are also
    //         packed in here so the daemon owns one self-contained record.
    let service_info = build_service_info(ad)?;

    // Step 2: spin up (or reuse) the shared `ServiceDaemon` per §7.3
    //         lifecycle — advertiser + browser share one daemon to keep
    //         UDP-5353 bound exactly once and avoid `MDNS_BIND_FAIL`.
    let daemon = ensure_service_daemon()?;

    // Step 3: register the service so the daemon starts the recurring
    //         announcement loop (initial burst + 1s / 2s / 4s back-off per
    //         RFC 6762 §8.3). Errors here map to `MDNS_BIND_FAIL` /
    //         `MDNS_DAEMON_MISSING` per §11.
    mdns_register(daemon, service_info)
}

/// Start browsing `_phantom-mesh._tcp.local.` and filter by cluster hash.
///
/// `expected_cluster_id_hash` is the 16-hex `cl` value derived via
/// `compute_cluster_id_hash`; mismatched advertisements are silently dropped
/// per §8 transition table (no log — `cl` mismatch is the common case on
/// shared LANs and would otherwise drown the log).
///
/// 中文: 開始 browse 並用 `cl` 雜湊過濾。不符的 advertisement 靜默丟（§8 規定,
/// 不 log — 共用 LAN 下不符是常態，會塞爆 log）。
pub fn start_browser(expected_cluster_id_hash: &str) -> Result<(), MdnsError> {
    // Step 1: get the shared daemon (same one the advertiser uses) so we
    //         do not race two binds on UDP-5353 per §7.3.
    let daemon = ensure_service_daemon()?;

    // Step 2: open a browse channel on `_phantom-mesh._tcp.local.` —
    //         returns an `mdns_sd::Receiver<ServiceEvent>` that emits
    //         `ServiceFound` / `ServiceResolved` / `ServiceRemoved` as
    //         peers come and go on the LAN.
    let event_rx = mdns_browse(daemon)?;

    // Step 3: per-event handler — for each `ServiceResolved` event:
    //         (a) pull TXT pairs out of `ServiceInfo`,
    //         (b) call `parse_txt_records` to get a `PeerAdvertisement`,
    //         (c) drop the record silently (no log) if `parsed.cl` does
    //             not match `expected_cluster_id_hash` — this is the §8
    //             cluster-filter gate that keeps shared-LAN noise out,
    //         (d) emit `DiscoveryEvent::PeerAdded` / `PeerRemoved` /
    //             `SearchStarted` / `SearchStopped` to the caller.
    dispatch_browse_events(event_rx, expected_cluster_id_hash, None)
}

/// Like [`start_browser`], but forwards each matched `DiscoveryEvent` to a
/// caller-supplied `std::sync::mpsc::Sender` instead of dropping it. This is
/// the SPEC-11 §7.3 desktop callback path: a consumer (e.g. the SPEC-17 §572
/// event-bus adapter) creates a channel, hands the `Sender` here, and drains
/// the matching `Receiver`. `mdns-sd` is sync, so the drain runs on a
/// `std::thread`; the thread exits on `SearchStopped` or when the receiver is
/// dropped.
///
/// 中文: 同 `start_browser`，但把過濾後的 `DiscoveryEvent` 透過呼叫方給的
/// `Sender` 往下游送，而不是丟掉 —— 這就是 §7.3 desktop callback 路徑。
pub fn start_browser_with_sink(
    expected_cluster_id_hash: &str,
    sink: std::sync::mpsc::Sender<DiscoveryEvent>,
) -> Result<(), MdnsError> {
    let daemon = ensure_service_daemon()?;
    let event_rx = mdns_browse(daemon)?;
    dispatch_browse_events(event_rx, expected_cluster_id_hash, Some(sink))
}

/// Parse raw TXT key/value pairs into a `PeerAdvertisement`.
///
/// Required keys per §7.2: `v`, `pf`, `cl`, `ca`, `os`, `na`. Any missing
/// required key → `MdnsError::TxtParseError`; total bytes > 400 → also
/// treated as parse error here (advertise-side `MDNS_TXT_OVERSIZE` is enforced
/// in `start_advertiser`).
///
/// Note: this signature takes only the TXT half; SRV port and A/AAAA addresses
/// are filled in by the caller from the `mdns-sd` `ServiceInfo` event.
///
/// 中文: 把 mDNS raw TXT 鍵值對解析成 `PeerAdvertisement`。缺必要欄位回
/// `TxtParseError`；port / addrs 由 caller 從 `ServiceInfo` 補上，不在這層。
pub fn parse_txt_records(raw: &[(String, String)]) -> Result<PeerAdvertisement, MdnsError> {
    // Step 1: turn the `&[(String, String)]` slice into a key→value lookup
    //         so the 6 required-field checks below are O(1) — duplicates
    //         (last-wins) and oversized total bytes both surface here as
    //         `TxtParseError`.
    let txt_map = parse_txt(raw)?;

    // Step 2: validate the version sentinel — `v` MUST be `"1"` for v0.6.0
    //         per §7.2. Any other value → `TxtParseError` (v0.7.0 E2EE will
    //         bump to `"2"` and demand handshake renegotiation).
    require_v_equals_1(&txt_map)?;

    // Step 3: extract the 5 remaining TXT fields (`pf`, `cl`, `ca`, `os`,
    //         `na`). Missing key → `TxtParseError { detail: "missing pf" }`
    //         etc.; `ca` is split on comma + trimmed; `os` is coerced via
    //         `parse_peer_os` to the `PeerOs` enum (unknown string
    //         such as `"windows"` instead of `"win"` → parse error per §7.2).
    let pf = extract_field(&txt_map, "pf")?;
    let cl = extract_field(&txt_map, "cl")?;
    let ca = extract_ca_list(&txt_map)?;
    let os = parse_peer_os(&extract_field(&txt_map, "os")?)?;
    let na = extract_field(&txt_map, "na")?;

    // Step 4: assemble the `PeerAdvertisement` with `port = 0` + empty
    //         `addrs` placeholders — the browser layer fills these from
    //         the `ServiceInfo` SRV + A/AAAA fields after parse returns.
    Ok(PeerAdvertisement {
        v: 1,
        pf,
        cl,
        ca,
        os,
        na,
        port: 0,
        addrs: Vec::new(),
    })
}

/// Compute the 16-hex cluster-id hash for the TXT `cl` record.
///
/// Stage 2 implementation: `hex::encode(&Sha256::digest(cluster_secret)[..8])`.
/// **Do not** confuse with the SPEC-12 12-hex identity fingerprint — different
/// purpose (cluster dedup vs identity display), different truncation length.
///
/// 中文: 算 `cl` 欄位的 16-hex 雜湊。Stage 2 = `hex(sha256(cluster_secret)[..8])`。
/// 不要和 SPEC-12 的 12-hex identity fingerprint 混淆 — 用途和截斷長度都不同。
pub fn compute_cluster_id_hash(cluster_secret: &[u8]) -> String {
    // Step 1: SHA-256 the cluster secret → 32 raw bytes (full digest).
    // Step 2: truncate to the first 8 bytes per §7.2 — 8 bytes = 16 hex
    //         chars, the wire length the TXT `cl` field is defined as.
    // Step 3: lower-case hex encode → final 16-char ASCII string.
    sha256_hex(cluster_secret, 8)
}

/// Compute the 8-hex pubkey-fingerprint short form for the TXT `pf` record.
///
/// Stage 2 implementation: `hex::encode(&Sha256::digest(pubkey)[..4])`.
/// **Do not** confuse with the SPEC-12 12-hex `IdentityPublic.fingerprint`:
/// that one is 6-byte truncation for UI display; this one is 4-byte truncation
/// for mDNS-packet brevity. Both are SHA-256 of the verifying key but neither
/// is interchangeable with the other.
///
/// 中文: 算 `pf` 欄位的 8-hex 短指紋。Stage 2 = `hex(sha256(pubkey)[..4])`。
/// 與 SPEC-12 的 12-hex 指紋不同截斷長度 — 用途不一樣（mDNS 封包瘦身 vs UI
/// 顯示），不可互換。
pub fn compute_pubkey_fingerprint_short(pubkey: &[u8]) -> String {
    // Step 1: SHA-256 the pubkey bytes → 32 raw bytes (full digest).
    // Step 2: truncate to the first 4 bytes per §7.2 — 4 bytes = 8 hex
    //         chars, deliberately shorter than the SPEC-12 12-hex form
    //         to keep total TXT packet size under the §7.2 400-byte cap.
    // Step 3: lower-case hex encode → final 8-char ASCII string.
    sha256_hex(pubkey, 4)
}

// ─── Stage 4 inner helpers — real `mdns-sd` 0.13 calls ───────────────────────
//
// All five previously-pseudocode helpers now use the `mdns-sd` crate
// (added to core/Cargo.toml in commit fb72982). The §7 service type
// `_phantom-mesh._tcp.local.` is the single constant the daemon binds on.

/// SPEC-11 §7.1 — the canonical mDNS service type. Changing this string
/// is a wire-break across all platforms (Rust / Swift / Kotlin).
const SERVICE_TYPE: &str = "_phantom-mesh._tcp.local.";

/// Process-wide shared `ServiceDaemon` per §7.3 — advertiser + browser
/// must share one daemon so UDP-5353 is bound exactly once. `OnceLock`
/// from `std::sync` keeps us off the `once_cell` crate (not in deps).
static SERVICE_DAEMON: std::sync::OnceLock<mdns_sd::ServiceDaemon> =
    std::sync::OnceLock::new();

/// Build the `mdns_sd::ServiceInfo` from a `PeerAdvertisement`. The 6 TXT
/// pairs (v/pf/cl/ca/os/na) come straight from typed fields per §7.2; `ca`
/// is comma-joined; `os` uses the lowercase `wire_str()`. The host_name
/// is synthesised from `na` (sanitised to ASCII-safe + `.local.`) because
/// core has no `hostname` crate — the daemon only needs a valid mDNS host
/// label, not the real OS hostname. Errors from `mdns-sd` (non-ASCII TXT
/// key, malformed IP) surface as `MdnsError::TxtParseError`.
fn build_service_info(ad: &PeerAdvertisement) -> Result<mdns_sd::ServiceInfo, MdnsError> {
    // Compose the 6 TXT records per §7.2. `v` is stringified `u8`; `ca`
    // is lex-sort-friendly comma join; `os` is the lowercase wire string.
    let v_str = ad.v.to_string();
    let ca_joined = ad.ca.join(",");
    let os_str = ad.os.wire_str().to_string();
    let txt_pairs: Vec<(String, String)> = vec![
        ("v".to_string(), v_str),
        ("pf".to_string(), ad.pf.clone()),
        ("cl".to_string(), ad.cl.clone()),
        ("ca".to_string(), ca_joined),
        ("os".to_string(), os_str),
        ("na".to_string(), ad.na.clone()),
    ];

    // Synthesise a mDNS host label from the nickname — strip non-ASCII
    // and whitespace so the daemon does not reject it. Falls back to a
    // deterministic per-fingerprint label if `na` is entirely non-ASCII.
    let host_label: String = ad
        .na
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let host_name = if host_label.is_empty() {
        format!("phantom-{}.local.", ad.pf)
    } else {
        format!("{}.local.", host_label)
    };

    // Collect addrs into a Vec<IpAddr>; an empty Vec is acceptable to
    // mdns-sd (it will discover via interface enumeration at register).
    let addrs: Vec<std::net::IpAddr> = ad.addrs.clone();

    mdns_sd::ServiceInfo::new(
        SERVICE_TYPE,
        &ad.na,
        &host_name,
        &addrs[..],
        ad.port,
        &txt_pairs[..],
    )
    .map_err(|e| MdnsError::TxtParseError {
        detail: format!("ServiceInfo::new: {}", e),
    })
}

/// Get-or-init the shared `ServiceDaemon`. First call constructs one
/// (binding UDP-5353); subsequent calls reuse. IO errors at construction
/// map to `BindFail` (port busy) — `DaemonMissing` is reserved for Linux
/// avahi-daemon checks higher up the stack.
fn ensure_service_daemon() -> Result<&'static mdns_sd::ServiceDaemon, MdnsError> {
    if let Some(d) = SERVICE_DAEMON.get() {
        return Ok(d);
    }
    let daemon = mdns_sd::ServiceDaemon::new().map_err(|e| MdnsError::BindFail {
        detail: format!("ServiceDaemon::new: {}", e),
    })?;
    // Race-safe: if another thread won the init, drop ours and return theirs.
    let _ = SERVICE_DAEMON.set(daemon);
    Ok(SERVICE_DAEMON.get().expect("just initialised"))
}

/// Register a built `ServiceInfo` with the daemon. The daemon owns the
/// announce loop (RFC 6762 §8.3 back-off). Errors map to `BindFail`.
fn mdns_register(
    daemon: &mdns_sd::ServiceDaemon,
    info: mdns_sd::ServiceInfo,
) -> Result<(), MdnsError> {
    daemon.register(info).map_err(|e| MdnsError::BindFail {
        detail: format!("ServiceDaemon::register: {}", e),
    })
}

/// Start browsing `_phantom-mesh._tcp.local.` and return the event channel.
/// The receiver yields `ServiceEvent` variants per `mdns-sd` 0.13.
fn mdns_browse(
    daemon: &mdns_sd::ServiceDaemon,
) -> Result<mdns_sd::Receiver<mdns_sd::ServiceEvent>, MdnsError> {
    daemon.browse(SERVICE_TYPE).map_err(|e| MdnsError::BindFail {
        detail: format!("ServiceDaemon::browse: {}", e),
    })
}

/// Spawn a `std::thread` that drains the `ServiceEvent` receiver, parses
/// each `ServiceResolved` payload via `parse_txt_records`, applies the
/// `cl` cluster filter (silently drops non-matches per §8), and would
/// forward `DiscoveryEvent`s to a downstream sink. The current scaffold
/// has no sink wired — the SPEC-17 §572 Tauri event-bus adapter is the
/// next layer up. Returns immediately after spawning.
///
/// `mdns-sd` 0.13 is sync (its `Receiver` is a `flume::Receiver`), so
/// we use `std::thread` rather than `tokio::spawn` to keep this module
/// runtime-agnostic.
/// Pure reduction of a resolved mDNS service into a `DiscoveryEvent::PeerAdded`,
/// extracted from the browse thread so the §8 parse-error + cluster-filter drops
/// and the SRV-port / A-AAAA-addr backfill are unit-testable WITHOUT a live
/// `mdns_sd::Receiver` or LAN multicast. Returns `None` when the TXT payload
/// fails to parse (§8 silent drop) or advertises a DIFFERENT cluster
/// (cluster-filter drop) — the two branches a live test could never reach.
fn reduce_resolved(
    raw: &[(String, String)],
    port: u16,
    addrs: &[IpAddr],
    expected_cluster_id_hash: &str,
) -> Option<DiscoveryEvent> {
    let mut parsed = parse_txt_records(raw).ok()?; // §8 silent drop on parse error
    if parsed.cl != expected_cluster_id_hash {
        return None; // §8 cluster-filter silent drop
    }
    // Backfill SRV port + A/AAAA addrs from the resolved ServiceInfo so the
    // emitted advertisement is complete.
    parsed.port = port;
    parsed.addrs = addrs.to_vec();
    Some(DiscoveryEvent::PeerAdded(parsed))
}

/// Pure mapping from a single `mdns_sd::ServiceEvent` to the SPEC-11 §8
/// `DiscoveryEvent`, or `None` for the events that produce no observable
/// discovery transition: the §8 silent drops (parse error / cluster mismatch,
/// both inside `reduce_resolved`) and the pre-resolve `ServiceFound` (we wait
/// for `ServiceResolved` to get the TXT payload). Extracted from the browse
/// thread so the event → discovery translation is unit-testable WITHOUT a live
/// `mdns_sd::Receiver` or LAN multicast.
fn map_service_event(
    ev: mdns_sd::ServiceEvent,
    expected_cluster_id_hash: &str,
) -> Option<DiscoveryEvent> {
    match ev {
        mdns_sd::ServiceEvent::ServiceResolved(info) => {
            // Reconstruct the raw TXT pair list from ServiceInfo's property
            // iterator so we can reuse the pure `reduce_resolved` (parse +
            // cluster-filter + SRV/A backfill).
            let raw: Vec<(String, String)> = info
                .get_properties()
                .iter()
                .map(|p| (p.key().to_string(), p.val_str().to_string()))
                .collect();
            let addrs: Vec<IpAddr> = info.get_addresses().iter().copied().collect();
            reduce_resolved(&raw, info.get_port(), &addrs, expected_cluster_id_hash)
        }
        mdns_sd::ServiceEvent::ServiceRemoved(_ty, fullname) => Some(DiscoveryEvent::PeerRemoved {
            instance_name: fullname,
        }),
        mdns_sd::ServiceEvent::SearchStarted(_) => Some(DiscoveryEvent::SearchStarted),
        mdns_sd::ServiceEvent::SearchStopped(_) => Some(DiscoveryEvent::SearchStopped),
        // Pre-resolve event; nothing to emit yet — wait for `ServiceResolved`.
        mdns_sd::ServiceEvent::ServiceFound(_, _) => None,
    }
}

fn dispatch_browse_events(
    events: mdns_sd::Receiver<mdns_sd::ServiceEvent>,
    expected_cluster_id_hash: &str,
    sink: Option<std::sync::mpsc::Sender<DiscoveryEvent>>,
) -> Result<(), MdnsError> {
    let expected = expected_cluster_id_hash.to_string();
    std::thread::spawn(move || {
        while let Ok(ev) = events.recv() {
            // `SearchStopped` ends the drain regardless of whether a sink is
            // wired (§8: clean stop), so latch it before `ev` is consumed.
            let stop = matches!(ev, mdns_sd::ServiceEvent::SearchStopped(_));
            if let Some(emit) = map_service_event(ev, &expected) {
                if let Some(tx) = &sink {
                    // A dropped receiver means the consumer is gone — there is
                    // nothing left to forward to, so stop draining.
                    if tx.send(emit).is_err() {
                        break;
                    }
                }
                // No sink wired (`start_browser`): the event is observed and
                // discarded, preserving the original scaffold behaviour.
            }
            if stop {
                break;
            }
        }
    });
    Ok(())
}

/// Fold raw TXT `&[(String, String)]` pairs into a key→value lookup table.
/// Duplicates use last-wins semantics (RFC 6763 §6.4 leaves this undefined,
/// so we pick the spec-friendly behaviour). Total bytes (sum of all key.len
/// + value.len) > 400 → `MdnsError::TxtParseError` per §7.2 size cap.
fn parse_txt(
    raw: &[(String, String)],
) -> Result<std::collections::HashMap<String, String>, MdnsError> {
    let total_bytes: usize = raw.iter().map(|(k, v)| k.len() + v.len()).sum();
    if total_bytes > 400 {
        return Err(MdnsError::TxtParseError {
            detail: format!("TXT total bytes {} > 400 cap", total_bytes),
        });
    }
    let mut map = std::collections::HashMap::with_capacity(raw.len());
    for (k, v) in raw {
        map.insert(k.clone(), v.clone());
    }
    Ok(map)
}

/// Validate the `v` version sentinel — MUST equal `"1"` for v0.6.0 per §7.2.
/// v0.7.0 E2EE will bump to `"2"` and require handshake renegotiation; until
/// then any non-`"1"` value (or a missing `v` key) maps to `TxtParseError`.
fn require_v_equals_1(
    txt: &std::collections::HashMap<String, String>,
) -> Result<(), MdnsError> {
    match txt.get("v").map(String::as_str) {
        Some("1") => Ok(()),
        Some(other) => Err(MdnsError::TxtParseError {
            detail: format!("v={} (expected \"1\")", other),
        }),
        None => Err(MdnsError::TxtParseError {
            detail: "missing v".to_string(),
        }),
    }
}

/// Pull a single TXT field by key; missing → canonical `TxtParseError`.
fn extract_field(
    txt: &std::collections::HashMap<String, String>,
    key: &str,
) -> Result<String, MdnsError> {
    txt.get(key)
        .cloned()
        .ok_or_else(|| MdnsError::TxtParseError {
            detail: format!("missing {}", key),
        })
}

/// Split the `ca` field on commas, trim whitespace, drop empty tags. A
/// missing `ca` key is treated as `TxtParseError` (§7.2 marks it required —
/// even peers with zero capabilities advertise `ca=` with empty value, which
/// parses to an empty Vec rather than absent).
fn extract_ca_list(
    txt: &std::collections::HashMap<String, String>,
) -> Result<Vec<String>, MdnsError> {
    let raw = txt.get("ca").ok_or_else(|| MdnsError::TxtParseError {
        detail: "missing ca".to_string(),
    })?;
    Ok(raw
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect())
}

/// Coerce the lowercase `os` wire string to the `PeerOs` enum. Unknown
/// strings (e.g. `"windows"` instead of `"win"`) → `TxtParseError` so a
/// drifting peer can't silently slip into a misclassified bucket.
fn parse_peer_os(s: &str) -> Result<PeerOs, MdnsError> {
    match s {
        "mac" => Ok(PeerOs::Mac),
        "win" => Ok(PeerOs::Win),
        "linux" => Ok(PeerOs::Linux),
        "ios" => Ok(PeerOs::Ios),
        "android" => Ok(PeerOs::Android),
        other => Err(MdnsError::TxtParseError {
            detail: format!("unknown os \"{}\"", other),
        }),
    }
}

/// SHA-256 of `bytes`, truncated to the first `truncate_to_bytes` bytes,
/// returned as lower-case hex (2 chars per byte). Used for both the `pf`
/// 8-hex (4-byte) and `cl` 16-hex (8-byte) TXT fingerprints per §7.2.
fn sha256_hex(bytes: &[u8], truncate_to_bytes: usize) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    hex::encode(&digest[..truncate_to_bytes])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn peer_advertisement_round_trip_smoke() {
        // §7.2 invariant: the 6 TXT records + SRV port + A addrs survive
        // serde JSON round-trip. Stage 1 only checks the wire shape; Stage 2
        // will add size-guard + cluster-hash equality checks.
        let ad = PeerAdvertisement {
            v: 1,
            pf: "3f2a91b0".to_string(),
            cl: "b4e7d2a8c1f30569".to_string(),
            ca: vec![
                "always-on".to_string(),
                "gpu".to_string(),
                "vision".to_string(),
            ],
            os: PeerOs::Mac,
            na: "工作室筆電".to_string(),
            port: 7878,
            addrs: vec![IpAddr::V4(Ipv4Addr::new(192, 168, 1, 42))],
        };
        let j = serde_json::to_string(&ad).unwrap();
        // Verify wire field names are camelCase (TS-facing).
        assert!(j.contains("\"v\":1"), "wire shape: {}", j);
        assert!(j.contains("\"pf\":\"3f2a91b0\""), "wire shape: {}", j);
        assert!(j.contains("\"os\":\"mac\""), "lowercase os enum: {}", j);
        let back: PeerAdvertisement = serde_json::from_str(&j).unwrap();
        assert_eq!(ad.v, back.v);
        assert_eq!(ad.pf, back.pf);
        assert_eq!(ad.cl, back.cl);
        assert_eq!(ad.ca, back.ca);
        assert_eq!(ad.os, back.os);
        assert_eq!(ad.na, back.na);
        assert_eq!(ad.port, back.port);
        assert_eq!(ad.addrs, back.addrs);
    }

    #[test]
    fn peer_os_wire_strings_are_lowercase() {
        // §7.2 invariant: TXT `os` values are exactly the 5 lowercase strings.
        // Changing any of these is a wire-break across Rust / Swift / Kotlin.
        assert_eq!(PeerOs::Mac.wire_str(), "mac");
        assert_eq!(PeerOs::Win.wire_str(), "win");
        assert_eq!(PeerOs::Linux.wire_str(), "linux");
        assert_eq!(PeerOs::Ios.wire_str(), "ios");
        assert_eq!(PeerOs::Android.wire_str(), "android");
        // serde rename_all = "lowercase" must agree with wire_str().
        for os in [PeerOs::Mac, PeerOs::Win, PeerOs::Linux, PeerOs::Ios, PeerOs::Android] {
            let j = serde_json::to_string(&os).unwrap();
            assert_eq!(j, format!("\"{}\"", os.wire_str()));
        }
    }

    #[test]
    fn discovery_event_payload_peer_added_shape() {
        // SPEC-17 §572 wire shape: `{ kind: "peer_added", peer: {...} }`
        // with no `instanceName` field present (skip_serializing_if works).
        let ad = PeerAdvertisement {
            v: 1,
            pf: "00000000".to_string(),
            cl: "0000000000000000".to_string(),
            ca: vec![],
            os: PeerOs::Ios,
            na: "phone".to_string(),
            port: 7878,
            addrs: vec![],
        };
        let payload = DiscoveryEventPayload {
            kind: DiscoveryEventKind::PeerAdded,
            peer: Some(ad),
            instance_name: None,
        };
        let j = serde_json::to_string(&payload).unwrap();
        assert!(j.contains("\"kind\":\"peer_added\""), "kind shape: {}", j);
        assert!(j.contains("\"peer\":{"), "peer present: {}", j);
        assert!(!j.contains("instanceName"), "instanceName omitted: {}", j);
    }

    #[test]
    fn mdns_error_serializes_with_code_tag() {
        // §11 invariant: error wire shape uses `{"code": "..."}` tag so the
        // UI can dispatch on the machine-readable code string.
        let e = MdnsError::PermissionDenied {
            detail: "iOS NWBrowser denied".to_string(),
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("permission_denied"), "wire shape: {}", j);

        let e2 = MdnsError::TxtOversize { total_bytes: 9001 };
        let j2 = serde_json::to_string(&e2).unwrap();
        assert!(j2.contains("txt_oversize"), "wire shape: {}", j2);
        assert!(j2.contains("9001"), "payload preserved: {}", j2);
    }

    // ─── Stage 3 KAT (known-answer-test) vectors ─────────────────────────
    //
    // These replace the Stage 2 `compute_cluster_id_hash_panics_at_sha_step`
    // marker now that `sha256_hex` is real. The TXT-parse helpers are also
    // pinned with positive + negative vectors below. The single remaining
    // `#[should_panic(expected = "Stage 4")]` marker tracks the still-
    // pseudocode `mdns-sd`-dependent helpers.

    /// §7.2 — `cl` is exactly 16 hex chars (8 raw bytes truncated SHA-256).
    /// Empty cluster secret → SHA-256("") = `e3b0c442…`, first 16 hex =
    /// `e3b0c44298fc1c14`. Locks the truncation length in place; a future
    /// bump to 24 hex would silently break shared-LAN cluster filtering.
    #[test]
    fn compute_cluster_id_hash_matches_empty_secret_kat() {
        let h = compute_cluster_id_hash(b"");
        assert_eq!(h.len(), 16, "cl must be 16 hex chars per §7.2");
        assert_eq!(h, "e3b0c44298fc1c14", "SHA-256(\"\")[..8] hex pin");
    }

    /// §7.2 — `pf` is exactly 8 hex chars (4 raw bytes truncated SHA-256).
    /// Independent test from `cl` to catch any accidental cross-wire of the
    /// two truncation lengths.
    #[test]
    fn compute_pubkey_fingerprint_short_matches_empty_pubkey_kat() {
        let h = compute_pubkey_fingerprint_short(b"");
        assert_eq!(h.len(), 8, "pf must be 8 hex chars per §7.2");
        assert_eq!(h, "e3b0c442", "SHA-256(\"\")[..4] hex pin");
    }

    /// §7.2 — round-trip a full 6-field TXT record set through `parse_txt_records`
    /// (positive case). The resulting `PeerAdvertisement` should have all 5
    /// non-`v` fields populated; `port` / `addrs` are left as caller-fill
    /// placeholders per the function's contract.
    #[test]
    fn parse_txt_records_round_trip_positive() {
        let raw = vec![
            ("v".to_string(), "1".to_string()),
            ("pf".to_string(), "3f2a91b0".to_string()),
            ("cl".to_string(), "b4e7d2a8c1f30569".to_string()),
            ("ca".to_string(), "always-on,gpu, vision".to_string()),
            ("os".to_string(), "linux".to_string()),
            ("na".to_string(), "desktop".to_string()),
        ];
        let ad = parse_txt_records(&raw).expect("valid TXT must parse");
        assert_eq!(ad.v, 1);
        assert_eq!(ad.pf, "3f2a91b0");
        assert_eq!(ad.cl, "b4e7d2a8c1f30569");
        assert_eq!(ad.ca, vec!["always-on", "gpu", "vision"]); // whitespace trimmed
        assert_eq!(ad.os, PeerOs::Linux);
        assert_eq!(ad.na, "desktop");
        assert_eq!(ad.port, 0, "port is caller-filled by SRV layer");
        assert!(ad.addrs.is_empty(), "addrs is caller-filled by A/AAAA layer");
    }

    fn valid_raw(cl: &str) -> Vec<(String, String)> {
        vec![
            ("v".to_string(), "1".to_string()),
            ("pf".to_string(), "3f2a91b0".to_string()),
            ("cl".to_string(), cl.to_string()),
            ("ca".to_string(), "always-on,gpu".to_string()),
            ("os".to_string(), "linux".to_string()),
            ("na".to_string(), "desktop".to_string()),
        ]
    }

    #[test]
    fn reduce_resolved_accepts_matching_cluster() {
        // Same cluster → PeerAdded with the SRV port + A/AAAA addrs backfilled.
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        let out = reduce_resolved(&valid_raw("b4e7d2a8c1f30569"), 7878, &[ip], "b4e7d2a8c1f30569");
        match out {
            Some(DiscoveryEvent::PeerAdded(p)) => {
                assert_eq!(p.cl, "b4e7d2a8c1f30569");
                assert_eq!(p.port, 7878, "SRV port backfilled");
                assert_eq!(p.addrs, vec![ip], "A/AAAA addrs backfilled");
                assert_eq!(p.na, "desktop");
            }
            other => panic!("expected PeerAdded, got {other:?}"),
        }
    }

    #[test]
    fn reduce_resolved_drops_cluster_mismatch() {
        // §8 cluster-filter: a peer on a DIFFERENT cluster is silently dropped.
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        let out = reduce_resolved(&valid_raw("b4e7d2a8c1f30569"), 7878, &[ip], "0000000000000000");
        assert!(out.is_none(), "mismatched cluster must drop to None");
    }

    #[test]
    fn reduce_resolved_drops_parse_error() {
        // §8 parse-error: a TXT payload missing the required `na` key is dropped.
        let mut raw = valid_raw("b4e7d2a8c1f30569");
        raw.retain(|(k, _)| k != "na");
        let out = reduce_resolved(&raw, 7878, &[], "b4e7d2a8c1f30569");
        assert!(out.is_none(), "unparseable TXT must drop to None");
    }

    #[test]
    fn map_service_event_translates_lifecycle_variants() {
        // §8 lifecycle: the non-`ServiceResolved` arms map deterministically
        // without a live daemon (`ServiceResolved` is covered by the
        // `reduce_resolved_*` tests above).
        let cl = "b4e7d2a8c1f30569";

        // ServiceRemoved → PeerRemoved, preserving the instance name.
        let ev = mdns_sd::ServiceEvent::ServiceRemoved(
            "_phantom-mesh._tcp.local.".to_string(),
            "z13._phantom-mesh._tcp.local.".to_string(),
        );
        match map_service_event(ev, cl) {
            Some(DiscoveryEvent::PeerRemoved { instance_name }) => {
                assert_eq!(instance_name, "z13._phantom-mesh._tcp.local.");
            }
            other => panic!("expected PeerRemoved, got {other:?}"),
        }

        // SearchStarted / SearchStopped map to their discovery counterparts.
        assert!(matches!(
            map_service_event(
                mdns_sd::ServiceEvent::SearchStarted("_phantom-mesh._tcp.local.".to_string()),
                cl,
            ),
            Some(DiscoveryEvent::SearchStarted)
        ));
        assert!(matches!(
            map_service_event(
                mdns_sd::ServiceEvent::SearchStopped("_phantom-mesh._tcp.local.".to_string()),
                cl,
            ),
            Some(DiscoveryEvent::SearchStopped)
        ));

        // ServiceFound is pre-resolve: nothing to emit yet.
        let ev = mdns_sd::ServiceEvent::ServiceFound(
            "_phantom-mesh._tcp.local.".to_string(),
            "z13._phantom-mesh._tcp.local.".to_string(),
        );
        assert!(map_service_event(ev, cl).is_none());
    }

    /// §7.2 — wrong `v` version sentinel must reject with `TxtParseError`. A
    /// future v0.7.0 peer broadcasting `v=2` should not be silently accepted
    /// by a v0.6.0 receiver — handshake renegotiation comes first.
    #[test]
    fn parse_txt_records_rejects_wrong_version() {
        let raw = vec![
            ("v".to_string(), "2".to_string()),
            ("pf".to_string(), "00000000".to_string()),
            ("cl".to_string(), "0000000000000000".to_string()),
            ("ca".to_string(), "".to_string()),
            ("os".to_string(), "mac".to_string()),
            ("na".to_string(), "x".to_string()),
        ];
        let err = parse_txt_records(&raw).expect_err("v=2 must reject");
        match err {
            MdnsError::TxtParseError { detail } => {
                assert!(detail.contains("v="), "detail surfaces wrong v: {}", detail)
            }
            other => panic!("expected TxtParseError, got {:?}", other),
        }
    }

    /// §7.2 — unknown `os` enum value (e.g. `"windows"` instead of `"win"`)
    /// must reject so a drifting peer can't quietly slip into a wrong bucket.
    #[test]
    fn parse_txt_records_rejects_unknown_os() {
        let raw = vec![
            ("v".to_string(), "1".to_string()),
            ("pf".to_string(), "00000000".to_string()),
            ("cl".to_string(), "0000000000000000".to_string()),
            ("ca".to_string(), "".to_string()),
            ("os".to_string(), "windows".to_string()),
            ("na".to_string(), "x".to_string()),
        ];
        let err = parse_txt_records(&raw).expect_err("os=windows must reject");
        match err {
            MdnsError::TxtParseError { detail } => {
                assert!(detail.contains("windows"), "detail names bad os: {}", detail)
            }
            other => panic!("expected TxtParseError, got {:?}", other),
        }
    }

    /// §7.2 — total TXT bytes > 400 must reject with `TxtParseError`. The
    /// `ca` capability tag list is the most common growth axis on the wire.
    #[test]
    fn parse_txt_records_rejects_oversize() {
        let huge_ca = "x".repeat(500); // single 500-byte value blows the cap
        let raw = vec![
            ("v".to_string(), "1".to_string()),
            ("ca".to_string(), huge_ca),
        ];
        let err = parse_txt_records(&raw).expect_err("oversize must reject");
        match err {
            MdnsError::TxtParseError { detail } => {
                assert!(detail.contains("400"), "detail mentions 400 cap: {}", detail)
            }
            other => panic!("expected TxtParseError, got {:?}", other),
        }
    }

    // ─── Stage 4 KAT — mdns-sd live construction smoke test ─────────────
    //
    // The previous Stage 3 `#[should_panic(expected = "Stage 4")]` marker
    // is gone — the 5 helpers now have real bodies. Mocking the LAN
    // multicast for a deterministic register+browse round-trip is hard
    // (depends on interface state, firewall, OS), so the live network
    // path stays behind `#[ignore]` and is opted into by humans with
    // `cargo test -- --ignored`.

    /// `ServiceInfo::new` accepts our 6 TXT records + 7878 SRV port + an
    /// empty addrs slice without erroring. Catches signature drift in
    /// future mdns-sd majors and bad TXT key composition (RFC 6763 §6.4
    /// disallows `=` and non-ASCII in keys — all our keys are ASCII).
    #[test]
    fn build_service_info_accepts_minimal_advertisement() {
        let ad = PeerAdvertisement {
            v: 1,
            pf: "00000000".to_string(),
            cl: "0000000000000000".to_string(),
            ca: vec!["always-on".to_string()],
            os: PeerOs::Mac,
            na: "lab-mac".to_string(),
            port: 7878,
            addrs: vec![],
        };
        let info = build_service_info(&ad).expect("ServiceInfo::new must accept §7.2 TXT shape");
        assert_eq!(info.get_port(), 7878, "SRV port round-trips");
        assert_eq!(
            info.get_property_val_str("os"),
            Some("mac"),
            "os TXT round-trips lowercase",
        );
        assert_eq!(
            info.get_property_val_str("cl"),
            Some("0000000000000000"),
            "cl TXT round-trips",
        );
    }

    /// Live mDNS smoke test — binds UDP-5353, registers, browses. Behind
    /// `#[ignore]` because it requires a usable network stack and clean
    /// port 5353 (avahi / Bonjour conflicts). Run with
    /// `cargo test -p phantom-mesh -- --ignored mdns_wire_live`.
    #[test]
    #[ignore]
    fn mdns_wire_live_register_and_browse_smoke() {
        let ad = PeerAdvertisement {
            v: 1,
            pf: "deadbeef".to_string(),
            cl: "feedfacecafebabe".to_string(),
            ca: vec!["always-on".to_string(), "gpu".to_string()],
            os: PeerOs::Mac,
            na: "phantom-live-test".to_string(),
            port: 7878,
            addrs: vec![],
        };
        start_advertiser(&ad).expect("register must succeed on a clean LAN");
        start_browser(&ad.cl).expect("browse must succeed on a clean LAN");
        // No assertion on event reception — a fully deterministic round-trip
        // would need a fake interface; this scaffold only proves the daemon
        // accepts our calls end-to-end.
    }
}
