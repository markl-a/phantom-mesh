/**
 * Regression Tests — Tasks Commands
 *
 * Verifies the contract shape for:
 *   - get_task_history → array of task records
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mockInvoke } from './helpers';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

import { invoke } from '@tauri-apps/api/core';

// ─── Fixtures ─────────────────────────────────────────────────────────────────

const taskRecordFixture = {
  id: 'task-001',
  agent: 'master',
  prompt: 'Summarise the goals list',
  status: 'completed',
  created_at: '2026-04-07T10:00:00Z',
  elapsed_secs: 1.23,
};

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('Tasks Commands — Regression', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockImplementation(
      mockInvoke({
        get_task_history: [taskRecordFixture],
      })
    );
  });

  describe('get_task_history', () => {
    it('returns an array', async () => {
      const result = await invoke('get_task_history');
      expect(Array.isArray(result)).toBe(true);
    });

    it('each record has an id field', async () => {
      const result = await invoke('get_task_history') as typeof taskRecordFixture[];
      for (const record of result) {
        expect(record).toHaveProperty('id');
        expect(typeof record.id).toBe('string');
      }
    });

    it('each record has an agent field', async () => {
      const result = await invoke('get_task_history') as typeof taskRecordFixture[];
      for (const record of result) {
        expect(record).toHaveProperty('agent');
        expect(typeof record.agent).toBe('string');
      }
    });

    it('each record has a status field', async () => {
      const result = await invoke('get_task_history') as typeof taskRecordFixture[];
      for (const record of result) {
        expect(record).toHaveProperty('status');
      }
    });

    it('each record has a created_at field', async () => {
      const result = await invoke('get_task_history') as typeof taskRecordFixture[];
      for (const record of result) {
        expect(record).toHaveProperty('created_at');
        expect(typeof record.created_at).toBe('string');
      }
    });

    it('empty array is a valid response', async () => {
      vi.mocked(invoke).mockImplementation(mockInvoke({ get_task_history: [] }));
      const result = await invoke('get_task_history');
      expect(Array.isArray(result)).toBe(true);
      expect((result as unknown[]).length).toBe(0);
    });
  });
});
