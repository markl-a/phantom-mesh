#!/usr/bin/env node
// WebView CDP smoke for the phantom-mesh Tauri Android app.
//
// WHY: the app is a Tauri WebView, so `uiautomator dump` shows NO DOM text
// (the WebView is opaque) and `adb shell input tap x,y` is flaky. The reliable
// way to drive/verify the real UI is the Chrome DevTools Protocol (CDP), which
// debug builds expose as an abstract unix socket `@webview_devtools_remote_<pid>`.
// This complements scripts/smoke-android-emulator.sh: that one proves the NATIVE
// side (process alive, MeshNodeService foreground, phantom:// deep-link); this
// one proves the WEBVIEW side (DOM reachable, not a blank/white screen, and —
// when --expr is given — that a real DOM path runs through safeInvoke→native).
//
// Requires: a debug APK already installed + launched on a connected device, and
// Node ≥ 22 (global fetch + global WebSocket). No npm deps.
//
// Usage:
//   node app/tests/android/webview-cdp-smoke.mjs
//   node app/tests/android/webview-cdp-smoke.mjs --pkg ai.phantommesh.app --port 9222
//   node app/tests/android/webview-cdp-smoke.mjs --expr "document.title"
//   node app/tests/android/webview-cdp-smoke.mjs --testid mesh-status   # assert a [data-testid] exists
//
// Exit 0 = PASS (WebView reachable + non-blank, and any --expr/--testid held);
// exit 1 = FAIL; exit 2 = setup error (no device / no debug socket / Node too old).

import { execSync, execFileSync } from "node:child_process";

// ── args ──────────────────────────────────────────────────────────────────
const argv = process.argv.slice(2);
const arg = (name, def) => {
  const i = argv.indexOf(name);
  return i >= 0 && i + 1 < argv.length ? argv[i + 1] : def;
};
const PKG = arg("--pkg", "ai.phantommesh.app");
const PORT = arg("--port", "9222");
const EXPR = arg("--expr", null); // arbitrary probe; must be truthy to pass
const TESTID = arg("--testid", null); // assert document.querySelector([data-testid=..]) exists
let ADB = arg("--adb", process.env.ADB || "adb");

const fail = (msg) => { console.error(`❌ CDP SMOKE FAIL: ${msg}`); process.exit(1); };
const setup = (msg) => { console.error(`⚠  setup: ${msg}`); process.exit(2); };
const ok = (msg) => console.log(`✓ ${msg}`);

if (typeof WebSocket === "undefined") setup("global WebSocket missing — need Node ≥ 22 (v24 recommended)");

// Raw shell string form — kept ONLY for the fixed `adb forward` cmd below where
// no untrusted/user-supplied value is interpolated.
const sh = (cmd) => execSync(cmd, { stdio: ["ignore", "pipe", "pipe"] }).toString().trim();
// Argv-based form — no shell, so testid / package names / paths can't break out.
const adb = (...args) => execFileSync(ADB, args, { stdio: ["ignore", "pipe", "pipe"] }).toString().trim();

// ── 1. resolve the app pid + its webview debug socket ───────────────────────
let pid;
try {
  // `pidof` may return multiple pids (e.g. a separate :sandboxed_process).
  // Keep ALL of them — the WebView socket can be owned by any process in the
  // app's group, but it must be one of OURS, never a stray system webview.
  pid = adb("shell", "pidof", PKG).split(/\s+/).filter(Boolean);
} catch {
  setup(`adb not runnable or no device — is an emulator/device connected? (ADB=${ADB})`);
}
if (!pid || pid.length === 0) fail(`${PKG} is not running — install + launch it first (scripts/smoke-android-emulator.sh)`);
ok(`app pid ${pid.join(",")}`);

// The abstract socket is named `webview_devtools_remote_<pid>` where <pid> is
// the process that opened the WebView. B1 FIX: do NOT grab the FIRST matching
// socket — on a device with other Chrome/WebView apps (or a stale socket) that
// silently smoke-tests the WRONG app and reports false-green. Require the pid
// suffix to belong to one of OUR resolved pids.
let sock;
const pidSet = new Set(pid);
try {
  const unix = adb("shell", "cat", "/proc/net/unix");
  const socks = unix
    .split("\n")
    .map((l) => {
      const m = l.match(/@(webview_devtools_remote_(\d+))\b/);
      return m ? { name: m[1], pid: m[2] } : null;
    })
    .filter(Boolean);
  if (socks.length === 0) {
    fail("no @webview_devtools_remote_* socket — is this a DEBUG build? (release strips CDP)");
  }
  const mine = socks.find((s) => pidSet.has(s.pid));
  if (!mine) {
    fail(
      `no webview_devtools_remote_<pid> socket matched our pid(s) [${pid.join(",")}] — ` +
        `found sockets for pid(s) [${socks.map((s) => s.pid).join(",")}], all belonging to OTHER processes. ` +
        `Refusing to smoke a foreign WebView (false-green guard).`,
    );
  }
  sock = mine.name;
} catch (e) {
  // execFileSync throws on non-zero exit; surface the real cause but don't let a
  // failed `cat` masquerade as "socket present".
  if (typeof sock === "undefined") fail(`could not read /proc/net/unix: ${e.message}`);
}
ok(`webview socket ${sock} (owned by our pid)`);

