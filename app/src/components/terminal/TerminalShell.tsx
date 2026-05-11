/**
 * TerminalShell — the unified UI surface across all 5 platforms (web,
 * Tauri desktop, Tauri iOS, Tauri Android, Termux). Renders a header,
 * a scrollback transcript, and a single-line prompt. No tabs, no
 * sidebars — slash commands handle everything (/help /agent /model …).
 *
 * v1.5 prototype: scrollback is in-memory only; daemon wiring lands
 * on 5/2 (replace the demo seed with a fetch to /api/sessions and a
 * WebSocket subscribe to /ws). Worker capability indicator + handoff
 * surface arrive on 5/3 (§2 of the v1.5 plan).
 */

import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import AnsiText from "./AnsiText";
import SquadGrid, { type SquadPanelData } from "./SquadGrid";
import "../../styles/terminal-tokens.css";

// ── Transcript model ────────────────────────────────────────────────────

type LineKind =
  | "input"      // user typed
  | "output"     // agent / tool response (may carry ANSI)
  | "system"    // /help banner, slash command echo, errors
  | "warn"
  | "error";

interface Line {
  id: string;
  kind: LineKind;
  text: string;
  ts: number;
}

let LID = 0;
const mkLine = (kind: LineKind, text: string): Line => ({
  id: `${++LID}`,
  kind,
  text,
  ts: Date.now(),
});

// ── Initial seed (replaced by /api/sessions fetch on 5/2) ───────────────

const SEED: Line[] = [
  mkLine("system", "phantom 0.4.0 (\x1b[36m6a58dda9ef+\x1b[0m, macos-aarch64) · session local"),
  mkLine("system", "Type \x1b[36m/help\x1b[0m for commands, \x1b[36m/agent\x1b[0m to switch agent."),
  mkLine("output", "\x1b[2m─── ready ───\x1b[0m"),
];

// ── Slash commands (prototype subset; full set wired on 5/2) ────────────

interface CmdResult {
  lines: Line[];
  newAgent?: string;
  /** When set, parent should kick off a Squad Pipeline run with this plan */
  startDispatch?: SquadPanelData[];
}

function runSlash(cmd: string, agent: string): CmdResult {
  const [head, ...rest] = cmd.trim().slice(1).split(/\s+/);
  switch (head) {
    case "help":
      return {
        lines: [
          mkLine("system", "\x1b[1mAvailable commands\x1b[0m"),
          mkLine("output", "  \x1b[36m/help\x1b[0m                  this list"),
          mkLine("output", "  \x1b[36m/agent <name>\x1b[0m          switch agent (master, coder, …)"),
          mkLine("output", "  \x1b[36m/model\x1b[0m                 show current model"),
          mkLine("output", "  \x1b[36m/cost\x1b[0m                  session cost summary"),
          mkLine("output", "  \x1b[36m/clear\x1b[0m                 clear scrollback"),
          mkLine("output", "  \x1b[36m/dispatch <NL>\x1b[0m         Squad Pipeline — fan a NL goal across mesh peers"),
          mkLine("output", "  \x1b[2m(more arrive when daemon wiring lands on 5/2)\x1b[0m"),
        ],
      };
    case "agent": {
      const next = rest[0];
      if (!next) return { lines: [mkLine("system", `current agent: \x1b[35m${agent}\x1b[0m`)] };
      return {
        lines: [mkLine("system", `agent: \x1b[35m${agent}\x1b[0m → \x1b[35m${next}\x1b[0m`)],
        newAgent: next,
      };
    }
    case "model":
      return { lines: [mkLine("output", "\x1b[2m(daemon connection pending — 5/2)\x1b[0m")] };
    case "cost":
      return { lines: [mkLine("output", "\x1b[2m(daemon connection pending — 5/2)\x1b[0m")] };
    case "clear":
      return { lines: [] };
    case "dispatch": {
      const goal = rest.join(" ").trim();
      if (!goal) {
        return {
          lines: [
            mkLine("error", "usage: /dispatch <natural-language goal>"),
            mkLine("output", "  example: /dispatch 對 codebase 做安全分析"),
          ],
        };
      }
      // Mocked dispatch plan for the prototype. Real plan comes from
      // [agent.dispatcher] via /rpc/squad/dispatch when daemon wiring
      // lands (task #36, 5/2). The mock matches the example in
      // SPEC-FREEZE-V1 §9.3 Act 2 step [3].
      const plan: SquadPanelData[] = [
        { peer: "z13",    agent: "recon",   state: "pending", output: "" },
        { peer: "oracle", agent: "enrich",  state: "pending", output: "" },
        { peer: "ayaneo", agent: "review",  state: "pending", output: "" },
        { peer: "ipad",   agent: "triage",  state: "pending", output: "" },
      ];
      return {
        lines: [
          mkLine("system", `▸ /dispatch  \x1b[35m${goal}\x1b[0m`),
          mkLine("output", "\x1b[2m  dispatcher agent emitting plan…\x1b[0m"),
          mkLine("output", "\x1b[2m  4 peers selected: z13 (recon), oracle (enrich), ayaneo (review), ipad (triage)\x1b[0m"),
          mkLine("output", "\x1b[2m  (mock dispatch — daemon wiring lands 5/2, real RPC fan-out follows)\x1b[0m"),
        ],
        startDispatch: plan,
      };
    }
    default:
      return { lines: [mkLine("error", `unknown command: /${head}`)] };
  }
}

