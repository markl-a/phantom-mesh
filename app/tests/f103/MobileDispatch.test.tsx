// F103 · Integration tests — MobileDispatch screen.
//
// Mirrors the F101 MobileCluster.test.tsx pattern: mock the `safeInvoke`
// wrapper, drive Tauri events via the `__setDispatchListenImpl` test
// seam, and assert on rendered DOM + store state.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {
  useDispatchStore,
} from '../../src/stores/dispatchStore';
import { __setDispatchListenImpl } from '../../src/components/mobile/MobileDispatch';
import type { DispatchFrame } from '../../src/lib/dispatchTypes';

// ── invoke() mock ────────────────────────────────────────────────────────

const invokeMock = vi.fn();
vi.mock('../../src/lib/tauri-compat', () => ({
  isTauri: () => false,
  safeInvoke: (...args: unknown[]) => invokeMock(...args),
}));

// ── listen() test seam ───────────────────────────────────────────────────

type EmittedHandler = (ev: { payload: DispatchFrame }) => void;
let lastHandler: EmittedHandler | null = null;
let lastEventName: string | null = null;
let unlistenCalls = 0;

beforeEach(() => {
  invokeMock.mockReset();
  lastHandler = null;
  lastEventName = null;
  unlistenCalls = 0;
  useDispatchStore.getState().reset();

  __setDispatchListenImpl(async (name, handler) => {
    lastEventName = name;
    lastHandler = handler as EmittedHandler;
    return () => {
      unlistenCalls += 1;
    };
  });
});

afterEach(() => {
  __setDispatchListenImpl(null);
});

async function renderScreen() {
  const { default: MobileDispatch } = await import(
    '../../src/components/mobile/MobileDispatch'
  );
  return render(<MobileDispatch />);
}

