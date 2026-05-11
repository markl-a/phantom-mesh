// core/src/mesh.rs — cluster peer manager
//
// Peers are configured in agents.toml under [cluster]:
//
//   [cluster]
//   peers = ["http://192.168.1.2:7878", "http://100.x.x.5:7878"]
//   cluster_secret = "shared-hmac-key"
//   node_name = "my-node"
//
// ── Future: Tailscale integration ─────────────────────────────────────────────
//
// Each PeerInfo will carry a `tailscale_ip: Option<String>` once we integrate
// with the Tailscale API (or parse `/usr/bin/tailscale status --json`).  When
// present, the mesh will prefer the Tailscale IP over the raw LAN/WAN address
// for stable end-to-end encrypted addressing that survives NAT changes and
// network reconfigurations.  The connection pool would be keyed on the
// Tailscale IP so that a peer's DNS name or public address can change without
// losing the warm connection.
// ──────────────────────────────────────────────────────────────────────────────

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

// ── Config ─────────────────────────────────────────────────────────────────

// Config loaded from agents.toml [cluster] section
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClusterConfig {
    #[serde(default)]
    pub peers: Vec<String>, // e.g. ["http://100.x.x.2:7878"]
    pub cluster_secret: Option<String>,
    pub node_name: Option<String>,
    /// What this node can do, e.g. ["rust", "python", "analysis", "web"]
    /// Used by evolve --distributed to route tasks to capable nodes.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Sandbox-worker capability subset. Empty (default) = full worker
    /// (this node may run any tool, no restriction). Non-empty = only
    /// tasks whose `required_caps` ⊆ this set are dispatched here.
    /// Standard sandbox set per SPEC-FREEZE-V1 §3:
    /// `["file_in_container","memory","web","subagent","llm_local"]`
    /// Set this on iOS Tauri / Android Tauri APK builds via their
    /// bundled `agents.toml`; leave empty on Mac / Windows / Linux /
    /// Termux full workers.
    #[serde(default)]
    pub worker_caps: Vec<String>,
    /// Optional coordinator URL. When set, this node registers itself on startup
    /// and fetches the live peer list from the coordinator instead of relying on
    /// hardcoded `peers`. e.g. "http://coordinator.example.com:7900"
    pub coordinator: Option<String>,
}

// ── Coordinator wire types ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorRegistration {
    pub name: String,
    pub url: String,
    pub capabilities: Vec<String>,
    pub secret_hash: String, // SHA-256(cluster_secret), hex encoded
}

// ── PeerInfo ────────────────────────────────────────────────────────────────
//
// PeerInfo is the richer, persistent record stored on disk and used for health
// tracking and routing decisions.  PeerStatus (below) is the lighter wire type
// returned by /rpc/ping and the cluster status API.

/// Persistent peer record with health-tracking fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    /// Base URL of the peer, e.g. "http://192.168.1.2:7878"
    pub url: String,
    /// Human-readable display name reported by the peer.
    pub name: String,
    /// Semantic version string reported by the peer.
    pub version: String,
    /// Whether the last ping succeeded.
    pub online: bool,
    /// Number of active tasks the peer is currently handling.
    pub active_tasks: u32,
    /// Peer uptime in seconds (self-reported).
    pub uptime_secs: u64,
    /// Unix timestamp (seconds) of the last successful ping.
    pub last_seen_unix: u64,
    /// Capabilities declared by this peer (from their agents.toml).
    #[serde(default)]
    pub capabilities: Vec<String>,

    // ── Health-tracking fields ──────────────────────────────────────────
    /// Wall-clock instant of the last successful ping.
    /// Not serialised to disk — reconstructed on load as `None`.
    #[serde(skip)]
    pub last_seen: Option<Instant>,
    /// How many consecutive RPC failures we have seen since the last success.
    pub consecutive_failures: u32,
}

impl PeerInfo {
    /// Returns `true` when the peer has been seen within `timeout_secs` seconds.
    /// A peer that has never been seen (e.g. just loaded from disk) is always
    /// considered unhealthy until a successful ping updates `last_seen`.
    pub fn is_healthy(&self, timeout_secs: u64) -> bool {
        match self.last_seen {
            Some(instant) => instant.elapsed().as_secs() < timeout_secs,
            None => false,
        }
    }
}

