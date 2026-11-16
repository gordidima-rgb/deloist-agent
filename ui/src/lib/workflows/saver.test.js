import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Saver } from './saver.js';

function make(apiImpl, extra = {}) {
  const log = { states: [], saved: [], conflicts: [], errors: [] };
  const calls = [];
  const api = vi.fn(async (path, opts) => {
    calls.push({ path, opts });
    return apiImpl(path, opts, calls.length);
  });
  let graph = { nodes: [], edges: [], strokes: [] };
  const saver = new Saver({
    api,
    id: 'w1',
    revision: 5,
    getGraph: () => graph,
    on: {
      state: (s) => log.states.push(s),
      saved: (r) => log.saved.push(r),
      conflict: (w) => log.conflicts.push(w),
      error: (e) => log.errors.push(e),
    },
    ...extra,
  });
  return { saver, api, calls, log, setGraph: (g) => (graph = g) };
}

describe('Saver', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('debounces many edits into one PUT with the base revision', async () => {
    const { saver, calls, log, setGraph } = make(async () => ({ revision: 6 }));
    setGraph({ nodes: [1], edges: [], strokes: [] });
    saver.schedule();
    saver.schedule();
    vi.advanceTimersByTime(500);
    saver.schedule();
    vi.advanceTimersByTime(799);
    expect(calls).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(2);
    expect(calls).toHaveLength(1);
    expect(calls[0].path).toBe('/api/workflows/w1/graph');
    expect(calls[0].opts.method).toBe('PUT');
    expect(calls[0].opts.body).toEqual({ base_revision: 5, graph: { nodes: [1], edges: [], strokes: [] } });
    expect(saver.revision).toBe(6);
    expect(log.saved).toEqual([6]);
    expect(log.states.at(-1)).toBe('idle');
  });

  it('uses the new revision for the next save', async () => {
    const { saver, calls } = make(async (_p, _o, n) => ({ revision: 5 + n }));
    saver.schedule();
    await vi.advanceTimersByTimeAsync(900);
    saver.schedule();
    await vi.advanceTimersByTimeAsync(900);
    expect(calls.map((c) => c.opts.body.base_revision)).toEqual([5, 6]);
  });

  it('never has two saves in flight and writes once more after', async () => {
    let release;
    const gate = new Promise((r) => (release = r));
    const { saver, calls } = make(async (_p, _o, n) => {
      if (n === 1) await gate;
      return { revision: 5 + n };
    });
    saver.schedule();
    await vi.advanceTimersByTimeAsync(900);
    expect(calls).toHaveLength(1);
    saver.schedule();
    await vi.advanceTimersByTimeAsync(900);
    expect(calls).toHaveLength(1);
    release();
    await vi.advanceTimersByTimeAsync(10);
    expect(calls).toHaveLength(2);
    expect(calls[1].opts.body.base_revision).toBe(6);
  });

  it('on 409 hands the fresh workflow out and stops writing', async () => {
    const fresh = { id: 'w1', revision: 9 };
    const { saver, calls, log } = make(async () => {
      throw Object.assign(new Error('conflict'), { status: 409, data: { conflict: true, workflow: fresh } });
    });
    saver.schedule();
    await vi.advanceTimersByTimeAsync(900);
    expect(log.conflicts).toEqual([fresh]);
    expect(log.errors).toHaveLength(0);
    saver.schedule();
    await vi.advanceTimersByTimeAsync(900);
    expect(calls).toHaveLength(1);
  });

  it('on 400 reports the error with its node id and keeps trying later', async () => {
    const { saver, log } = make(async () => {
      throw Object.assign(new Error('bad'), { status: 400, data: { error: 'bad', code: 'bad_edge', node_id: 'n1' } });
    });
    saver.schedule();
    await vi.advanceTimersByTimeAsync(900);
    expect(log.errors[0].data.node_id).toBe('n1');
    expect(log.states.at(-1)).toBe('error');
    expect(saver.dead).toBe(false);
  });

  it('flush writes at once and close waits for pending work', async () => {
    const { saver, calls } = make(async () => ({ revision: 6 }));
    saver.schedule();
    await saver.close();
    expect(calls).toHaveLength(1);
    expect(saver.dead).toBe(true);
    expect(saver.pending).toBe(false);
  });
});
