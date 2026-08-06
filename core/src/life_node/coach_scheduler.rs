//! SPEC-23 §G1 coach scheduler — desktop unit generators.
//!
//! The coach loop's missing "spine": at the user's local 21:00 (configurable)
//! an OS scheduler must fire `spectyn coach review` so the daily reflection
//! runs without the user typing a command. This module **only GENERATES** the
//! per-OS unit text + the canonical install paths — it deliberately does NOT
//! install / load them, because loading a `launchd`（macOS 背景任務 daemon）
//! agent, enabling a `systemd`（Linux 系統管理器）user timer, or registering a
//! `schtasks`（Windows 工作排程）task all mutate the host. That install step is
//! left to an explicit installer / operator action; keeping generation pure
//! makes it fully unit-testable and side-effect-free.
//!
//! Desktop targets only (launchd / systemd / schtasks). Mobile schedulers
//! (iOS `BGTaskScheduler` ≤ 30 s budget / Android `WorkManager` ≥ 15 min period)
//! are a separate platform concern (SPEC-23 §15) and are not generated here.
//!
//! The triggered command is `<spectyn-exe> coach review` (the AI daily-review
//! summary + tomorrow-action; see `spectyn coach review` in the CLI). The
//! scheduler intentionally does NOT wake the machine — it only fires while the
//! host is already awake (SPEC-23 §D `wake`).

use std::path::PathBuf;

/// launchd `Label` / schtasks task name / systemd unit stem for the coach job.
/// Mirrors the existing `ai.spectynmesh.serve` service convention.
pub const COACH_LABEL: &str = "ai.spectynmesh.coach";

/// Default trigger: local 21:00 (SPEC-23 §0 — the locked default review time).
pub const DEFAULT_HOUR: u8 = 21;
pub const DEFAULT_MINUTE: u8 = 0;

/// A validated local time-of-day for the daily coach trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoachSchedule {
    pub hour: u8,
    pub minute: u8,
}

impl Default for CoachSchedule {
    fn default() -> Self {
        Self { hour: DEFAULT_HOUR, minute: DEFAULT_MINUTE }
    }
}

impl CoachSchedule {
    /// Construct from an hour (0–23) + minute (0–59), rejecting out-of-range
    /// values so a malformed setting can never produce a broken unit file.
    pub fn new(hour: u8, minute: u8) -> Result<Self, String> {
        if hour > 23 {
            return Err(format!("coach schedule hour out of range (0–23): {hour}"));
        }
        if minute > 59 {
            return Err(format!("coach schedule minute out of range (0–59): {minute}"));
        }
        Ok(Self { hour, minute })
    }
}

/// macOS `launchd` per-user LaunchAgent plist firing the coach review daily at
/// `schedule` via `StartCalendarInterval`. `RunAtLoad` is false (the trigger is
/// purely time-based; we never run it just because the agent (re)loaded).
///
/// The fired command is `coach review --save` so each daily run PERSISTS its
/// review (the events brief + tomorrow-action + the proactive partner "Daily
/// alignment" reflection) to `~/.spectyn-mesh/reviews/{date}.md` — that saved
/// file is the daily artifact the partner produces, and its appearance is how a
/// fire is verified. stdout/stderr are redirected to
/// `~/.spectyn-mesh/coach.{out,err}.log` (mirroring the serve agent) so a failed
/// run is diagnosable instead of vanishing into launchd's void.
///
/// `exe_path` is the absolute path to the `spectyn` binary (the caller resolves
/// it, e.g. via `std::env::current_exe`). XML-escaped so an unusual install
/// path can't break the plist.
pub fn launchd_plist(exe_path: &str, schedule: CoachSchedule) -> String {
    let exe = xml_escape(exe_path);
    let (out_log, err_log) = coach_log_paths();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \t<key>Label</key>\n\t<string>{label}</string>\n\
         \t<key>ProgramArguments</key>\n\t<array>\n\
         \t\t<string>{exe}</string>\n\
         \t\t<string>coach</string>\n\
         \t\t<string>review</string>\n\
         \t\t<string>--save</string>\n\
         \t</array>\n\
         \t<key>StartCalendarInterval</key>\n\t<dict>\n\
         \t\t<key>Hour</key>\n\t\t<integer>{hour}</integer>\n\
         \t\t<key>Minute</key>\n\t\t<integer>{minute}</integer>\n\
         \t</dict>\n\
         \t<key>RunAtLoad</key>\n\t<false/>\n\
         \t<key>StandardOutPath</key>\n\t<string>{out_log}</string>\n\
         \t<key>StandardErrorPath</key>\n\t<string>{err_log}</string>\n\
         </dict>\n</plist>\n",
        label = COACH_LABEL,
        exe = exe,
        hour = schedule.hour,
        minute = schedule.minute,
        out_log = xml_escape(&out_log),
        err_log = xml_escape(&err_log),
    )
}

