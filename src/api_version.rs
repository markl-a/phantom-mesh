//! API versioning and deprecation system for Clawtex.
//! Supports semver-style version negotiation, endpoint registration with deprecation
//! tracking, and version extraction from HTTP headers and URL paths.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

// ── Constants ──────────────────────────────────────────────────────────────────

/// Current API version of the Clawtex daemon.
pub const CURRENT_API_VERSION: ApiVersion = ApiVersion { major: 1, minor: 2, patch: 0 };

/// Minimum API version that the daemon will still serve.
pub const MIN_SUPPORTED_VERSION: ApiVersion = ApiVersion { major: 1, minor: 0, patch: 0 };

// ── ApiVersion ─────────────────────────────────────────────────────────────────

/// Semantic version for API endpoints: major.minor.patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ApiVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl ApiVersion {
    /// Create a new ApiVersion.
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch }
    }

    /// Parse a version string. Accepted formats: "v1", "v1.2", "v1.2.3", "1", "1.2", "1.2.3".
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty version string".to_string());
        }
        let s = s.strip_prefix('v').or_else(|| s.strip_prefix('V')).unwrap_or(s);
        let parts: Vec<&str> = s.split('.').collect();
        match parts.len() {
            1 => {
                let major = parts[0].parse::<u32>().map_err(|e| format!("invalid major: {}", e))?;
                Ok(Self { major, minor: 0, patch: 0 })
            }
            2 => {
                let major = parts[0].parse::<u32>().map_err(|e| format!("invalid major: {}", e))?;
                let minor = parts[1].parse::<u32>().map_err(|e| format!("invalid minor: {}", e))?;
                Ok(Self { major, minor, patch: 0 })
            }
            3 => {
                let major = parts[0].parse::<u32>().map_err(|e| format!("invalid major: {}", e))?;
                let minor = parts[1].parse::<u32>().map_err(|e| format!("invalid minor: {}", e))?;
                let patch = parts[2].parse::<u32>().map_err(|e| format!("invalid patch: {}", e))?;
                Ok(Self { major, minor, patch })
            }
            _ => Err(format!("too many version segments: {}", parts.len())),
        }
    }

    /// Check whether a requested version is compatible with the current version.
    /// Compatible means: same major version, and requested minor.patch <= current minor.patch.
    pub fn is_compatible(requested: &ApiVersion, current: &ApiVersion) -> bool {
        if requested.major != current.major {
            return false;
        }
        if requested.minor > current.minor {
            return false;
        }
        if requested.minor == current.minor && requested.patch > current.patch {
            return false;
        }
        true
    }

    /// Return a tuple for ordering comparisons.
    fn as_tuple(&self) -> (u32, u32, u32) {
        (self.major, self.minor, self.patch)
    }
}

impl fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl PartialOrd for ApiVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ApiVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_tuple().cmp(&other.as_tuple())
    }
}

// ── VersionedEndpoint ──────────────────────────────────────────────────────────

/// An API endpoint with version bounds and deprecation metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionedEndpoint {
    /// The endpoint path (e.g., "/status", "/hand/:name/run").
    pub path: String,
    /// Minimum version that supports this endpoint.
    pub min_version: ApiVersion,
    /// Maximum version that supports this endpoint (None = still active).
    pub max_version: Option<ApiVersion>,
    /// Whether this endpoint is deprecated.
    pub deprecated: bool,
    /// Date when the endpoint will be removed (ISO 8601 string, e.g., "2026-06-01").
    pub sunset_date: Option<String>,
    /// Optional description or handler info string.
    pub handler_info: String,
}

// ── DeprecationNotice ──────────────────────────────────────────────────────────

/// A notice describing a deprecated endpoint and its replacement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeprecationNotice {
    /// The endpoint that is deprecated.
    pub endpoint: String,
    /// Version since which the endpoint has been deprecated.
    pub deprecated_since: ApiVersion,
    /// Date when the endpoint will be removed.
    pub sunset_date: Option<String>,
    /// Replacement endpoint path, if any.
    pub replacement: Option<String>,
    /// Human-readable message.
    pub message: String,
}