// ── Header ──────────────────────────────────────────────────────────────

interface PeerDot {
  name: string;
  status: "online" | "warn" | "offline";
}

const DEMO_PEERS: PeerDot[] = [
  { name: "mac",     status: "online" },
  { name: "z13-win", status: "online" },
  { name: "linux",   status: "warn" },
  { name: "ios",     status: "offline" },
  { name: "android", status: "offline" },
];

function dotColour(s: PeerDot["status"]): string {
  if (s === "online") return "var(--term-ok)";
  if (s === "warn")   return "var(--term-warn)";
  return "var(--term-dim)";
}

function Header({ version, node, peers }: { version: string; node: string; peers: PeerDot[] }) {
  return (
    <div
      style={{
        display: "flex",
        gap: "1em",
        alignItems: "center",
        padding: "var(--term-pad-y) var(--term-pad-x)",
        borderBottom: "1px solid var(--term-dim)",
        fontFamily: "var(--term-mono)",
        fontSize: "0.85em",
        color: "var(--term-dim)",
        background: "var(--term-bg)",
        flexShrink: 0,
      }}
    >
      <span>
        <span style={{ color: "var(--term-fg)" }}>phantom</span>{" "}
        <span style={{ color: "var(--term-prompt)" }}>{version}</span>
      </span>
      <span>·</span>
      <span style={{ color: "var(--term-accent)" }}>{node}</span>
      <span>·</span>
      <span style={{ display: "flex", gap: "0.25em" }}>
        {peers.map((p) => (
          <span
            key={p.name}
            title={`${p.name}: ${p.status}`}
            style={{ color: dotColour(p.status), fontSize: "0.9em" }}
          >
            ●
          </span>
        ))}
      </span>
    </div>
  );
}

// ── Scrollback ──────────────────────────────────────────────────────────

function lineColour(kind: LineKind): string {
  if (kind === "input")  return "var(--term-prompt)";
  if (kind === "warn")   return "var(--term-warn)";
  if (kind === "error")  return "var(--term-error)";
  if (kind === "system") return "var(--term-dim)";
  return "var(--term-fg)";
}

function Scrollback({ lines }: { lines: Line[] }) {
  const ref = useRef<HTMLDivElement>(null);
  // Keep latest line in view (user can scroll up; only auto-scroll if
  // they were already at bottom).
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const isNearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
    if (isNearBottom) el.scrollTop = el.scrollHeight;
  }, [lines]);

  return (
    <div
      ref={ref}
      role="log"
      style={{
        flex: 1,
        overflowY: "auto",
        padding: "var(--term-pad-y) var(--term-pad-x)",
        fontFamily: "var(--term-mono)",
        lineHeight: "var(--term-cell-h)",
        background: "var(--term-bg)",
        color: "var(--term-fg)",
      }}
    >
      {lines.map((l) => (
        <div key={l.id} style={{ color: lineColour(l.kind), whiteSpace: "pre-wrap" }}>
          {l.kind === "input" ? (
            <>
              <span style={{ color: "var(--term-prompt)" }}>{"▸ "}</span>
              {l.text}
            </>
          ) : (
            <AnsiText text={l.text} />
          )}
        </div>
      ))}
    </div>
  );
}

// ── Prompt ──────────────────────────────────────────────────────────────

