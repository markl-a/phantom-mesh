// IPC command-contract — captureFocus wrappers (lib/captureFocus).
//
// startSession / analyzeSession's BigInt wire-safety is already covered by
// tests/focus/captureFocus.wire.test.ts. This file pins the rest of the
// app↔core contract for SPEC-21 capture_focus: the EXACT Tauri command names,
// the arg-object shape (keys + JSON-safe types), the resolved-value handling,
// and the localStorage mirror that completeSession writes. Getting any command
// name or arg key wrong silently routes the call to the tauri-compat HTTP
// fallback (no-op) or 400s on a real device.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const safeInvoke = vi.fn();
vi.mock('../../src/lib/tauri-compat', () => ({
  isTauri: () => true,
  safeInvoke: (...args: unknown[]) => safeInvoke(...args),
}));

import {
  recordInterruption,
  completeSession,
  focusStatus,
  listRecent,
  clearRecent,
} from '../../src/lib/captureFocus';
import type { FocusSessionResult } from '../../src/lib/generated/capture_focus/FocusSessionResult';

beforeEach(() => {
  localStorage.clear();
  safeInvoke.mockReset();
});
afterEach(() => vi.restoreAllMocks());

describe('captureFocus — recordInterruption contract', () => {
  it('invokes focus_record_interruption with { sessionId, kind } (JSON-safe)', async () => {
    safeInvoke.mockResolvedValue(null);
    await recordInterruption('sess-1', 'notification');

    const [cmd, args] = safeInvoke.mock.calls[0] as [
      string,
      { sessionId: unknown; kind: unknown },
    ];
    expect(cmd).toBe('focus_record_interruption');
    expect(Object.keys(args).sort()).toEqual(['kind', 'sessionId']);
    expect(args.sessionId).toBe('sess-1');
    expect(args.kind).toBe('notification');
    expect(typeof args.sessionId).toBe('string');
    expect(() => JSON.stringify(args)).not.toThrow();
  });
});

describe('captureFocus — completeSession contract', () => {
  const result: FocusSessionResult = {
    actualDurationMs: BigInt(1500000),
    interruptions: 2,
    completionPct: 100,
    summary: 'done',
    suggestion: 'keep going',
  };

  it('invokes focus_complete_session with only { sessionId } and returns the result', async () => {
    safeInvoke.mockResolvedValue(result);
    const got = await completeSession('sess-2', { mode: 'pomodoro25', label: 'write' });

    const [cmd, args] = safeInvoke.mock.calls[0] as [string, { sessionId: unknown }];
    expect(cmd).toBe('focus_complete_session');
    expect(Object.keys(args)).toEqual(['sessionId']);
    expect(args.sessionId).toBe('sess-2');
    expect(() => JSON.stringify(args)).not.toThrow();
    expect(got).toBe(result);
  });

  it('mirrors the completed session into the listRecent localStorage cache', async () => {
    safeInvoke.mockResolvedValue(result);
    await completeSession('sess-3', { mode: 'deep_work50', label: 'study' });

    const recent = listRecent();
    expect(recent).toHaveLength(1);
    expect(recent[0].sessionId).toBe('sess-3');
    expect(recent[0].mode).toBe('deep_work50');
    expect(recent[0].label).toBe('study');
    expect(typeof recent[0].completedAtMs).toBe('number');
    // The mirrored result round-trips its BigInt actualDurationMs.
    expect(recent[0].result.actualDurationMs).toBe(BigInt(1500000));

    clearRecent();
    expect(listRecent()).toEqual([]);
  });

  it('does not mirror when the invoke rejects (error propagates)', async () => {
    safeInvoke.mockRejectedValue(new Error('focus.session_not_found: sess-x'));
    await expect(
      completeSession('sess-x', { mode: 'sprint10', label: null }),
    ).rejects.toThrow(/session_not_found/);
    expect(listRecent()).toEqual([]);
  });
});

describe('captureFocus — focusStatus contract', () => {
  it('invokes focus_status with an empty args object', async () => {
    safeInvoke.mockResolvedValue(null);
    await focusStatus();

    const [cmd, args] = safeInvoke.mock.calls[0] as [string, Record<string, unknown>];
    expect(cmd).toBe('focus_status');
    expect(args).toEqual({});
  });

  it('returns a typed ActiveFocus when the backend yields one', async () => {
    const active = {
      sessionId: 'sess-9',
      startedAtMs: 1700000000000,
      plannedDurationMs: 1500000,
      task: 'deep work',
      interruptions: 1,
    };
    safeInvoke.mockResolvedValue(active);
    const got = await focusStatus();
    expect(got).toEqual(active);
  });

  it('normalizes a malformed / empty backend response to null', async () => {
    safeInvoke.mockResolvedValue({ notASession: true });
    expect(await focusStatus()).toBeNull();

    safeInvoke.mockResolvedValue(null);
    expect(await focusStatus()).toBeNull();
  });
});
