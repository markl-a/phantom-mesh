// SPEC-34 §10D — pure decision for the Android system-back bridge.
//
// When the native Android WebView fires a hardware/gesture back press, the
// MobileShell handler must decide between two outcomes:
//   - "navigate-back": pop one SPA (single-page-app) history entry, i.e.
//     navigate(-1) within React Router.
//   - "passthrough": hand control back to Android so the OS performs its
//     default action (exit the app).
//
// We key on React Router's history index (`window.history.state.idx`), NOT on
// bottom-tab membership. Rationale: tab→tab navigation still pushes onto the
// history stack, so a tab route can have a prior entry worth popping. Only the
// first SPA entry (idx === 0) truly has nothing left to pop and should exit.
//
// This is the *pure* core of the decision. The interactive parts (the /focus
// window.confirm guard and the actual navigate(-1) / passthrough side-effects)
// stay in the MobileShell handler.

export type BackAction = "navigate-back" | "passthrough";

/**
 * Decide what an Android system-back press should do, given React Router's
 * current history index (`window.history.state.idx`).
 *
 * @param historyIdx the `idx` from `window.history.state`; may be undefined or
 *   null off-Android / before the first navigation.
 * @returns "navigate-back" when there is SPA history to pop (idx > 0),
 *   otherwise "passthrough" (first entry → let Android exit the app).
 */
export function decideSystemBack(historyIdx: number | undefined | null): BackAction {
  return (historyIdx ?? 0) > 0 ? "navigate-back" : "passthrough";
}
