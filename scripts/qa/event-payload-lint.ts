#!/usr/bin/env node
/*
 * SPEC-17 §13 / G7 — event-payload secret lint.
 *
 * Big Goal pillar served: P4 (加密為先 / encryption-first). Backend->frontend
 * event payloads must NEVER carry a plaintext user secret (API key, OAuth
 * token, private key). This lint scans the payloads passed to Tauri
 * `.emit(...)` call sites and flags forbidden field names.
 *
 * STAGE (per SPEC-17 §migration): stage 1 = WARN-ONLY. This run reports a
 * deviation list and ALWAYS exits 0 unless --strict is passed (or the env
 * EVENT_PAYLOAD_LINT_STRICT=1 is set), so it can land on PR without blocking
 * while the deviation list is triaged. Flip to strict at GA.
 *
 * SCOPE / KNOWN LIMITATION (logged, not silent): v1 inspects only INLINE
 * `json!({ ... })` payloads — the dominant emit pattern. Struct-typed payloads
 * (e.g. `win.emit(name, SomeStruct { ... })`) are NOT yet field-resolved;
 * they are counted and reported as "unchecked" so coverage is honest. A
 * follow-up can resolve struct definitions. This keeps false positives near
 * zero (it never scans command/keystore structs that legitimately hold
 * tokens, e.g. broker_login.rs).
 *
 * Also NOT detected (documented false-negative, vanishingly rare / not valid
 * `json!` syntax in practice): a raw-string used as an object KEY, e.g.
 * `json!({ r#"access_token"#: v })`. Raw strings are blanked wholesale, so a
 * raw-string key is skipped. Acceptable for stage-1; revisit if it ever appears.
 *
 * SECRET-SAFE OUTPUT: only the forbidden field NAME + event name + file:line are
 * printed — never the payload value — so the lint can't itself leak a literal
 * secret into CI logs. The balanced-paren scanner skips string/char literals and
 * comments so a `)` inside a string can't truncate a payload (false-green).
 *
 * No external deps — runs on plain `node` (>=23.6 strips TS types) or `npx tsx`.
 *
 * Usage:
 *   node scripts/qa/event-payload-lint.ts                 # warn-only
 *   node scripts/qa/event-payload-lint.ts --strict        # exit 1 on any hit
 *   node scripts/qa/event-payload-lint.ts --root <dir>    # override scan roots
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

// Forbidden field-name roots per SPEC-17 §13. Matched as case-insensitive
// SUBSTRINGS (not exact) so real-world variants are caught: bare `token` is
// rare, payloads use `access_token` / `id_token` / `broker_token`, and keys
// appear as `client_secret` / `provider_api_key`. Over-matching (e.g. a
// hypothetical `token_count`) is acceptable because this lint is WARN-ONLY in
// stage 1 — a flagged-but-benign field is triaged, never blocks a build.
const FORBIDDEN = ["api_key", "token", "secret", "password", "private_key"];
const FORBIDDEN_RE = new RegExp(`(${FORBIDDEN.join("|")})`, "i");

// Default scan roots: where Tauri events are emitted.
const DEFAULT_ROOTS = ["app/src-tauri/src", "core/src"];

const argv = process.argv.slice(2);
const strict =
  argv.includes("--strict") || process.env.EVENT_PAYLOAD_LINT_STRICT === "1";
const rootIdx = argv.indexOf("--root");
if (rootIdx >= 0 && (!argv[rootIdx + 1] || argv[rootIdx + 1].startsWith("--"))) {
  // Reject `--root` with no value AND `--root --strict` (which would otherwise
  // swallow the flag as a bogus root path and silently scan nothing).
  console.error("usage: --root <dir> requires a directory argument");
  process.exit(2);
}
const roots = rootIdx >= 0 ? [argv[rootIdx + 1]] : DEFAULT_ROOTS;

interface Finding {
  file: string;
  line: number;
  key: string;
  event: string; // event NAME only — never the payload value (avoid leaking secrets to CI logs)
}

function walkRustFiles(dir: string): string[] {
  let out: string[] = [];
  let entries: string[];
  try {
    entries = readdirSync(dir);
  } catch {
    return out; // root may not exist on this checkout slice — skip quietly
  }
  for (const name of entries) {
    const p = join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) {
      if (name === "target" || name === "node_modules") continue;
      out = out.concat(walkRustFiles(p));
    } else if (name.endsWith(".rs")) {
      out.push(p);
    }
  }
  return out;
}

// Rust raw string? If a raw string literal (r"...", r#"..."#, r##"..."##, ...)
// starts at index `i` (with `r` at a token boundary), return the index JUST PAST
// its closing delimiter; else -1. Raw strings have no escapes and may contain `"`
// and `)`, so the normal scanners must skip them wholesale (codex round-5: a `)`
// or forbidden word inside r#"..."# otherwise truncates/false-flags).
function rawStringEnd(s: string, i: number): number {
  if (s[i] !== "r") return -1;
  if (i > 0 && /[A-Za-z0-9_]/.test(s[i - 1])) return -1; // part of an identifier
  let j = i + 1;
  let hashes = 0;
  while (s[j] === "#") { hashes++; j++; }
  if (s[j] !== '"') return -1;
  j++;
  const close = '"' + "#".repeat(hashes);
  const idx = s.indexOf(close, j);
  return idx < 0 ? s.length : idx + close.length;
}

// Return the substring from `openParen` spanning balanced parentheses, while
// IGNORING parens that live inside Rust string literals / char literals / line
// + block comments. Without this, a `)` inside a string (e.g. emit("a)b", ...))
// would truncate the payload early and silently drop later forbidden keys =
// false-green (codex review finding).
function balancedParens(src: string, openParen: number): string {
  let depth = 0;
  let inStr = false; // inside "..."
  let inChar = false; // inside '...'
  let inLine = false; // inside // ...
  let inBlock = false; // inside /* ... */
  for (let i = openParen; i < src.length; i++) {
    const c = src[i];
    const n = src[i + 1];
    if (inLine) {
      if (c === "\n") inLine = false;
      continue;
    }
    if (inBlock) {
      if (c === "*" && n === "/") {
        inBlock = false;
        i++;
      }
      continue;
    }
    if (inStr) {
      if (c === "\\") i++; // skip escaped char
      else if (c === '"') inStr = false;
      continue;
    }
    if (inChar) {
      if (c === "\\") i++;
      else if (c === "'") inChar = false;
      continue;
    }
    if (c === "/" && n === "/") { inLine = true; i++; continue; }
    if (c === "/" && n === "*") { inBlock = true; i++; continue; }
    if (c === "r" && (n === '"' || n === "#")) {
      const end = rawStringEnd(src, i);
      if (end >= 0) { i = end - 1; continue; } // skip raw string wholesale
    }
    if (c === '"') { inStr = true; continue; }
    if (c === "'") { inChar = true; continue; }
    if (c === "(") depth++;
    else if (c === ")") {
      depth--;
      if (depth === 0) return src.slice(openParen, i + 1);
    }
  }
  return src.slice(openParen); // unbalanced (truncated) — scan what we have
}

