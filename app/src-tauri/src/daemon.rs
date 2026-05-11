use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{Manager, State};

/// Managed state that holds the daemon child process handle.
pub struct DaemonState {
    pub process: Mutex<Option<Child>>,
    pub port: u16,
    pub binary_path: Mutex<Option<PathBuf>>,
    /// Watchdog: number of automatic restarts performed
    pub restart_count: AtomicU32,
    /// Watchdog: set to false to disable auto-restart (e.g. user explicitly stopped)
    pub watchdog_enabled: AtomicBool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub running: bool,
    pub pid: Option<u32>,
    pub port: u16,
    pub binary_path: Option<String>,
    pub healthy: bool,
    pub restart_count: u32,
    pub watchdog_enabled: bool,
}

impl DaemonState {
    pub fn new(port: u16) -> Self {
        Self {
            process: Mutex::new(None),
            port,
            binary_path: Mutex::new(None),
            restart_count: AtomicU32::new(0),
            watchdog_enabled: AtomicBool::new(false),
        }
    }

    /// Try to locate the phantom-mesh binary in several well-known locations.
    /// Search order:
    ///   1. Explicit path stored in state (from config)
    ///   2. Tauri sidecar location (same dir as running exe — where bundle puts externalBin)
    ///   3. Relative dev paths: ../../core/target/release and debug (inc. exFAT workaround)
    ///   4. Fall back to bare name (relies on PATH)
    pub fn find_binary(&self) -> PathBuf {
        // 1. Explicit override
        if let Ok(guard) = self.binary_path.lock() {
            if let Some(ref p) = *guard {
                if p.exists() {
                    return p.clone();
                }
            }
        }

        let exe_name = if cfg!(windows) {
            "phantom-mesh.exe"
        } else {
            "phantom-mesh"
        };

        // Tauri sidecar name includes target triple
        let sidecar_name = if cfg!(windows) {
            "phantom-mesh-x86_64-pc-windows-msvc.exe"
        } else if cfg!(target_os = "macos") {
            if cfg!(target_arch = "aarch64") {
                "phantom-mesh-aarch64-apple-darwin"
            } else {
                "phantom-mesh-x86_64-apple-darwin"
            }
        } else {
            "phantom-mesh-x86_64-unknown-linux-gnu"
        };

        // 2. Tauri sidecar: same directory as running executable
        if let Ok(current_exe) = std::env::current_exe() {
            if let Some(dir) = current_exe.parent() {
                // Check sidecar name first (production bundle)
                let sidecar = dir.join(sidecar_name);
                if sidecar.exists() {
                    return sidecar;
                }
                // Also check plain name
                let candidate = dir.join(exe_name);
                if candidate.exists() {
                    return candidate;
                }
            }
        }

        // 3. Dev-time relative paths (from src-tauri/), including exFAT workaround dirs
        let dev_candidates = [
            PathBuf::from("../core/target2/release").join(exe_name),
            PathBuf::from("../core/target2/debug").join(exe_name),
            PathBuf::from("../core/target3/release").join(exe_name),
            PathBuf::from("../core/target3/debug").join(exe_name),
            PathBuf::from("../core/target/release").join(exe_name),
            PathBuf::from("../core/target/debug").join(exe_name),
        ];
        for candidate in &dev_candidates {
            if candidate.exists() {
                return candidate.clone();
            }
        }

        // Also try from the exe dir as base
        if let Ok(current_exe) = std::env::current_exe() {
            if let Some(dir) = current_exe.parent() {
                let dev_candidates_from_exe = [
                    dir.join("../core/target2/release").join(exe_name),
                    dir.join("../core/target2/debug").join(exe_name),
                    dir.join("../core/target3/release").join(exe_name),
                    dir.join("../core/target3/debug").join(exe_name),
                    dir.join("../core/target/release").join(exe_name),
                    dir.join("../core/target/debug").join(exe_name),
                ];
                for candidate in &dev_candidates_from_exe {
                    if candidate.exists() {
                        return candidate.clone();
                    }
                }
            }
        }

        // 4. Bare name — hope it's on PATH
        PathBuf::from(exe_name)
    }

    /// Kill the daemon process if running. Returns true if a process was stopped.
    pub fn kill(&self) -> bool {
        let mut guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ref mut child) = *guard {
            let _ = child.kill();
            let _ = child.wait(); // Reap zombie
            tracing::info!("Daemon process killed");
            *guard = None;
            true
        } else {
            false
        }
    }

    /// Check whether the stored child process is still alive.
    pub fn is_running(&self) -> bool {
        let mut guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
        match *guard {
            Some(ref mut child) => {
                // try_wait: Ok(Some(_)) = exited, Ok(None) = still running, Err = error
                match child.try_wait() {
                    Ok(Some(_status)) => {
                        // Process exited — clean up handle
                        *guard = None;
                        false
                    }
                    Ok(None) => true,
                    Err(_) => {
                        *guard = None;
                        false
                    }
                }
            }
            None => false,
        }
    }

    /// Return the PID of the child process, if running.
    pub fn pid(&self) -> Option<u32> {
        let guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().map(|c| c.id())
    }
}