// ── PeerStatus ──────────────────────────────────────────────────────────────

/// Lightweight wire type returned by /rpc/ping and the cluster status API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerStatus {
    pub url: String,
    pub name: String,
    pub version: String,
    pub online: bool,
    pub active_tasks: u32,
    pub uptime_secs: u64,
    pub last_seen: u64, // unix timestamp
    /// Node capabilities declared in agents.toml [cluster] capabilities = [...]
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Sandbox worker capability subset (Squad Pipeline §11.2/§11.5 of
    /// SPEC-FREEZE-V1). Empty = full worker, may run any tool. Non-empty =
    /// only tasks whose `required_caps` ⊆ this set will be dispatched here.
    /// Standard sandbox set: ["file_in_container","memory","web","subagent","llm_local"].
    #[serde(default)]
    pub worker_caps: Vec<String>,
    /// Agents configured on this peer (keys of `[agent.*]`). Coordinator
    /// uses this to plan Squad Pipeline dispatch — it can't ask Z13 to
    /// run "recon-agent" if Z13's agents.toml doesn't have that key.
    /// Populated by the daemon at /rpc/ping time, never persisted.
    #[serde(default)]
    pub agents: Vec<String>,
}

impl From<&PeerInfo> for PeerStatus {
    fn from(p: &PeerInfo) -> Self {
        PeerStatus {
            url: p.url.clone(),
            name: p.name.clone(),
            version: p.version.clone(),
            online: p.online,
            active_tasks: p.active_tasks,
            uptime_secs: p.uptime_secs,
            last_seen: p.last_seen_unix,
            capabilities: p.capabilities.clone(),
            // Cached PeerInfo doesn't track worker_caps / agents — these
            // come from each peer's own /rpc/ping payload, not from the
            // local cache. Default to "full worker, agents unknown".
            worker_caps: Vec::new(),
            agents: Vec::new(),
        }
    }
}

// ── Peer persistence ────────────────────────────────────────────────────────

/// Path to the peers list cache: `~/.phantom-mesh/peers.json`.
fn peers_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".phantom-mesh").join("peers.json"))
}

/// Save the peer list to `~/.phantom-mesh/peers.json`.
/// Best-effort — errors are logged but not propagated.
pub async fn save_peers(peers: &[PeerInfo]) {
    let path = match peers_path() {
        Some(p) => p,
        None => {
            tracing::warn!("save_peers: could not determine home directory");
            return;
        }
    };

    // Ensure the parent directory exists.
    if let Some(dir) = path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(dir).await {
            tracing::warn!("save_peers: failed to create {:?}: {}", dir, e);
            return;
        }
    }

    let json = match serde_json::to_string_pretty(peers) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("save_peers: serialization failed: {}", e);
            return;
        }
    };

    if let Err(e) = tokio::fs::write(&path, json).await {
        tracing::warn!("save_peers: write to {:?} failed: {}", path, e);
    } else {
        tracing::debug!("save_peers: wrote {} peers to {:?}", peers.len(), path);
    }
}

/// Load the peer list from `~/.phantom-mesh/peers.json`.
/// Returns an empty vec if the file does not exist or cannot be parsed.
pub async fn load_peers() -> Vec<PeerInfo> {
    let path = match peers_path() {
        Some(p) => p,
        None => return vec![],
    };

    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(_) => return vec![],
    };

    match serde_json::from_slice::<Vec<PeerInfo>>(&bytes) {
        Ok(mut peers) => {
            // `last_seen` (Instant) is skipped during deserialization; leave as None
            // so callers know these peers need a fresh ping before being trusted.
            for p in &mut peers {
                p.last_seen = None;
            }
            tracing::debug!("load_peers: loaded {} peers from {:?}", peers.len(), path);
            peers
        }
        Err(e) => {
            tracing::warn!("load_peers: parse error in {:?}: {}", path, e);
            vec![]
        }
    }
}

// ── mDNS peer discovery ─────────────────────────────────────────────────────