impl fmt::Display for DeprecationNotice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DEPRECATED '{}' since {}", self.endpoint, self.deprecated_since)?;
        if let Some(ref sunset) = self.sunset_date {
            write!(f, " (sunset: {})", sunset)?;
        }
        if let Some(ref repl) = self.replacement {
            write!(f, " -> use '{}'", repl)?;
        }
        if !self.message.is_empty() {
            write!(f, ": {}", self.message)?;
        }
        Ok(())
    }
}

// ── ApiRegistry ────────────────────────────────────────────────────────────────

/// Registry of versioned API endpoints.
#[derive(Debug, Clone)]
pub struct ApiRegistry {
    endpoints: HashMap<String, Vec<VersionedEndpoint>>,
}

impl ApiRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            endpoints: HashMap::new(),
        }
    }

    /// Register an endpoint for a given version with handler info.
    pub fn register_endpoint(&mut self, path: &str, version: ApiVersion, handler_info: &str) {
        let endpoint = VersionedEndpoint {
            path: path.to_string(),
            min_version: version,
            max_version: None,
            deprecated: false,
            sunset_date: None,
            handler_info: handler_info.to_string(),
        };
        self.endpoints.entry(path.to_string()).or_default().push(endpoint);
    }

    /// Register a full VersionedEndpoint directly.
    pub fn register_versioned_endpoint(&mut self, endpoint: VersionedEndpoint) {
        self.endpoints
            .entry(endpoint.path.clone())
            .or_default()
            .push(endpoint);
    }

    /// Resolve the best matching endpoint for a given path and version.
    /// Returns the endpoint whose version range contains the requested version.
    pub fn resolve_endpoint(&self, path: &str, version: &ApiVersion) -> Option<&VersionedEndpoint> {
        let entries = self.endpoints.get(path)?;
        // Find the best match: min_version <= version, and max_version (if set) >= version.
        let mut best: Option<&VersionedEndpoint> = None;
        for ep in entries {
            if ep.min_version > *version {
                continue;
            }
            if let Some(ref max_v) = ep.max_version {
                if *max_v < *version {
                    continue;
                }
            }
            // Prefer the endpoint with the highest min_version (most specific).
            match best {
                None => best = Some(ep),
                Some(prev) if ep.min_version > prev.min_version => best = Some(ep),
                _ => {}
            }
        }
        best
    }

    /// List all endpoints that are available for a given version.
    pub fn list_endpoints(&self, version: &ApiVersion) -> Vec<&VersionedEndpoint> {
        let mut result = Vec::new();
        for entries in self.endpoints.values() {
            for ep in entries {
                if ep.min_version <= *version {
                    if let Some(ref max_v) = ep.max_version {
                        if *max_v < *version {
                            continue;
                        }
                    }
                    result.push(ep);
                }
            }
        }
        result.sort_by(|a, b| a.path.cmp(&b.path));
        result
    }

    /// Return all deprecated endpoints across all versions.
    pub fn deprecated_endpoints(&self) -> Vec<&VersionedEndpoint> {
        let mut result = Vec::new();
        for entries in self.endpoints.values() {
            for ep in entries {
                if ep.deprecated {
                    result.push(ep);
                }
            }
        }
        result.sort_by(|a, b| a.path.cmp(&b.path));
        result
    }

    /// Mark an endpoint as deprecated. Applies to all registered versions at that path.
    pub fn deprecate_endpoint(&mut self, path: &str, sunset_date: Option<&str>) {
        if let Some(entries) = self.endpoints.get_mut(path) {
            for ep in entries.iter_mut() {
                ep.deprecated = true;
                ep.sunset_date = sunset_date.map(|s| s.to_string());
            }
        }
    }

    /// Total number of endpoint registrations (including multiple versions of same path).
    pub fn total_registrations(&self) -> usize {
        self.endpoints.values().map(|v| v.len()).sum()
    }
}

impl Default for ApiRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Version extraction helpers ─────────────────────────────────────────────────

