import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import ListRowHarness from './ListRowHarness.test.svelte';

let instance;
let host;

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

function render(props) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(ListRowHarness, { target: host, props });
  flushSync();
}

describe('ListRow', () => {
  it('chevron row toggles its body', () => {
    render({ variant: 'chevron' });
    const btn = host.querySelector('button.k-lrow__main');
    expect(btn.getAttribute('aria-expanded')).toBe('false');
    expect(host.querySelector('.k-lrow__acc').hasAttribute('inert')).toBe(true);
    btn.click();
    flushSync();
    expect(btn.getAttribute('aria-expanded')).toBe('true');
    expect(host.querySelector('.k-lrow.is-open')).toBeTruthy();
    expect(host.querySelector('.k-lrow__acc').hasAttribute('inert')).toBe(false);
  });

  it('action row is not pressable as a whole, button fires onaction', () => {
    const onaction = vi.fn();
    render({ variant: 'action', actionLabel: 'Войти', onaction });
    expect(host.querySelector('button.k-lrow__main')).toBeNull();
    host.querySelector('.k-btn').click();
    expect(onaction).toHaveBeenCalledTimes(1);
  });

  it('switch row flips and reports', () => {
    const onchange = vi.fn();
    render({ variant: 'switch', onchange });
    host.querySelector('[role="switch"]').click();
    flushSync();
    expect(onchange).toHaveBeenCalledWith(true);
  });
});
