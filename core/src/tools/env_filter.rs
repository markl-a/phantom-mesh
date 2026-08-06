//! Environment-variable injection guard for tool-spawned subprocesses.
//!
//! Background (audit C-2, 2026-05-15 tools-audit):
//!
//! Several tools (`shell`, `bash_run_background`, etc.) accept an
//! `env: { K: V, … }` parameter that is merged unfiltered into the child
//! process environment. A model that has any file-write tool can stage a
//! malicious shared library in `/tmp` and then call `shell` with
//! `{"command": "ls", "env": {"LD_PRELOAD": "/tmp/x.so"}}` → arbitrary
//! native code runs as the spectyn-mesh user.
//!
//! Mirror attacks exist on every supported platform:
//! - **Linux:** `LD_PRELOAD`, `LD_LIBRARY_PATH`, `LD_AUDIT`.
//! - **macOS:** `DYLD_INSERT_LIBRARIES`, `DYLD_LIBRARY_PATH`,
//!   `DYLD_FRAMEWORK_PATH` (effective when SIP-disabled binaries are
//!   invoked, which `sandbox-exec` does NOT block from env-overrides).
//! - **Windows:** prepending a malicious binary directory to `PATH`
//!   intercepts later calls to `cmd.exe` / `git.exe`.
//! - **Cross-language tooling:** `GIT_SSH_COMMAND`, `GIT_EXTERNAL_DIFF`,
//!   `EDITOR`, `PAGER`, `NODE_OPTIONS`, `PYTHONPATH`, `PYTHONSTARTUP`,
//!   `PERL5OPT`, `RUBYOPT` all hand the model a code-execution primitive
//!   the next time the corresponding interpreter starts.
//!
//! Approval gates are NOT a substitute — the model already knows about
//! `SPECTYN_AUTO_APPROVE` from its training corpus, and the `env` arg is
//! frequently set on perfectly legitimate calls (`PYTHONUNBUFFERED=1`,
//! `RUST_LOG=debug`, etc.) so a blanket prompt would be useless noise.
//!
//! Strategy: a small **deny-list** of names + name-prefixes that are
//! known process-takeover vectors. Anything else is allowed through. The
//! deny-list is intentionally short and conservative — the goal is to
//! cut the high-impact known vectors, not to lock down every var the
//! agent might want to set. (Future hardening: switch to an allow-list
//! once we have a survey of which env vars real-world tasks need.)

use std::collections::HashMap;

/// Exact-match denied env var names. Setting any of these via a tool
/// `env` parameter is rejected outright.
const DENY_EXACT: &[&str] = &[
    // PATH manipulation: prepending a writable dir intercepts every
    // subsequent program lookup. The agent NEVER needs to override this
    // for legitimate work — if they do, they can use a wrapper script
    // installed in the workspace.
    "PATH",
    "Path",    // Windows PowerShell sometimes uses this casing
    "PATHEXT", // Windows: extends what counts as an executable
    // HOME / USER hijacking — points config-file lookups at attacker
    // dirs (e.g. `~/.gitconfig` with `core.sshCommand = …`).
    "HOME",
    "USER",
    "USERNAME",
    "USERPROFILE",
    "LOGNAME",
    // Git remote-side code-exec vectors. `GIT_SSH_COMMAND` is the
    // canonical "run my binary instead of ssh" knob; all the others
    // are documented in `git-config(1)` as attack surface.
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "GIT_SSH_VARIANT",
    "GIT_EXTERNAL_DIFF",
    "GIT_PAGER",
    "GIT_EDITOR",
    "GIT_HOOKS_PATH",
    "GIT_TEMPLATE_DIR",
    "GIT_PROXY_COMMAND",
    "GIT_TERMINAL_PROMPT",
    "GIT_CONFIG",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_SYSTEM",
    // Generic editor/pager hijacks. Most tools will spawn `$EDITOR`
    // for interactive flows (e.g. `git commit` without `-m`).
    "EDITOR",
    "VISUAL",
    "PAGER",
    // Node / npm / Python / Perl / Ruby loader hijacks.
    "NODE_OPTIONS",
    "NODE_PATH",
    "NPM_CONFIG_PREFIX",
    "NPM_CONFIG_USERCONFIG",
    "PYTHONPATH",
    "PYTHONSTARTUP",
    "PYTHONHOME",
    "PYTHONUSERBASE",
    "PERL5OPT",
    "PERL5LIB",
    "PERLIO_DEBUG",
    "RUBYOPT",
    "RUBYLIB",
    "IRBRC",
    // SSL / certificate redirect — point trust store at attacker CA,
    // then MITM every HTTPS request the spawned tool makes.
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    "NODE_EXTRA_CA_CERTS",
    // Java / .NET loader hijacks (less common in our workload but
    // cheap to block).
    "CLASSPATH",
    "JAVA_TOOL_OPTIONS",
    "_JAVA_OPTIONS",
    "JDK_JAVA_OPTIONS",
];