/// `(stdout, stderr)` log paths the launchd coach agent redirects to, under
/// `~/.spectyn-mesh/` next to the serve agent's logs. Falls back to bare
/// filenames (cwd-relative) if there's no home dir — launchd always has one in
/// practice, but the generator must stay total.
fn coach_log_paths() -> (String, String) {
    match crate::cli_config::spectyn_data_dir() {
        Ok(base) => (
            base.join("coach.out.log").to_string_lossy().into_owned(),
            base.join("coach.err.log").to_string_lossy().into_owned(),
        ),
        Err(_) => ("coach.out.log".to_string(), "coach.err.log".to_string()),
    }
}

/// Linux `systemd` **user** service unit (oneshot) that runs the coach review.
/// Paired with [`systemd_timer_unit`] — the timer fires the service.
///
/// The exe path is quoted + escaped via [`systemd_exec_escape`]: systemd
/// tokenises `ExecStart=` on whitespace and expands `%` specifiers, so a path
/// with spaces / `%` / quotes would otherwise mis-split or be reinterpreted
/// (the launchd + schtasks generators already isolate the path; this keeps the
/// systemd path equally hostile-path-safe).
pub fn systemd_service_unit(exe_path: &str) -> String {
    format!(
        "[Unit]\n\
         Description=spectyn-mesh coach daily review (SPEC-23)\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         ExecStart={exe} coach review --save\n",
        exe = systemd_exec_escape(exe_path),
    )
}

/// Escape an executable path for a systemd `ExecStart=` value: first STRIP any
/// control characters — a unit file is line-based, so a literal newline in the
/// path could otherwise inject a second `ExecStart=`/directive line (POSIX
/// paths may technically contain `\n`; systemd also forbids control chars in the
/// command). Then wrap in double quotes (so whitespace doesn't split it into
/// separate argv entries), escape `\` and `"` inside the quotes, and double `%`
/// so it is a literal percent and not a systemd specifier (`%h`, `%n`, …).
fn systemd_exec_escape(path: &str) -> String {
    let cleaned: String = path.chars().filter(|c| !c.is_control()).collect();
    let inner = cleaned
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    format!("\"{inner}\"")
}

/// Linux `systemd` **user** timer firing the coach service daily at `schedule`
/// in the host's local timezone. `Persistent=true` catches up one missed run
/// after the machine was asleep/off at the trigger time (SPEC-23 G1: desktop
/// punctual ±60 s, but a powered-off box should still get yesterday's review).
pub fn systemd_timer_unit(schedule: CoachSchedule) -> String {
    format!(
        "[Unit]\n\
         Description=spectyn-mesh coach daily review timer (SPEC-23)\n\
         \n\
         [Timer]\n\
         OnCalendar=*-*-* {hour:02}:{minute:02}:00\n\
         Persistent=true\n\
         \n\
         [Install]\n\
         WantedBy=timers.target\n",
        hour = schedule.hour,
        minute = schedule.minute,
    )
}

/// Windows `schtasks` argument vector to register the daily coach review task.
/// Returned as args (not a shell string) so the caller spawns `schtasks`
/// without a shell — no quoting/injection surface. `/F` overwrites an existing
/// task so re-running is idempotent.
pub fn windows_schtasks_create_args(exe_path: &str, schedule: CoachSchedule) -> Vec<String> {
    vec![
        "/Create".to_string(),
        "/TN".to_string(),
        COACH_LABEL.to_string(),
        "/TR".to_string(),
        format!("\"{exe_path}\" coach review --save"),
        "/SC".to_string(),
        "DAILY".to_string(),
        "/ST".to_string(),
        format!("{:02}:{:02}", schedule.hour, schedule.minute),
        "/F".to_string(),
    ]
}

