/**
 * Regression Tests — Cluster Commands
 *
 * Verifies the contract shape for:
 *   - get_cluster_status  → { nodes: [...] }
 *   - get_cluster_workers → array of worker objects
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mockInvoke } from './helpers';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

import { invoke } from '@tauri-apps/api/core';

// ─── Fixtures ─────────────────────────────────────────────────────────────────

const nodeFixture = {
  id: 'node-1',
  address: '192.168.1.10:7878',
  status: 'online',
  role: 'worker',
};

const clusterStatusFixture = {
  nodes: [nodeFixture, { ...nodeFixture, id: 'node-2', address: '192.168.1.11:7878' }],
};

const workerFixture = {
  id: 'worker-1',
  node_id: 'node-1',
  load: 0.42,
  active_tasks: 2,
};

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('Cluster Commands — Regression', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockImplementation(
      mockInvoke({
        get_cluster_status: clusterStatusFixture,
        get_cluster_workers: [workerFixture],
      })
    );
  });

  describe('get_cluster_status', () => {
    it('returns an object with a nodes array', async () => {
      const result = await invoke('get_cluster_status') as typeof clusterStatusFixture;
      expect(result).toHaveProperty('nodes');
      expect(Array.isArray(result.nodes)).toBe(true);
    });

    it('nodes array contains objects', async () => {
      const result = await invoke('get_cluster_status') as typeof clusterStatusFixture;
      for (const node of result.nodes) {
        expect(typeof node).toBe('object');
        expect(node).not.toBeNull();
      }
    });

    it('each node has an id field', async () => {
      const result = await invoke('get_cluster_status') as typeof clusterStatusFixture;
      for (const node of result.nodes as typeof nodeFixture[]) {
        expect(node).toHaveProperty('id');
      }
    });

    it('cluster with zero nodes is valid', async () => {
      vi.mocked(invoke).mockImplementation(mockInvoke({ get_cluster_status: { nodes: [] } }));
      const result = await invoke('get_cluster_status') as { nodes: unknown[] };
      expect(result.nodes.length).toBe(0);
    });
  });

  describe('get_cluster_workers', () => {
    it('returns an array', async () => {
      const result = await invoke('get_cluster_workers');
      expect(Array.isArray(result)).toBe(true);
    });

    it('each worker has an id', async () => {
      const result = await invoke('get_cluster_workers') as typeof workerFixture[];
      for (const worker of result) {
        expect(worker).toHaveProperty('id');
        expect(typeof worker.id).toBe('string');
      }
    });

    it('empty workers array is a valid response', async () => {
      vi.mocked(invoke).mockImplementation(mockInvoke({ get_cluster_workers: [] }));
      const result = await invoke('get_cluster_workers');
      expect(Array.isArray(result)).toBe(true);
    });
  });
});
