// Component tests — NotifLockCard (SPEC-34 Screen 17 notification-denied fallback).
//
// The card self-gates on the live notification permission (dynamic-imported
// @tauri-apps/plugin-notification) and a localStorage ack flag. These tests
// pin the gating so a regression can't silently show/hide it wrong.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

const ACK_KEY = 'notif_perm_acknowledged';

// Mutable permission state the mocked plugin reports.
let granted = false;
const requestPermission = vi.fn(async () => (granted ? 'granted' : 'denied'));
vi.mock('@tauri-apps/plugin-notification', () => ({
  isPermissionGranted: async () => granted,
  requestPermission: () => requestPermission(),
}));

import NotifLockCard from '../../src/components/mobile/NotifLockCard';

beforeEach(() => {
  granted = false;
  localStorage.clear();
  requestPermission.mockClear();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('<NotifLockCard />', () => {
  it('shows the fallback when permission is denied and not acknowledged', async () => {
    render(<NotifLockCard />);
    expect(await screen.findByText('無通知權限')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '去設定' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '先用 app 內卡片' })).toBeInTheDocument();
  });

  it('stays hidden when permission is granted', async () => {
    granted = true;
    render(<NotifLockCard />);
    // Give the async effect a tick; the card must never appear.
    await new Promise((r) => setTimeout(r, 0));
    expect(screen.queryByText('無通知權限')).not.toBeInTheDocument();
  });

  it('stays hidden once the user has acknowledged', async () => {
    localStorage.setItem(ACK_KEY, 'true');
    render(<NotifLockCard />);
    await new Promise((r) => setTimeout(r, 0));
    expect(screen.queryByText('無通知權限')).not.toBeInTheDocument();
  });

  it('hides the card when "去設定" grants permission', async () => {
    render(<NotifLockCard />);
    const goSettings = await screen.findByRole('button', { name: '去設定' });
    granted = true; // the system prompt will now resolve as granted
    await userEvent.click(goSettings);
    await waitFor(() =>
      expect(screen.queryByText('無通知權限')).not.toBeInTheDocument(),
    );
    expect(requestPermission).toHaveBeenCalled();
    // A later mount with permission granted must stay hidden (no stale ack).
    expect(localStorage.getItem(ACK_KEY)).toBeNull();
  });

  it('keeps the card visible when "去設定" is still denied', async () => {
    render(<NotifLockCard />);
    const goSettings = await screen.findByRole('button', { name: '去設定' });
    await userEvent.click(goSettings); // granted stays false → still denied
    await waitFor(() => expect(requestPermission).toHaveBeenCalled());
    expect(screen.getByText('無通知權限')).toBeInTheDocument();
  });

  it('dismisses and records the ack when "先用 app 內卡片" is tapped', async () => {
    render(<NotifLockCard />);
    const dismiss = await screen.findByRole('button', { name: '先用 app 內卡片' });
    await userEvent.click(dismiss);
    await waitFor(() =>
      expect(screen.queryByText('無通知權限')).not.toBeInTheDocument(),
    );
    expect(localStorage.getItem(ACK_KEY)).toBe('true');
  });
});
