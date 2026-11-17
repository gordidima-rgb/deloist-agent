import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import { mountComponent } from '../../lib/mount.svelte.js';
import DrawerMemory from './DrawerMemory.svelte';

const memory = {
  facts: [
    { id: 'f1', title: 'Любит кофе', body: 'Пьёт без сахара', kind: 'user', entities: ['кофе'], links: [] },
    { id: 'f2', title: 'Не звонить после 22', body: '', kind: 'feedback', entities: [], links: [] },
  ],
  used: 120,
  limit: 1000,
  journal: [{ id: 'j1', at: Date.now(), who: { kind: 'human' }, action: 'add', note: '', added: ['Любит кофе'], undoable: true }],
};

function fakeApi() {
  return vi.fn(async (path, opts = {}) => {
    if (path === '/api/bots/b1/memory') return memory;
    if (path === '/api/memory/facts' && opts.method === 'POST') return { id: 'f3' };
    if (path === '/api/memory/facts/f2' && opts.method === 'DELETE') return {};
    if (path === '/api/memory/undo/j1' && opts.method === 'POST') return {};
    throw new Error(`unexpected request: ${path} ${opts.method || 'GET'}`);
  });
}

let handle;
let host;

function mount(props) {
  host = document.createElement('div');
  document.body.append(host);
  handle = mountComponent(DrawerMemory, host, props);
}

async function waitFor(check) {
  await vi.waitFor(() => {
    flushSync();
    check();
  });
}

const buttonByText = (root, text) => [...root.querySelectorAll('button')].find((b) => b.textContent.trim() === text);

afterEach(() => {
  handle?.destroy();
  host?.remove();
  handle = null;
  host = null;
});

describe('DrawerMemory', () => {
  it('renders one group per kind with its count, the counter and the journal', async () => {
    const api = fakeApi();
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => {
      const titles = [...host.querySelectorAll('.k-section__title')].map((h) => h.textContent);
      expect(titles).toContain('О человеке · 1');
      expect(titles).toContain('Поправка · 1');
    });
    expect(host.textContent).toContain('120');
    expect(host.querySelector('.dm-jr-title').textContent).toBe('Дописал в память');
    expect(api).toHaveBeenCalledWith('/api/bots/b1/memory');
  });

  it('adds a fact with scope set to the bot id', async () => {
    const api = fakeApi();
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => expect(host.querySelectorAll('.dm-fact').length).toBe(2));
    buttonByText(host, 'Добавить факт').click();
    flushSync();
    const form = host.querySelector('form.dm-form');
    const [title, body] = [form.querySelector('input'), form.querySelector('textarea')];
    title.value = 'Новый';
    title.dispatchEvent(new Event('input'));
    body.value = 'Текст';
    body.dispatchEvent(new Event('input'));
    form.dispatchEvent(new Event('submit', { cancelable: true }));
    await waitFor(() => expect(api).toHaveBeenCalledWith('/api/memory/facts', {
      method: 'POST',
      body: { title: 'Новый', body: 'Текст', kind: 'user', entities: [], scope: 'b1' },
    }));
  });

  it('deletes a fact and shows the toast about the journal', async () => {
    const api = fakeApi();
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => expect(host.querySelectorAll('.dm-fact').length).toBe(2));
    const row = [...host.querySelectorAll('.dm-fact')].find((el) => el.textContent.includes('Не звонить'));
    buttonByText(row, 'Удалить').click();
    await waitFor(() => {
      expect(api).toHaveBeenCalledWith('/api/memory/facts/f2', { method: 'DELETE' });
      expect(document.body.textContent).toContain('Удалено. Отменить можно в журнале');
    });
  });

  it('undoes a journal entry', async () => {
    const api = fakeApi();
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => expect(host.querySelector('.dm-jr')).not.toBeNull());
    buttonByText(host.querySelector('.dm-jr'), 'Отменить').click();
    await waitFor(() => expect(api).toHaveBeenCalledWith('/api/memory/undo/j1', { method: 'POST' }));
  });

  it('shows a section per team with its facts', async () => {
    const api = vi.fn(async () => ({
      ...memory,
      teams: [{ id: 'r1', name: 'Картинки', used: 40, limit: 8000, facts: [{ id: 'm18', title: 'Сюжет выбирает человек', body: 'и Художник', kind: 'project' }] }],
    }));
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => {
      const titles = [...host.querySelectorAll('.k-section__title')].map((h) => h.textContent);
      expect(titles).toContain('Память команды «Картинки» · 1');
    });
    expect(host.textContent).toContain('Сюжет выбирает человек');
  });
});
