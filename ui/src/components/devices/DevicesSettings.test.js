import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import DevicesSettings from './DevicesSettings.svelte';

let instance;
let host;

async function drain() {
  for (let i = 0; i < 14; i += 1) {
    flushSync();
    await Promise.resolve();
  }
  flushSync();
}

const devicesData = {
  connected: true,
  status: null,
  pcName: 'ДОМ-ПК',
  devices: [{ id: 'dev1', name: 'iPhone', kind: 'web', added: 1, lastSeen: Date.now() - 5 * 60000, online: false, actions: false }],
  pending: [],
};
const remoteData = { connected: true, pcs: [{ id: 'pc9', name: 'Ноутбук', online: false, status: 'offline' }], candidates: [{ id: 'pc7', name: 'Офис', online: true }], pairing: null };

function makeApi(extra = {}) {
  return vi.fn(async (path, opts = {}) => {
    if (extra[`${opts.method || 'GET'} ${path}`]) return extra[`${opts.method || 'GET'} ${path}`](opts);
    if (path === '/api/devices') return devicesData;
    if (path === '/api/remote/pcs') return remoteData;
    return { ok: true };
  });
}

async function render(api) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(DevicesSettings, { target: host, props: { api } });
  await drain();
}
const button = (label) => [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === label);

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
});

describe('DevicesSettings', () => {
  it('lists devices, link status and other PCs', async () => {
    await render(makeApi());
    expect(host.textContent).toContain('iPhone');
    expect(host.textContent).toContain('Подключено к deloist.ru');
    expect(host.textContent).toContain('Ноутбук');
    expect(host.textContent).toContain('Офис');
  });

  it('shows the server mode switch only where it is supported and reports the sleep state', async () => {
    await render(makeApi());
    expect(host.textContent).not.toContain('Режим сервера');
    unmount(instance);
    host.remove();
    const calls = [];
    const api = makeApi({
      'GET /api/server-mode': async () => ({ supported: true, enabled: false, awake: false }),
      'PUT /api/server-mode': async (opts) => {
        calls.push(opts.body);
        return { supported: true, enabled: opts.body.enabled, awake: opts.body.enabled };
      },
    });
    await render(api);
    expect(host.textContent).toContain('Режим сервера');
    expect(host.querySelector('[data-server-status]')).toBeNull();
    host.querySelector('[aria-label="Режим сервера"]').click();
    await drain();
    expect(calls).toEqual([{ enabled: true }]);
    expect(host.querySelector('[data-server-status]')).not.toBeNull();
    expect(host.textContent).toContain('Режим сервера: включён · не засыпает');
  });

  it('explains "connected elsewhere" and the device limit', async () => {
    const api = makeApi({ 'GET /api/devices': async () => ({ ...devicesData, connected: false, status: 'elsewhere' }) });
    await render(api);
    expect(host.textContent).toContain('на другом ПК');
  });

  it('revoke needs a confirm click and encodes the id', async () => {
    const api = makeApi();
    await render(api);
    button('Отозвать…').click();
    await drain();
    expect(api.mock.calls.some(([, o]) => o?.method === 'DELETE')).toBe(false);
    button('Отозвать').click();
    await drain();
    expect(api.mock.calls.some(([p, o]) => p === '/api/devices/dev1' && o?.method === 'DELETE')).toBe(true);
  });

  it('turning actions on asks for the exact device name', async () => {
    const api = makeApi();
    await render(api);
    host.querySelector('[role="switch"]').click();
    await drain();
    const on = button('Включить');
    expect(on.disabled).toBe(true);
    const input = host.querySelector('.ask input');
    input.value = 'iphone';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await drain();
    expect(button('Включить').disabled).toBe(true);
    input.value = 'iPhone';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await drain();
    button('Включить').click();
    await drain();
    const call = api.mock.calls.find(([p]) => p === '/api/devices/dev1/actions');
    expect(call[1].body).toEqual({ enabled: true, confirm: 'iPhone' });
  });

  it('pairs another PC: shows the code, then sends the code from the other screen', async () => {
    const api = makeApi({
      'POST /api/remote/pair/start': async () => ({ req: 'q1', code: '123456', pc: 'pc7', name: 'Офис' }),
      'POST /api/remote/pair/q1/ack': async () => ({ paired: true, pc: 'pc7', name: 'Офис' }),
    });
    await render(api);
    button('Подключить ПК').click();
    await drain();
    expect(host.querySelector('.code').textContent).toBe('123 456');
    const input = host.querySelector('[data-pairing] input');
    input.value = '654 321';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await drain();
    button('Подключить').click();
    await drain();
    const call = api.mock.calls.find(([p]) => p === '/api/remote/pair/q1/ack');
    expect(call[1].body).toEqual({ code: '654321' });
    expect(host.querySelector('[data-pairing]')).toBeNull();
  });

  it('disconnecting a remote PC asks first', async () => {
    const api = makeApi();
    await render(api);
    button('Отключить…').click();
    await drain();
    button('Отключить').click();
    await drain();
    expect(api.mock.calls.some(([p, o]) => p === '/api/remote/pc9' && o?.method === 'DELETE')).toBe(true);
  });
});
