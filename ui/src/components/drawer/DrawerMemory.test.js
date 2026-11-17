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
  file: '- Любит кофе\n  Пьёт без сахара\n',
  fullLimit: 2500,
  brain: 'Отвечай коротко.',
  brainLimit: 4000,
  sharedByBot: [{ id: 's1', title: 'Общий факт', body: 'для всех' }],
  journal: [{ id: 'j1', at: Date.now(), who: { kind: 'human' }, action: 'add', note: '', added: ['Любит кофе'], undoable: true }],
};

function fakeApi() {
  return vi.fn(async (path, opts = {}) => {
    if (path === '/api/bots/b1/memory') return memory;
    if (path === '/api/bots/b1/memory-file' && opts.method === 'PUT') return { file: opts.body.text, entryId: 'j2' };
    if (path === '/api/bots/b1/brain' && opts.method === 'PUT') return { brain: opts.body.text };
    if (path === '/api/memory/facts/m18' && opts.method === 'DELETE') return {};
    if (path === '/api/memory/facts/m18' && opts.method === 'PATCH') return { fact: {} };
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
  it('shows memory.md and brain.md in text areas with hints, the counter and the journal', async () => {
    const api = fakeApi();
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => expect(host.querySelectorAll('textarea').length).toBe(2));
    const [file, brain] = host.querySelectorAll('textarea');
    expect(file.value).toBe(memory.file);
    expect(brain.value).toBe('Отвечай коротко.');
    expect(host.textContent).toMatch(/Пока память меньше 2\s500 знаков/);
    expect(host.textContent).toContain('120');
    expect(host.querySelector('.dm-jr-title').textContent).toBe('Дописал в память');
    expect(host.textContent).toContain('Общий факт');
  });

  it('saves memory.md through the memory-file route and reloads', async () => {
    const api = fakeApi();
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => expect(host.querySelectorAll('textarea').length).toBe(2));
    const file = host.querySelector('textarea');
    expect(buttonByText(host, 'Сохранить').disabled).toBe(true);
    file.value = '- Новая запись\n';
    file.dispatchEvent(new Event('input'));
    flushSync();
    buttonByText(host, 'Сохранить').click();
    await waitFor(() => expect(api).toHaveBeenCalledWith('/api/bots/b1/memory-file', { method: 'PUT', body: { text: '- Новая запись\n' } }));
  });

  it('saves brain.md through the brain route', async () => {
    const api = fakeApi();
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => expect(host.querySelectorAll('textarea').length).toBe(2));
    const brain = host.querySelectorAll('textarea')[1];
    brain.value = 'Всегда на «вы».';
    brain.dispatchEvent(new Event('input'));
    flushSync();
    const buttons = [...host.querySelectorAll('button')].filter((b) => b.textContent.trim() === 'Сохранить');
    buttons[1].click();
    await waitFor(() => expect(api).toHaveBeenCalledWith('/api/bots/b1/brain', { method: 'PUT', body: { text: 'Всегда на «вы».' } }));
  });

  it('shows the server message when the file is refused', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (opts.method === 'PUT') throw new Error('В памяти секрет');
      return memory;
    });
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => expect(host.querySelectorAll('textarea').length).toBe(2));
    const file = host.querySelector('textarea');
    file.value = '- ключ';
    file.dispatchEvent(new Event('input'));
    flushSync();
    buttonByText(host, 'Сохранить').click();
    await waitFor(() => expect(document.body.textContent).toContain('В памяти секрет'));
  });

  it('does not overwrite typed text when a reload arrives', async () => {
    const api = fakeApi();
    let reload;
    mount({ bot: { id: 'b1' }, api, registerReload: (fn) => { if (fn) reload = fn; } });
    await waitFor(() => expect(host.querySelectorAll('textarea').length).toBe(2));
    const file = host.querySelector('textarea');
    file.value = '- набираю';
    file.dispatchEvent(new Event('input'));
    flushSync();
    await reload();
    flushSync();
    expect(host.querySelector('textarea').value).toBe('- набираю');
  });

  it('reverts the typed text with «Отменить правку»', async () => {
    const api = fakeApi();
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => expect(host.querySelectorAll('textarea').length).toBe(2));
    const file = host.querySelector('textarea');
    file.value = '- лишнее';
    file.dispatchEvent(new Event('input'));
    flushSync();
    buttonByText(host, 'Отменить правку').click();
    flushSync();
    expect(host.querySelector('textarea').value).toBe(memory.file);
  });

  it('undoes a journal entry', async () => {
    const api = fakeApi();
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => expect(host.querySelector('.dm-jr')).not.toBeNull());
    buttonByText(host.querySelector('.dm-jr'), 'Отменить').click();
    await waitFor(() => expect(api).toHaveBeenCalledWith('/api/memory/undo/j1', { method: 'POST' }));
  });

  it('shows a section per team and deletes a team fact', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (opts.method === 'DELETE') return {};
      return {
        ...memory,
        teams: [{ id: 'r1', name: 'Картинки', used: 40, limit: 8000, facts: [{ id: 'm18', title: 'Сюжет выбирает человек', body: 'и Художник', kind: 'project' }] }],
      };
    });
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => {
      const titles = [...host.querySelectorAll('.k-section__title')].map((h) => h.textContent);
      expect(titles).toContain('Память команды «Картинки» · 1');
    });
    expect(host.textContent).toContain('Сюжет выбирает человек');
    buttonByText(host, 'Удалить').click();
    await waitFor(() => expect(api).toHaveBeenCalledWith('/api/memory/facts/m18', { method: 'DELETE' }));
  });

  it('pins a team fact with the "always in context" switch', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (opts.method === 'PATCH') return { fact: {} };
      return { ...memory, teams: [{ id: 'r1', name: 'Картинки', facts: [{ id: 'm18', title: 'Сюжет', body: '', kind: 'project', pinned: false }] }] };
    });
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => expect(host.querySelector('[role="switch"]')).not.toBeNull());
    const toggle = host.querySelector('[role="switch"]');
    expect(toggle.getAttribute('aria-checked')).toBe('false');
    toggle.click();
    await waitFor(() => expect(api).toHaveBeenCalledWith('/api/memory/facts/m18', { method: 'PATCH', body: { pinned: true } }));
  });
});