/// Check whether the daemon HTTP endpoint is healthy.
async fn check_health(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{}/health", port);
    // Use a lightweight one-shot client; GET /health is infrequent so pooling isn't needed here.
    match reqwest::get(&url).await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_state_new() {
        let state = DaemonState::new(7878);
        assert_eq!(state.port, 7878);
        assert!(!state.is_running());
        assert!(state.pid().is_none());
        assert_eq!(state.restart_count.load(Ordering::Relaxed), 0);
        assert!(!state.watchdog_enabled.load(Ordering::Relaxed));
    }

    #[test]
    fn test_daemon_state_kill_when_not_running() {
        let state = DaemonState::new(7878);
        assert!(!state.kill()); // No process to kill
    }

    #[test]
    fn test_find_binary_returns_path() {
        let state = DaemonState::new(7878);
        let binary = state.find_binary();
        // Should always return something (at minimum the bare name fallback)
        let name = binary.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("phantom-mesh"));
    }

    #[test]
    fn test_daemon_info_serialization() {
        let info = DaemonInfo {
            running: true,
            pid: Some(12345),
            port: 7878,
            binary_path: Some("/usr/bin/phantom-mesh".to_string()),
            healthy: true,
            restart_count: 3,
            watchdog_enabled: true,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"running\":true"));
        assert!(json.contains("\"pid\":12345"));
        assert!(json.contains("\"restart_count\":3"));
        assert!(json.contains("\"watchdog_enabled\":true"));
    }

    #[test]
    fn test_set_binary_path() {
        let state = DaemonState::new(7878);
        {
            let mut guard = state.binary_path.lock().unwrap();
            *guard = Some(PathBuf::from("/tmp/phantom-mesh"));
        }
        // find_binary should check explicit path first (but it won't exist, so fallback)
        let binary = state.find_binary();
        assert!(!binary.as_os_str().is_empty());
    }

    #[test]
    fn test_watchdog_toggle() {
        let state = DaemonState::new(7878);
        assert!(!state.watchdog_enabled.load(Ordering::Relaxed));
        state.watchdog_enabled.store(true, Ordering::Relaxed);
        assert!(state.watchdog_enabled.load(Ordering::Relaxed));
    }

    #[test]
    fn test_restart_count_increment() {
        let state = DaemonState::new(7878);
        state.restart_count.fetch_add(1, Ordering::Relaxed);
        state.restart_count.fetch_add(1, Ordering::Relaxed);
        assert_eq!(state.restart_count.load(Ordering::Relaxed), 2);
    }
}

// ─── Internal spawn helper (used by start_daemon and watchdog) ──────────────

/// Spawn the phantom-mesh process. Returns the Child on success.
fn spawn_daemon(binary: &PathBuf, port: u16, config_path: Option<&PathBuf>) -> Result<Child, String> {
    let mut args = vec![
        "--host".to_string(), "0.0.0.0".to_string(),
        "--port".to_string(), port.to_string(),
    ];
    if let Some(cp) = config_path {
        args.push("--config".to_string());
        args.push(cp.to_string_lossy().to_string());
    }
    args.push("daemon".to_string());

    Command::new(binary)
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn phantom-mesh at {:?}: {}", binary, e))
}

// ─── Watchdog — crash auto-restart ──────────────────────────────────────────

/// Maximum automatic restarts before the watchdog backs off.
const MAX_AUTO_RESTARTS: u32 = 5;
/// Interval between watchdog polls (seconds).
const WATCHDOG_INTERVAL_SECS: u64 = 10;
/// Backoff cooldown after MAX_AUTO_RESTARTS is reached (seconds).
const WATCHDOG_COOLDOWN_SECS: u64 = 120;

