//! Dynamic capability detection — each node detects its hardware capabilities
//! at startup (shell, GPU, sensors, camera, etc.) instead of hardcoding by platform.
//! The capabilities list is registered with the cluster and used for tool routing.

pub mod detector;

use serde::{Deserialize, Serialize};

/// A capability this node supports.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    Shell { restricted: bool },
    Browser,
    CpuCompute,
    GpuCompute { api: String },   // "metal", "vulkan", "directx", "cuda"
    LocalLlm { backend: String }, // "ollama", "coreml", "llamacpp"
    Camera,
    Microphone,
    Gps,
    Accelerometer,
    Gyroscope,
    FileSandbox, // sandboxed file access (mobile)
    FileSystem,  // full filesystem access (desktop)
    Network,
    Npu,
    Stt { engine: String }, // "system", "whisper"
}

impl Capability {
    /// Short string identifier for this capability (used in heartbeats).
    pub fn id(&self) -> String {
        match self {
            Self::Shell { restricted } => {
                if *restricted {
                    "shell_limited".into()
                } else {
                    "shell".into()
                }
            }
            Self::Browser => "browser".into(),
            Self::CpuCompute => "cpu_compute".into(),
            Self::GpuCompute { api } => format!("gpu_compute:{}", api),
            Self::LocalLlm { backend } => format!("local_llm:{}", backend),
            Self::Camera => "camera".into(),
            Self::Microphone => "microphone".into(),
            Self::Gps => "gps".into(),
            Self::Accelerometer => "accelerometer".into(),
            Self::Gyroscope => "gyroscope".into(),
            Self::FileSandbox => "file_sandbox".into(),
            Self::FileSystem => "file_system".into(),
            Self::Network => "network".into(),
            Self::Npu => "npu".into(),
            Self::Stt { engine } => format!("stt:{}", engine),
        }
    }

    /// Parse from short string identifier.
    pub fn from_id(id: &str) -> Option<Self> {
        // Handle compound forms first (contain ':')
        if let Some((prefix, value)) = id.split_once(':') {
            return match prefix {
                "gpu_compute" => Some(Self::GpuCompute {
                    api: value.to_string(),
                }),
                "local_llm" => Some(Self::LocalLlm {
                    backend: value.to_string(),
                }),
                "stt" => Some(Self::Stt {
                    engine: value.to_string(),
                }),
                _ => None,
            };
        }

        // Simple forms (no ':')
        match id {
            "shell" => Some(Self::Shell { restricted: false }),
            "shell_limited" => Some(Self::Shell { restricted: true }),
            "browser" => Some(Self::Browser),
            "cpu_compute" => Some(Self::CpuCompute),
            "camera" => Some(Self::Camera),
            "microphone" => Some(Self::Microphone),
            "gps" => Some(Self::Gps),
            "accelerometer" => Some(Self::Accelerometer),
            "gyroscope" => Some(Self::Gyroscope),
            "file_sandbox" => Some(Self::FileSandbox),
            "file_system" => Some(Self::FileSystem),
            "network" => Some(Self::Network),
            "npu" => Some(Self::Npu),
            _ => None,
        }
    }
}

/// Qualifier indicating the level of access for a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityQualifier {
    /// Full unrestricted access.
    Full,
    /// Limited access (e.g., shell with whitelist).
    Restricted,
    /// Sandboxed access (e.g., mobile file access).
    Sandbox,
    /// Not available.
    None,
}

impl std::fmt::Display for CapabilityQualifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => write!(f, "full"),
            Self::Restricted => write!(f, "restricted"),
            Self::Sandbox => write!(f, "sandbox"),
            Self::None => write!(f, "none"),
        }
    }
}

/// A capability with its qualifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualifiedCapability {
    pub capability: Capability,
    pub qualifier: CapabilityQualifier,
}

impl QualifiedCapability {
    pub fn new(capability: Capability, qualifier: CapabilityQualifier) -> Self {
        Self {
            capability,
            qualifier,
        }
    }

