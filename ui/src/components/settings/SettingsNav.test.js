import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import SettingsNav, { select } from './SettingsNav.svelte';

const sections = [
  { id: 'model', label: 'Модель' },
  { id: 'look', label: 'Внешний вид' },
  { id: 'browser', label: 'Браузер' },
  { id: 'updates', label: 'Обновления' },
];

let instance;
let host;

beforeEach(() => {
  select('model');
});

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

function render(props = {}) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(SettingsNav, { target: host, props: { sections, ...props } });
  flushSync();
}

function press(target, key) {
  target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
  flushSync();
}

describe('SettingsNav', () => {
  it('renders a kit list and marks the selected section', () => {
    render();
    const rows = [...host.querySelectorAll('.k-list-item')];
    expect(rows).toHaveLength(sections.length);
    expect(rows[0].classList.contains('is-active')).toBe(true);
    expect(rows[0].getAttribute('aria-current')).toBe('true');
    expect(rows[0].querySelector('.k-list-item__title').textContent).toBe('Модель');
  });

  it('moves focus with the arrow keys and selects with Enter', () => {
    const onselect = vi.fn();
    render({ onselect });
    const buttons = [...host.querySelectorAll('.k-list-item__main')];

    buttons[0].focus();
    press(buttons[0], 'ArrowDown');
    expect(document.activeElement).toBe(buttons[1]);
    expect(onselect).not.toHaveBeenCalled();
    expect(host.querySelectorAll('.k-list-item')[0].classList.contains('is-active')).toBe(true);

    press(buttons[1], 'Enter');
    expect(onselect).toHaveBeenCalledWith('look');
    expect(host.querySelectorAll('.k-list-item')[1].classList.contains('is-active')).toBe(true);
    expect(host.querySelectorAll('.k-list-item')[0].classList.contains('is-active')).toBe(false);
  });

  it('selects the previous section from the first row and wraps around', () => {
    render();
    const buttons = [...host.querySelectorAll('.k-list-item__main')];
    buttons[0].focus();
    press(buttons[0], 'ArrowUp');
    expect(document.activeElement).toBe(buttons[buttons.length - 1]);
  });

  it('select(sectionId) changes the active section from outside', () => {
    render();
    select('browser');
    flushSync();
    const rows = [...host.querySelectorAll('.k-list-item')];
    expect(rows[2].classList.contains('is-active')).toBe(true);
    expect(rows[0].classList.contains('is-active')).toBe(false);
  });

  it('remembers the section between mounts', () => {
    render();
    select('updates');
    flushSync();
    unmount(instance);
    instance = null;

    render();
    const rows = [...host.querySelectorAll('.k-list-item')];
    expect(rows[3].classList.contains('is-active')).toBe(true);
  });
});
