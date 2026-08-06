// F-CRIT-3 fail-CLOSED: install.sh must ABORT (never install) when the SHA256
// verification cannot succeed.
//
// scripts/install.sh downloads a binary, then verifies it against a `.sha256`
// sidecar via _verify-download.sh's verify_sha256() BEFORE chmod/move into PATH.
// Two ways verification must fail-closed:
//   1. tampered binary — the sidecar's hash does not match the binary => abort,
//      delete the temp binary, never install.
//   2. missing sidecar — the `.sha256` 404s/absent => abort, never install.
//
// In both cases the installer must exit non-zero, say something about sha /
// mismatch / verify, and the target binary at
// $HOME/.spectyn-mesh/bin/spectyn must NOT exist.
//
// This is the security invariant that stops a compromised mirror / MITM from
// swapping in a malicious spectyn binary.

// scripts/install.sh is a POSIX shell script; this test shells out to `sh` and
// spins up `python3 -m http.server`. On Windows both are usually absent, so the
// Command::new(...).expect(...) calls below would PANIC (a hard FAIL, not a
// skip). Gate the whole file to Unix.
#![cfg(unix)]

use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

// The R2 object name install.sh maps to on this host. Mirrors install.sh's
// detect_target() for linux/macos x86_64/aarch64.
fn r2_object() -> &'static str {
    // We only run the binary-serving cases on targets install.sh supports.
    // (Intel macs are unsupported by install.sh; CI runners are x86_64 linux.)
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "spectyn-linux-x86_64"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "spectyn-aarch64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "spectyn-aarch64-apple-darwin"
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "aarch64"),
    )))]
    {
        ""
    }
}

fn unique_dir(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "spectyn-install-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&p).expect("create unique temp dir");
    p
}

// Pick a free localhost port by binding then dropping the listener.
fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    l.local_addr().expect("local_addr").port()
}