    /// Format for CLI display.
    pub fn display_line(&self) -> String {
        format!("{:<25} {}", self.capability.id(), self.qualifier)
    }
}

/// All capabilities detected on this node.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeCapabilities {
    pub capabilities: Vec<Capability>,
    /// Qualified capabilities with access levels.
    #[serde(default)]
    pub qualified: Vec<QualifiedCapability>,
}

impl NodeCapabilities {
    pub fn new() -> Self {
        Self {
            capabilities: Vec::new(),
            qualified: Vec::new(),
        }
    }

    pub fn add(&mut self, cap: Capability) {
        if !self.capabilities.contains(&cap) {
            self.capabilities.push(cap);
        }
    }

    pub fn has(&self, cap: &Capability) -> bool {
        self.capabilities.contains(cap)
    }

    /// Check if this node can run a tool that requires the given capability string.
    pub fn has_id(&self, id: &str) -> bool {
        self.capabilities.iter().any(|c| c.id() == id)
    }

    /// Get all capability IDs as strings.
    pub fn ids(&self) -> Vec<String> {
        self.capabilities.iter().map(|c| c.id()).collect()
    }

    /// Format capabilities for CLI display.
    pub fn format_display(&self) -> String {
        if self.qualified.is_empty() && self.capabilities.is_empty() {
            return "No capabilities detected.".to_string();
        }

        let mut out = String::new();
        out.push_str(&format!("{:<25} {}\n", "Capability", "Qualifier"));
        out.push_str(&format!("{}\n", "-".repeat(40)));

        if !self.qualified.is_empty() {
            for qc in &self.qualified {
                out.push_str(&format!("{}\n", qc.display_line()));
            }
        } else {
            for cap in &self.capabilities {
                out.push_str(&format!("{:<25} full\n", cap.id()));
            }
        }
        out
    }
}

/// Stable report shape for platform-aware node setup and routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCapabilityReport {
    pub schema_version: u16,
    pub platform: PlatformInfo,
    pub capability_ids: Vec<String>,
    pub capabilities: NodeCapabilities,
}

impl NodeCapabilityReport {
    pub fn detect() -> Self {
        Self::from_capabilities(detector::detect())
    }

    pub fn from_capabilities(capabilities: NodeCapabilities) -> Self {
        let mut capability_ids = capabilities.ids();
        capability_ids.sort();

        Self {
            schema_version: 1,
            platform: PlatformInfo::current(),
            capability_ids,
            capabilities,
        }
    }

    pub fn format_display(&self) -> String {
        let mut out = String::new();
        out.push_str("Node Capability Report\n");
        out.push_str(&format!("Schema: {}\n", self.schema_version));
        out.push_str(&format!(
            "Platform: {} {} ({})\n",
            self.platform.os, self.platform.arch, self.platform.family
        ));
        out.push_str(&format!("Service model: {}\n", self.platform.service_model));
        out.push_str(&format!(
            "Default node mode: {}\n\n",
            self.platform.default_node_mode
        ));
        out.push_str("Capabilities:\n");
        for id in &self.capability_ids {
            out.push_str(&format!("  {}\n", id));
        }
        out
    }
}

/// Current target platform metadata used by setup flows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformInfo {
    pub os: String,
    pub family: String,
    pub arch: String,
    pub service_model: String,
    pub default_node_mode: String,
}

impl PlatformInfo {
    pub fn current() -> Self {
        Self::for_target(
            std::env::consts::OS,
            std::env::consts::FAMILY,
            std::env::consts::ARCH,
        )
    }

    pub fn for_target(os: &str, family: &str, arch: &str) -> Self {
        Self {
            os: os.to_string(),
            family: family.to_string(),
            arch: arch.to_string(),
            service_model: service_model_for_os(os).to_string(),
            default_node_mode: default_node_mode_for_os(os).to_string(),
        }
    }
}

fn service_model_for_os(os: &str) -> &'static str {
    match os {
        "windows" => "windows_service",
        "linux" => "systemd",
        "macos" => "launchd",
        "android" | "ios" => "mobile_bridge",
        _ => "manual",
    }
}

