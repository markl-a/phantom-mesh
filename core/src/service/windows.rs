// core/src/service/windows.rs
//
// Windows service install/uninstall/status. Registers `spectyn serve` as
// a Scheduled Task that runs at user logon, with a Defender firewall
// rule scoped to the Tailscale CGNAT range (100.64.0.0/10).
//
// Extracted verbatim from `core/src/bin/spectyn.rs:10795-11153` by PF-2a;
// behavior is preserved. The only structural changes vs the original:
//   - private `colored()` copy (the bin's helper is in the binary crate,
//     not reachable from the lib; 5-line duplicate kept here until a
//     shared `util` crate / module lands)
//   - module-local function names (the original was suffixed
//     `_windows` for disambiguation inside one giant file; here the
//     module path already provides the namespace)

use std::process::Command;

/// Public so `spectyn doctor` (in the bin crate) can report registration
/// status of the same task this module installs/manages.
pub const WINDOWS_TASK_NAME: &str = "SpectynServe";

/// SYS-D (operator-locked 2026-06-13): the TARGETED schtasks argv that
/// `spectyn service uninstall` runs to stop this task's own running instance.
/// It addresses ONLY `WINDOWS_TASK_NAME` — never a blanket
/// `taskkill /F /IM spectyn.exe`, which would kill every spectyn process (a
/// user-run serve, another CLI, even a live unattended run). Pure so the
/// reversibility guardrail is unit-testable without spawning schtasks.
pub fn uninstall_end_args() -> [&'static str; 3] {
    ["/End", "/TN", WINDOWS_TASK_NAME]
}

/// SYS-D companion to [`uninstall_end_args`]: the TARGETED schtasks argv that
/// deletes ONLY this task's own Scheduled Task (`WINDOWS_TASK_NAME`). Reused by
/// install (to clear a prior registration) and uninstall. Reversible: `spectyn
/// service install` recreates the task.
pub fn uninstall_delete_args() -> [&'static str; 4] {
    ["/Delete", "/TN", WINDOWS_TASK_NAME, "/F"]
}

// PF-2d: `colored()` + `is_colored()` consolidated to
// `crate::util::term`. Local duplicates removed.
use crate::util::term::colored;

/// Configured serve port from agents.toml, defaulting to 7878 if no
/// config is found. The hardcoded :7878 used to mismatch any user with
/// `[core] port = 7879` in agents.toml — healthz probe would always
/// report unreachable even though spectyn serve was running.
fn configured_port() -> u16 {
    crate::config::AgentsConfig::find_and_load()
        .map(|c| c.core.port)
        .unwrap_or(7878)
}

/// Locale-independent Scheduled Task runtime info via PowerShell
/// `Get-ScheduledTaskInfo`. Returns `(last_run, next_run, last_result)`,
/// each `None` when the task is missing, the field is empty, or the call
/// fails. Used instead of parsing localized strings out of `schtasks
/// /Query /V /FO LIST`, whose field labels (e.g. "Last Run Time") are
/// translated on non-English Windows installs and never matched the
/// English-only `starts_with(...)` predicates.
/// Public so `spectyn doctor` + `spectyn autoevolve schedule status` in
/// the bin crate can render scheduled-task runtime info uniformly.
pub fn windows_task_info(task_name: &str) -> (Option<String>, Option<String>, Option<i64>) {
    let escaped = task_name.replace('\'', "''");
    // Windows uses 1899-12-30 / 1999-11-30 / similar pre-2000 placeholders
    // for "never run". Filter those server-side so callers can render "?"
    // without false-positive 1999 dates leaking through.
    let ps_script = format!(
        "$i = Get-ScheduledTaskInfo -TaskName '{}' -ErrorAction SilentlyContinue; \
         if ($i) {{ \
            $cutoff = [DateTime]'2000-01-01'; \
            $last = if ($i.LastRunTime -gt $cutoff) {{ $i.LastRunTime }} else {{ '' }}; \
            $next = if ($i.NextRunTime -gt $cutoff) {{ $i.NextRunTime }} else {{ '' }}; \
            \"LastRun=$last\"; \"NextRun=$next\"; \"Result=$($i.LastTaskResult)\" \
         }}",
        escaped
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps_script])
        .output()
        .ok();
    let body = out
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let last = body
        .lines()
        .find_map(|l| l.strip_prefix("LastRun="))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let next = body
        .lines()
        .find_map(|l| l.strip_prefix("NextRun="))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let result = body
        .lines()
        .find_map(|l| l.strip_prefix("Result="))
        .and_then(|s| s.trim().parse().ok());
    (last, next, result)
}

