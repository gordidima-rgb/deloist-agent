import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import Select from './Select.svelte';

let instance;
let host;

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

describe('Select', () => {
  it('reports the new value on a native change event', () => {
    const onchange = vi.fn();
    host = document.createElement('div');
    document.body.append(host);
    instance = mount(Select, {
      target: host,
      props: {
        value: 'a',
        options: [
          { value: 'a', label: 'Первый' },
          { value: 'b', label: 'Второй' },
        ],
        onchange,
      },
    });
    flushSync();

    const select = host.querySelector('select');
    select.value = 'b';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    flushSync();

    expect(onchange).toHaveBeenCalledTimes(1);
    expect(select.value).toBe('b');
    expect(host.textContent).toContain('Второй');
  });
});
