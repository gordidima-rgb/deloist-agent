import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import MailSettings from './MailSettings.svelte';

const bots = [
  { id: 'b1', name: 'Альфа' },
  { id: 'b2', name: 'Бета' },
];
const routines = [
  { id: 'r1', botId: 'b1', title: 'Разбор почты', kind: 'task' },
  { id: 'r2', botId: 'b1', title: 'Напоминание', kind: 'reminder' },
  { id: 'r3', botId: 'b2', title: 'Хук', kind: 'task', hook: true },
];
const presets = [{ id: 'yandex', label: 'Яндекс', help: 'https://id.yandex.ru/security/app-passwords' }];
const account = {
  id: 'a1',
  label: 'Рабочая',
  address: 'me@yandex.ru',
  bots: ['b1'],
  autoBots: [],
  allow: ['@corp.ru'],
  trusted: [],
  hasPassword: false,
  lastError: 'login failed',
};

let instance;
let host;

function makeApi(overrides = {}, mail = {}) {
  const state = { accounts: [account], presets, triggers: [], ...mail };
  return vi.fn(async (path, opts = {}) => {
    const key = `${opts.method || 'GET'} ${path}`;
    if (overrides[key]) return overrides[key](opts);
    if (key === 'GET /api/mail') return state;
    if (key === 'GET /api/mail/sent') return { sent: [] };
    throw new Error(`unexpected request: ${key}`);
  });
}

function render(api, props = {}) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(MailSettings, { target: host, props: { api, bots, routines, ...props } });
  flushSync();
  return host;
}

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

function type(input, value) {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
  vi.restoreAllMocks();
});

