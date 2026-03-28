//! Integration tests for the dynamic capability detection system.

use phantom_mesh::capabilities::detector::detect;
use phantom_mesh::capabilities::{Capability, NodeCapabilities};

#[test]
fn test_detect_basic() {
    let caps = detect();
    assert!(
        caps.has(&Capability::CpuCompute),
        "CpuCompute must always be detected"
    );
    assert!(
        caps.has(&Capability::Network),
        "Network must always be detected"
    );
}

#[test]
fn test_detect_shell_on_desktop() {
    // On Windows/Mac/Linux, Shell { restricted: false } should be present.
    if cfg!(target_os = "windows") || cfg!(target_os = "macos") || cfg!(target_os = "linux") {
        let caps = detect();
        assert!(
            caps.has(&Capability::Shell { restricted: false }),
            "Shell should be detected on desktop. Got: {:?}",
            caps.ids()
        );
    }
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
            .unwrap_or_else(|| panic!("from_id('{}') returned None for {:?}", id, cap));
        assert_eq!(
            cap, roundtripped,
            "Roundtrip failed: {:?} -> '{}' -> {:?}",
            cap, id, roundtripped
        );
    }
}

#[test]
fn test_node_capabilities_has() {
    let mut caps = NodeCapabilities::new();
    caps.add(Capability::CpuCompute);
    caps.add(Capability::Network);
    caps.add(Capability::GpuCompute {
        api: "metal".into(),
    });

    assert!(caps.has(&Capability::CpuCompute));
    assert!(caps.has(&Capability::Network));
    assert!(caps.has(&Capability::GpuCompute {
        api: "metal".into()
    }));
    assert!(!caps.has(&Capability::Browser));
    assert!(!caps.has(&Capability::Camera));
    assert!(!caps.has(&Capability::GpuCompute {
        api: "vulkan".into()
    }));
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
    caps.add(Capability::GpuCompute {
        api: "metal".into(),
    });
    caps.add(Capability::GpuCompute {
        api: "metal".into(),
    });

    assert_eq!(
        caps.capabilities.len(),
        3,
        "Duplicate capabilities should be deduplicated"
    );
}

#[test]
fn test_capability_from_id_compound() {
    assert_eq!(
        Capability::from_id("gpu_compute:metal"),
        Some(Capability::GpuCompute {
            api: "metal".into()
        })
    );
    assert_eq!(
        Capability::from_id("gpu_compute:vulkan"),
        Some(Capability::GpuCompute {
            api: "vulkan".into()
        })
    );
    assert_eq!(
        Capability::from_id("gpu_compute:cuda"),
        Some(Capability::GpuCompute {
            api: "cuda".into()
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
    // Unknown compound prefix → None
    assert_eq!(Capability::from_id("unknown_prefix:value"), None);
}

#[test]
fn test_capability_from_id_simple() {
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
    assert_eq!(Capability::from_id("camera"), Some(Capability::Camera));
    assert_eq!(
        Capability::from_id("microphone"),
        Some(Capability::Microphone)
    );
    assert_eq!(Capability::from_id("gps"), Some(Capability::Gps));
    assert_eq!(
        Capability::from_id("accelerometer"),
        Some(Capability::Accelerometer)
    );
    assert_eq!(
        Capability::from_id("gyroscope"),
        Some(Capability::Gyroscope)
    );
    assert_eq!(
        Capability::from_id("file_sandbox"),
        Some(Capability::FileSandbox)
    );
    assert_eq!(
        Capability::from_id("file_system"),
        Some(Capability::FileSystem)
    );
    assert_eq!(Capability::from_id("network"), Some(Capability::Network));
    assert_eq!(Capability::from_id("npu"), Some(Capability::Npu));
    // Unknown simple id → None
    assert_eq!(Capability::from_id("nonexistent"), None);
    assert_eq!(Capability::from_id(""), None);
}