/// Extract an ApiVersion from an HTTP header string containing "X-API-Version: vN".
/// Scans line by line for the header name (case-insensitive).
pub fn extract_version_from_header(headers: &str) -> Option<ApiVersion> {
    for line in headers.lines() {
        let line = line.trim();
        // Match "X-API-Version: <value>" case-insensitively.
        if let Some(idx) = line.find(':') {
            let key = line[..idx].trim();
            if key.eq_ignore_ascii_case("X-API-Version") {
                let val = line[idx + 1..].trim();
                return ApiVersion::parse(val).ok();
            }
        }
    }
    None
}

/// Extract a version prefix from a URL path.
/// Given "/v1/status", returns Some((ApiVersion(1,0,0), "/status")).
/// Given "/v2.1/hand/run", returns Some((ApiVersion(2,1,0), "/hand/run")).
/// Given "/status" (no version prefix), returns None.
pub fn extract_version_from_path(path: &str) -> Option<(ApiVersion, &str)> {
    let path = path.trim();
    if !path.starts_with('/') {
        return None;
    }
    let after_slash = &path[1..];
    // The version segment is everything up to the next '/' or end of string.
    let (version_seg, rest) = match after_slash.find('/') {
        Some(idx) => (&after_slash[..idx], &path[1 + idx..]),
        None => (after_slash, ""),
    };

    // Version segment must start with 'v' or 'V' followed by a digit.
    if !version_seg.starts_with('v') && !version_seg.starts_with('V') {
        return None;
    }
    let digits_part = &version_seg[1..];
    if digits_part.is_empty() || !digits_part.chars().next().unwrap().is_ascii_digit() {
        return None;
    }

    let version = ApiVersion::parse(version_seg).ok()?;
    let remaining = if rest.is_empty() { "/" } else { rest };
    Some((version, remaining))
}

// ── VersionNegotiator ──────────────────────────────────────────────────────────

/// Negotiates the best API version to use given a client request and server capabilities.
pub struct VersionNegotiator;

