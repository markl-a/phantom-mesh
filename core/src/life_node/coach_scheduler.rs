//! SPEC-23 §G1 coach scheduler — desktop unit generators.
//!
//! The coach loop's missing "spine": at the user's local 21:00 (configurable)
//! an OS scheduler must fire `phantom coach review` so the daily reflection
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
//! The triggered command is `<phantom-exe> coach review` (the AI daily-review
//! summary + tomorrow-action; see `phantom coach review` in the CLI). The
//! scheduler intentionally does NOT wake the machine — it only fires while the
//! host is already awake (SPEC-23 §D `wake`).

use std::path::PathBuf;

/// launchd `Label` / schtasks task name / systemd unit stem for the coach job.
/// Mirrors the existing `ai.phantommesh.serve` service convention.
pub const COACH_LABEL: &str = "ai.phantommesh.coach";

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
/// `exe_path` is the absolute path to the `phantom` binary (the caller resolves
/// it, e.g. via `std::env::current_exe`). XML-escaped so an unusual install
/// path can't break the plist.
pub fn launchd_plist(exe_path: &str, schedule: CoachSchedule) -> String {
    let exe = xml_escape(exe_path);
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
         \t</array>\n\
         \t<key>StartCalendarInterval</key>\n\t<dict>\n\
         \t\t<key>Hour</key>\n\t\t<integer>{hour}</integer>\n\
         \t\t<key>Minute</key>\n\t\t<integer>{minute}</integer>\n\
         \t</dict>\n\
         \t<key>RunAtLoad</key>\n\t<false/>\n\
         </dict>\n</plist>\n",
        label = COACH_LABEL,
        exe = exe,
        hour = schedule.hour,
        minute = schedule.minute,
    )
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
         Description=phantom-mesh coach daily review (SPEC-23)\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         ExecStart={exe} coach review\n",
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
         Description=phantom-mesh coach daily review timer (SPEC-23)\n\
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
        format!("\"{exe_path}\" coach review"),
        "/SC".to_string(),
        "DAILY".to_string(),
        "/ST".to_string(),
        format!("{:02}:{:02}", schedule.hour, schedule.minute),
        "/F".to_string(),
    ]
}

/// Canonical per-user LaunchAgent plist path:
/// `~/Library/LaunchAgents/ai.phantommesh.coach.plist`. `None` if no home dir.
pub fn launchd_plist_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join("Library")
            .join("LaunchAgents")
            .join(format!("{COACH_LABEL}.plist"))
    })
}

