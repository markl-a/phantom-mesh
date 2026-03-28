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

/// All capabilities detected on this node.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeCapabilities {
    pub capabilities: Vec<Capability>,
}

impl NodeCapabilities {
    pub fn new() -> Self {
        Self {
            capabilities: Vec::new(),
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
            Capability::GpuCompute {
                api: "cuda".into(),
            },
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
        let deserialized: NodeCapabilities =
            serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.capabilities.len(), 3);
        assert!(deserialized.has(&Capability::CpuCompute));
        assert!(deserialized.has(&Capability::GpuCompute {
            api: "metal".into()
        }));
        assert!(deserialized.has(&Capability::Shell { restricted: false }));
    }
}
