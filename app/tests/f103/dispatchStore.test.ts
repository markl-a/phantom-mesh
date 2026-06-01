// F103 · Unit tests for the dispatch store reducer.
//
// Covers:
//   - startDispatch initializes state + sets current
//   - applyFrame token/status/done/error transitions
//   - terminal-phase guard (late stray frame after `done` is ignored)
//   - markCancelled is a no-op on terminal states
//   - reset clears everything
//
// Mirrors the E002 acceptance: phase transitions must be tested via the
// reducer surface so the UI is purely a render function over store
// state.

import { beforeEach, describe, expect, it } from 'vitest';
import {
  useDispatchStore,
  isTerminalPhase,
} from '../../src/stores/dispatchStore';

function seed(id = 'd-1') {
  useDispatchStore.getState().startDispatch({
    id,
    prompt: 'hi',
    caps: ['gpu'],
    provider: undefined,
    startedAt: 1_700_000_000_000,
  });
}

beforeEach(() => {
  useDispatchStore.getState().reset();
});

describe('dispatchStore', () => {
  it('startDispatch initializes a row and sets current', () => {
    seed('d-1');
    const s = useDispatchStore.getState();
    expect(s.current).toBe('d-1');
    expect(s.byId['d-1']).toMatchObject({
      id: 'd-1',
      prompt: 'hi',
      caps: ['gpu'],
      phase: 'submitting',
      tokens: [],
    });
  });

  it('token frame appends + flips submitting→running on first token', () => {
    seed();
    useDispatchStore
      .getState()
      .applyFrame('d-1', { type: 'token', text: 'hello ' });
    let s = useDispatchStore.getState();
    expect(s.byId['d-1'].phase).toBe('running');
    expect(s.byId['d-1'].tokens).toEqual(['hello ']);

    useDispatchStore.getState().applyFrame('d-1', { type: 'token', text: 'world' });
    s = useDispatchStore.getState();
    expect(s.byId['d-1'].tokens).toEqual(['hello ', 'world']);
  });

  it('status:queued and status:running transition phase correctly', () => {
    seed();
    useDispatchStore
      .getState()
      .applyFrame('d-1', { type: 'status', phase: 'queued' });
    expect(useDispatchStore.getState().byId['d-1'].phase).toBe('queued');

    useDispatchStore
      .getState()
      .applyFrame('d-1', { type: 'status', phase: 'running' });
    expect(useDispatchStore.getState().byId['d-1'].phase).toBe('running');
  });

  it('done frame transitions to done with result', () => {
    seed();
    useDispatchStore.getState().applyFrame('d-1', { type: 'token', text: 'h' });
    useDispatchStore
      .getState()
      .applyFrame('d-1', { type: 'done', result: 'final' });
    const row = useDispatchStore.getState().byId['d-1'];
    expect(row.phase).toBe('done');
    expect(row.result).toBe('final');
    expect(row.completedAt).toBeGreaterThan(0);
  });

  it('error frame transitions to failed with code+message', () => {
    seed();
    useDispatchStore
      .getState()
      .applyFrame('d-1', { type: 'error', code: 'E_NET', message: 'boom' });
    const row = useDispatchStore.getState().byId['d-1'];
    expect(row.phase).toBe('failed');
    expect(row.errorCode).toBe('E_NET');
    expect(row.errorMessage).toBe('boom');
  });

  it('status:cancelled frame transitions to cancelled', () => {
    seed();
    useDispatchStore
      .getState()
      .applyFrame('d-1', { type: 'status', phase: 'cancelled' });
    const row = useDispatchStore.getState().byId['d-1'];
    expect(row.phase).toBe('cancelled');
    expect(row.completedAt).toBeGreaterThan(0);
  });

  it('frames arriving after a terminal phase are ignored (race guard)', () => {
    seed();
    useDispatchStore
      .getState()
      .applyFrame('d-1', { type: 'done', result: 'ok' });
    // A late status:running should NOT undo the `done` transition.
    useDispatchStore
      .getState()
      .applyFrame('d-1', { type: 'status', phase: 'running' });
    const row = useDispatchStore.getState().byId['d-1'];
    expect(row.phase).toBe('done');
    expect(row.result).toBe('ok');
  });

  it('error frame DOES override `done` (server-side rollback signal)', () => {
    // F103 risk register: we explicitly allow error frames to override
    // a prior done in case the broker emits a late failure (rare but
    // possible — e.g. post-stream validation fails). Document the
    // contract via this test.
    seed();
    useDispatchStore.getState().applyFrame('d-1', { type: 'done', result: 'ok' });
    useDispatchStore
      .getState()
      .applyFrame('d-1', { type: 'error', code: 'E_LATE', message: 'rolled back' });
    const row = useDispatchStore.getState().byId['d-1'];
    expect(row.phase).toBe('failed');
    expect(row.errorCode).toBe('E_LATE');
  });

  it('applyFrame is a no-op for unknown dispatch ids', () => {
    seed();
    useDispatchStore
      .getState()
      .applyFrame('unknown', { type: 'token', text: 'x' });
    expect(useDispatchStore.getState().byId).not.toHaveProperty('unknown');
  });

  it('markCancelled is a no-op when phase is terminal', () => {
    seed();
    useDispatchStore.getState().applyFrame('d-1', { type: 'done', result: 'r' });
    useDispatchStore.getState().markCancelled('d-1');
    expect(useDispatchStore.getState().byId['d-1'].phase).toBe('done');
  });

  it('isTerminalPhase classifies done/failed/cancelled', () => {
    expect(isTerminalPhase('done')).toBe(true);
    expect(isTerminalPhase('failed')).toBe(true);
    expect(isTerminalPhase('cancelled')).toBe(true);
    expect(isTerminalPhase('running')).toBe(false);
    expect(isTerminalPhase('queued')).toBe(false);
    expect(isTerminalPhase('idle')).toBe(false);
    expect(isTerminalPhase('submitting')).toBe(false);
  });
});
