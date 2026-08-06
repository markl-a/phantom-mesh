//! NodeManifest — formalized, read-only description of a known mesh node.
//!
//! This is the LOCAL / known-peers view: it composes the data the CLI already
//! has on disk (this node's `[cluster]` config from `agents.toml`, the runtime
//! platform/capability detector, and the broker-pulled `peers.json` roster)
//! into one serde-stable struct. It does **not** make network calls — no
//! `/rpc/join`, no heartbeat. `spectyn nodes inspect <node>` and `spectyn nodes
//! caps` render manifests built purely from cached/config state.
//!
//! 中文: NodeManifest 是「已知節點」的唯讀描述（本機 + peers.json 名冊）。
//! 它只組合磁碟上已有的資料（agents.toml 的 [cluster]、runtime 能力偵測、
//! broker 拉下來的 peers.json），不做任何網路呼叫。

use serde::{Deserialize, Serialize};

use crate::cli_config::{read_peers_json, resolve_self_node_name, ClusterPeer};

/// A formalized, serde-stable description of a single mesh node as seen from
/// local/cached state.
///
/// Fields are intentionally `Option` where the source data may be absent (a
/// peer pulled from `peers.json` carries no OS/arch/version — only the local
/// node, which we detect at runtime, has those). `online` is `None` for any
/// node whose liveness we can't determine without a network call (every node
/// here, since this view is offline) — kept in the shape so a future
/// heartbeat lane can populate it without a schema break.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeManifest {
    /// Stable node identifier. For the local node this is its configured
    /// `node_name` (or the resolved hostname); for a peer it is the
    /// `peers.json` `name` (which doubles as the dispatch id).
    pub node_id: String,
    /// Human display name. Same as `node_id` today, but kept distinct so a
    /// future friendly-label field can diverge.
    pub name: String,
    /// `true` when this manifest describes the machine the CLI is running on.
    pub is_local: bool,
    /// Operating system (`std::env::consts::OS`), e.g. `"windows"`,
    /// `"linux"`, `"macos"`. Only known for the local node.
    pub os: Option<String>,
    /// CPU architecture (`std::env::consts::ARCH`), e.g. `"x86_64"`,
    /// `"aarch64"`. Only known for the local node.
    pub arch: Option<String>,
    /// Hostname of the node, when resolvable. Only known for the local node.
    pub hostname: Option<String>,
    /// Capability tags this node advertises. For the local node these come
    /// from `[cluster].capabilities` in `agents.toml`; for a peer they come
    /// from its `peers.json` cache row.
    pub capabilities: Vec<String>,
    /// `spectyn` binary version string. Only known for the local node (same
    /// value as `spectyn --version --short` / `CARGO_PKG_VERSION`).
    pub version: Option<String>,
    /// Base URL / address this node is reachable at. `None` for the local
    /// node (it has no outbound URL in this view); the peer URL otherwise.
    pub base_url: Option<String>,
    /// Liveness, when known WITHOUT a network call. Always `None` in this
    /// offline view; reserved for a future heartbeat lane.
    pub online: Option<bool>,
    /// Last-seen unix-ms timestamp, when known. Always `None` in this offline
    /// view; reserved for a future heartbeat lane.
    pub last_seen_ms: Option<u64>,
}

impl NodeManifest {
    /// The `spectyn` binary version (matches `spectyn --version`).
    pub fn binary_version() -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    /// Build the manifest for the machine the CLI is running on, composing the
    /// runtime platform/capability detector with the configured `[cluster]`
    /// node name + capabilities. No network calls.
    pub fn local() -> Self {
        let platform = crate::capabilities::PlatformInfo::current();
        let name = resolve_self_node_name().unwrap_or_else(|| "local".to_string());
        let hostname = resolve_self_node_name();
        // Configured capability tags advertised to the cluster. Falls back to
        // the empty list when no `[cluster].capabilities` is set.
        let capabilities = local_cluster_capabilities();

        Self {
            node_id: name.clone(),
            name,
            is_local: true,
            os: Some(platform.os),
            arch: Some(platform.arch),
            hostname,
            capabilities,
            version: Some(Self::binary_version()),
            base_url: None,
            online: None,
            last_seen_ms: None,
        }
    }

    /// Build a manifest from a `peers.json` roster row. Peers carry only the
    /// cached name/url/capabilities — OS/arch/version are unknown offline.
    pub fn from_peer(peer: &ClusterPeer) -> Self {
        Self {
            node_id: peer.name.clone(),
            name: peer.label.clone().unwrap_or_else(|| peer.name.clone()),
            is_local: false,
            os: None,
            arch: None,
            hostname: None,
            capabilities: peer.capabilities.clone(),
            version: None,
            base_url: Some(peer.url.clone()),
            online: None,
            last_seen_ms: None,
        }
    }

