// F105 · Component tests — MobileNodeAdmin (E002 §"Settings screen" extension).
//
// Mirrors the F103 MobileDispatch.test.tsx setup: mock `safeInvoke` and
// drive the component via render() + userEvent. This file grows feature
// by feature — see the per-commit test groups below.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

// ── invoke() mock ────────────────────────────────────────────────────────

const invokeMock = vi.fn();
vi.mock('../../src/lib/tauri-compat', () => ({
  isTauri: () => false,
  safeInvoke: (...args: unknown[]) => invokeMock(...args),
}));

import { useClusterStore } from '../../src/stores/clusterStore';

beforeEach(() => {
  invokeMock.mockReset();
  // Reset cluster store between tests so optimistic-insert assertions
  // start from a clean slate.
  useClusterStore.setState({ nodes: [], coordinatorId: null });
});

afterEach(() => {
  vi.useRealTimers();
});

async function renderScreen() {
  const { default: MobileNodeAdmin } = await import(
    '../../src/components/mobile/MobileNodeAdmin'
  );
  return render(<MobileNodeAdmin />);
}

describe('<MobileNodeAdmin /> — broker token rotation', () => {
  it('renders the redacted current-token preview on mount', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_broker_token_preview') {
        return Promise.resolve({
          token_preview: '********wxyz',
          broker_url: 'https://phantommesh.io',
          expires_at_ms: 1_700_000_000_000,
          configured: true,
        });
      }
      if (cmd === 'get_heartbeat_interval') return Promise.resolve(30);
      return Promise.resolve(null);
    });
    await renderScreen();
    await waitFor(() => {
      expect(screen.getByTestId('rotate-current-preview').textContent).toBe(
        '********wxyz',
      );
    });
    // The rotate button is enabled once we know a token is configured.
    const btn = screen.getByTestId('rotate-button') as HTMLButtonElement;
    expect(btn.disabled).toBe(false);
  });

  it('rotate button calls rotate_broker_token and refreshes preview', async () => {
    let previewCalls = 0;
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_broker_token_preview') {
        previewCalls += 1;
        return Promise.resolve({
          token_preview: previewCalls === 1 ? '********oldd' : '********newr',
          broker_url: 'https://phantommesh.io',
          expires_at_ms: 0,
          configured: true,
        });
      }
      if (cmd === 'get_heartbeat_interval') return Promise.resolve(30);
      if (cmd === 'rotate_broker_token') {
        return Promise.resolve({
          token_preview: '********newr',
          rotated_at_unix: 1_700_000_001,
        });
      }
      return Promise.resolve(null);
    });
    await renderScreen();
    await waitFor(() => {
      expect(screen.getByTestId('rotate-current-preview').textContent).toBe(
        '********oldd',
      );
    });

    await userEvent.click(screen.getByTestId('rotate-button'));

    await waitFor(() => {
      // Success pill surfaces the new redacted token.
      const pill = screen.getByTestId('rotate-pill');
      expect(pill.textContent).toContain('********newr');
    });
    // Preview row refreshed to the rotated value.
    expect(screen.getByTestId('rotate-current-preview').textContent).toBe(
      '********newr',
    );
    // Verify the round-trip invoked rotate.
    expect(invokeMock).toHaveBeenCalledWith('rotate_broker_token');
  });

  it('rotate button surfaces failure pill on error', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_broker_token_preview') {
        return Promise.resolve({
          token_preview: '********xxxx',
          broker_url: 'https://phantommesh.io',
          expires_at_ms: 0,
          configured: true,
        });
      }
      if (cmd === 'get_heartbeat_interval') return Promise.resolve(30);
      if (cmd === 'rotate_broker_token') {
        return Promise.reject(new Error('E_SETTINGS_AUTH_REQUIRED'));
      }
      return Promise.resolve(null);
    });
    await renderScreen();
    await waitFor(() => screen.getByTestId('rotate-current-preview'));
    await userEvent.click(screen.getByTestId('rotate-button'));
    await waitFor(() => {
      expect(screen.getByTestId('rotate-pill').textContent).toContain(
        'E_SETTINGS_AUTH_REQUIRED',
      );
    });
  });
});

// ── F105 chunk 2 · manual peer add ───────────────────────────────────────