/// Prefix-matched denied env var names. Any key beginning with one of
/// these (case-sensitive) is rejected. Covers the dynamic-loader knobs
/// that come in dozens of variants (`LD_PRELOAD`, `LD_LIBRARY_PATH`,
/// `LD_AUDIT`, `DYLD_INSERT_LIBRARIES`, `DYLD_LIBRARY_PATH`, …).
const DENY_PREFIX: &[&str] = &[
    "LD_",   // Linux glibc loader
    "DYLD_", // macOS dyld loader
    "XDG_",  // freedesktop config-dir redirect (steals app config)
    "GIO_",  // glib I/O hooks (debug-trap any GIO-backed tool)
    "GTK_",  // GTK module load hook (`GTK_MODULES`)
    "QT_",   // Qt plugin / library path hooks
    "GST_",  // GStreamer plugin path hook
    "VST_",  // VST plugin path
    "MOZ_",  // Mozilla product loader hooks
];

/// Validate a user-supplied `env` map intended for a child process. On
/// success returns the map unchanged. On rejection returns a string
/// describing which key was blocked and why — pass it back to the
/// model as the tool's error so the next call can try without the bad
/// key (or the human can see the audit trail).
///
/// The check is **case-sensitive** because every denied vector above
/// is itself case-sensitive on its target OS — e.g. on Linux,
/// `ld_preload` literally has no effect, only `LD_PRELOAD` does. The
/// one exception is Windows `Path` (lowercase), which we match
/// explicitly above.
pub fn validate_extra_env(env: &HashMap<String, String>) -> Result<(), String> {
    for key in env.keys() {
        if let Some(reason) = check_key(key) {
            return Err(format!(
                "blocked env var '{}': {}. \
                 The `env` parameter cannot set process-loader, \
                 SSL trust, PATH, HOME, or interpreter-hook variables \
                 — these are code-execution vectors. Drop the key and retry.",
                key, reason
            ));
        }
    }
    Ok(())
}

