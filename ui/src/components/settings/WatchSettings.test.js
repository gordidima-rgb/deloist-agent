import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import WatchSettings from './WatchSettings.svelte';

const routines = [
  { id: 'r1', botId: 'b1', title: 'Комментировать X', hook: false },
  { id: 'r2', botId: 'b1', title: 'Вебхук', hook: true },
];

let instance;
let host;

function render(api) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(WatchSettings, { target: host, props: { api, routines } });
  flushSync();
}

async function drain() {
  for (let i = 0; i < 12; i += 1) {
    flushSync();
    await Promise.resolve();
  }
  flushSync();
}

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

describe('WatchSettings', () => {
  it('lists active page triggers of known routines and removes one', async () => {
    const trigger = { routineId: 'r1', urls: ['https://x.com/a'], selector: 'article', contains: '', lastCheck: 0, lastError: 'страница просит вход' };
    const api = vi.fn(async (path, opts) => (opts?.method === 'DELETE' ? { ok: true } : { triggers: [trigger] }));
    render(api);
    await drain();
    const card = host.querySelector('[data-watch="r1"]');
    expect(card.textContent).toContain('https://x.com/a');
    expect(card.textContent).toContain('страница просит вход');
    [...card.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Убрать').click();
    await drain();
    expect(api).toHaveBeenCalledWith('/api/watch/triggers/r1', { method: 'DELETE' });
  });

  it('saves the addresses one per line, deduplicated, with the default selector', async () => {
    const api = vi.fn(async () => ({ triggers: [] }));
    render(api);
    await drain();
    const area = host.querySelector('textarea');
    area.value = 'https://x.com/a\nhttps://x.com/b\nhttps://x.com/a';
    area.dispatchEvent(new Event('input', { bubbles: true }));
    await drain();
    [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Сохранить сторожа').click();
    await drain();
    const put = api.mock.calls.find(([, opts]) => opts?.method === 'PUT');
    expect(put[0]).toBe('/api/watch/triggers/r1');
    expect(put[1].body).toEqual({ urls: ['https://x.com/a', 'https://x.com/b'], selector: 'article', contains: '' });
  });
});
