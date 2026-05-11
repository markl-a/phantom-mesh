import { test, expect } from '@playwright/test';

const BASE_URL = 'http://localhost:5173';
const API_URL = 'http://localhost:7878';

test.describe('Phantom Mesh E2E Smoke Tests', () => {

  test('GUI loads successfully', async ({ page }) => {
    await page.goto(BASE_URL);
    await expect(page).toHaveTitle(/phantom/i);
    // Page should render without JS errors
    const errors: string[] = [];
    page.on('pageerror', (err) => errors.push(err.message));
    await page.waitForTimeout(2000);
    // Allow some errors but flag critical ones
    const critical = errors.filter(e => !e.includes('ResizeObserver'));
    expect(critical.length).toBeLessThan(3);
  });

  test('GUI renders main layout', async ({ page }) => {
    await page.goto(BASE_URL);
    await page.waitForTimeout(1500);
    // Take screenshot for visual check
    await page.screenshot({ path: 'e2e/screenshots/main-layout.png', fullPage: true });
    // Page should have some visible content
    const bodyText = await page.textContent('body');
    expect(bodyText).toBeTruthy();
    expect(bodyText!.length).toBeGreaterThan(10);
  });

  test('Daemon API health check', async ({ request }) => {
    const resp = await request.get(`${API_URL}/health`);
    expect(resp.ok()).toBeTruthy();
    const body = await resp.json();
    expect(body.status).toBe('ok');
    expect(body.version).toBe('0.5.0');
  });

  test('Daemon API tools list (with auth)', async ({ request }) => {
    const resp = await request.get(`${API_URL}/tools`, {
      headers: { 'Authorization': 'Bearer e9723eea85484da6b39d5abdcdcef6bf' }
    });
    expect(resp.ok()).toBeTruthy();
    const body = await resp.json();
    expect(body.tools.length).toBeGreaterThan(5);
  });

  test('Daemon API create task', async ({ request }) => {
    const resp = await request.post(`${API_URL}/task`, {
      headers: {
        'Authorization': 'Bearer e9723eea85484da6b39d5abdcdcef6bf',
        'Content-Type': 'application/json',
      },
      data: { title: 'playwright test', prompt: 'say hello from playwright' },
    });
    expect(resp.ok()).toBeTruthy();
    const body = await resp.json();
    expect(body.task_id).toBeTruthy();
    expect(body.status).toBe('pending');
  });

  test('Daemon API task history', async ({ request }) => {
    const resp = await request.get(`${API_URL}/task/history`, {
      headers: { 'Authorization': 'Bearer e9723eea85484da6b39d5abdcdcef6bf' }
    });
    expect(resp.ok()).toBeTruthy();
    const body = await resp.json();
    expect(body.tasks).toBeInstanceOf(Array);
    expect(body.tasks.length).toBeGreaterThan(0);
  });

  test('Daemon API metrics', async ({ request }) => {
    const resp = await request.get(`${API_URL}/metrics`, {
      headers: { 'Authorization': 'Bearer e9723eea85484da6b39d5abdcdcef6bf' }
    });
    expect(resp.ok()).toBeTruthy();
    const text = await resp.text();
    expect(text).toContain('phantom_mesh_uptime_seconds');
    expect(text).toContain('phantom_mesh_tools_registered');
  });

  test('Daemon API 401 without auth', async ({ request }) => {
    const resp = await request.get(`${API_URL}/tools`);
    expect(resp.status()).toBe(401);
  });

  test('GUI screenshot - after navigation', async ({ page }) => {
    await page.goto(BASE_URL);
    await page.waitForTimeout(2000);
    // Try clicking around if there are navigation elements
    const nav = page.locator('nav, [role="navigation"], aside');
    if (await nav.count() > 0) {
      await page.screenshot({ path: 'e2e/screenshots/with-nav.png', fullPage: true });
    }
  });

});
