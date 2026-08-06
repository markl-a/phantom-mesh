// core/src/service/linux.rs
//
// Linux service install/uninstall/status. Registers `spectyn serve` as a
// systemd `--user` unit that runs at user login, with a journal-routed
// log stream and env-var propagation from the install-time shell.
//
// Extracted verbatim from `core/src/bin/spectyn.rs:8185-8326` (+ the
// `build_extra_env_systemd` helper at :6626-6644 and the
// `PROPAGATED_ENV_KEYS` const that's now shared via `service/mod.rs`)
// by PF-2b. Behavior is preserved. Structural changes vs the original:
//   - private `colored()` copy (matches PF-2a's pattern in `windows.rs`;
//     PF-2d will consolidate into a shared `util`)
//   - `LINUX_UNIT_NAME` and `run_service_subcommand` are `pub` so the bin's
//     `spectyn doctor` (which still lives in spectyn.rs) can reference
//     them via `spectyn_mesh::service::linux::*`
//   - `build_extra_env_systemd` is private to this module since only the
//     install path inside this file calls it

use std::path::{Path, PathBuf};
use std::process::Command;

/// Public so `spectyn doctor` (in the bin crate) can report registration
/// status of the same unit this module installs/manages.
pub const LINUX_UNIT_NAME: &str = "spectyn-mesh.service";

// PF-2d: `colored()` + `is_colored()` consolidated to
// `crate::util::term`. Local duplicates removed.
use crate::util::term::colored;

/// Pure file-side install: write the rendered unit file under
/// `<home>/.config/systemd/user/spectyn-mesh.service` and ensure the log
/// directory exists. Returns the unit-file path so callers can log it.
///
/// Split out from `run_service_subcommand("install")` so the file-side of
/// install can be exercised in tests without spawning `systemctl --user`
/// or touching the real `$HOME`. The async caller still runs `systemctl
/// daemon-reload` + `enable --now` after this returns.
///
/// Idempotent: re-running over an existing unit file overwrites it
/// cleanly (no leftover-symlink failure mode). Paired with
/// `uninstall_files_in_dir` for the round-trip idempotency test.
pub(crate) fn install_files_in_dir(home: &Path) -> anyhow::Result<(PathBuf, Vec<&'static str>)> {
    let unit_dir = home.join(".config/systemd/user");
    let unit_path = unit_dir.join(LINUX_UNIT_NAME);
    let log_path = home.join(".spectyn-mesh/data/spectyn-serve.log");

    let bin_self = std::env::current_exe()?;
    let bin = std::fs::canonicalize(&bin_self).unwrap_or(bin_self);
    let bin_str = bin.display().to_string();

    let cwd = std::env::current_dir().unwrap_or_else(|_| home.to_path_buf());
    let work_dir = if cwd.join("dist").is_dir() && cwd.join("scripts").is_dir() {
        cwd.display().to_string()
    } else {
        home.join(".spectyn-mesh").display().to_string()
    };

    std::fs::create_dir_all(&unit_dir)?;
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let (extra_env, env_names) = build_extra_env_systemd();
    let rendered = render_unit_file(
        &bin_str,
        &work_dir,
        &home.display().to_string(),
        &log_path.display().to_string(),
        &extra_env,
    );
    std::fs::write(&unit_path, &rendered)?;
    Ok((unit_path, env_names))
}

/// Pure file-side uninstall: remove the unit file (and the
/// `default.target.wants` WantedBy symlink, if `systemctl --user enable`
/// has previously created it) under `<home>/.config/systemd/user/`.
/// Returns true iff the unit file was actually removed. No-op if not
/// present, so round-trip idempotency holds: install → uninstall →
/// install again must succeed.
///
/// Note: in the full `run_service_subcommand("uninstall")` path,
/// `systemctl --user disable` is what normally removes the WantedBy
/// symlink. We additionally clean it here so round-tripping under tests
/// (which deliberately skip systemctl) doesn't leave dangling state, and
/// also so crash-recovery (uninstall after a failed `disable`) is safe.
pub(crate) fn uninstall_files_in_dir(home: &Path) -> anyhow::Result<bool> {
    let unit_dir = home.join(".config/systemd/user");
    let unit_path = unit_dir.join(LINUX_UNIT_NAME);
    let wants_link = unit_dir.join("default.target.wants").join(LINUX_UNIT_NAME);

    // Remove the WantedBy symlink first if it exists. We use
    // `symlink_metadata` so dangling symlinks (left over from a prior
    // crashed `systemctl disable`) are still cleaned up — `exists()`
    // follows links and would return false for those.
    if std::fs::symlink_metadata(&wants_link).is_ok() {
        std::fs::remove_file(&wants_link)?;
    }

    let removed = unit_path.exists();
    if removed {
        std::fs::remove_file(&unit_path)?;
    }
    Ok(removed)
}

