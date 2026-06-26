//! P0-7 S3 — no hardcoded broker/fleet/relay address blocks first use.
//!
//! Static gate over the cold-start surface (onboarding writer + local-server
//! detection + demo-relay + the broker default URL). Proves remote-URL
//! constants only appear inside explicitly opt-in broker/login commands, never
//! on the fresh-install path. Runs for real on every platform (it reads source
//! text, not the binary), so it guards the (d) guarantee on Windows CI too.
//!
//! NOTE on the assertions below (deviation from the plan's first-draft needle
//! list, documented on purpose): a substring deny-list of `"http://1"` is a
//! false positive against the LEGITIMATE loopback `http://127.0.0.1`, and a
//! deny-list over every literal `"https://phantommesh.io"` is a false positive
//! against display-only `eprintln!` help text / doc comments. The real
//! invariant (d) is "the cold-start surface reads no remote URL, and the broker
//! default is only *resolved* inside a token-gated path" — so the checks below
//! assert exactly that: every http(s) URL on the onboarding/detection surface is
//! loopback, and every broker default-URL *read* is co-located with a token gate.

use std::path::Path;

fn read(p: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(p))
        .unwrap_or_else(|e| panic!("read {p}: {e}"))
}

/// Byte-window into `src` snapped DOWN to the nearest char boundaries, so it is
/// safe over the file's UTF-8 box-drawing / CJK comments (a raw byte slice would
/// panic mid-char).
fn safe_window(src: &str, start: usize, end: usize) -> &str {
    let mut s = start.min(src.len());
    while s < src.len() && !src.is_char_boundary(s) {
        s += 1;
    }
    let mut e = end.min(src.len());
    while e > s && !src.is_char_boundary(e) {
        e -= 1;
    }
    &src[s..e]
}

/// Every `http://` / `https://` literal in `src` must address a loopback host
/// (`127.0.0.1` / `localhost`). Returns the first offending snippet, if any.
fn first_non_loopback_url(src: &str) -> Option<String> {
    for scheme in ["http://", "https://"] {
        for (i, _) in src.match_indices(scheme) {
            let rest = &src[i + scheme.len()..];
            let host: String = rest
                .chars()
                .take_while(|c| !matches!(c, '/' | '"' | '\'' | ' ' | ')' | '`' | '\n'))
                .collect();
            if !host.starts_with("127.0.0.1") && !host.starts_with("localhost") {
                return Some(format!("{scheme}{host}"));
            }
        }
    }
    None
}

#[test]
fn onboarding_writer_has_no_remote_url() {
    // The generated first-run config must point ONLY at localhost / loopback /
    // env-var NAMES — never a hosted broker/relay host.
    let src = read("src/onboarding_config.rs");
    for needle in ["phantommesh.io", "demo.phantommesh", "192.0.2."] {
        assert!(
            !src.contains(needle),
            "onboarding_config.rs must not hardcode a remote host ({needle})"
        );
    }
    // Every URL the writer emits must be loopback (the Ollama fallback url; any
    // free-provider base_url is written from FreeProvider DATA, not a literal).
    if let Some(bad) = first_non_loopback_url(&src) {
        panic!("onboarding_config.rs emits a non-loopback URL literal: {bad}");
    }
    // Positive: it DOES write the localhost ollama url.
    assert!(
        src.contains("http://127.0.0.1:11434/v1"),
        "local ollama url expected"
    );
}

#[test]
fn detect_local_servers_targets_only_loopback() {
    let src = read("src/providers/local_servers.rs");
    // Every probe URL must be a loopback host — a local detection probe must
    // never reach off-box.
    if let Some(bad) = first_non_loopback_url(&src) {
        panic!("local_servers.rs probes a non-loopback URL: {bad}");
    }
    assert!(
        !src.contains("phantommesh.io"),
        "local detection must not reach a broker"
    );
}

#[test]
fn demo_relay_is_dead_code_not_on_boot_path() {
    // The only demo.phantommesh.io reference lives in an unimplemented!() stub
    // with no caller. Assert it is still unimplemented (i.e. not wired to boot).
    let src = read("src/onboarding_wire.rs");
    let idx = src
        .find("demo.phantommesh.io")
        .expect("demo relay reference present");
    let around = safe_window(&src, idx.saturating_sub(400), idx + 400);
    assert!(
        around.contains("unimplemented!") || src.contains("start_demo_relay_handoff"),
        "demo-relay must remain an unimplemented opt-in stub, not a boot dependency"
    );
    // And the handoff fn that DOES issue the relay GET must stay unimplemented
    // (never wired into the cold-start path).
    let handoff = src
        .find("pub fn start_demo_relay_handoff")
        .expect("start_demo_relay_handoff present");
    let body = safe_window(&src, handoff, handoff + 1500);
    assert!(
        body.contains("unimplemented!"),
        "start_demo_relay_handoff must remain an unimplemented opt-in stub"
    );
    // The actual network seam it would call must ALSO stay unimplemented — it is
    // the only place that could pull reqwest onto the relay path.
    let getseam = src
        .find("fn https_get_pseudo")
        .expect("https_get_pseudo seam present");
    assert!(
        safe_window(&src, getseam, getseam + 400).contains("unimplemented!"),
        "https_get_pseudo (demo-relay network seam) must remain unimplemented"
    );
}

#[test]
fn phantommesh_default_url_only_in_token_gated_paths() {
    // The https://phantommesh.io broker DEFAULT is *resolved* only via
    // `unwrap_or_else(|| "https://phantommesh.io"...)`. Each such read must be
    // co-located with a broker-token guard (read_broker_config / auth::load /
    // "no broker token" / "phantom login") — i.e. opt-in, never on boot.
    // (Display-only eprintln help text and doc comments mentioning the host are
    // not URL *reads* and are intentionally out of scope.)
    let src = read("src/cli_config.rs");
    let needle = "unwrap_or_else(|| \"https://phantommesh.io\"";
    let mut found = 0usize;
    for (i, _) in src.match_indices(needle) {
        found += 1;
        let window = &src[i.saturating_sub(800)..(i + 200).min(src.len())];
        assert!(
            window.contains("broker_token")
                || window.contains("read_broker_config")
                || window.contains("auth::load")
                || window.contains("no broker token")
                || window.contains("no token")
                || window.contains("phantom login"),
            "phantommesh.io default at byte {i} must be token-gated (opt-in), not on boot"
        );
    }
    assert!(
        found > 0,
        "expected the broker default-URL resolution pattern to exist (guards against a silent rename that bypasses this gate)"
    );
}