describe('MailSettings', () => {
  it('lists the mailbox with a no-password badge, the error and the help link', async () => {
    render(makeApi());
    await drain();
    expect(host.textContent).toContain('Рабочая');
    expect(host.textContent).toContain('me@yandex.ru');
    expect(host.textContent).toContain('нет пароля');
    expect(host.textContent).toContain('login failed');
    const link = host.querySelector('a[href^="https://id.yandex.ru"]');
    expect(link.textContent).toBe('Создать пароль приложения');
  });

  it('checks a mailbox and asks before deleting it', async () => {
    const api = makeApi({
      'POST /api/mail/accounts/a1/check': () => ({ ok: true }),
      'DELETE /api/mail/accounts/a1': () => ({ ok: true }),
    });
    const onchanged = vi.fn();
    render(api, { onchanged });
    await drain();

    button('Проверить').click();
    await drain();
    expect(api).toHaveBeenCalledWith('/api/mail/accounts/a1/check', { method: 'POST' });

    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false);
    button('Удалить').click();
    await drain();
    expect(api).not.toHaveBeenCalledWith('/api/mail/accounts/a1', { method: 'DELETE' });
    confirm.mockReturnValue(true);
    button('Удалить').click();
    await drain();
    expect(api).toHaveBeenCalledWith('/api/mail/accounts/a1', { method: 'DELETE' });
    expect(onchanged).toHaveBeenCalledTimes(1);
  });

  it('adds a mailbox from a preset and clears the password afterwards', async () => {
    const api = makeApi({ 'POST /api/mail/accounts': () => ({ account }) });
    render(api);
    await drain();
    type(host.querySelector('input[type="email"]'), 'new@yandex.ru');
    const passwords = host.querySelectorAll('input[type="password"]');
    type(passwords[0], 'app-secret');
    flushSync();
    button('Подключить ящик').click();
    await drain();
    expect(api).toHaveBeenCalledWith('/api/mail/accounts', {
      method: 'POST',
      body: { address: 'new@yandex.ru', password: 'app-secret', preset: 'yandex' },
    });
    expect(host.textContent).not.toContain('app-secret');
  });

  it('shows the server refusal under the add form', async () => {
    const api = makeApi({
      'POST /api/mail/accounts': () => {
        throw new Error('Сервер не принял вход');
      },
    });
    render(api);
    await drain();
    type(host.querySelector('input[type="email"]'), 'new@yandex.ru');
    type(host.querySelectorAll('input[type="password"]')[0], 'x');
    flushSync();
    button('Подключить ящик').click();
    await drain();
    expect(host.querySelector('[role="alert"]').textContent).toBe('Сервер не принял вход');
  });

  it('patches only the changed fields from the settings card', async () => {
    const api = makeApi({ 'PATCH /api/mail/accounts/a1': () => ({ account }) });
    render(api);
    await drain();
    button('Настроить').click();
    flushSync();
    expect(host.textContent).toContain('Автоматическая работа');

    // Белый список: добавляем адрес, остальное не трогаем.
    const areas = host.querySelectorAll('textarea');
    type(areas[0], '@corp.ru\nboss@x.ru');
    flushSync();
    // Включаем авторабота бота с доступом.
    host.querySelector('.ml-switch-row button[role="switch"]').click();
    flushSync();
    button('Сохранить').click();
    await drain();
    expect(api).toHaveBeenCalledWith('/api/mail/accounts/a1', {
      method: 'PATCH',
      body: { autoBots: ['b1'], allow: ['@corp.ru', 'boss@x.ru'] },
    });
  });

  it('offers only routines with a mailbox for their bot and saves a trigger', async () => {
    const api = makeApi({ 'PUT /api/mail/triggers/r1': () => ({ trigger: {} }) });
    render(api);
    await drain();
    const selects = [...host.querySelectorAll('select')];
    const routineSelect = selects.find((s) => [...s.options].some((o) => o.textContent === 'Разбор почты'));
    expect([...routineSelect.options].map((o) => o.textContent)).toEqual(['Разбор почты']);

    const inputs = [...host.querySelectorAll('input')];
    const from = inputs.find((i) => i.closest('.k-field')?.textContent.includes('От кого содержит'));
    type(from, 'boss@');
    flushSync();
    button('Сохранить триггер').click();
    await drain();
    expect(api).toHaveBeenCalledWith('/api/mail/triggers/r1', {
      method: 'PUT',
      body: { accountId: 'a1', folder: 'INBOX', from: 'boss@', subject: '' },
    });
  });

  it('says to give access first when the routine bot has no mailbox', async () => {
    render(makeApi({}, { accounts: [{ ...account, bots: ['b2'] }] }));
    await drain();
    expect(host.textContent).toContain('Сначала откройте ящик боту этой рутины');
    expect(button('Сохранить триггер')).toBeUndefined();
  });

  it('shows an existing trigger and removes it', async () => {
    const api = makeApi(
      { 'DELETE /api/mail/triggers/r1': () => ({ ok: true }) },
      { triggers: [{ routineId: 'r1', accountId: 'a1', folder: 'INBOX', from: 'boss@', subject: '', lastError: 'boom' }] },
    );
    render(api);
    await drain();
    const card = host.querySelector('[data-trigger="r1"]');
    expect(card.textContent).toContain('Разбор почты');
    expect(card.textContent).toContain('от: boss@');
    expect(card.textContent).toContain('boom');
    button('Убрать').click();
    await drain();
    expect(api).toHaveBeenCalledWith('/api/mail/triggers/r1', { method: 'DELETE' });
  });

  it('shows the sent log, newest first, and an empty state without rows', async () => {
    const rows = Array.from({ length: 25 }, (_, i) => ({
      at: 1700000000000 + i,
      bot: 'Альфа',
      label: 'x@y.ru',
      subject: `Тема ${i}`,
      how: i === 0 ? 'auto' : 'approved',
    }));
    render(makeApi({ 'GET /api/mail/sent': () => ({ sent: rows }) }));
    await drain();
    expect(host.querySelectorAll('.ml-sent-row').length).toBe(20);
    expect(host.querySelector('.ml-sent-row').textContent).toContain('авто');
    unmount(instance);
    host.remove();
    render(makeApi());
    await drain();
    expect(host.textContent).toContain('Бот пока ничего не отправлял');
  });
});