// Blank the CONTENTS of string VALUES and comments (same length, so indices still
// map to `src`) WITHOUT touching string KEYS. A forbidden word inside a payload
// value — e.g. `"note": "{ token: x }"` — must not be read as an object key
// (codex false-positive finding), but quoted keys like `"access_token":` MUST be
// preserved or we'd blank the very thing we detect. A string is a VALUE when the
// last significant code char before it is `:`; otherwise (after `{` `,` `(` `[`)
// it is a key / arg and is kept. Char literals + comments are always blanked.
function maskStringValues(s: string): string {
  const out = s.split("");
  let lastSig = ""; // last non-space code char seen outside strings/comments
  let i = 0;
  while (i < s.length) {
    const c = s[i];
    const n = s[i + 1];
    if (c === "/" && n === "/") {
      i += 2;
      while (i < s.length && s[i] !== "\n") { out[i] = " "; i++; }
      continue;
    }
    if (c === "/" && n === "*") {
      out[i] = " "; out[i + 1] = " "; i += 2;
      while (i < s.length && !(s[i] === "*" && s[i + 1] === "/")) { out[i] = " "; i++; }
      if (i < s.length) { out[i] = " "; out[i + 1] = " "; i += 2; } // consume closing */
      // NOTE: deliberately do NOT touch lastSig — a comment between `:` and the
      // value must not flip the value into key-position (codex round-4 finding).
      continue;
    }
    if (c === "r" && (n === '"' || n === "#")) {
      const end = rawStringEnd(s, i);
      if (end >= 0) {
        // raw strings are never object keys — blank the whole literal so no
        // `{ token: ... }` inside one is ever read as a key.
        for (let j = i; j < end; j++) out[j] = " ";
        i = end; lastSig = '"'; continue;
      }
    }
    if (c === "'") {
      // char literal — always blank contents
      i++;
      while (i < s.length && s[i] !== "'") {
        if (s[i] === "\\" && i + 1 < s.length) { out[i] = " "; out[i + 1] = " "; i += 2; continue; }
        out[i] = " "; i++;
      }
      i++; lastSig = "'"; continue;
    }
    if (c === '"') {
      const isValue = lastSig === ":"; // value strings get blanked; key strings kept
      i++;
      while (i < s.length && s[i] !== '"') {
        if (s[i] === "\\" && i + 1 < s.length) {
          if (isValue) { out[i] = " "; out[i + 1] = " "; }
          i += 2; continue;
        }
        if (isValue) out[i] = " ";
        i++;
      }
      i++; lastSig = '"'; continue;
    }
    if (c !== " " && c !== "\t" && c !== "\n" && c !== "\r") lastSig = c;
    i++;
  }
  return out.join("");
}

