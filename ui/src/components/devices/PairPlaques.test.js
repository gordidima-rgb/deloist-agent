import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import PairPlaques from './PairPlaques.svelte';

let instance;
let host;

async function drain() {
  for (let i = 0; i < 12; i += 1) {
    flushSync();
    await Promise.resolve();
  }
  flushSync();
}

async function render(api) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(PairPlaques, { target: host, props: { api } });
  await drain();
}

const emit = async (detail) => {
  window.dispatchEvent(new CustomEvent('deloist:event', { detail }));
  await drain();
};
const request = { type: 'device_pair_request', req: 'r1', name: 'iPhone <b>Димы</b>', kind: 'web', ua: 'Safari · iOS', geo: '', at: Date.now() };
const typeCode = async (value) => {
  const input = host.querySelector('.plaque__code');
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  await drain();
};
const button = (label) => [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === label);

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  vi.useRealTimers();
});

describe('PairPlaques', () => {
  it('shows nothing without a request and a plaque for each event; names stay text', async () => {
    await render(async () => ({ devices: [], pending: [] }));
    expect(host.querySelector('.plaque')).toBeNull();
    await emit(request);
    await emit(request);
    expect(host.querySelectorAll('.plaque').length).toBe(1);
    expect(host.querySelector('.plaque__title').textContent).toContain('iPhone <b>Димы</b>');
    expect(host.querySelector('.plaque b')).toBeNull();
  });

  it('allows only after six digits are typed and shows the ack code', async () => {
    const api = vi.fn(async (path, opts) => (path === '/api/devices' ? { devices: [], pending: [] } : { allowed: true, ack: '123456' }));
    await render(api);
    await emit(request);
    expect(button('Разрешить').disabled).toBe(true);
    await typeCode('12345');
    expect(button('Разрешить').disabled).toBe(true);
    await typeCode('654 321');
    expect(button('Разрешить').disabled).toBe(false);
    button('Разрешить').click();
    await drain();
    const call = api.mock.calls.find(([path]) => path === '/api/devices/pair/r1');
    expect(call[1].body).toEqual({ allow: true, code: '654321' });
    expect(host.querySelector('.plaque__ack').textContent).toBe('123 456');
    button('Готово').click();
    await drain();
    expect(host.querySelector('.plaque')).toBeNull();
  });

  it('shows the remaining tries after a wrong code and keeps the plaque', async () => {
    const api = vi.fn(async (path) => {
      if (path === '/api/devices') return { devices: [], pending: [] };
      const err = new Error('Код не совпал. Осталось попыток: 2');
      err.status = 409;
      throw err;
    });
    await render(api);
    await emit(request);
    await typeCode('111111');
    button('Разрешить').click();
    await drain();
    expect(host.querySelector('.plaque__error').textContent).toContain('Осталось попыток: 2');
    expect(host.querySelector('.plaque__code')).not.toBeNull();
  });

  it('deny sends no code and removes the plaque; a request id is encoded in the address', async () => {
    const api = vi.fn(async (path) => (path === '/api/devices' ? { devices: [], pending: [] } : { allowed: false }));
    await render(api);
    await emit({ ...request, req: 'a/b?c' });
    button('Запретить').click();
    await drain();
    const call = api.mock.calls.find(([path]) => path.startsWith('/api/devices/pair/'));
    expect(call[0]).toBe('/api/devices/pair/a%2Fb%3Fc');
    expect(call[1].body).toEqual({ allow: false });
    expect(host.querySelector('.plaque')).toBeNull();
  });

  it('drops the plaque when the request disappears from the list and when its time is up', async () => {
    let pending = [request];
    await render(async () => ({ devices: [], pending }));
    expect(host.querySelectorAll('.plaque').length).toBe(1);
    pending = [];
    await emit({ type: 'devices_changed' });
    expect(host.querySelector('.plaque')).toBeNull();

    vi.useFakeTimers();
    pending = [];
    await emit({ ...request, req: 'r2', at: Date.now() });
    expect(host.querySelectorAll('.plaque').length).toBe(1);
    vi.advanceTimersByTime(5 * 60 * 1000 + 10);
    await drain();
    expect(host.querySelector('.plaque')).toBeNull();
  });
});
