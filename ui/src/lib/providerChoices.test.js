import { beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { flushSync } from 'svelte';
import { mountComponent } from './mount.svelte.js';
import ChatModelPopover from '../components/ChatModelPopover.svelte';

const source = readFileSync('../web/providers.js', 'utf8');
const provider = (id) => ({ id, driver: id, name: id, enabled: true, status: 'connected', models: [{ id, name: id }] });
const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };

beforeEach(() => {
  document.body.innerHTML = '<div id="modal-providers"></div><div id="prov-list"></div><div id="prov-detail"></div><input id="prov-search">';
  window.DeloistUI = { mountComponent: () => ({ update() {} }) };
  window.tr = (s) => s;
  window.eval(source);
});

describe('provider choices', () => {
  it('prewarms, shares requests, renders the last snapshot during refresh, and retains it on failure', async () => {
    const first = deferred();
    const second = deferred();
    const api = vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise).mockRejectedValueOnce(new Error('offline'));
    const choices = window.DeloistProviders;
    choices.start({ api });
    const one = choices.choices();
    expect(api).toHaveBeenCalledTimes(1);
    first.resolve({ providers: [provider('old')], plan: { models: [] } });
    await one;
    const refresh = choices.choices();
    const concurrent = choices.choices();
    expect(api).toHaveBeenCalledTimes(2);
    const host = document.createElement('div');
    const anchor = document.createElement('button');
    document.body.append(host, anchor);
    const handle = mountComponent(ChatModelPopover, host, { ...choices.snapshot(), anchor, value: { driver: 'old', model: 'old' } });
    flushSync();
    expect(host.textContent).toContain('old');
    second.resolve({ providers: [provider('new')], plan: { models: [] } });
    const data = await refresh;
    expect(await concurrent).toEqual(data);
    handle.update(data); flushSync();
    expect(host.textContent).toContain('new');
    expect(await choices.choices()).toEqual(data);
    handle.destroy(); host.remove(); anchor.remove();
  });

  it('shows loading on a cold start', () => {
    const host = document.createElement('div');
    document.body.append(host);
    const handle = mountComponent(ChatModelPopover, host, { loading: true });
    flushSync();
    expect(host.querySelector('[role=status]')).not.toBeNull();
    expect(host.querySelector('.mp-item')).toBeNull();
    handle.destroy(); host.remove();
  });

  it('selects a real provider when the cold placeholder disappears', () => {
    const host = document.createElement('div');
    document.body.append(host);
    const handle = mountComponent(ChatModelPopover, host, { loading: true, settings: { driver: 'real' } });
    flushSync();
    handle.update({ loading: false, providers: [provider('real')] });
    flushSync();
    expect([...host.querySelectorAll('.mp-item')].some((row) => row.textContent.includes('real'))).toBe(true);
    handle.destroy(); host.remove();
  });
});
