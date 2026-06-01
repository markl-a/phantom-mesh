// NATIVE-WINDOW E2E — programmatic WebdriverIO (no mocha / tsx / spec-glob).
//
// Drives the REAL Tauri WKWebView window via the tauri-wd bridge. We use the
// programmatic `remote()` API run by plain `node` (which executes .mjs natively)
// to avoid the wdio+mocha+tsx ESM loader bug that mangled the spec file:// URL.
//
// Exit 0 = all assertions passed; non-zero = failure (printed). Prereqs handled
// by scripts/run-native-e2e.sh: vite :5173, tauri-wd :4444, debug app built.
import { remote } from 'webdriverio';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const APP_BINARY = resolve(__dirname, '../../src-tauri/target/debug/phantom-mesh-app');

let failures = 0;
const check = (name, cond, detail = '') => {
  if (cond) {
    console.log(`  ✓ ${name}`);
  } else {
    console.log(`  ✗ ${name}${detail ? ' — ' + detail : ''}`);
    failures++;
  }
};

const browser = await remote({
  hostname: '127.0.0.1',
  port: 4444,
  path: '/',
  logLevel: 'warn',
  capabilities: { 'tauri:options': { binary: APP_BINARY } },
});

try {
  // The plugin injects into the live webview; let the SPA mount.
  await browser.pause(3000);

  const body = await browser.$('body');
  const exists = await body.isExisting();
  check('native window has a <body>', exists);

  const text = exists ? await body.getText() : '';
  check('onboarding shows "Phantom Mesh"', text.includes('Phantom Mesh'),
    `body text (first 120): ${JSON.stringify(text.slice(0, 120))}`);
  check('provider list shows "OpenRouter"', text.includes('OpenRouter'));
  check('provider list shows "Anthropic"', text.includes('Anthropic'));
} catch (e) {
  console.log(`  ✗ driver error: ${e.message}`);
  failures++;
} finally {
  await browser.deleteSession().catch(() => {});
}

console.log(`\nNATIVE-WDIO: ${failures === 0 ? 'PASS' : 'FAIL'} (${failures} failed assertion(s))`);
process.exit(failures === 0 ? 0 : 1);
