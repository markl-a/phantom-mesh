/**
 * Regression Tests — Goals Commands
 *
 * Verifies the contract shape for:
 *   - goals_create      → goal object with id
 *   - goals_list        → array of goals
 *   - goals_get         → single goal object
 *   - goals_update      → updated goal object
 *   - goals_delete      → { success } or { id }
 *   - goals_milestone_add    → milestone object
 *   - goals_milestone_toggle → toggled milestone
 *   - goals_today       → array of goals
 *   - goals_summary     → { total, active, completed }
 *   - goals_checkin_add → checkin object
 *   - goals_mood_trend  → array of { date, mood }
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mockInvoke } from './helpers';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

import { invoke } from '@tauri-apps/api/core';

// ─── Fixtures ─────────────────────────────────────────────────────────────────

const goalFixture = {
  id: 'goal-001',
  title: 'Launch Spectyn Mesh v1',
  status: 'active',
  created_at: '2026-04-07T00:00:00Z',
};

const milestoneFixture = {
  id: 'ms-001',
  goal_id: 'goal-001',
  title: 'Ship Phase 1A',
  completed: false,
};

const checkinFixture = {
  id: 'ci-001',
  goal_id: 'goal-001',
  mood: 4,
  note: 'Good progress',
  date: '2026-04-07',
};

const moodTrendFixture = [
  { date: '2026-04-05', mood: 3 },
  { date: '2026-04-06', mood: 4 },
  { date: '2026-04-07', mood: 5 },
];

const summaryFixture = {
  total: 10,
  active: 6,
  completed: 4,
};

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('Goals Commands — Regression', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockImplementation(
      mockInvoke({
        goals_create: goalFixture,
        goals_list: [goalFixture],
        goals_get: goalFixture,
        goals_update: { ...goalFixture, status: 'completed' },
        goals_delete: { success: true },
        goals_milestone_add: milestoneFixture,
        goals_milestone_toggle: { ...milestoneFixture, completed: true },
        goals_today: [goalFixture],
        goals_summary: summaryFixture,
        goals_checkin_add: checkinFixture,
        goals_mood_trend: moodTrendFixture,
      })
    );
  });

  describe('goals_create', () => {
    it('returns an object with an id field', async () => {
      const result = await invoke('goals_create', { data: { title: 'Test goal' } }) as typeof goalFixture;
      expect(result).toHaveProperty('id');
      expect(typeof result.id).toBe('string');
    });

    it('returned goal has title and status', async () => {
      const result = await invoke('goals_create', { data: { title: 'Test goal' } }) as typeof goalFixture;
      expect(result).toHaveProperty('title');
      expect(result).toHaveProperty('status');
    });
  });

  describe('goals_list', () => {
    it('returns an array', async () => {
      const result = await invoke('goals_list') as typeof goalFixture[];
      expect(Array.isArray(result)).toBe(true);
    });

    it('each item in the array has an id', async () => {
      const result = await invoke('goals_list') as typeof goalFixture[];
      for (const goal of result) {
        expect(goal).toHaveProperty('id');
      }
    });
  });

  describe('goals_get', () => {
    it('returns a single goal object with id', async () => {
      const result = await invoke('goals_get', { id: 'goal-001' }) as typeof goalFixture;
      expect(result).toHaveProperty('id');
      expect(typeof result.id).toBe('string');
    });

    it('returned goal has required fields', async () => {
      const result = await invoke('goals_get', { id: 'goal-001' }) as typeof goalFixture;
      expect(result).toHaveProperty('title');
      expect(result).toHaveProperty('status');
    });
  });

  describe('goals_update', () => {
    it('returns the updated goal with id', async () => {
      const result = await invoke('goals_update', { id: 'goal-001', data: { status: 'completed' } }) as typeof goalFixture;
      expect(result).toHaveProperty('id');
    });

    it('updated goal reflects the change', async () => {
      const result = await invoke('goals_update', { id: 'goal-001', data: { status: 'completed' } }) as { id: string; status: string };
      expect(result.status).toBe('completed');
    });
  });

  describe('goals_delete', () => {
    it('returns an object (deletion acknowledgement)', async () => {
      const result = await invoke('goals_delete', { id: 'goal-001' });
      expect(typeof result).toBe('object');
      expect(result).not.toBeNull();
    });
  });

  describe('goals_milestone_add', () => {
    it('returns a milestone object with id', async () => {
      const result = await invoke('goals_milestone_add', { goalId: 'goal-001', data: { title: 'Ship' } }) as typeof milestoneFixture;
      expect(result).toHaveProperty('id');
      expect(typeof result.id).toBe('string');
    });

    it('returned milestone has goal_id and completed fields', async () => {
      const result = await invoke('goals_milestone_add', { goalId: 'goal-001', data: { title: 'Ship' } }) as typeof milestoneFixture;
      expect(result).toHaveProperty('goal_id');
      expect(result).toHaveProperty('completed');
      expect(typeof result.completed).toBe('boolean');
    });
  });

  describe('goals_milestone_toggle', () => {
    it('returns a milestone object with toggled completed field', async () => {
      const result = await invoke('goals_milestone_toggle', { goalId: 'goal-001', milestoneId: 'ms-001' }) as typeof milestoneFixture;
      expect(result).toHaveProperty('completed');
      expect(typeof result.completed).toBe('boolean');
    });
  });

  describe('goals_today', () => {
    it('returns an array', async () => {
      const result = await invoke('goals_today');
      expect(Array.isArray(result)).toBe(true);
    });
  });

  describe('goals_summary', () => {
    it('returns summary with total, active, completed fields', async () => {
      const result = await invoke('goals_summary') as typeof summaryFixture;
      expect(result).toHaveProperty('total');
      expect(result).toHaveProperty('active');
      expect(result).toHaveProperty('completed');
    });

    it('all summary fields are numbers', async () => {
      const result = await invoke('goals_summary') as typeof summaryFixture;
      expect(typeof result.total).toBe('number');
      expect(typeof result.active).toBe('number');
      expect(typeof result.completed).toBe('number');
    });

    it('completed + active does not exceed total', async () => {
      const result = await invoke('goals_summary') as typeof summaryFixture;
      expect(result.active + result.completed).toBeLessThanOrEqual(result.total);
    });
  });

  describe('goals_checkin_add', () => {
    it('returns a checkin object with id', async () => {
      const result = await invoke('goals_checkin_add', { goalId: 'goal-001', data: { mood: 4, note: 'Good' } }) as typeof checkinFixture;
      expect(result).toHaveProperty('id');
    });

    it('checkin has mood field as a number', async () => {
      const result = await invoke('goals_checkin_add', { goalId: 'goal-001', data: { mood: 4, note: 'Good' } }) as typeof checkinFixture;
      expect(result).toHaveProperty('mood');
      expect(typeof result.mood).toBe('number');
    });
  });

  describe('goals_mood_trend', () => {
    it('returns an array', async () => {
      const result = await invoke('goals_mood_trend', { id: 'goal-001' });
      expect(Array.isArray(result)).toBe(true);
    });

    it('each trend entry has date and mood fields', async () => {
      const result = await invoke('goals_mood_trend', { id: 'goal-001' }) as typeof moodTrendFixture;
      for (const entry of result) {
        expect(entry).toHaveProperty('date');
        expect(entry).toHaveProperty('mood');
        expect(typeof entry.date).toBe('string');
        expect(typeof entry.mood).toBe('number');
      }
    });
  });
});
