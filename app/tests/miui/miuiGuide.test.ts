// SPEC-34 §9 — MIUI guide bridge: graceful degradation tests.
//
// The whole point of the bridge is that it returns safe defaults when the
// native miui_guide_* commands aren't implemented yet — so the dialog still
// renders + the manual steps still guide. In a plain-browser / web build
// safeInvoke's httpFallback resolves an unknown command to `{}` (see the note
// in lib/dailyReview.ts), so that — not a throw — is the realistic absent path.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const safeInvoke = vi.fn();
vi.mock('../../src/lib/tauri-compat', () => ({
  isTauri: () => false,
  safeInvoke: (...a: unknown[]) => safeInvoke(...a),
}));

beforeEach(() => safeInvoke.mockReset());
afterEach(() => vi.clearAllMocks());

import {
  checkShouldShowMiuiGuide,
  dismissMiuiGuide,
  openMiuiAutostart,
  openMiuiBatteryOptimization,
} from '../../src/lib/miuiGuide';

describe('miuiGuide bridge', () => {
  it('checkShouldShow returns the native result when valid', async () => {
    safeInvoke.mockResolvedValue({ should_show: true, is_miui: true, last_dismissed_ms: 123 });
    expect(await checkShouldShowMiuiGuide()).toEqual({
      should_show: true,
      is_miui: true,
      last_dismissed_ms: 123,
    });
  });

  it('checkShouldShow falls back to not-showing when the command is absent ({})', async () => {
    safeInvoke.mockResolvedValue({});
    expect(await checkShouldShowMiuiGuide()).toEqual({
      should_show: false,
      is_miui: false,
      last_dismissed_ms: null,
    });
  });

  it('checkShouldShow falls back when the command resolves null', async () => {
    safeInvoke.mockResolvedValue(null);
    expect(await checkShouldShowMiuiGuide()).toEqual({
      should_show: false,
      is_miui: false,
      last_dismissed_ms: null,
    });
  });

  it('dismiss forwards the camelCase arg (Tauri renames to dont_show_again)', async () => {
    safeInvoke.mockResolvedValue(undefined);
    await dismissMiuiGuide(true);
    expect(safeInvoke).toHaveBeenCalledWith('miui_guide_dismiss', { dontShowAgain: true });
  });

  it('openAutostart returns true only on {ok:true}, false on junk', async () => {
    safeInvoke.mockResolvedValue({ ok: true });
    expect(await openMiuiAutostart()).toBe(true);
    safeInvoke.mockResolvedValue({});
    expect(await openMiuiAutostart()).toBe(false);
    safeInvoke.mockResolvedValue(null);
    expect(await openMiuiAutostart()).toBe(false);
  });

  it('openBatteryOptimization returns true only on {ok:true}', async () => {
    safeInvoke.mockResolvedValue({ ok: true });
    expect(await openMiuiBatteryOptimization()).toBe(true);
    safeInvoke.mockResolvedValue({ ok: false });
    expect(await openMiuiBatteryOptimization()).toBe(false);
  });
});