/// Returns `Some(reason)` if `key` matches the deny-list, `None` if
/// allowed. Public for use in unit tests of dependent crates.
pub fn check_key(key: &str) -> Option<&'static str> {
    for denied in DENY_EXACT {
        if key == *denied {
            return Some("exact-match deny-list");
        }
    }
    for prefix in DENY_PREFIX {
        if key.starts_with(prefix) {
            return Some("dynamic-loader / plugin-path prefix");
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn one(k: &str, v: &str) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert(k.into(), v.into());
        m
    }

    #[test]
    fn blocks_ld_preload_linux_loader_hijack() {
        let err = validate_extra_env(&one("LD_PRELOAD", "/tmp/x.so")).unwrap_err();
        assert!(err.contains("LD_PRELOAD"), "got: {}", err);
        assert!(err.contains("loader"), "got: {}", err);
    }

    #[test]
    fn blocks_ld_library_path_and_ld_audit() {
        assert!(validate_extra_env(&one("LD_LIBRARY_PATH", "/tmp")).is_err());
        assert!(validate_extra_env(&one("LD_AUDIT", "/tmp/a.so")).is_err());
    }

    #[test]
    fn blocks_dyld_insert_libraries_macos_loader_hijack() {
        let err = validate_extra_env(&one("DYLD_INSERT_LIBRARIES", "/tmp/x.dylib")).unwrap_err();
        assert!(err.contains("DYLD_INSERT_LIBRARIES"), "got: {}", err);
    }

    #[test]
    fn blocks_dyld_library_path_and_framework_path() {
        assert!(validate_extra_env(&one("DYLD_LIBRARY_PATH", "/tmp")).is_err());
        assert!(validate_extra_env(&one("DYLD_FRAMEWORK_PATH", "/tmp")).is_err());
    }

    #[test]
    fn blocks_path_override_windows_and_unix() {
        assert!(validate_extra_env(&one("PATH", "/tmp/evil:/usr/bin")).is_err());
        // Windows PowerShell sometimes uses lowercase `Path`.
        assert!(validate_extra_env(&one("Path", "C:\\evil;C:\\Windows")).is_err());
        assert!(validate_extra_env(&one("PATHEXT", ".COM;.EXE;.BAT;.evil")).is_err());
    }

    #[test]
    fn blocks_home_and_user_hijack() {
        assert!(validate_extra_env(&one("HOME", "/tmp/fake-home")).is_err());
        assert!(validate_extra_env(&one("USERPROFILE", "C:\\fake")).is_err());
        assert!(validate_extra_env(&one("USER", "root")).is_err());
    }

    #[test]
    fn blocks_git_ssh_command_remote_exec_vector() {
        let err = validate_extra_env(&one("GIT_SSH_COMMAND", "/tmp/evil-ssh")).unwrap_err();
        assert!(err.contains("GIT_SSH_COMMAND"), "got: {}", err);
        assert!(validate_extra_env(&one("GIT_EXTERNAL_DIFF", "/tmp/evil")).is_err());
        assert!(validate_extra_env(&one("GIT_HOOKS_PATH", "/tmp/hooks")).is_err());
    }

    #[test]
    fn blocks_node_python_perl_ruby_loader_hijacks() {
        assert!(validate_extra_env(&one("NODE_OPTIONS", "--require=/tmp/evil.js")).is_err());
        assert!(validate_extra_env(&one("PYTHONSTARTUP", "/tmp/evil.py")).is_err());
        assert!(validate_extra_env(&one("PYTHONPATH", "/tmp")).is_err());
        assert!(validate_extra_env(&one("PERL5OPT", "-MEvil")).is_err());
        assert!(validate_extra_env(&one("RUBYOPT", "-rEvil")).is_err());
    }

    #[test]
    fn blocks_ssl_cert_redirect_mitm_vector() {
        assert!(validate_extra_env(&one("SSL_CERT_FILE", "/tmp/attacker.pem")).is_err());
        assert!(validate_extra_env(&one("REQUESTS_CA_BUNDLE", "/tmp/x.pem")).is_err());
        assert!(validate_extra_env(&one("CURL_CA_BUNDLE", "/tmp/x.pem")).is_err());
        assert!(validate_extra_env(&one("NODE_EXTRA_CA_CERTS", "/tmp/x.pem")).is_err());
    }

    #[test]
    fn blocks_xdg_config_redirect() {
        // `XDG_CONFIG_HOME` is the standard "redirect every Linux app's
        // config dir" knob. Setting it to a writable temp dir means
        // every subsequent tool reads attacker-controlled config.
        assert!(validate_extra_env(&one("XDG_CONFIG_HOME", "/tmp/evil")).is_err());
        assert!(validate_extra_env(&one("XDG_DATA_DIRS", "/tmp/evil")).is_err());
    }

    #[test]
    fn blocks_editor_and_pager_hijack() {
        // `git commit` without `-m` spawns `$EDITOR`. `man` and many
        // CLIs spawn `$PAGER`. Either is a code-exec vector.
        assert!(validate_extra_env(&one("EDITOR", "/tmp/evil")).is_err());
        assert!(validate_extra_env(&one("VISUAL", "/tmp/evil")).is_err());
        assert!(validate_extra_env(&one("PAGER", "/tmp/evil")).is_err());
    }

    #[test]
    fn blocks_java_classpath_and_tool_options() {
        assert!(validate_extra_env(&one("CLASSPATH", "/tmp/evil.jar")).is_err());
        assert!(validate_extra_env(&one("JAVA_TOOL_OPTIONS", "-javaagent:/tmp/evil.jar")).is_err());
    }

    #[test]
    fn allows_legitimate_app_config_vars() {
        // These are the kinds of vars real tasks set — must NOT be
        // blocked or the agent loses its day-to-day workflow.
        assert!(validate_extra_env(&one("PYTHONUNBUFFERED", "1")).is_ok());
        assert!(validate_extra_env(&one("RUST_LOG", "debug")).is_ok());
        assert!(validate_extra_env(&one("RUST_BACKTRACE", "1")).is_ok());
        assert!(validate_extra_env(&one("CARGO_TARGET_DIR", "/tmp/build")).is_ok());
        assert!(validate_extra_env(&one("NO_COLOR", "1")).is_ok());
        assert!(validate_extra_env(&one("FORCE_COLOR", "1")).is_ok());
        assert!(validate_extra_env(&one("CI", "true")).is_ok());
        assert!(validate_extra_env(&one("DEBUG", "1")).is_ok());
        assert!(validate_extra_env(&one("SPECTYN_TEST_VAR", "hello")).is_ok());
        assert!(validate_extra_env(&one("MY_API_KEY", "secret")).is_ok());
    }

    #[test]
    fn rejects_only_the_first_bad_key_but_message_names_it() {
        let mut m = HashMap::new();
        m.insert("RUST_LOG".into(), "debug".into());
        m.insert("LD_PRELOAD".into(), "/tmp/x.so".into());
        let err = validate_extra_env(&m).unwrap_err();
        // Either order is fine, but it MUST cite the bad key.
        assert!(err.contains("LD_PRELOAD"), "got: {}", err);
    }

    #[test]
    fn empty_env_is_allowed() {
        let m: HashMap<String, String> = HashMap::new();
        assert!(validate_extra_env(&m).is_ok());
    }

    #[test]
    fn check_key_returns_none_for_safe_var() {
        assert!(check_key("RUST_LOG").is_none());
        assert!(check_key("PYTHONUNBUFFERED").is_none());
    }

    #[test]
    fn check_key_is_case_sensitive_for_loader_prefixes() {
        // `ld_preload` (lowercase) literally has no effect on glibc —
        // blocking it would just produce confusing false positives in
        // tests / scripts that happen to pick that name. Only the
        // canonical uppercase form is dangerous.
        assert!(check_key("ld_preload").is_none());
        assert!(check_key("dyld_insert_libraries").is_none());
        // The uppercase forms ARE blocked (regression guard).
        assert!(check_key("LD_PRELOAD").is_some());
        assert!(check_key("DYLD_INSERT_LIBRARIES").is_some());
    }
}
