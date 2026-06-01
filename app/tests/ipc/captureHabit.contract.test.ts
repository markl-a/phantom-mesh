// IPC command-contract — captureHabit wrappers (lib/captureHabit).
//
// SPEC-22 habit chip capture. Pins the four Tauri command names + arg shapes:
//   habit_checkin({ checkin }) → HabitStreak   (timestampMs must be a number)
//   habit_list()              → HabitSummary[]
//   habit_create({ def })     → void
//   habit_streak({ habitSlug })→ HabitStreak
// A wrong command name / arg key drops the call into the tauri-compat HTTP
// fallback (silent no-op — the live "streak never persisted" bug), so these
// are regression guards on the wire itself.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const safeInvoke = vi.fn();
vi.mock('../../src/lib/tauri-compat', () => ({
  isTauri: () => true,
  safeInvoke: (...args: unknown[]) => safeInvoke(...args),
}));

import {
  buildCheckin,
  recordCheckin,
  listHabits,
  createHabit,
  streak,
  ensureCheckin,
} from '../../src/lib/captureHabit';
import type { HabitDefinition } from '../../src/lib/generated/capture_habit/HabitDefinition';
import type { HabitStreak } from '../../src/lib/generated/capture_habit/HabitStreak';
import type { HabitSummary } from '../../src/lib/generated/capture_habit/HabitSummary';

const streakOf = (slug: string): HabitStreak => ({
  habitSlug: slug,
  currentStreak: 1,
  longestStreak: 1,
  lastCheckinAt: null,
});

const summaryOf = (slug: string): HabitSummary => ({
  habitSlug: slug,
  last7dCount: 3,
  last30dCount: 9,
  lastCheckinAt: null,
  streak: streakOf(slug),
});

beforeEach(() => safeInvoke.mockReset());
afterEach(() => vi.restoreAllMocks());

describe('captureHabit — recordCheckin contract', () => {
  it('invokes habit_checkin with { checkin } whose timestampMs is a number (JSON-safe)', async () => {
    safeInvoke.mockResolvedValue(streakOf('water'));
    const got = await recordCheckin(buildCheckin('water', { note: '250ml' }));

    const [cmd, args] = safeInvoke.mock.calls[0] as [
      string,
      { checkin: { habitSlug: unknown; timestampMs: unknown; note: unknown; source: unknown } },
    ];
    expect(cmd).toBe('habit_checkin');
    expect(Object.keys(args)).toEqual(['checkin']);
    expect(args.checkin.habitSlug).toBe('water');
    expect(args.checkin.note).toBe('250ml');
    expect(args.checkin.source).toBe('manual');
    // buildCheckin produces a BigInt timestampMs; the wrapper must coerce it.
    expect(typeof args.checkin.timestampMs).toBe('number');
    expect(typeof args.checkin.timestampMs).not.toBe('bigint');
    expect(() => JSON.stringify(args)).not.toThrow();
    expect(got).toEqual(streakOf('water'));
  });
});

describe('captureHabit — listHabits contract', () => {
  it('invokes habit_list with no args and returns the summary array', async () => {
    const summaries: HabitSummary[] = [summaryOf('water')];
    safeInvoke.mockResolvedValue(summaries);
    const got = await listHabits();

    const [cmd, args] = safeInvoke.mock.calls[0] as [string, unknown];
    expect(cmd).toBe('habit_list');
    expect(args).toBeUndefined();
    expect(got).toEqual(summaries);
  });
});

describe('captureHabit — createHabit contract', () => {
  it('invokes habit_create with { def } unchanged', async () => {
    safeInvoke.mockResolvedValue(null);
    const def: HabitDefinition = {
      slug: 'read',
      label: '讀書',
      targetFrequency: { kind: 'daily' },
      tags: [],
      createdAt: '2026-05-31T00:00:00.000Z',
    };
    await createHabit(def);

    const [cmd, args] = safeInvoke.mock.calls[0] as [string, { def: HabitDefinition }];
    expect(cmd).toBe('habit_create');
    expect(Object.keys(args)).toEqual(['def']);
    expect(args.def).toEqual(def);
    expect(() => JSON.stringify(args)).not.toThrow();
  });
});

describe('captureHabit — streak contract', () => {
  it('invokes habit_streak with { habitSlug }', async () => {
    safeInvoke.mockResolvedValue(streakOf('coffee'));
    const got = await streak('coffee');

    const [cmd, args] = safeInvoke.mock.calls[0] as [string, { habitSlug: unknown }];
    expect(cmd).toBe('habit_streak');
    expect(Object.keys(args)).toEqual(['habitSlug']);
    expect(args.habitSlug).toBe('coffee');
    expect(got).toEqual(streakOf('coffee'));
  });
});

describe('captureHabit — ensureCheckin contract', () => {
  it('creates the chip then checks in when the slug is not yet in the palette', async () => {
    // 1st invoke = habit_list (empty), 2nd = habit_create, 3rd = habit_checkin.
    safeInvoke
      .mockResolvedValueOnce([] as HabitSummary[]) // habit_list
      .mockResolvedValueOnce(null) // habit_create
      .mockResolvedValueOnce(streakOf('walk')); // habit_checkin

    const got = await ensureCheckin('walk', '走路', { note: '15min' });

    const cmds = safeInvoke.mock.calls.map((c) => c[0]);
    expect(cmds).toEqual(['habit_list', 'habit_create', 'habit_checkin']);

    const createArgs = safeInvoke.mock.calls[1][1] as { def: HabitDefinition };
    expect(createArgs.def.slug).toBe('walk');
    expect(createArgs.def.label).toBe('走路');
    expect(createArgs.def.targetFrequency).toEqual({ kind: 'daily' });

    const checkinArgs = safeInvoke.mock.calls[2][1] as {
      checkin: { habitSlug: string; timestampMs: unknown; note: unknown };
    };
    expect(checkinArgs.checkin.habitSlug).toBe('walk');
    expect(checkinArgs.checkin.note).toBe('15min');
    expect(typeof checkinArgs.checkin.timestampMs).toBe('number');
    expect(got).toEqual(streakOf('walk'));
  });

  it('skips habit_create when the chip already exists', async () => {
    safeInvoke
      .mockResolvedValueOnce([summaryOf('water')] as HabitSummary[]) // habit_list
      .mockResolvedValueOnce(streakOf('water')); // habit_checkin

    await ensureCheckin('water', '水');

    const cmds = safeInvoke.mock.calls.map((c) => c[0]);
    expect(cmds).toEqual(['habit_list', 'habit_checkin']);
    expect(cmds).not.toContain('habit_create');
  });
});
