import { useEffect, useState } from "react";

const MOBILE_BREAKPOINT = 768; // matches Tailwind `md:`

/**
 * Detect whether the current device should render the mobile UI (MobileShell
 * + `/cluster` route) instead of the desktop sidebar/onboarding flow.
 *
 * Width alone is too brittle: a Xiaomi tablet reports 1600×2560 px and so
 * was being mis-classified as desktop, hiding the cluster-dispatch UI. We
 * therefore treat a device as "mobile" if ANY of these are true:
 *   1. Viewport width < MOBILE_BREAKPOINT (the original behaviour — phones).
 *   2. The device exposes touch points (`navigator.maxTouchPoints > 0`).
 *      Covers Android tablets, iPads, touch-only ChromeOS, etc.
 *   3. The user-agent string contains a known mobile/tablet platform
 *      (Android / iPhone / iPad / iPod). Belt-and-braces for stripped-down
 *      WebViews that lie about `maxTouchPoints`.
 *
 * Desktop Macs/Windows boxes with mice continue to hit the desktop branch
 * because they satisfy none of these. Macs with trackpads do NOT report
 * `maxTouchPoints > 0` in Safari/Chrome, so they remain unaffected.
 */
function detectIsMobile(): boolean {
  if (typeof window === "undefined") return false;

  // (1) classic narrow-viewport heuristic
  if (window.innerWidth < MOBILE_BREAKPOINT) return true;

  // (2) touch capability — works for Android tablets, iPads, etc.
  const nav = typeof navigator !== "undefined" ? navigator : undefined;
  if (nav && typeof nav.maxTouchPoints === "number" && nav.maxTouchPoints > 0) {
    return true;
  }

  // (3) UA fallback for embedded WebViews that under-report touch
  const ua = nav?.userAgent ?? "";
  if (/Android|iPhone|iPad|iPod/i.test(ua)) return true;

  return false;
}

export function useIsMobile(): boolean {
  const [isMobile, setIsMobile] = useState(detectIsMobile);

  useEffect(() => {
    if (typeof window === "undefined") return;
    const mql = window.matchMedia(`(max-width: ${MOBILE_BREAKPOINT - 1}px)`);
    // Re-run the full detection on viewport changes so rotating a tablet or
    // resizing a desktop window still resolves correctly (touch + UA checks
    // remain stable across resizes but width may flip).
    const handler = () => setIsMobile(detectIsMobile());
    mql.addEventListener("change", handler);
    setIsMobile(detectIsMobile());
    return () => mql.removeEventListener("change", handler);
  }, []);

  return isMobile;
}