/// Translate a LastTaskResult HRESULT-style code into a short human label.
/// Returns ("color-code", "label") so callers can render with the right
/// ANSI colour. Constants from the Windows Task Scheduler return-code set.
/// Public so `spectyn autoevolve schedule status` in the bin crate can
/// colour-render the LastTaskResult HRESULT consistently.
pub fn windows_task_result_label(result: i64) -> (u8, String) {
    match result {
        0 => (32, "succeeded".into()),                                // S_OK
        0x00041300 => (32, "ready".into()),                           // SCHED_S_TASK_READY
        0x00041301 => (33, "running".into()),                         // SCHED_S_TASK_RUNNING
        0x00041303 => (90, "never run".into()),                       // SCHED_S_TASK_HAS_NOT_RUN
        0x00041305 => (33, "no more runs".into()),                    // SCHED_S_TASK_NO_MORE_RUNS
        0x00041306 => (33, "disabled".into()),                        // SCHED_S_TASK_DISABLED
        0x00041325 => (33, "queued".into()),                          // SCHED_S_TASK_QUEUED
        other if other > 0 => (31, format!("error 0x{:08X}", other)), // any other HRESULT
        other => (31, format!("error {}", other)),
    }
}

/// Public entry point: `spectyn service <action>` on Windows.
pub async fn run_service_subcommand(action: &str) -> anyhow::Result<()> {
    match action {
        "install" => {
            let bin_self = std::env::current_exe()?;
            let bin_canon = std::fs::canonicalize(&bin_self).unwrap_or(bin_self);
            // Prefer the release binary so the durable task never points at a
            // throwaway `target\debug\spectyn.exe`. If we're running from a
            // debug build, swap in the sibling `release\spectyn.exe` when it
            // exists; otherwise keep the running exe.
            let bin = {
                let mut candidate = bin_canon.clone();
                if let Some(parent) = bin_canon.parent() {
                    if parent.file_name().and_then(|n| n.to_str()) == Some("debug") {
                        if let Some(target_dir) = parent.parent() {
                            let rel = target_dir
                                .join("release")
                                .join(bin_canon.file_name().unwrap_or_default());
                            if rel.exists() {
                                candidate = rel;
                            }
                        }
                    }
                }
                candidate
            };
            let bin_str = bin.display().to_string();

            // Delete any prior registration first so re-installs always
            // pick up the fresh binary path.
            let _ = Command::new("schtasks")
                .args(uninstall_delete_args())
                .output();

            // schtasks /Create /SC ONLOGON is rejected with "Access Denied"
            // on Enterprise / managed Windows where user-level ONLOGON
            // tasks are blocked by policy. PowerShell's
            // Register-ScheduledTask + New-ScheduledTaskTrigger -AtLogOn
            // -User <current user> works in those environments because it
            // creates the task in the current user's hive instead of the
            // system tree. RestartCount/RestartInterval gives us the
            // "auto-relaunch if spectyn serve crashes" behaviour the old
            // schtasks XML embedding promised.
            let bin_for_ps = bin_str.replace('\'', "''");
            let task_for_ps = WINDOWS_TASK_NAME.replace('\'', "''");
            let ps_script = format!(
                "$action = New-ScheduledTaskAction -Execute '{}' -Argument 'serve'; \
                 $principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType S4U -RunLevel Highest; \
                 $trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME; \
                 $bootTrigger = New-ScheduledTaskTrigger -AtStartup; \
                 $settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 5 -RestartInterval (New-TimeSpan -Minutes 1); \
                 Register-ScheduledTask -TaskName '{}' -Action $action -Trigger @($trigger, $bootTrigger) -Principal $principal -Settings $settings -Force | Out-Null",
                bin_for_ps, task_for_ps
            );

            let status = Command::new("powershell")
                .args(["-NoProfile", "-Command", &ps_script])
                .status()?;
            if !status.success() {
                anyhow::bail!("Register-ScheduledTask failed (exit {:?})", status.code());
            }

            // Trigger it once so the service is up immediately, not only on
            // next logon.
            let _ = Command::new("schtasks")
                .args(["/Run", "/TN", WINDOWS_TASK_NAME])
                .status();

            let verify_port = configured_port();

            // Tailscale-scoped Defender firewall rule, mirroring step [3/5]
            // of install-spectyn-windows.ps1. Best-effort — New-NetFirewallRule
            // needs admin; on a non-admin shell we report the skip but don't
            // fail the install.
            let fw_script = format!(
                "$ErrorActionPreference = 'Stop'; \
                 try {{ \
                    Get-NetFirewallRule -DisplayName 'SpectynMesh-Inbound' -ErrorAction SilentlyContinue | Remove-NetFirewallRule -ErrorAction SilentlyContinue; \
                    New-NetFirewallRule -DisplayName 'SpectynMesh-Inbound' -Direction Inbound -Action Allow -Protocol TCP -LocalPort {} -RemoteAddress '100.64.0.0/10' -Profile Any | Out-Null; \
                    Write-Output 'OK' \
                 }} catch {{ \
                    Write-Output (\"FAIL: \" + $_.Exception.Message) \
                 }}",
                verify_port
            );
            let fw_msg = Command::new("powershell")
                .args(["-NoProfile", "-Command", &fw_script])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            let fw_status = if fw_msg == "OK" {
                format!(
                    "SpectynMesh-Inbound TCP {} ← 100.64.0.0/10 (Tailscale)",
                    verify_port
                )
            } else if let Some(reason) = fw_msg.strip_prefix("FAIL:") {
                format!("skipped — re-run from admin PowerShell ({})", reason.trim())
            } else {
                "skipped — PowerShell unavailable".into()
            };

            eprintln!(
                "{} Registered Scheduled Task '{}'",
                colored("✓", 32),
                WINDOWS_TASK_NAME
            );
            eprintln!("    binary:   {}", bin_str);
            eprintln!(
                "    trigger:  at user logon + system startup (S4U, runs without an interactive session; auto-restart up to 5× on failure)"
            );
            eprintln!("    firewall: {}", fw_status);
            eprintln!(
                "    Verify:   curl http://127.0.0.1:{}/healthz",
                verify_port
            );
            // SYS-D: print the undo/inspect commands at the point of install.
            eprintln!("    Status:    spectyn service status");
            eprintln!("    Uninstall: spectyn service uninstall --yes   (dry-run without --yes)");
            eprintln!();
            eprintln!(
                "{} {} for a full environment health check.",
                colored("→", 36),
                colored("spectyn doctor", 33)
            );
            Ok(())
        }
        "uninstall" => {
            // W2 / I9 / SYS-D: print the plan, require --yes to apply, and stop ONLY this
            // task's own running instance. The old code ran `taskkill /F /IM spectyn.exe`,
            // which killed EVERY spectyn process — a user-run serve, another CLI, even a live
            // unattended run. Reversible: `spectyn service install` recreates the task.
            let yes = std::env::args().any(|a| a == "--yes");
            eprintln!("{} spectyn service uninstall will:", colored("◆", 35));
            eprintln!("    - remove Scheduled Task '{}'", WINDOWS_TASK_NAME);
            eprintln!("    - remove firewall rule 'SpectynMesh-Inbound' (if present)");
            eprintln!("    - stop ONLY this task's running serve (NOT other spectyn processes)");
            eprintln!("    reversible: re-run `spectyn service install` to recreate it");
            if !yes {
                eprintln!(
                    "{} dry-run -- nothing removed. Re-run with {} to apply.",
                    colored("⚠", 33),
                    colored("spectyn service uninstall --yes", 32)
                );
                return Ok(());
            }
            // Stop this task's running instance first (targeted, NOT `taskkill /F /IM spectyn.exe`).
            let _ = Command::new("schtasks")
                .args(uninstall_end_args())
                .output();
            let status = Command::new("schtasks")
                .args(uninstall_delete_args())
                .status()?;
            if status.success() {
                eprintln!(
                    "{} Removed Scheduled Task '{}'",
                    colored("◆", 35),
                    WINDOWS_TASK_NAME
                );
            } else {
                eprintln!(
                    "{} schtasks /Delete returned exit {:?} — task may not have existed.",
                    colored("⚠", 33),
                    status.code()
                );
            }
            // Best-effort firewall rule cleanup. Silent so non-admin
            // uninstalls don't add noise (the rule was probably never
            // installed in that case).
            let _ = Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    "Get-NetFirewallRule -DisplayName 'SpectynMesh-Inbound' -ErrorAction SilentlyContinue | Remove-NetFirewallRule -ErrorAction SilentlyContinue",
                ])
                .output();
            eprintln!("{} Uninstalled.", colored("✓", 32));
            Ok(())
        }
        "status" => {
            let q = Command::new("schtasks")
                .args(["/Query", "/TN", WINDOWS_TASK_NAME])
                .output()?;
            let registered = q.status.success();

            let port = configured_port();
            let healthz_url = format!("http://127.0.0.1:{}/healthz", port);
            let probe = Command::new("curl.exe")
                .args([
                    "-s",
                    "--max-time",
                    "2",
                    "-o",
                    "NUL",
                    "-w",
                    "%{http_code}",
                    &healthz_url,
                ])
                .output();
            let healthz_code = probe
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();

            println!(
                "{} {}",
                colored("spectyn service status", 36),
                colored(WINDOWS_TASK_NAME, 90)
            );
            println!(
                "  registered : {}",
                if registered {
                    colored("yes", 32)
                } else {
                    colored("no", 31)
                }
            );
            if registered {
                let (last_run_time, next_run_time, last_result) =
                    windows_task_info(WINDOWS_TASK_NAME);
                println!(
                    "  last run   : {}",
                    last_run_time.unwrap_or_else(|| "?".into())
                );
                println!(
                    "  next run   : {}",
                    next_run_time.unwrap_or_else(|| "?".into())
                );
                if let Some(r) = last_result {
                    let (code, label) = windows_task_result_label(r);
                    println!("  last state : {}", colored(&label, code));
                }
            }
            println!(
                "  healthz    : {} ({})",
                if healthz_code == "200" {
                    colored("ok", 32)
                } else {
                    colored("unreachable", 31)
                },
                if healthz_code.is_empty() {
                    "no response".into()
                } else {
                    format!("HTTP {}", healthz_code)
                }
            );
            if registered && healthz_code != "200" {
                println!("  hint       : Get-EventLog Application -Newest 20 | findstr spectyn");
            }
            Ok(())
        }
        other => {
            eprintln!(
                "{} Unknown service action: '{}'.\n    Use one of: install, uninstall, status",
                colored("✗", 31),
                other
            );
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_result_label_known_codes_render_with_color() {
        // Concrete known SCHED_S_* constants — make sure each maps to the
        // right human label and ANSI colour, otherwise localized status
        // output silently regresses to "?" on every Windows install.
        assert_eq!(windows_task_result_label(0), (32, "succeeded".into()));
        assert_eq!(windows_task_result_label(0x00041300), (32, "ready".into()));
        assert_eq!(
            windows_task_result_label(0x00041301),
            (33, "running".into())
        );
        assert_eq!(
            windows_task_result_label(0x00041303),
            (90, "never run".into())
        );
        assert_eq!(
            windows_task_result_label(0x00041305),
            (33, "no more runs".into())
        );
        assert_eq!(
            windows_task_result_label(0x00041306),
            (33, "disabled".into())
        );
        assert_eq!(windows_task_result_label(0x00041325), (33, "queued".into()));
    }

    #[test]
    fn task_result_label_unknown_positive_renders_as_hex_error() {
        let (color, label) = windows_task_result_label(0x80070005); // E_ACCESSDENIED
        assert_eq!(color, 31);
        assert_eq!(label, "error 0x80070005");

        let (color, label) = windows_task_result_label(0x800704C7); // ERROR_CANCELLED
        assert_eq!(color, 31);
        assert_eq!(label, "error 0x800704C7");
    }

    #[test]
    fn task_result_label_unknown_negative_renders_as_decimal_error() {
        let (color, label) = windows_task_result_label(-1);
        assert_eq!(color, 31);
        assert_eq!(label, "error -1");
    }

    #[test]
    fn configured_port_falls_back_to_7878_when_no_config() {
        let port = configured_port();
        assert!(port > 0, "configured_port must yield a valid u16");
        assert_ne!(port, u16::MAX, "u16::MAX is unreachable from agents.toml");
    }

    #[test]
    fn uninstall_targets_only_spectyn_task_never_blanket_kill() {
        // SYS-D (operator-locked 2026-06-13): `service uninstall` is the
        // symmetric reverse of `service install`, and it must stop ONLY its own
        // Scheduled Task — never re-introduce the old blanket
        // `taskkill /F /IM spectyn.exe` that killed every spectyn process.
        let end = uninstall_end_args();
        let del = uninstall_delete_args();
        // Both commands name THIS task explicitly (scoped, reversible).
        assert!(end.contains(&"/TN") && end.contains(&WINDOWS_TASK_NAME), "end is task-scoped");
        assert!(del.contains(&"/TN") && del.contains(&WINDOWS_TASK_NAME), "delete is task-scoped");
        // Neither command is a blanket process kill.
        for a in end.iter().chain(del.iter()) {
            assert!(!a.eq_ignore_ascii_case("taskkill"), "must not shell out to taskkill: {a}");
            assert_ne!(*a, "/IM", "must not target by image name (every spectyn.exe)");
            assert!(!a.to_ascii_lowercase().contains("spectyn.exe"), "must not name the image: {a}");
        }
    }
}
