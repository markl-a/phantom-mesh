/**
 * Minimal ANSI-to-React renderer. Handles the 16-colour SGR codes the
 * spectyn Mac TUI emits (`\e[31m` red, `\e[1m` bold, `\e[0m` reset, …)
 * plus 256-colour where used. We deliberately do NOT pull xterm.js or
 * ansi-to-react: this module is ~90 lines and covers the ~12 SGR codes
 * the agent loop actually emits, vs. >200KB gz for the alternatives.
 *
 * If we discover we need more (256-colour foreground/background, true
 * colour, hyperlinks via OSC 8), expand here rather than reach for a
 * dep — the API stays stable.
 */

import { Fragment, type ReactNode } from "react";

const ANSI_FG: Record<number, string> = {
  30: "var(--term-ansi-black)",
  31: "var(--term-ansi-red)",
  32: "var(--term-ansi-green)",
  33: "var(--term-ansi-yellow)",
  34: "var(--term-ansi-blue)",
  35: "var(--term-ansi-magenta)",
  36: "var(--term-ansi-cyan)",
  37: "var(--term-ansi-white)",
  90: "var(--term-dim)",
  91: "var(--term-error)",
  92: "var(--term-ok)",
  93: "var(--term-warn)",
  94: "var(--term-ansi-blue)",
  95: "var(--term-accent)",
  96: "var(--term-prompt)",
  97: "var(--term-fg)",
};

interface Span {
  text: string;
  fg?: string;
  bold?: boolean;
  dim?: boolean;
  italic?: boolean;
  underline?: boolean;
}

const ANSI_RE = /\x1b\[([0-9;]*)m/g;

function parse(input: string): Span[] {
  const out: Span[] = [];
  let last = 0;
  let cur: Omit<Span, "text"> = {};

  const flush = (end: number) => {
    if (end > last) {
      out.push({ text: input.slice(last, end), ...cur });
    }
    last = end;
  };

  ANSI_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = ANSI_RE.exec(input))) {
    flush(m.index);
    last = m.index + m[0].length;
    const codes = m[1] === "" ? [0] : m[1].split(";").map((s) => parseInt(s, 10));
    for (const c of codes) {
      if (c === 0)         cur = {};
      else if (c === 1)    cur = { ...cur, bold: true };
      else if (c === 2)    cur = { ...cur, dim: true };
      else if (c === 3)    cur = { ...cur, italic: true };
      else if (c === 4)    cur = { ...cur, underline: true };
      else if (c === 22)   cur = { ...cur, bold: false, dim: false };
      else if (c === 23)   cur = { ...cur, italic: false };
      else if (c === 24)   cur = { ...cur, underline: false };
      else if (c === 39)   cur = { ...cur, fg: undefined };
      else if (ANSI_FG[c]) cur = { ...cur, fg: ANSI_FG[c] };
      // 38;5;N (256-colour) and 38;2;R;G;B (truecolor) are rare in
      // spectyn output — ignored for now, falls through as default fg.
    }
  }
  flush(input.length);
  return out;
}

export interface AnsiTextProps {
  text: string;
  className?: string;
}

export default function AnsiText({ text, className }: AnsiTextProps): ReactNode {
  const spans = parse(text);
  return (
    <span className={className}>
      {spans.map((s, i) => (
        <Fragment key={i}>
          <span
            style={{
              color: s.fg,
              fontWeight: s.bold ? 600 : undefined,
              opacity: s.dim ? 0.6 : undefined,
              fontStyle: s.italic ? "italic" : undefined,
              textDecoration: s.underline ? "underline" : undefined,
            }}
          >
            {s.text}
          </span>
        </Fragment>
      ))}
    </span>
  );
}

// Exported for unit tests
export const __test = { parse };
