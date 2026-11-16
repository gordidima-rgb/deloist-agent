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
let firstNode = '';
const starterNodeId = () => firstNode;
async function drain(n = 25) {
  for (let i = 0; i < n; i += 1) {
    flushSync();
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

function backend(over = {}) {
  const wf = { id: 'w1', title: 'Первый', revision: 2, graph: starterGraph(), last_run: { status: 'done', at: 1700000000 } };
  firstNode = wf.graph.nodes[0].id;
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

  it('previews a version read-only and restores it', async () => {
    const restored = { id: 'w1', title: 'Первый', revision: 9, graph: starterGraph() };
    const { api, state } = backend({
      'GET /api/workflows/w1/versions': async () => [{ v: 3, kind: 'auto', at: 1700000000000, revision: 1, steps: 3 }],
      'GET /api/workflows/w1/versions/3': async () => ({ graph: { nodes: [], edges: [], strokes: [] } }),
      'POST /api/workflows/w1/versions/3/restore': async () => ({ workflow: restored }),
    });
    await render(api);
    host.querySelector('[aria-label="Версии"]').click();
    await drain();
    host.querySelector('.wf-hp__row').click();
    await drain();
    expect(state.calls).toContain('GET /api/workflows/w1/versions/3');
    expect(host.querySelector('.wfs-preview')).not.toBeNull();
    const restore = [...host.querySelectorAll('.wfs-preview button')].find((b) => b.textContent.includes('Сделать текущей'));
    restore.click();
    await drain();
    expect(state.calls).toContain('POST /api/workflows/w1/versions/3/restore');
    expect(host.querySelector('.wfs-preview')).toBeNull();
    expect(host.querySelectorAll('.svelte-flow__node').length).toBe(3);
  });

  it('closing the preview changes nothing on the server', async () => {
    const { api, state } = backend({
      'GET /api/workflows/w1/versions': async () => [{ v: 1, kind: 'auto', at: 1700000000000, steps: 3 }],
      'GET /api/workflows/w1/versions/1': async () => ({ graph: { nodes: [], edges: [], strokes: [] } }),
    });
    await render(api);
    host.querySelector('[aria-label="Версии"]').click();
    await drain();
    host.querySelector('.wf-hp__row').click();
    await drain();
    host.querySelector('.wfs-preview [aria-label="Закрыть просмотр"]').click();
    await drain();
    expect(host.querySelector('.wfs-preview')).toBeNull();
    expect(state.calls.some((c) => c.startsWith('POST') || c.startsWith('PUT'))).toBe(false);
  });

  it('shows an old run from history without touching the latest one', async () => {
    const old = { id: 'old', workflow_id: 'w1', status: 'done', source: 'manual', started_at: 1700000000000, ended_at: 1700000005000, steps: [{ id: 's', run_id: 'old', node_id: starterNodeId(), attempt: 1, status: 'done', output: 'старый результат', started_at: 1700000000000, ended_at: 1700000001000 }] };
    const { api } = backend({
      'GET /api/workflows/w1/runs?limit=20': async () => ({ runs: [old], next_before: null }),
    });
    await render(api);
    host.querySelector('[aria-label="История"]').click();
    await drain();
    host.querySelector('.wf-hp__row').click();
    await drain();
    expect(host.textContent).toContain('Показан прошлый запуск');
  });

  it('toggles enabled from the list and the top bar through PATCH', async () => {
    const patches = [];
    const { api } = backend({
      'PATCH /api/workflows/w1': async (opts) => {
        patches.push(opts.body);
        return { workflow: { id: 'w1', title: 'Первый', enabled: opts.body.enabled, revision: 2 } };
      },
    });
    await render(api);
    const rowSwitch = host.querySelector('.wfl-sw [role="switch"]');
    expect(rowSwitch.getAttribute('aria-checked')).toBe('true');
    rowSwitch.click();
    await drain();
    expect(patches).toEqual([{ enabled: false }]);
    expect(host.querySelector('.wfl-item.off')).not.toBeNull();
    expect(host.querySelector('.wf-en.off')).not.toBeNull();
    host.querySelector('.wf-en [role="switch"]').click();
    await drain();
    expect(patches[1]).toEqual({ enabled: true });
    expect(host.querySelector('.wfl-item.off')).toBeNull();
  });

  it('applies a bot edit in place and keeps the canvas mounted', async () => {
    const { api, state } = backend();
    await render(api);
    const nodeEl = host.querySelector('.svelte-flow__node');
    const g = starterGraph();
    g.nodes = g.nodes.slice(0, 2);
    g.edges = [];
    state.wf = { ...state.wf, revision: 5, graph: { ...g, nodes: g.nodes.map((n, i) => ({ ...n, id: state.wf.graph.nodes[i].id })) } };
    window.dispatchEvent(new CustomEvent('deloist:event', { detail: { type: 'workflows', workflow_id: 'w1' } }));
    await drain();
    // тот же узел остался в DOM (холст не пересобирали), а удалённый затухает и исчезает чуть позже
    expect(host.contains(nodeEl)).toBe(true);
    await new Promise((r) => setTimeout(r, 700));
    await drain(5);
    expect(host.querySelectorAll('.svelte-flow__node').length).toBe(2);
    expect(host.contains(nodeEl)).toBe(true);
  });

  it('does not overwrite unsaved local edits when someone else changes the workflow', async () => {
    const toast = vi.fn();
    window.DeloistUI = { toast };
    const { api, state } = backend();
    await render(api);
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    try {
      const area = host.querySelector('.wfn-area');
      area.value = 'моя правка';
      area.dispatchEvent(new Event('input', { bubbles: true }));
      state.wf = { ...state.wf, revision: 9, graph: { nodes: [], edges: [], strokes: [] } };
      window.dispatchEvent(new CustomEvent('deloist:event', { detail: { type: 'workflows', workflow_id: 'w1' } }));
      await vi.advanceTimersByTimeAsync(100);
    } finally {
      vi.useRealTimers();
    }
    await drain(5);
    expect(host.querySelectorAll('.svelte-flow__node').length).toBe(3);
    expect(host.querySelector('.wfn-area').value).toBe('моя правка');
    expect(toast).toHaveBeenCalledTimes(1);
    expect(toast.mock.calls[0][1].action.label).toBe('Загрузить свежую');
    delete window.DeloistUI;
  });
});
