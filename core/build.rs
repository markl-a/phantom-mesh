// Inject git commit + build date into the binary as compile-time env vars
// so `phantom --version` can show provenance, not just "0.1.0".

use std::process::Command;

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn main() {
    let git_hash =
        run("git", &["rev-parse", "--short=10", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let git_dirty = run("git", &["status", "--porcelain"])
        .map(|s| if s.is_empty() { "" } else { "+" })
        .unwrap_or("");

    // Use UTC for reproducibility; fall back to local if `date -u` flag isn't
    // accepted (rare).
    let build_date = run("date", &["-u", "+%Y-%m-%d"])
        .or_else(|| run("date", &["+%Y-%m-%d"]))
        .unwrap_or_else(|| "unknown".into());

    println!("cargo:rustc-env=PHANTOM_GIT_HASH={}{}", git_hash, git_dirty);
    println!("cargo:rustc-env=PHANTOM_BUILD_DATE={}", build_date);

    // Re-run when HEAD moves (covers commits + checkouts).
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}
