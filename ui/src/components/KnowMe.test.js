import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import { mountComponent } from '../lib/mount.svelte.js';
import KnowMe from './KnowMe.svelte';

const found = {
  found: true,
  root: 'C:\\hermes',
  profiles: [
    { name: 'default', personaChars: 120, memoryEntries: 3 },
    { name: 'work', personaChars: 80, memoryEntries: 1 },
  ],
};

function deferred() {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
}

function fakeApi({ hermes = found, botIds = ['b1', 'b2'], importError = '', gate = null } = {}) {
  return vi.fn(async (path, opts = {}) => {
    if (path === '/api/know-me/sources') return [{ id: 'claude', name: 'Claude', found: true, count: 3 }];
    if (path === '/api/import/hermes' && !opts.method) return hermes;
    if (path === '/api/import/hermes' && opts.method === 'POST') {
      if (gate) await gate.promise;
      if (importError) throw new Error(importError);
      return { botIds };
    }
    throw new Error(`unexpected request: ${path}`);
  });
}

let handle;
let host;

function mount(props) {
  host = document.createElement('div');
  document.body.append(host);
  handle = mountComponent(KnowMe, host, props);
}

async function waitFor(check) {
  await vi.waitFor(() => {
    flushSync();
    check();
  });
}

const importButton = () => host.querySelector('.km-hermes-actions .km-btn');

afterEach(() => {
  handle?.destroy();
  host?.remove();
  handle = null;
  host = null;
});

describe('KnowMe · Hermes', () => {
  it('shows the Hermes block with one checked line per profile when found', async () => {
    const api = fakeApi();
    mount({ api });
    await waitFor(() => expect(host.querySelector('.km-hermes')).not.toBeNull());

    expect(host.querySelector('.km-hermes-title').textContent).toBe('Hermes Agent');
    const labels = [...host.querySelectorAll('.km-hermes .k-check__label')].map((el) => el.textContent);
    expect(labels).toEqual([
      'default — характер 120 знаков, 3 записей памяти',
      'work — характер 80 знаков, 1 записей памяти',
    ]);
    const boxes = [...host.querySelectorAll('.km-hermes input[type="checkbox"]')];
    expect(boxes).toHaveLength(2);
    expect(boxes.every((box) => box.checked)).toBe(true);
  });

  it('posts every checked profile, disables the button while busy, reports the ids', async () => {
    const gate = deferred();
    const onimported = vi.fn();
    const api = fakeApi({ gate });
    mount({ api, onimported });
    await waitFor(() => expect(host.querySelector('.km-hermes')).not.toBeNull());

    importButton().click();
    flushSync();
    expect(api).toHaveBeenCalledWith('/api/import/hermes', {
      method: 'POST',
      body: { profiles: ['default', 'work'] },
    });
    expect(importButton().disabled).toBe(true);

    gate.resolve();
    await waitFor(() => expect(onimported).toHaveBeenCalledWith(['b1', 'b2']));
    expect(host.querySelector('.km-hermes .km-note').textContent).toBe('Перенесено ботов: 2');
    expect(importButton().disabled).toBe(false);
  });

  it('shows the server error text and does not call onimported', async () => {
    const onimported = vi.fn();
    const api = fakeApi({ importError: 'Не нашёл папку Hermes на этом компьютере' });
    mount({ api, onimported });
    await waitFor(() => expect(host.querySelector('.km-hermes')).not.toBeNull());

    importButton().click();
    await waitFor(() => expect(host.querySelector('.km-hermes .km-error')?.textContent).toBe('Не нашёл папку Hermes на этом компьютере'));
    expect(onimported).not.toHaveBeenCalled();
  });

  it('renders no Hermes block when the profiles were not found', async () => {
    const api = fakeApi({ hermes: { found: false, root: '', profiles: [] } });
    mount({ api });
    await waitFor(() => expect(host.querySelector('.km-sources li')).not.toBeNull());

    expect(api).toHaveBeenCalledWith('/api/import/hermes');
    expect(host.querySelector('.km-hermes')).toBeNull();
    expect(host.textContent).not.toContain('Hermes Agent');
  });
});
