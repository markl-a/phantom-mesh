#!/usr/bin/env node
/*
 * Regression self-test for the SPEC-17 lints (event-payload-lint.ts +
 * tauri-cmd-lint.ts). Dependency-free; runs on node (>=23.6 to exec the .ts
 * lints via type-stripping) on any OS incl. Windows.
 *
 * These lints were hardened across many review rounds (string/comment/raw-string
 * lexing, lifetime handling, object-key-only matching, secret-safe output). This
 * harness pins that behavior so a future edit can't silently regress it.
 *
 * Usage: node scripts/qa/lint-selftest.mjs   (exit 0 = all pass, 1 = a failure)
 */

import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const QA_DIR = dirname(fileURLToPath(import.meta.url));
const EP = join(QA_DIR, "event-payload-lint.ts");
const TC = join(QA_DIR, "tauri-cmd-lint.ts");

let pass = 0,
  fail = 0;

// Run a lint; return {code, out}. Never throws on non-zero exit.
function run(script, args) {
  try {
    const out = execFileSync("node", [script, ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    return { code: 0, out };
  } catch (e) {
    return { code: e.status ?? 1, out: (e.stdout || "") + (e.stderr || "") };
  }
}

function fixture(name, content) {
  const dir = mkdtempSync(join(tmpdir(), "lint-selftest-"));
  writeFileSync(join(dir, name), content);
  return dir;
}

function check(label, cond) {
  if (cond) { pass++; console.log(`  PASS  ${label}`); }
  else { fail++; console.log(`  FAIL  ${label}`); }
}

// ── event-payload-lint ──────────────────────────────────────────────────────
console.log("event-payload-lint:");
{
  // Positive: forbidden key in inline json! emit payload -> flagged, --strict=1.
  const d = fixture("a.rs", `fn f(app: AppHandle){ let _=app.emit("e", serde_json::json!({ "access_token": x })); }`);
  const r = run(EP, ["--root", d, "--strict"]);
  check("forbidden key 'access_token' flagged + --strict exits 1", r.code === 1 && /access_token/.test(r.out));
  rmSync(d, { recursive: true, force: true });
}
{
  // Negative: forbidden words only in string VALUE / line comment / raw string / value-side path — none flagged.
  const d = fixture("b.rs", [
    `fn f(app: AppHandle){`,
    `  // a .unwrap() and token in a comment`,
    `  let _=app.emit("e", serde_json::json!({`,
    `    "note": "this { token: x } is text",`,
    `    "raw": r#"weird ) { secret: y }"#,`,
    `    "kind": Foo::Token,`,
    `    "ok": true,`,
    `  }));`,
    `}`,
  ].join("\n"));
  const r = run(EP, ["--root", d]);
  check("value/comment/raw-string/value-path words NOT flagged (exit 0, 0 findings)", r.code === 0 && /no forbidden/i.test(r.out));
  check("secret-safe: payload values not echoed", !/weird \)/.test(r.out));
  rmSync(d, { recursive: true, force: true });
}
{
  // --root with no arg -> exit 2 (guard).
  const r = run(EP, ["--root"]);
  check("--root with no arg exits 2", r.code === 2);
}

// ── tauri-cmd-lint ──────────────────────────────────────────────────────────
console.log("tauri-cmd-lint:");
{
  // R3: non-Result return is a violation; Result<(),String> is INFO not violation.
  const d = fixture("c.rs", [
    `#[tauri::command]`,
    `pub async fn list_things() -> Vec<String> { vec![] }`,
    `#[tauri::command]`,
    `pub async fn do_thing() -> Result<(), String> { Ok(()) }`,
  ].join("\n"));
  const r = run(TC, ["--root", d]);
  check("Vec return flagged R3 violation", /list_things\s+R3/.test(r.out));
  check("Result<(),String> is INFO not violation (unit-paren bug guard)", /\[info\].*do_thing\s+R3/.test(r.out) && !/\bdo_thing\s+R3\b(?!.*transitional)/.test(r.out.replace(/\[info\][^\n]*/g, "")));
  rmSync(d, { recursive: true, force: true });
}
{
  // R4: .unwrap() in comment/string NOT flagged; a real one IS. Lifetimes + generics analyzed.
  const d = fixture("d.rs", [
    `#[tauri::command]`,
    `pub async fn safe_one<'a, R: Runtime>(s: &'a str) -> Result<(), String> {`,
    `  // this .unwrap() lives in a comment`,
    `  let _ = "and .expect( in a string";`,
    `  Ok(())`,
    `}`,
    `#[tauri::command]`,
    `pub async fn bad_one(s: String) -> Result<(), String> { foo().unwrap(); Ok(()) }`,
  ].join("\n"));
  const r = run(TC, ["--root", d]);
  check("comment/string .unwrap()/.expect( NOT flagged R4", !/safe_one\s+R4/.test(r.out));
  check("real .unwrap() flagged R4", /bad_one\s+R4/.test(r.out));
  check("generic+lifetime fn analyzed (commands=2, 0 unanalyzed)", /commands=2\b/.test(r.out) && /unanalyzed=0/.test(r.out));
  rmSync(d, { recursive: true, force: true });
}
{
  // #[tauri::command] inside a /* */ comment must NOT mis-attribute the next fn.
  const d = fixture("e.rs", [
    `/* docs: #[tauri::command] is how you register */`,
    `pub fn not_a_command() -> i32 { 0 }`,
  ].join("\n"));
  const r = run(TC, ["--root", d]);
  check("attr in block comment not mis-attributed (commands=0)", /commands=0\b/.test(r.out));
  rmSync(d, { recursive: true, force: true });
}

// ── summary ─────────────────────────────────────────────────────────────────
console.log("");
console.log(`lint-selftest: ${pass} passed, ${fail} failed`);
process.exit(fail === 0 ? 0 : 1);
