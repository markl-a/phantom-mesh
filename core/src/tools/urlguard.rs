//! Shared SSRF guard for all outbound HTTP tools.
//!
//! Extracted from `core/src/tools/fetch.rs` (which had the only correct
//! implementation pre-T7b) so that `web_fetch`, `http_get`, and `http_post`
//! all enforce the same loopback / private-IP / link-local block.
//!
//! Set `PHANTOM_FETCH_ALLOW_LOCAL=1` to permit private/loopback hosts
//! (intended for explicit, audited local-net workflows).

const MAX_URL_LEN: usize = 8192;

/// Validate that `url` is a public, http(s) URL.
///
/// Returns `Err(human-readable reason)` if blocked.
/// Order of checks (cheapest first):
///   1. Length cap
///   2. Scheme must be http:// or https://
///   3. Host parse (strip path + userinfo + port)
///   4. Allowed-local override (`PHANTOM_FETCH_ALLOW_LOCAL=1`) short-circuits remaining checks
///   5. Hostname must not be `localhost` / `ip6-localhost` / `::1`
///   6. IPv4 host: reject 127/8, 10/8, 172.16-31/12, 192.168/16, 169.254/16, 0/8
///   7. IPv6 host: reject `::1` (loopback) and `fc00::/7` (unique-local)
pub fn validate_url(url: &str) -> Result<(), String> {
    if url.len() > MAX_URL_LEN {
        return Err(format!(
            "URL exceeds maximum length of {} characters",
            MAX_URL_LEN
        ));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL must start with http:// or https://".to_string());
    }
    let after_scheme = if let Some(s) = url.strip_prefix("https://") {
        s
    } else {
        &url["http://".len()..]
    };

    // Host parse. IPv6 literal in URLs is `[::1]:443/path`; we strip the brackets.
    let host_with_port = after_scheme
        .split('/')
        .next()
        .unwrap_or("")
        .split('@')
        .last()
        .unwrap_or("");
    let host = if let Some(rest) = host_with_port.strip_prefix('[') {
        // IPv6 literal
        rest.split(']').next().unwrap_or("").to_lowercase()
    } else {
        host_with_port
            .split(':')
            .next()
            .unwrap_or("")
            .to_lowercase()
    };

    let allow_local = std::env::var("PHANTOM_FETCH_ALLOW_LOCAL").as_deref() == Ok("1");
    if allow_local {
        return Ok(());
    }

    if host == "localhost" || host == "ip6-localhost" || host == "::1" {
        return Err("blocked: private/loopback host".to_string());
    }
    if let Some(msg) = is_private_ipv4(&host) {
        return Err(msg);
    }
    if let Some(msg) = is_private_ipv6(&host) {
        return Err(msg);
    }
    Ok(())
}

pub fn is_private_ipv4(host: &str) -> Option<String> {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let octets: Vec<u8> = parts.iter().filter_map(|p| p.parse::<u8>().ok()).collect();
    if octets.len() != 4 {
        return None;
    }
    let (a, b, _c, _d) = (octets[0], octets[1], octets[2], octets[3]);
    if a == 127 {
        return Some("blocked: loopback address".to_string());
    }
    if a == 10 {
        return Some("blocked: private IP range 10.x.x.x".to_string());
    }
    if a == 172 && (16..=31).contains(&b) {
        return Some("blocked: private IP range 172.16-31.x.x".to_string());
    }
    if a == 192 && b == 168 {
        return Some("blocked: private IP range 192.168.x.x".to_string());
    }
    if a == 169 && b == 254 {
        return Some("blocked: link-local address".to_string());
    }
    if a == 0 {
        return Some("blocked: reserved IP range 0.x.x.x".to_string());
    }
    None
}

/// Returns Some(reason) if `host` parses as a private/reserved IPv6 literal.
/// Intentionally permissive — we only block the high-impact cases the
/// audit named (T13-N6 explicit list).
pub fn is_private_ipv6(host: &str) -> Option<String> {
    // We accept hosts that have already had `[]` stripped by validate_url.
    let h = host.trim();
    if h == "::1" {
        return Some("blocked: IPv6 loopback".to_string());
    }
    if h == "::" || h == "0:0:0:0:0:0:0:0" {
        return Some("blocked: IPv6 unspecified".to_string());
    }
    // Unique-local addresses: fc00::/7  (first byte = 0xfc or 0xfd)
    let lower = h.to_ascii_lowercase();
    if lower.starts_with("fc") || lower.starts_with("fd") {
        // Ensure it really is an IPv6 form (contains ':').
        if lower.contains(':') {
            return Some("blocked: IPv6 unique-local (fc00::/7)".to_string());
        }
    }
    // Link-local: fe80::/10
    if lower.starts_with("fe8")
        || lower.starts_with("fe9")
        || lower.starts_with("fea")
        || lower.starts_with("feb")
    {
        if lower.contains(':') {
            return Some("blocked: IPv6 link-local (fe80::/10)".to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize all tests that read/write PHANTOM_FETCH_ALLOW_LOCAL.
    // Process-global env state would race under cargo's default parallel runner.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn ok_public() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("PHANTOM_FETCH_ALLOW_LOCAL");
        assert!(validate_url("https://example.com/path?q=1").is_ok());
        assert!(validate_url("http://example.com").is_ok());
    }
    #[test]
    fn err_no_scheme() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(validate_url("example.com").is_err());
    }
    #[test]
    fn err_private_ipv4() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("PHANTOM_FETCH_ALLOW_LOCAL");
        assert!(validate_url("http://127.0.0.1/").is_err());
        assert!(validate_url("http://10.0.0.1/").is_err());
        assert!(validate_url("http://172.16.0.1/").is_err());
        assert!(validate_url("http://192.168.1.1/").is_err());
        assert!(validate_url("http://localhost/").is_err());
        assert!(validate_url("http://169.254.169.254/latest/meta-data/").is_err());
        assert!(validate_url("http://0.0.0.0/").is_err());
    }
    #[test]
    fn err_private_ipv6() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("PHANTOM_FETCH_ALLOW_LOCAL");
        assert!(validate_url("http://[::1]/").is_err());
        assert!(validate_url("http://[fc00::1]/").is_err());
        assert!(validate_url("http://[fe80::1]/").is_err());
    }
    #[test]
    fn allow_local_override() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("PHANTOM_FETCH_ALLOW_LOCAL", "1");
        let r = validate_url("http://127.0.0.1/");
        std::env::remove_var("PHANTOM_FETCH_ALLOW_LOCAL");
        assert!(r.is_ok(), "override should permit loopback");
    }
}
