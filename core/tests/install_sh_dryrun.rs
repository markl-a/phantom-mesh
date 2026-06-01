// MAC-CUJ01-INST-007: install.sh dry-run must preview without writing anything.
//
// scripts/install.sh honours PHANTOM_INSTALL_DRY_RUN=1: it prints a "would
// install ... / no files written." preview and exits 0 *before* any download or
// file write. This hermetic test drives that path under an isolated HOME and
// asserts (a) exit 0, (b) the preview wording is present, and (c) no phantom
// binary was created anywhere under the isolated HOME. "Safe preview" is the
// invariant — a user inspecting the installer must be able to see what it would
// do with zero side effects.
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

// Recursively check that no regular file named "phantom" exists under `dir`.
fn has_phantom_binary(dir: &std::path::Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if has_phantom_binary(&p) {
                return true;
            }
        } else if p.file_name().and_then(|n| n.to_str()) == Some("phantom") {
            return true;
        }
    }
    false
}

#[test]
fn install_sh_dry_run_previews_without_writing() {
    let script = repo_root().join("scripts").join("install.sh");
    if !script.exists() {
        eprintln!("skip: install.sh not found at {script:?}");
        return;
    }

    let home = std::env::temp_dir().join(format!(
        "phantom-install-dryrun-{}-{}",
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
        .env("PHANTOM_INSTALL_DRY_RUN", "1")
        .output()
        .expect("run install.sh");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    // (a) must exit cleanly.
    assert!(
        output.status.success(),
        "dry-run must exit 0 but exited {:?}. output:\n{combined}",
        output.status.code()
    );

    // (b) must announce the preview and that nothing was written.
    let lower = combined.to_lowercase();
    assert!(
        lower.contains("would install"),
        "dry-run should say 'would install'. output:\n{combined}"
    );
    assert!(
        lower.contains("no files written"),
        "dry-run should say 'no files written'. output:\n{combined}"
    );

    // (c) no binary actually landed under the isolated HOME.
    assert!(
        !has_phantom_binary(&home),
        "dry-run must not write a phantom binary, but one appeared under {home:?}"
    );

    let _ = std::fs::remove_dir_all(&home);
}
