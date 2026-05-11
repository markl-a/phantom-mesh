/**
 * Regression Tests — Memory Commands
 *
 * Verifies the contract shape for:
 *   - get_memory_observations → array of observation objects
 *   - search_memory           → search results (array)
 *   - get_memory_stats        → stats object
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mockInvoke } from './helpers';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

import { invoke } from '@tauri-apps/api/core';

// ─── Fixtures ─────────────────────────────────────────────────────────────────

const observationFixture = {
  id: 'obs-001',
  content: 'User prefers dark mode',
  source: 'chat',
  created_at: '2026-04-07T09:00:00Z',
  tags: ['ui', 'preference'],
};

const statsFixture = {
  total_observations: 42,
  unique_sources: 3,
  oldest_observation: '2026-01-01T00:00:00Z',
  newest_observation: '2026-04-07T09:00:00Z',
};

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('Memory Commands — Regression', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockImplementation(
      mockInvoke({
        get_memory_observations: [observationFixture],
        search_memory: [observationFixture],
        get_memory_stats: statsFixture,
      })
    );
  });

  describe('get_memory_observations', () => {
    it('returns an array', async () => {
      const result = await invoke('get_memory_observations');
      expect(Array.isArray(result)).toBe(true);
    });

    it('each observation has an id field', async () => {
      const result = await invoke('get_memory_observations') as typeof observationFixture[];
      for (const obs of result) {
        expect(obs).toHaveProperty('id');
        expect(typeof obs.id).toBe('string');
      }
    });

    it('each observation has a content field', async () => {
      const result = await invoke('get_memory_observations') as typeof observationFixture[];
      for (const obs of result) {
        expect(obs).toHaveProperty('content');
        expect(typeof obs.content).toBe('string');
      }
    });

    it('each observation has a created_at string', async () => {
      const result = await invoke('get_memory_observations') as typeof observationFixture[];
      for (const obs of result) {
        expect(obs).toHaveProperty('created_at');
        expect(typeof obs.created_at).toBe('string');
      }
    });

    it('empty array is a valid response', async () => {
      vi.mocked(invoke).mockImplementation(mockInvoke({ get_memory_observations: [] }));
      const result = await invoke('get_memory_observations');
      expect(Array.isArray(result)).toBe(true);
      expect((result as unknown[]).length).toBe(0);
    });
  });

  describe('search_memory', () => {
    it('returns an array for a given query', async () => {
      const result = await invoke('search_memory', { query: 'dark mode' });
      expect(Array.isArray(result)).toBe(true);
    });

    it('search results have the same shape as observations', async () => {
      const result = await invoke('search_memory', { query: 'dark mode' }) as typeof observationFixture[];
      for (const obs of result) {
        expect(obs).toHaveProperty('id');
        expect(obs).toHaveProperty('content');
      }
    });

    it('empty result set is valid', async () => {
      vi.mocked(invoke).mockImplementation(mockInvoke({ search_memory: [] }));
      const result = await invoke('search_memory', { query: 'nonexistent' });
      expect(Array.isArray(result)).toBe(true);
    });
  });

  describe('get_memory_stats', () => {
    it('returns an object', async () => {
      const result = await invoke('get_memory_stats');
      expect(typeof result).toBe('object');
      expect(result).not.toBeNull();
    });

    it('total_observations is a non-negative number', async () => {
      const result = await invoke('get_memory_stats') as typeof statsFixture;
      expect(result).toHaveProperty('total_observations');
      expect(typeof result.total_observations).toBe('number');
      expect(result.total_observations).toBeGreaterThanOrEqual(0);
    });
  });
});
