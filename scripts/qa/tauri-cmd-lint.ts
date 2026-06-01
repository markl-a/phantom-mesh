#!/usr/bin/env node
/*
 * SPEC-17 §7.2 / §8 — Tauri command lint.
 *
 * Big Goal: this is infra/CI that enforces the P3-facing command contract so the
 * Tauri bridge (SPEC-17) doesn't drift. Scans Rust `#[tauri::command]` functions
 * and checks the rules a regex/brace scan can verify with HIGH CONFIDENCE.
 *
 * STAGE (SPEC-17 §migration): stage 1 = WARN-ONLY. Exits 0 unless --strict.
 *
 * RULES (deliberately scoped to low-false-positive checks):
 *   R1  name must be snake_case — FLAG any uppercase letter (mixedCase/camelCase).
 *       NOTE: SPEC-17 §8 also wants a verb component (<domain>_<verb> / <verb>_<noun>).
 *       That is NOT enforced here: distinguishing verb vs noun needs an English
 *       lexicon and any allowlist over-flags real verbs (register/navigate/record/
 *       analyze/sign/...). Verb-prefix enforcement is DEFERRED to a curated pass —
 *       reported honestly rather than guessed. (This avoids the ~35 false positives
 *       an allowlist produced.)
 *   R2  must be `pub fn` or `pub async fn`. Non-pub => violation; non-async => INFO
 *       (async preferred, not required).
 *   R3  return type must be `Result<...>`. `Result<_, String>` => INFO (transitional
 *       per §3.1 G2); a non-Result return => violation.
 *   R4  command body must not contain `.unwrap()` / `.expect(` / `panic!(` (best
 *       effort textual scan of the fn body) => violation.
 *
 * Association is anchored: after `#[tauri::command]` we skip ONLY trivia (other
 * attributes, doc/line/block comments, whitespace) to reach the fn — so private
 * helper fns nearby are never mis-read as commands. If no fn is found in the
 * trivia window, the site is reported as "unanalyzed" (honest coverage), never
 * silently attributed to the wrong fn.
 *
 * Dependency-free; runs on node >=23.6 (type stripping) or `npx tsx`.
 * Secret-safe: prints only fn name + file:line + rule — never source values.
 *
 * Usage:
 *   node scripts/qa/tauri-cmd-lint.ts            # warn-only
 *   node scripts/qa/tauri-cmd-lint.ts --strict   # exit 1 on any violation
 *   node scripts/qa/tauri-cmd-lint.ts --root <dir>
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const DEFAULT_ROOTS = ["app/src-tauri/src", "core/src"];

const argv = process.argv.slice(2);
const strict = argv.includes("--strict") || process.env.TAURI_CMD_LINT_STRICT === "1";
const rootIdx = argv.indexOf("--root");
if (rootIdx >= 0 && (!argv[rootIdx + 1] || argv[rootIdx + 1].startsWith("--"))) {
  console.error("usage: --root <dir> requires a directory argument");
  process.exit(2);
}
const roots = rootIdx >= 0 ? [argv[rootIdx + 1]] : DEFAULT_ROOTS;

type Sev = "violation" | "info";
interface Finding { file: string; line: number; fn: string; rule: string; sev: Sev; reason: string; }
const findings: Finding[] = [];
const unanalyzed: Array<{ file: string; line: number }> = [];
let commandCount = 0;

function walkRust(dir: string): string[] {
  let out: string[] = [];
  let entries: string[];
  try { entries = readdirSync(dir); } catch { return out; }
  for (const name of entries) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) {
      if (name === "target" || name === "node_modules") continue;
      out = out.concat(walkRust(p));
    } else if (name.endsWith(".rs")) out.push(p);
  }
  return out;
}

// From index `i`, skip trivia (whitespace, #[..] attributes, // and /* */ comments)
// and return the index of the next real token, or -1 if we run past `limit`.
function skipTrivia(s: string, i: number, limit: number): number {
  while (i < limit) {
    const c = s[i];
    if (c === " " || c === "\t" || c === "\r" || c === "\n") { i++; continue; }
    if (c === "/" && s[i + 1] === "/") { while (i < limit && s[i] !== "\n") i++; continue; }
    if (c === "/" && s[i + 1] === "*") { i += 2; while (i < limit && !(s[i] === "*" && s[i + 1] === "/")) i++; i += 2; continue; }
    if (c === "#" && s[i + 1] === "[") {
      // skip balanced [...]
      i += 1; let depth = 0;
      do { if (s[i] === "[") depth++; else if (s[i] === "]") depth--; i++; } while (i < limit && depth > 0);
      continue;
    }
    return i;
  }
  return -1;
}

