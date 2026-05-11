/**
 * SquadGrid — multi-peer streaming view shown during a /dispatch run.
 *
 * Layout: each dispatched peer gets one panel. Panels are stacked
 * vertically (mobile and desktop both — vertical scrolling beats
 * horizontal cramming on a 4" iPhone or a Pixel Fold). Within each
 * panel: header (peer + agent + status dot) + scrolling output area
 * with the same ANSI rendering as the main TerminalShell scrollback.
 *
 * Lifecycle:
 *  - Created when /dispatch slash kicks off (state.dispatching = true)
 *  - One <SquadPanel> per entry in dispatcher's plan
 *  - Each panel updates as its peer's response arrives
 *  - When all panels report `done`, parent collapses the grid back
 *    to the main transcript and renders the synthesizer's final output
 *
 * v0.1.0 prototype: response is single-shot (synchronous /rpc/squad/dispatch
 * returns the full output once the agent finishes). Each panel goes
 * through three states: pending (dot grey, "..."), streaming (we don't
 * actually stream in v0.1.0 — see Squad-b commit body), done (dot
 * green, full text), or error (dot red, error message).
 *
 * v0.2 will switch to SSE / WebSocket per-peer streams so panels fill
 * incrementally during the agent's run. The component already accepts
 * partial `output` updates so swapping the data source is the only
 * change needed.
 */

import { useMemo } from "react";
import AnsiText from "./AnsiText";

// ── Types ──────────────────────────────────────────────────────────────

export type PanelState = "pending" | "streaming" | "done" | "error";

export interface SquadPanelData {
  /** Peer name (e.g. "z13", "oracle", "ipad") */
  peer: string;
  /** Agent name on that peer (e.g. "recon", "enrich", "triage") */
  agent: string;
  /** Current panel state */
  state: PanelState;
  /** Output text — may be partial during streaming, final on done */
  output: string;
  /** Optional elapsed-ms surface (shown in header when state = done) */
  elapsedMs?: number;
}

export interface SquadGridProps {
  /** One entry per peer in the active dispatch plan */
  panels: SquadPanelData[];
}

// ── Single panel ──────────────────────────────────────────────────────

function dotColor(state: PanelState): string {
  if (state === "done") return "var(--term-ok)";
  if (state === "streaming") return "var(--term-prompt)";
  if (state === "error") return "var(--term-error)";
  return "var(--term-dim)";
}

function stateLabel(state: PanelState): string {
  if (state === "pending") return "queued";
  if (state === "streaming") return "running…";
  if (state === "done") return "done";
  return "error";
}

function SquadPanel({ data }: { data: SquadPanelData }) {
  const { peer, agent, state, output, elapsedMs } = data;
  return (
    <div
      style={{
        borderTop: "1px solid var(--term-dim)",
        padding: "var(--term-pad-y) var(--term-pad-x)",
        fontFamily: "var(--term-mono)",
        background: "var(--term-bg)",
        color: "var(--term-fg)",
      }}
    >
      {/* Header: dot + peer + agent + state + elapsed */}
      <div
        style={{
          display: "flex",
          gap: "0.6em",
          alignItems: "center",
          fontSize: "0.85em",
          color: "var(--term-dim)",
          marginBottom: "0.35em",
        }}
      >
        <span style={{ color: dotColor(state), fontSize: "0.9em" }}>●</span>
        <span style={{ color: "var(--term-accent)" }}>{peer}</span>
        <span>·</span>
        <span style={{ color: "var(--term-fg)" }}>{agent}</span>
        <span>·</span>
        <span>{stateLabel(state)}</span>
        {state === "done" && elapsedMs !== undefined ? (
          <span style={{ marginLeft: "auto" }}>{elapsedMs} ms</span>
        ) : null}
      </div>
      {/* Output area */}
      <div
        style={{
          whiteSpace: "pre-wrap",
          lineHeight: "var(--term-cell-h)",
          maxHeight: "12em",
          overflowY: "auto",
        }}
      >
        {state === "pending" ? (
          <span style={{ color: "var(--term-dim)" }}>…</span>
        ) : output ? (
          <AnsiText text={output} />
        ) : state === "streaming" ? (
          <span style={{ color: "var(--term-dim)" }}>…streaming…</span>
        ) : (
          <span style={{ color: "var(--term-dim)" }}>(no output)</span>
        )}
      </div>
    </div>
  );
}

// ── Grid ──────────────────────────────────────────────────────────────

export default function SquadGrid({ panels }: SquadGridProps) {
  const allDone = useMemo(
    () => panels.length > 0 && panels.every((p) => p.state === "done" || p.state === "error"),
    [panels],
  );

  return (
    <div
      role="region"
      aria-label="Squad dispatch grid"
      style={{
        margin: "0.4em 0",
        border: "1px solid var(--term-dim)",
        background: "var(--term-bg)",
      }}
    >
      <div
        style={{
          padding: "var(--term-pad-y) var(--term-pad-x)",
          fontFamily: "var(--term-mono)",
          fontSize: "0.85em",
          color: "var(--term-dim)",
          background: "var(--term-bg)",
        }}
      >
        ▸ squad dispatch · {panels.length} peer{panels.length === 1 ? "" : "s"}
        {allDone ? " · all done" : " · running…"}
      </div>
      {panels.map((p, i) => (
        <SquadPanel key={`${p.peer}:${p.agent}:${i}`} data={p} />
      ))}
    </div>
  );
}
