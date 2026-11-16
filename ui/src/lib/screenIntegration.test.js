import { describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import { mountComponent } from './mount.svelte.js';
import * as screenLayout from './screenLayout.js';
import ScreenControl from '../components/ScreenControl.svelte';
import { readFileSync } from 'node:fs';
const screenSource = readFileSync('../web/screen.js', 'utf8');

describe('screen control requests', () => {
  it('ignores an old bot response and clears an error when switching bots', async () => {
    const requests = [];
    vi.stubGlobal('EventSource', class { close() {} });
    vi.stubGlobal('fetch', vi.fn((url) => new Promise((resolve) => requests.push({ url, resolve }))));
    window.DeloistUI = { screenLayout, mountScreenControl: (target, props) => mountComponent(ScreenControl, target, props) };
    window.eval(screenSource);
    const tick = async () => { await new Promise((resolve) => setTimeout(resolve, 0)); flushSync(); };
    window.DeloistScreen.open('first'); flushSync();
    document.querySelector('.screen-control button').click(); flushSync();
    expect(requests[0].url).toContain('/first/');
    expect(document.querySelector('.screen-control button').disabled).toBe(true);
    window.DeloistScreen.open('second'); flushSync();
    expect(document.querySelector('.screen-control button').disabled).toBe(false);
    requests[0].resolve({ ok: true, json: async () => ({}) }); await tick();
    expect(document.querySelector('.screen-control-status')).toBeNull();
    document.querySelector('.screen-control button').click(); flushSync();
    expect(requests[1].url).toContain('/second/');
    requests[1].resolve({ ok: false, json: async () => ({ error: 'Failed' }) }); await tick();
    expect(document.querySelector('[role="alert"]').textContent).toBe('Failed');
    window.DeloistScreen.open('third'); flushSync();
    expect(document.querySelector('[role="alert"]')).toBeNull();
    expect(document.querySelector('[data-note]').textContent).toBe('');
    window.DeloistScreen.close();
    vi.unstubAllGlobals();
  });
});
