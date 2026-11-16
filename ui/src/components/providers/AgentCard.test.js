import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import AgentCard from './AgentCard.svelte';

const base = {
  id: 'antigravity', name: 'Gemini', supported: true, installed: null, latest: '1.5.7', update: false,
  status: 'not_installed', accounts: [], maxAccounts: 3, models: [], hiddenModels: [], isDefault: false, hasKey: false,
};
const account = (id, label, extra = {}) => ({ id, kind: 'google', label, connected: true, status: 'connected', ...extra });

let instance;
let host;

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

function render(provider, onaction = vi.fn()) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(AgentCard, { target: host, props: { provider, onaction } });
  flushSync();
  return { host, onaction };
}

const button = (root, text) => [...root.querySelectorAll('button')].find((b) => b.textContent.trim() === text);

describe('AgentCard', () => {
  it('not installed: one Connect button that starts the install', () => {
    const { host, onaction } = render(base);
    expect(host.dataset?.state ?? host.querySelector('.agc').dataset.state).toBe('not-installed');
    button(host, 'Подключить').click();
    expect(onaction).toHaveBeenCalledWith('install');
    expect(host.textContent).not.toContain('Ключ API');
  });

  it('installing: shows percent, signature check and unpacking, and can be cancelled', () => {
    let { host, onaction } = render({ ...base, status: 'installing', progress: { phase: 'installing', percent: 42 } });
    expect(host.querySelector('.agc').dataset.state).toBe('installing');
    expect(host.querySelector('[data-testid="agc-progress"]').textContent).toContain('42');
    button(host, 'Отмена').click();
    expect(onaction).toHaveBeenCalledWith('cancel');
    unmount(instance);
    host.remove();
    ({ host } = render({ ...base, status: 'verifying' }));
    expect(host.querySelector('[data-testid="agc-progress"]').textContent).toContain('Проверка подписи');
    unmount(instance);
    host.remove();
    ({ host } = render({ ...base, status: 'unpacking' }));
    expect(host.querySelector('[data-testid="agc-progress"]').textContent).toContain('Распаковка');
  });

  it('signing in: tells the person to finish in the browser', () => {
    const { host } = render({ ...base, installed: '1.5.7', status: 'connecting' });
    expect(host.querySelector('.agc').dataset.state).toBe('signing-in');
    expect(host.querySelector('[data-testid="agc-signin"]').textContent).toContain('браузер');
  });

  it('signing in with a known link: open and copy buttons, the link itself is never in the page', () => {
    const { host, onaction } = render({ ...base, installed: '1.5.7', status: 'connecting', signinLink: true });
    button(host, 'Открыть страницу входа').click();
    expect(onaction).toHaveBeenLastCalledWith('signin-open');
    button(host, 'Скопировать ссылку').click();
    expect(onaction).toHaveBeenLastCalledWith('signin-copy');
    expect(host.innerHTML).not.toContain('accounts.google.com');
    unmount(instance);
    host.remove();
    const again = render({ ...base, installed: '1.5.7', status: 'connecting', signinLink: false });
    expect(button(again.host, 'Открыть страницу входа')).toBeUndefined();
  });

  it('error: plain words and a way to retry', () => {
    const { host, onaction } = render({ ...base, status: 'error', error: 'архив агента не совпал с контрольной суммой: не ставлю' });
    expect(host.querySelector('.agc').dataset.state).toBe('error');
    expect(host.querySelector('[data-testid="agc-error"]').textContent).toContain('контрольной суммой');
    button(host, 'Подключить').click();
    expect(onaction).toHaveBeenCalledWith('install');
  });

  it('installed without an account: Connect signs in', () => {
    const { host, onaction } = render({ ...base, installed: '1.5.7', status: 'not_connected' });
    expect(host.querySelector('.agc').dataset.state).toBe('not-connected');
    button(host, 'Подключить').click();
    expect(onaction).toHaveBeenCalledWith('connect');
  });

  it('connected: accounts, update, models with switches, default and disconnect', () => {
    const provider = {
      ...base, installed: '1.5.7', latest: '1.5.8', update: true, status: 'connected', hasKey: true, keyTail: 'WXYZ',
      accounts: [account('a1', 'me@example.com'), account('key', 'Ключ API', { kind: 'key' })],
      models: [{ id: 'gemini-pro', name: 'Gemini Pro' }, { id: 'gpt-oss', name: 'GPT OSS' }],
      hiddenModels: ['gpt-oss'],
    };
    const { host, onaction } = render(provider);
    expect(host.querySelector('.agc').dataset.state).toBe('connected');
    expect(host.textContent).toContain('me@example.com');
    expect(host.textContent).toContain('…WXYZ');
    expect(host.textContent).toContain('1 из 2');
    button(host, 'Обновить').click();
    expect(onaction).toHaveBeenLastCalledWith('install');
    button(host, 'Проверить обновления').click();
    expect(onaction).toHaveBeenLastCalledWith('refresh');
    button(host, 'Перезапустить').click();
    expect(onaction).toHaveBeenLastCalledWith('check');
    button(host, 'Добавить аккаунт').click();
    expect(onaction).toHaveBeenLastCalledWith('account-add');
    [...host.querySelectorAll('li')].find((li) => li.textContent.includes('me@example.com')).querySelector('button').click();
    expect(onaction).toHaveBeenLastCalledWith('account-remove', { id: 'a1' });
    button(host, 'Убрать ключ').click();
    expect(onaction).toHaveBeenLastCalledWith('key-remove');
    const hiddenRow = host.querySelector('[data-model="gpt-oss"] [role="switch"]');
    expect(hiddenRow.getAttribute('aria-checked')).toBe('false');
    hiddenRow.click();
    expect(onaction).toHaveBeenLastCalledWith('model', { id: 'gpt-oss', on: true });
    button(host, 'Сделать основным').click();
    expect(onaction).toHaveBeenLastCalledWith('default');
    button(host, 'Отключить').click();
    expect(onaction).toHaveBeenLastCalledWith('disconnect');
  });

  it('the fourth account is not offered when three are connected', () => {
    const { host } = render({ ...base, installed: '1.5.7', status: 'connected', accounts: [account('a1', 'a'), account('a2', 'b'), account('a3', 'c')] });
    expect(button(host, 'Добавить аккаунт')).toBeUndefined();
  });

  it('the key field sends the key once and clears itself; the key is never shown', () => {
    const { host, onaction } = render({ ...base, installed: '1.5.7', status: 'connected', accounts: [account('a1', 'me@example.com')] });
    const input = host.querySelector('input[type="password"]');
    input.value = 'AIzaSyFAKEKEY0123456789';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
    button(host, 'Сохранить ключ').click();
    flushSync();
    expect(onaction).toHaveBeenCalledWith('key-save', { apiKey: 'AIzaSyFAKEKEY0123456789' });
    expect(host.querySelector('input[type="password"]').value).toBe('');
    expect(host.textContent).not.toContain('FAKEKEY');
  });
});

