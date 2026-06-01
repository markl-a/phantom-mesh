// IPC command-contract — captureFood (lib/captureFood) + recall (lib/recall).
//
// captureFood: food_analyze({ request }) where request.timestampMs is a BigInt
// (ts-rs i64) that MUST be coerced to a number before the wire — the same
// "Do not know how to serialize a BigInt" class that crashed focus/onboarding.
// recall: recall_search({ query, kind, since, limit }) — pins the exact arg
// keys (null defaults for optional fields) and the array-normalization of the
// response. Both wrappers also expose error-mapping helpers.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const safeInvoke = vi.fn();
vi.mock('../../src/lib/tauri-compat', () => ({
  isTauri: () => true,
  safeInvoke: (...args: unknown[]) => safeInvoke(...args),
}));

import {
  buildFoodRequest,
  analyzeFood,
  describeFoodError,
  FOOD_LOG_KIND,
} from '../../src/lib/captureFood';
import { recallSearch } from '../../src/lib/recall';
import type { FoodAnalysisResult } from '../../src/lib/generated/capture_food/FoodAnalysisResult';

const foodResult: FoodAnalysisResult = {
  summary: 'balanced plate',
  macroEstimate: { proteinG: 30, carbsG: 40, fatG: 20, calories: 500 },
  fatLossScore: 0.7,
  suggestion: 'more protein',
  confidence: 0.9,
};

beforeEach(() => safeInvoke.mockReset());
afterEach(() => vi.restoreAllMocks());

describe('captureFood — analyzeFood contract', () => {
  it('invokes food_analyze with { request } whose timestampMs is a number (JSON-safe)', async () => {
    safeInvoke.mockResolvedValue(foodResult);
    const got = await analyzeFood(buildFoodRequest('chicken salad'));

    const [cmd, args] = safeInvoke.mock.calls[0] as [
      string,
      { request: { text: unknown; imagePath: unknown; kind: unknown; tag: unknown; timestampMs: unknown } },
    ];
    expect(cmd).toBe('food_analyze');
    expect(Object.keys(args)).toEqual(['request']);
    expect(args.request.text).toBe('chicken salad');
    expect(args.request.imagePath).toBeNull();
    expect(args.request.kind).toBe(FOOD_LOG_KIND);
    expect(args.request.tag).toEqual(['fat_loss']);
    // buildFoodRequest sets a BigInt timestampMs; the wrapper must coerce it.
    expect(typeof args.request.timestampMs).toBe('number');
    expect(typeof args.request.timestampMs).not.toBe('bigint');
    expect(() => JSON.stringify(args)).not.toThrow();
    expect(got).toEqual(foodResult);
  });

  it('maps the not_yet_wired wire error to a UI string', () => {
    expect(describeFoodError('food.not_yet_wired: SPEC-20 Stage 4')).toContain('尚未實作');
    expect(describeFoodError('food.image_too_large')).toContain('圖片太大');
    expect(describeFoodError('weird-unmapped')).toBe('weird-unmapped');
  });
});

describe('recall — recallSearch contract', () => {
  it('invokes recall_search with all four arg keys, defaulting optionals', async () => {
    safeInvoke.mockResolvedValue([]);
    await recallSearch({ query: 'salad' });

    const [cmd, args] = safeInvoke.mock.calls[0] as [
      string,
      { query: unknown; kind: unknown; since: unknown; limit: unknown },
    ];
    expect(cmd).toBe('recall_search');
    expect(Object.keys(args).sort()).toEqual(['kind', 'limit', 'query', 'since']);
    expect(args.query).toBe('salad');
    expect(args.kind).toBeNull();
    expect(args.since).toBeNull();
    expect(args.limit).toBe(50);
    expect(typeof args.limit).toBe('number');
  });

  it('passes through explicit kind / since / limit', async () => {
    safeInvoke.mockResolvedValue([]);
    await recallSearch({ query: '', kind: 'focus', since: '2026-01-01', limit: 10 });

    const [, args] = safeInvoke.mock.calls[0] as [
      string,
      { query: unknown; kind: unknown; since: unknown; limit: unknown },
    ];
    expect(args.query).toBe('');
    expect(args.kind).toBe('focus');
    expect(args.since).toBe('2026-01-01');
    expect(args.limit).toBe(10);
  });

  it('returns the hit array on success', async () => {
    const hits = [
      { eventId: 'e1', timestamp: '2026-05-31T00:00:00Z', kind: 'food', summary: 'lunch' },
    ];
    safeInvoke.mockResolvedValue(hits);
    expect(await recallSearch({ query: 'lunch' })).toEqual(hits);
  });

  it('normalizes a non-array backend response to an empty array', async () => {
    safeInvoke.mockResolvedValue({ unexpected: true });
    expect(await recallSearch({ query: 'x' })).toEqual([]);
  });
});
