//! Desktop behavior capture — read the frontmost macOS app (life capability ① "sense").
//!
//! Produces ONE `focus` event describing the currently active application so
//! that `spectyn recall <appname> --kind focus` can surface it later.
//!
//! ## Why `/usr/bin/lsappinfo` and NOT native NSWorkspace / Accessibility
//!
//! `lsappinfo` reads the SAME LaunchServices frontmost-process state that
//! `NSWorkspace.frontmostApplication` reads, but with ZERO Accessibility /
//! AXUIElement use and therefore zero TCC (privacy) prompt for the user.
//! Going native (objc2-app-kit) would require a new crate dependency, which is
//! out of scope here (no `Cargo.toml` edits, and `core/` has no `cc` build-dep
//! for an objc shim). Reading the window TITLE *would* need Accessibility or
//! Screen-Recording permission, so we deliberately leave `window_title = None`:
//! the acceptance recalls BY APP NAME, so the title is not required.

/// The currently frontmost application as reported by LaunchServices.
pub struct FrontmostApp {
    pub name: String,
    pub bundle_id: Option<String>,
    pub window_title: Option<String>,
}

/// Parse a single `lsappinfo info -only <key>` value line.
///
/// Input looks like `"LSDisplayName"="Safari"`, possibly with trailing
/// whitespace/newline and surrounding spaces. Returns the contents of the
/// quoted segment AFTER the `=`. Returns `None` if the line is unparseable or
/// the value is empty.
pub fn parse_lsappinfo_value(raw: &str) -> Option<String> {
    // Find the `=` separating key from value, then take everything after it.
    let after_eq = raw.trim().splitn(2, '=').nth(1)?.trim();
    // The value is a quoted segment: take what is between the first and last
    // double-quote.
    let inner = after_eq.strip_prefix('"')?;
    let value = inner.strip_suffix('"')?;
    if value.is_empty() {
        return None;
    }
    Some(value.to_string())
}

/// Format the focus-event text for capture.
///
/// Pure: `"Active app: Safari (com.apple.Safari)"`. If `bundle_id` is `None`,
/// the parenthetical is omitted. If `window_title` is `Some`, it is appended
/// after an em-dash. This string becomes the capture `--text`, so recall-by-app
/// name matches against it.
pub fn focus_event_text(app: &FrontmostApp) -> String {
    let mut s = format!("Active app: {}", app.name);
    if let Some(b) = &app.bundle_id {
        s.push_str(&format!(" ({})", b));
    }
    if let Some(t) = &app.window_title {
        s.push_str(&format!(" — {}", t));
    }
    s
}

/// Read the current frontmost application via LaunchServices (`lsappinfo`).
///
/// macOS body is cfg-gated; on other platforms a stub returns an
/// `Unsupported` error so the `spectyn` bin still builds everywhere.
#[cfg(target_os = "macos")]
pub fn read_frontmost() -> std::io::Result<FrontmostApp> {
    use std::io::{Error, ErrorKind};
    use std::process::Command;

    // 1. `lsappinfo front` → ASN token on stdout, e.g. `ASN:0x0-0xc00c:`
    let front = Command::new("/usr/bin/lsappinfo").arg("front").output()?;
    if !front.status.success() {
        return Err(Error::new(
            ErrorKind::Other,
            "lsappinfo front failed",
        ));
    }
    let asn = String::from_utf8_lossy(&front.stdout).trim().to_string();
    if asn.is_empty() {
        return Err(Error::new(
            ErrorKind::NotFound,
            "lsappinfo front returned no frontmost ASN",
        ));
    }

    // 2. `lsappinfo info -only name <asn>` → `"LSDisplayName"="Safari"`
    let name_out = Command::new("/usr/bin/lsappinfo")
        .args(["info", "-only", "name", &asn])
        .output()?;
    let name = parse_lsappinfo_value(&String::from_utf8_lossy(&name_out.stdout))
        .ok_or_else(|| {
            Error::new(
                ErrorKind::NotFound,
                "could not read frontmost app name from lsappinfo",
            )
        })?;

    // 3. `lsappinfo info -only bundleid <asn>` → `"CFBundleIdentifier"="com.apple.Safari"`
    //    Optional: some apps have no bundle id.
    let bundle_out = Command::new("/usr/bin/lsappinfo")
        .args(["info", "-only", "bundleid", &asn])
        .output()?;
    let bundle_id =
        parse_lsappinfo_value(&String::from_utf8_lossy(&bundle_out.stdout));

    Ok(FrontmostApp {
        name,
        bundle_id,
        // No cheap no-Accessibility / no-Screen-Recording way to read the title.
        window_title: None,
    })
}

/// Non-macOS stub so the `spectyn` bin builds on ios/android/linux/windows.
#[cfg(not(target_os = "macos"))]
pub fn read_frontmost() -> std::io::Result<FrontmostApp> {
    use std::io::{Error, ErrorKind};
    Err(Error::new(
        ErrorKind::Unsupported,
        "active-app capture is macOS-only",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_value_display_name() {
        assert_eq!(
            parse_lsappinfo_value("\"LSDisplayName\"=\"Safari\""),
            Some("Safari".to_string())
        );
    }

    #[test]
    fn parse_value_bundleid_with_trailing_newline() {
        assert_eq!(
            parse_lsappinfo_value("\"CFBundleIdentifier\"=\"com.apple.Safari\"\n"),
            Some("com.apple.Safari".to_string())
        );
    }

    #[test]
    fn parse_value_malformed_returns_none() {
        assert_eq!(parse_lsappinfo_value("garbage"), None);
    }

    #[test]
    fn parse_value_empty_returns_none() {
        assert_eq!(parse_lsappinfo_value("\"KEY\"=\"\""), None);
    }

    #[test]
    fn focus_text_some_bundle_none_title() {
        let app = FrontmostApp {
            name: "Safari".to_string(),
            bundle_id: Some("com.apple.Safari".to_string()),
            window_title: None,
        };
        assert_eq!(focus_event_text(&app), "Active app: Safari (com.apple.Safari)");
    }

    #[test]
    fn focus_text_none_bundle() {
        let app = FrontmostApp {
            name: "Safari".to_string(),
            bundle_id: None,
            window_title: None,
        };
        assert_eq!(focus_event_text(&app), "Active app: Safari");
    }

    #[test]
    fn focus_text_some_title() {
        let app = FrontmostApp {
            name: "Safari".to_string(),
            bundle_id: Some("com.apple.Safari".to_string()),
            window_title: Some("Apple".to_string()),
        };
        assert_eq!(
            focus_event_text(&app),
            "Active app: Safari (com.apple.Safari) — Apple"
        );
    }
}