/// Discover local peers advertised under the `_phantom-mesh._tcp` service type.
///
/// Tries `dns-sd` (macOS / Bonjour) first, then falls back to `avahi-browse`
/// (Linux).  This is a best-effort, fire-and-forget operation — on failure or
/// when neither tool is available the function returns an empty vec.
///
/// Returned strings are peer base-URLs inferred from the TXT record `url=…`
/// field.  If no `url=` field is present the entry is skipped.
pub async fn discover_local_peers() -> Vec<String> {
    fn parse_urls(text: &str) -> Vec<String> {
        text.lines()
            .filter_map(|line| {
                // Look for a `url=http://…` field anywhere in the line.
                let lower = line.to_lowercase();
                let idx = lower.find("url=http")?;
                let rest = &line[idx + 4..]; // skip "url="
                let end = rest
                    .find(|c: char| c.is_whitespace() || c == ';')
                    .unwrap_or(rest.len());
                Some(rest[..end].to_string())
            })
            .collect()
    }

    // macOS: dns-sd -B runs forever; spawn it, drain stdout for ~1500ms,
    // then kill the child. This makes the function self-terminating instead
    // of relying on the caller's tokio::time::timeout(...) wrapper.
    {
        use tokio::io::AsyncReadExt;
        let spawned = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("dns-sd -B _phantom-mesh._tcp local. 2>/dev/null")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn();

        if let Ok(mut child) = spawned {
            let mut buf = Vec::with_capacity(4096);
            if let Some(mut stdout) = child.stdout.take() {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(1500),
                    stdout.read_to_end(&mut buf),
                )
                .await;
            }
            // Best-effort kill — child holds the descriptor open otherwise.
            let _ = child.start_kill();
            let _ = child.wait().await;

            let text = String::from_utf8_lossy(&buf);
            let urls = parse_urls(&text);
            if !urls.is_empty() {
                tracing::debug!("discover_local_peers: found {} peers via dns-sd", urls.len());
                return urls;
            }
        }
    }

    // Linux: avahi-browse -t already self-terminates, so a plain output() is fine.
    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg("avahi-browse -t -r -p _phantom-mesh._tcp 2>/dev/null")
        .output()
        .await;

    if let Ok(out) = output {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            let urls = parse_urls(&text);
            if !urls.is_empty() {
                tracing::debug!(
                    "discover_local_peers: found {} peers via avahi-browse",
                    urls.len()
                );
                return urls;
            }
        }
    }

    tracing::debug!("discover_local_peers: no peers found via mDNS");
    vec![]
}

/// Discover peers on the user's Tailscale tailnet running phantom serve.
///
/// Pipeline:
///   1. `tailscale status --json` — no sudo, requires user to be in a tailnet.
///   2. Collect each online peer's IPv4 TailscaleIP (skip IPv6 — some
///      routers don't reliably forward HTTP over v6 on the tailnet).
///   3. Probe each IP at the standard phantom port for `/info` —
///      `phantom serve` always replies 200 there.
///   4. Return base URLs of peers that responded inside `probe_timeout`.
///
/// Best-effort: returns empty Vec when `tailscale` is missing, the tailnet
/// is in single-machine mode, or no peer answers on either of the probe ports.
///
/// Probes both 7878 (the default) and 7879 (commonly used when 7878 is taken)
/// — these are the two ports the existing cluster nodes use in practice.
pub async fn discover_tailscale_peers() -> Vec<String> {
    discover_tailscale_peers_with(&[7878, 7879], Duration::from_secs(2)).await
}

