import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import ProviderList from './ProviderList.svelte';

const row = (id, name, extra = {}) => ({ id, name, badge: '', tone: 'neutral', svg: '', img: '', letter: name.charAt(0), ...extra });

const groups = [
  { key: 'local', title: 'Найдено', action: { name: 'scan', label: 'Сканировать' }, note: 'Пока ничего', rows: [] },
  {
    key: 'other',
    title: 'Другие',
    rows: [
      row('chatgpt', 'ChatGPT', { badge: 'Не загружено', tone: 'attention', svg: '<svg data-logo="x"></svg>' }),
      row('gemini', 'Gemini', { badge: 'Подключено', tone: 'brand' }),
      row('gemini', 'Gemini', { badge: 'Не подключено' }),
      row('__add', 'Добавить своё', { letter: '+', plus: true }),
    ],
  },
];

let instance;
let host;

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

function render(props = {}) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(ProviderList, { target: host, props: { groups, selected: '', ...props } });
  flushSync();
  return host;
}

describe('ProviderList', () => {
  it('draws groups, rows, badges and tolerates the same id twice', () => {
    render();
    expect(host.querySelectorAll('.prov-group')).toHaveLength(2);
    expect(host.querySelector('.prov-note').textContent).toBe('Пока ничего');
    expect(host.querySelectorAll('.k-list-item')).toHaveLength(4);
    expect(host.querySelector('.k-badge--attention').textContent).toBe('Не загружено');
    expect(host.querySelector('.k-badge--brand').textContent).toBe('Подключено');
  });

  it('shows the logo svg, or the first letter when there is none', () => {
    render();
    const items = [...host.querySelectorAll('.k-list-item')];
    expect(items[0].querySelector('.prov-logo svg[data-logo]')).not.toBeNull();
    expect(items[1].querySelector('.prov-logo').textContent.trim()).toBe('G');
    expect(items[3].querySelector('.prov-logo--plus').textContent.trim()).toBe('+');
  });

  it('marks the selected row and reports clicks', () => {
    const onselect = vi.fn();
    render({ selected: 'chatgpt', onselect });
    const items = [...host.querySelectorAll('.k-list-item')];
    expect(items[0].classList.contains('is-active')).toBe(true);
    expect(items[1].classList.contains('is-active')).toBe(false);
    items[1].querySelector('.k-list-item__main').click();
    expect(onselect).toHaveBeenCalledWith('gemini');
  });

  it('reports the group action by name', () => {
    const onaction = vi.fn();
    render({ onaction });
    host.querySelector('.prov-group__action').click();
    expect(onaction).toHaveBeenCalledWith('scan');
  });

  it('replaces a missing image with the first letter', () => {
    render({ groups: [{ key: 'a', rows: [row('x', 'Xtra', { img: '/nope.svg' })] }] });
    const img = host.querySelector('.prov-logo img');
    expect(img).not.toBeNull();
    img.dispatchEvent(new Event('error'));
    expect(host.querySelector('.prov-logo img')).toBeNull();
    expect(host.querySelector('.prov-logo').textContent.trim()).toBe('X');
  });
});
