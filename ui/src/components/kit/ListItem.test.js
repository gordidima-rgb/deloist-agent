import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import ListItem from './ListItem.svelte';

let instance;
let host;

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

function render(props) {
  host = document.createElement('ul');
  document.body.append(host);
  instance = mount(ListItem, { target: host, props: { title: 'Мира', subtitle: 'Ищет источники', ...props } });
  flushSync();
}

describe('ListItem', () => {
  it('calls onclick from the row button', () => {
    const onclick = vi.fn();
    render({ onclick });
    host.querySelector('.k-list-item__main').click();
    flushSync();
    expect(onclick).toHaveBeenCalledTimes(1);
  });

  it('marks the active row for assistive technology and styling', () => {
    render({ active: true });
    const row = host.querySelector('.k-list-item');
    expect(row.classList.contains('is-active')).toBe(true);
    expect(row.getAttribute('aria-current')).toBe('true');
  });
});