/// Same as `discover_tailscale_peers` but probes the explicit list of ports
/// with the given per-probe timeout. Used by tests and by callers that
/// know the cluster's ports up front.
pub async fn discover_tailscale_peers_with(ports: &[u16], probe_timeout: Duration) -> Vec<String> {
    let output = tokio::process::Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .await;
    let stdout = match output {
        Ok(o) if o.status.success() => o.stdout,
        Ok(o) => {
            tracing::debug!(
                "discover_tailscale_peers: tailscale CLI returned {}",
                o.status,
            );
            return vec![];
        }
        Err(e) => {
            tracing::debug!("discover_tailscale_peers: tailscale CLI not found: {}", e);
            return vec![];
        }
    };

    let parsed: serde_json::Value = match serde_json::from_slice(&stdout) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("discover_tailscale_peers: bad JSON: {}", e);
            return vec![];
        }
    };

    let ips = extract_tailscale_peer_ips(&parsed);
    if ips.is_empty() {
        tracing::debug!("discover_tailscale_peers: no online tailnet peers reported");
        return vec![];
    }

    let client = match reqwest::Client::builder().timeout(probe_timeout).build() {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    // Cross-product (ip × port) — peers don't advertise their port, so
    // probe each candidate. The first port to answer wins per IP.
    let probes = ips.iter().flat_map(|ip| {
        ports.iter().map(move |port| (ip.clone(), *port))
    }).map(|(ip, port)| {
        let client = client.clone();
        async move {
            let url = format!("http://{}:{}/healthz", ip, port);
            match client.get(&url).send().await {
                Ok(r) if r.status().is_success() => {
                    Some(format!("http://{}:{}", ip, port))
                }
                _ => None,
            }
        }
    });

    let results: Vec<Option<String>> = futures::future::join_all(probes).await;
    // Dedupe: keep first hit per IP (lowest-port wins).
    let mut seen_ips = std::collections::HashSet::new();
    let mut alive = Vec::new();
    for url in results.into_iter().flatten() {
        if let Some(ip_part) = url.strip_prefix("http://").and_then(|s| s.split(':').next()) {
            if seen_ips.insert(ip_part.to_string()) {
                alive.push(url);
            }
        }
    }
    tracing::debug!(
        "discover_tailscale_peers: {}/{} tailnet peers responding (ports {:?})",
        alive.len(),
        ips.len(),
        ports,
    );
    alive
}

/// Pure-fn extractor — split out for unit tests. Walks the JSON shape
/// that `tailscale status --json` produces, returning IPv4 strings of
/// online peers.
fn extract_tailscale_peer_ips(status: &serde_json::Value) -> Vec<String> {
    let mut ips = Vec::new();
    let Some(peers) = status.get("Peer").and_then(|p| p.as_object()) else {
        return ips;
    };
    for peer in peers.values() {
        let online = peer.get("Online").and_then(|v| v.as_bool()).unwrap_or(false);
        if !online {
            continue;
        }
        let Some(addrs) = peer.get("TailscaleIPs").and_then(|v| v.as_array()) else {
            continue;
        };
        for addr in addrs {
            if let Some(s) = addr.as_str() {
                if !s.contains(':') {
                    ips.push(s.to_string());
                }
            }
        }
    }
    ips
}

// ── RPC helper with retry ────────────────────────────────────────────────────

const RPC_MAX_RETRIES: u32 = 2;
const RPC_RETRY_DELAY_MS: u64 = 500;

/// Send a POST request with JSON body to `url`, retrying up to `RPC_MAX_RETRIES`
/// times with a `RPC_RETRY_DELAY_MS` ms delay on connection errors or timeouts.
async fn post_with_retry(
    client: &reqwest::Client,
    url: &str,
    body: String,
    auth_token: Option<&str>,
) -> Result<reqwest::Response, String> {
    let mut last_err = String::new();

    for attempt in 0..=RPC_MAX_RETRIES {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(RPC_RETRY_DELAY_MS)).await;
            tracing::debug!("post_with_retry: attempt {} for {}", attempt + 1, url);
        }

        let mut req = client
            .post(url)
            .header("Content-Type", "application/json")
            .body(body.clone());

        if let Some(token) = auth_token {
            req = req.header("X-Cluster-Auth", token);
        }

        match req.send().await {
            Ok(resp) => return Ok(resp),
            Err(e) => {
                last_err = e.to_string();
                // Only retry on connection / timeout errors.
                if !e.is_connect() && !e.is_timeout() {
                    break;
                }
                tracing::warn!("post_with_retry: transient error (attempt {}): {}", attempt + 1, e);
            }
        }
    }

    Err(last_err)
}

// ── Message routing ─────────────────────────────────────────────────────────