fn default_node_mode_for_os(os: &str) -> &'static str {
    match os {
        "android" | "ios" => "mobile",
        _ => "desktop",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_id_simple() {
        assert_eq!(Capability::Shell { restricted: false }.id(), "shell");
        assert_eq!(Capability::Shell { restricted: true }.id(), "shell_limited");
        assert_eq!(Capability::Browser.id(), "browser");
        assert_eq!(Capability::CpuCompute.id(), "cpu_compute");
        assert_eq!(Capability::Camera.id(), "camera");
        assert_eq!(Capability::Microphone.id(), "microphone");
        assert_eq!(Capability::Gps.id(), "gps");
        assert_eq!(Capability::Accelerometer.id(), "accelerometer");
        assert_eq!(Capability::Gyroscope.id(), "gyroscope");
        assert_eq!(Capability::FileSandbox.id(), "file_sandbox");
        assert_eq!(Capability::FileSystem.id(), "file_system");
        assert_eq!(Capability::Network.id(), "network");
        assert_eq!(Capability::Npu.id(), "npu");
    }

    #[test]
    fn test_capability_id_compound() {
        assert_eq!(
            Capability::GpuCompute {
                api: "metal".into()
            }
            .id(),
            "gpu_compute:metal"
        );
        assert_eq!(
            Capability::LocalLlm {
                backend: "ollama".into()
            }
            .id(),
            "local_llm:ollama"
        );
        assert_eq!(
            Capability::Stt {
                engine: "whisper".into()
            }
            .id(),
            "stt:whisper"
        );
    }

    #[test]
    fn test_from_id_simple() {
        assert_eq!(
            Capability::from_id("shell"),
            Some(Capability::Shell { restricted: false })
        );
        assert_eq!(
            Capability::from_id("shell_limited"),
            Some(Capability::Shell { restricted: true })
        );
        assert_eq!(Capability::from_id("browser"), Some(Capability::Browser));
        assert_eq!(
            Capability::from_id("cpu_compute"),
            Some(Capability::CpuCompute)
        );
        assert_eq!(Capability::from_id("network"), Some(Capability::Network));
        assert_eq!(Capability::from_id("npu"), Some(Capability::Npu));
    }

    #[test]
    fn test_from_id_compound() {
        assert_eq!(
            Capability::from_id("gpu_compute:metal"),
            Some(Capability::GpuCompute {
                api: "metal".into()
            })
        );
        assert_eq!(
            Capability::from_id("local_llm:ollama"),
            Some(Capability::LocalLlm {
                backend: "ollama".into()
            })
        );
        assert_eq!(
            Capability::from_id("stt:whisper"),
            Some(Capability::Stt {
                engine: "whisper".into()
            })
        );
    }

    #[test]
    fn test_from_id_unknown() {
        assert_eq!(Capability::from_id("unknown_thing"), None);
        assert_eq!(Capability::from_id("bad_prefix:value"), None);
        assert_eq!(Capability::from_id(""), None);
    }

    #[test]
    fn test_node_capabilities_has() {
        let mut caps = NodeCapabilities::new();
        caps.add(Capability::CpuCompute);
        caps.add(Capability::Network);

        assert!(caps.has(&Capability::CpuCompute));
        assert!(caps.has(&Capability::Network));
        assert!(!caps.has(&Capability::Browser));
        assert!(!caps.has(&Capability::Camera));
    }

    #[test]
    fn test_node_capabilities_has_id() {
        let mut caps = NodeCapabilities::new();
        caps.add(Capability::GpuCompute {
            api: "metal".into(),
        });

        assert!(caps.has_id("gpu_compute:metal"));
        assert!(!caps.has_id("gpu_compute:vulkan"));
        assert!(!caps.has_id("cpu_compute"));
    }

    #[test]
    fn test_node_capabilities_ids() {
        let mut caps = NodeCapabilities::new();
        caps.add(Capability::CpuCompute);
        caps.add(Capability::Network);
        caps.add(Capability::GpuCompute {
            api: "metal".into(),
        });

        let ids = caps.ids();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&"cpu_compute".to_string()));
        assert!(ids.contains(&"network".to_string()));
        assert!(ids.contains(&"gpu_compute:metal".to_string()));
    }

    #[test]
    fn test_node_capabilities_dedup() {
        let mut caps = NodeCapabilities::new();
        caps.add(Capability::CpuCompute);
        caps.add(Capability::CpuCompute);
        caps.add(Capability::Network);
        caps.add(Capability::Network);

        assert_eq!(caps.capabilities.len(), 2);
    }

    #[test]
    fn test_capability_id_roundtrip() {
        let cases = vec![
            Capability::Shell { restricted: false },
            Capability::Shell { restricted: true },
            Capability::Browser,
            Capability::CpuCompute,
            Capability::GpuCompute {
                api: "metal".into(),
            },
            Capability::GpuCompute {
                api: "vulkan".into(),
            },
            Capability::GpuCompute {
                api: "directx".into(),
            },
            Capability::GpuCompute { api: "cuda".into() },
            Capability::LocalLlm {
                backend: "ollama".into(),
            },
            Capability::LocalLlm {
                backend: "coreml".into(),
            },
            Capability::LocalLlm {
                backend: "llamacpp".into(),
            },
            Capability::Camera,
            Capability::Microphone,
            Capability::Gps,
            Capability::Accelerometer,
            Capability::Gyroscope,
            Capability::FileSandbox,
            Capability::FileSystem,
            Capability::Network,
            Capability::Npu,
            Capability::Stt {
                engine: "system".into(),
            },
            Capability::Stt {
                engine: "whisper".into(),
            },
        ];

        for cap in cases {
            let id = cap.id();
            let roundtripped = Capability::from_id(&id)
                .unwrap_or_else(|| panic!("from_id failed for id '{}' (original: {:?})", id, cap));
            assert_eq!(
                cap, roundtripped,
                "Roundtrip failed: {:?} -> '{}' -> {:?}",
                cap, id, roundtripped
            );
        }
    }

    #[test]
    fn test_serde_roundtrip() {
        let mut caps = NodeCapabilities::new();
        caps.add(Capability::CpuCompute);
        caps.add(Capability::GpuCompute {
            api: "metal".into(),
        });
        caps.add(Capability::Shell { restricted: false });

        let json = serde_json::to_string(&caps).expect("serialize");
        let deserialized: NodeCapabilities = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.capabilities.len(), 3);
        assert!(deserialized.has(&Capability::CpuCompute));
        assert!(deserialized.has(&Capability::GpuCompute {
            api: "metal".into()
        }));
        assert!(deserialized.has(&Capability::Shell { restricted: false }));
    }

    #[test]
    fn test_node_capability_report_sorts_capability_ids() {
        let mut caps = NodeCapabilities::new();
        caps.add(Capability::Network);
        caps.add(Capability::CpuCompute);

        let report = NodeCapabilityReport::from_capabilities(caps);

        assert_eq!(report.schema_version, 1);
        assert_eq!(report.capability_ids, vec!["cpu_compute", "network"]);
        assert_eq!(report.capabilities.capabilities.len(), 2);
        assert!(!report.platform.os.is_empty());
        assert!(!report.platform.arch.is_empty());
    }

    #[test]
    fn test_platform_info_maps_supported_service_models() {
        let cases = [
            ("windows", "windows_service", "desktop"),
            ("linux", "systemd", "desktop"),
            ("macos", "launchd", "desktop"),
            ("android", "mobile_bridge", "mobile"),
            ("ios", "mobile_bridge", "mobile"),
        ];

        for (os, service_model, node_mode) in cases {
            let info = PlatformInfo::for_target(os, "test-family", "test-arch");
            assert_eq!(info.service_model, service_model);
            assert_eq!(info.default_node_mode, node_mode);
        }
    }
}
