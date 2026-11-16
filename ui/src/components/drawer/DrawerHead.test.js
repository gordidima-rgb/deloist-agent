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
  it('shows the five tabs and marks the current one', () => {
    const host = mount({ value: 'memory' });
    const tabs = [...host.querySelectorAll('.k-tab')];
    expect(tabs.map((t) => t.dataset.id)).toEqual(['profile', 'memory', 'files', 'routines', 'goals']);
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

  it('tabs carry an icon and keep their name for screen readers', () => {
    const host = mount({ value: 'profile' });
    for (const tab of host.querySelectorAll('.k-tab')) {
      expect(tab.querySelector('svg.k-tab__icon')).not.toBeNull();
      expect(tab.getAttribute('aria-label')).toBe(tab.querySelector('.k-tab__label').textContent);
    }
  });

  it('expand button reports the next state and follows update()', async () => {
    const onfull = vi.fn();
    const host = mount({ onfull });
    const btn = host.querySelector('#btn-drawer-full');
    expect(btn.getAttribute('aria-pressed')).toBe('false');
    btn.click();
    expect(onfull).toHaveBeenCalledWith(true);
    handle.update({ full: true });
    await Promise.resolve();
    expect(btn.getAttribute('aria-pressed')).toBe('true');
    btn.click();
    expect(onfull).toHaveBeenLastCalledWith(false);
  });

  it('rail shows only section icons: a click picks the section, the arrow expands', () => {
    const onchange = vi.fn();
    const onexpand = vi.fn();
    const host = mount({ value: 'files', rail: true, onchange, onexpand });
    expect(host.querySelector('.k-tab')).toBeNull();
    expect(host.querySelector('#btn-drawer-full')).toBeNull();
    const icons = [...host.querySelectorAll('.drawer-rail-icons [data-id]')];
    expect(icons.map((b) => b.dataset.id)).toEqual(['profile', 'memory', 'files', 'routines', 'goals']);
    expect(host.querySelector('[data-id="files"]').classList.contains('is-current')).toBe(true);
    host.querySelector('[data-id="routines"]').click();
    expect(onchange).toHaveBeenCalledWith('routines');
    host.querySelector('#btn-drawer-expand').click();
    expect(onexpand).toHaveBeenCalledTimes(1);
  });

  it('close button calls onclose', () => {
    const onclose = vi.fn();
    const host = mount({ onclose });
    host.querySelector('#btn-drawer-close').click();
    expect(onclose).toHaveBeenCalledTimes(1);
  });
});
