// SPEC-31 §3.2(D) Reduced Motion helper.
//
// The CSS `scroll-behavior: auto !important` override in index.css does NOT
// affect a JS `scrollIntoView({ behavior: "smooth" })` call — an explicit
// "smooth" always animates regardless of CSS (per MDN). So gate the behavior
// on the OS "Reduce Motion" preference: reduce-motion users get an instant
// jump, everyone else keeps the smooth auto-scroll.
export function reducedMotionScrollBehavior(): ScrollBehavior {
  try {
    return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches
      ? "auto"
      : "smooth";
  } catch {
    return "smooth";
  }
}