/// Send `message` to the healthiest available peer and return its response.
///
/// Selection policy:
/// 1. Filter to peers where `is_healthy(timeout_secs=30)` is true.
/// 2. Among those, pick the one with the lowest `consecutive_failures`.
/// 3. POST `{"message": message}` to `<peer_url>/rpc/message` with retry.
/// 4. Return the `"output"` field from the JSON response, or `None` on failure.
pub async fn route_to_best_peer(peers: &[PeerInfo], message: &str) -> Option<String> {
    let best = peers
        .iter()
        .filter(|p| p.is_healthy(30))
        .min_by_key(|p| p.consecutive_failures)?;

    let url = format!("{}/rpc/message", best.url.trim_end_matches('/'));
    let body = serde_json::json!({ "message": message }).to_string();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let resp = post_with_retry(&client, &url, body, None).await.ok()?;
    let data: serde_json::Value = resp.json().await.ok()?;
    data["output"].as_str().map(|s| s.to_string())
}

// ── ClusterManager ──────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ClusterManager {
    pub config: ClusterConfig,
    peers: Arc<RwLock<Vec<PeerInfo>>>,
    client: reqwest::Client,
    start_time: Instant,
}

impl ClusterManager {
    pub fn new(config: ClusterConfig) -> Self {
        let peers = config
            .peers
            .iter()
            .map(|url| PeerInfo {
                url: url.clone(),
                name: url.clone(),
                version: "unknown".into(),
                online: false,
                active_tasks: 0,
                uptime_secs: 0,
                last_seen_unix: 0,
                last_seen: None,
                consecutive_failures: 0,
                capabilities: vec![],
            })
            .collect();
        Self {
            config,
            peers: Arc::new(RwLock::new(peers)),
            // 180s ceiling so cross-mesh /rpc/message survives a remote agent
            // run (LLM streaming can take 30-90s for a complex task). Pings
            // hit /rpc/ping which always returns ~instantly so the headroom
            // is harmless for ping_peer; only meaningful for assign / message.
            //
            // If you want a true ping with a tight bound, use the dedicated
            // probe path in `discover_tailscale_peers_with(probe_timeout)`.
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(180))
                .build()
                .unwrap_or_default(),
            start_time: Instant::now(),
        }
    }

    /// Return this node's own status info as a JSON value suitable for /rpc/ping responses.
    pub fn own_info(&self, node_name: &str, version: &str) -> serde_json::Value {
        serde_json::json!({
            "name": node_name,
            "version": version,
            "uptime_secs": self.start_time.elapsed().as_secs(),
            "active_tasks": 0,
            "online": true,
            "capabilities": self.config.capabilities,
        })
    }

    /// Return this node's own PeerStatus, suitable for /rpc/ping serialization.
    /// Uses the configured node_name and the compiled package version.
    pub fn own_peer_status(&self) -> PeerStatus {
        let node_name = self
            .config
            .node_name
            .clone()
            .unwrap_or_else(|| "local".to_string());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        PeerStatus {
            url: String::new(),
            name: node_name,
            version: env!("CARGO_PKG_VERSION").to_string(),
            online: true,
            active_tasks: 0,
            uptime_secs: self.start_time.elapsed().as_secs(),
            last_seen: now,
            capabilities: self.config.capabilities.clone(),
            worker_caps: self.config.worker_caps.clone(),
            // `agents` populated by serve.rs::rpc_ping at request time
            // (it has access to AppState.agent_runtime.config().agent),
            // not from ClusterManager which doesn't see [agent.*] keys.
            agents: Vec::new(),
        }
    }

    /// Kept for backwards compatibility — delegates to `own_peer_status`.
    pub fn own_peer_info(&self) -> PeerStatus {
        self.own_peer_status()
    }

    /// Ping a single peer, update its PeerInfo in the list, and return the result.
    ///
    /// Wrapped in a 5s deadline regardless of `self.client`'s configured
    /// ceiling. The shared client is set to 180s so cross-mesh
    /// `/rpc/message` calls (which can stream a remote LLM turn) survive,
    /// but `/rpc/ping` should never need more than a couple seconds —
    /// without this guard, an offline peer made `phantom peer list` hang
    /// for the full 180s × N peers, which a CLI user reads as "frozen".
    pub async fn ping_peer(&self, url: &str) -> Result<PeerStatus, String> {
        const PING_DEADLINE: Duration = Duration::from_secs(5);
        let ping_url = format!("{}/rpc/ping", url.trim_end_matches('/'));
        let body = String::new();
        let result = match tokio::time::timeout(
            PING_DEADLINE,
            post_with_retry(&self.client, &ping_url, body, None),
        ).await {
            Ok(r) => r,
            Err(_) => Err(format!("ping timeout after {:?}", PING_DEADLINE)),
        };

        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        match result {
            Ok(resp) => {
                let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
                let capabilities = data["capabilities"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();
                let info = PeerInfo {
                    url: url.to_string(),
                    name: data["name"].as_str().unwrap_or(url).to_string(),
                    version: data["version"].as_str().unwrap_or("unknown").to_string(),
                    online: true,
                    active_tasks: data["active_tasks"].as_u64().unwrap_or(0) as u32,
                    uptime_secs: data["uptime_secs"].as_u64().unwrap_or(0),
                    last_seen_unix: now_unix,
                    last_seen: Some(Instant::now()),
                    consecutive_failures: 0,
                    capabilities,
                };
                let status = PeerStatus::from(&info);
                let mut peers = self.peers.write().await;
                if let Some(p) = peers.iter_mut().find(|p| p.url == url) {
                    *p = info;
                }
                Ok(status)
            }
            Err(e) => {
                // Record the failure without resetting last_seen.
                let mut peers = self.peers.write().await;
                if let Some(p) = peers.iter_mut().find(|p| p.url == url) {
                    p.online = false;
                    p.consecutive_failures = p.consecutive_failures.saturating_add(1);
                }
                Err(e)
            }
        }
    }

    /// Refresh all configured peers in parallel and return updated status list.
    pub async fn refresh_all(&self) -> Vec<PeerStatus> {
        let futs: Vec<_> = self.config.peers.iter()
            .map(|url| self.ping_peer(url))
            .collect();
        futures::future::join_all(futs).await;
        self.status().await
    }

    /// Return all peers' current cached status (lightweight wire type).
    pub async fn status(&self) -> Vec<PeerStatus> {
        self.peers.read().await.iter().map(PeerStatus::from).collect()
    }

    /// Return the full PeerInfo records (with health-tracking fields).
    pub async fn peer_infos(&self) -> Vec<PeerInfo> {
        self.peers.read().await.clone()
    }

    /// HMAC-SHA256 auth token: HMAC-SHA256(key=secret, msg=body) encoded as lowercase hex.
    pub fn make_auth_token(&self, body: &str) -> String {
        self.make_auth_token_bytes(body.as_bytes())
    }

    /// Make auth token from raw bytes (used internally for &Bytes compatibility).
    /// Uses real HMAC-SHA256 so it matches `openssl dgst -sha256 -hmac "$SECRET"`.
    /// Panics if `cluster_secret` is not configured — callers must ensure a secret
    /// is set before generating tokens.
    fn make_auth_token_bytes(&self, body: &[u8]) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let secret = self
            .config
            .cluster_secret
            .as_deref()
            .filter(|s| !s.is_empty())
            .expect("cluster_secret must be set before generating auth tokens");
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }

    /// Verify an inbound cluster auth token against the expected HMAC-SHA256.
    /// Accepts `&[u8]` so it works with both `&str` and `axum::body::Bytes`.
    /// Returns `false` immediately when no `cluster_secret` is configured.
    /// Uses constant-time comparison (via the `subtle` crate) to prevent timing attacks.
    pub fn verify_auth(&self, token: &str, body: impl AsRef<[u8]>) -> bool {
        use subtle::ConstantTimeEq;
        // Reject all requests when no secret is configured.
        if self.config.cluster_secret.as_deref().map(|s| s.is_empty()).unwrap_or(true) {
            return false;
        }
        let expected = self.make_auth_token_bytes(body.as_ref());
        bool::from(expected.as_bytes().ct_eq(token.as_bytes()))
    }

    /// Forward a task to the least-loaded online peer and return its output.
    /// Calls `/rpc/message` for a synchronous round-trip.
    pub async fn assign_task_to_best_peer(&self, agent: &str, prompt: &str) -> Option<String> {
        let peers = self.peer_infos().await;
        let best = peers
            .iter()
            .filter(|p| p.online)
            .min_by_key(|p| p.active_tasks)?;

        let body = serde_json::json!({
            "message": prompt,
            "agent": agent,
        })
        .to_string();

        let url = format!("{}/rpc/message", best.url.trim_end_matches('/'));
        let resp = post_with_retry(&self.client, &url, body, None).await.ok()?;
        let data: serde_json::Value = resp.json().await.ok()?;
        data["output"].as_str().map(|s| s.to_string())
    }

    /// Dispatch a task asynchronously to the least-loaded online peer.
    /// Returns the `job_id` for polling via `/rpc/task/status/:id`.
    /// The request is HMAC-auth'd when `cluster_secret` is configured.
    pub async fn assign_task_async(&self, agent: &str, prompt: &str) -> Option<String> {
        let peers = self.peer_infos().await;
        let best = peers
            .iter()
            .filter(|p| p.online)
            .min_by_key(|p| p.active_tasks)?;

        let body = serde_json::json!({ "agent": agent, "prompt": prompt }).to_string();
        let auth_token = if self.config.cluster_secret.as_deref().map(|s| !s.is_empty()).unwrap_or(false) {
            Some(self.make_auth_token(&body))
        } else {
            None
        };

        let url = format!("{}/rpc/task/assign", best.url.trim_end_matches('/'));
        let resp = post_with_retry(&self.client, &url, body, auth_token.as_deref()).await.ok()?;
        let data: serde_json::Value = resp.json().await.ok()?;
        data["job_id"].as_str().map(|s| s.to_string())
    }

    /// Dispatch a task to a specific peer URL (with HMAC auth when configured).
    /// Returns the `job_id` for polling, or None on failure.
    pub async fn assign_task_to_peer(&self, peer_url: &str, agent: &str, prompt: &str) -> Option<String> {
        let body = serde_json::json!({ "agent": agent, "prompt": prompt }).to_string();
        let auth_token = if self.config.cluster_secret.as_deref().map(|s| !s.is_empty()).unwrap_or(false) {
            Some(self.make_auth_token(&body))
        } else {
            None
        };
        let url = format!("{}/rpc/task/assign", peer_url.trim_end_matches('/'));
        let resp = post_with_retry(&self.client, &url, body, auth_token.as_deref()).await.ok()?;
        let data: serde_json::Value = resp.json().await.ok()?;
        data["job_id"].as_str().map(|s| s.to_string())
    }

    /// Poll a job's result from the given peer.  Returns `(status, output)`.
    pub async fn poll_task(&self, peer_url: &str, job_id: &str) -> Option<(String, Option<String>)> {
        let url = format!("{}/rpc/task/status/{}", peer_url.trim_end_matches('/'), job_id);
        let resp = self.client.get(&url).send().await.ok()?;
        let data: serde_json::Value = resp.json().await.ok()?;
        let status = data["status"].as_str()?.to_string();
        let output = data["output"].as_str().map(|s| s.to_string());
        Some((status, output))
    }

    /// Dynamically add a peer URL (from coordinator discovery) if not already known.
    pub async fn add_peer_url(&self, url: &str) {
        let mut peers = self.peers.write().await;
        if !peers.iter().any(|p| p.url == url) {
            peers.push(PeerInfo {
                url: url.to_string(),
                name: url.to_string(),
                version: "unknown".into(),
                online: false,
                active_tasks: 0,
                uptime_secs: 0,
                last_seen_unix: 0,
                last_seen: None,
                consecutive_failures: 0,
                capabilities: vec![],
            });
        }
    }

    // ── Coordinator client ──────────────────────────────────────────────────

    fn secret_hash(secret: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        secret.hash(&mut h);
        format!("{:016x}", h.finish())
    }

    /// Register this node with the coordinator and return Ok(()) on success.
    pub async fn register_with_coordinator(&self, self_url: &str) -> Result<(), String> {
        let coord = match &self.config.coordinator {
            Some(c) => c.clone(),
            None => return Ok(()),
        };
        let secret_hash = self.config.cluster_secret.as_deref()
            .map(Self::secret_hash)
            .unwrap_or_default();
        let reg = CoordinatorRegistration {
            name: self.config.node_name.clone().unwrap_or_else(|| "phantom".into()),
            url: self_url.to_string(),
            capabilities: self.config.capabilities.clone(),
            secret_hash,
        };
        let url = format!("{}/register", coord.trim_end_matches('/'));
        self.client
            .post(&url)
            .header("content-type", "application/json")
            .body(serde_json::to_string(&reg).unwrap_or_default())
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Fetch active peers from the coordinator and add any new ones to our peer list.
    /// Returns the number of new peers added.
    pub async fn fetch_coordinator_peers(&self, self_url: &str) -> usize {
        let coord = match &self.config.coordinator {
            Some(c) => c.clone(),
            None => return 0,
        };
        let secret_hash = self.config.cluster_secret.as_deref()
            .map(Self::secret_hash)
            .unwrap_or_default();
        let url = format!("{}/peers?secret_hash={}", coord.trim_end_matches('/'), secret_hash);
        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(_) => return 0,
        };
        let peers: Vec<CoordinatorRegistration> = match resp.json().await {
            Ok(v) => v,
            Err(_) => return 0,
        };
        let mut added = 0;
        for peer in &peers {
            if peer.url != self_url {
                let before = self.peers.read().await.len();
                self.add_peer_url(&peer.url).await;
                // update capabilities from coordinator info right away
                {
                    let mut list = self.peers.write().await;
                    if let Some(p) = list.iter_mut().find(|p| p.url == peer.url) {
                        if p.capabilities.is_empty() {
                            p.capabilities = peer.capabilities.clone();
                            p.name = peer.name.clone();
                        }
                    }
                }
                if self.peers.read().await.len() > before { added += 1; }
            }
        }
        added
    }
}

