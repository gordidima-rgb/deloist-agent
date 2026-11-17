import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import RemotePcs from './RemotePcs.svelte';
import RemoteChat from './RemoteChat.svelte';

let instances = [];
let hosts = [];

async function drain() {
  for (let i = 0; i < 16; i += 1) {
    flushSync();
    await Promise.resolve();
  }
  flushSync();
}

function render(Component, props) {
  const host = document.createElement('div');
  document.body.append(host);
  hosts.push(host);
  instances.push(mount(Component, { target: host, props }));
  return host;
}

afterEach(() => {
  instances.forEach((i) => unmount(i));
  hosts.forEach((h) => h.remove());
  instances = [];
  hosts = [];
  localStorage.clear();
});

const state = { pc: 'Ноутбук', bots: [{ id: 'b1', name: '<i>Бот</i>', emoji: '🙂', color: '#7c5cff', skin: 3, preview: { text: 'привет', role: 'assistant' }, at: Date.now(), busy: false }] };

function pcsApi(pcs) {
  return vi.fn(async (path) => {
    if (path === '/api/remote/pcs') return { connected: true, pcs, candidates: [], pairing: null };
    if (path.endsWith('/state')) return state;
    if (path.endsWith('/needs')) return { items: [{ id: 'c1', botId: 'b1', title: 'x' }] };
    return {};
  });
}

describe('RemotePcs', () => {
  it('renders nothing without remote PCs', async () => {
    const host = render(RemotePcs, { api: pcsApi([]) });
    await drain();
    expect(host.querySelector('[data-remote-pcs]')).toBeNull();
  });

  it('shows an online section with bots, text only, and opens the chat on click', async () => {
    const onopen = vi.fn();
    const host = render(RemotePcs, { api: pcsApi([{ id: 'pc/1', name: 'Ноутбук', online: true }]), onopen });
    await drain();
    expect(host.querySelector('.rp-name').textContent).toBe('Ноутбук');
    expect(host.querySelector('.rp-state').textContent).toContain('в сети');
    expect(host.querySelector('.k-list-item__title').textContent).toBe('<i>Бот</i>');
    expect(host.querySelector('.k-list-item i')).toBeNull();
    expect(host.querySelector('.orb').getAttribute('style') || '').not.toContain('url');
    expect(host.querySelector('.k-badge, .k-list-item__badge')).not.toBeNull();
    host.querySelector('.k-list-item__main').click();
    expect(onopen).toHaveBeenCalledTimes(1);
    expect(onopen.mock.calls[0][0].id).toBe('pc/1');
  });

  it('an offline PC is grey, says "не в сети" and does not open', async () => {
    const onopen = vi.fn();
    const host = render(RemotePcs, { api: pcsApi([{ id: 'pc2', name: 'Офис', online: false }]), onopen });
    await drain();
    expect(host.querySelector('.rp-pc').classList.contains('is-off')).toBe(true);
    expect(host.querySelector('.rp-state').textContent).toContain('не в сети');
    expect(host.querySelector('button.k-list-item__main')).toBeNull();
  });

  it('collapses and remembers it', async () => {
    const host = render(RemotePcs, { api: pcsApi([{ id: 'pc1', name: 'Н', online: true }]) });
    await drain();
    host.querySelector('.rp-head').click();
    await drain();
    expect(host.querySelector('.rp-list')).toBeNull();
    expect(localStorage.getItem('deloist:sidebar-remote-closed')).toContain('pc1');
  });
});

function chatApi(overrides = {}) {
  return vi.fn(async (path, opts = {}) => {
    const key = `${opts.method || 'GET'} ${path.split('?')[0]}`;
    if (overrides[key]) return overrides[key](opts);
    if (path.includes('/thread')) {
      return { messages: [
        { id: 'm1', role: 'user', text: 'привет <b>x</b>', at: 1 },
        { id: 'm2', role: 'assistant', text: '<img src=x onerror=alert(1)> [файл](C:/Users/me/workspaces/bot/a.md) [сайт](https://evil.example/p)', at: 2 },
      ] };
    }
    if (path.endsWith('/state')) return { bots: [{ id: 'b1', busy: true, queued: 0 }] };
    if (path.endsWith('/needs')) {
      return { items: [
        { id: 'c1', botId: 'b 1', title: 'Запустить dir', detail: 'dir', remote: true, answers: ['once', 'deny'] },
        { id: 'c2', botId: 'b 1', title: 'Записать файл', detail: 'x', remote: false, answers: ['deny'] },
        { id: 'c3', botId: 'other', title: 'Чужой бот', remote: true, answers: ['once', 'deny'] },
      ] };
    }
    return {};
  });
}

const chatProps = (api) => ({ api, pc: { id: 'pc/1', name: 'Ноутбук', online: true }, bot: { id: 'b 1', name: 'Бот', busy: true } });

