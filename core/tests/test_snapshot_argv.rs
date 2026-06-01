//! T7 codex audit (2026-05-15): snapshot::apply must NOT interpolate
//! caller paths into a `sh -c` string. macOS-only because the entire
//! module is `#[cfg(target_os = "macos")]`.

#![cfg(target_os = "macos")]

// The test is a SOURCE-LEVEL check (cheaper than spinning up a real
// snapshot, which would require sudo). We assert the file does not
// contain the dangerous pattern. The point is to catch regressions
// where someone re-introduces sh -c shelling out.

#[test]
fn snapshot_apply_does_not_use_sh_c_for_user_paths() {
    let src = include_str!("../src/snapshot.rs");
    // Find the apply function body (between `pub async fn apply` and
    // the next top-level `pub fn` / `pub async fn` / end of file).
    let start = src
        .find("pub async fn apply")
        .expect("snapshot::apply must exist");
    let after = &src[start..];
    let end = after
        .find("\npub fn ")
        .or_else(|| {
            after
                .find("\npub async fn ")
                .map(|p| if p == 0 { after.len() } else { p })
        })
        .unwrap_or(after.len());
    let body = &after[..end];

    assert!(
        !body.contains("Command::new(\"sh\")"),
        "snapshot::apply must not Command::new(\"sh\") — use argv-list `sudo` instead"
    );
    assert!(
        !body.contains("\"-c\""),
        "snapshot::apply must not pass `-c` to a shell"
    );
    // Positive: it should call sudo with argv args.
    assert!(
        body.contains("Command::new(\"sudo\")"),
        "snapshot::apply should drive each step through `sudo` with argv args"
    );
}
