/**
 * /term route — full-viewport TerminalShell, no sidebar, no chrome.
 *
 * On 5/2 this becomes the default landing page on Tauri desktop +
 * mobile; web keeps it as an opt-in route alongside the legacy full
 * app. See _planning-audit/MASTER-PLAN.md §1 v1.5.
 */

import TerminalShell from "../components/terminal/TerminalShell";

export default function Terminal() {
  return <TerminalShell />;
}