impl VersionNegotiator {
    /// Negotiate the API version to use.
    /// - If `requested` is Some and is present in `supported`, use it.
    /// - If `requested` is Some but not in `supported`, find the latest compatible version.
    /// - If `requested` is None, fall back to the latest supported version.
    /// - `supported` must not be empty.
    pub fn negotiate(requested: Option<ApiVersion>, supported: &[ApiVersion]) -> ApiVersion {
        assert!(!supported.is_empty(), "supported versions must not be empty");

        let mut sorted: Vec<ApiVersion> = supported.to_vec();
        sorted.sort();
        let latest = *sorted.last().unwrap();

        match requested {
            None => latest,
            Some(req) => {
                // Exact match first
                if sorted.contains(&req) {
                    return req;
                }
                // Find the latest compatible version (same major, highest minor.patch <= requested)
                let mut best: Option<ApiVersion> = None;
                for &v in sorted.iter().rev() {
                    if ApiVersion::is_compatible(&v, &req) || ApiVersion::is_compatible(&req, &v) {
                        // We want the highest version in `supported` that the client can use.
                        // "Client requests v1.2, server has v1.0, v1.1, v1.3" -> pick v1.1 (highest <= v1.2)
                        if v.major == req.major && v <= req {
                            best = Some(v);
                            break; // sorted descending iteration, first match is highest
                        }
                    }
                }
                best.unwrap_or(latest)
            }
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // -- ApiVersion::parse tests --

    #[test]
    fn test_parse_v1() {
        let v = ApiVersion::parse("v1").unwrap();
        assert_eq!(v, ApiVersion::new(1, 0, 0));
    }

    #[test]
    fn test_parse_v1_2() {
        let v = ApiVersion::parse("v1.2").unwrap();
        assert_eq!(v, ApiVersion::new(1, 2, 0));
    }

    #[test]
    fn test_parse_v1_2_3() {
        let v = ApiVersion::parse("v1.2.3").unwrap();
        assert_eq!(v, ApiVersion::new(1, 2, 3));
    }

    #[test]
    fn test_parse_no_prefix() {
        let v = ApiVersion::parse("2.1.0").unwrap();
        assert_eq!(v, ApiVersion::new(2, 1, 0));
    }

    #[test]
    fn test_parse_uppercase_v() {
        let v = ApiVersion::parse("V3").unwrap();
        assert_eq!(v, ApiVersion::new(3, 0, 0));
    }

    #[test]
    fn test_parse_empty_fails() {
        assert!(ApiVersion::parse("").is_err());
    }

    #[test]
    fn test_parse_invalid_fails() {
        assert!(ApiVersion::parse("vx.y.z").is_err());
    }

    #[test]
    fn test_parse_too_many_segments() {
        assert!(ApiVersion::parse("v1.2.3.4").is_err());
    }

    // -- Display --

    #[test]
    fn test_display() {
        let v = ApiVersion::new(1, 2, 3);
        assert_eq!(v.to_string(), "v1.2.3");
    }

    // -- is_compatible --

    #[test]
    fn test_compatible_same_version() {
        let v = ApiVersion::new(1, 2, 0);
        assert!(ApiVersion::is_compatible(&v, &v));
    }

    #[test]
    fn test_compatible_lower_minor() {
        let req = ApiVersion::new(1, 0, 0);
        let cur = ApiVersion::new(1, 2, 0);
        assert!(ApiVersion::is_compatible(&req, &cur));
    }

    #[test]
    fn test_incompatible_different_major() {
        let req = ApiVersion::new(2, 0, 0);
        let cur = ApiVersion::new(1, 2, 0);
        assert!(!ApiVersion::is_compatible(&req, &cur));
    }

    #[test]
    fn test_incompatible_higher_minor() {
        let req = ApiVersion::new(1, 5, 0);
        let cur = ApiVersion::new(1, 2, 0);
        assert!(!ApiVersion::is_compatible(&req, &cur));
    }

    #[test]
    fn test_incompatible_higher_patch() {
        let req = ApiVersion::new(1, 2, 5);
        let cur = ApiVersion::new(1, 2, 3);
        assert!(!ApiVersion::is_compatible(&req, &cur));
    }

    #[test]
    fn test_compatible_lower_patch() {
        let req = ApiVersion::new(1, 2, 0);
        let cur = ApiVersion::new(1, 2, 3);
        assert!(ApiVersion::is_compatible(&req, &cur));
    }

    // -- Ordering --

    #[test]
    fn test_ordering() {
        let v1 = ApiVersion::new(1, 0, 0);
        let v1_1 = ApiVersion::new(1, 1, 0);
        let v2 = ApiVersion::new(2, 0, 0);
        assert!(v1 < v1_1);
        assert!(v1_1 < v2);
    }

    // -- extract_version_from_header --

    #[test]
    fn test_extract_version_from_header_present() {
        let headers = "Content-Type: application/json\r\nX-API-Version: v1\r\nAccept: */*";
        let v = extract_version_from_header(headers).unwrap();
        assert_eq!(v, ApiVersion::new(1, 0, 0));
    }

    #[test]
    fn test_extract_version_from_header_full_semver() {
        let headers = "X-API-Version: v2.3.1";
        let v = extract_version_from_header(headers).unwrap();
        assert_eq!(v, ApiVersion::new(2, 3, 1));
    }

    #[test]
    fn test_extract_version_from_header_missing() {
        let headers = "Content-Type: application/json\r\nAccept: */*";
        assert!(extract_version_from_header(headers).is_none());
    }

    #[test]
    fn test_extract_version_from_header_case_insensitive() {
        let headers = "x-api-version: v3";
        let v = extract_version_from_header(headers).unwrap();
        assert_eq!(v, ApiVersion::new(3, 0, 0));
    }

    // -- extract_version_from_path --

    #[test]
    fn test_extract_version_from_path_v1_status() {
        let (v, rest) = extract_version_from_path("/v1/status").unwrap();
        assert_eq!(v, ApiVersion::new(1, 0, 0));
        assert_eq!(rest, "/status");
    }

    #[test]
    fn test_extract_version_from_path_v2_1() {
        let (v, rest) = extract_version_from_path("/v2.1/hand/run").unwrap();
        assert_eq!(v, ApiVersion::new(2, 1, 0));
        assert_eq!(rest, "/hand/run");
    }

    #[test]
    fn test_extract_version_from_path_no_version() {
        assert!(extract_version_from_path("/status").is_none());
    }

    #[test]
    fn test_extract_version_from_path_only_version() {
        let (v, rest) = extract_version_from_path("/v1").unwrap();
        assert_eq!(v, ApiVersion::new(1, 0, 0));
        assert_eq!(rest, "/");
    }

    #[test]
    fn test_extract_version_from_path_no_leading_slash() {
        assert!(extract_version_from_path("v1/status").is_none());
    }

    // -- ApiRegistry --

    #[test]
    fn test_registry_register_and_resolve() {
        let mut reg = ApiRegistry::new();
        reg.register_endpoint("/status", ApiVersion::new(1, 0, 0), "status_handler");
        let ep = reg.resolve_endpoint("/status", &ApiVersion::new(1, 0, 0)).unwrap();
        assert_eq!(ep.path, "/status");
        assert_eq!(ep.handler_info, "status_handler");
    }

    #[test]
    fn test_registry_resolve_compatible_version() {
        let mut reg = ApiRegistry::new();
        reg.register_endpoint("/status", ApiVersion::new(1, 0, 0), "status_v1");
        // Requesting v1.2 should still resolve v1.0 endpoint (compatible)
        let ep = reg.resolve_endpoint("/status", &ApiVersion::new(1, 2, 0)).unwrap();
        assert_eq!(ep.handler_info, "status_v1");
    }

    #[test]
    fn test_registry_resolve_picks_highest_min() {
        let mut reg = ApiRegistry::new();
        reg.register_endpoint("/status", ApiVersion::new(1, 0, 0), "status_v1.0");
        reg.register_endpoint("/status", ApiVersion::new(1, 1, 0), "status_v1.1");
        // Requesting v1.2 should pick the v1.1 endpoint (highest min_version that fits)
        let ep = reg.resolve_endpoint("/status", &ApiVersion::new(1, 2, 0)).unwrap();
        assert_eq!(ep.handler_info, "status_v1.1");
    }

    #[test]
    fn test_registry_resolve_not_found() {
        let mut reg = ApiRegistry::new();
        reg.register_endpoint("/status", ApiVersion::new(2, 0, 0), "status_v2");
        // Requesting v1 should not match v2 endpoint
        assert!(reg.resolve_endpoint("/status", &ApiVersion::new(1, 0, 0)).is_none());
    }

    #[test]
    fn test_registry_list_endpoints() {
        let mut reg = ApiRegistry::new();
        reg.register_endpoint("/status", ApiVersion::new(1, 0, 0), "status");
        reg.register_endpoint("/health", ApiVersion::new(1, 1, 0), "health");
        let listed = reg.list_endpoints(&ApiVersion::new(1, 2, 0));
        assert_eq!(listed.len(), 2);
    }

    #[test]
    fn test_registry_list_excludes_future() {
        let mut reg = ApiRegistry::new();
        reg.register_endpoint("/status", ApiVersion::new(1, 0, 0), "status");
        reg.register_endpoint("/future", ApiVersion::new(2, 0, 0), "future");
        let listed = reg.list_endpoints(&ApiVersion::new(1, 5, 0));
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, "/status");
    }

    #[test]
    fn test_registry_deprecated_endpoints() {
        let mut reg = ApiRegistry::new();
        reg.register_endpoint("/old", ApiVersion::new(1, 0, 0), "old_handler");
        reg.register_endpoint("/new", ApiVersion::new(1, 0, 0), "new_handler");
        reg.deprecate_endpoint("/old", Some("2026-06-01"));
        let deprecated = reg.deprecated_endpoints();
        assert_eq!(deprecated.len(), 1);
        assert_eq!(deprecated[0].path, "/old");
        assert_eq!(deprecated[0].sunset_date, Some("2026-06-01".to_string()));
    }

    #[test]
    fn test_registry_max_version_filter() {
        let mut reg = ApiRegistry::new();
        let ep = VersionedEndpoint {
            path: "/legacy".to_string(),
            min_version: ApiVersion::new(1, 0, 0),
            max_version: Some(ApiVersion::new(1, 3, 0)),
            deprecated: true,
            sunset_date: Some("2026-12-01".to_string()),
            handler_info: "legacy_handler".to_string(),
        };
        reg.register_versioned_endpoint(ep);
        // v1.2 is within range
        assert!(reg.resolve_endpoint("/legacy", &ApiVersion::new(1, 2, 0)).is_some());
        // v1.5 is beyond max_version
        assert!(reg.resolve_endpoint("/legacy", &ApiVersion::new(1, 5, 0)).is_none());
    }

    // -- VersionNegotiator --

    #[test]
    fn test_negotiator_no_preference_uses_latest() {
        let supported = vec![
            ApiVersion::new(1, 0, 0),
            ApiVersion::new(1, 1, 0),
            ApiVersion::new(1, 2, 0),
        ];
        let result = VersionNegotiator::negotiate(None, &supported);
        assert_eq!(result, ApiVersion::new(1, 2, 0));
    }

    #[test]
    fn test_negotiator_exact_match() {
        let supported = vec![
            ApiVersion::new(1, 0, 0),
            ApiVersion::new(1, 1, 0),
            ApiVersion::new(1, 2, 0),
        ];
        let result = VersionNegotiator::negotiate(Some(ApiVersion::new(1, 1, 0)), &supported);
        assert_eq!(result, ApiVersion::new(1, 1, 0));
    }

    #[test]
    fn test_negotiator_compatible_fallback() {
        let supported = vec![
            ApiVersion::new(1, 0, 0),
            ApiVersion::new(1, 1, 0),
            ApiVersion::new(1, 3, 0),
        ];
        // Requesting v1.2 -- not exact, but v1.1 is the highest that is <= v1.2
        let result = VersionNegotiator::negotiate(Some(ApiVersion::new(1, 2, 0)), &supported);
        assert_eq!(result, ApiVersion::new(1, 1, 0));
    }

    #[test]
    fn test_negotiator_no_compatible_falls_to_latest() {
        let supported = vec![
            ApiVersion::new(2, 0, 0),
            ApiVersion::new(2, 1, 0),
        ];
        // Requesting v1.0 -- no major match, falls back to latest
        let result = VersionNegotiator::negotiate(Some(ApiVersion::new(1, 0, 0)), &supported);
        assert_eq!(result, ApiVersion::new(2, 1, 0));
    }

    // -- DeprecationNotice --

    #[test]
    fn test_deprecation_notice_display() {
        let notice = DeprecationNotice {
            endpoint: "/old/status".to_string(),
            deprecated_since: ApiVersion::new(1, 1, 0),
            sunset_date: Some("2026-06-01".to_string()),
            replacement: Some("/status".to_string()),
            message: "Use the new endpoint".to_string(),
        };
        let s = notice.to_string();
        assert!(s.contains("DEPRECATED '/old/status'"));
        assert!(s.contains("since v1.1.0"));
        assert!(s.contains("sunset: 2026-06-01"));
        assert!(s.contains("-> use '/status'"));
        assert!(s.contains("Use the new endpoint"));
    }

    #[test]
    fn test_deprecation_notice_minimal() {
        let notice = DeprecationNotice {
            endpoint: "/old".to_string(),
            deprecated_since: ApiVersion::new(1, 0, 0),
            sunset_date: None,
            replacement: None,
            message: String::new(),
        };
        let s = notice.to_string();
        assert!(s.contains("DEPRECATED '/old' since v1.0.0"));
        assert!(!s.contains("sunset"));
        assert!(!s.contains("-> use"));
    }

    // -- Constants --

    #[test]
    fn test_constants() {
        assert!(CURRENT_API_VERSION >= MIN_SUPPORTED_VERSION);
        assert_eq!(CURRENT_API_VERSION.major, 1);
        assert_eq!(MIN_SUPPORTED_VERSION.major, 1);
    }

    // -- Edge cases --

    #[test]
    fn test_parse_whitespace_trimmed() {
        let v = ApiVersion::parse("  v1.0.0  ").unwrap();
        assert_eq!(v, ApiVersion::new(1, 0, 0));
    }

    #[test]
    fn test_registry_total_registrations() {
        let mut reg = ApiRegistry::new();
        reg.register_endpoint("/a", ApiVersion::new(1, 0, 0), "a1");
        reg.register_endpoint("/a", ApiVersion::new(1, 1, 0), "a2");
        reg.register_endpoint("/b", ApiVersion::new(1, 0, 0), "b1");
        assert_eq!(reg.total_registrations(), 3);
    }
}
