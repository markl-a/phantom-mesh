// SPEC-17 §11.2 deep-link navigation — maps a parsed DeepLinkRoute (host/path)
// to an in-app route. The Rust side (core::dispatch_deep_link, called from
// lib.rs on_open_url) already enforces the §8/§11.2 allowlist, path-traversal
// rejection, query caps, and OAuth-token sanitization, and only emits the
// `deep-link://navigate` event for credential-free navigation hosts. This is the
// final, defensive host→route mapping: anything not in the small known set
// returns null (no navigation) so a future/unexpected host can never deep-link
// the webview to an arbitrary path.
//
// oauth / demo-mode are NOT here — they're handled by their own listeners
// (broker_login_finish handoff / demo-mode flag) before this ever fires.

export function deepLinkPath(host?: string | null, path?: string | null): string | null {
  switch (host) {
    case "chat":
      return "/";
    case "mesh":
      return "/cluster";
    case "settings":
      // /settings/<section> deep-links a Settings sub-panel (MobileSettings
      // reads the section from the URL). Reuse only the first cleaned segment;
      // core already stripped `..`/`%2e%2e`, but keep this defensive.
      return path && /^[a-z0-9-]+$/i.test(path) ? `/settings/${path}` : "/settings";
    default:
      return null;
  }
}
