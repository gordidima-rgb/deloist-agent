import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import ScreenControl from './ScreenControl.svelte';

let host, instance;
function render(props = {}) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(ScreenControl, { target: host, props });
  flushSync();
  return host;
}
afterEach(() => { if (instance) unmount(instance); host?.remove(); instance = host = null; });

describe('ScreenControl', () => {
  it('offers to take control and calls the callback', () => {
    const ontoggle = vi.fn();
    const el = render({ ontoggle });
    expect(el.querySelector('button').textContent).toContain('Взять управление');
    expect(el.querySelector('.screen-control-status')).toBeNull();
    el.querySelector('button').click();
    expect(ontoggle).toHaveBeenCalledOnce();
  });
  it('shows human control and offers to return it', () => {
    const el = render({ held: true });
    expect(el.querySelector('.screen-control-status').textContent).toBe('Управление у тебя');
    expect(el.querySelector('button').textContent).toContain('Вернуть боту');
  });
  it('disables the action while busy and announces errors', () => {
    const ontoggle = vi.fn();
    const el = render({ busy: true, error: 'Ошибка', ontoggle });
    expect(el.querySelector('button').disabled).toBe(true);
    expect(el.querySelector('button').textContent).toContain('Подождите');
    expect(el.querySelector('[role="alert"]').textContent).toBe('Ошибка');
    el.querySelector('button').click();
    expect(ontoggle).not.toHaveBeenCalled();
  });
});
