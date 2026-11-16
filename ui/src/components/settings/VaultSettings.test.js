import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import VaultSettings from './VaultSettings.svelte';

const absent = {
  bitwarden: { cli: false, status: 'absent', email: '' },
  onepassword: { cli: false, connected: false },
};
const locked = {
  bitwarden: { cli: true, status: 'locked', email: 'me@example.com' },
  onepassword: { cli: true, connected: false },
};
const unlocked = {
  bitwarden: { cli: true, status: 'unlocked', email: 'me@example.com' },
  onepassword: { cli: true, connected: false },
};
const connected = {
  bitwarden: { cli: true, status: 'unlocked', email: 'me@example.com' },
  onepassword: { cli: true, connected: true },
};
const logins = [
  {
    id: 'bw:one',
    source: 'bitwarden',
    title: 'Мой сайт',
    username: 'me@example.com',
    hasTotp: true,
    urls: ['https://example.com'],
  },
  {
    id: 'op:two',
    source: '1password',
    title: 'Работа',
    username: 'work@example.com',
    hasTotp: false,
    urls: ['https://example.com/login'],
  },
];

let instance;
let host;

function render(api, props = {}) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(VaultSettings, { target: host, props: { api, ...props } });
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

function statusCalls(api) {
  return api.mock.calls.filter(([path, opts]) => path === '/api/vault' && !opts?.method).length;
}

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
  vi.restoreAllMocks();
});

describe('VaultSettings', () => {
  it('unlocks Bitwarden with the master password and keeps it out of the page', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/vault' && !opts.method) return locked;
      if (path === '/api/vault/bitwarden/unlock' && opts.method === 'POST') return { status: 'unlocked' };
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    const input = host.querySelector('input[type="password"]');
    expect(input).not.toBeNull();
    input.value = 'master-secret';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();

    button('Разблокировать').click();
    await drain();

    expect(api).toHaveBeenCalledWith('/api/vault/bitwarden/unlock', {
      method: 'POST',
      body: { password: 'master-secret' },
    });
    expect(host.textContent).toContain('Открыт для me@example.com.');
    expect(host.textContent).not.toContain('master-secret');
    const labels = [...host.querySelectorAll('label')].map((node) => node.textContent.trim());
    expect(labels).not.toContain('Мастер-пароль');
    for (const node of host.querySelectorAll('input[type="password"]')) expect(node.value).toBe('');
    const textInputs = [...host.querySelectorAll('input[type="text"]')];
    expect(textInputs.some((node) => node.value === 'master-secret')).toBe(false);
  });

  it('clears the master password and shows the server error after a 400', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/vault' && !opts.method) return locked;
      if (path === '/api/vault/bitwarden/unlock' && opts.method === 'POST') {
        throw new Error('Неверный мастер-пароль');
      }
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    const input = host.querySelector('input[type="password"]');
    input.value = 'wrong-secret';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();

    button('Разблокировать').click();
    await drain();

    expect(host.querySelector('.k-hint.is-error').textContent).toBe('Неверный мастер-пароль');
    expect(host.querySelector('input[type="password"]').value).toBe('');
    expect(host.textContent).not.toContain('wrong-secret');
    const textInputs = [...host.querySelectorAll('input[type="text"]')];
    expect(textInputs.some((node) => node.value === 'wrong-secret')).toBe(false);
  });

  it('locks Bitwarden and brings the master-password field back', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/vault' && !opts.method) return unlocked;
      if (path === '/api/vault/bitwarden/lock' && opts.method === 'POST') return { status: 'locked' };
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    button('Заблокировать').click();
    await drain();

    expect(api).toHaveBeenCalledWith('/api/vault/bitwarden/lock', { method: 'POST' });
    expect(host.querySelector('input[type="password"]')).not.toBeNull();
    expect(host.textContent).toContain('Мастер-пароль');
  });

  it('connects 1Password with the token and clears the field', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/vault' && !opts.method) return unlocked;
      if (path === '/api/vault/onepassword' && opts.method === 'PUT') return { connected: true };
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    const input = host.querySelector('input[type="password"]');
    input.value = 'ops_secret';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();

    button('Подключить').click();
    await drain();

    expect(api).toHaveBeenCalledWith('/api/vault/onepassword', {
      method: 'PUT',
      body: { token: 'ops_secret' },
    });
    expect(host.textContent).toContain('Подключено');
    expect(host.querySelector('input[type="password"]')).toBeNull();
    expect(host.textContent).not.toContain('ops_secret');
  });

  it('asks for confirmation and calls DELETE when 1Password is disconnected', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/vault' && !opts.method) return connected;
      if (path === '/api/vault/onepassword' && opts.method === 'DELETE') return { connected: false };
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();
    expect(host.textContent).toContain('Подключено');

    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false);
    button('Отключить').click();
    await drain();
    expect(api).not.toHaveBeenCalledWith('/api/vault/onepassword', { method: 'DELETE' });

    confirm.mockReturnValue(true);
    button('Отключить').click();
    await drain();
    expect(confirm).toHaveBeenCalled();
    expect(api).toHaveBeenCalledWith('/api/vault/onepassword', { method: 'DELETE' });
    expect(host.textContent).toContain('Скопируй токен');
  });

  it('normalises the site address and renders the found logins', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/vault' && !opts.method) return absent;
      if (path.startsWith('/api/vault/logins?origin=')) return { logins };
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    const input = host.querySelector('input[type="text"]');
    input.value = 'example.com/login';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();

    button('Проверить').click();
    await drain();

    const call = api.mock.calls.find(([path]) => path.startsWith('/api/vault/logins?origin='));
    expect(call).toBeTruthy();
    expect(new URL(call[0], 'http://local').searchParams.get('origin')).toBe('https://example.com');
    expect(host.textContent).toContain('Мой сайт');
    expect(host.textContent).toContain('me@example.com');
    expect(host.textContent).toContain('Bitwarden');
    expect(host.textContent).toContain('1Password');
    expect(host.textContent).toContain('есть код');
    expect(host.textContent).toContain('Входы для https://example.com');
    expect(host.textContent).not.toContain('pw-secret');
  });

  it('says when a site has no saved logins', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/vault' && !opts.method) return absent;
      if (path.startsWith('/api/vault/logins?origin=')) return { logins: [] };
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    const input = host.querySelector('input[type="text"]');
    input.value = 'https://example.com';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();

    button('Проверить').click();
    await drain();
    expect(host.textContent).toContain('Для этого сайта входов нет');
  });

  it('explains how to install the CLIs and re-checks on demand', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/vault' && !opts.method) return absent;
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    expect(host.textContent).toContain('Нужна программа Bitwarden CLI (bw)');
    expect(host.textContent).toContain('Нужна программа 1Password CLI (op)');
    const before = statusCalls(api);
    [...host.querySelectorAll('button')].find((node) => node.textContent.trim() === 'Проверить снова').click();
    await drain();
    expect(statusCalls(api)).toBeGreaterThan(before);
  });

  it('tells the person to run bw login when Bitwarden is signed out', async () => {
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/vault' && !opts.method) {
        return {
          bitwarden: { cli: true, status: 'unauthenticated', email: '' },
          onepassword: { cli: true, connected: false },
        };
      }
      throw new Error(`unexpected request: ${path}`);
    });
    render(api);
    await drain();

    expect(host.textContent).toContain('Выполни в терминале bw login, потом нажми «Проверить снова»');
    expect(host.querySelector('input[type="password"]')).not.toBeNull();
  });
});
