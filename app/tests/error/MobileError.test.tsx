// Component tests — MobileErrorView (SPEC-34 Screen 16 actions).
//
// Pins the three actions the error screen exposes: retry, report (external
// GitHub-issue link), and reset (clears onboarding localStorage). Renders the
// presentational view directly (no router needed).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

import { MobileErrorView } from '../../src/components/mobile/MobileError';

// jsdom won't let us spy on window.location.assign directly, so the reset test
// swaps window.location; restore the original descriptor here (restoreAllMocks
// doesn't undo a defineProperty) to avoid leaking the stub into other tests.
const origLocation = Object.getOwnPropertyDescriptor(window, 'location');

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  vi.restoreAllMocks();
  if (origLocation) Object.defineProperty(window, 'location', origLocation);
});

describe('<MobileErrorView />', () => {
  it('shows the error code passed in', () => {
    render(<MobileErrorView code="RENDER" />);
    expect(screen.getByText(/code: RENDER/)).toBeInTheDocument();
    expect(screen.getByText('發生未預期錯誤')).toBeInTheDocument();
  });

  it('report is an external link to a prefilled GitHub issue', () => {
    render(<MobileErrorView code="E42" />);
    const report = screen.getByRole('link', { name: /回報問題/ });
    expect(report).toHaveAttribute('target', '_blank');
    const href = report.getAttribute('href') ?? '';
    expect(href).toContain('github.com/markl-a/spectyn-mesh/issues/new');
    expect(href).toContain(encodeURIComponent('E42'));
  });

  it('reset clears onboarding keys after confirmation', async () => {
    localStorage.setItem('spectyn_mesh_v2_onboarded', 'true');
    localStorage.setItem('spectyn_mesh_v2_onboarded_mode', 'demo');
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    const assign = vi.fn();
    Object.defineProperty(window, 'location', {
      value: { ...window.location, assign },
      writable: true,
    });

    render(<MobileErrorView code="X" />);
    await userEvent.click(screen.getByRole('button', { name: /重設並重新設定/ }));

    expect(localStorage.getItem('spectyn_mesh_v2_onboarded')).toBeNull();
    expect(localStorage.getItem('spectyn_mesh_v2_onboarded_mode')).toBeNull();
    expect(assign).toHaveBeenCalledWith('/');
  });
});
