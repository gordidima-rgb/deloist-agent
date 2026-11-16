import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import SlackSettings from './SlackSettings.svelte';

const bots = [
  { id: 'b1', name: 'Альфа' },
  { id: 'b2', name: 'Бета' },
];
const disconnected = { connected: false, enabled: false, botId: '', paired: false, team: '', username: '' };
const connected = { connected: true, enabled: true, botId: 'b1', paired: false, team: 'Deloist Test', username: 'demo_bot' };
const paired = { ...connected, paired: true };

let instance;
let host;

function render(api, props = {}) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(SlackSettings, { target: host, props: { api, bots, ...props } });
  flushSync();
  return host;
}

// Дожидаемся ответов api и эффектов Svelte без опоры на настоящие таймеры.
async function drain() {
  for (let i = 0; i < 12; i += 1) {
    flushSync();
    await Promise.resolve();
  }
  flushSync();
}

function button(text) {
  return [...host.querySelectorAll('button')].find((node) => node.textContent.trim() === text);
}

function passwords() {
  return [...host.querySelectorAll('input[type="password"]')];
}

function getCalls(api) {
  return api.mock.calls.filter(([path, opts]) => path === '/api/slack' && !opts?.method).length;
}

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
  delete navigator.clipboard;
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('SlackSettings', () => {
  it('sends both tokens and the chosen bot, then keeps no token in the form', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/slack' && !opts.method) return disconnected;
      if (path === '/api/slack' && opts.method === 'PUT') return { username: 'demo_bot', team: 'Deloist Test' };
      throw new Error(`unexpected request: ${path}`);
    });
    const onchanged = vi.fn();
    render(api, { onchanged });
    await drain();

    const fields = passwords();
    expect(fields).toHaveLength(2);
    fields[0].value = 'xoxb-secret';
    fields[0].dispatchEvent(new Event('input', { bubbles: true }));
    fields[1].value = 'xapp-secret';
    fields[1].dispatchEvent(new Event('input', { bubbles: true }));
    const select = host.querySelector('select');
    select.value = 'b1';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    flushSync();

    button('Подключить').click();
    await drain();

    expect(api).toHaveBeenCalledWith('/api/slack', {
      method: 'PUT',
      body: { botToken: 'xoxb-secret', appToken: 'xapp-secret', botId: 'b1' },
    });
    expect(onchanged).toHaveBeenCalledTimes(1);
    expect(passwords()).toHaveLength(0);
    expect(host.textContent).not.toContain('xoxb-secret');
    expect(host.textContent).not.toContain('xapp-secret');
    expect(host.textContent).toContain('Бот demo_bot подключён.');
    expect(host.textContent).toContain('Рабочее пространство: Deloist Test');
  });

  it('shows a manifest for Socket Mode and copies it', async () => {
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/slack' && !opts.method) return disconnected;
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    const manifest = host.querySelector('.sl-manifest').textContent;
    expect(manifest).toContain('socket_mode_enabled: true');
    expect(manifest).toContain('messages_tab_enabled: true');
    expect(manifest).toContain('messages_tab_read_only_enabled: false');
    expect(manifest).toContain('message.im');
    expect(manifest).toContain('chat:write');
    expect(manifest).toContain('im:history');
    expect(manifest).toContain('im:read');
    expect(manifest).toContain('users:read');
    expect(host.textContent).toContain('Install to Workspace');
    expect(host.textContent).toContain('xoxb-…');
    expect(host.textContent).toContain('xapp-…');

    button('Скопировать').click();
    await drain();
    expect(writeText).toHaveBeenCalledWith(manifest);
    expect(button('Скопировано')).toBeTruthy();
  });

  it('shows the pairing code and stops polling once the DMs are paired', async () => {
    vi.useFakeTimers();
    let isPaired = false;
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/slack' && !opts.method) return isPaired ? paired : connected;
      if (path === '/api/slack/pair' && opts.method === 'POST') {
        isPaired = true;
        return { code: '123456', expiresAt: Date.now() + 600000 };
      }
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    button('Получить код').click();
    await drain();
    expect(host.querySelector('.sl-code').textContent).toBe('123456');
    expect(host.textContent).toContain('start 123456');

    await vi.advanceTimersByTimeAsync(3000);
    await drain();
    expect(host.textContent).toContain('Личка привязана');
    expect(host.querySelector('.sl-code')).toBeNull();

    const calls = getCalls(api);
    await vi.advanceTimersByTimeAsync(9000);
    await drain();
    expect(getCalls(api)).toBe(calls);
  });

  it('asks for confirmation and calls DELETE when the person disconnects', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/slack' && !opts.method) return paired;
      if (path === '/api/slack' && opts.method === 'DELETE') return { ok: true };
      throw new Error(`unexpected request: ${path}`);
    });
    const onchanged = vi.fn();
    render(api, { onchanged });
    await drain();
    expect(host.textContent).toContain('Личка привязана. Пиши боту demo_bot в Slack — отвечает Альфа.');

    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false);
    button('Отключить').click();
    await drain();
    expect(api).not.toHaveBeenCalledWith('/api/slack', { method: 'DELETE' });

    confirm.mockReturnValue(true);
    button('Отключить').click();
    await drain();
    expect(confirm).toHaveBeenCalled();
    expect(api).toHaveBeenCalledWith('/api/slack', { method: 'DELETE' });
    expect(onchanged).toHaveBeenCalledTimes(1);
    expect(passwords()).toHaveLength(2);
  });

  it('changes the answering bot with a PUT that carries no tokens', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/slack' && !opts.method) return paired;
      if (path === '/api/slack' && opts.method === 'PUT') return { username: 'demo_bot', team: 'Deloist Test' };
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    const select = host.querySelector('select');
    expect(select.value).toBe('b1');
    select.value = 'b2';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await drain();

    expect(api).toHaveBeenCalledWith('/api/slack', { method: 'PUT', body: { botId: 'b2' } });
    for (const [path, opts] of api.mock.calls) {
      if (path === '/api/slack' && opts?.method === 'PUT') {
        expect(opts.body).not.toHaveProperty('botToken');
        expect(opts.body).not.toHaveProperty('appToken');
      }
    }
    expect(host.querySelector('select').value).toBe('b2');
    expect(host.textContent).toContain('— отвечает Бета.');
  });

  it('shows the server error under the connect form and keeps both tokens for a retry', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/slack' && !opts.method) return disconnected;
      if (path === '/api/slack' && opts.method === 'PUT') throw new Error('Slack отклонил токен');
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    const fields = passwords();
    fields[0].value = 'xoxb-bad';
    fields[0].dispatchEvent(new Event('input', { bubbles: true }));
    fields[1].value = 'xapp-bad';
    fields[1].dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
    button('Подключить').click();
    await drain();

    expect(host.querySelector('[role="alert"]').textContent).toBe('Slack отклонил токен');
    const after = passwords();
    expect(after[0].value).toBe('xoxb-bad');
    expect(after[1].value).toBe('xapp-bad');
  });
});
