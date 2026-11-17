import { afterEach, describe, expect, it } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import LimitRing from './LimitRing.svelte';

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
  instance = mount(LimitRing, { target: host, props });
  flushSync();
  return host.querySelector('.k-ring');
}

describe('LimitRing', () => {
  it('draws the arc by the share left and names the percent only in the label', () => {
    const ring = render({ left: 0.73, label: 'Редактор, осталось 73%' });
    expect(ring.style.getPropertyValue('--left')).toBe('0.73');
    expect(ring.getAttribute('aria-label')).toBe('Редактор, осталось 73%');
    expect(ring.textContent.trim()).toBe('');
  });

  it('turns low under 15 percent, empty at zero and dashed when the limit is unknown', () => {
    expect(render({ left: 0.1 }).classList.contains('is-low')).toBe(true);
    unmount(instance);
    host.remove();
    expect(render({ left: 0 }).classList.contains('is-empty')).toBe(true);
    unmount(instance);
    host.remove();
    expect(render({ left: null }).classList.contains('is-unknown')).toBe(true);
  });

  it('shows one badge by state', () => {
    const ring = render({ left: 0.5, badge: 'need', count: 2 });
    expect(ring.querySelector('.k-ring__need').textContent).toBe('2');
    expect(ring.querySelector('.k-ring__work')).toBeNull();
  });
});