describe('<MobileDispatch />', () => {
  it('disables submit until prompt is non-whitespace', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_dispatch_providers') return Promise.resolve([]);
      return Promise.resolve(null);
    });
    await renderScreen();

    const submit = screen.getByTestId('dispatch-submit') as HTMLButtonElement;
    expect(submit.disabled).toBe(true);

    const prompt = screen.getByTestId('dispatch-prompt') as HTMLTextAreaElement;
    await userEvent.type(prompt, '   ');
    expect(submit.disabled).toBe(true);

    await userEvent.clear(prompt);
    await userEvent.type(prompt, 'do the thing');
    expect(submit.disabled).toBe(false);
  });

  it('submits with the user prompt and selected caps', async () => {
    invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'list_dispatch_providers') return Promise.resolve([]);
      if (cmd === 'dispatch_task') {
        // Capture the request body for the assertion below.
        (invokeMock as unknown as { lastArgs?: unknown }).lastArgs = args;
        return Promise.resolve({
          dispatch_id: 'd-test-1',
          started_at_unix: 1_700_000_000,
        });
      }
      return Promise.resolve(null);
    });

    await renderScreen();
    const prompt = screen.getByTestId('dispatch-prompt') as HTMLTextAreaElement;
    await userEvent.type(prompt, 'tell me a joke');
    // Toggle two caps.
    await userEvent.click(screen.getByTestId('cap-chip-gpu'));
    await userEvent.click(screen.getByTestId('cap-chip-vision'));

    await userEvent.click(screen.getByTestId('dispatch-submit'));

    await waitFor(() => {
      // Phase tag should appear once the store has the dispatch.
      expect(screen.getByTestId('dispatch-phase')).toBeInTheDocument();
    });

    const captured = (invokeMock as unknown as { lastArgs?: Record<string, unknown> })
      .lastArgs;
    expect(captured).toBeDefined();
    const req = captured?.request as Record<string, unknown>;
    expect(req.prompt).toBe('tell me a joke');
    expect(req.required_caps).toEqual(['gpu', 'vision']);
    expect(req.provider_override).toBeNull();
  });

  it('paints tokens as dispatch::token::<id> events arrive', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_dispatch_providers') return Promise.resolve([]);
      if (cmd === 'dispatch_task') {
        return Promise.resolve({
          dispatch_id: 'd-stream-1',
          started_at_unix: 1_700_000_000,
        });
      }
      return Promise.resolve(null);
    });

    await renderScreen();
    await userEvent.type(screen.getByTestId('dispatch-prompt'), 'go');
    await userEvent.click(screen.getByTestId('dispatch-submit'));

    // Wait for the listener to attach.
    await waitFor(() => expect(lastHandler).not.toBeNull());
    expect(lastEventName).toBe('dispatch::token::d-stream-1');

    await act(async () => {
      lastHandler!({ payload: { type: 'status', phase: 'running' } });
      lastHandler!({ payload: { type: 'token', text: 'hello ' } });
      lastHandler!({ payload: { type: 'token', text: 'world' } });
    });

    await waitFor(() => {
      const pre = screen.getByTestId('dispatch-tokens');
      expect(pre.textContent).toContain('hello world');
    });
  });

  it('renders error block when an error frame arrives', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_dispatch_providers') return Promise.resolve([]);
      if (cmd === 'dispatch_task') {
        return Promise.resolve({
          dispatch_id: 'd-err-1',
          started_at_unix: 1_700_000_000,
        });
      }
      return Promise.resolve(null);
    });

    await renderScreen();
    await userEvent.type(screen.getByTestId('dispatch-prompt'), 'fail-me');
    await userEvent.click(screen.getByTestId('dispatch-submit'));
    await waitFor(() => expect(lastHandler).not.toBeNull());

    await act(async () => {
      lastHandler!({
        payload: { type: 'error', code: 'E_BROKER_DOWN', message: '503' },
      });
    });

    await waitFor(() => {
      const block = screen.getByTestId('dispatch-failure');
      expect(block.textContent).toContain('E_BROKER_DOWN');
      expect(block.textContent).toContain('503');
    });
  });

  it('cancel button calls cancel_dispatch and flips phase to cancelled', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_dispatch_providers') return Promise.resolve([]);
      if (cmd === 'dispatch_task') {
        return Promise.resolve({
          dispatch_id: 'd-cancel-1',
          started_at_unix: 1_700_000_000,
        });
      }
      if (cmd === 'cancel_dispatch') {
        (invokeMock as unknown as { cancelArg?: unknown }).cancelArg = arguments;
        return Promise.resolve(null);
      }
      return Promise.resolve(null);
    });

    await renderScreen();
    await userEvent.type(screen.getByTestId('dispatch-prompt'), 'streaming');
    await userEvent.click(screen.getByTestId('dispatch-submit'));
    await waitFor(() => expect(lastHandler).not.toBeNull());

    // Push one token so the UI sees in-flight state.
    await act(async () => {
      lastHandler!({ payload: { type: 'token', text: 'x' } });
    });

    // Cancel button should now be rendered.
    const cancelBtn = await screen.findByTestId('dispatch-cancel');
    await userEvent.click(cancelBtn);

    await waitFor(() => {
      // Phase tag should read 'cancelled'.
      expect(screen.getByTestId('dispatch-phase').textContent).toContain(
        'cancelled',
      );
    });
  });

  it('shows submitError surface when dispatch_task rejects', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_dispatch_providers') return Promise.resolve([]);
      if (cmd === 'dispatch_task') {
        return Promise.reject(new Error('E_DISPATCH_AUTH_REQUIRED'));
      }
      return Promise.resolve(null);
    });

    await renderScreen();
    await userEvent.type(screen.getByTestId('dispatch-prompt'), 'whatever');
    await userEvent.click(screen.getByTestId('dispatch-submit'));

    await waitFor(() => {
      const err = screen.getByTestId('dispatch-error');
      // Friendly Traditional-Chinese copy is shown to the user…
      expect(err.textContent).toContain('登入');
      // …with the raw stable code retained (tiny) for bug reports.
      expect(err.textContent).toContain('E_DISPATCH_AUTH_REQUIRED');
    });
  });

  it('caps chip strip enforces max-3 selection', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_dispatch_providers') return Promise.resolve([]);
      return Promise.resolve(null);
    });

    await renderScreen();
    await userEvent.click(screen.getByTestId('cap-chip-gpu'));
    await userEvent.click(screen.getByTestId('cap-chip-vision'));
    await userEvent.click(screen.getByTestId('cap-chip-camera'));

    // 4th chip should be disabled.
    const audio = screen.getByTestId('cap-chip-audio') as HTMLButtonElement;
    expect(audio.disabled).toBe(true);

    // Untoggling a selected one frees a slot.
    await userEvent.click(screen.getByTestId('cap-chip-gpu'));
    expect((screen.getByTestId('cap-chip-audio') as HTMLButtonElement).disabled).toBe(
      false,
    );
  });

  it('unsubscribes from dispatch::token event on unmount (no leak)', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_dispatch_providers') return Promise.resolve([]);
      if (cmd === 'dispatch_task') {
        return Promise.resolve({
          dispatch_id: 'd-unmount-1',
          started_at_unix: 1_700_000_000,
        });
      }
      return Promise.resolve(null);
    });

    const { unmount } = await renderScreen();
    await userEvent.type(screen.getByTestId('dispatch-prompt'), 'go');
    await userEvent.click(screen.getByTestId('dispatch-submit'));
    await waitFor(() => expect(lastHandler).not.toBeNull());

    unmount();
    await waitFor(() => expect(unlistenCalls).toBeGreaterThanOrEqual(1));
  });
});
