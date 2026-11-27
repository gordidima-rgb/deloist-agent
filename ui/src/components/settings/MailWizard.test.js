import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import MailWizard from './MailWizard.svelte';

const presets = [
  { id: 'yandex', label: 'Яндекс Почта', imapHost: 'imap.yandex.ru', smtpHost: 'smtp.yandex.ru', help: 'https://passport.yandex.ru/security/app-passwords', domains: ['yandex.ru', 'ya.ru', 'yandex.com'] },
  { id: 'mailru', label: 'Почта Mail', help: 'https://help.mail.ru/mail/login/mailer/', domains: ['mail.ru', 'inbox.ru'] },
  { id: 'vk', label: 'VK Почта', help: 'https://help.mail.ru/mail/login/mailer/', domains: ['vk.com'] },
  { id: 'rambler', label: 'Рамблер Почта', help: 'https://help.rambler.ru/mail/mail-pochtovye-klienty/1277', domains: ['rambler.ru', 'lenta.ru'] },
];

let instance;
let host;

function render(api, props = {}) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(MailWizard, { target: host, props: { api, presets, pollMs: 0, ...props } });
  flushSync();
}

async function drain() {
  for (let i = 0; i < 20; i += 1) {
    flushSync();
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

const button = (text) => [...host.querySelectorAll('button')].find((n) => n.textContent.trim() === text);

function type(input, value) {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  flushSync();
}

const email = () => host.querySelector('input[type="email"]');

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
  vi.restoreAllMocks();
});

describe('MailWizard', () => {
  it('detects the service from the domain and shows its steps, not the server fields', () => {
    render(vi.fn());
    expect(host.querySelector('[data-step]')).toBeNull();
    for (const [address, id] of [['a@ya.ru', 'yandex'], ['a@inbox.ru', 'mailru'], ['a@vk.com', 'vk'], ['a@lenta.ru', 'rambler']]) {
      type(email(), address);
      expect(host.querySelector('[data-service]').dataset.service, address).toBe(id);
    }
    expect(host.querySelectorAll('[data-step]').length).toBe(2);
    expect(host.querySelector('[data-advanced]')).toBeNull();
    expect(host.textContent).not.toContain('IMAP: сервер');
  });

  it('opens the exact pages from the step buttons', () => {
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    render(vi.fn());
    type(email(), 'me@yandex.ru');
    expect(host.textContent).toContain('Почтовые программы');
    button('Открыть страницу паролей').click();
    expect(open).toHaveBeenCalledWith('https://passport.yandex.ru/security/app-passwords', '_blank', 'noopener');
    type(email(), 'me@mail.ru');
    expect(host.textContent).toContain('Пароли для внешних приложений');
    button('Открыть справку Mail').click();
    expect(open).toHaveBeenLastCalledWith('https://help.mail.ru/mail/login/mailer/', '_blank', 'noopener');
  });

  it('asks for the service when the domain is unknown and shows the server fields under Advanced', () => {
    render(vi.fn());
    type(email(), 'me@corp.ru');
    expect(host.textContent).toContain('Службу по адресу определить не вышло');
    const select = host.querySelector('select');
    select.value = 'custom';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    flushSync();
    expect(host.querySelector('[data-advanced]')).not.toBeNull();
    expect(host.textContent).toContain('IMAP: сервер');
  });

  it('connects with the detected preset and clears the password', async () => {
    const api = vi.fn(async () => ({ account: {} }));
    const onadded = vi.fn();
    render(api, { onadded });
    type(email(), 'me@yandex.ru');
    expect(button('Подключить').disabled).toBe(true);
    type(host.querySelector('input[type="password"]'), 'app-secret');
    button('Подключить').click();
    await drain();
    expect(api).toHaveBeenCalledWith('/api/mail/accounts', { method: 'POST', body: { address: 'me@yandex.ru', password: 'app-secret', preset: 'yandex' } });
    expect(onadded).toHaveBeenCalledTimes(1);
    expect(host.textContent).not.toContain('app-secret');
  });

  it('shows the plain-language refusal from the server under the form', async () => {
    const message = 'Неверный пароль приложения или IMAP выключен в настройках почты — проверь шаг 1 и 2.';
    render(vi.fn(async () => { throw new Error(message); }));
    type(email(), 'me@mail.ru');
    type(host.querySelector('input[type="password"]'), 'x');
    button('Подключить').click();
    await drain();
    expect(host.querySelector('[role="alert"]').textContent).toBe(message);
  });

  it('shows the Yandex button only when the build enables it and only for Yandex addresses', () => {
    render(vi.fn(), { yandexOauth: false });
    type(email(), 'me@yandex.ru');
    expect(button('Войти через Яндекс')).toBeUndefined();
    unmount(instance);
    host.remove();
    render(vi.fn(), { yandexOauth: true });
    type(email(), 'me@yandex.ru');
    expect(button('Войти через Яндекс')).toBeDefined();
    type(email(), 'me@mail.ru');
    expect(button('Войти через Яндекс')).toBeUndefined();
  });

  it('signs in with Yandex: starts, waits until ready, then finishes', async () => {
    const states = [{ status: 'waiting' }, { status: 'ready', address: 'me@yandex.ru' }];
    const api = vi.fn(async (path) => {
      if (path === '/api/mail/oauth/yandex/start') return { flow: 'oa1' };
      if (path === '/api/mail/oauth/yandex/oa1') return states.shift();
      if (path === '/api/mail/oauth/yandex/oa1/finish') return { account: {} };
      throw new Error(path);
    });
    const onadded = vi.fn();
    render(api, { yandexOauth: true, onadded });
    type(email(), 'me@yandex.ru');
    button('Войти через Яндекс').click();
    await drain();
    expect(api).toHaveBeenCalledWith('/api/mail/oauth/yandex/oa1/finish', { method: 'POST', body: {} });
    expect(onadded).toHaveBeenCalledTimes(1);
    expect(host.querySelector('[role="alert"]')).toBeNull();
  });

  it('shows a failed Yandex sign-in as a message', async () => {
    const api = vi.fn(async (path) => (path.endsWith('/start') ? { flow: 'oa1' } : { status: 'failed', error: 'вход отменён в браузере' }));
    render(api, { yandexOauth: true });
    type(email(), 'me@yandex.ru');
    button('Войти через Яндекс').click();
    await drain();
    expect(host.querySelector('[role="alert"]').textContent).toBe('вход отменён в браузере');
  });
});