// Return the substring spanning balanced braces starting at the `{` at `open`.
function balancedBraces(s: string, open: number): string {
  let depth = 0, inStr = false, inLine = false, inBlock = false;
  for (let i = open; i < s.length; i++) {
    const c = s[i], n = s[i + 1];
    if (inLine) { if (c === "\n") inLine = false; continue; }
    if (inBlock) { if (c === "*" && n === "/") { inBlock = false; i++; } continue; }
    if (inStr) { if (c === "\\") i++; else if (c === '"') inStr = false; continue; }
    if (c === "/" && n === "/") { inLine = true; i++; continue; }
    if (c === "/" && n === "*") { inBlock = true; i++; continue; }
    if (c === "r" && (n === '"' || n === "#") && !/[A-Za-z0-9_]/.test(s[i - 1])) {
      // raw string r#"..."# may contain `"` and `}` — skip it wholesale so a
      // `}` inside it can't prematurely close the body (codex finding).
      let j = i + 1, hashes = 0;
      while (s[j] === "#") { hashes++; j++; }
      if (s[j] === '"') {
        const close = '"' + "#".repeat(hashes);
        const end = s.indexOf(close, j + 1);
        i = (end < 0 ? s.length : end + close.length) - 1; // loop i++ advances past
        continue;
      }
    }
    if (c === '"') { inStr = true; continue; }
    if (c === "'") {
      // char literal ('x' / '\n') — skip its contents; a lifetime ('a) is ignored
      // (no near closing quote) so it can't swallow a later '}' (lifetime fix).
      if (n === "\\" && s[i + 3] === "'") { i += 3; continue; }
      if (n !== "\\" && s[i + 2] === "'") { i += 2; continue; }
      continue;
    }
    if (c === "{") depth++;
    else if (c === "}") { depth--; if (depth === 0) return s.slice(open, i + 1); }
  }
  return s.slice(open);
}

// Blank the CONTENTS of string/char literals (incl. raw strings) and comments,
// keeping length, so R4's `.unwrap()`/`.expect(`/`panic!` scan can't false-match
// text inside a comment or string literal (QA-sweep + codex finding).
function stripCommentsStrings(s: string): string {
  const out = s.split("");
  let i = 0;
  while (i < s.length) {
    const c = s[i], n = s[i + 1];
    if (c === "/" && n === "/") { while (i < s.length && s[i] !== "\n") { out[i] = " "; i++; } continue; }
    if (c === "/" && n === "*") { out[i] = " "; out[i + 1] = " "; i += 2; while (i < s.length && !(s[i] === "*" && s[i + 1] === "/")) { out[i] = " "; i++; } if (i < s.length) { out[i] = " "; out[i + 1] = " "; i += 2; } continue; }
    if (c === "r" && (n === '"' || n === "#") && (i === 0 || !/[A-Za-z0-9_]/.test(s[i - 1]))) {
      // raw string r#"..."#
      let j = i + 1, hashes = 0;
      while (s[j] === "#") { hashes++; j++; }
      if (s[j] === '"') {
        const close = '"' + "#".repeat(hashes);
        const end = s.indexOf(close, j + 1);
        const stop = end < 0 ? s.length : end + close.length;
        for (let k = i; k < stop; k++) out[k] = " ";
        i = stop; continue;
      }
    }
    if (c === '"') { out[i] = " "; i++; while (i < s.length && s[i] !== '"') { if (s[i] === "\\") { out[i] = " "; i++; } if (i < s.length) out[i] = " "; i++; } if (i < s.length) { out[i] = " "; i++; } continue; }
    if (c === "'") {
      // Distinguish a CHAR LITERAL ('x' / '\n') from a Rust LIFETIME ('a, 'static).
      // Only a char literal closes within 1 char (or 2 with an escape); a lifetime
      // has no near closing quote — treating it as a string would consume to the
      // next ' anywhere (e.g. another lifetime) and blank real code (false neg).
      if (s[i + 1] === "\\" && s[i + 3] === "'") { out[i + 1] = " "; out[i + 2] = " "; i += 4; continue; }
      if (s[i + 1] !== "\\" && s[i + 2] === "'") { out[i + 1] = " "; i += 3; continue; }
      i++; continue; // lifetime / stray quote → ordinary code
    }
    i++;
  }
  return out.join("");
}

