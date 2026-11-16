import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, tick, unmount } from 'svelte';
import Tabs from './Tabs.svelte';

let instance;
let host;

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

const items = [
  { id: 'chat', label: 'Чат' },
  { id: 'files', label: 'Файлы' },
  { id: 'reports', label: 'Отчёты' },
];

describe('Tabs', () => {
  it('ArrowRight selects the next tab and reports it', async () => {
    const onchange = vi.fn();
    host = document.createElement('div');
    document.body.append(host);
    instance = mount(Tabs, { target: host, props: { items, value: 'chat', label: 'Разделы', onchange } });
    flushSync();

    const tabs = [...host.querySelectorAll('.k-tab')];
    expect(tabs[0].getAttribute('aria-selected')).toBe('true');

    tabs[0].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }));
    flushSync();
    await tick();
    flushSync();

    expect(onchange).toHaveBeenCalledWith('files');
    expect(host.querySelectorAll('.k-tab')[1].getAttribute('aria-selected')).toBe('true');
    expect(host.querySelectorAll('.k-tab')[0].getAttribute('aria-selected')).toBe('false');
    expect(document.activeElement).toBe(host.querySelectorAll('.k-tab')[1]);
  });

  it('End selects the last tab', () => {
    host = document.createElement('div');
    document.body.append(host);
    instance = mount(Tabs, { target: host, props: { items, value: 'chat' } });
    flushSync();

    host.querySelector('.k-tab').dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true, cancelable: true }));
    flushSync();

    expect(host.querySelectorAll('.k-tab')[2].getAttribute('aria-selected')).toBe('true');
  });
});