/// Canonical per-user LaunchAgent plist path:
/// `~/Library/LaunchAgents/ai.spectynmesh.coach.plist`. `None` if no home dir.
pub fn launchd_plist_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join("Library")
            .join("LaunchAgents")
            .join(format!("{COACH_LABEL}.plist"))
    })
}

/// Canonical systemd **user** unit directory: `~/.config/systemd/user`. The
/// service + timer land here as `spectyn-coach.service` / `spectyn-coach.timer`.
/// `None` if no home dir.
pub fn systemd_user_unit_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".config").join("systemd").join("user"))
}

/// Minimal XML text escaping for the plist string fields (path may contain
/// `&`/`<`/`>`/quotes on an unusual install location).
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Which desktop scheduler backend to render. Kept as an explicit enum (rather
/// than only `cfg!`) so the CLI's `render_cli_unit` output is unit-testable for
/// every platform regardless of where the test suite runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerTarget {
    /// macOS `launchd` LaunchAgent.
    Launchd,
    /// Linux `systemd` user service + timer.
    Systemd,
    /// Windows Task Scheduler (`schtasks`).
    Schtasks,
}

/// Render the operator-facing "here is your scheduler unit + how to install it"
/// document for `spectyn coach schedule`. PURE: returns a string, installs
/// nothing — the CLI writes it to stdout and the user installs manually (we
/// never mutate the host automatically). The leading comment lines tell the
/// user the exact install command for their platform.
pub fn render_cli_unit(target: SchedulerTarget, exe_path: &str, schedule: CoachSchedule) -> String {
    match target {
        SchedulerTarget::Launchd => {
            let path = launchd_plist_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "~/Library/LaunchAgents/ai.spectynmesh.coach.plist".to_string());
            format!(
                "# macOS launchd LaunchAgent — write the plist below to:\n\
                 #   {path}\n\
                 # then load it (spectyn does NOT auto-install):\n\
                 #   launchctl load -w \"{path}\"\n\
                 {plist}",
                plist = launchd_plist(exe_path, schedule),
            )
        }
        SchedulerTarget::Systemd => {
            let dir = systemd_user_unit_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "~/.config/systemd/user".to_string());
            format!(
                "# Linux systemd user units — write BOTH files under:\n\
                 #   {dir}/spectyn-coach.service  and  {dir}/spectyn-coach.timer\n\
                 # then enable (spectyn does NOT auto-install):\n\
                 #   systemctl --user enable --now spectyn-coach.timer\n\
                 # ===== spectyn-coach.service =====\n{svc}\
                 # ===== spectyn-coach.timer =====\n{timer}",
                svc = systemd_service_unit(exe_path),
                timer = systemd_timer_unit(schedule),
            )
        }
        SchedulerTarget::Schtasks => {
            let args = windows_schtasks_create_args(exe_path, schedule);
            format!(
                "# Windows Task Scheduler — review then run (spectyn does NOT auto-install):\n\
                 schtasks {}\n",
                args.join(" "),
            )
        }
    }
}

// ── installer (the explicit side-effecting layer) ───────────────────────────
//
// Everything above is PURE (generates unit text + paths). `spectyn coach
// install-schedule` is the deliberate, operator-invoked step that finally
// MUTATES the host: it writes the canonical unit file(s) and registers them
// with the OS scheduler (launchctl / systemctl --user / schtasks) so the daily
// `spectyn coach review` fires without the user typing anything. Kept here, next
// to the generators, so the install paths + commands stay in lock-step with the
// unit text they consume.

/// Outcome of an install: which file(s) were written + the loader command run,
/// so the CLI can print a precise "here's what I did" report.
#[derive(Debug, Clone)]
pub struct InstallOutcome {
    /// Absolute paths of the unit file(s) written.
    pub files_written: Vec<PathBuf>,
    /// Human-readable loader command that was executed (for the report).
    pub loaded_with: String,
}

