//! Runtime hardware capability detection.
//!
//! [`detect()`] probes the local environment for available hardware and
//! software features, returning a [`NodeCapabilities`] snapshot suitable for
//! registration with the cluster hub.

use super::{Capability, NodeCapabilities};

/// Detect all available capabilities on this machine.
pub fn detect() -> NodeCapabilities {
    let mut caps = NodeCapabilities::new();

    // Always available
    caps.add(Capability::CpuCompute);
    caps.add(Capability::Network);

    // Shell detection
    detect_shell(&mut caps);

    // File access
    detect_file_access(&mut caps);

    // GPU detection
    detect_gpu(&mut caps);

    // Microphone (best-effort)
    detect_microphone(&mut caps);

    // Browser (always available in Tauri)
    caps.add(Capability::Browser);

    // Local LLM (check for ollama)
    detect_local_llm(&mut caps);

    caps
}

fn detect_shell(caps: &mut NodeCapabilities) {
    // Try running `echo test` — if it works, shell is available
    let result = std::process::Command::new(if cfg!(windows) { "cmd" } else { "sh" })
        .args(if cfg!(windows) {
            &["/C", "echo test"] as &[&str]
        } else {
            &["-c", "echo test"]
        })
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();

    match result {
        Ok(output) if output.status.success() => {
            caps.add(Capability::Shell { restricted: false });
        }
        _ => {
            // On mobile/sandboxed, shell might be restricted or unavailable
        }
    }
}

fn detect_file_access(caps: &mut NodeCapabilities) {
    // On desktop: full filesystem
    // On mobile: sandboxed (detected by platform)
    if cfg!(target_os = "android") || cfg!(target_os = "ios") {
        caps.add(Capability::FileSandbox);
    } else {
        caps.add(Capability::FileSystem);
    }
}

fn detect_gpu(caps: &mut NodeCapabilities) {
    // Platform-specific GPU detection
    if cfg!(target_os = "macos") {
        caps.add(Capability::GpuCompute {
            api: "metal".into(),
        });
    } else if cfg!(target_os = "windows") {
        // Could be DirectX, Vulkan, or CUDA
        // For now, just mark as directx (most common on Windows)
        caps.add(Capability::GpuCompute {
            api: "directx".into(),
        });
    } else if cfg!(target_os = "linux") {
        caps.add(Capability::GpuCompute {
            api: "vulkan".into(),
        });
    }
    // Android: vulkan, iOS: metal — handled by cfg
    if cfg!(target_os = "android") {
        caps.add(Capability::GpuCompute {
            api: "vulkan".into(),
        });
    }
    if cfg!(target_os = "ios") {
        caps.add(Capability::GpuCompute {
            api: "metal".into(),
        });
    }
}

fn detect_microphone(caps: &mut NodeCapabilities) {
    // On desktop: assume mic available (most machines have one)
    // This is a best-effort heuristic — real check would need audio API
    caps.add(Capability::Microphone);
}

fn detect_local_llm(caps: &mut NodeCapabilities) {
    // Check if ollama is available
    let result = std::process::Command::new("ollama")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();

    if let Ok(output) = result {
        if output.status.success() {
            caps.add(Capability::LocalLlm {
                backend: "ollama".into(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::Capability;

    #[test]
    fn test_detect_basic() {
        let caps = detect();
        // CpuCompute and Network are always added
        assert!(
            caps.has(&Capability::CpuCompute),
            "CpuCompute should always be detected"
        );
        assert!(
            caps.has(&Capability::Network),
            "Network should always be detected"
        );
    }

    #[test]
    fn test_detect_shell_on_desktop() {
        // On Windows/Mac/Linux desktop, shell should be available
        if cfg!(target_os = "windows") || cfg!(target_os = "macos") || cfg!(target_os = "linux") {
            let caps = detect();
            assert!(
                caps.has(&Capability::Shell { restricted: false }),
                "Shell should be detected on desktop platforms. Got: {:?}",
                caps.ids()
            );
        }
    }

    #[test]
    fn test_detect_filesystem_on_desktop() {
        if cfg!(target_os = "windows") || cfg!(target_os = "macos") || cfg!(target_os = "linux") {
            let caps = detect();
            assert!(
                caps.has(&Capability::FileSystem),
                "FileSystem should be detected on desktop platforms. Got: {:?}",
                caps.ids()
            );
        }
    }

    #[test]
    fn test_detect_gpu_on_desktop() {
        let caps = detect();
        if cfg!(target_os = "windows") {
            assert!(caps.has_id("gpu_compute:directx"));
        } else if cfg!(target_os = "macos") {
            assert!(caps.has_id("gpu_compute:metal"));
        } else if cfg!(target_os = "linux") {
            assert!(caps.has_id("gpu_compute:vulkan"));
        }
    }

    #[test]
    fn test_detect_browser_always_present() {
        let caps = detect();
        assert!(
            caps.has(&Capability::Browser),
            "Browser should always be detected (Tauri)"
        );
    }

    #[test]
    fn test_detect_microphone_always_present() {
        let caps = detect();
        assert!(
            caps.has(&Capability::Microphone),
            "Microphone should always be detected (best-effort heuristic)"
        );
    }

    #[test]
    fn test_detect_no_duplicates() {
        let caps = detect();
        let ids = caps.ids();
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            ids.len(),
            sorted.len(),
            "detect() should not produce duplicate capabilities"
        );
    }
}
