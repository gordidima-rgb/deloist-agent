import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import { mountComponent } from '../../lib/mount.svelte.js';
import DrawerGoals from './DrawerGoals.svelte';

const goals = [
  { id: 'g1', botId: 'b1', title: 'Отвечать в чате', status: 'active', recurring: true, progress: [] },
  { id: 'g2', botId: 'b1', title: 'Собрать отчёт', status: 'paused', recurring: false, progress: [{ at: 1, text: 'Черновик готов' }] },
];

function fakeApi() {
  return vi.fn(async (path, opts = {}) => {
    if (path === '/api/bots/b1/checkin' && opts.method === 'PUT') return {};
    if (path === '/api/goals/g1' && opts.method === 'PATCH') return {};
    if (path === '/api/goals/g1' && opts.method === 'DELETE') return {};
    throw new Error(`unexpected request: ${path} ${opts.method || 'GET'}`);
  });
}

let handle;
let host;
let originalConfirm;

function mount(props) {
  host = document.createElement('div');
  document.body.append(host);
  handle = mountComponent(DrawerGoals, host, props);
}

async function waitFor(check) {
  await vi.waitFor(() => {
    flushSync();
    check();
  });
}

const buttonByText = (root, text) => [...root.querySelectorAll('button')].find((b) => b.textContent.trim() === text);

beforeEach(() => {
  originalConfirm = window.confirm;
});

afterEach(() => {
  window.confirm = originalConfirm;
  handle?.destroy();
  host?.remove();
  handle = null;
  host = null;
});

describe('DrawerGoals', () => {
  it('turns self-check on and sends the interval with it', async () => {
    const api = fakeApi();
    mount({ bot: { id: 'b1' }, goals, checkin: { botId: 'b1', enabled: false, everyMinutes: 180 }, api });
    flushSync();
    host.querySelector('.k-switch-button').click();
    await waitFor(() => expect(api).toHaveBeenCalledWith('/api/bots/b1/checkin', {
      method: 'PUT',
      body: { enabled: true, everyMinutes: 180 },
    }));
  });

  it('pauses an active goal with PATCH', async () => {
    const api = fakeApi();
    mount({ bot: { id: 'b1' }, goals, checkin: null, api });
    flushSync();
    const row = [...host.querySelectorAll('.k-list-item')].find((el) => el.textContent.includes('Отвечать в чате'));
    buttonByText(row, 'Пауза').click();
    await waitFor(() => expect(api).toHaveBeenCalledWith('/api/goals/g1', { method: 'PATCH', body: { status: 'paused' } }));
  });

  it('asks before deleting a goal and does nothing when the answer is no', async () => {
    const api = fakeApi();
    window.confirm = vi.fn(() => false);
    mount({ bot: { id: 'b1' }, goals, checkin: null, api });
    flushSync();
    const row = [...host.querySelectorAll('.k-list-item')].find((el) => el.textContent.includes('Отвечать в чате'));
    buttonByText(row, 'Удалить').click();
    flushSync();
    expect(window.confirm).toHaveBeenCalledWith('Удалить цель «Отвечать в чате»?');
    expect(api).not.toHaveBeenCalledWith('/api/goals/g1', { method: 'DELETE' });
  });

  it('deletes a goal after confirmation', async () => {
    const api = fakeApi();
    window.confirm = vi.fn(() => true);
    mount({ bot: { id: 'b1' }, goals, checkin: null, api });
    flushSync();
    const row = [...host.querySelectorAll('.k-list-item')].find((el) => el.textContent.includes('Отвечать в чате'));
    buttonByText(row, 'Удалить').click();
    await waitFor(() => expect(api).toHaveBeenCalledWith('/api/goals/g1', { method: 'DELETE' }));
  });

  it('shows the empty text when the bot has no goals', () => {
    mount({ bot: { id: 'b1' }, goals: [], checkin: null, api: fakeApi() });
    flushSync();
    expect(host.querySelector('.k-list')).toBeNull();
    expect(host.textContent).toContain('Пока нет целей');
  });
});