describe('RemoteChat', () => {
  it('renders remote text safely: no tags, no file anchors, real address and source mark on links', async () => {
    const host = render(RemoteChat, chatProps(chatApi()));
    await drain();
    expect(host.querySelector('.rc-msg.is-user .rc-plain').textContent).toBe('привет <b>x</b>');
    const md = host.querySelector('.rc-md');
    expect(md.querySelector('img')).toBeNull();
    expect(md.querySelector('[data-file]')).toBeNull();
    expect(md.textContent).toContain('https://evil.example/p');
    expect(md.textContent).toContain('с другого ПК');
  });

  it('puts remote ids through encodeURIComponent in addresses', async () => {
    const api = chatApi();
    render(RemoteChat, chatProps(api));
    await drain();
    expect(api.mock.calls.some(([p]) => p.startsWith('/api/remote/pc%2F1/bots/b%201/thread'))).toBe(true);
  });

  it('shows only this bot cards; allow once and deny; closed cards say "approve on that PC"', async () => {
    const api = chatApi({ 'POST /api/remote/pc%2F1/approvals/c1': async () => ({}) });
    const host = render(RemoteChat, chatProps(api));
    await drain();
    expect(host.querySelectorAll('[data-card]').length).toBe(2);
    const closed = host.querySelector('[data-card="c2"]');
    expect(closed.textContent).toContain('Одобрите на том ПК');
    expect(closed.querySelector('[data-answer="once"]')).toBeNull();
    expect(closed.querySelector('[data-answer="deny"]')).not.toBeNull();
    host.querySelector('[data-card="c1"] [data-answer="once"]').click();
    await drain();
    const call = api.mock.calls.find(([p]) => p === '/api/remote/pc%2F1/approvals/c1');
    expect(call[1].body).toEqual({ answer: 'once' });
  });

  it('on_pc from the server turns into a note and keeps the card', async () => {
    const err = Object.assign(new Error('x'), { status: 409, data: { code: 'on_pc' } });
    const api = chatApi({ 'POST /api/remote/pc%2F1/approvals/c1': async () => { throw err; } });
    const host = render(RemoteChat, chatProps(api));
    await drain();
    host.querySelector('[data-card="c1"] [data-answer="once"]').click();
    await drain();
    const card = host.querySelector('[data-card="c1"]');
    // Одно из двух: кнопки разрешения или строка «Одобрите на том ПК» с тихим «Запретить».
    expect(card.textContent).toContain('Одобрите на том ПК');
    expect(card.textContent).not.toContain('Одобрите на ПК');
    expect(card.querySelector('[data-answer="once"]')).toBeNull();
    expect(card.querySelector('[data-answer="deny"]').className).toContain('k-btn--ghost');
    expect(card.querySelector('.k-btn--primary')).toBeNull();
  });

  it('an approvable card has action buttons and no "approve on that PC" line', async () => {
    const host = render(RemoteChat, chatProps(chatApi()));
    await drain();
    const card = host.querySelector('[data-card="c1"]');
    expect(card.querySelector('[data-answer="once"]')).not.toBeNull();
    expect(card.textContent).not.toContain('Одобрите на');
    expect(card.querySelectorAll('.k-btn--primary').length).toBe(1);
    expect(card.querySelector('[data-answer="deny"]').className).toContain('k-btn--ghost');
    const closed = host.querySelector('[data-card="c2"]');
    expect(closed.querySelector('.k-btn--primary')).toBeNull();
  });

  it('draws remote bots as glass orbs when the engine exists, passing a validated skin and colour', async () => {
    const mountOrb = vi.fn(() => ({ destroy() {} }));
    window.CrewAvatar = { mount: mountOrb };
    try {
      render(RemotePcs, { api: pcsApi([{ id: 'pc1', name: 'Н', online: true }]) });
      await drain();
      expect(mountOrb).toHaveBeenCalled();
      expect(mountOrb.mock.calls[0][1]).toMatchObject({ skin: 3, color: '#7c5cff' });
    } finally {
      delete window.CrewAvatar;
    }
  });

  it('sends a message and updates live from remote_event', async () => {
    const api = chatApi({ 'POST /api/remote/pc%2F1/bots/b%201/messages': async () => ({ queued: false }) });
    const host = render(RemoteChat, chatProps(api));
    await drain();
    const area = host.querySelector('textarea');
    area.value = 'сделай';
    area.dispatchEvent(new Event('input', { bubbles: true }));
    await drain();
    host.querySelector('.rc-composer button[type="submit"]').click();
    await drain();
    const call = api.mock.calls.find(([p, o]) => p.endsWith('/messages') && o?.method === 'POST');
    expect(call[1].body).toEqual({ text: 'сделай' });
    window.dispatchEvent(new CustomEvent('deloist:event', { detail: { type: 'remote_event', pc: 'pc/1', event: { type: 'approval', data: { id: 'c9', botId: 'b 1', title: 'Новая', remote: true, answers: ['once', 'deny'] } } } }));
    await drain();
    expect(host.querySelector('[data-card="c9"]')).not.toBeNull();
    window.dispatchEvent(new CustomEvent('deloist:event', { detail: { type: 'remote_event', pc: 'pc/1', event: { type: 'approval_closed', data: { id: 'c9' } } } }));
    await drain();
    expect(host.querySelector('[data-card="c9"]')).toBeNull();
  });

  it('an offline PC disables the composer', async () => {
    const host = render(RemoteChat, { ...chatProps(chatApi()), pc: { id: 'pc/1', name: 'Н', online: false } });
    await drain();
    expect(host.querySelector('textarea').disabled).toBe(true);
  });
});
