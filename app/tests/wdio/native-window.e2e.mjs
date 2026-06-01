// NATIVE-WINDOW E2E — drives the REAL Tauri WKWebView window (not a browser)
// via tauri-wd. This is the macOS-native counterpart to the Playwright web-shell
// E2E: same React UI, but rendered in the actual app window the user ships.
import { browser, expect } from '@wdio/globals';

describe('Phantom Mesh — native window (WKWebView via tauri-wd)', () => {
  it('renders the onboarding screen in the native window', async () => {
    // The plugin injects into the live webview; give the SPA a moment to mount.
    await browser.pause(2500);

    const body = await browser.$('body');
    await expect(body).toBeExisting();

    const text = await body.getText();
    // The onboarding screen's stable brand/heading text.
    expect(text).toContain('Phantom Mesh');
  });

  it('lists real providers in the native window', async () => {
    const body = await browser.$('body');
    const text = await body.getText();
    // Same real provider names asserted by the Playwright web-shell test —
    // proves the actual shipped window renders the provider list, not just a
    // browser tab.
    expect(text).toContain('OpenRouter');
    expect(text).toContain('Anthropic');
  });
});