/// Canonical systemd **user** unit directory: `~/.config/systemd/user`. The
/// service + timer land here as `phantom-coach.service` / `phantom-coach.timer`.
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
/// document for `phantom coach schedule`. PURE: returns a string, installs
/// nothing — the CLI writes it to stdout and the user installs manually (we
/// never mutate the host automatically). The leading comment lines tell the
/// user the exact install command for their platform.
pub fn render_cli_unit(target: SchedulerTarget, exe_path: &str, schedule: CoachSchedule) -> String {
    match target {
        SchedulerTarget::Launchd => {
            let path = launchd_plist_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "~/Library/LaunchAgents/ai.phantommesh.coach.plist".to_string());
            format!(
                "# macOS launchd LaunchAgent — write the plist below to:\n\
                 #   {path}\n\
                 # then load it (phantom does NOT auto-install):\n\
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
                 #   {dir}/phantom-coach.service  and  {dir}/phantom-coach.timer\n\
                 # then enable (phantom does NOT auto-install):\n\
                 #   systemctl --user enable --now phantom-coach.timer\n\
                 # ===== phantom-coach.service =====\n{svc}\
                 # ===== phantom-coach.timer =====\n{timer}",
                svc = systemd_service_unit(exe_path),
                timer = systemd_timer_unit(schedule),
            )
        }
        SchedulerTarget::Schtasks => {
            let args = windows_schtasks_create_args(exe_path, schedule);
            format!(
                "# Windows Task Scheduler — review then run (phantom does NOT auto-install):\n\
                 schtasks {}\n",
                args.join(" "),
            )
        }
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
        let p = launchd_plist("/usr/local/bin/phantom", CoachSchedule::new(21, 0).unwrap());
        assert!(p.starts_with("<?xml"), "valid plist header");
        assert!(p.contains("<string>ai.phantommesh.coach</string>"), "coach label");
        // Triggers `phantom coach review`.
        assert!(p.contains("<string>/usr/local/bin/phantom</string>"));
        assert!(p.contains("<string>coach</string>") && p.contains("<string>review</string>"));
        // Time-based, not run-at-load.
        assert!(p.contains("<key>StartCalendarInterval</key>"));
        assert!(p.contains("<key>Hour</key>\n\t\t<integer>21</integer>"));
        assert!(p.contains("<key>Minute</key>\n\t\t<integer>0</integer>"));
        assert!(p.contains("<key>RunAtLoad</key>\n\t<false/>"), "never run just on load");
    }

    #[test]
    fn launchd_plist_xml_escapes_path() {
        let p = launchd_plist("/opt/a&b<c>d\"e'f/phantom", CoachSchedule::default());
        assert!(p.contains("/opt/a&amp;b&lt;c&gt;d&quot;e&apos;f/phantom"), "all 5 escaped: {p}");
        assert!(!p.contains("a&b<c>"), "raw markup must not survive");
    }

    #[test]
    fn systemd_units_fire_coach_review_daily() {
        let svc = systemd_service_unit("/usr/bin/phantom");
        assert!(svc.contains("Type=oneshot"));
        assert!(svc.contains("ExecStart=\"/usr/bin/phantom\" coach review"), "quoted exe: {svc}");

        let timer = systemd_timer_unit(CoachSchedule::new(21, 0).unwrap());
        assert!(timer.contains("OnCalendar=*-*-* 21:00:00"), "daily 21:00 local: {timer}");
        assert!(timer.contains("Persistent=true"), "catch up a missed run");
        assert!(timer.contains("WantedBy=timers.target"));
    }

    #[test]
    fn systemd_execstart_is_hostile_path_safe() {
        // Spaces must not split into extra argv; `%` must not become a systemd
        // specifier; quotes must be escaped inside the quoted value.
        let svc = systemd_service_unit("/opt/my apps/100% phantom\"x/phantom");
        assert!(
            svc.contains("ExecStart=\"/opt/my apps/100%% phantom\\\"x/phantom\" coach review"),
            "spaces quoted, %->%%, \" escaped: {svc}"
        );
        // The raw single-% (specifier risk) must not survive.
        assert!(!svc.contains("100% phantom"), "literal % must be doubled");
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
        let args = windows_schtasks_create_args("C:\\phantom.exe", CoachSchedule::new(21, 0).unwrap());
        assert!(args.contains(&"/Create".to_string()));
        assert!(args.contains(&"/SC".to_string()) && args.contains(&"DAILY".to_string()));
        assert!(args.contains(&"/F".to_string()), "idempotent overwrite");
        assert!(args.iter().any(|a| a == "21:00"), "start time 21:00");
        assert!(args.iter().any(|a| a.contains("coach review")), "runs coach review");
        assert!(args.contains(&"ai.phantommesh.coach".to_string()), "task name");
    }

    #[test]
    fn install_paths_use_canonical_locations() {
        if let Some(p) = launchd_plist_path() {
            assert!(p.ends_with("Library/LaunchAgents/ai.phantommesh.coach.plist"), "{p:?}");
        }
        if let Some(d) = systemd_user_unit_dir() {
            assert!(d.ends_with(".config/systemd/user"), "{d:?}");
        }
    }

    #[test]
    fn render_cli_unit_embeds_unit_and_install_hint_per_target() {
        let sched = CoachSchedule::new(21, 0).unwrap();

        let mac = render_cli_unit(SchedulerTarget::Launchd, "/usr/local/bin/phantom", sched);
        assert!(mac.contains("launchctl load -w"), "macOS install hint: {mac}");
        assert!(mac.contains("<key>StartCalendarInterval</key>"), "embeds the plist");
        assert!(mac.contains("phantom does NOT auto-install"), "no-auto-install disclaimer");

        let linux = render_cli_unit(SchedulerTarget::Systemd, "/usr/bin/phantom", sched);
        assert!(linux.contains("systemctl --user enable --now phantom-coach.timer"), "linux hint");
        assert!(linux.contains("phantom-coach.service") && linux.contains("phantom-coach.timer"));
        assert!(linux.contains("OnCalendar=*-*-* 21:00:00"), "embeds the timer");

        let win = render_cli_unit(SchedulerTarget::Schtasks, "C:\\phantom.exe", sched);
        assert!(win.contains("schtasks /Create"), "windows command: {win}");
        assert!(win.contains("coach review") && win.contains("/SC DAILY"));
        assert!(win.contains("phantom does NOT auto-install"));
    }
}
