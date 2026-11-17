import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import InstaboxCard from './InstaboxCard.svelte';
import InstaboxRing from './InstaboxRing.svelte';
import { cardVisible, ringState, instaboxFeed, DISMISS_KEY } from '../../lib/instabox.js';

const none = { signedIn: false, machine: null, job: null };
const withState = (state, extra = {}) => ({ signedIn: true, machine: { state, ...extra }, job: null });

let host;
let instance;
afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = host = null;
  localStorage.clear();
});

async function render(Comp, status, onopen = () => {}) {
  host = document.createElement('div');
  document.body.append(host);
  const api = vi.fn(async () => {
    if (status instanceof Error) throw status;
    return status;
  });
  instance = mount(Comp, { target: host, props: { api, onopen } });
  for (let i = 0; i < 6; i += 1) { await Promise.resolve(); flushSync(); }
  return api;
}

describe('ringState', () => {
  it('maps machine states', () => {
    expect(ringState(none)).toEqual({ connected: false, kind: 'none' });
    expect(ringState(withState('missing')).connected).toBe(false);
    expect(ringState(withState('running')).kind).toBe('running');
    expect(ringState(withState('paused')).kind).toBe('sleeping');
    expect(ringState(withState('unreachable')).kind).toBe('offline');
    const c = ringState(withState('cooldown', { cooldownUntil: '2026-11-16T15:30:00Z' }));
    expect(c.kind).toBe('cooldown');
    expect(c.until.toISOString()).toBe('2026-11-16T15:30:00.000Z');
    expect(ringState({ ...withState('running'), job: { state: 'running' } }).kind).toBe('busy');
  });
});

describe('cardVisible', () => {
  it('shows only when loaded, not connected, not dismissed', () => {
    expect(cardVisible({ loaded: true, status: none, dismissed: false })).toBe(true);
    expect(cardVisible({ loaded: false, status: null, dismissed: false })).toBe(false);
    expect(cardVisible({ loaded: true, status: none, dismissed: true })).toBe(false);
    expect(cardVisible({ loaded: true, status: withState('paused'), dismissed: false })).toBe(false);
  });
});

describe('InstaboxCard', () => {
  it('opens the cloud screen and can be hidden for good', async () => {
    const onopen = vi.fn();
    await render(InstaboxCard, none, onopen);
    expect(host.textContent).toContain('Бесплатный сервер');
    host.querySelector('.ibx-btn').click();
    expect(onopen).toHaveBeenCalled();
    host.querySelector('.ibx-x').click();
    flushSync();
    expect(host.querySelector('.ibx-card')).toBeNull();
    expect(localStorage.getItem(DISMISS_KEY)).toBe('1');
  });
  it('stays hidden when connected or the status call fails', async () => {
    await render(InstaboxCard, withState('running'));
    expect(host.querySelector('.ibx-card')).toBeNull();
    unmount(instance); host.remove();
    await render(InstaboxCard, new Error('x'));
    expect(host.querySelector('.ibx-card')).toBeNull();
  });
});

describe('InstaboxRing', () => {
  it('dashed before connecting, status dot after', async () => {
    await render(InstaboxRing, none);
    expect(host.querySelector('.ibx-ring.ibx-off').title).toBe('Подключить бесплатный сервер');
    expect(host.querySelector('.ibx-dot')).toBeNull();
  });
  it('shows the state in the tooltip', async () => {
    await render(InstaboxRing, withState('paused'));
    const b = host.querySelector('.ibx-ring');
    expect(b.dataset.ibx).toBe('sleeping');
    expect(b.title).toContain('спит — проснётся от сообщения');
    expect(host.querySelector('.ibx-dot')).not.toBeNull();
  });
});

describe('feed', () => {
  it('shares one request and skips while the window is hidden', async () => {
    const api = vi.fn(async () => none);
    const doc = Object.assign(new EventTarget(), { hidden: true });
    const feed = instaboxFeed(api, doc);
    const off1 = feed.subscribe(() => {});
    const off2 = feed.subscribe(() => {});
    await Promise.resolve();
    expect(api).not.toHaveBeenCalled();
    doc.hidden = false;
    doc.dispatchEvent(new Event('visibilitychange'));
    await Promise.resolve(); await Promise.resolve();
    expect(api).toHaveBeenCalledTimes(1);
    doc.dispatchEvent(new Event('visibilitychange'));
    await Promise.resolve();
    expect(api).toHaveBeenCalledTimes(1);
    off1(); off2();
  });
});