/// Pure helper: parse `systemctl --user status <unit> --no-pager` stdout
/// into `(active, pid)`. Extracted from `run_service_subcommand`'s status
/// branch so the line-parsing contract can be pinned without spawning
/// systemctl.
///
/// - `active` is true iff some line contains both `Active:` and
///   `active (running)` (matches the original predicate exactly).
/// - `pid` is `Some(<first whitespace token after the colon>)` for the
///   first line whose trimmed prefix is `Main PID:`; `None` if no such
///   line exists. (The status branch falls back to `"?"` only when
///   `active` was true but no PID was found -- that fallback stays at
///   the call site to keep this helper pure.)
pub(crate) fn parse_systemctl_status(stdout: &str) -> (bool, Option<String>) {
    let active = stdout
        .lines()
        .any(|l| l.contains("Active:") && l.contains("active (running)"));
    let pid = stdout
        .lines()
        .find(|l| l.trim_start().starts_with("Main PID:"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|s| s.split_whitespace().next())
        .map(|s| s.to_string());
    (active, pid)
}

/// Pure helper: render the systemd unit file content by substituting
/// the template placeholders. Extracted from `run_service_subcommand`'s
/// install path so the substitution can be unit-tested without spawning
/// systemctl or touching disk.
fn render_unit_file(bin: &str, work_dir: &str, home: &str, log: &str, extra_env: &str) -> String {
    let tmpl: &str = include_str!("../../../templates/spectyn-mesh.service.tmpl");
    tmpl.replace("__SPECTYN_BIN__", bin)
        .replace("__WORK_DIR__", work_dir)
        .replace("__HOME__", home)
        .replace("__LOG__", log)
        .replace("__EXTRA_ENV__", extra_env)
}

/// Build the Linux systemd `Environment=...` lines fragment from the
/// current process's env. Returns (fragment, names_included).
/// Uses `super::PROPAGATED_ENV_KEYS` (the shared list with macOS).
fn build_extra_env_systemd() -> (String, Vec<&'static str>) {
    let mut s = String::new();
    let mut included: Vec<&'static str> = Vec::new();
    for &k in super::PROPAGATED_ENV_KEYS {
        if let Ok(v) = std::env::var(k) {
            if v.is_empty() {
                continue;
            }
            // systemd Environment= line: quote the value so spaces and
            // shell metachars survive intact. Inner double-quotes get
            // backslash-escaped per systemd.exec(5).
            let esc = v.replace('\\', "\\\\").replace('"', "\\\"");
            s.push_str(&format!("Environment=\"{}={}\"\n", k, esc));
            included.push(k);
        }
    }
    if s.ends_with('\n') {
        s.pop();
    }
    (s, included)
}

/// `spectyn service <action>` for Linux.
///
/// Usage:
///   spectyn service install     write unit + daemon-reload + start
///   spectyn service uninstall   stop + disable + remove unit
///   spectyn service status      systemctl --user status (parsed) + healthz
/// Resolve the port `spectyn serve` will actually listen on, to probe/print the
/// right healthz URL. The installed unit runs `spectyn serve` with no `--port`,
/// so this mirrors serve's remaining precedence: `SPECTYN_PORT` env >
/// `~/.spectyn-mesh/agents.toml [core].port` > 7878 (the `--port` flag the unit
/// never passes is intentionally out of scope). Best-effort; falls back to 7878.
fn resolve_serve_port() -> u16 {
    if let Ok(p) = std::env::var("SPECTYN_PORT") {
        if let Ok(n) = p.trim().parse::<u16>() {
            if n != 0 {
                return n;
            }
        }
    }
    crate::cli_config::resolve_home_dir()
        .ok()
        .map(|h| serve_port_from_config(&h))
        .unwrap_or(7878)
}

/// Path-injectable core of [`resolve_serve_port`]: read `[core].port` from
/// `<home>/.spectyn-mesh/agents.toml`, falling back to 7878 (missing file,
/// parse error, absent/out-of-range value). Separated so tests can target a
/// tempdir without mutating the process-global `$HOME`.
fn serve_port_from_config(home: &std::path::Path) -> u16 {
    // Typed deserialize (the proven path config.rs uses). serde ignores the
    // other agents.toml sections; a `u16` port that's out of range (e.g. 70000)
    // fails to deserialize → None → 7878. Port 0 is treated as unset.
    #[derive(serde::Deserialize)]
    struct Doc {
        core: Option<Core>,
    }
    #[derive(serde::Deserialize)]
    struct Core {
        port: Option<u16>,
    }
    std::fs::read_to_string(home.join(".spectyn-mesh/agents.toml"))
        .ok()
        .and_then(|s| toml::from_str::<Doc>(&s).ok())
        .and_then(|d| d.core)
        .and_then(|c| c.port)
        .filter(|&p| p != 0)
        .unwrap_or(7878)
}

pub async fn run_service_subcommand(action: &str) -> anyhow::Result<()> {
    let home = crate::cli_config::resolve_home_dir()?;
    let unit_path = home.join(".config/systemd/user").join(LINUX_UNIT_NAME);
    let log_path = home.join(".spectyn-mesh/data/spectyn-serve.log");
    // Probe/print the port serve actually uses, not a hardcoded 7878 (D6).
    let port = resolve_serve_port();

    match action {
        "install" => {
            // File-side install (write unit + create log dir) factored to
            // `install_files_in_dir` for testability. We re-derive
            // `bin_str` / `work_dir` here purely for the user-facing log
            // lines below; the helper computes its own identical values.
            let bin_self = std::env::current_exe()?;
            let bin = std::fs::canonicalize(&bin_self).unwrap_or(bin_self);
            let bin_str = bin.display().to_string();
            let cwd = std::env::current_dir().unwrap_or_else(|_| home.clone());
            let work_dir = if cwd.join("dist").is_dir() && cwd.join("scripts").is_dir() {
                cwd.display().to_string()
            } else {
                home.join(".spectyn-mesh").display().to_string()
            };

            let (unit_path, env_names) = install_files_in_dir(&home)?;
            eprintln!("{} Wrote {}", colored("◆", 35), unit_path.display());
            if !env_names.is_empty() {
                eprintln!(
                    "    propagated env: {} key(s) → {}",
                    env_names.len(),
                    env_names.join(", ")
                );
            }

            let _ = Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status();
            let s_enable = Command::new("systemctl")
                .args(["--user", "enable", "--now", LINUX_UNIT_NAME])
                .status()?;
            if !s_enable.success() {
                anyhow::bail!("systemctl --user enable --now failed");
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            eprintln!(
                "{} Enabled and started '{}'",
                colored("✓", 32),
                LINUX_UNIT_NAME
            );
            eprintln!("    binary:    {}", bin_str);
            eprintln!("    cwd:       {}", work_dir);
            eprintln!("    log:       {}", log_path.display());
            eprintln!("    Verify:    curl http://127.0.0.1:{port}/healthz");
            eprintln!();
            eprintln!(
                "{} run `loginctl enable-linger $USER` (needs sudo) so the \
                 service stays alive between logins.",
                colored("⚠", 33)
            );
            eprintln!();
            eprintln!(
                "{} {} for a full environment health check.",
                colored("→", 36),
                colored("spectyn doctor", 33)
            );
            Ok(())
        }
        "uninstall" => {
            let _ = Command::new("systemctl")
                .args(["--user", "disable", "--now", LINUX_UNIT_NAME])
                .status();
            let removed = uninstall_files_in_dir(&home)?;
            if removed {
                eprintln!("{} Removed {}", colored("◆", 35), unit_path.display());
            }
            let _ = Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status();
            eprintln!("{} Uninstalled.", colored("✓", 32));
            Ok(())
        }
        "status" => {
            let q = Command::new("systemctl")
                .args(["--user", "status", LINUX_UNIT_NAME, "--no-pager"])
                .output()?;
            let body = String::from_utf8_lossy(&q.stdout).to_string();
            // exit 0 = active, 3 = inactive — both still mean "registered"
            let registered = unit_path.exists();
            let (active, pid_opt) = parse_systemctl_status(&body);
            let pid = pid_opt.unwrap_or_else(|| "?".into());

            let healthz_url = format!("http://127.0.0.1:{port}/healthz");
            let probe = Command::new("curl")
                .args([
                    "-s",
                    "--max-time",
                    "2",
                    "-o",
                    "/dev/null",
                    "-w",
                    "%{http_code}",
                    healthz_url.as_str(),
                ])
                .output();
            let healthz_code = probe
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();

            println!(
                "{} {}",
                colored("spectyn service status", 36),
                colored(LINUX_UNIT_NAME, 90)
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
                println!(
                    "  active     : {}",
                    if active {
                        colored("yes (running)", 32)
                    } else {
                        colored("no", 31)
                    }
                );
                if active {
                    println!("  pid        : {}", pid);
                }
                println!("  unit       : {}", unit_path.display());
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
                println!(
                    "  hint       : journalctl --user -u {} -n 20",
                    LINUX_UNIT_NAME
                );
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
    fn serve_port_from_config_reads_core_port() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".spectyn-mesh");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("agents.toml"), "[core]\nport = 17890\n").unwrap();
        assert_eq!(serve_port_from_config(tmp.path()), 17890);
    }

    #[test]
    fn serve_port_from_config_defaults_when_absent_or_invalid() {
        // No agents.toml at all → 7878.
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(serve_port_from_config(tmp.path()), 7878);
        // Present but no [core].port → 7878.
        let dir = tmp.path().join(".spectyn-mesh");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("agents.toml"), "[providers.gemini]\napi_key=\"x\"\n").unwrap();
        assert_eq!(serve_port_from_config(tmp.path()), 7878);
        // Out-of-range port → 7878.
        std::fs::write(dir.join("agents.toml"), "[core]\nport = 70000\n").unwrap();
        assert_eq!(serve_port_from_config(tmp.path()), 7878);
    }

    /// Moved from spectyn.rs:6718-6731. Pins the systemd `Environment=`
    /// fragment format. Uses `with_env` from the bin's tests — but
    /// since we're now in the lib, replicate the env-snapshot pattern
    /// inline with the shared `env_lock`.
    #[test]
    fn extra_env_systemd_emits_quoted_environment_lines() {
        let _g = crate::env_lock::acquire();
        let key = "GROQ_API_KEY";
        let prev = std::env::var(key).ok();
        std::env::set_var(key, "gsk_test_value");
        let (s, names) = build_extra_env_systemd();
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        assert!(names.contains(&"GROQ_API_KEY"));
        assert!(s.contains(r#"Environment="GROQ_API_KEY=gsk_test_value""#));
    }

    /// V5+PF-2b LIN P0: pin the structural contract of the rendered
    /// systemd unit file. If the template ever drops `[Unit]`/`[Service]`/
    /// `[Install]` sections, or the `ExecStart` / `WantedBy` /
    /// `Restart=on-failure` / `Type=` directives, or stops substituting
    /// `__SPECTYN_BIN__` / `__EXTRA_ENV__`, this test fails loudly.
    #[test]
    fn systemd_unit_file_content_correct() {
        let extra = "Environment=\"GROQ_API_KEY=gsk_test_value\"";
        let rendered = render_unit_file(
            "/usr/local/bin/spectyn",
            "/home/u/.spectyn-mesh",
            "/home/u",
            "/home/u/.spectyn-mesh/data/spectyn-serve.log",
            extra,
        );

        // [Unit] section: heading + Description + ordering directive
        assert!(rendered.contains("[Unit]"), "missing [Unit]:\n{}", rendered);
        assert!(
            rendered.contains("Description="),
            "missing Description=:\n{}",
            rendered
        );
        assert!(
            rendered.contains("After=network-online.target"),
            "missing After= ordering directive:\n{}",
            rendered
        );

        // [Service] section: heading + Type + ExecStart with substituted
        // bin path + Restart=on-failure
        assert!(
            rendered.contains("[Service]"),
            "missing [Service]:\n{}",
            rendered
        );
        let has_type = rendered.contains("Type=simple") || rendered.contains("Type=notify");
        assert!(has_type, "missing Type=simple|notify:\n{}", rendered);
        assert!(
            rendered.contains("ExecStart=/usr/local/bin/spectyn serve"),
            "ExecStart not substituted with bin path:\n{}",
            rendered
        );
        assert!(
            rendered.contains("Restart=on-failure"),
            "missing Restart=on-failure:\n{}",
            rendered
        );

        // [Install] section: heading + WantedBy
        assert!(
            rendered.contains("[Install]"),
            "missing [Install]:\n{}",
            rendered
        );
        assert!(
            rendered.contains("WantedBy=default.target"),
            "missing WantedBy=default.target:\n{}",
            rendered
        );

        // Placeholder substitution: __EXTRA_ENV__ is replaced (not left
        // literal) and the supplied Environment= line is present.
        assert!(
            !rendered.contains("__EXTRA_ENV__"),
            "__EXTRA_ENV__ placeholder not substituted:\n{}",
            rendered
        );
        assert!(
            !rendered.contains("__SPECTYN_BIN__"),
            "__SPECTYN_BIN__ placeholder not substituted:\n{}",
            rendered
        );
        assert!(
            rendered.contains(r#"Environment="GROQ_API_KEY=gsk_test_value""#),
            "extra_env line not propagated into rendered unit:\n{}",
            rendered
        );
    }

    /// V5+PF-2b LIN P0: pin the systemctl status output parser contract.
    /// Four cases exercise the predicate (`Active:` + `active (running)`)
    /// and the `Main PID:` extraction across the shapes systemd actually
    /// emits: running, inactive (dead), failed, and empty (no unit /
    /// daemon-reload not run yet).
    #[test]
    fn status_parses_systemctl_output() {
        // 1. Active running — taken from real `systemctl --user status` output.
        let running = "\
● spectyn-mesh.service - Spectyn Mesh daemon
     Loaded: loaded (/home/u/.config/systemd/user/spectyn-mesh.service; enabled)
     Active: active (running) since Mon 2026-05-18 09:12:33 PDT; 2min ago
   Main PID: 8842 (spectyn)
      Tasks: 7 (limit: 18874)
     Memory: 12.4M
        CPU: 145ms
     CGroup: /user.slice/user-1000.slice/...
";
        let (active, pid) = parse_systemctl_status(running);
        assert!(active, "running output must be detected as active");
        assert_eq!(pid.as_deref(), Some("8842"));

        // 2. Inactive (dead) — unit is registered but stopped.
        let inactive = "\
● spectyn-mesh.service - Spectyn Mesh daemon
     Loaded: loaded (/home/u/.config/systemd/user/spectyn-mesh.service; disabled)
     Active: inactive (dead)
";
        let (active, pid) = parse_systemctl_status(inactive);
        assert!(!active, "inactive output must not be detected as active");
        assert_eq!(pid, None, "no Main PID: line → None");

        // 3. Failed (start failed) — Active: failed; usually no Main PID line
        //    but systemd does sometimes leave the last PID around. Predicate
        //    requires `active (running)` substring, so this must be inactive.
        let failed = "\
● spectyn-mesh.service - Spectyn Mesh daemon
     Loaded: loaded (/home/u/.config/systemd/user/spectyn-mesh.service; enabled)
     Active: failed (Result: exit-code) since Mon 2026-05-18 09:15:00 PDT; 5s ago
    Process: 8901 ExecStart=/usr/local/bin/spectyn serve (code=exited, status=1)
";
        let (active, pid) = parse_systemctl_status(failed);
        assert!(!active, "failed output must not be detected as active");
        assert_eq!(pid, None, "no `Main PID:` line in failed output → None");

        // 4. Empty stdout — what we get when systemctl errors before printing
        //    (e.g., user bus not available, unit file missing pre-install).
        let (active, pid) = parse_systemctl_status("");
        assert!(!active);
        assert_eq!(pid, None);
    }

    /// V5+PF-2b LIN P0: pin that the file-side of `service install` and
    /// `service uninstall` round-trip cleanly. Specifically:
    ///
    ///   install → unit file exists
    ///   uninstall → unit file (and any `default.target.wants` symlink)
    ///                are both gone
    ///   install AGAIN → succeeds and re-creates the unit file
    ///
    /// Why this matters: a bug where uninstall left a dangling symlink
    /// (or where install bailed because the file already existed) would
    /// make the second install fail. Real users hit this when they
    /// uninstall-then-reinstall to pick up a binary path change.
    ///
    /// We deliberately exercise only the file-side path
    /// (`install_files_in_dir` / `uninstall_files_in_dir`) — the
    /// `systemctl --user enable/disable` calls in
    /// `run_service_subcommand` would touch the real user bus, which is
    /// not appropriate for a unit test (and isn't reliably available
    /// inside WSL2 without `--systemd=true` in /etc/wsl.conf).
    #[test]
    fn install_then_uninstall_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path();

        let unit_path = home.join(".config/systemd/user").join(LINUX_UNIT_NAME);
        let wants_link = home
            .join(".config/systemd/user/default.target.wants")
            .join(LINUX_UNIT_NAME);

        // Round 1: install writes the unit file.
        let (p1, _) = install_files_in_dir(home).expect("install round 1");
        assert_eq!(p1, unit_path);
        assert!(
            unit_path.exists(),
            "install round 1 should create unit file at {}",
            unit_path.display()
        );

        // Simulate `systemctl --user enable spectyn-mesh.service` having
        // run (which is what the real install path does after writing
        // the unit file): a relative symlink under
        // `default.target.wants/` pointing back at the unit file. The
        // uninstall path must clean this up even when systemctl is
        // skipped, otherwise the next install can fail or leave dangling
        // state on disk.
        std::fs::create_dir_all(wants_link.parent().unwrap()).expect("wants dir");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&unit_path, &wants_link).expect("symlink");
        assert!(
            std::fs::symlink_metadata(&wants_link).is_ok(),
            "wants symlink should exist after we stub `systemctl enable`"
        );

        // Round 2: uninstall removes both the unit file and the
        // WantedBy symlink.
        let removed = uninstall_files_in_dir(home).expect("uninstall");
        assert!(removed, "uninstall should report file was removed");
        assert!(
            !unit_path.exists(),
            "uninstall should remove unit file at {}",
            unit_path.display()
        );
        assert!(
            std::fs::symlink_metadata(&wants_link).is_err(),
            "uninstall should remove WantedBy symlink at {}",
            wants_link.display()
        );

        // Round 3: idempotency — uninstall a second time on a clean dir
        // is a no-op (no Err), and reports nothing was removed.
        let removed_again = uninstall_files_in_dir(home).expect("uninstall on clean dir");
        assert!(
            !removed_again,
            "second uninstall on clean dir should report no removal"
        );

        // Round 4: install AGAIN must succeed. This is the core
        // round-trip idempotency assertion — if uninstall had left
        // anything behind that made install bail, this would fail.
        let (p2, _) = install_files_in_dir(home).expect("install round 2");
        assert_eq!(p2, unit_path);
        assert!(
            unit_path.exists(),
            "install round 2 should re-create unit file at {}",
            unit_path.display()
        );

        // And install over an existing file must also be idempotent
        // (overwriting the unit file is the documented behavior — users
        // re-run install to pick up env/path changes).
        let (p3, _) = install_files_in_dir(home).expect("install round 3");
        assert_eq!(p3, unit_path);
        assert!(unit_path.exists());
    }
}
