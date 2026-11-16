import { afterEach, describe, expect, it } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import RoomMembers from './RoomMembers.svelte';

const bots = [
  { id: 'a1', name: 'Скаут', role: 'ищет источники' },
  { id: 'b2', name: 'Редактор', role: '' },
  { id: 'c3', name: '<b>Хак</b>', role: 'x' },
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
  instance = mount(RoomMembers, { target: host, props: { bots, selected: [], empty: 'Пусто', ...props } });
  flushSync();
  return host;
}

const checked = () => [...host.querySelectorAll('input:checked')].map((el) => el.value);

describe('RoomMembers', () => {
  it('draws one real checkbox per bot with the bot id as value', () => {
    render();
    const inputs = [...host.querySelectorAll('input[type="checkbox"]')];
    expect(inputs.map((el) => el.value)).toEqual(['a1', 'b2', 'c3']);
    expect(host.querySelectorAll('.k-check')).toHaveLength(3);
  });

  it('checks the members passed in, readable through input:checked', () => {
    render({ selected: ['b2', 'c3'] });
    expect(checked()).toEqual(['b2', 'c3']);
  });

  it('labels with name and role, and escapes the name', () => {
    render();
    const labels = [...host.querySelectorAll('.k-check__label')].map((el) => el.textContent);
    expect(labels[0]).toBe('Скаут · ищет источники');
    expect(labels[1]).toBe('Редактор');
    expect(host.querySelector('.rm-list b')).toBeNull();
  });

  it('ticking a box changes the checked list and change bubbles to the container', () => {
    render();
    let fired = 0;
    host.addEventListener('change', () => fired++);
    const first = host.querySelector('input[value="a1"]');
    first.click();
    flushSync();
    expect(checked()).toEqual(['a1']);
    expect(fired).toBe(1);
  });

  it('shows the empty text when there are no bots', () => {
    render({ bots: [] });
    expect(host.querySelector('.rm-empty').textContent).toBe('Пусто');
    expect(host.querySelectorAll('input')).toHaveLength(0);
  });
});
