import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import TelegramSettings from './TelegramSettings.svelte';

const bots = [
  { id: 'b1', name: 'Альфа' },
  { id: 'b2', name: 'Бета' },
];
const disconnected = { connected: false, enabled: false, botId: '', chatPaired: false, username: '' };
const connected = { connected: true, enabled: true, botId: 'b1', chatPaired: false, username: 'demo_bot' };
const paired = { ...connected, chatPaired: true };

let instance;
let host;

function render(api, props = {}) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(TelegramSettings, { target: host, props: { api, bots, ...props } });
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
  return api.mock.calls.filter(([path, opts]) => path === '/api/telegram' && !opts?.method).length;
}

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('TelegramSettings', () => {
  it('sends the token and the chosen bot, then keeps no token in the form', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/telegram' && !opts.method) return disconnected;
      if (path === '/api/telegram' && opts.method === 'PUT') return { username: 'demo_bot' };
      throw new Error(`unexpected request: ${path}`);
    });
    const onchanged = vi.fn();
    render(api, { onchanged });
    await drain();

    const input = host.querySelector('input[type="password"]');
    expect(input).not.toBeNull();
    input.value = '123456:secret';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    const select = host.querySelector('select');
    select.value = 'b1';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    flushSync();

    button('Подключить').click();
    await drain();

    expect(api).toHaveBeenCalledWith('/api/telegram', {
      method: 'PUT',
      body: { token: '123456:secret', botId: 'b1' },
    });
    expect(onchanged).toHaveBeenCalledTimes(1);
    expect(host.querySelector('input[type="password"]')).toBeNull();
    expect(host.textContent).not.toContain('123456:secret');
    expect(host.textContent).toContain('Бот @demo_bot подключён.');
  });

  it('shows the pairing code and stops polling once the chat is paired', async () => {
    vi.useFakeTimers();
    let isPaired = false;
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/telegram' && !opts.method) return isPaired ? paired : connected;
      if (path === '/api/telegram/pair' && opts.method === 'POST') {
        isPaired = true;
        return { code: '123456', expiresAt: Date.now() + 600000 };
      }
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    button('Получить код').click();
    await drain();
    expect(host.querySelector('.tg-code').textContent).toBe('123456');
    expect(host.textContent).toContain('/start 123456');

    await vi.advanceTimersByTimeAsync(3000);
    await drain();
    expect(host.textContent).toContain('Чат привязан');
    expect(host.querySelector('.tg-code')).toBeNull();

    const calls = getCalls(api);
    await vi.advanceTimersByTimeAsync(9000);
    await drain();
    expect(getCalls(api)).toBe(calls);
  });

  it('asks for confirmation and calls DELETE when the person disconnects', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/telegram' && !opts.method) return paired;
      if (path === '/api/telegram' && opts.method === 'DELETE') return { ok: true };
      throw new Error(`unexpected request: ${path}`);
    });
    const onchanged = vi.fn();
    render(api, { onchanged });
    await drain();
    expect(host.textContent).toContain('Чат привязан. Пиши боту @demo_bot — отвечает Альфа.');

    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false);
    button('Отключить').click();
    await drain();
    expect(api).not.toHaveBeenCalledWith('/api/telegram', { method: 'DELETE' });

    confirm.mockReturnValue(true);
    button('Отключить').click();
    await drain();
    expect(confirm).toHaveBeenCalled();
    expect(api).toHaveBeenCalledWith('/api/telegram', { method: 'DELETE' });
    expect(onchanged).toHaveBeenCalledTimes(1);
    expect(host.querySelector('input[type="password"]')).not.toBeNull();
  });

  it('shows the server error under the connect form and keeps the token for a retry', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/telegram' && !opts.method) return disconnected;
      if (path === '/api/telegram' && opts.method === 'PUT') throw new Error('Telegram не принял токен: Unauthorized');
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

    expect(host.querySelector('[role="alert"]').textContent).toBe('Telegram не принял токен: Unauthorized');
    expect(host.querySelector('input[type="password"]').value).toBe('bad-token');
  });

  it('stops the pairing poll when the screen is unmounted', async () => {
    vi.useFakeTimers();
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/telegram' && !opts.method) return connected;
      if (path === '/api/telegram/pair' && opts.method === 'POST') {
        return { code: '654321', expiresAt: Date.now() + 600000 };
      }
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    button('Получить код').click();
    await drain();
    expect(host.querySelector('.tg-code').textContent).toBe('654321');

    const calls = getCalls(api);
    unmount(instance);
    instance = null;
    await vi.advanceTimersByTimeAsync(3000);
    expect(getCalls(api)).toBe(calls);
  });
});