impl Default for ClusterManager {
    fn default() -> Self {
        Self::new(ClusterConfig::default())
    }
}

// ── Task assign request/response ───────────────────────────────────────────

/// Inbound payload for POST /rpc/task/assign.
#[derive(Debug, Deserialize)]
pub struct TaskAssignRequest {
    #[serde(default = "default_agent")]
    pub agent: String,
    pub prompt: String,
}

fn default_agent() -> String {
    "master".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_tailscale_peer_ips_filters_offline_and_ipv6() {
        let status = json!({
            "Self": {
                "TailscaleIPs": ["100.64.0.1", "fd7a:115c:a1e0::1"],
                "Online": true
            },
            "Peer": {
                "node-a": {
                    "Online": true,
                    "TailscaleIPs": ["100.64.0.2", "fd7a:115c:a1e0::2"]
                },
                "node-b-offline": {
                    "Online": false,
                    "TailscaleIPs": ["100.64.0.3"]
                },
                "node-c": {
                    "Online": true,
                    "TailscaleIPs": ["100.64.0.4"]
                },
                "node-d-no-online-field": {
                    "TailscaleIPs": ["100.64.0.5"]
                }
            }
        });
        let ips = extract_tailscale_peer_ips(&status);
        // Self is intentionally excluded; only Peer.* online ipv4.
        assert_eq!(ips, vec!["100.64.0.2", "100.64.0.4"]);
    }

    #[test]
    fn extract_tailscale_peer_ips_handles_missing_peer_object() {
        let status = json!({ "Self": { "TailscaleIPs": ["100.64.0.1"] } });
        assert!(extract_tailscale_peer_ips(&status).is_empty());
    }

    #[test]
    fn extract_tailscale_peer_ips_handles_empty_tailnet() {
        let status = json!({ "Peer": {} });
        assert!(extract_tailscale_peer_ips(&status).is_empty());
    }

    #[test]
    fn extract_tailscale_peer_ips_skips_peers_with_no_ips_array() {
        let status = json!({
            "Peer": {
                "weird-node": { "Online": true /* no TailscaleIPs */ }
            }
        });
        assert!(extract_tailscale_peer_ips(&status).is_empty());
    }
}