const findings: Finding[] = [];
let emitSites = 0;
let inlineJsonSites = 0;
let uncheckedStructSites = 0;

const EMIT_RE = /\.\s*emit(?:_to|_all|_filter)?\s*\(/g;

for (const root of roots) {
  for (const file of walkRustFiles(root)) {
    const src = readFileSync(file, "utf8");
    EMIT_RE.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = EMIT_RE.exec(src)) !== null) {
      emitSites++;
      const openParen = src.indexOf("(", m.index);
      if (openParen < 0) continue;
      const call = balancedParens(src, openParen);
      // Event NAME (safe to print; never the payload value — codex review: echoing
      // the payload could leak a literal secret into CI logs). The event name is
      // the LAST string literal BEFORE the `json!` payload: for `emit(event, ..)`
      // that's the lone string; for `emit_to(target, event, ..)` it's the 2nd —
      // either way the one immediately preceding the payload. Variable name => placeholder.
      const jsonAt = call.indexOf("json!");
      const prePayload = jsonAt >= 0 ? call.slice(0, jsonAt) : call;
      const preStrings = [...prePayload.matchAll(/"((?:[^"\\]|\\.)*)"/g)].map((x) => x[1]);
      const event = preStrings.length ? preStrings[preStrings.length - 1] : "<dynamic-name>";
      // Is the payload an inline json! macro?
      if (/json!\s*\(\s*\{/.test(call)) {
        inlineJsonSites++;
        // Scan a copy with string VALUES + comments blanked (keys preserved) so
        // forbidden words inside payload values aren't mistaken for object keys.
        const masked = maskStringValues(call);
        // Extract OBJECT KEYS only: an identifier (optionally quoted) that sits
        // immediately after `{` or `,` and is followed by `:`. Anchoring on
        // `[{,]` excludes value-side `Foo::Bar` path segments and `match` arms,
        // which a bare `\bident:` pattern wrongly flagged (claude review).
        const keyRe = /[{,]\s*"?([A-Za-z_][A-Za-z0-9_]*)"?\s*:/g;
        let k: RegExpExecArray | null;
        while ((k = keyRe.exec(masked)) !== null) {
          const key = k[1];
          if (key && FORBIDDEN_RE.test(key)) {
            // Line of the offending KEY itself. k.index points at the leading
            // `{`/`,` (which may be on a previous line for multiline json!), so
            // advance by the key's offset within the match to land on the key.
            const keyAbs = openParen + k.index + k[0].indexOf(key);
            const line = src.slice(0, keyAbs).split("\n").length;
            findings.push({ file, line, key, event });
          }
        }
      } else {
        // Struct-typed or variable payload — not field-resolved in v1.
        uncheckedStructSites++;
      }
    }
  }
}

// ---- report ----
const cwd = process.cwd();
console.log("event-payload-lint (SPEC-17 §13 / G7) — secret-in-event scan");
console.log(
  `  scanned roots: ${roots.join(", ")}  |  emit sites: ${emitSites}` +
    `  (inline json!: ${inlineJsonSites}, struct/var unchecked: ${uncheckedStructSites})`
);
console.log(`  mode: ${strict ? "STRICT (fail on hit)" : "WARN-ONLY (stage 1)"}`);

if (uncheckedStructSites > 0) {
  console.log(
    `  NOTE: ${uncheckedStructSites} struct/variable-typed emit payload(s) are NOT field-resolved in v1 ` +
      `(coverage gap, tracked) — see header.`
  );
}

if (findings.length === 0) {
  console.log("  RESULT: no forbidden secret field names in inline event payloads. OK.");
  process.exit(0);
}

console.log(`  RESULT: ${findings.length} forbidden field(s) in event payloads:`);
for (const f of findings) {
  // Print field name + event name + location ONLY — never the payload value.
  console.log(`    ${relative(cwd, f.file)}:${f.line}  event='${f.event}'  forbidden-field='${f.key}'`);
}

if (strict) {
  console.error(
    "  FAIL (strict): event payloads must not carry plaintext secrets (SPEC-17 §13)."
  );
  process.exit(1);
}
console.log(
  "  WARN-ONLY: not failing the build (stage 1). Triage the list above; flip --strict at GA."
);
process.exit(0);
