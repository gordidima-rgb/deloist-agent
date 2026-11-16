import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import RoomResources from './RoomResources.svelte';

const skills = [
  { id: 'sk1', name: 'Стиль писем' },
  { id: 'sk2', name: '<b>Хак</b>' },
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
  instance = mount(RoomResources, { target: host, props: { folders: [], skills: [], allSkills: skills, ...props } });
  flushSync();
  return host;
}

const input = () => host.querySelector('input[type="text"]');
const type = (text) => {
  input().value = text;
  input().dispatchEvent(new Event('input', { bubbles: true }));
  flushSync();
};

describe('RoomResources', () => {
  it('shows the saved folders and skills with the scope hint', () => {
    render({ folders: ['D:\\Проекты\\сайт'], skills: ['sk2'] });
    expect(host.textContent).toContain('D:\\Проекты\\сайт');
    expect([...host.querySelectorAll('input:checked')].map((el) => el.value)).toEqual(['sk2']);
    expect(host.textContent).toContain('только внутри этой команды');
    // Текст навыка не становится разметкой.
    expect(host.querySelector('b')).toBeNull();
  });

  it('adds a folder with the button and with Enter, without repeats', () => {
    const onchange = vi.fn();
    render({ onchange });
    type('D:\\a');
    [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Добавить').click();
    flushSync();
    expect(onchange).toHaveBeenLastCalledWith({ folders: ['D:\\a'], skills: [] });
    type('d:\\A');
    input().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    flushSync();
    expect(host.querySelectorAll('.rr-item').length).toBe(1);
    expect(input().value).toBe('');
  });

  it('removes a folder and reports chosen skills', () => {
    const onchange = vi.fn();
    render({ folders: ['D:\\a', 'D:\\b'], onchange });
    host.querySelector('.rr-del').click();
    flushSync();
    expect(onchange).toHaveBeenLastCalledWith({ folders: ['D:\\b'], skills: [] });
    const box = host.querySelector('input[value="sk1"]');
    box.click();
    flushSync();
    expect(onchange).toHaveBeenLastCalledWith({ folders: ['D:\\b'], skills: ['sk1'] });
  });

  it('stops at the folder cap and says so', () => {
    render({ folders: ['D:\\a'], max: 1 });
    type('D:\\b');
    [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Добавить').click();
    flushSync();
    expect(host.querySelectorAll('.rr-item').length).toBe(1);
    expect(host.querySelector('[role="status"]')?.textContent).toContain('1');
  });
});
