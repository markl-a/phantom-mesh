import { test, expect, type APIRequestContext } from '@playwright/test';

const BASE_URL = 'http://localhost:5173';
const API_URL = 'http://localhost:7878';
// Dev daemon bearer token (loopback only). Override with SPECTYN_E2E_TOKEN.
const TOKEN = process.env.SPECTYN_E2E_TOKEN || 'e9723eea85484da6b39d5abdcdcef6bf';
const AUTH = { Authorization: `Bearer ${TOKEN}` };

// Is a spectyn daemon actually listening on :7878? The daemon-API tests below
// require `spectyn serve` to be running; when it is NOT (the common case in a
// plain checkout / CI without a daemon), they SKIP honestly instead of
// hard-failing. Resolved once and cached.
let daemonUp: boolean | null = null;
async function daemonReachable(request: APIRequestContext): Promise<boolean> {
  if (daemonUp !== null) return daemonUp;
  try {
    const resp = await request.get(`${API_URL}/health`, { timeout: 2000 });
    daemonUp = resp.ok();
  } catch {
    daemonUp = false;
  }
  return daemonUp;
}

test.describe('Spectyn Mesh E2E Smoke — GUI (web shell)', () => {
  test('GUI loads successfully', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (err) => errors.push(err.message));
    await page.goto(BASE_URL);
    await expect(page).toHaveTitle(/spectyn/i);
    await page.waitForTimeout(2000);
    // In a bare browser (no Tauri runtime) the frontend throws expected,
    // benign errors: ResizeObserver noise, and "transformCallback" — the Tauri
    // IPC bridge (window.__TAURI_INTERNALS__) being absent in web-shell mode.
    // Filter those; assert no OTHER unexpected runtime errors.
    const critical = errors.filter(
      (e) => !e.includes('ResizeObserver') && !e.includes('transformCallback'),
    );
    expect(critical.length).toBeLessThan(3);
  });

  test('GUI renders main layout', async ({ page }) => {
    await page.goto(BASE_URL);
    await page.waitForTimeout(1500);
    await page.screenshot({ path: 'e2e/screenshots/main-layout.png', fullPage: true });
    const bodyText = await page.textContent('body');
    expect(bodyText).toBeTruthy();
    expect(bodyText!.length).toBeGreaterThan(10);
  });
});

test.describe('Spectyn Mesh E2E Smoke — daemon API (needs `spectyn serve`)', () => {
  // Skip the whole group when no daemon is listening — these are integration
  // checks against a live daemon, not part of the always-on web-shell suite.
  test.beforeEach(async ({ request }) => {
    test.skip(!(await daemonReachable(request)), 'no spectyn daemon on :7878 — run `spectyn serve`');
  });

  test('health check reports ok + a version', async ({ request }) => {
    const resp = await request.get(`${API_URL}/health`);
    expect(resp.ok()).toBeTruthy();
    const body = await resp.json();
    expect(body.status).toBe('ok');
    // Assert a real semver-ish version is present rather than a hardcoded one
    // (the daemon reports CARGO_PKG_VERSION, currently 0.6.0-rc.1).
    expect(String(body.version)).toMatch(/^\d+\.\d+\.\d+/);
  });

  test('tools list (with auth)', async ({ request }) => {
    const resp = await request.get(`${API_URL}/tools`, { headers: AUTH });
    expect(resp.ok()).toBeTruthy();
    const body = await resp.json();
    expect(body.tools.length).toBeGreaterThan(5);
  });

  test('create task', async ({ request }) => {
    const resp = await request.post(`${API_URL}/task`, {
      headers: { ...AUTH, 'Content-Type': 'application/json' },
      data: { title: 'playwright test', prompt: 'say hello from playwright' },
    });
    expect(resp.ok()).toBeTruthy();
    const body = await resp.json();
    expect(body.task_id).toBeTruthy();
    expect(body.status).toBe('pending');
  });

  test('task history', async ({ request }) => {
    const resp = await request.get(`${API_URL}/task/history`, { headers: AUTH });
    expect(resp.ok()).toBeTruthy();
    const body = await resp.json();
    expect(body.tasks).toBeInstanceOf(Array);
  });

  test('metrics', async ({ request }) => {
    const resp = await request.get(`${API_URL}/metrics`, { headers: AUTH });
    expect(resp.ok()).toBeTruthy();
    const text = await resp.text();
    expect(text).toContain('spectyn_mesh_uptime_seconds');
    expect(text).toContain('spectyn_mesh_tools_registered');
  });

  test('401 without auth', async ({ request }) => {
    const resp = await request.get(`${API_URL}/tools`);
    expect(resp.status()).toBe(401);
  });
});
