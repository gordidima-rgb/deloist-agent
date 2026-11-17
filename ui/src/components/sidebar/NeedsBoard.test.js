import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import NeedsBoard from './NeedsBoard.svelte';

let instance;
let host;
function render(props) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(NeedsBoard, { target: host, props });
  flushSync();
}
afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
});

describe('NeedsBoard', () => {
  it('hides empty sections and shows counts', () => {
    render({
      sections: [
        { key: 'needs', title: 'Нужен ты', items: [] },
        { key: 'waiting', title: 'Ждёт', items: [{ id: 'a', where: 'Альфа', title: 'привет', tag: 'ждёт конца хода' }, { id: 'b', where: 'Бета' }] },
      ],
    });
    expect(host.querySelectorAll('.nb-section')).toHaveLength(1);
    expect(host.querySelector('.nb-count').textContent).toBe('2');
    expect(host.querySelector('.nb-tag').textContent).toBe('ждёт конца хода');
  });

  it('reports the section and the item on click', () => {
    const onpick = vi.fn();
    render({ sections: [{ key: 'unread', title: 'Непрочитанные', items: [{ id: 'x', where: 'Гамма' }] }], onpick });
    host.querySelector('.nb-item .k-lrow__t--btn').click();
    expect(onpick).toHaveBeenCalledWith('unread', { id: 'x', where: 'Гамма' });
  });
});
