// Static syntax check for the rendered install.ps1.
//
// What it catches: regex literals inside the rendered PowerShell that
// have invalid syntax. The failure mode this exists to gate is the TS
// template-literal escape bug:
//
//   source TS:    -match 'providers\s*=\s*\['
//   rendered PS:  -match 'providerss*=s*['     ← `\s` eaten by JS template
//
// PowerShell is happy with the string literal but blows up at -match
// time with "未結束的 [] 組合". The user only sees the error after
// install.ps1 ran 90% of its job — too late to be useful.
//
// We catch it here by extracting every -match 'X' literal and running
// `new RegExp(X)` (PCRE-ish, close enough to .NET's engine that
// unterminated character classes / unbalanced groups fail in both).
//
// Run: npm run check:install
// Wired into ci-fast.yml as a deploy gate.

import { renderInstallPs1, renderInstallSh } from "../src/routes/dist";

interface RegexFinding {
  source: string;   // "install.ps1" / "install.sh"
  literal: string;  // the inner regex text
  index: number;    // offset into the rendered script
  error: string;
}

function checkPsRegexes(label: string, script: string): RegexFinding[] {
  // PowerShell regex sources:
  //   -match 'X' / -notmatch 'X' / -replace 'X','Y' / Select-String -Pattern 'X'
  // We only chase single-quoted literals (the bug we're guarding against
  // is in single-quoted ones; double-quoted ones get JS-side interpolation
  // and we'd have to handle expansion).
  //
  // Note: this regex is intentionally permissive about what counts as
  // a regex source — false positives are acceptable as long as they're
  // also valid regexes. False NEGATIVES (missing a real bad regex) are
  // what we're trying to avoid.
  const operatorRegex =
    /(?:-match|-notmatch|-replace|-Pattern\s+|Select-String[^']*-Pattern\s+|RegEx[^']*['"])\s*'([^']+)'/g;
  const findings: RegexFinding[] = [];
  let m: RegExpExecArray | null;
  while ((m = operatorRegex.exec(script)) !== null) {
    try {
      // RegExp() will throw on unterminated [] / unbalanced () / etc.
      // .NET regex semantics differ subtly from JS but the structural
      // failures we care about (unterminated classes, unbalanced groups)
      // are caught by both engines.
      // eslint-disable-next-line @typescript-eslint/no-unused-vars
      const _r = new RegExp(m[1]);
    } catch (e) {
      findings.push({
        source: label,
        literal: m[1],
        index:   m.index,
        error:   (e as Error).message,
      });
    }
  }
  return findings;
}

const ps  = renderInstallPs1("https://phantommesh.io");
const sh  = renderInstallSh("https://phantommesh.io");

const psFindings = checkPsRegexes("install.ps1", ps);
const shFindings = checkPsRegexes("install.sh",  sh);
const all = [...psFindings, ...shFindings];

if (all.length > 0) {
  console.error("✗ install script regex check FAILED:\n");
  for (const f of all) {
    // Try to extract a snippet of context around the bad regex.
    const script = f.source === "install.ps1" ? ps : sh;
    const lineStart = script.lastIndexOf("\n", f.index) + 1;
    const lineEnd   = script.indexOf("\n", f.index);
    const line      = script.slice(lineStart, lineEnd === -1 ? undefined : lineEnd);
    const lineNum   = script.slice(0, f.index).split("\n").length;
    console.error(`  ${f.source}:${lineNum}`);
    console.error(`    bad regex: '${f.literal}'`);
    console.error(`    error:     ${f.error}`);
    console.error(`    line:      ${line.trim()}`);
    console.error();
  }
  console.error(`Total: ${all.length} regex literal(s) failed to parse.`);
  console.error();
  console.error("Common cause: TS template literal ate a backslash. Source `\\s` becomes");
  console.error("rendered `s`. Use `\\\\s` in the .ts source to output `\\s` in PS.");
  process.exit(1);
}

const psMatches = (ps.match(/-match\s+'/g) || []).length;
const shMatches = (sh.match(/-match\s+'/g) || []).length;
console.log(`✓ install.ps1: ${psMatches} -match literals all parse`);
console.log(`✓ install.sh:  ${shMatches} -match literals all parse`);
