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
  const state = { wf, calls: [] };
  const api = vi.fn(async (path, opts = {}) => {
    const method = opts.method || 'GET';
    state.calls.push(`${method} ${path}`);
    if (over[`${method} ${path}`]) return over[`${method} ${path}`](opts);
    if (path === '/api/workflows/w1' && method === 'GET') return { workflow: state.wf, triggers: [] };
    if (path.endsWith('/runs?limit=1')) return { runs: [] };
    throw Object.assign(new Error('unexpected ' + method + ' ' + path), { status: 404 });
  });
  return { api, state };
}

async function render(api) {
  host = document.createElement('div');
  host.style.cssText = 'width:1200px;height:800px';
  document.body.append(host);
  instance = mount(WorkflowsScreen, { target: host, props: { id: 'w1', api, bots: [{ id: 'a', name: 'Аналитик', color: '#4fb4ff' }] } });
  await drain();
}

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
});

describe('WorkflowsScreen', () => {
  it('opens the given workflow on the canvas', async () => {
    const { api, state } = backend();
    await render(api);
    expect(state.calls).toContain('GET /api/workflows/w1');
    expect(host.querySelectorAll('.svelte-flow__node').length).toBe(3);
  });

  it('shows a retry when the workflow cannot be loaded', async () => {
    const { api } = backend({ 'GET /api/workflows/w1': async () => Promise.reject(Object.assign(new Error('нет такого'), { status: 404 })) });
    await render(api);
    expect(host.textContent).toContain('Не удалось загрузить процессы');
    expect([...host.querySelectorAll('button')].some((b) => b.textContent.includes('Повторить'))).toBe(true);
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
    // правка не потеряна: холст прежний, показан выбор, автосохранение стоит
    expect(host.querySelector('.wfs-conflict')).not.toBeNull();
    expect(host.querySelector('.wfn-area').value).toBe('новое поручение');
    expect(state.calls.filter((c) => c.startsWith('PUT'))).toHaveLength(1);
    // «Загрузить свежую»: холст собирается заново из свежей версии (она пустая)
    [...host.querySelectorAll('.wfs-cf-btn')].find((b) => b.textContent.includes('свежую')).click();
    await drain();
    expect(host.querySelector('.wfs-conflict')).toBeNull();
    expect(host.querySelectorAll('.svelte-flow__node').length).toBe(0);
    expect(state.calls.filter((c) => c.startsWith('PUT'))).toHaveLength(1);
  });

  it('keeps my edits after a 409 and re-saves with the fresh revision', async () => {
    const conflict = { id: 'w1', title: 'Первый', revision: 7, graph: { nodes: [], edges: [], strokes: [] } };
    const bases = [];
    const { api, state } = backend({
      'PUT /api/workflows/w1/graph': async (opts) => {
        bases.push(opts.body.base_revision);
        if (bases.length === 1) throw Object.assign(new Error('conflict'), { status: 409, data: { conflict: true, workflow: conflict } });
        return { revision: 8 };
      },
    });
    await render(api);
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    try {
      const area = host.querySelector('.wfn-area');
      area.value = 'мои правки';
      area.dispatchEvent(new Event('input', { bubbles: true }));
      await vi.advanceTimersByTimeAsync(2500);
      // ещё правка, пока выбор не сделан: автосохранение стоит
      area.value = 'мои правки ещё';
      area.dispatchEvent(new Event('input', { bubbles: true }));
      await vi.advanceTimersByTimeAsync(3000);
    } finally {
      vi.useRealTimers();
    }
    await drain();
    expect(bases).toEqual([2]);
    [...host.querySelectorAll('.wfs-cf-btn')].find((b) => b.textContent.includes('Оставить')).click();
    await drain();
    expect(bases).toEqual([2, 7]);
    expect(host.querySelector('.wfs-conflict')).toBeNull();
    expect(host.querySelector('.wfn-area').value).toBe('мои правки ещё');
    expect(state.calls.filter((c) => c.startsWith('PUT'))).toHaveLength(2);
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
    host.querySelector('.wf-hp__row .k-lrow__t').click();
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
    host.querySelector('.wf-hp__row .k-lrow__t').click();
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
    host.querySelector('.wf-hp__row .k-lrow__t').click();
    await drain();
    expect(host.textContent).toContain('Показан прошлый запуск');
  });

  it('toggles enabled from the top bar through PATCH', async () => {
    const patches = [];
    const { api } = backend({
      'PATCH /api/workflows/w1': async (opts) => {
        patches.push(opts.body);
        return { workflow: { id: 'w1', title: 'Первый', enabled: opts.body.enabled, revision: 2 } };
      },
    });
    await render(api);
    host.querySelector('.wf-en [role="switch"]').click();
    await drain();
    expect(patches).toEqual([{ enabled: false }]);
    expect(host.querySelector('.wf-en.off')).not.toBeNull();
    host.querySelector('.wf-en [role="switch"]').click();
    await drain();
    expect(patches[1]).toEqual({ enabled: true });
    expect(host.querySelector('.wf-en.off')).toBeNull();
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
    const btns = [...host.querySelectorAll('.wfs-cf-btn')].map((b) => b.textContent.trim());
    expect(btns).toEqual(['Оставить мои правки', 'Загрузить свежую']);
    delete window.DeloistUI;
  });
});
