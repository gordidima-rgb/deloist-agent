import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import Menu from './Menu.svelte';

let instance;
let host;

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

const items = [
  { id: 'a', label: 'Копировать', icon: 'copy', shortcut: 'Ctrl+C' },
  { id: 'sep', kind: 'sep' },
  { id: 'b', label: 'Закрепить', kind: 'check', checked: true },
  { id: 'c', label: 'Серый', disabled: true, hint: 'нет' },
  { id: 'd', label: 'Удалить', danger: true },
];

function render(props = {}) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(Menu, { target: host, props: { items, label: 'Действия', ...props } });
  flushSync();
}

describe('Menu', () => {
  it('draws rows, shortcut, separator and the danger item', () => {
    render();
    expect(host.querySelectorAll('.k-menu__item').length).toBe(4);
    expect(host.querySelector('.k-menu__kbd').textContent).toBe('Ctrl+C');
    expect(host.querySelector('.k-menu__sep')).toBeTruthy();
    expect(host.querySelector('.is-danger').textContent).toContain('Удалить');
    expect(host.querySelector('[role="menuitemcheckbox"]').getAttribute('aria-checked')).toBe('true');
  });

  it('picks an item, but not a disabled one', () => {
    const onpick = vi.fn();
    render({ onpick });
    const rows = host.querySelectorAll('.k-menu__item');
    rows[0].click();
    rows[2].click();
    expect(onpick).toHaveBeenCalledTimes(1);
    expect(onpick).toHaveBeenCalledWith('a');
  });

  it('walks with arrows and closes on Escape', async () => {
    const onclose = vi.fn();
    render({ onclose });
    await Promise.resolve();
    const menu = host.querySelector('.k-menu');
    const first = host.querySelector('.k-menu__item');
    expect(document.activeElement).toBe(first);
    menu.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    expect(document.activeElement.textContent).toContain('Закрепить');
    menu.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(onclose).toHaveBeenCalledWith(true);
  });

  it('places a floating menu at a point', () => {
    render({ x: 40, y: 30 });
    const menu = host.querySelector('.k-menu');
    expect(menu.classList.contains('k-menu--float')).toBe(true);
    expect(menu.classList.contains('is-placed')).toBe(true);
  });

  describe('search and groups', () => {
    const grouped = [
      { id: 'g1', kind: 'group', label: 'Входы' },
      { id: 'in', label: 'Сообщение', sub: 'старт', keywords: 'message' },
      { id: 'g2', kind: 'group', label: 'Шаги' },
      { id: 'st', label: 'Вопрос боту', sub: 'ответ' },
    ];

    it('draws group titles and the second line', () => {
      render({ items: grouped, search: 'Искать' });
      expect([...host.querySelectorAll('.k-menu__group')].map((g) => g.textContent)).toEqual(['Входы', 'Шаги']);
      expect(host.querySelector('.k-menu__two small').textContent).toBe('старт');
    });

    it('filters by label, sub and keywords, drops empty groups', () => {
      render({ items: grouped, search: 'Искать', empty: 'Пусто' });
      const input = host.querySelector('.k-menu__search input');
      input.value = 'message';
      input.dispatchEvent(new Event('input', { bubbles: true }));
      flushSync();
      expect(host.querySelectorAll('.k-menu__item').length).toBe(1);
      expect([...host.querySelectorAll('.k-menu__group')].map((g) => g.textContent)).toEqual(['Входы']);
      input.value = 'zzz';
      input.dispatchEvent(new Event('input', { bubbles: true }));
      flushSync();
      expect(host.querySelector('.k-menu__empty').textContent).toBe('Пусто');
    });

    it('Enter in the search picks the first visible item', () => {
      const onpick = vi.fn();
      render({ items: grouped, search: 'Искать', onpick });
      const input = host.querySelector('.k-menu__search input');
      input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
      expect(onpick).toHaveBeenCalledWith('in');
    });
  });
});
