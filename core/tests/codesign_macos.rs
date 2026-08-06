//! Mac-side codesign / Gatekeeper / notarytool integration tests.
//!
//! These exercise the *toolchain*, not actual distribution signing —
//! they verify that the macOS shipping pipeline spectyn relies on
//! (`codesign`, `xattr`, `xcrun notarytool`) is reachable and behaves
//! as documented. Real signed releases happen through CI with a stored
//! Developer ID cert; that's out of scope here.

#![cfg(target_os = "macos")]

use std::process::Command;

fn tmp_path(suffix: &str) -> String {
    format!(
        "/tmp/spectyn-codesign-test-{}-{}",
        std::process::id(),
        suffix
    )
}

/// MAC P0 — `codesign -s -` (ad-hoc signing) must succeed on a regular
/// Mach-O binary and produce a verifiable signature. Ad-hoc signing
/// requires no Developer ID cert; it's the form spectyn emits on
/// `cargo install` so the binary will at least run on the developer's
/// own Mac without Gatekeeper triggering on every launch.
#[test]
fn codesign_self_sign_succeeds() {
    let target = tmp_path("self-sign");
    // /bin/echo is universal2 Mach-O on every macOS — perfect fixture.
    std::fs::copy("/bin/echo", &target).expect("copy /bin/echo");
    // Strip any inherited signature first (codesign -f re-signs in place).
    let sign = Command::new("codesign")
        .args(["-s", "-", "-f", &target])
        .output()
        .expect("codesign must spawn");
    assert!(
        sign.status.success(),
        "codesign -s - -f {} failed: {}",
        target,
        String::from_utf8_lossy(&sign.stderr)
    );

    // Verify the freshly-applied signature.
    let verify = Command::new("codesign")
        .args(["-v", "--verbose=2", &target])
        .output()
        .expect("codesign -v must spawn");
    let stderr = String::from_utf8_lossy(&verify.stderr);
    assert!(
        verify.status.success(),
        "codesign -v {} failed: {}",
        target,
        stderr
    );
    // The verbose output should mention "satisfies its Designated Requirement"
    // OR at minimum identify the signer (ad-hoc → "Signature=adhoc").
    assert!(
        stderr.contains("adhoc") || stderr.contains("valid on disk"),
        "codesign -v stderr unexpected — did the ad-hoc sign actually happen?\n{}",
        stderr
    );

    let _ = std::fs::remove_file(&target);
}

/// MAC P0 — The Gatekeeper quarantine xattr that browser/curl downloads
/// attach to files must be removable via `xattr -d com.apple.quarantine`.
/// spectyn's `self-update` flow relies on this to avoid the "downloaded
/// from the internet" pop-up after pulling a fresh binary.
#[test]
fn gatekeeper_quarantine_strip_via_xattr() {
    let target = tmp_path("quarantine");
    std::fs::write(&target, "test content").expect("write fixture");

    // Set the quarantine attr the way Safari/curl/AirDrop would.
    // Format: <flag-hex>;<unix-time-hex>;<agent>;<uuid> — flag 0083 is
    // "downloaded from internet, not yet approved by user".
    let set = Command::new("xattr")
        .args([
            "-w",
            "com.apple.quarantine",
            "0083;00000000;spectyn-tdd;",
            &target,
        ])
        .output()
        .expect("xattr -w must spawn");
    assert!(
        set.status.success(),
        "xattr -w (set quarantine) failed: {}",
        String::from_utf8_lossy(&set.stderr)
    );

    // Confirm it's actually there.
    let probe_before = Command::new("xattr")
        .args(["-p", "com.apple.quarantine", &target])
        .output()
        .expect("xattr -p must spawn");
    assert!(
        probe_before.status.success(),
        "quarantine attr not visible after set: {}",
        String::from_utf8_lossy(&probe_before.stderr)
    );

    // Strip it.
    let strip = Command::new("xattr")
        .args(["-d", "com.apple.quarantine", &target])
        .output()
        .expect("xattr -d must spawn");
    assert!(
        strip.status.success(),
        "xattr -d (strip quarantine) failed: {}",
        String::from_utf8_lossy(&strip.stderr)
    );

    // Confirm it's gone — `xattr -p` of a missing attr returns non-zero
    // with "No such xattr" on stderr.
    let probe_after = Command::new("xattr")
        .args(["-p", "com.apple.quarantine", &target])
        .output()
        .expect("xattr -p must spawn");
    assert!(
        !probe_after.status.success(),
        "quarantine attr survived strip — xattr -p still returned 0. \
         stdout: {}",
        String::from_utf8_lossy(&probe_after.stdout)
    );

    let _ = std::fs::remove_file(&target);
}

/// MAC P0 — `xcrun notarytool` must be available + understand the
/// canonical `submit` / `history` / `info` subcommands. We don't
/// actually submit anything here (that costs real notarization-queue
/// time + needs valid app-specific password), but we do verify the
/// toolchain is reachable.
///
/// If the operator has stored notarytool credentials via
/// `xcrun notarytool store-credentials <profile>`, set
/// `SPECTYN_NOTARY_PROFILE=<profile>` and this test will also run
/// `notarytool history` (round-trips to Apple's notarization service,
/// validates the cred pipeline) — the stronger form of the test.
#[test]
fn notarytool_submit_dryrun_validates() {
    let help = Command::new("xcrun")
        .args(["notarytool", "--help"])
        .output()
        .expect("xcrun notarytool must spawn");
    assert!(
        help.status.success(),
        "xcrun notarytool --help failed — Xcode not installed or \
         notarytool missing?\nstderr: {}",
        String::from_utf8_lossy(&help.stderr)
    );

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&help.stdout),
        String::from_utf8_lossy(&help.stderr)
    );
    for required in &["submit", "history", "info", "store-credentials"] {
        assert!(
            combined.contains(required),
            "notarytool --help missing expected subcommand `{}`. \
             Xcode CLT version mismatch?\nfull help (truncated):\n{}",
            required,
            combined.chars().take(800).collect::<String>()
        );
    }

    // Stronger variant: if a keychain profile is set up, hit Apple.
    if let Ok(profile) = std::env::var("SPECTYN_NOTARY_PROFILE") {
        let history = Command::new("xcrun")
            .args(["notarytool", "history", "--keychain-profile", &profile])
            .output()
            .expect("xcrun notarytool history must spawn");
        assert!(
            history.status.success(),
            "notarytool history with profile `{}` failed (does the \
             profile exist, are creds still valid?): {}",
            profile,
            String::from_utf8_lossy(&history.stderr)
        );
    } else {
        eprintln!(
            "SKIPPED: notarytool_submit_dryrun_validates (history round-trip only) — \
             SPECTYN_NOTARY_PROFILE unset. Set via `xcrun notarytool \
             store-credentials <name>` then export SPECTYN_NOTARY_PROFILE=<name> for \
             the stronger variant of this test."
        );
    }
}
