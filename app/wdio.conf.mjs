// WebdriverIO config for NATIVE-WINDOW E2E of the Tauri app on macOS.
//
// macOS has no system WKWebView WebDriver, so we drive the REAL native window
// via the tauri-wd bridge (cargo install tauri-webdriver-automation) talking to
// the in-app tauri-plugin-webdriver-automation (registered debug-only in
// src-tauri/lib.rs). Flow:
//   wdio (:4444) -> tauri-wd -> spawns the debug app -> plugin server (dynamic)
//
// Prereqs (the runner script scripts/run-native-e2e.sh sets these up):
//   1. vite dev server on :5173 (the debug app loads its frontend from devUrl)
//   2. tauri-wd --port 4444
//   3. a freshly-built debug binary WITH the plugin
//
// See docs/e2e-app-native-webdriver.md.
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
// Absolute path to the debug app binary (tauri-wd spawns this).
const APP_BINARY = resolve(__dirname, 'src-tauri/target/debug/spectyn-mesh-app');

export const config = {
  runner: 'local',
  hostname: '127.0.0.1',
  port: 4444,
  path: '/',
  specs: ['./tests/wdio/**/*.e2e.mjs'],
  maxInstances: 1,
  capabilities: [
    {
      // Custom Tauri capability — tells tauri-wd which binary to launch.
      'tauri:options': { binary: APP_BINARY },
    },
  ],
  logLevel: 'warn',
  framework: 'mocha',
  // 'dot' is bundled with @wdio/cli (no extra install); 'spec' would need a
  // separate @wdio/spec-reporter package.
  reporters: ['dot'],
  mochaOpts: { ui: 'bdd', timeout: 60000 },
};
