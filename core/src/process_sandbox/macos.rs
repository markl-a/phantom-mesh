// core/src/sandbox/macos.rs
//
// macOS Seatbelt (sandbox-exec) wrapping for shell tools.
//
// Strategy: deny-by-default; allow reads everywhere; restrict writes
// to cwd, `$HOME`, `/tmp`, `/private/tmp`; allow network and exec
// because almost every dev tool needs them. This is a safety net
// against accidental `rm -rf /` and writes to system paths (`/etc`,
// `/usr/local`, `/Applications`, `/Library`), NOT a substitute for
// full process isolation.
//
// Activation: on by default whenever `/usr/bin/sandbox-exec` exists.
// Opt out per-session with `SPECTYN_SANDBOX=0`. Pass
// `SPECTYN_SANDBOX=strict` to drop `$HOME` from the writable list
// (only cwd + tmp).
//
// Caveats:
// - sandbox-exec is technically deprecated by Apple but still ships
//   in macOS 15 and is what every tool (xcodebuild, swift package
//   manager, etc.) uses internally. Replacement TBD.
// - Profile uses `(param "...")` substitution to avoid string
//   escaping issues with paths containing spaces.
//
// PF-6: Moved from `core/src/platform/macos.rs::sandbox` to this
// file. `wrap` kept `pub(crate)` for `platform/macos.rs` callers.

use super::Sandbox;
use tokio::process::Command;

