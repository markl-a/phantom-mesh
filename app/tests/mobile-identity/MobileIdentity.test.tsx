// Component tests — MobileIdentity (mobile port of settings/IdentityPanel.tsx,
// BIG-GOAL P4 identity & privacy). Covers status card render, the "no
// identity" empty state, the static encryption honesty rail, and the export
// flow (kind + since → data_export with format).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

const invokeMock = vi.fn();
vi.mock('../../src/lib/tauri-compat', () => ({
  isTauri: () => false,
  safeInvoke: (...args: unknown[]) => invokeMock(...args),
}));

beforeEach(() => invokeMock.mockReset());
afterEach(() => vi.useRealTimers());

async function renderPanel() {
  const { default: MobileIdentity } = await import(
    '../../src/components/mobile/MobileIdentity'
  );
  return render(<MobileIdentity />);
}

const sampleStatus = {
  hasIdentity: true,
  fingerprint: 'age1abcdef0123456789xyz',
  createdAt: '2026-05-01',
  keystore: 'KeystoreFile',
  identityLine: 'age1abcdef0123456789xyz acer-2026-05-01',
};

describe('<MobileIdentity />', () => {
  it('renders the status card + honesty rail when identity_status returns a key', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'identity_status') return Promise.resolve(sampleStatus);
      return Promise.resolve(null);
    });
    await renderPanel();
    await waitFor(() =>
      expect(screen.getByTestId('mobile-identity-root')).toBeTruthy(),
    );
    expect(screen.getByTestId('identity-status-card')).toBeTruthy();
    expect(screen.getByText(sampleStatus.fingerprint)).toBeTruthy();
    expect(screen.getByText(/建立於 2026-05-01/)).toBeTruthy();
    // Honesty rail static lists render.
    expect(screen.getByTestId('identity-honesty-rail')).toBeTruthy();
    expect(screen.getByText(/identity\.key/)).toBeTruthy();
    expect(screen.getByText(/agents\.toml/)).toBeTruthy();
  });

  it('shows the "no identity" empty state when hasIdentity is false', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'identity_status')
        return Promise.resolve({
          hasIdentity: false,
          fingerprint: '',
          createdAt: '',
          keystore: 'Unavailable',
          identityLine: null,
        });
      return Promise.resolve(null);
    });
    await renderPanel();
    await waitFor(() =>
      expect(screen.getByTestId('identity-empty')).toBeTruthy(),
    );
  });

  it('shows the warning when the backend returns null/invalid', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'identity_status') return Promise.resolve(null);
      return Promise.resolve(null);
    });
    await renderPanel();
    await waitFor(() =>
      expect(screen.getByTestId('identity-error')).toBeTruthy(),
    );
  });

  it('export → data_export is called with the picked kind + since', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'identity_status') return Promise.resolve(sampleStatus);
      if (cmd === 'data_export')
        return Promise.resolve('~/.spectyn-mesh/exports/events-2026-05-30.json');
      return Promise.resolve(null);
    });
    await renderPanel();
    await waitFor(() => screen.getByTestId('export-json'));

    await userEvent.selectOptions(screen.getByTestId('export-kind'), 'focus');
    await userEvent.click(screen.getByTestId('export-json'));

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some(
          (c) =>
            c[0] === 'data_export' &&
            (c[1] as { format: string; kind: string | null }).format === 'json' &&
            (c[1] as { format: string; kind: string | null }).kind === 'focus',
        ),
      ).toBe(true);
    });
    await waitFor(() =>
      expect(screen.getByTestId('export-msg').textContent).toContain('events-2026-05-30.json'),
    );
  });

  it('open-folder button invokes open_exports_folder', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'identity_status') return Promise.resolve(sampleStatus);
      if (cmd === 'open_exports_folder') return Promise.resolve(null);
      return Promise.resolve(null);
    });
    await renderPanel();
    await waitFor(() => screen.getByTestId('export-open-folder'));
    await userEvent.click(screen.getByTestId('export-open-folder'));
    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some((c) => c[0] === 'open_exports_folder'),
      ).toBe(true);
    });
  });
});