/// Spawn a background task that monitors the daemon process and restarts it
/// if it dies unexpectedly. The watchdog respects `DaemonState::watchdog_enabled`
/// and stops attempting restarts after `MAX_AUTO_RESTARTS` consecutive failures.
pub fn spawn_watchdog(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(WATCHDOG_INTERVAL_SECS)).await;

            let state = app.state::<DaemonState>();

            // Watchdog disabled (user explicitly stopped daemon)?
            if !state.watchdog_enabled.load(Ordering::Relaxed) {
                continue;
            }

            // Daemon still running? Nothing to do.
            if state.is_running() {
                continue;
            }

            // Daemon died — check restart budget
            let restarts = state.restart_count.load(Ordering::Relaxed);
            if restarts >= MAX_AUTO_RESTARTS {
                tracing::warn!(
                    "Watchdog: daemon crashed {} times — entering cooldown ({}s)",
                    restarts,
                    WATCHDOG_COOLDOWN_SECS,
                );
                tokio::time::sleep(std::time::Duration::from_secs(WATCHDOG_COOLDOWN_SECS)).await;
                // Reset counter after cooldown to allow retries
                state.restart_count.store(0, Ordering::Relaxed);
                continue;
            }

            tracing::warn!("Watchdog: daemon not running — attempting restart #{}", restarts + 1);

            let binary = state.find_binary();
            let port = state.port;
            let config_path = app.path().app_config_dir().ok().map(|d| d.join("agents.toml"));

            match spawn_daemon(&binary, port, config_path.as_ref()) {
                Ok(child) => {
                    let pid = child.id();
                    {
                        let mut guard = state.process.lock().unwrap_or_else(|e| e.into_inner());
                        *guard = Some(child);
                    }
                    state.restart_count.fetch_add(1, Ordering::Relaxed);
                    tracing::info!("Watchdog: daemon restarted (PID {})", pid);

                    // Wait and verify it didn't crash immediately
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    if state.is_running() {
                        // Wait for health
                        for _ in 0..20 {
                            if check_health(port).await {
                                tracing::info!("Watchdog: daemon healthy after restart");
                                // Successful restart — reset counter
                                state.restart_count.store(0, Ordering::Relaxed);
                                break;
                            }
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        }
                    }
                }
                Err(e) => {
                    state.restart_count.fetch_add(1, Ordering::Relaxed);
                    tracing::error!("Watchdog: restart failed — {}", e);
                }
            }
        }
    });
}

// ─── Tauri Commands ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn start_daemon(
    app: tauri::AppHandle,
    state: State<'_, DaemonState>,
) -> Result<String, String> {
    // Already running?
    if state.is_running() {
        // Ensure watchdog is enabled
        state.watchdog_enabled.store(true, Ordering::Relaxed);
        return Ok("Daemon is already running".to_string());
    }

    let binary = state.find_binary();
    let port = state.port;

    // Use Tauri app config dir so daemon reads the onboarding-written config
    let config_path = app
        .path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("agents.toml"));

    tracing::info!(
        "Starting phantom-mesh daemon: {:?} --host 0.0.0.0 --port {} --config {:?} daemon",
        binary,
        port,
        config_path,
    );

    let child = spawn_daemon(&binary, port, config_path.as_ref())?;

    let pid = child.id();
    {
        let mut guard = state.process.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(child);
    }

    // Enable watchdog for auto-restart and reset counter
    state.watchdog_enabled.store(true, Ordering::Relaxed);
    state.restart_count.store(0, Ordering::Relaxed);

    // Wait for the process to start (daemon takes 15-18s due to LLM provider reachability checks)
    tokio::time::sleep(std::time::Duration::from_millis(3000)).await;

    // Verify the process didn't immediately crash
    if !state.is_running() {
        return Err("Daemon process exited immediately after spawn".to_string());
    }

    // Try health check with retries (up to ~30s total: 3s initial + 27×1s)
    let mut healthy = false;
    for _ in 0..27 {
        if check_health(port).await {
            healthy = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    }

    if healthy {
        Ok(format!("Daemon started (PID {}, port {})", pid, port))
    } else {
        // Process is running but not yet healthy — that's OK for large startup
        Ok(format!(
            "Daemon started (PID {}, port {}) — health check pending",
            pid, port
        ))
    }
}

#[tauri::command]
pub async fn stop_daemon(state: State<'_, DaemonState>) -> Result<String, String> {
    // Disable watchdog so it doesn't auto-restart after explicit stop
    state.watchdog_enabled.store(false, Ordering::Relaxed);
    if state.kill() {
        Ok("Daemon stopped".to_string())
    } else {
        Ok("No daemon was running".to_string())
    }
}

#[tauri::command]
pub async fn daemon_status(state: State<'_, DaemonState>) -> Result<DaemonInfo, String> {
    let running = state.is_running();
    let pid = state.pid();
    let port = state.port;
    let binary_path = {
        let guard = state.binary_path.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().map(|p| p.display().to_string())
    };

    let healthy = if running {
        check_health(port).await
    } else {
        false
    };

    Ok(DaemonInfo {
        running,
        pid,
        port,
        binary_path,
        healthy,
        restart_count: state.restart_count.load(Ordering::Relaxed),
        watchdog_enabled: state.watchdog_enabled.load(Ordering::Relaxed),
    })
}