/// Inline SBPL profile. `(param "CWD")` and `(param "HOME")` are
/// substituted via `-D NAME=value` flags at spawn time.
const SBPL_PROFILE_RELAXED: &str = r#"(version 1)
(deny default)
(allow process-fork)
(allow process-exec*)
(allow file-read*)
(allow file-write*
    (subpath (param "CWD"))
    (subpath (param "HOME"))
    (subpath "/private/tmp")
    (subpath "/tmp")
    (subpath "/private/var/folders")
    (regex #"^/dev/(null|zero|tty|fd|std(in|out|err))"))
(allow network*)
(allow signal)
(allow ipc-posix-shm*)
(allow ipc-posix-sem)
(allow mach-lookup)
(allow mach-priv-task-port)
(allow sysctl-read)
(allow iokit-open)
(allow file-issue-extension)
"#;

/// Strict variant — drops `$HOME` from writable subpaths so the agent
/// can't trash dotfiles. Pass `SPECTYN_SANDBOX=strict`.
const SBPL_PROFILE_STRICT: &str = r#"(version 1)
(deny default)
(allow process-fork)
(allow process-exec*)
(allow file-read*)
(allow file-write*
    (subpath (param "CWD"))
    (subpath "/private/tmp")
    (subpath "/tmp")
    (subpath "/private/var/folders")
    (regex #"^/dev/(null|zero|tty|fd|std(in|out|err))"))
(allow network*)
(allow signal)
(allow ipc-posix-shm*)
(allow ipc-posix-sem)
(allow mach-lookup)
(allow mach-priv-task-port)
(allow sysctl-read)
(allow iokit-open)
(allow file-issue-extension)
"#;

fn mode() -> &'static str {
    match std::env::var("SPECTYN_SANDBOX").as_deref() {
        Ok("0") | Ok("off") | Ok("disabled") => "off",
        Ok("strict") => "strict",
        _ => "relaxed",
    }
}

/// Returns `Some((program, args))` rewritten to invoke through
/// sandbox-exec, or `None` when the sandbox is disabled or unavailable.
/// Callers fall back to spawning `(program, args)` unmodified.
///
/// PF-6: was `pub(super) fn wrap` inside `platform/macos.rs::sandbox`;
/// promoted to `pub(crate)` so callers from `crate::platform::macos`
/// (a sibling, not a parent now) can still reach it.
pub(crate) fn wrap(program: &str, args: &[String]) -> Option<(String, Vec<String>)> {
    match mode() {
        "off" => return None,
        _ => {}
    }
    // sandbox-exec ships with macOS — but bail safely if it ever
    // disappears so we don't break shell tools entirely.
    if !std::path::Path::new("/usr/bin/sandbox-exec").exists() {
        return None;
    }
    // Canonicalize: macOS `/var/folders/...` (mktemp default) is a
    // symlink to `/private/var/folders/...`, and sandbox-exec checks
    // the *resolved* path against the profile. Without this realpath()
    // pass, writes inside a tempdir-cwd get denied even though the
    // profile says cwd is writable — because the param holds the
    // symlink form, not what the kernel sees. Fall back to the
    // unresolved path if canonicalize fails (path missing, permission
    // denied).
    let raw_cwd = std::env::current_dir().unwrap_or_else(|_| "/private/tmp".into());
    let cwd = std::fs::canonicalize(&raw_cwd)
        .unwrap_or(raw_cwd)
        .to_str()
        .map(String::from)
        .unwrap_or_else(|| "/private/tmp".to_string());
    let raw_home = std::env::var("HOME").unwrap_or_else(|_| "/private/tmp".into());
    let home = std::fs::canonicalize(&raw_home)
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or(raw_home);
    let profile = if mode() == "strict" {
        SBPL_PROFILE_STRICT
    } else {
        SBPL_PROFILE_RELAXED
    };

    let mut wrapped: Vec<String> = Vec::with_capacity(args.len() + 7);
    wrapped.push("-D".to_string());
    wrapped.push(format!("CWD={}", cwd));
    wrapped.push("-D".to_string());
    wrapped.push(format!("HOME={}", home));
    wrapped.push("-p".to_string());
    wrapped.push(profile.to_string());
    wrapped.push(program.to_string());
    wrapped.extend(args.iter().cloned());
    Some(("/usr/bin/sandbox-exec".to_string(), wrapped))
}

pub struct MacosSandbox;

impl Sandbox for MacosSandbox {
    fn wrap_command(&self, program: &str, args: &[String], _raw_cmd: &str) -> Command {
        match wrap(program, args) {
            Some((wrapped_program, wrapped_args)) => {
                let mut cmd = Command::new(wrapped_program);
                cmd.args(wrapped_args);
                cmd
            }
            None => {
                // Sandbox unavailable — spawn unwrapped.
                let mut cmd = Command::new(program);
                cmd.args(args);
                cmd
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each test acquires the shared env-lock guard so parallel test
    // runners can't race on `SPECTYN_SANDBOX`. See `crate::env_lock`
    // in `lib.rs` for the rationale.

    #[test]
    fn wrap_returns_none_when_disabled() {
        let _g = crate::env_lock::acquire();
        std::env::set_var("SPECTYN_SANDBOX", "0");
        let r = wrap("/bin/echo", &["hi".to_string()]);
        std::env::remove_var("SPECTYN_SANDBOX");
        assert!(r.is_none());
    }

    #[test]
    fn wrap_inserts_sandbox_exec_with_profile() {
        let _g = crate::env_lock::acquire();
        std::env::remove_var("SPECTYN_SANDBOX");
        let (p, a) = wrap("/bin/echo", &["hi".to_string()])
            .expect("sandbox should wrap by default on macOS");
        assert_eq!(p, "/usr/bin/sandbox-exec");
        assert!(a.contains(&"-p".to_string()));
        assert!(a.iter().any(|x| x.contains("(version 1)")));
        assert!(a.iter().any(|x| x.starts_with("CWD=")));
        assert!(a.iter().any(|x| x.starts_with("HOME=")));
        assert_eq!(a.last().unwrap(), "hi");
    }

    /// MAC P0 (V5) — real integration test: spawn sandbox-exec with
    /// `SBPL_PROFILE_RELAXED` and confirm that a write to a path
    /// *outside* every `(subpath ...)` allowlist is rejected by the
    /// kernel. /Library/Caches is mode 1777 (world-writable + sticky)
    /// so a regular user CAN normally touch a file there — sandbox is
    /// the only thing that should stop it. Cleanup removes the file
    /// if it somehow leaked through (would indicate a sandbox bug).
    #[tokio::test]
    async fn seatbelt_denies_unallowed_path() {
        // Skip if sandbox-exec is missing (Apple may eventually remove
        // it — see module-level "deprecated" caveat).
        if !std::path::Path::new("/usr/bin/sandbox-exec").exists() {
            eprintln!("sandbox-exec missing — skipping; macOS removed it?");
            return;
        }

        let pid = std::process::id();
        let denied_target = format!("/Library/Caches/spectyn-tdd-seatbelt-deny-{}.tmp", pid);

        let cwd = std::env::current_dir()
            .ok()
            .and_then(|p| std::fs::canonicalize(p).ok())
            .and_then(|p| p.to_str().map(String::from))
            .unwrap_or_else(|| "/private/tmp".into());
        let home = std::env::var("HOME").unwrap_or_else(|_| "/private/tmp".into());
        let canon_home = std::fs::canonicalize(&home)
            .ok()
            .and_then(|p| p.to_str().map(String::from))
            .unwrap_or(home);

        let status = Command::new("/usr/bin/sandbox-exec")
            .arg("-D")
            .arg(format!("CWD={}", cwd))
            .arg("-D")
            .arg(format!("HOME={}", canon_home))
            .arg("-p")
            .arg(SBPL_PROFILE_RELAXED)
            .arg("/bin/sh")
            .arg("-c")
            .arg(format!("touch {}", denied_target))
            .status()
            .await
            .expect("sandbox-exec must spawn");

        // Clean up if leaked (should not happen — assertion below
        // is what gates the test).
        let _ = std::fs::remove_file(&denied_target);

        assert!(
            !status.success(),
            "SBPL profile allowed write to {} (exit {:?}); the relaxed \
             profile lists writable subpaths explicitly (cwd, home, /tmp, \
             /private/tmp, /private/var/folders, /dev/*) and \
             /Library/Caches is not among them — Seatbelt should deny.",
            denied_target,
            status
        );
    }

    #[test]
    fn strict_mode_drops_home_from_profile() {
        let _g = crate::env_lock::acquire();
        std::env::set_var("SPECTYN_SANDBOX", "strict");
        let (_p, a) = wrap("/bin/echo", &[]).expect("strict mode wraps");
        std::env::remove_var("SPECTYN_SANDBOX");
        let profile = a.iter().find(|x| x.contains("(version 1)")).unwrap();
        assert!(
            !profile.contains("(subpath (param \"HOME\"))"),
            "strict mode must not allow $HOME writes"
        );
    }
}
