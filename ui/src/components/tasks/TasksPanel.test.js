import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import TasksPanel from './TasksPanel.svelte';

// jsdom не умеет Element.animate, а Svelte-переходы окна на нём держатся.
if (!Element.prototype.animate) {
  Element.prototype.animate = () => ({ onfinish: null, oncancel: null, cancel() {}, finish() {}, addEventListener() {}, removeEventListener() {} });
}

let instance;
const now = Date.now();
const tasks = [
  { id: 'a', from: 'x', fromName: 'Альфа', to: 'y', toName: 'Бета', text: 'Сделай картинки', state: 'running', created: now - 1000, startedBy: 'human', tokens: 1200, deadline: 0, summary: '', resultFiles: [] },
  { id: 'b', from: 'x', fromName: 'Альфа', to: 'y', toName: 'Бета', text: 'Старое', state: 'done', created: now - 5000, finished: now - 2000, summary: 'Готово', resultFiles: ['C:/d/workspaces/y/out/pic.png'], tokens: 0, deadline: 0 },
];
afterEach(() => {
  if (instance) unmount(instance);
  document.body.innerHTML = '';
  instance = null;
});

describe('TasksPanel', () => {
  it('lists open tasks first and cancels only open ones', async () => {
    const oncancel = vi.fn();
    instance = mount(TasksPanel, { target: document.body, props: { open: true, tasks, oncancel } });
    flushSync();
    expect(document.querySelectorAll('.tp-row')).toHaveLength(1);
    const cancel = [...document.querySelectorAll('button')].find((b) => b.textContent.includes('Отменить поручение'));
    cancel.click();
    expect(oncancel).toHaveBeenCalledWith(expect.objectContaining({ id: 'a' }));
  });

  it('opens the week list with focus and shows result file cards', () => {
    const onopenfile = vi.fn();
    instance = mount(TasksPanel, { target: document.body, props: { open: true, tasks, focus: 'b', onopenfile } });
    flushSync();
    expect(document.querySelectorAll('.tp-row')).toHaveLength(2);
    document.querySelector('.k-filecard .k-btn').click();
    expect(onopenfile).toHaveBeenCalledWith('C:/d/workspaces/y/out/pic.png', expect.objectContaining({ id: 'b' }));
    expect(document.querySelector('.k-filecard b').textContent).toBe('pic.png');
  });
});
