// Routing regression — App.tsx mobile branch.
//
// Guards the wiring bug where MobileShell exposed 派送 (/dispatch) and 歷史
// (/history) tabs but MobileApp never declared their routes, so they fell
// through the `*` catch-all and silently redirected to the chat screen.
// Each tab path must resolve to its own screen, and the terminal route must
// render outside the tab shell. Screens are stubbed so this asserts routing
// only, not screen internals.

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';

// Always take the mobile branch.
vi.mock('../../src/hooks/useIsMobile', () => ({ useIsMobile: () => true }));

// Stub the screens MobileApp routes to.
vi.mock('../../src/components/mobile/MobileConversation', () => ({ default: () => <div data-testid="screen-conversation" /> }));
vi.mock('../../src/components/focus/FocusPage', () => ({ default: () => <div data-testid="screen-focus" /> }));
vi.mock('../../src/components/mobile/MobileMesh', () => ({ default: () => <div data-testid="screen-cluster" /> }));
vi.mock('../../src/components/mobile/MobileDispatch', () => ({ default: () => <div data-testid="screen-dispatch" /> }));
vi.mock('../../src/components/mobile/MobileHistory', () => ({ default: () => <div data-testid="screen-history" /> }));
vi.mock('../../src/components/mobile/MobileSettings', () => ({ default: () => <div data-testid="screen-settings" /> }));

// Onboarding screens are imported by App but should not render once onboarded.
vi.mock('../../src/components/mobile/MobileFirstLaunch', () => ({ default: () => <div data-testid="onb-firstlaunch" /> }));
vi.mock('../../src/components/mobile/MobileJoinCluster', () => ({ default: () => <div data-testid="onb-join" /> }));
vi.mock('../../src/components/mobile/MobileOnboardingV2', () => ({ default: () => <div data-testid="onb-v2" /> }));

// Desktop pages + onboarding are imported by App but never rendered on mobile.
vi.mock('../../src/pages/Conversation', () => ({ default: () => null }));
vi.mock('../../src/pages/Dashboard', () => ({ default: () => null }));
vi.mock('../../src/pages/Goals', () => ({ default: () => null }));
vi.mock('../../src/pages/Browser', () => ({ default: () => null }));
vi.mock('../../src/pages/PageViewer', () => ({ default: () => null }));
vi.mock('../../src/pages/Settings', () => ({ default: () => null }));
vi.mock('../../src/pages/Terminal', () => ({ default: () => <div data-testid="screen-terminal" /> }));
vi.mock('../../src/components/StartupCheck', () => ({ default: () => null }));
vi.mock('../../src/components/onboarding/OnboardingQuickStart', () => ({ default: () => null, clearSession: () => {} }));

import App from '../../src/App';

beforeEach(() => {
  // This jsdom env ships only a partial localStorage shim, so install a
  // Map-backed one. Seed `phantom_mesh_v2_onboarded` so MobileApp skips the
  // first-launch picker and renders the tab shell.
  const store = new Map<string, string>([['phantom_mesh_v2_onboarded', 'true']]);
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => { store.set(k, String(v)); },
    removeItem: (k: string) => { store.delete(k); },
    clear: () => { store.clear(); },
    key: (i: number) => Array.from(store.keys())[i] ?? null,
    get length() { return store.size; },
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

const renderAt = (path: string) =>
  render(
    <MemoryRouter initialEntries={[path]}>
      <App />
    </MemoryRouter>,
  );

describe('MobileApp tab routing', () => {
  it.each([
    ['/', 'screen-conversation'],
    ['/focus', 'screen-focus'],
    ['/cluster', 'screen-cluster'],
    ['/dispatch', 'screen-dispatch'],
    ['/history', 'screen-history'],
    ['/settings', 'screen-settings'],
  ])('path %s resolves to its own screen', (path, testid) => {
    renderAt(path);
    expect(screen.getByTestId(testid)).toBeInTheDocument();
  });

  it('/dispatch does NOT fall back to the chat screen (regression)', () => {
    renderAt('/dispatch');
    expect(screen.queryByTestId('screen-conversation')).toBeNull();
    expect(screen.getByTestId('screen-dispatch')).toBeInTheDocument();
  });

  it('/history does NOT fall back to the chat screen (regression)', () => {
    renderAt('/history');
    expect(screen.queryByTestId('screen-conversation')).toBeNull();
    expect(screen.getByTestId('screen-history')).toBeInTheDocument();
  });

  it('unknown path falls back to the chat screen', () => {
    renderAt('/totally-unknown-path');
    expect(screen.getByTestId('screen-conversation')).toBeInTheDocument();
  });

  it('/term renders the full-screen terminal outside the tab shell', () => {
    renderAt('/term');
    expect(screen.getByTestId('screen-terminal')).toBeInTheDocument();
  });
});
