// SPEC-31 §6 + G1 — deep-link router: phantom://<route> → allowlisted React Router navigate.
//
// Plumbing (this app's actual convention, NOT the generic "deep-link:opened"):
// the Rust side (src-tauri/src/lib.rs `app.deep_link().on_open_url`) routes an
// OS-level `phantom://...` URL and re-emits it as a `deep-link://<kind>` Tauri
// event — e.g. `deep-link://oauth-callback` (brokerLogin), `deep-link://demo-mode`,
// `deep-link://mdns-peer`. This hook subscribes to `deep-link://route`, the
// navigation-routing channel for allowlisted phantom:// navigation URLs.
//   ⚠ PAIRS WITH A RUST EMIT — and it MUST be allowlist-gated to preserve the
//   V8-HIGH-5 security model. lib.rs on_open_url already filters at the Rust layer
//   so only safe URLs reach JS (the OS routes EVERY `phantom://…` here, scheme
//   registration is path-blind, so an attacker could otherwise deliver crafted
//   payloads to any front-end listener). Therefore lib.rs must emit
//   `emit("deep-link://route", url)` ONLY for URLs whose route is in the
//   navigation allowlist below (credential-free, pure navigation signal — same
//   justification as the existing `phantom://demo-mode` case), NOT for "any URL it
//   doesn't special-case" (that would regress V8-HIGH-5). Concretely: parse the
//   route in Rust, forward only if it matches DEEPLINK_ALLOWED, else drop + log.
//   The frontend DEEPLINK_ALLOWED check below is then defense-in-depth, not the
//   primary gate. That emit is src-tauri scope (not this frontend lock) — requested
//   via mesh outbox. Until it lands, this hook is correctly wired but inert.
// It parses the URL, checks the deep-link allowlist (G1), and navigates only when
// the target is `deeplink_enabled`; a disallowed route logs `ios.deeplink.disallowed`.
//
// Call once near the app root (mirrors App.tsx's `listen(...)` + `useNavigate`
// idiom). In web / non-Tauri mode `listen` rejects (no Tauri IPC) → harmless
// no-op, guarded with `.catch`.
//
// Note: the broker OAuth callback (`phantom://oauth/callback?p=...`) is handled
// separately by installBrokerLoginBridge() in brokerLogin.ts (a distinct
// `deep-link://oauth-callback` event), so it is intentionally NOT in this
// allowlist.

import { useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { useNavigate } from "react-router-dom";

/**
 * Allowlist of routes reachable from an external `phantom://` URL, per
 * SPEC-31 §10.2 catalog (every row marked `deep-link ✓`). Parameterised
 * routes (`/chat/:id`, `/mesh/peer/:id`, `/error/permission/:perm`) and the
 * callback-only broker route are intentionally excluded from this static set:
 *   - `:id` / `:perm` segments cannot be matched as a fixed string here;
 *   - `/onboarding/broker` deep-links are callback-only (brokerLogin bridge).
 *
 * A parsed route NOT in this set is rejected (G1) → `ios.deeplink.disallowed`.
 */
const DEEPLINK_ALLOWED: ReadonlySet<string> = new Set<string>([
  // coach review — push-notification cold-launch target (SPEC-31 JS1 / G1)
  "/coach/review",
  "/coach",
  // capture entries (SPEC-31 §10.2 rows 7-9)
  "/capture/food",
  "/capture/focus",
  "/capture/habit",
  // chat list (parameterised /chat/:id excluded — see note above)
  "/chat",
  // settings surfaces (SPEC-31 §10.2 rows 12-15)
  "/settings",
  "/settings/cluster",
  "/settings/identity",
  "/settings/providers",
  // vault / skill (stretch, but deeplink_enabled in catalog)
  "/vault",
  "/skills",
]);

/** Result of parsing a `phantom://<route>[?query]` URL. */
interface ParsedDeepLink {
  /** React Router path, leading-slash normalised (e.g. "/coach/review"). */
  path: string;
  /** Query string including the leading "?", or "" when absent. */
  search: string;
}

/**
 * Parse a `phantom://<route>[?query]` URL into a React Router path + search.
 *
 * The URL constructor doesn't reliably parse custom schemes (mirrors the
 * manual string parsing in brokerLogin.ts `extractPayload`), so we work the
 * string directly. Returns null when the URL isn't a `phantom://` deep-link.
 */
function parseDeepLink(url: string): ParsedDeepLink | null {
  const SCHEME = "phantom://";
  if (!url.startsWith(SCHEME)) return null;

  // Everything after the scheme is `<route>[?query]`.
  let rest = url.slice(SCHEME.length);

  // Split off the query string (first "?").
  let search = "";
  const qIdx = rest.indexOf("?");
  if (qIdx >= 0) {
    search = rest.slice(qIdx); // includes the leading "?"
    rest = rest.slice(0, qIdx);
  }

  // Strip a trailing slash from the route (but keep a bare-root route valid).
  let route = rest.replace(/\/+$/, "");

  // Normalise to a leading-slash React Router path. "phantom://coach/review"
  // → "/coach/review"; "phantom://" (empty route) → "/".
  const path = route.length === 0 ? "/" : "/" + route.replace(/^\/+/, "");

  return { path, search };
}

/**
 * useDeepLinkRouter — installs a `deep-link://route` listener (this app's
 * deep-link event convention) that routes allowlisted `phantom://` deep-links
 * through React Router. Call once near the app root. Installs in `useEffect`
 * and cleans up the listener on unmount. (Requires the paired lib.rs emit — see
 * the file header.)
 */
export function useDeepLinkRouter(): void {
  const navigate = useNavigate();

  // Guards against the (async) listener firing after unmount — tauri-plugin
  // -deep-link may flush a pending event right after mount, and the unlisten
  // resolves asynchronously, so navigate() could otherwise run post-unmount.
  const aliveRef = useRef(true);

  useEffect(() => {
    aliveRef.current = true;
    let unlisten: (() => void) | undefined;

    // This app's deep-link navigation channel: lib.rs re-emits ALLOWLISTED
    // navigation phantom:// URLs as `deep-link://route` (Rust-side allowlist-gated
    // per V8-HIGH-5 — see file header). In web / non-Tauri mode `listen` rejects →
    // harmless no-op.
    const sub = listen<string>("deep-link://route", (evt) => {
      if (!aliveRef.current) return;

      const url = evt.payload;
      const parsed = parseDeepLink(url);
      if (!parsed) {
        console.warn("ios.deeplink.disallowed", url);
        return;
      }

      // G1: only navigate to explicitly deeplink_enabled routes. Anything
      // else (including unknown / parameterised routes) is rejected + logged.
      if (!DEEPLINK_ALLOWED.has(parsed.path)) {
        console.warn("ios.deeplink.disallowed", parsed.path);
        return;
      }

      if (!aliveRef.current) return;
      // Preserve query params (e.g. ?date=YYYY-MM-DD for coach review).
      navigate(parsed.path + parsed.search);
    });

    sub.then((un) => {
      if (aliveRef.current) {
        unlisten = un;
      } else {
        // Unmounted before the listener resolved — tear it down immediately.
        un();
      }
    }).catch(() => {
      // No Tauri IPC (web mode) or plugin unavailable → no-op.
    });

    return () => {
      aliveRef.current = false;
      if (unlisten) unlisten();
    };
  }, [navigate]);
}