function Prompt({
  agent,
  onSubmit,
}: {
  agent: string;
  onSubmit: (raw: string) => void;
}) {
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  const handleKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      const t = draft.trim();
      if (!t) return;
      onSubmit(t);
      setDraft("");
    }
  };

  // Autofocus on mount + when agent changes (rendered once at top
  // level, so this is effectively focus-on-page-load).
  useEffect(() => {
    inputRef.current?.focus();
  }, [agent]);

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "0.5em",
        padding: "var(--term-pad-y) var(--term-pad-x)",
        borderTop: "1px solid var(--term-dim)",
        background: "var(--term-bg)",
        fontFamily: "var(--term-mono)",
        fontSize: "var(--term-fs)",
        flexShrink: 0,
      }}
    >
      <span style={{ color: "var(--term-accent)" }}>{agent}</span>
      <span style={{ color: "var(--term-prompt)" }}>{"▸"}</span>
      <input
        ref={inputRef}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={handleKey}
        spellCheck={false}
        autoCorrect="off"
        autoCapitalize="none"
        style={{
          flex: 1,
          background: "transparent",
          border: "none",
          outline: "none",
          color: "var(--term-fg)",
          fontFamily: "inherit",
          fontSize: "inherit",
          caretColor: "var(--term-prompt)",
        }}
      />
    </div>
  );
}

// ── Top-level ───────────────────────────────────────────────────────────

export interface TerminalShellProps {
  version?: string;
  node?: string;
  peers?: PeerDot[];
  initialAgent?: string;
}