const ATTR_RE = /#\s*\[\s*tauri\s*::\s*command\b[^\]]*\]/g;
// optional generic clause `<R: tauri::Runtime, ...>` between name and `(` so
// generic command fns aren't dropped as "unanalyzed" (false negative).
const FN_DECL_RE = /^(pub\b(?:\s*\([^)]*\))?\s+)?(async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:<[^()]*>)?\s*\(/;

for (const root of roots) {
  for (const file of walkRust(root)) {
    const src = readFileSync(file, "utf8");
    // Comment/string-stripped copy: a `#[tauri::command]` mentioned inside a
    // // line comment OR a /* */ block comment OR a string is blanked here, so
    // we skip it (otherwise we'd mis-attribute the next real fn). Handles both
    // comment kinds, replacing the earlier line-only guard.
    const srcStripped = stripCommentsStrings(src);
    ATTR_RE.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = ATTR_RE.exec(src)) !== null) {
      if (srcStripped[m.index] !== "#") { continue; } // attr was inside a comment/string
      const attrEnd = m.index + m[0].length;
      const attrLine = src.slice(0, m.index).split("\n").length;
      // skip trivia (incl. further attributes) within a generous window to the fn
      const tokenAt = skipTrivia(src, attrEnd, Math.min(src.length, attrEnd + 4000));
      if (tokenAt < 0) { unanalyzed.push({ file, line: attrLine }); continue; }
      const decl = FN_DECL_RE.exec(src.slice(tokenAt, tokenAt + 600));
      if (!decl) { unanalyzed.push({ file, line: attrLine }); continue; }
      commandCount++;
      const isPub = !!decl[1];
      const isAsync = !!decl[2];
      const name = decl[3];
      const fnLine = src.slice(0, tokenAt).split("\n").length;
      const add = (rule: string, sev: Sev, reason: string) =>
        findings.push({ file, line: fnLine, fn: name, rule, sev, reason });

      // R1 — snake_case (flag any uppercase = mixedCase/camelCase)
      if (/[A-Z]/.test(name)) add("R1", "violation", "command name is not snake_case (mixedCase/camelCase)");

      // R2 — visibility / async
      if (!isPub) add("R2", "violation", "command fn must be pub");
      else if (!isAsync) add("R2", "info", "command fn is not async (async preferred)");

      // signature region: from end of decl match, find param-close `)` then the
      // return arrow up to the body `{`.
      const declAbs = tokenAt + decl[0].length - 1; // at the `(`
      const sigSlice = src.slice(declAbs);
      // Find the return type + body. retStart is fixed ONCE when the PARAMETER
      // list closes; parens that appear later inside the return type (e.g. the
      // `()` in `Result<(), String>`) must NOT move it — that bug made unit-
      // bearing Result returns look non-Result (false R3).
      let pdepth = 0, bodyRel = -1, retStart = -1, paramsClosed = false;
      for (let i = 0; i < sigSlice.length; i++) {
        const c = sigSlice[i];
        if (c === "(") pdepth++;
        else if (c === ")") { pdepth--; if (!paramsClosed && pdepth === 0) { paramsClosed = true; retStart = i + 1; } }
        else if (c === "{" && paramsClosed && pdepth === 0) { bodyRel = i; break; }
        else if (c === ";" && paramsClosed && pdepth === 0) { break; } // trait/extern decl, no body
      }
      const retType = retStart >= 0 && bodyRel >= 0 ? sigSlice.slice(retStart, bodyRel) : "";
      // R3 — return must be Result<...>
      if (bodyRel >= 0) {
        if (!/->\s*Result\s*</.test(retType)) {
          add("R3", "violation", "command should return Result<T, CommandError> (or transitional Result<T, String>)");
        } else if (/->\s*Result\s*<[^>]*,\s*String\s*>/.test(retType)) {
          add("R3", "info", "Result<T, String> is transitional; prefer Result<T, CommandError>");
        }
        // R4 — no unwrap/expect/panic in body. Strip comments+strings first so
        // an `.unwrap()` in a comment or string literal isn't a false positive;
        // whitespace-tolerant (`.unwrap ()` is valid Rust).
        const body = stripCommentsStrings(balancedBraces(sigSlice, bodyRel));
        if (/\.unwrap\s*\(\s*\)|\.expect\s*\(|panic!\s*\(/.test(body)) {
          add("R4", "violation", "command body contains .unwrap()/.expect()/panic! — use CommandError");
        }
      }
    }
  }
}

// ---- report ----
const cwd = process.cwd();
const violations = findings.filter((f) => f.sev === "violation");
const infos = findings.filter((f) => f.sev === "info");
console.log("tauri-cmd-lint (SPEC-17 §7.2/§8) — command contract check");
console.log(`  scanned roots: ${roots.join(", ")}  |  #[tauri::command] fns: ${commandCount}`);
console.log(`  mode: ${strict ? "STRICT (fail on violation)" : "WARN-ONLY (stage 1)"}`);
console.log("  NOTE: R1 enforces snake_case only; verb-prefix (§8) is deferred (allowlist over-flags real verbs).");
if (unanalyzed.length) {
  console.log(`  NOTE: ${unanalyzed.length} #[tauri::command] site(s) had no analyzable fn within the trivia window (honest coverage gap):`);
  for (const u of unanalyzed) console.log(`        ${relative(cwd, u.file)}:${u.line}`);
}
for (const f of violations) console.log(`  ${relative(cwd, f.file)}:${f.line}  fn=${f.fn}  ${f.rule}  ${f.reason}`);
for (const f of infos) console.log(`  [info] ${relative(cwd, f.file)}:${f.line}  fn=${f.fn}  ${f.rule}  ${f.reason}`);
console.log(`  summary: commands=${commandCount} violations=${violations.length} info=${infos.length} unanalyzed=${unanalyzed.length}`);

if (violations.length === 0) { console.log("  RESULT: no command-contract violations. OK."); process.exit(0); }
if (strict) { console.error(`  FAIL (strict): ${violations.length} violation(s).`); process.exit(1); }
console.log("  WARN-ONLY: not failing the build (stage 1). Triage the violations above; flip --strict at GA.");
process.exit(0);
