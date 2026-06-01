import { defineConfig, devices } from '@playwright/test';

// Playwright config for the web-shell E2E of the Tauri frontend.
//
// macOS has no WKWebView WebDriver, so native-window E2E (tauri-driver) is not
// available here. The Tauri app embeds the same React UI that Vite serves, so we
// drive that UI in real headless browser engines instead. Per Tauri's testing
// guidance, the macOS/Linux runtime is WebKit — so we include a `webkit` project
// (Playwright's WebKit is closest to WKWebView; not identical, but far closer
// than Chromium). `chromium` is kept as a fast cross-check.
//
// Scope: this validates the frontend (render, navigation, layout, console
// health) — NOT Tauri IPC / menus / windowing (those are covered by the headless
// vitest IPC-contract + lifecycle tests). See docs/e2e-mac-real-testing.md.
export default defineConfig({
  // CRITICAL: only the e2e/ dir. Without this, the default testMatch sweeps the
  // whole repo and tries to run the vitest *.test.tsx files as Playwright tests,
  // which crashes (vitest vs playwright expect collision).
  testDir: './e2e',
  testMatch: '**/*.spec.ts',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: [['list']],
  use: {
    baseURL: 'http://localhost:5173',
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
    // WebKit — closest engine to the macOS WKWebView the Tauri app actually uses.
    { name: 'webkit', use: { ...devices['Desktop Safari'] } },
  ],
  // Auto-start the Vite dev server the specs target. reuseExistingServer so a
  // dev server you already have running is used instead of a second one.
  webServer: {
    command: 'npx vite --port 5173 --strictPort',
    url: 'http://localhost:5173',
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
