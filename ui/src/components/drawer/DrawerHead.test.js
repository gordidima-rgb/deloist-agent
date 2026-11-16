import { describe, it, expect, vi, afterEach } from 'vitest';
import { mountComponent } from '../../lib/mount.svelte.js';
import DrawerHead from './DrawerHead.svelte';

let handle;
afterEach(() => {
  handle?.destroy();
  document.body.innerHTML = '';
});

function mount(props) {
  const host = document.createElement('div');
  document.body.append(host);
  handle = mountComponent(DrawerHead, host, props);
  return host;
}

describe('DrawerHead', () => {
  it('shows the four tabs and marks the current one', () => {
    const host = mount({ value: 'memory' });
    const tabs = [...host.querySelectorAll('.k-tab')];
    expect(tabs.map((t) => t.dataset.id)).toEqual(['profile', 'memory', 'files', 'goals']);
    expect(tabs.find((t) => t.getAttribute('aria-selected') === 'true').dataset.id).toBe('memory');
  });

  it('reports a clicked tab and follows update() from outside', async () => {
    const onchange = vi.fn();
    const host = mount({ value: 'profile', onchange });
    host.querySelector('[data-id="files"]').click();
    expect(onchange).toHaveBeenCalledWith('files');
    handle.update({ value: 'goals' });
    await Promise.resolve();
    expect(host.querySelector('.k-tab[aria-selected="true"]').dataset.id).toBe('goals');
  });

  it('close button calls onclose', () => {
    const onclose = vi.fn();
    const host = mount({ onclose });
    host.querySelector('#btn-drawer-close').click();
    expect(onclose).toHaveBeenCalledTimes(1);
  });
});
