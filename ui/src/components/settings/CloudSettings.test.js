import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import CloudSettings from './CloudSettings.svelte';

const B = '/api/cloud/instabox';
const STEPS = [
  { id: 'machine', label: 'Создаю машину' },
  { id: 'port', label: 'Открываю порт' },
  { id: 'docker', label: 'Запускаю Deloist' },
];
const out = { signedIn: false, login: '', machine: null, job: null, signin: null };
const pendingSignin = {
  phase: 'pending',
  signedIn: false,
  userCode: 'ABCD-2345',
  verificationUri: 'https://instabox.sh/device',
  verificationUriComplete: 'https://instabox.sh/device?c=ABCD-2345',
  expiresAt: 1,
  error: null,
};
const signedNoMachine = { ...out, signedIn: true, login: 'dima' };
const deployed = (state = 'running', extra = {}) => ({
  ...signedNoMachine,
  machine: { state, ready: state === 'running', retryAfterSeconds: null, cooldownUntil: null, origin: 'https://m1.instabox.sh', name: 'deloist', ...extra },
  job: null,
});

let instance;
let host;

function render(api) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(CloudSettings, { target: host, props: { api } });
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

const button = (text) => [...host.querySelectorAll('button')].find((n) => n.textContent.trim() === text);
const calls = (api, path) => api.mock.calls.filter(([p]) => p === path);

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('CloudSettings', () => {
  it('shows the header and the connect button when not signed in', async () => {
    const api = vi.fn(async () => out);
    render(api);
    await drain();
    expect(host.textContent).toContain('Боты в облаке — бесплатно');
    expect(host.textContent).toContain('Машина засыпает без дела');
    expect(button('Подключить Instabox')).toBeTruthy();
  });

  it('starts sign-in, shows the code, and the reopen button calls signin/start again', async () => {
    const api = vi.fn(async (path) => (path === `${B}/signin/start` ? pendingSignin : out));
    render(api);
    await drain();
    button('Подключить Instabox').click();
    await drain();
    expect(host.querySelector('[data-testid="cloud-code"]').textContent).toBe('ABCD-2345');
    expect(host.textContent).toContain('нажмите Approve');
    expect(button('Отмена')).toBeTruthy();
    button('Открыть страницу ещё раз').click();
    await drain();
    expect(calls(api, `${B}/signin/start`)).toHaveLength(2);
    button('Отмена').click();
    await drain();
    expect(host.querySelector('[data-testid="cloud-code"]')).toBeNull();
    expect(button('Подключить Instabox')).toBeTruthy();
  });

  it('polls signin/status and moves on when approved', async () => {
    vi.useFakeTimers();
    let signedIn = false;
    const api = vi.fn(async (path) => {
      if (path === `${B}/signin/status`) {
        signedIn = true;
        return { ...pendingSignin, phase: 'approved', signedIn: true };
      }
      return signedIn ? signedNoMachine : { ...out, signin: pendingSignin };
    });
    render(api);
    await drain();
    expect(host.querySelector('[data-testid="cloud-code"]')).not.toBeNull();
    vi.advanceTimersByTime(2100);
    await drain();
    expect(button('Разместить ботов в облаке')).toBeTruthy();
  });

  it('provisions with steps, and shows a plain error with retry', async () => {
    let status = signedNoMachine;
    const api = vi.fn(async (path) => {
      if (path === `${B}/provision`) {
        status = { ...signedNoMachine, job: { kind: 'provision', state: 'error', step: 1, steps: STEPS, error: 'У Instabox нет свободного места.', retryAfterSeconds: null, startedAt: 1 } };
        return { started: true, kind: 'provision' };
      }
      return status;
    });
    render(api);
    await drain();
    button('Разместить ботов в облаке').click();
    await drain();
    expect(calls(api, `${B}/provision`)).toHaveLength(1);
    expect(host.textContent).toContain('Создаю машину');
    expect(host.textContent).toContain('Открываю порт');
    expect(host.textContent).toContain('У Instabox нет свободного места.');
    expect(button('Повторить')).toBeTruthy();
  });

  it('shows the deployed state: chip, address as text, open/update/delete', async () => {
    const api = vi.fn(async (path) => {
      if (path === `${B}/open`) return { ok: true, state: 'running', url: 'https://m1.instabox.sh/auth/gateway?t=x', opened: true };
      return deployed();
    });
    render(api);
    await drain();
    expect(host.querySelector('[data-testid="cloud-chip"]').textContent).toContain('работает');
    const origin = host.querySelector('[data-testid="cloud-origin"]');
    expect(origin.textContent).toBe('https://m1.instabox.sh');
    expect(host.querySelector('a')).toBeNull();
    expect(host.textContent).not.toContain('gateway?t=');
    button('Открыть облако').click();
    await drain();
    expect(calls(api, `${B}/open`)[0][1]).toEqual({ method: 'POST', body: { open: true } });
    button('Обновить').click();
    await drain();
    expect(calls(api, `${B}/update`)).toHaveLength(1);
  });

  it('shows sleeping and cooldown chips', async () => {
    let status = deployed('paused');
    const api = vi.fn(async () => status);
    render(api);
    await drain();
    expect(host.querySelector('[data-testid="cloud-chip"]').textContent).toContain('спит');
    unmount(instance);
    host.remove();
    status = deployed('cooldown', { cooldownUntil: new Date(2030, 0, 1, 14, 5).toISOString(), retryAfterSeconds: 600 });
    render(api);
    await drain();
    expect(host.querySelector('[data-testid="cloud-chip"]').textContent).toContain('пауза до 14:05');
  });

  it('retries open while the machine wakes', async () => {
    vi.useFakeTimers();
    let n = 0;
    const api = vi.fn(async (path) => {
      if (path === `${B}/open`) {
        n += 1;
        return n === 1 ? { ok: false, state: 'waking', retryAfterSeconds: 5 } : { ok: true, state: 'running' };
      }
      return deployed('paused');
    });
    render(api);
    await drain();
    button('Открыть облако').click();
    await drain();
    expect(host.textContent).toContain('просыпается');
    vi.advanceTimersByTime(5100);
    await drain();
    expect(calls(api, `${B}/open`)).toHaveLength(2);
  });

  it('deletes only after confirmation', async () => {
    const api = vi.fn(async () => deployed());
    render(api);
    await drain();
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false);
    button('Удалить машину').click();
    await drain();
    expect(confirmSpy).toHaveBeenCalledWith('Машина и все данные ботов в облаке будут удалены. Это нельзя отменить.');
    expect(calls(api, `${B}/delete`)).toHaveLength(0);
    confirmSpy.mockReturnValue(true);
    button('Удалить машину').click();
    await drain();
    expect(calls(api, `${B}/delete`)[0][1]).toEqual({ method: 'POST', body: { confirm: true } });
  });
});