    /// Render the manifest as a human-readable key/value table. ASCII-safe
    /// (no box-drawing / emoji) so it survives Windows CP950 consoles.
    pub fn render_table(&self) -> String {
        let mut out = String::new();
        let row = |out: &mut String, k: &str, v: &str| {
            out.push_str(&format!("  {:<14} {}\n", k, v));
        };
        row(&mut out, "node_id", &self.node_id);
        row(&mut out, "name", &self.name);
        row(
            &mut out,
            "kind",
            if self.is_local { "local" } else { "peer" },
        );
        row(&mut out, "os", self.os.as_deref().unwrap_or("-"));
        row(&mut out, "arch", self.arch.as_deref().unwrap_or("-"));
        row(&mut out, "hostname", self.hostname.as_deref().unwrap_or("-"));
        row(&mut out, "version", self.version.as_deref().unwrap_or("-"));
        row(&mut out, "base_url", self.base_url.as_deref().unwrap_or("-"));
        row(
            &mut out,
            "online",
            match self.online {
                Some(true) => "yes",
                Some(false) => "no",
                None => "unknown",
            },
        );
        let caps = if self.capabilities.is_empty() {
            "(none)".to_string()
        } else {
            self.capabilities.join(", ")
        };
        row(&mut out, "capabilities", &caps);
        out
    }
}

/// Read this node's advertised capability tags from `[cluster].capabilities`
/// in `agents.toml`. Returns an empty Vec when the file/section is absent —
/// never errors (mirrors the "if it works, use it" config reads elsewhere).
pub fn local_cluster_capabilities() -> Vec<String> {
    let Some(path) = crate::cli_config::agents_toml_path() else {
        return Vec::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(cfg) = toml::from_str::<crate::config::AgentsConfig>(&raw) else {
        return Vec::new();
    };
    cfg.cluster.capabilities
}

/// Resolve a node name (case-insensitive) against the local node + the
/// `peers.json` roster, returning its manifest. The special name `"local"`
/// (or an exact match on the local node's name) resolves to the local
/// manifest. Returns `None` when no known node matches.
pub fn resolve_node(name: &str) -> Option<NodeManifest> {
    let local = NodeManifest::local();
    if name.eq_ignore_ascii_case("local")
        || name.eq_ignore_ascii_case(&local.node_id)
        || name.eq_ignore_ascii_case(&local.name)
    {
        return Some(local);
    }
    let peers = read_peers_json().unwrap_or_default();
    peers
        .iter()
        .find(|p| {
            p.name.eq_ignore_ascii_case(name)
                || p.label
                    .as_deref()
                    .map(|l| l.eq_ignore_ascii_case(name))
                    .unwrap_or(false)
        })
        .map(NodeManifest::from_peer)
}

/// Every known node's manifest: the local node first, then each `peers.json`
/// roster row (excluding any peer whose name collides with the local node).
pub fn all_known_nodes() -> Vec<NodeManifest> {
    let local = NodeManifest::local();
    let local_id = local.node_id.clone();
    let mut out = vec![local];
    for p in read_peers_json().unwrap_or_default() {
        if p.name.eq_ignore_ascii_case(&local_id) {
            continue;
        }
        out.push(NodeManifest::from_peer(&p));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_manifest_has_os_arch_version() {
        let m = NodeManifest::local();
        assert!(m.is_local);
        assert!(m.os.is_some(), "local manifest must carry an OS");
        assert!(m.arch.is_some(), "local manifest must carry an arch");
        assert_eq!(m.version.as_deref(), Some(NodeManifest::binary_version().as_str()));
        assert!(m.base_url.is_none());
    }

    #[test]
    fn from_peer_maps_fields() {
        let peer = ClusterPeer {
            name: "peer-x".into(),
            url: "http://10.0.0.5:7878".into(),
            label: None,
            capabilities: vec!["rust".into(), "cargo".into()],
        };
        let m = NodeManifest::from_peer(&peer);
        assert!(!m.is_local);
        assert_eq!(m.node_id, "peer-x");
        assert_eq!(m.base_url.as_deref(), Some("http://10.0.0.5:7878"));
        assert_eq!(m.capabilities, vec!["rust".to_string(), "cargo".to_string()]);
        assert!(m.os.is_none(), "peer OS is unknown offline");
        assert!(m.version.is_none(), "peer version is unknown offline");
    }

    #[test]
    fn render_table_is_ascii_and_has_fields() {
        let m = NodeManifest::local();
        let t = m.render_table();
        assert!(t.is_ascii(), "table must be ASCII-safe for CP950 consoles");
        assert!(t.contains("node_id"));
        assert!(t.contains("capabilities"));
    }
}