/// macOS only: ensure `exe_path` carries a code signature launchd will accept.
///
/// A freshly `cargo build`'d Mach-O on Apple Silicon is **linker-signed** ad-hoc
/// (`codesign` flags `0x20002` = `adhoc,linker-signed`). When such a binary is
/// spawned by `launchd` (parent pid 1), macOS's code-signing monitor SIGKILLs it
/// with a *"Launch Constraint Violation"* (`EXC_CRASH` / `SIGKILL (Code Signature
/// Invalid)`) within ~1 s — before any work runs. The job then looks "installed"
/// (the plist loads fine) but every daily fire dies silently: empty logs, no
/// review written, `runs` resetting to 0. Running the same binary from an
/// interactive shell works, which is exactly why this slips through manual smoke
/// tests. A plain ad-hoc re-sign (`codesign --force --sign -`, flags `0x2`)
/// clears the linker-signed bit and launchd then runs the job to completion.
///
/// So before we register the LaunchAgent we re-sign the target binary ad-hoc iff
/// it is linker-signed. We only re-sign linker-signed binaries: a properly
/// installed / Developer-ID-signed `spectyn` is left untouched (re-signing would
/// strip a real signature). Best-effort + non-fatal: if `codesign` is missing or
/// fails we still install (the user may have pointed at an already-valid binary),
/// but we surface a warning to stderr so a dead trigger is diagnosable.
#[cfg(target_os = "macos")]
fn ensure_launchd_runnable_signature(exe_path: &str) {
    use std::process::Command;
    // Inspect the current signature. `codesign -dv` prints to stderr.
    let info = match Command::new("codesign").args(["-dv", exe_path]).output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!(
                "warning: could not run codesign to check {exe_path}: {e} — \
                 the launchd job may be SIGKILLed if the binary is linker-signed"
            );
            return;
        }
    };
    let desc = String::from_utf8_lossy(&info.stderr);
    // Only re-sign the launchd-hostile linker-signed ad-hoc case. A real
    // signature (Developer ID / proper ad-hoc `0x2`) is left as-is.
    if !desc.contains("linker-signed") {
        return;
    }
    let resign = Command::new("codesign")
        .args(["--force", "--sign", "-", exe_path])
        .output();
    match resign {
        Ok(o) if o.status.success() => {}
        Ok(o) => eprintln!(
            "warning: ad-hoc re-sign of {exe_path} failed: {} — \
             launchd may SIGKILL the daily coach job (linker-signed binary)",
            String::from_utf8_lossy(&o.stderr).trim()
        ),
        Err(e) => eprintln!(
            "warning: could not spawn codesign to re-sign {exe_path}: {e} — \
             launchd may SIGKILL the daily coach job (linker-signed binary)"
        ),
    }
}

