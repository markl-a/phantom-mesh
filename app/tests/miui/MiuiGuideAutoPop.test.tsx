// SPEC-34 G6 / J5 — MIUI guide auto-pop (MobileShell self-gating surface).
//
// MiuiGuideAutoPop mounts unconditionally but must ONLY open the guide when the
// native should_show is true (is_miui && !dont_show_again). On non-MIUI /
// dismissed devices it must render nothing — the whole point is to not nag users
// the guide doesn't apply to. The bridge is mocked so we drive both gates.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';

const checkShouldShowMiuiGuide = vi.fn();
const dismissMiuiGuide = vi.fn(async () => {});
const openMiuiAutostart = vi.fn(async () => false);
const openMiuiBatteryOptimization = vi.fn(async () => false);

vi.mock('../../src/lib/miuiGuide', () => ({
  checkShouldShowMiuiGuide: () => checkShouldShowMiuiGuide(),
  dismissMiuiGuide: (...a: unknown[]) => dismissMiuiGuide(...a),
  openMiuiAutostart: () => openMiuiAutostart(),
  openMiuiBatteryOptimization: () => openMiuiBatteryOptimization(),
}));

beforeEach(() => {
  checkShouldShowMiuiGuide.mockReset();
  dismissMiuiGuide.mockClear();
});
afterEach(() => vi.clearAllMocks());

async function renderAutoPop() {
  const { default: MiuiGuideAutoPop } = await import(
    '../../src/components/mobile/MiuiGuideAutoPop'
  );
  render(<MiuiGuideAutoPop />);
}

describe('<MiuiGuideAutoPop />', () => {
  it('auto-opens the guide when should_show is true (MIUI, not dismissed)', async () => {
    checkShouldShowMiuiGuide.mockResolvedValue({
      should_show: true,
      is_miui: true,
      last_dismissed_ms: null,
    });
    await renderAutoPop();
    await waitFor(() =>
      expect(screen.getByTestId('miui-guide-dialog')).toBeTruthy(),
    );
    // The detection banner reflects the native is_miui=true result. It renders
    // after the dialog's own should_show check resolves, so wait for it too
    // (a bare sync assert here is timing-flaky under load).
    await waitFor(() =>
      expect(screen.getByText(/已偵測到 MIUI/)).toBeTruthy(),
    );
  });

  it('does NOT pop when should_show is false (non-MIUI / dismissed)', async () => {
    checkShouldShowMiuiGuide.mockResolvedValue({
      should_show: false,
      is_miui: false,
      last_dismissed_ms: null,
    });
    await renderAutoPop();
    // Wait until the gate check has actually resolved, then assert no dialog —
    // a regression that ignored should_show would have opened it by now.
    await waitFor(() => expect(checkShouldShowMiuiGuide).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 30));
    expect(screen.queryByTestId('miui-guide-dialog')).toBeNull();
  });

  it('does NOT pop (and does not throw) if the gate check rejects', async () => {
    // Defensive: the real bridge never rejects, but a contract violation must
    // not crash the shell or auto-pop on doubt.
    checkShouldShowMiuiGuide.mockRejectedValue(new Error('boom'));
    await renderAutoPop();
    await waitFor(() => expect(checkShouldShowMiuiGuide).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 30));
    expect(screen.queryByTestId('miui-guide-dialog')).toBeNull();
  });
});
