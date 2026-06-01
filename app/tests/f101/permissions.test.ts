// Unit tests — lib/permissions.ts (SPEC-33 §11/§15 runtime permission gates).
// Covers the "never ask again" deny heuristic + status mapping for the
// notifications gate (the one driven by the Notification API, easy to stub in
// jsdom). The deny-count localStorage logic is what J1's permissions step and
// the Settings deep-link rely on.

import { describe, expect, it, vi, beforeEach } from 'vitest';
import {
  requestPermission, permissionsGateApplies,
} from '../../src/lib/permissions';

const notif = {
  permission: 'default' as NotificationPermission,
  requestPermission: vi.fn(),
};

beforeEach(() => {
  localStorage.clear();
  notif.requestPermission.mockReset();
  vi.stubGlobal('Notification', notif);
});

describe('permissionsGateApplies', () => {
  it('is true under jsdom (navigator defined)', () => {
    expect(permissionsGateApplies()).toBe(true);
  });
});

describe('requestPermission(notifications) — deny heuristic', () => {
  it('granted clears deny state and never flags never-ask-again', async () => {
    notif.requestPermission.mockResolvedValue('granted');
    const r = await requestPermission('notifications');
    expect(r).toEqual({ status: 'granted', neverAskAgain: false });
  });

  it('flips neverAskAgain only after the 2nd consecutive denial', async () => {
    notif.requestPermission.mockResolvedValue('denied');
    const first = await requestPermission('notifications');
    expect(first).toEqual({ status: 'denied', neverAskAgain: false });
    const second = await requestPermission('notifications');
    expect(second).toEqual({ status: 'denied', neverAskAgain: true });
  });

  it('a later grant resets the deny count', async () => {
    notif.requestPermission.mockResolvedValue('denied');
    await requestPermission('notifications');
    await requestPermission('notifications'); // now neverAskAgain would be true
    notif.requestPermission.mockResolvedValue('granted');
    const granted = await requestPermission('notifications');
    expect(granted).toEqual({ status: 'granted', neverAskAgain: false });
    // deny count cleared → a fresh denial is back to neverAskAgain:false
    notif.requestPermission.mockResolvedValue('denied');
    const afterReset = await requestPermission('notifications');
    expect(afterReset).toEqual({ status: 'denied', neverAskAgain: false });
  });
});
