import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { safeInvoke } from '../../src/lib/tauri-compat';

describe('tauri-compat — Browser Fallback Parity (Slice 02)', () => {
  const originalFetch = global.fetch;

  beforeEach(() => {
    // Mock global window
    global.window = {} as any;
    // @ts-ignore
    delete window.__TAURI__;
    
    // Mock fetch
    global.fetch = vi.fn();
  });

  afterEach(() => {
    global.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  it('send_message fallback aligns prompt and response shape', async () => {
    const mockResponse = {
      agent: 'master',
      result: 'Hello from the core!',
      tool_calls: [],
      elapsed: 0.42
    };

    (global.fetch as any).mockResolvedValue({
      ok: true,
      json: async () => mockResponse,
    });

    const result = await safeInvoke<any>('send_message', { prompt: 'Hi there' });

    // Verify request
    expect(global.fetch).toHaveBeenCalledWith(
      expect.stringContaining('/agent/master/run'),
      expect.objectContaining({
        method: 'POST',
        body: JSON.stringify({ prompt: 'Hi there' }),
      })
    );

    // Verify response parity (result -> output)
    expect(result).toEqual({
      agent: 'master',
      output: 'Hello from the core!',
      tool_calls: [],
      elapsed: 0.42
    });
  });

  it('send_message fallback supports explicit agent name', async () => {
    const mockResponse = {
      agent: 'researcher',
      result: 'Research findings...',
      tool_calls: [],
      elapsed: 1.0
    };

    (global.fetch as any).mockResolvedValue({
      ok: true,
      json: async () => mockResponse,
    });

    const result = await safeInvoke<any>('send_message', { prompt: 'Research X', agent: 'researcher' });

    // Verify request URL uses the explicit agent
    expect(global.fetch).toHaveBeenCalledWith(
      expect.stringContaining('/agent/researcher/run'),
      expect.any(Object)
    );

    expect(result.agent).toBe('researcher');
    expect(result.output).toBe('Research findings...');
  });

  it('send_message fallback handles legacy "message" arg for compatibility', async () => {
    (global.fetch as any).mockResolvedValue({
      ok: true,
      json: async () => ({ result: 'ok' }),
    });

    // Use "message" instead of "prompt"
    await safeInvoke('send_message', { message: 'Legacy prompt' });

    expect(global.fetch).toHaveBeenCalledWith(
      expect.any(String),
      expect.objectContaining({
        body: JSON.stringify({ prompt: 'Legacy prompt' }),
      })
    );
  });
});
