import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import DiscordSettings from './DiscordSettings.svelte';

const bots = [
  { id: 'b1', name: 'Альфа' },
  { id: 'b2', name: 'Бета' },
];
const disconnected = { connected: false, enabled: false, botId: '', paired: false, username: '' };
const connected = { connected: true, enabled: true, botId: 'b1', paired: false, username: 'demo_bot' };
const paired = { ...connected, paired: true };

let instance;
let host;

function render(api, props = {}) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(DiscordSettings, { target: host, props: { api, bots, ...props } });
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

function getCalls(api) {
  return api.mock.calls.filter(([path, opts]) => path === '/api/discord' && !opts?.method).length;
}

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('DiscordSettings', () => {
  it('sends the token and the chosen bot, then keeps no token in the form', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/discord' && !opts.method) return disconnected;
      if (path === '/api/discord' && opts.method === 'PUT') return { username: 'demo_bot' };
      throw new Error(`unexpected request: ${path}`);
    });
    const onchanged = vi.fn();
    render(api, { onchanged });
    await drain();

    const input = host.querySelector('input[type="password"]');
    expect(input).not.toBeNull();
    input.value = 'discord-secret';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    const select = host.querySelector('select');
    select.value = 'b1';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    flushSync();

    button('Подключить').click();
    await drain();

    expect(api).toHaveBeenCalledWith('/api/discord', {
      method: 'PUT',
      body: { token: 'discord-secret', botId: 'b1' },
    });
    expect(onchanged).toHaveBeenCalledTimes(1);
    expect(host.querySelector('input[type="password"]')).toBeNull();
    expect(host.textContent).not.toContain('discord-secret');
    expect(host.textContent).toContain('Бот demo_bot подключён.');
  });

  it('shows the pairing code and stops polling once the DMs are paired', async () => {
    vi.useFakeTimers();
    let isPaired = false;
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/discord' && !opts.method) return isPaired ? paired : connected;
      if (path === '/api/discord/pair' && opts.method === 'POST') {
        isPaired = true;
        return { code: '123456', expiresAt: Date.now() + 600000 };
      }
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    button('Получить код').click();
    await drain();
    expect(host.querySelector('.dc-code').textContent).toBe('123456');
    expect(host.textContent).toContain('start 123456');

    await vi.advanceTimersByTimeAsync(3000);
    await drain();
    expect(host.textContent).toContain('Личка привязана');
    expect(host.querySelector('.dc-code')).toBeNull();

    const calls = getCalls(api);
    await vi.advanceTimersByTimeAsync(9000);
    await drain();
    expect(getCalls(api)).toBe(calls);
  });

  it('asks for confirmation and calls DELETE when the person disconnects', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/discord' && !opts.method) return paired;
      if (path === '/api/discord' && opts.method === 'DELETE') return { ok: true };
      throw new Error(`unexpected request: ${path}`);
    });
    const onchanged = vi.fn();
    render(api, { onchanged });
    await drain();
    expect(host.textContent).toContain('Личка привязана. Пиши боту demo_bot — отвечает Альфа.');

    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false);
    button('Отключить').click();
    await drain();
    expect(api).not.toHaveBeenCalledWith('/api/discord', { method: 'DELETE' });

    confirm.mockReturnValue(true);
    button('Отключить').click();
    await drain();
    expect(confirm).toHaveBeenCalled();
    expect(api).toHaveBeenCalledWith('/api/discord', { method: 'DELETE' });
    expect(onchanged).toHaveBeenCalledTimes(1);
    expect(host.querySelector('input[type="password"]')).not.toBeNull();
  });

  it('changes the answering bot with a PUT that carries no token', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/discord' && !opts.method) return paired;
      if (path === '/api/discord' && opts.method === 'PUT') return { username: 'demo_bot' };
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    const select = host.querySelector('select');
    expect(select.value).toBe('b1');
    select.value = 'b2';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await drain();

    expect(api).toHaveBeenCalledWith('/api/discord', { method: 'PUT', body: { botId: 'b2' } });
    for (const [path, opts] of api.mock.calls) {
      if (path === '/api/discord' && opts?.method === 'PUT') expect(opts.body).not.toHaveProperty('token');
    }
    expect(host.querySelector('select').value).toBe('b2');
    expect(host.textContent).toContain('— отвечает Бета.');
  });

  it('shows the server error under the connect form and keeps the token for a retry', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/discord' && !opts.method) return disconnected;
      if (path === '/api/discord' && opts.method === 'PUT') throw new Error('Discord не принял токен: Unauthorized');
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    const input = host.querySelector('input[type="password"]');
    input.value = 'bad-token';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
    button('Подключить').click();
    await drain();

    expect(host.querySelector('[role="alert"]').textContent).toBe('Discord не принял токен: Unauthorized');
    expect(host.querySelector('input[type="password"]').value).toBe('bad-token');
  });

  it('expires the pairing code and stops polling after ten minutes', async () => {
    vi.useFakeTimers();
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/discord' && !opts.method) return connected;
      if (path === '/api/discord/pair' && opts.method === 'POST') {
        return { code: '111222', expiresAt: Date.now() + 600000 };
      }
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    button('Получить код').click();
    await drain();
    expect(host.querySelector('.dc-code').textContent).toBe('111222');

    await vi.advanceTimersByTimeAsync(600000);
    await drain();
    expect(host.querySelector('.dc-code')).toBeNull();
    expect(host.textContent).toContain('Код истёк');

    const calls = getCalls(api);
    await vi.advanceTimersByTimeAsync(9000);
    await drain();
    expect(getCalls(api)).toBe(calls);
  });

  it('stops the pairing poll when the screen is unmounted', async () => {
    vi.useFakeTimers();
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/discord' && !opts.method) return connected;
      if (path === '/api/discord/pair' && opts.method === 'POST') {
        return { code: '654321', expiresAt: Date.now() + 600000 };
      }
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    button('Получить код').click();
    await drain();
    expect(host.querySelector('.dc-code').textContent).toBe('654321');

    const calls = getCalls(api);
    unmount(instance);
    instance = null;
    await vi.advanceTimersByTimeAsync(3000);
    expect(getCalls(api)).toBe(calls);
  });
});