struct HttpServer {
    child: Child,
    port: u16,
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// Serve `root` over plain HTTP on a free port using python3's http.server, then
// block until it answers a request (so the installer's curl doesn't race the
// server's startup).
fn serve_dir(root: &Path) -> HttpServer {
    let port = free_port();
    let child = Command::new("python3")
        .arg("-m")
        .arg("http.server")
        .arg(port.to_string())
        .arg("--bind")
        .arg("127.0.0.1")
        .current_dir(root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn python3 -m http.server");

    // Wait until the server accepts a TCP connection (up to ~10s).
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        if Instant::now() > deadline {
            panic!("python3 http.server on port {port} never came up");
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    HttpServer { child, port }
}

fn sha256_hex(bytes: &[u8]) -> String {
    // Avoid pulling a crypto crate into the test: shell out to sha256sum /
    // shasum, the same tools install.sh relies on.
    let mut child = if Command::new("sha256sum")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        Command::new("sha256sum")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn sha256sum")
    } else {
        Command::new("shasum")
            .arg("-a")
            .arg("256")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn shasum")
    };
    child
        .stdin
        .take()
        .unwrap()
        .write_all(bytes)
        .expect("write to sha tool stdin");
    let out = child.wait_with_output().expect("sha tool output");
    let line = String::from_utf8_lossy(&out.stdout);
    line.split_whitespace()
        .next()
        .expect("sha tool produced a hash")
        .to_lowercase()
}

struct RunResult {
    success: bool,
    code: Option<i32>,
    combined: String,
    target_bin: PathBuf,
}

// Drive install.sh against a local fixture dir served over http, under an
// isolated HOME. `write_sidecar` controls whether the .sha256 sidecar exists,
// and `sidecar_hash` what it contains when present.
fn run_installer(case_tag: &str, write_sidecar: bool, sidecar_hash: Option<&str>) -> RunResult {
    let script = repo_root().join("scripts").join("install.sh");
    assert!(
        script.exists(),
        "install.sh not found at {script:?} — cannot run fail-closed test"
    );

    let obj = r2_object();
    assert!(
        !obj.is_empty(),
        "unsupported target for install.sh fixture test"
    );

    // Fixture web root: <root>/dist/<obj>[.sha256]
    let webroot = unique_dir(&format!("{case_tag}-web"));
    let dist = webroot.join("dist");
    std::fs::create_dir_all(&dist).expect("create dist dir");

    // The "binary": arbitrary bytes. Its REAL sha will not match the sidecar in
    // the mismatch case.
    let bin_bytes = b"#!/bin/sh\necho fake-spectyn-binary\n";
    std::fs::write(dist.join(obj), bin_bytes).expect("write fake binary");

    if write_sidecar {
        let hash = sidecar_hash.expect("sidecar_hash required when write_sidecar");
        std::fs::write(
            dist.join(format!("{obj}.sha256")),
            format!("{hash}  {obj}\n"),
        )
        .expect("write sidecar");
    }

    let server = serve_dir(&webroot);
    let base = format!("http://127.0.0.1:{}", server.port);

    // Isolated HOME so a (bug-induced) install lands nowhere real.
    let home = unique_dir(&format!("{case_tag}-home"));
    let target_bin = home.join(".spectyn-mesh").join("bin").join("spectyn");

    let output = Command::new("sh")
        .arg(&script)
        .env("HOME", &home)
        .env("SPECTYN_INSTALL_BASE", &base)
        // require_https refuses http://; the installer is being pointed at a
        // local fixture, so opt out of the scheme check ONLY. This does NOT
        // disable SHA verification — that is exactly what we're testing.
        .env("SPECTYN_ALLOW_INSECURE", "1")
        // Make sure no ambient skip leaks in.
        .env("SPECTYN_SKIP_VERIFY", "0")
        .output()
        .expect("run install.sh");

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let combined = format!("STDOUT:\n{stdout}\nSTDERR:\n{stderr}");

    drop(server);

    let res = RunResult {
        success: output.status.success(),
        code: output.status.code(),
        combined,
        target_bin: target_bin.clone(),
    };

    // Best-effort cleanup of the web root; HOME is cleaned by the caller after
    // its assertions (so failure messages can still reference it).
    let _ = std::fs::remove_dir_all(&webroot);

    res
}

#[test]
fn install_sh_aborts_on_sha256_mismatch() {
    // Sidecar advertises a syntactically-valid but WRONG hash (all zeros'ish):
    // 64 hex chars that won't match the served binary.
    let wrong = "deadbeef".repeat(8); // 64 hex chars, != real sha
    let res = run_installer("mismatch", true, Some(&wrong));

    assert!(
        !res.success,
        "FAIL-OPEN: installer accepted a tampered binary (exit {:?}).\n{}",
        res.code, res.combined
    );

    let lower = res.combined.to_lowercase();
    assert!(
        lower.contains("mismatch") || lower.contains("sha"),
        "abort message should mention sha/mismatch.\n{}",
        res.combined
    );

    assert!(
        !res.target_bin.exists(),
        "FAIL-CLOSED VIOLATED: target binary was created at {:?} despite sha mismatch.\n{}",
        res.target_bin, res.combined
    );

    if let Some(home) = res.target_bin.ancestors().nth(3) {
        let _ = std::fs::remove_dir_all(home);
    }
}

#[test]
fn install_sh_aborts_when_sidecar_missing() {
    // No .sha256 sidecar at all — the fixture serves only the binary, so the
    // sidecar URL 404s.
    let res = run_installer("nosidecar", false, None);

    assert!(
        !res.success,
        "FAIL-OPEN: installer accepted a binary with no sha256 sidecar (exit {:?}).\n{}",
        res.code, res.combined
    );

    let lower = res.combined.to_lowercase();
    assert!(
        lower.contains("sha256") || lower.contains("sidecar") || lower.contains("unverified"),
        "abort message should explain the missing/unverifiable sidecar.\n{}",
        res.combined
    );

    assert!(
        !res.target_bin.exists(),
        "FAIL-CLOSED VIOLATED: target binary was created at {:?} despite a missing sidecar.\n{}",
        res.target_bin, res.combined
    );

    if let Some(home) = res.target_bin.ancestors().nth(3) {
        let _ = std::fs::remove_dir_all(home);
    }
}