describe('<MobileNodeAdmin /> — manual peer add', () => {
  it('add button is disabled until URL shape is plausible', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_broker_token_preview') return Promise.resolve(null);
      if (cmd === 'get_heartbeat_interval') return Promise.resolve(30);
      return Promise.resolve(null);
    });
    await renderScreen();

    const add = screen.getByTestId('peer-add-button') as HTMLButtonElement;
    expect(add.disabled).toBe(true);

    const input = screen.getByTestId('peer-input') as HTMLInputElement;
    await userEvent.type(input, 'not-a-url');
    expect(add.disabled).toBe(true);

    await userEvent.clear(input);
    await userEvent.type(input, 'http://localhost:7878');
    expect(add.disabled).toBe(false);
  });

  it('add peer calls add_cluster_peer with the typed URL', async () => {
    invokeMock.mockImplementation(
      (cmd: string, args?: Record<string, unknown>) => {
        if (cmd === 'get_broker_token_preview') return Promise.resolve(null);
        if (cmd === 'get_heartbeat_interval') return Promise.resolve(30);
        if (cmd === 'add_cluster_peer') {
          (invokeMock as unknown as { lastArgs?: unknown }).lastArgs = args;
          return Promise.resolve(null);
        }
        return Promise.resolve(null);
      },
    );

    await renderScreen();
    const input = screen.getByTestId('peer-input') as HTMLInputElement;
    await userEvent.type(input, 'http://oracle.tail.ts.net:7878');
    await userEvent.click(screen.getByTestId('peer-add-button'));

    await waitFor(() => {
      expect(screen.getByTestId('peer-pill').textContent).toContain('已加入');
    });
    const captured = (invokeMock as unknown as { lastArgs?: Record<string, unknown> })
      .lastArgs;
    expect(captured).toBeDefined();
    expect(captured?.peerUrl).toBe('http://oracle.tail.ts.net:7878');
    // Optimistic insert remains in the cluster store after success.
    const nodes = useClusterStore.getState().nodes;
    expect(nodes.some((n) => n.name === 'http://oracle.tail.ts.net:7878')).toBe(
      true,
    );
    // Input cleared after success.
    expect((screen.getByTestId('peer-input') as HTMLInputElement).value).toBe('');
  });

  it('rolls back optimistic insert when add_cluster_peer rejects', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_broker_token_preview') return Promise.resolve(null);
      if (cmd === 'get_heartbeat_interval') return Promise.resolve(30);
      if (cmd === 'add_cluster_peer') {
        return Promise.reject(new Error('E_SETTINGS_PEER_URL_INVALID: foo'));
      }
      return Promise.resolve(null);
    });

    await renderScreen();
    await userEvent.type(
      screen.getByTestId('peer-input'),
      'http://oracle.tail.ts.net:7878',
    );
    await userEvent.click(screen.getByTestId('peer-add-button'));
    await waitFor(() => {
      expect(screen.getByTestId('peer-pill').textContent).toContain(
        'E_SETTINGS_PEER_URL_INVALID',
      );
    });
    // Optimistic node should be gone.
    const nodes = useClusterStore.getState().nodes;
    expect(nodes.some((n) => n.name === 'http://oracle.tail.ts.net:7878')).toBe(
      false,
    );
  });
});

// ── F105 chunk 3 · heartbeat-interval slider ─────────────────────────────

describe('<MobileNodeAdmin /> — heartbeat-interval slider', () => {
  it('initialises slider from get_heartbeat_interval and commits on release', async () => {
    // Real timers. The component's commit debounce is 500ms, well
    // inside the default test timeout. Fake timers collide with React
    // 19's async-effect scheduling under jsdom — see the slider commits
    // never resolving when we tried `vi.useFakeTimers()`.
    invokeMock.mockImplementation(
      (cmd: string, args?: Record<string, unknown>) => {
        if (cmd === 'get_broker_token_preview') return Promise.resolve(null);
        if (cmd === 'get_heartbeat_interval') return Promise.resolve(60);
        if (cmd === 'set_heartbeat_interval') {
          (invokeMock as unknown as { setArgs?: unknown }).setArgs = args;
          return Promise.resolve(null);
        }
        return Promise.resolve(null);
      },
    );
    await renderScreen();

    // Initial value should reflect the persisted 60s.
    await waitFor(() => {
      expect(screen.getByTestId('heartbeat-value').textContent).toBe('60s');
    });
    const slider = screen.getByTestId('heartbeat-slider') as HTMLInputElement;
    expect(slider.value).toBe('60');

    // Move the slider — value updates locally without an invoke.
    fireEvent.change(slider, { target: { value: '120' } });
    expect(screen.getByTestId('heartbeat-value').textContent).toBe('120s');
    expect((invokeMock as unknown as { setArgs?: unknown }).setArgs).toBeUndefined();

    // Release → debounced commit ~500ms later.
    fireEvent.pointerUp(slider);
    await waitFor(
      () => {
        expect(screen.getByTestId('heartbeat-pill').textContent).toContain('120');
      },
      { timeout: 2000 },
    );
    const setArgs = (invokeMock as unknown as { setArgs?: Record<string, unknown> })
      .setArgs;
    expect(setArgs).toBeDefined();
    expect(setArgs?.secs).toBe(120);
  });

  it('surfaces failure pill when set_heartbeat_interval rejects', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_broker_token_preview') return Promise.resolve(null);
      if (cmd === 'get_heartbeat_interval') return Promise.resolve(30);
      if (cmd === 'set_heartbeat_interval') {
        return Promise.reject(new Error('E_SETTINGS_HEARTBEAT_OUT_OF_RANGE'));
      }
      return Promise.resolve(null);
    });
    await renderScreen();
    await waitFor(() => {
      expect(screen.getByTestId('heartbeat-value').textContent).toBe('30s');
    });

    const slider = screen.getByTestId('heartbeat-slider') as HTMLInputElement;
    fireEvent.change(slider, { target: { value: '200' } });
    fireEvent.pointerUp(slider);
    await waitFor(
      () => {
        expect(screen.getByTestId('heartbeat-pill').textContent).toContain(
          'E_SETTINGS_HEARTBEAT_OUT_OF_RANGE',
        );
      },
      { timeout: 2000 },
    );
  });
});