/// Install + load the macOS launchd LaunchAgent for the daily coach review.
///
/// Writes the canonical plist to `~/Library/LaunchAgents/ai.spectynmesh.coach.plist`
/// then `launchctl load -w` it (idempotent: an already-loaded agent is unloaded
/// first so re-running with a new `--at` actually re-registers the new time).
///
/// Before loading we [`ensure_launchd_runnable_signature`] the target binary: a
/// freshly built (linker-signed) `spectyn` is SIGKILLed by launchd's code-signing
/// monitor, so without this the trigger installs but never actually fires.
#[cfg(target_os = "macos")]
pub fn install_launchd(exe_path: &str, schedule: CoachSchedule) -> Result<InstallOutcome, String> {
    use std::process::Command;
    let plist_path =
        launchd_plist_path().ok_or_else(|| "no home dir for LaunchAgents".to_string())?;
    if let Some(parent) = plist_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    // Make the binary launchd-runnable BEFORE we load the agent, else the daily
    // fire dies with a code-signing SIGKILL and writes nothing.
    ensure_launchd_runnable_signature(exe_path);
    let plist = launchd_plist(exe_path, schedule);
    std::fs::write(&plist_path, plist)
        .map_err(|e| format!("write {}: {e}", plist_path.display()))?;

    // Best-effort unload first so a re-install with a new time replaces the old
    // registration (launchctl load on an already-loaded label is a no-op). We
    // ignore the unload's status — "not loaded" is the expected first-run case.
    let _ = Command::new("launchctl")
        .arg("unload")
        .arg(&plist_path)
        .output();
    let out = Command::new("launchctl")
        .arg("load")
        .arg("-w")
        .arg(&plist_path)
        .output()
        .map_err(|e| format!("spawn launchctl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "launchctl load -w {} failed: {}",
            plist_path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(InstallOutcome {
        files_written: vec![plist_path.clone()],
        loaded_with: format!("launchctl load -w {}", plist_path.display()),
    })
}

/// Install + enable the Linux systemd **user** service + timer for the daily
/// coach review. Writes both units under `~/.config/systemd/user/`, reloads the
/// user manager, then `systemctl --user enable --now spectyn-coach.timer`.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn install_systemd(exe_path: &str, schedule: CoachSchedule) -> Result<InstallOutcome, String> {
    use std::process::Command;
    let dir = systemd_user_unit_dir().ok_or_else(|| "no home dir for systemd units".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let svc_path = dir.join("spectyn-coach.service");
    let timer_path = dir.join("spectyn-coach.timer");
    std::fs::write(&svc_path, systemd_service_unit(exe_path))
        .map_err(|e| format!("write {}: {e}", svc_path.display()))?;
    std::fs::write(&timer_path, systemd_timer_unit(schedule))
        .map_err(|e| format!("write {}: {e}", timer_path.display()))?;

    // Reload so the manager sees the new/updated units, then enable --now so the
    // timer is registered AND started for the current session.
    let reload = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output()
        .map_err(|e| format!("spawn systemctl: {e}"))?;
    if !reload.status.success() {
        return Err(format!(
            "systemctl --user daemon-reload failed: {}",
            String::from_utf8_lossy(&reload.stderr).trim()
        ));
    }
    let enable = Command::new("systemctl")
        .args(["--user", "enable", "--now", "spectyn-coach.timer"])
        .output()
        .map_err(|e| format!("spawn systemctl: {e}"))?;
    if !enable.status.success() {
        return Err(format!(
            "systemctl --user enable --now spectyn-coach.timer failed: {}",
            String::from_utf8_lossy(&enable.stderr).trim()
        ));
    }
    Ok(InstallOutcome {
        files_written: vec![svc_path, timer_path],
        loaded_with: "systemctl --user enable --now spectyn-coach.timer".to_string(),
    })
}

/// Register the Windows Task Scheduler daily coach review task via `schtasks`.
/// `/F` makes it idempotent (overwrites an existing task), so re-running with a
/// new `--at` replaces the old trigger.
#[cfg(target_os = "windows")]
pub fn install_schtasks(exe_path: &str, schedule: CoachSchedule) -> Result<InstallOutcome, String> {
    use std::process::Command;
    let args = windows_schtasks_create_args(exe_path, schedule);
    let out = Command::new("schtasks")
        .args(&args)
        .output()
        .map_err(|e| format!("spawn schtasks: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "schtasks {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(InstallOutcome {
        files_written: Vec::new(), // schtasks stores the task in its own registry.
        loaded_with: format!("schtasks {}", args.join(" ")),
    })
}

/// Install the daily coach trigger for the host OS. Dispatches to the
/// per-platform installer above. `exe_path` is the absolute path to the
/// `spectyn` binary the scheduler should run.
pub fn install_for_host(exe_path: &str, schedule: CoachSchedule) -> Result<InstallOutcome, String> {
    #[cfg(target_os = "macos")]
    {
        install_launchd(exe_path, schedule)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        install_systemd(exe_path, schedule)
    }
    #[cfg(target_os = "windows")]
    {
        install_schtasks(exe_path, schedule)
    }
    #[cfg(not(any(target_os = "macos", unix, target_os = "windows")))]
    {
        let _ = (exe_path, schedule);
        Err("no supported desktop scheduler for this OS".to_string())
    }
}

/// Uninstall + unload the daily coach trigger for the host OS (the inverse of
/// [`install_for_host`]). Best-effort + idempotent: removing an already-gone
/// unit / task is treated as success so a re-run never errors.
pub fn uninstall_for_host() -> Result<Vec<PathBuf>, String> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        let mut removed = Vec::new();
        if let Some(plist_path) = launchd_plist_path() {
            if plist_path.exists() {
                let _ = Command::new("launchctl")
                    .arg("unload")
                    .arg(&plist_path)
                    .output();
                std::fs::remove_file(&plist_path)
                    .map_err(|e| format!("remove {}: {e}", plist_path.display()))?;
                removed.push(plist_path);
            }
        }
        Ok(removed)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::process::Command;
        let _ = Command::new("systemctl")
            .args(["--user", "disable", "--now", "spectyn-coach.timer"])
            .output();
        let mut removed = Vec::new();
        if let Some(dir) = systemd_user_unit_dir() {
            for name in ["spectyn-coach.timer", "spectyn-coach.service"] {
                let p = dir.join(name);
                if p.exists() {
                    std::fs::remove_file(&p)
                        .map_err(|e| format!("remove {}: {e}", p.display()))?;
                    removed.push(p);
                }
            }
        }
        let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).output();
        Ok(removed)
    }
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        let _ = Command::new("schtasks")
            .args(["/Delete", "/TN", COACH_LABEL, "/F"])
            .output();
        Ok(Vec::new())
    }
    #[cfg(not(any(target_os = "macos", unix, target_os = "windows")))]
    {
        Err("no supported desktop scheduler for this OS".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_schedule_is_local_21_00() {
        let s = CoachSchedule::default();
        assert_eq!((s.hour, s.minute), (21, 0));
        assert_eq!((DEFAULT_HOUR, DEFAULT_MINUTE), (21, 0));
    }

    #[test]
    fn schedule_new_rejects_out_of_range() {
        assert!(CoachSchedule::new(24, 0).is_err(), "hour 24 invalid");
        assert!(CoachSchedule::new(21, 60).is_err(), "minute 60 invalid");
        assert!(CoachSchedule::new(0, 0).is_ok());
        assert!(CoachSchedule::new(23, 59).is_ok());
    }

    #[test]
    fn launchd_plist_has_label_command_and_calendar_interval() {
        let p = launchd_plist("/usr/local/bin/spectyn", CoachSchedule::new(21, 0).unwrap());
        assert!(p.starts_with("<?xml"), "valid plist header");
        assert!(p.contains("<string>ai.spectynmesh.coach</string>"), "coach label");
        // Triggers `spectyn coach review --save` (each fire persists the review).
        assert!(p.contains("<string>/usr/local/bin/spectyn</string>"));
        assert!(p.contains("<string>coach</string>") && p.contains("<string>review</string>"));
        assert!(p.contains("<string>--save</string>"), "fire persists the daily review");
        // Time-based, not run-at-load.
        assert!(p.contains("<key>StartCalendarInterval</key>"));
        assert!(p.contains("<key>Hour</key>\n\t\t<integer>21</integer>"));
        assert!(p.contains("<key>Minute</key>\n\t\t<integer>0</integer>"));
        assert!(p.contains("<key>RunAtLoad</key>\n\t<false/>"), "never run just on load");
        // Output is captured so a failed daily run is diagnosable (not lost).
        assert!(p.contains("<key>StandardOutPath</key>"), "stdout captured: {p}");
        assert!(p.contains("<key>StandardErrorPath</key>"), "stderr captured: {p}");
        assert!(p.contains("coach.out.log") && p.contains("coach.err.log"), "log paths: {p}");
    }

    #[test]
    fn launchd_plist_xml_escapes_path() {
        let p = launchd_plist("/opt/a&b<c>d\"e'f/spectyn", CoachSchedule::default());
        assert!(p.contains("/opt/a&amp;b&lt;c&gt;d&quot;e&apos;f/spectyn"), "all 5 escaped: {p}");
        assert!(!p.contains("a&b<c>"), "raw markup must not survive");
    }

    #[test]
    fn systemd_units_fire_coach_review_daily() {
        let svc = systemd_service_unit("/usr/bin/spectyn");
        assert!(svc.contains("Type=oneshot"));
        assert!(svc.contains("ExecStart=\"/usr/bin/spectyn\" coach review"), "quoted exe: {svc}");

        let timer = systemd_timer_unit(CoachSchedule::new(21, 0).unwrap());
        assert!(timer.contains("OnCalendar=*-*-* 21:00:00"), "daily 21:00 local: {timer}");
        assert!(timer.contains("Persistent=true"), "catch up a missed run");
        assert!(timer.contains("WantedBy=timers.target"));
    }

    #[test]
    fn systemd_execstart_is_hostile_path_safe() {
        // Spaces must not split into extra argv; `%` must not become a systemd
        // specifier; quotes must be escaped inside the quoted value.
        let svc = systemd_service_unit("/opt/my apps/100% spectyn\"x/spectyn");
        assert!(
            svc.contains("ExecStart=\"/opt/my apps/100%% spectyn\\\"x/spectyn\" coach review"),
            "spaces quoted, %->%%, \" escaped: {svc}"
        );
        // The raw single-% (specifier risk) must not survive.
        assert!(!svc.contains("100% spectyn"), "literal % must be doubled");
    }

    #[test]
    fn systemd_execstart_strips_control_chars_no_line_injection() {
        // A newline/tab in the path must NOT inject a second directive line into
        // the line-based unit file (control chars are stripped before quoting).
        let svc = systemd_service_unit("/opt/p\nExecStart=/evil\tx");
        let exec_lines = svc.lines().filter(|l| l.starts_with("ExecStart=")).count();
        assert_eq!(exec_lines, 1, "exactly one ExecStart directive (no injection): {svc}");
        assert!(
            svc.contains("ExecStart=\"/opt/pExecStart=/evilx\" coach review"),
            "control chars removed inline, rest quoted: {svc}"
        );
    }

    #[test]
    fn systemd_timer_zero_pads_single_digit_time() {
        let timer = systemd_timer_unit(CoachSchedule::new(9, 5).unwrap());
        assert!(timer.contains("OnCalendar=*-*-* 09:05:00"), "zero-padded: {timer}");
    }

    #[test]
    fn windows_schtasks_args_are_shell_free_and_daily() {
        let args = windows_schtasks_create_args("C:\\spectyn.exe", CoachSchedule::new(21, 0).unwrap());
        assert!(args.contains(&"/Create".to_string()));
        assert!(args.contains(&"/SC".to_string()) && args.contains(&"DAILY".to_string()));
        assert!(args.contains(&"/F".to_string()), "idempotent overwrite");
        assert!(args.iter().any(|a| a == "21:00"), "start time 21:00");
        assert!(args.iter().any(|a| a.contains("coach review")), "runs coach review");
        assert!(args.contains(&"ai.spectynmesh.coach".to_string()), "task name");
    }

    #[test]
    fn install_paths_use_canonical_locations() {
        if let Some(p) = launchd_plist_path() {
            assert!(p.ends_with("Library/LaunchAgents/ai.spectynmesh.coach.plist"), "{p:?}");
        }
        if let Some(d) = systemd_user_unit_dir() {
            assert!(d.ends_with(".config/systemd/user"), "{d:?}");
        }
    }

    #[test]
    fn render_cli_unit_embeds_unit_and_install_hint_per_target() {
        let sched = CoachSchedule::new(21, 0).unwrap();

        let mac = render_cli_unit(SchedulerTarget::Launchd, "/usr/local/bin/spectyn", sched);
        assert!(mac.contains("launchctl load -w"), "macOS install hint: {mac}");
        assert!(mac.contains("<key>StartCalendarInterval</key>"), "embeds the plist");
        assert!(mac.contains("spectyn does NOT auto-install"), "no-auto-install disclaimer");

        let linux = render_cli_unit(SchedulerTarget::Systemd, "/usr/bin/spectyn", sched);
        assert!(linux.contains("systemctl --user enable --now spectyn-coach.timer"), "linux hint");
        assert!(linux.contains("spectyn-coach.service") && linux.contains("spectyn-coach.timer"));
        assert!(linux.contains("OnCalendar=*-*-* 21:00:00"), "embeds the timer");

        let win = render_cli_unit(SchedulerTarget::Schtasks, "C:\\spectyn.exe", sched);
        assert!(win.contains("schtasks /Create"), "windows command: {win}");
        assert!(win.contains("coach review") && win.contains("/SC DAILY"));
        assert!(win.contains("spectyn does NOT auto-install"));
    }
}
