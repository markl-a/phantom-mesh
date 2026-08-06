// MAC-CUJ01-INST-004: install.sh HTTPS-only enforcement (F-CRIT-3 security invariant).
//
// scripts/install.sh has a require_https() guard that refuses any non-HTTPS
// SPECTYN_INSTALL_BASE. This hermetic test shells out to `sh scripts/install.sh`
// with an http:// base URL and asserts the installer aborts (exit != 0) with an
// error mentioning HTTPS. This is a security invariant: an attacker must not be
// able to point the installer at a plaintext URL for MITM during download.

// scripts/install.sh is a POSIX shell script; this test shells out to `sh`.
// On Windows `sh` is usually absent, so Command::new("sh").output().expect(...)
// below would PANIC (a hard FAIL, not a skip). Gate the whole file to Unix.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn install_sh_rejects_non_https_base_url() {
    let script = repo_root().join("scripts").join("install.sh");
    if !script.exists() {
        eprintln!(
            "SKIPPED: install_sh_rejects_non_https_base_url — install.sh not found at {:?}",
            script
        );
        return;
    }

    // Isolate HOME to a unique temp dir so the installer cannot touch the real
    // user environment even if the guard were to fail.
    let home = std::env::temp_dir().join(format!(
        "spectyn-install-https-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&home).expect("create isolated HOME");

    let output = Command::new("sh")
        .arg(&script)
        .env("HOME", &home)
        .env("SPECTYN_INSTALL_BASE", "http://insecure.example")
        .output()
        .expect("failed to invoke sh scripts/install.sh");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // F-CRIT-3: must abort with a non-zero exit code.
    assert!(
        !output.status.success(),
        "installer accepted a non-HTTPS base URL (exit ok). stdout={:?} stderr={:?}",
        stdout,
        stderr
    );

    // The error must mention HTTPS so operators know why it refused.
    assert!(
        stderr.to_lowercase().contains("https"),
        "stderr did not mention HTTPS. stderr={:?} stdout={:?}",
        stderr,
        stdout
    );

    // Sanity: the insecure scheme should not have leaked into a success path.
    assert!(
        !stdout.contains("installer: base=http://"),
        "installer proceeded with an insecure base. stdout={:?}",
        stdout
    );

    let _ = std::fs::remove_dir_all(&home);
}