export default function TerminalShell({
  version = "0.4.0",
  node = "mac-coordinator",
  peers = DEMO_PEERS,
  initialAgent = "master",
}: TerminalShellProps) {
  const [lines, setLines] = useState<Line[]>(SEED);
  const [agent, setAgent] = useState(initialAgent);
  // Active Squad Pipeline panels — non-empty array means a /dispatch
  // is in flight. Empty array means scrollback shows normal transcript.
  const [squadPanels, setSquadPanels] = useState<SquadPanelData[]>([]);

  // Fetch /api/sessions on mount to replace the SEED's hardcoded
  // version line with live daemon-reported version + session count.
  // Falls back silently to SEED if daemon isn't reachable (offline
  // demo prep, or running TerminalShell as a static html preview).
  useEffect(() => {
    const ac = new AbortController();
    (async () => {
      try {
        const [sessRes, verRes] = await Promise.all([
          fetch("/api/sessions", { signal: ac.signal }),
          fetch("/api/version", { signal: ac.signal }),
        ]);
        if (!sessRes.ok || !verRes.ok) return;
        const sessions: Array<{ id: string; message_count: number }> = await sessRes.json();
        const ver: { version: string; commit: string } = await verRes.json();
        const totalMsgs = sessions.reduce((sum, s) => sum + (s.message_count ?? 0), 0);
        setLines((cur) => [
          mkLine(
            "system",
            `phantom \x1b[36m${ver.version}\x1b[0m (\x1b[36m${ver.commit}\x1b[0m, macos-aarch64) · ${sessions.length} session${sessions.length === 1 ? "" : "s"} · ${totalMsgs} messages`,
          ),
          ...cur.slice(1), // drop the SEED's hardcoded version line; keep the rest
        ]);
      } catch (_e) {
        // Daemon unreachable — leave SEED as-is. Don't surface the
        // error in scrollback to keep the prototype quiet for offline
        // preview.
      }
    })();
    return () => ac.abort();
  }, []);

  // Map a panel's peer name → daemon URL. For 5/9 demo this is read
  // from /api/status's cluster.peers field once daemon-mounted; for
  // the prototype, derive from the same hardcoded config the SquadGrid
  // mock used. When the user picks "real" mode (env var or future
  // settings panel), URLs here resolve via /api/status fetched on
  // mount. v0.1.0 ships with both modes side-by-side so demo can
  // toggle between mock + live without reload.
  const PEER_URL_MAP: Record<string, string> = {
    z13: "http://100.87.70.65:7878",
    oracle: "http://100.107.205.98:7878", // placeholder until Oracle Cloud A1 stood up
    ayaneo: "http://100.107.205.98:7878",
    ipad: "http://localhost:7878", // sandbox worker on same Tailscale, IP TBD
  };

  // Try real /rpc/squad/dispatch fan-out; fall back to mock on any
  // single-peer failure so the demo doesn't crater. Each panel's
  // state goes pending → streaming (when fetch fires) → done | error.
  const runRealDispatch = useCallback(async (plan: SquadPanelData[]) => {
    setSquadPanels(plan);

    // Fan out: one fetch per (peer, agent) entry. We DON'T await
    // sequentially — Promise.all so all peers run in parallel.
    const promises = plan.map(async (p, idx) => {
      const url = PEER_URL_MAP[p.peer];
      if (!url) {
        setSquadPanels((cur) =>
          cur.map((x, i) =>
            i === idx ? { ...x, state: "error", output: `unknown peer: ${p.peer}` } : x,
          ),
        );
        return;
      }
      setSquadPanels((cur) =>
        cur.map((x, i) => (i === idx ? { ...x, state: "streaming" } : x)),
      );
      const started = Date.now();
      try {
        // NOTE: HMAC required when cluster_secret configured; the
        // browser can't compute it without server-side help. For
        // demo, run against a coordinator that mirrors itself — Mac
        // dispatching to peers via the daemon's own outbound HMAC
        // (a future /api/dispatch proxy lands the HMAC server-side).
        // For prototype, hit /rpc/squad/dispatch directly and accept
        // 401 as "expected when secret set; will hit dispatch proxy
        // post-#36"
        const resp = await fetch(`${url}/rpc/squad/dispatch`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            agent: p.agent,
            prompt: `(mock prompt for ${p.peer}'s ${p.agent})`,
            wire_version: 1,
          }),
        });
        const elapsed = Date.now() - started;
        if (!resp.ok) {
          const errBody = await resp.text();
          setSquadPanels((cur) =>
            cur.map((x, i) =>
              i === idx
                ? {
                    ...x,
                    state: "error",
                    output: `HTTP ${resp.status}: ${errBody.slice(0, 200)}`,
                    elapsedMs: elapsed,
                  }
                : x,
            ),
          );
          return;
        }
        const data = await resp.json();
        setSquadPanels((cur) =>
          cur.map((x, i) =>
            i === idx
              ? {
                  ...x,
                  state: "done",
                  output: data.output ?? "(empty response)",
                  elapsedMs: data.elapsed_ms ?? elapsed,
                }
              : x,
          ),
        );
      } catch (e: unknown) {
        const elapsed = Date.now() - started;
        const msg = e instanceof Error ? e.message : String(e);
        setSquadPanels((cur) =>
          cur.map((x, i) =>
            i === idx
              ? { ...x, state: "error", output: `network error: ${msg}`, elapsedMs: elapsed }
              : x,
          ),
        );
      }
    });

    await Promise.all(promises);

    // All panels resolved (done or error). Synthesizer step lives
    // inside the daemon at /api/dispatch/synthesize (lands post-#36
    // when full daemon wiring is done). For now, surface the
    // collected outputs in scrollback as a TL;DR list and collapse
    // the grid.
    setSquadPanels((cur) => {
      setLines((curLines) => [
        ...curLines,
        mkLine("system", "\x1b[2m  squad pipeline run complete (synthesizer pending #36 daemon wiring)\x1b[0m"),
        ...cur.map((p) =>
          mkLine(
            "output",
            `  \x1b[36m${p.peer}\x1b[0m · \x1b[35m${p.agent}\x1b[0m → ${p.state === "done" ? "✓" : "✗"} ${p.elapsedMs ? `${p.elapsedMs}ms` : ""}`,
          ),
        ),
      ]);
      return [];
    });
  }, []);

  const submit = useCallback(
    (raw: string) => {
      // Echo the input line first
      const next: Line[] = [mkLine("input", raw)];
      if (raw.startsWith("/")) {
        const r = runSlash(raw, agent);
        next.push(...r.lines);
        setLines((cur) => (raw === "/clear" ? [] : [...cur, ...next]));
        if (r.newAgent) setAgent(r.newAgent);
        if (r.startDispatch) runRealDispatch(r.startDispatch);
        return;
      }
      // Non-slash: pending daemon wiring (5/2). Stub a clear note so
      // the prototype is honest about what's not wired.
      next.push(
        mkLine("output", "\x1b[2m(prompt sent — daemon wiring lands 5/2; nothing reached an LLM)\x1b[0m"),
      );
      setLines((cur) => [...cur, ...next]);
    },
    [agent, runRealDispatch],
  );

  // Layout: full-viewport flex column.
  const style = useMemo(
    () => ({
      display: "flex",
      flexDirection: "column" as const,
      height: "100vh",
      width: "100%",
      background: "var(--term-bg)",
      fontSize: "var(--term-fs)",
    }),
    [],
  );

  return (
    <div style={style}>
      <Header version={version} node={node} peers={peers} />
      <Scrollback lines={lines} />
      {squadPanels.length > 0 ? <SquadGrid panels={squadPanels} /> : null}
      <Prompt agent={agent} onSubmit={submit} />
    </div>
  );
}
