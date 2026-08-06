// SPEC-34 G6 / J5 — MIUI compatibility guide dialog tests.
//
// The bridge (lib/miuiGuide) is mocked so we drive the deep-link
// success/fallback paths and assert the dialog's guidance + dismiss behaviour
// without a Tauri runtime. The manual steps are the substance (spectyn can
// only guide, not toggle MIUI settings), so they must always render.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

const dismissMiuiGuide = vi.fn(async () => {});
const openMiuiAutostart = vi.fn(async () => true);
const openMiuiBatteryOptimization = vi.fn(async () => true);
const checkShouldShowMiuiGuide = vi.fn(async () => ({
  should_show: false,
  is_miui: false,
  last_dismissed_ms: null,
}));

vi.mock('../../src/lib/miuiGuide', () => ({
  dismissMiuiGuide: (...a: unknown[]) => dismissMiuiGuide(...a),
  openMiuiAutostart: () => openMiuiAutostart(),
  openMiuiBatteryOptimization: () => openMiuiBatteryOptimization(),
  checkShouldShowMiuiGuide: () => checkShouldShowMiuiGuide(),
}));

beforeEach(() => {
  dismissMiuiGuide.mockClear();
  openMiuiAutostart.mockReset().mockResolvedValue(true);
  openMiuiBatteryOptimization.mockReset().mockResolvedValue(true);
  checkShouldShowMiuiGuide
    .mockReset()
    .mockResolvedValue({ should_show: false, is_miui: false, last_dismissed_ms: null });
});

afterEach(() => vi.clearAllMocks());

async function renderDialog(open: boolean, onClose = vi.fn()) {
  const { default: MiuiGuideDialog } = await import(
    '../../src/components/mobile/MiuiGuideDialog'
  );
  render(<MiuiGuideDialog open={open} onClose={onClose} />);
  return { onClose };
}

describe('<MiuiGuideDialog />', () => {
  it('renders nothing when closed', async () => {
    await renderDialog(false);
    expect(screen.queryByTestId('miui-guide-dialog')).toBeNull();
  });

  it('renders title, both manual step blocks and the 3 actions when open', async () => {
    await renderDialog(true);
    expect(screen.getByText(/偵測到 MIUI/)).toBeTruthy();
    // Manual steps (the substance) are always present — assert on phrases
    // unique to each step block (the intro <p> also mentions 自啟動/電池).
    expect(screen.getByText(/應用管理/)).toBeTruthy();
    expect(screen.getByText(/電池與效能/)).toBeTruthy();
    expect(screen.getByText('自啟設定')).toBeTruthy();
    expect(screen.getByText('電池設定')).toBeTruthy();
    expect(screen.getByTestId('miui-dont-show-again')).toBeTruthy();
  });

  it('shows the not-MIUI detection banner on a non-Xiaomi device', async () => {
    checkShouldShowMiuiGuide.mockResolvedValue({
      should_show: false,
      is_miui: false,
      last_dismissed_ms: null,
    });
    await renderDialog(true);
    await waitFor(() =>
      expect(screen.getByTestId('miui-detect-banner').textContent).toMatch(/不是小米/),
    );
  });

  it('shows the MIUI-detected banner on a Xiaomi device', async () => {
    checkShouldShowMiuiGuide.mockResolvedValue({
      should_show: true,
      is_miui: true,
      last_dismissed_ms: null,
    });
    await renderDialog(true);
    await waitFor(() =>
      expect(screen.getByTestId('miui-detect-banner').textContent).toMatch(/已偵測到 MIUI/),
    );
  });

  it('persists dont-show-again then closes', async () => {
    const { onClose } = await renderDialog(true);
    await userEvent.click(screen.getByTestId('miui-dont-show-again'));
    await waitFor(() => expect(dismissMiuiGuide).toHaveBeenCalledWith(true));
    expect(onClose).toHaveBeenCalled();
  });

  it('shows a manual-steps fallback when the autostart deep-link cannot launch', async () => {
    openMiuiAutostart.mockResolvedValue(false);
    await renderDialog(true);
    await userEvent.click(screen.getByText('自啟設定'));
    await waitFor(() =>
      expect(screen.getByText(/無法直接跳轉.*手動開啟/)).toBeTruthy(),
    );
  });

  it('does not show the battery fallback when its deep-link succeeds', async () => {
    openMiuiBatteryOptimization.mockResolvedValue(true);
    await renderDialog(true);
    await userEvent.click(screen.getByText('電池設定'));
    await waitFor(() => expect(openMiuiBatteryOptimization).toHaveBeenCalled());
    expect(screen.queryByText(/無法直接跳轉.*手動設定/)).toBeNull();
  });

  it('closes via the 完成 button and the backdrop', async () => {
    const onClose = vi.fn();
    await renderDialog(true, onClose);
    await userEvent.click(screen.getByText('完成'));
    expect(onClose).toHaveBeenCalledTimes(1);
    // Backdrop (the overlay itself) also closes; the inner panel stops it.
    await userEvent.click(screen.getByTestId('miui-guide-dialog'));
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it('closes on Escape', async () => {
    const onClose = vi.fn();
    await renderDialog(true, onClose);
    screen.getByTestId('miui-guide-dialog').focus();
    await userEvent.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalled();
  });

  it('does not fire a second intent on rapid double-tap (busy guard)', async () => {
    let resolve!: (v: boolean) => void;
    openMiuiAutostart.mockReturnValue(new Promise<boolean>((r) => (resolve = r)));
    await renderDialog(true);
    const btn = screen.getByText('自啟設定');
    await userEvent.click(btn);
    await userEvent.click(btn); // second tap while the first is in flight
    resolve(true);
    await waitFor(() => expect(openMiuiAutostart).toHaveBeenCalledTimes(1));
  });

  it('clears a stale fallback warning when the dialog is reopened', async () => {
    const { default: MiuiGuideDialog } = await import(
      '../../src/components/mobile/MiuiGuideDialog'
    );
    openMiuiAutostart.mockResolvedValue(false);
    const { rerender } = render(
      <MiuiGuideDialog open onClose={vi.fn()} />,
    );
    await userEvent.click(screen.getByText('自啟設定'));
    await waitFor(() =>
      expect(screen.getByText(/無法直接跳轉.*手動開啟/)).toBeTruthy(),
    );
    // Close then reopen — the stale warning must be gone.
    rerender(<MiuiGuideDialog open={false} onClose={vi.fn()} />);
    rerender(<MiuiGuideDialog open onClose={vi.fn()} />);
    expect(screen.queryByText(/無法直接跳轉.*手動開啟/)).toBeNull();
  });
});
