// Component tests — MobileHands (mobile port of settings/HandsPanel.tsx).
// Verifies the get_hands shape normalisation (array / { hands: [] } /
// { pipelines: [] } / unknown → raw JSON fallback).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';

const invokeMock = vi.fn();
vi.mock('../../src/lib/tauri-compat', () => ({
  isTauri: () => false,
  safeInvoke: (...args: unknown[]) => invokeMock(...args),
}));

beforeEach(() => invokeMock.mockReset());
afterEach(() => vi.useRealTimers());

async function renderPanel() {
  const { default: MobileHands } = await import(
    '../../src/components/mobile/MobileHands'
  );
  return render(<MobileHands />);
}

describe('<MobileHands />', () => {
  it('renders a list from a plain array of hands', async () => {
    invokeMock.mockResolvedValue([
      { name: 'web_search', description: '搜尋網路內容' },
      { name: 'code_run', description: '在 sandbox 中執行程式碼' },
    ]);
    await renderPanel();
    await waitFor(() => expect(screen.getByTestId('mobile-hands-list')).toBeTruthy());
    expect(screen.getByTestId('hand-web_search')).toBeTruthy();
    expect(screen.getByText('code_run')).toBeTruthy();
    expect(screen.getByText('搜尋網路內容')).toBeTruthy();
  });

  it('unwraps a { hands: [...] } envelope', async () => {
    invokeMock.mockResolvedValue({ hands: [{ name: 'pdf_extract' }] });
    await renderPanel();
    await waitFor(() => expect(screen.getByTestId('hand-pdf_extract')).toBeTruthy());
  });

  it('also unwraps the { pipelines: [...] } envelope (legacy name)', async () => {
    invokeMock.mockResolvedValue({ pipelines: [{ name: 'screenshot_chain' }] });
    await renderPanel();
    await waitFor(() => expect(screen.getByTestId('hand-screenshot_chain')).toBeTruthy());
  });

  it('falls back to raw JSON for an unrecognised shape', async () => {
    invokeMock.mockResolvedValue({ totally_unexpected: 99 });
    await renderPanel();
    await waitFor(() => expect(screen.getByTestId('mobile-hands-raw')).toBeTruthy());
    expect(screen.getByTestId('mobile-hands-raw').textContent).toContain('totally_unexpected');
  });

  it('renders the disabled badge when a hand has enabled:false', async () => {
    invokeMock.mockResolvedValue([{ name: 'legacy_thing', enabled: false }]);
    await renderPanel();
    await waitFor(() => expect(screen.getByTestId('hand-legacy_thing')).toBeTruthy());
    expect(screen.getByText('disabled')).toBeTruthy();
  });
});
