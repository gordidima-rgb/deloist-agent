import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import WorkflowsScreen from './WorkflowsScreen.svelte';
import { starterGraph } from '../../lib/workflows/graph.js';

// Svelte Flow меряет элементы: в jsdom для этого нужны заглушки.
beforeAll(() => {
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  globalThis.DOMMatrixReadOnly ??= class {
    constructor() {
      this.m22 = 1;
    }
  };
  Element.prototype.scrollTo ??= () => {};
  window.matchMedia ??= (query) => ({ matches: false, media: query, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
});

let host;
let instance;
async function drain(n = 25) {
  for (let i = 0; i < n; i += 1) {
    flushSync();
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

function backend(over = {}) {
  const wf = { id: 'w1', title: 'Первый', revision: 2, graph: starterGraph(), last_run: { status: 'done', at: 1700000000 } };
  const state = { wf, calls: [], list: [{ id: 'w1', title: 'Первый', steps: 3, last_run: wf.last_run }] };
  const api = vi.fn(async (path, opts = {}) => {
    const method = opts.method || 'GET';
    state.calls.push(`${method} ${path}`);
    if (over[`${method} ${path}`]) return over[`${method} ${path}`](opts);
    if (path === '/api/workflows' && method === 'GET') return { workflows: state.list };
    if (path === '/api/workflows' && method === 'POST') {
      const w = { id: 'w2', title: opts.body.title, revision: 1, graph: opts.body.graph };
      state.list = [...state.list, { id: 'w2', title: w.title, steps: 3, last_run: null }];
      return { workflow: w };
    }
    if (path === '/api/workflows/w1' && method === 'GET') return { workflow: state.wf, triggers: [] };
    if (path === '/api/workflows/w2' && method === 'GET') return { workflow: { id: 'w2', title: 'Новый процесс', revision: 1, graph: starterGraph() }, triggers: [] };
    if (path === '/api/workflows/w1' && method === 'DELETE') {
      state.list = [];
      return { ok: true };
    }
    if (path.endsWith('/runs?limit=1')) return { runs: [] };
    throw Object.assign(new Error('unexpected ' + method + ' ' + path), { status: 404 });
  });
  return { api, state };
}

async function render(api) {
  try {
    localStorage.clear();
  } catch {}
  host = document.createElement('div');
  host.style.cssText = 'width:1200px;height:800px';
  document.body.append(host);
  instance = mount(WorkflowsScreen, { target: host, props: { api, bots: [{ id: 'a', name: 'Аналитик', color: '#4fb4ff' }] } });
  await drain();
}

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
});

describe('WorkflowsScreen', () => {
  it('lists workflows and opens the first one on the canvas', async () => {
    const { api, state } = backend();
    await render(api);
    expect(host.textContent).toContain('Первый');
    expect(host.textContent).toContain('Шагов: 3');
    expect(state.calls).toContain('GET /api/workflows/w1');
    expect(host.querySelectorAll('.svelte-flow__node').length).toBe(3);
  });

  it('shows the empty state with a create button when there are no workflows', async () => {
    const { api } = backend({ 'GET /api/workflows': async () => ({ workflows: [] }) });
    await render(api);
    expect(host.textContent).toContain('Соберите процесс из шагов');
    expect([...host.querySelectorAll('button')].some((b) => b.textContent.includes('Создать процесс'))).toBe(true);
  });

  it('creates a workflow from the starter graph and opens it', async () => {
    const { api, state } = backend();
    await render(api);
    host.querySelector('.wfl-new').click();
    await drain();
    expect(state.calls).toContain('POST /api/workflows');
    expect(state.calls).toContain('GET /api/workflows/w2');
  });

  it('deletes only after a second click on the trash button', async () => {
    const { api, state } = backend();
    await render(api);
    const trash = host.querySelectorAll('.wfl-acts button')[2];
    trash.click();
    await drain(3);
    expect(state.calls).not.toContain('DELETE /api/workflows/w1');
    host.querySelectorAll('.wfl-acts button')[2].click();
    await drain();
    expect(state.calls).toContain('DELETE /api/workflows/w1');
  });

  it('saves an edit with the loaded revision and reloads on a 409', async () => {
    const conflict = { id: 'w1', title: 'Первый', revision: 7, graph: { nodes: [], edges: [], strokes: [] } };
    let puts = 0;
    const { api, state } = backend({
      'PUT /api/workflows/w1/graph': async (opts) => {
        puts += 1;
        expect(opts.body.base_revision).toBe(2);
        throw Object.assign(new Error('conflict'), { status: 409, data: { conflict: true, workflow: conflict } });
      },
    });
    await render(api);
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    try {
      const area = host.querySelector('.wfn-area');
      area.value = 'новое поручение';
      area.dispatchEvent(new Event('input', { bubbles: true }));
      await vi.advanceTimersByTimeAsync(2500);
    } finally {
      vi.useRealTimers();
    }
    await drain();
    expect(puts).toBe(1);
    // после конфликта холст собран заново из свежей версии: она пустая
    expect(host.querySelectorAll('.svelte-flow__node').length).toBe(0);
    expect(state.calls.filter((c) => c.startsWith('PUT'))).toHaveLength(1);
  });

  it('refreshes the run on a workflows event for the open process', async () => {
    const { api, state } = backend();
    await render(api);
    const before = state.calls.filter((c) => c.endsWith('/runs?limit=1')).length;
    window.dispatchEvent(new CustomEvent('deloist:event', { detail: { type: 'workflows', workflow_id: 'w1', run_id: 'r1' } }));
    await drain();
    expect(state.calls.filter((c) => c.endsWith('/runs?limit=1')).length).toBeGreaterThan(before);
  });
});
