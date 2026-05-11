use std::path::Path;

fn main() {
    // ── Copy phantom-mesh binary to binaries/ for sidecar bundling ──────────
    // Must run BEFORE tauri_build::build() which validates externalBin paths.
    // Tauri expects: binaries/phantom-mesh-{TARGET}[.exe]
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.is_empty() {
        let ext = if target.contains("windows") {
            ".exe"
        } else {
            ""
        };
        let sidecar_filename = format!("phantom-mesh-{}{}", target, ext);
        let binaries_dir = Path::new("binaries");
        let dest = binaries_dir.join(&sidecar_filename);

        if !dest.exists() {
            std::fs::create_dir_all(binaries_dir).ok();

            // Search for phantom-mesh binary in well-known locations
            let source_name = format!("phantom-mesh{}", ext);
            let candidates: Vec<std::path::PathBuf> = vec![
                // Standard cargo target dirs
                format!("../../core/target/release/{}", source_name).into(),
                format!("../../core/target/debug/{}", source_name).into(),
                // exFAT workaround: CARGO_TARGET_DIR=targetN
                format!("../../core/target1/release/{}", source_name).into(),
                format!("../../core/target1/debug/{}", source_name).into(),
                format!("../../core/target2/release/{}", source_name).into(),
                format!("../../core/target2/debug/{}", source_name).into(),
                // Tauri-specific target dir
                format!("../../../tauri-target/release/{}", source_name).into(),
                format!("../../../tauri-target/debug/{}", source_name).into(),
            ];

            let mut found = false;
            for candidate in &candidates {
                if candidate.exists() {
                    match std::fs::copy(candidate, &dest) {
                        Ok(_) => {
                            println!(
                                "cargo:warning=Copied phantom-mesh sidecar from {:?}",
                                candidate
                            );
                            found = true;
                            break;
                        }
                        Err(e) => {
                            println!(
                                "cargo:warning=Failed to copy {:?}: {}",
                                candidate, e
                            );
                        }
                    }
                }
            }

            if !found {
                // Create a minimal placeholder so tauri_build doesn't fail.
                // Dev mode uses daemon.rs find_binary() which resolves the real binary.
                // Production builds MUST replace this with the real binary.
                println!("cargo:warning=phantom-mesh sidecar binary not found!");
                println!("cargo:warning=Creating placeholder. For production: cargo build --release -p phantom-mesh");

                #[cfg(target_os = "windows")]
                {
                    // Create a tiny .exe placeholder (copy cmd.exe as stub — it won't be used in dev)
                    // Actually, just create an empty file so the build passes
                    std::fs::write(&dest, b"placeholder").ok();
                }
                #[cfg(not(target_os = "windows"))]
                {
                    std::fs::write(&dest, b"#!/bin/sh\necho 'placeholder'").ok();
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
                            .ok();
                    }
                }
            }
        }
    }

    // tauri::generate_context!() bakes frontendDist (../dist) into the
    // binary via include_dir! at compile time. cargo doesn't watch
    // frontendDist by default, so a frontend-only edit silently keeps the
    // stale HTML in the .app. Tell cargo to rerun whenever vite output
    // changes so the next build picks it up.
    println!("cargo:rerun-if-changed=../dist/index.html");
    println!("cargo:rerun-if-changed=../dist/assets");

    // ── Standard Tauri build (validates externalBin, generates code) ────────
    tauri_build::build();
}
