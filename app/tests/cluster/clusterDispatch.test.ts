// Unit tests — clusterDispatch (lib/clusterDispatch).
//
// This is the iOS chat cluster-mode wire: HMAC-sign the body, POST
// /rpc/task/assign, poll /rpc/task/status/:id until done|error. It was
// E2E-verified against a live mac coordinator (demo-mobile-swarm.sh returns
// real LLM answers), but the wire details — auth header format and request
// body shape — deserve a fast regression guard: get them wrong and every
// dispatch 401s or 400s.
//
// In jsdom the navigator UA has no iPhone token, so httpFetch resolves to the
// @tauri-apps/plugin-http `fetch`, which we mock.

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { createHmac } from 'node:crypto';

const fetchMock = vi.fn();
vi.mock('@tauri-apps/plugin-http', () => ({ fetch: (...a: unknown[]) => fetchMock(...a) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

import { dispatchToCluster } from '../../src/lib/clusterDispatch';

type FetchInit = { method?: string; headers?: Record<string, string>; body?: string };

function resp(status: number, json: unknown) {
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: '',
    text: async () => JSON.stringify(json),
    json: async () => json,
  };
}

beforeEach(() => {
  fetchMock.mockReset();
});

describe('dispatchToCluster guards', () => {
  it('rejects a missing coordinator URL without any network call', async () => {
    const r = await dispatchToCluster({ coordinatorUrl: '', secret: 's', agent: 'master', prompt: 'hi' });
    expect(r).toEqual({ ok: false, error: 'coordinator URL missing' });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('rejects a missing secret without any network call', async () => {
    const r = await dispatchToCluster({ coordinatorUrl: 'http://x:7878', secret: '', agent: 'master', prompt: 'hi' });
    expect(r).toEqual({ ok: false, error: 'cluster secret missing' });
    expect(fetchMock).not.toHaveBeenCalled();
  });
});

describe('dispatchToCluster wire format', () => {
  it('signs the body with HMAC-SHA256 and posts the {agent,prompt} shape', async () => {
    const secret = 's3cr3t-test';
    const agent = 'master';
    const prompt = 'ping';
    fetchMock
      .mockResolvedValueOnce(resp(202, { job_id: 'job-1' }))            // assign
      .mockResolvedValueOnce(resp(200, { status: 'done', output: 'pong' })); // status poll

    const r = await dispatchToCluster({
      coordinatorUrl: 'http://coord:7878/', secret, agent, prompt,
      pollIntervalMs: 1, maxWaitMs: 2000,
    });

    expect(r.ok).toBe(true);
    expect(r.output).toBe('pong');
    expect(r.jobId).toBe('job-1');

    // First call = assign. Verify URL, body shape, and auth header.
    const [assignUrl, assignInit] = fetchMock.mock.calls[0] as [string, FetchInit];
    expect(assignUrl).toBe('http://coord:7878/rpc/task/assign'); // trailing slash stripped
    expect(assignInit.method).toBe('POST');
    const expectedBody = JSON.stringify({ agent, prompt });
    expect(assignInit.body).toBe(expectedBody);
    const expectedAuth = createHmac('sha256', secret).update(expectedBody).digest('hex');
    expect(assignInit.headers?.['X-Cluster-Auth']).toBe(expectedAuth);

    // Second call = status poll for the returned job id.
    const [statusUrl] = fetchMock.mock.calls[1] as [string];
    expect(statusUrl).toBe('http://coord:7878/rpc/task/status/job-1');
  });

  it('returns an error when assign is rejected', async () => {
    fetchMock.mockResolvedValueOnce(resp(401, { error: 'bad auth' }));
    const r = await dispatchToCluster({ coordinatorUrl: 'http://c:7878', secret: 's', agent: 'master', prompt: 'hi' });
    expect(r.ok).toBe(false);
    expect(r.error).toContain('401');
  });

  it('surfaces a task-level error from the status poll', async () => {
    fetchMock
      .mockResolvedValueOnce(resp(202, { job_id: 'job-2' }))
      .mockResolvedValueOnce(resp(200, { status: 'error', error: 'all providers failed' }));
    const r = await dispatchToCluster({
      coordinatorUrl: 'http://c:7878', secret: 's', agent: 'master', prompt: 'hi',
      pollIntervalMs: 1, maxWaitMs: 2000,
    });
    expect(r.ok).toBe(false);
    expect(r.error).toBe('all providers failed');
    expect(r.jobId).toBe('job-2');
  });
});
