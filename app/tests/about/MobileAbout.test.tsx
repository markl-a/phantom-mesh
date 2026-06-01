// Component tests — MobileAbout (SPEC-34 Screen 14 關於).
//
// Pins: version+SHA render from get_version, the OSS link rows, graceful
// degradation when the version probe fails, and the restart-onboarding action
// clearing the onboarding localStorage keys.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

const getVersion = vi.fn();
vi.mock('../../src/lib/api', () => ({
  getVersion: () => getVersion(),
}));

const getAppVersion = vi.fn();
vi.mock('@tauri-apps/api/app', () => ({
  getVersion: () => getAppVersion(),
}));

import MobileAbout from '../../src/components/mobile/MobileAbout';

// jsdom doesn't allow vi.spyOn(window.location, 'assign') (the method isn't a
// configurable own-property), so we swap window.location wholesale and restore
// the original descriptor in afterEach (restoreAllMocks doesn't undo a
// defineProperty). The stub only needs `assign` — the component reads nothing
// else off location.
const origLocation = Object.getOwnPropertyDescriptor(window, 'location');

beforeEach(() => {
  localStorage.clear();
  getVersion.mockReset();
  getAppVersion.mockReset();
});

afterEach(() => {
  vi.restoreAllMocks();
  if (origLocation) Object.defineProperty(window, 'location', origLocation);
});

describe('<MobileAbout />', () => {
  it('shows version + short commit when get_version resolves', async () => {
    getVersion.mockResolvedValue({ version: '0.5.0', commit: 'abc123def456' });
    render(<MobileAbout />);
    await waitFor(() =>
      expect(screen.getByText(/v0\.5\.0 · abc123de/)).toBeInTheDocument(),
    );
  });

  it('falls back to the Tauri app version when get_version rejects', async () => {
    getVersion.mockRejectedValue(new Error('not wired'));
    getAppVersion.mockResolvedValue('0.5.0');
    render(<MobileAbout />);
    await waitFor(() =>
      expect(screen.getByText(/v0\.5\.0/)).toBeInTheDocument(),
    );
    expect(getAppVersion).toHaveBeenCalled();
  });

  it('degrades to a dash only when both version sources fail', async () => {
    getVersion.mockRejectedValue(new Error('not wired'));
    getAppVersion.mockRejectedValue(new Error('not tauri'));
    render(<MobileAbout />);
    await waitFor(() => expect(getAppVersion).toHaveBeenCalled());
    expect(screen.getByText('—')).toBeInTheDocument();
    expect(screen.queryByText(/^v\d/)).not.toBeInTheDocument();
    expect(screen.getByText('Phantom Mesh')).toBeInTheDocument();
  });

  it('renders Source + License as external links', () => {
    getVersion.mockResolvedValue({ version: '0', commit: '0' });
    render(<MobileAbout />);
    const source = screen.getByRole('link', { name: /Source/ });
    expect(source).toHaveAttribute('href', 'https://github.com/markl-a/phantom-mesh');
    expect(source).toHaveAttribute('target', '_blank');
    expect(source).toHaveAttribute('rel', 'noreferrer noopener');
    const license = screen.getByRole('link', { name: /License/ });
    expect(license.getAttribute('href')).toContain('/LICENSE');
    expect(license).toHaveAttribute('target', '_blank');
    expect(license).toHaveAttribute('rel', 'noreferrer noopener');
  });

  it('restart onboarding clears the onboarding localStorage keys', async () => {
    getVersion.mockResolvedValue({ version: '0', commit: '0' });
    localStorage.setItem('phantom_mesh_v2_onboarded', 'true');
    localStorage.setItem('phantom_mesh_v2_onboarded_mode', 'demo');
    const assign = vi.fn();
    Object.defineProperty(window, 'location', {
      value: { ...window.location, assign },
      writable: true,
    });

    render(<MobileAbout />);
    await userEvent.click(screen.getByRole('button', { name: /重啟 onboarding/ }));

    expect(localStorage.getItem('phantom_mesh_v2_onboarded')).toBeNull();
    expect(localStorage.getItem('phantom_mesh_v2_onboarded_mode')).toBeNull();
    expect(assign).toHaveBeenCalledWith('/');
  });
});