// ── 2. forward + discover the page target ───────────────────────────────────
try { adb("forward", `tcp:${PORT}`, `localabstract:${sock}`); } catch (e) { fail(`adb forward failed: ${e.message}`); }
ok(`forwarded tcp:${PORT} -> localabstract:${sock}`);

const cleanup = () => { try { adb("forward", "--remove", `tcp:${PORT}`); } catch {} };

let wsUrl;
try {
  // AbortController timeout — a wedged CDP endpoint must NOT hang the smoke
  // forever (would otherwise stall CI / appear to pass-by-timeout).
  const ac = new AbortController();
  const fetchTimer = setTimeout(() => ac.abort(), 10000);
  let res;
  try {
    res = await fetch(`http://localhost:${PORT}/json/list`, { signal: ac.signal });
  } finally {
    clearTimeout(fetchTimer);
  }
  const targets = await res.json();
  const page = targets.find((t) => t.type === "page" && t.webSocketDebuggerUrl) || targets.find((t) => t.webSocketDebuggerUrl);
  if (!page) { cleanup(); fail("no debuggable page target on the CDP endpoint"); }
  wsUrl = page.webSocketDebuggerUrl;
  ok(`page target: ${page.url || "(blank url)"}`);
} catch (e) { cleanup(); fail(`/json/list failed: ${e.message}`); }

// ── 3. CDP session: Runtime.enable, then evaluate probes ────────────────────
const ws = new WebSocket(wsUrl);
let nextId = 1;
const pending = new Map();
const send = (method, params = {}) =>
  new Promise((resolve, reject) => {
    const id = nextId++;
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, method, params }));
  });
ws.addEventListener("message", (ev) => {
  let m;
  try { m = JSON.parse(ev.data); } catch { return; }
  if (m.id && pending.has(m.id)) {
    const { resolve, reject } = pending.get(m.id);
    pending.delete(m.id);
    m.error ? reject(new Error(m.error.message)) : resolve(m.result);
  }
});

const evaluate = async (expression) => {
  const r = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || "JS exception");
  return r.result.value;
};

const result = { pkg: PKG, pid: pid.join(","), socket: sock };
const timeout = setTimeout(() => { cleanup(); fail("CDP connection timed out after 15s"); }, 15000);

ws.addEventListener("error", (e) => { clearTimeout(timeout); cleanup(); fail(`websocket error: ${e.message || e.type}`); });

ws.addEventListener("open", async () => {
  try {
    await send("Runtime.enable");

    // core "is the UI actually rendered" probe — guards against white screen
    result.title = await evaluate("document.title");
    result.href = await evaluate("location.href");
    result.bodyTextLen = await evaluate("(document.body && document.body.innerText || '').trim().length");
    result.buttonCount = await evaluate("document.querySelectorAll('button').length");
    result.testidCount = await evaluate("document.querySelectorAll('[data-testid]').length");

    if (result.bodyTextLen < 1 && result.buttonCount < 1) {
      clearTimeout(timeout); cleanup();
      fail(`WebView rendered a blank screen (no text, no buttons) — title=${JSON.stringify(result.title)}`);
    }
    ok(`DOM live: title=${JSON.stringify(result.title)} buttons=${result.buttonCount} testids=${result.testidCount} bodyLen=${result.bodyTextLen}`);

    if (TESTID) {
      // JSON.stringify the testid so quotes/brackets in a user-supplied value
      // can't break out of the selector string (no raw interpolation).
      const sel = `[data-testid=${JSON.stringify(TESTID)}]`;
      const found = await evaluate(`!!document.querySelector(${JSON.stringify(sel)})`);
      if (!found) { clearTimeout(timeout); cleanup(); fail(`[data-testid="${TESTID}"] not found in DOM`); }
      ok(`[data-testid="${TESTID}"] present`);
      result.testid = TESTID;
    }

    if (EXPR) {
      const v = await evaluate(EXPR);
      result.expr = { source: EXPR, value: v };
      if (!v) { clearTimeout(timeout); cleanup(); fail(`--expr returned falsy: ${EXPR} => ${JSON.stringify(v)}`); }
      ok(`--expr ok: ${EXPR} => ${JSON.stringify(v)}`);
    }

    clearTimeout(timeout);
    cleanup();
    console.log("\n" + JSON.stringify(result, null, 2));
    console.log("\n✅ CDP SMOKE PASS — WebView reachable + rendered");
    ws.close();
    process.exit(0);
  } catch (e) {
    clearTimeout(timeout); cleanup(); fail(e.message);
  }
});
