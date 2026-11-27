import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import TasksAndProcesses from './TasksAndProcesses.svelte';
import { starterGraph } from '../../lib/workflows/graph.js';

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
afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
});

const flush = async () => {
  for (let i = 0; i < 20; i += 1) {
    flushSync();
    await new Promise((r) => setTimeout(r, 0));
  }
};

const item = { id: 'w1', title: 'Отчёт', enabled: true, updated_at: 1, card: { trigger: 'schedule', schedule: { kind: 'daily', time: '09:00' }, bot_id: 'a', task: 'Дело', quiet: true } };
const api = vi.fn(async (path) => {
  if (path === '/api/workflows') return { workflows: [item] };
  if (path === '/api/workflows/w1') return { workflow: { id: 'w1', title: 'Отчёт', revision: 1, graph: starterGraph() }, triggers: [] };
  if (path.endsWith('/runs?limit=1')) return { runs: [] };
  return {};
});

async function render(props = {}) {
  host = document.createElement('div');
  host.style.cssText = 'width:1000px;height:700px';
  document.body.append(host);
  instance = mount(TasksAndProcesses, { target: host, props: { api, bots: [{ id: 'a', name: 'Аналитик' }], ...props } });
  await flush();
}

describe('TasksAndProcesses', () => {
  it('has the four tabs and starts on the processes list', async () => {
    await render();
    expect([...host.querySelectorAll('[role="tab"]')].map((t) => t.getAttribute('data-id'))).toEqual(['todos', 'reminders', 'habits', 'processes']);
    expect(host.querySelector('[role="tab"][aria-selected="true"]').getAttribute('data-id')).toBe('processes');
    expect(host.querySelectorAll('.pc').length).toBe(1);
  });

  it('opens the canvas from an expanded card and comes back with the list button', async () => {
    await render();
    host.querySelector('.pc-chain-btn').click();
    await flush();
    host.querySelector('.pc-canvas').click();
    await flush();
    expect(host.querySelectorAll('.svelte-flow__node').length).toBe(3);
    expect(host.querySelector('.pc')).toBeNull();
    host.querySelector('.wf-ib--list').click();
    await flush();
    expect(host.querySelectorAll('.pc').length).toBe(1);
  });

  it('switches to the reminders tab', async () => {
    await render({ reminders: [] });
    host.querySelector('[role="tab"][data-id="reminders"]').click();
    await flush();
    expect(host.querySelector('[role="tab"][aria-selected="true"]').getAttribute('data-id')).toBe('reminders');
    expect(host.querySelector('.pc')).toBeNull();
  });
});
