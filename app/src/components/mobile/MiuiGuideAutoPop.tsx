import { useEffect, useState } from "react";
import MiuiGuideDialog from "./MiuiGuideDialog";
import { checkShouldShowMiuiGuide } from "../../lib/miuiGuide";

// SPEC-34 G6 / J5 — proactively surface the MIUI (小米系統) background-kill guide
// on MIUI / Redmi devices, where spectyn's foreground node service gets reaped
// overnight. Self-gates on the native should_show (is_miui && !dont_show_again),
// so it NEVER pops on non-MIUI devices / desktop / web, nor after the user ticked
// 不再提示. Rendered once by MobileShell, mirroring the NotifLockCard self-gating
// pattern.
//
// This is the v1 detection-based surface. The richer SPEC trigger — pop only
// AFTER the foreground service actually fails to start — is the deferred Kotlin
// service-stage refinement; until then, MIUI detection alone is a safe proxy
// (every MIUI device benefits from the whitelist guidance regardless).
//
// 完成 just closes for this session (should_show stays true → it nudges again
// next launch); only 不再提示 (which calls miui_guide_dismiss) silences it for
// good. That re-nudge is intentional — the kill is silent + nightly.
export default function MiuiGuideAutoPop() {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    let alive = true;
    void checkShouldShowMiuiGuide()
      .then((r) => {
        if (alive && r.should_show) setOpen(true);
      })
      // checkShouldShowMiuiGuide already swallows native errors → safe default,
      // but guard the promise anyway so a contract violation can never surface
      // as an unhandled rejection (and never auto-pops on doubt).
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  // Closed state renders nothing (MiuiGuideDialog returns null when !open), so
  // mounting this unconditionally on non-MIUI devices costs only one native
  // should_show check that resolves to false.
  return <MiuiGuideDialog open={open} onClose={() => setOpen(false)} />;
}