describe('AgentCard for an ACP agent without accounts', () => {
  const claude = {
    id: 'claude-acp', name: 'Claude', kind: 'acp', supported: true, installed: '2.1.0', latest: '2.2.0', update: true,
    status: 'connected', enabled: true, isDefault: false, models: [{ id: 'sonnet', name: 'Sonnet' }, { id: 'haiku', name: 'Haiku' }], hiddenModels: [],
  };

  it('has no accounts block and no Gemini-only controls, but update and restart', () => {
    const { host, onaction } = render(claude);
    expect(host.querySelector('.agc').dataset.state).toBe('connected');
    expect(host.querySelector('h3').textContent).toBe('Claude');
    expect(host.textContent).not.toContain('Аккаунты');
    expect(host.textContent).not.toContain('Ключ API');
    expect(host.textContent).not.toContain('Google');
    expect(button(host, 'Проверить обновления')).toBeUndefined();
    expect(host.textContent).toContain('v2.1.0 · доступна 2.2.0');
    button(host, 'Обновить').click();
    expect(onaction).toHaveBeenLastCalledWith('install');
    button(host, 'Перезапустить').click();
    expect(onaction).toHaveBeenLastCalledWith('check');
    button(host, 'Удалить').click();
    expect(onaction).toHaveBeenLastCalledWith('remove');
  });

  it('no latest version: no update line, plain version', () => {
    const { host } = render({ ...claude, latest: undefined, update: false });
    expect(host.textContent).toContain('v2.1.0 · загружена Deloist');
    expect(button(host, 'Обновить')).toBeUndefined();
  });

  it('shows who is signed in as a read-only line when the program knows it', () => {
    const { host } = render({ ...claude, account: { label: 'me@example.com' } });
    expect(host.textContent).toContain('me@example.com');
    expect(button(host, 'Выйти')).toBeUndefined();
    expect(button(host, 'Добавить аккаунт')).toBeUndefined();
  });

  it('switches: models, hide all and the enabled toggle report the action', () => {
    const { host, onaction } = render(claude);
    host.querySelector('[data-model="haiku"] [role="switch"]').click();
    expect(onaction).toHaveBeenLastCalledWith('model', { id: 'haiku', on: false });
    button(host, 'Скрыть все').click();
    expect(onaction).toHaveBeenLastCalledWith('models-all', { hide: true });
    host.querySelector('[data-row="enabled"] [role="switch"]').click();
    expect(onaction).toHaveBeenLastCalledWith('enabled', { on: false });
    button(host, 'Сделать основным').click();
    expect(onaction).toHaveBeenLastCalledWith('default');
  });

  it('not installed: neutral text and one button; npm install shows its own step', () => {
    let { host, onaction } = render({ ...claude, installed: null, update: false, status: 'not_installed', models: [] });
    expect(host.querySelector('.agc').dataset.state).toBe('not-installed');
    expect(host.textContent).not.toContain('Google');
    expect(host.textContent).not.toContain('85 МБ');
    button(host, 'Подключить').click();
    expect(onaction).toHaveBeenCalledWith('install');
    unmount(instance);
    host.remove();
    ({ host } = render({ ...claude, installed: null, status: 'npm' }));
    expect(host.querySelector('[data-testid="agc-progress"]').textContent).toContain('Ставлю агента');
  });

  it('error without a connection: plain words and a retry', () => {
    const { host, onaction } = render({ ...claude, status: 'error', error: 'агент не ответил' });
    expect(host.querySelector('[data-testid="agc-error"]').textContent).toContain('не ответил');
    button(host, 'Подключить').click();
    expect(onaction).toHaveBeenCalledWith('connect');
  });
});
