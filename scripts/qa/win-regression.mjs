#!/usr/bin/env node
/*
 * Windows live-regression harness: re-checks the behaviors found by the
 * 2026-05-29 Windows usage-matrix against a running serve, so the dev loop can
 * auto-detect when an owner FIXES a routed bug (red -> green) without re-running
 * the whole 7-agent workflow. Dependency-free (node fetch + child_process).
 *
 * Two test classes:
 *   GREEN-GUARD : things that work today and must STAY working (fail = regression)
 *   BUG-WATCH   : known-broken items (T-AUTH-LOCAL etc.) — reported but do NOT
 *                 fail the run, so this stays usable in CI as a monitor until the
 *                 owner fixes them; pass --strict to make BUG-WATCH failures count.
 *
 * Usage: node scripts/qa/win-regression.mjs [base-url] [--strict]
 * Exit 0 = all green-guards pass (and, with --strict, all bug-watch fixed).
 */
import { spawnSync } from "node:child_process";
import { mkdtempSync, existsSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const args = process.argv.slice(2);
const strict = args.includes("--strict");
const BASE = args.find((a) => a.startsWith("http")) || "http://127.0.0.1:7878";
const PH = process.env.PHANTOM_BIN || "C:/Users/m4932/.local/bin/phantom";

let greenFail = 0, bugOpen = 0, bugFixed = 0;
const log = (s) => console.log(s);

async function http(method, path, body) {
  try {
    const r = await fetch(`${BASE}${path}`, {
      method,
      headers: body ? { "Content-Type": "application/json" } : undefined,
      body: body ? JSON.stringify(body) : undefined,
      signal: AbortSignal.timeout(8000),
    });
    return { code: r.status, text: await r.text().catch(() => "") };
  } catch (e) { return { code: 0, text: String(e.message) }; }
}
function cli(a, timeoutMs = 15000) {
  // spawnSync captures BOTH stdout+stderr (phantom prints status to stderr).
  const r = spawnSync(PH, a, { encoding: "utf8", timeout: timeoutMs });
  return { code: r.status ?? 1, out: (r.stdout || "") + (r.stderr || "") };
}
const green = (label, ok, ev) => { if (ok) log(`  GREEN ok   ${label}`); else { greenFail++; log(`  GREEN FAIL ${label}  <- ${ev}`); } };
const bug = (id, fixed, ev) => { if (fixed) { bugFixed++; log(`  BUG FIXED  ${id}  (${ev}) -- re-verify + tell owner!`); } else { bugOpen++; log(`  BUG open   ${id}  (${ev})`); } };

const main = async () => {
  log(`win-regression @ ${BASE}`);
  // --- GREEN-GUARD: must stay working ---
  green("serve /healthz", (await http("GET", "/healthz")).code === 200, "healthz");
  green("/api/status 200", (await http("GET", "/api/status")).code === 200, "status");
  green("/api/projects 200", (await http("GET", "/api/projects")).code === 200, "projects");
  green("/m UI 200", (await http("GET", "/m")).code === 200, "m-ui");
  green("/ws auth-gated (401)", (await http("GET", "/ws")).code === 401, "ws");
  { const r = cli(["--version"]); green("phantom --version", r.code === 0 && /0\.6\.0/.test(r.out), r.out.trim().slice(0, 60)); }
  { const r = cli(["cluster", "status"], 20000); green("phantom cluster status", /peer|reachable|node/i.test(r.out), r.out.replace(/\s+/g, " ").slice(0, 60)); }

  // --- BUG-WATCH: known broken, auto-detect the fix ---
  { const r = await http("POST", "/api/chat", { message: "ping" }); bug("T-AUTH-LOCAL /api/chat", !(r.code === 401 && /X-Cluster-Auth/.test(r.text)), `code=${r.code}`); }
  { const who = cli(["whoami"], 6000); const loggedOut = /not logged in|尚未登入/.test(who.out);
    const r = cli(["sessions"], 12000);
    // `sessions` exit code flips with auth state: logged-out exits 0 with "not logged in",
    // logged-in hits the broker and 401s (exit 1). Only claim FIXED when logged IN — otherwise
    // a logout would masquerade as a fix (false-positive observed 2026-05-30, see done/ note).
    const fixed = !loggedOut && !(r.code !== 0 && /401|unauthenticated/.test(r.out));
    bug("T-AUTH-LOCAL sessions", fixed, loggedOut ? `code=${r.code} logged-out:inconclusive — re-test after login` : `code=${r.code}`); }
  { const r = cli(["focus", "status"], 8000); bug("focus(SPEC-21) wired", !/尚未接線|not.*wired|Stage 4/i.test(r.out) && r.code === 0, `code=${r.code}`); }
  { const r = cli(["skill", "run", "--help"], 8000); bug("skill-run hermes feature", !/experimental-hermes-curator|built without/i.test(r.out), "feature gate"); }
  // SPEC-46 exec-plan watches (auto-detect when core fixes them):
  { const r = cli(["--version"], 8000); bug("SPEC-46 I8 --version built-date", !/built unknown/i.test(r.out), "build.rs date -u on Windows"); }
  { const r = cli(["selftest", "--json", "--p0-only"], 30000); bug("SPEC-46 I10 selftest --json output", (r.out || "").trim().length > 0, `bytes=${(r.out||"").trim().length}`); }
  // SPEC-46 I1 watch: `init --help` must be inert (read-only, no scaffold). Run in a throwaway
  // temp cwd so a still-broken `init` scaffolds into temp, never the repo's tracked PHANTOM.md.
  { const tmp = mkdtempSync(join(tmpdir(), "ph-i1-"));
    spawnSync(PH, ["init", "--help"], { cwd: tmp, encoding: "utf8", timeout: 8000 });
    const scaffolded = existsSync(join(tmp, "PHANTOM.md"));
    bug("SPEC-46 I1 init --help inert", !scaffolded, scaffolded ? "scaffolded PHANTOM.md" : "inert");
    try { rmSync(tmp, { recursive: true, force: true }); } catch {} }

  log(`\nwin-regression: green-fails=${greenFail}, bugs-open=${bugOpen}, bugs-FIXED=${bugFixed}`);
  const fail = greenFail > 0 || (strict && bugOpen > 0);
  process.exit(fail ? 1 : 0);
};
main().catch((e) => { console.error("[error]", e.message); process.exit(2); });
