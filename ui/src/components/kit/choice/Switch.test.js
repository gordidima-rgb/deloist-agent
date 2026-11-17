import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import Switch from './Switch.svelte';

let instance;
let host;

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

describe('Switch', () => {
  it('toggles with Space and reports the new state', () => {
    const onchange = vi.fn();
    host = document.createElement('div');
    document.body.append(host);
    instance = mount(Switch, { target: host, props: { label: 'Озвучивать ответы', onchange } });
    flushSync();

    const button = host.querySelector('[role="switch"]');
    expect(button.getAttribute('aria-checked')).toBe('false');

    button.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true }));
    flushSync();

    expect(button.getAttribute('aria-checked')).toBe('true');
    expect(onchange).toHaveBeenCalledWith(true);
  });
});
