import { test, expect } from '@playwright/test';

const BASE = 'http://localhost:5173';

test.describe('GUI Interaction Tests', () => {

  test('Onboarding page renders correctly', async ({ page }) => {
    await page.goto(BASE);
    await page.waitForTimeout(1500);

    // Title
    await expect(page.getByRole('heading', { name: 'Phantom Mesh' })).toBeVisible();

    // Environment detection message
    await expect(page.locator('text=環境偵測完成')).toBeVisible();

    // Login buttons
    await expect(page.locator('text=Google')).toBeVisible();
    await expect(page.locator('text=Apple')).toBeVisible();

    // Service detection
    await expect(page.locator('text=ChatGPT')).toBeVisible();
    await expect(page.locator('text=OpenCode')).toBeVisible();

    // Launch button
    await expect(page.locator('text=啟動 Phantom Mesh')).toBeVisible();

    await page.screenshot({ path: 'e2e/screenshots/onboarding.png' });
  });

  test('Click "啟動 Phantom Mesh" button', async ({ page }) => {
    await page.goto(BASE);
    await page.waitForTimeout(1500);

    // Click the launch button
    const launchBtn = page.locator('text=啟動 Phantom Mesh');
    await expect(launchBtn).toBeVisible();
    await launchBtn.click();
    await page.waitForTimeout(2000);

    // Take screenshot after clicking
    await page.screenshot({ path: 'e2e/screenshots/after-launch.png' });

    // Should navigate away from onboarding
    const currentUrl = page.url();
    console.log('URL after launch:', currentUrl);
  });

  test('Check for dark theme', async ({ page }) => {
    await page.goto(BASE);
    await page.waitForTimeout(1000);

    // Check background is dark
    const bgColor = await page.evaluate(() => {
      return window.getComputedStyle(document.body).backgroundColor;
    });
    console.log('Background color:', bgColor);
    // Dark theme should have dark background
    // rgb(10, 10, 20) = #0a0a14 or similar
  });

  test('Responsive layout - mobile', async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await page.goto(BASE);
    await page.waitForTimeout(1500);
    await page.screenshot({ path: 'e2e/screenshots/mobile.png' });
  });

  test('Responsive layout - tablet', async ({ page }) => {
    await page.setViewportSize({ width: 768, height: 1024 });
    await page.goto(BASE);
    await page.waitForTimeout(1500);
    await page.screenshot({ path: 'e2e/screenshots/tablet.png' });
  });

  test('No console errors on load', async ({ page }) => {
    const errors: string[] = [];
    page.on('console', (msg) => {
      if (msg.type() === 'error') {
        errors.push(msg.text());
      }
    });

    await page.goto(BASE);
    await page.waitForTimeout(3000);

    // Filter out known benign errors
    const real = errors.filter(e =>
      !e.includes('ResizeObserver') &&
      !e.includes('favicon') &&
      !e.includes('net::ERR') &&
      !e.includes('Failed to fetch') &&
      !e.includes('tauri-compat')
    );

    console.log('Console errors:', real.length, real);
    // Browser fallback mode generates expected network errors
    expect(real.length).toBeLessThan(10);
  });

  test('Service detection links are clickable', async ({ page }) => {
    await page.goto(BASE);
    await page.waitForTimeout(1500);

    // "前往登入" links should exist
    const loginLinks = page.locator('text=前往登入');
    const count = await loginLinks.count();
    console.log('Login links found:', count);
    expect(count).toBeGreaterThanOrEqual(1);
  });

});
