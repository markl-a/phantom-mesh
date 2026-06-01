// Regression test — captureFocus BigInt wire bug (found live on the emulator:
// tapping "開始" in the focus tab threw "Do not know how to serialize a
// BigInt"). FocusSessionRequest.plannedDurationMs and FocusSessionResult.
// actualDurationMs are BigInt (ts-rs u64); Tauri's invoke can't serialize them.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const safeInvoke = vi.fn();
vi.mock('../../src/lib/tauri-compat', () => ({
  isTauri: () => true,
  safeInvoke: (...args: unknown[]) => safeInvoke(...args),
}));

import {
  buildSessionRequest,
  startSession,
  analyzeSession,
} from '../../src/lib/captureFocus';
import type { FocusSessionResult } from '../../src/lib/generated/capture_focus/FocusSessionResult';

beforeEach(() => safeInvoke.mockReset());
afterEach(() => vi.restoreAllMocks());

describe('captureFocus — BigInt wire safety', () => {
  it('startSession sends plannedDurationMs as a number (JSON-safe)', async () => {
    safeInvoke.mockResolvedValue('session-1');
    await startSession(buildSessionRequest('pomodoro25'));

    const [cmd, args] = safeInvoke.mock.calls[0] as [
      string,
      { req: { plannedDurationMs: unknown } },
    ];
    expect(cmd).toBe('focus_start_session');
    expect(typeof args.req.plannedDurationMs).toBe('number');
    expect(() => JSON.stringify(args)).not.toThrow();
  });

  it('analyzeSession sends actualDurationMs as a number (JSON-safe)', async () => {
    safeInvoke.mockResolvedValue({});
    const result: FocusSessionResult = {
      actualDurationMs: BigInt(123456),
      interruptions: 0,
      completionPct: 100,
      summary: '',
      suggestion: '',
    };
    await analyzeSession(result);

    const [, args] = safeInvoke.mock.calls[0] as [
      string,
      { result: { actualDurationMs: unknown } },
    ];
    expect(typeof args.result.actualDurationMs).toBe('number');
    expect(() => JSON.stringify(args)).not.toThrow();
  });
});
