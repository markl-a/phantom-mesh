// Regression test — onboardingFsm BigInt wire bug.
//
// advanceOnboarding/rollbackOnboarding pass the snapshot to Tauri's invoke.
// The snapshot's enteredAtMs is a BigInt (ts-rs maps the Rust u64), which
// Tauri's IPC cannot serialize — it throws "Do not know how to serialize a
// BigInt", which on a device blocked the entire onboarding flow (caught and
// shown as "無法推進 onboarding"). These tests pin that the invoke args carry
// a plain number, so the serialization can't regress.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const safeInvoke = vi.fn();
vi.mock('../../src/lib/tauri-compat', () => ({
  isTauri: () => true,
  safeInvoke: (...args: unknown[]) => safeInvoke(...args),
}));

import { advanceOnboarding, rollbackOnboarding } from '../../src/lib/onboardingFsm';

beforeEach(() => {
  localStorage.clear();
  safeInvoke.mockReset();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('onboardingFsm — BigInt wire safety', () => {
  it('advanceOnboarding sends enteredAtMs as a number, never a bigint', async () => {
    safeInvoke.mockResolvedValue('picked_language');
    await advanceOnboarding({ demoRelayUsed: true });

    expect(safeInvoke).toHaveBeenCalledWith(
      'onboarding_advance',
      expect.objectContaining({
        snapshot: expect.objectContaining({ enteredAtMs: expect.any(Number) }),
      }),
    );
    const [, args] = safeInvoke.mock.calls[0] as [string, { snapshot: { enteredAtMs: unknown } }];
    expect(typeof args.snapshot.enteredAtMs).toBe('number');
    expect(typeof args.snapshot.enteredAtMs).not.toBe('bigint');
    // The args must be JSON-serializable (what Tauri's IPC does internally).
    expect(() => JSON.stringify(args)).not.toThrow();
  });

  it('rollbackOnboarding also sends a number enteredAtMs', async () => {
    safeInvoke.mockResolvedValue('created_identity');
    await rollbackOnboarding();

    const [, args] = safeInvoke.mock.calls[0] as [string, { snapshot: { enteredAtMs: unknown } }];
    expect(typeof args.snapshot.enteredAtMs).toBe('number');
    expect(() => JSON.stringify(args)).not.toThrow();
  });

  it('persists a JSON-safe (number enteredAtMs) snapshot to localStorage', async () => {
    safeInvoke.mockResolvedValue('picked_language');
    await advanceOnboarding({});

    const raw = localStorage.getItem('spectyn_mesh_onboarding_snapshot');
    expect(raw).toBeTruthy();
    const parsed = JSON.parse(raw as string);
    expect(parsed.currentState).toBe('picked_language');
    expect(typeof parsed.enteredAtMs).toBe('number');
  });

  it('falls back client-side (no throw) when the backend is not_yet_wired', async () => {
    safeInvoke.mockRejectedValue(
      new Error('onboarding.not_yet_wired: SPEC-28 Stage 3 deferred'),
    );
    const res = await advanceOnboarding({});
    expect(res.softFailed).toBe(true);
    expect(res.state).toBeTruthy();
  });
});
