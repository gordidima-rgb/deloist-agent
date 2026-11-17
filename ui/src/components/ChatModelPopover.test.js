import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import { mountComponent } from '../lib/mount.svelte.js';
import ChatModelPopover from './ChatModelPopover.svelte';

const plan = { driver: 'deloist', defaultModel: 'deepseek-flash', models: [{ id: 'deepseek-flash', name: 'DeepSeek Flash' }, { id: 'gpt-6.1-terra', name: 'GPT-6.1 Terra' }] };
const codex = {
  id: 'codex', kind: 'acp', name: 'ChatGPT', status: 'connected', enabled: true, driver: 'acp:codex',
  models: [{ id: 'gpt-5', name: 'GPT-5' }],
  efforts: [{ id: 'low', name: 'Низкое' }, { id: 'xhigh', name: 'Очень высокое' }],
  effortDefault: 'low',
  options: [{ id: 'speed', name: 'Скорость', values: [{ id: 'normal', name: 'Обычная' }, { id: 'fast', name: 'Быстрая' }], default: 'normal' }],
};
const bare = { id: 'other', kind: 'acp', name: 'Other', status: 'connected', enabled: true, driver: 'acp:other', models: [{ id: 'm1', name: 'M1' }] };

let handle;
let host;
let anchor;
function mount(props) {
  anchor = document.createElement('button');
  document.body.append(anchor);
  host = document.createElement('div');
  document.body.append(host);
  handle = mountComponent(ChatModelPopover, host, { providers: [codex, bare], plan, settings: { driver: 'deloist', model: 'deepseek-flash' }, value: { driver: '', model: '' }, effort: '', anchor, defaultLabel: 'Как в настройках: DeepSeek Flash', ...props });
  flushSync();
}
afterEach(() => {
  handle?.destroy();
  host?.remove();
  anchor?.remove();
});

const row = (text) => [...host.querySelectorAll('.mp-item')].find((b) => b.textContent.includes(text));
// Select кита: видимая кнопка несёт id, значение держит скрытый нативный select рядом.
const nativeOf = (selector) => host.querySelector(selector)?.closest('.k-select')?.querySelector('select');
const change = (el, value) => {
  el.value = value;
  el.dispatchEvent(new Event('change', { bubbles: true }));
  flushSync();
};

describe('ChatModelPopover', () => {
  it('opens at once without the trigger row and shows the default row first', () => {
    mount();
    expect(host.querySelector('.mp-pop')).not.toBeNull();
    expect(host.querySelector('.mp-row').classList.contains('bare')).toBe(true);
    expect(host.querySelector('.mp-item').textContent).toContain('Как в настройках: DeepSeek Flash');
  });

  it('opens on the group of the effective driver while the bot follows settings', () => {
    mount({ providers: [codex, bare], settings: { driver: 'acp:other', model: 'm1' } });
    expect(host.querySelector('.mp-title').textContent).toBe('Other');
    expect(host.querySelector('.mp-item.sel').textContent).toContain('Как в настройках');
  });

  it('selecting a model passes { driver, model }', () => {
    const onselect = vi.fn();
    mount({ onselect });
    row('GPT-6.1 Terra').click();
    expect(onselect).toHaveBeenCalledWith({ driver: 'deloist', model: 'gpt-6.1-terra' });
  });

  it('«Как в настройках» clears the choice', () => {
    const onselect = vi.fn();
    mount({ onselect, value: { driver: 'deloist', model: 'gpt-6.1-terra' } });
    row('Как в настройках').click();
    expect(onselect).toHaveBeenCalledWith({ driver: '', model: '' });
  });

  it('effort uses the provider list and saves; auto clears', () => {
    const oneffort = vi.fn();
    mount({ oneffort, value: { driver: 'acp:codex', model: 'gpt-5' }, effort: 'low' });
    const select = nativeOf('#cmp-effort');
    expect([...select.options].map((o) => o.textContent)).toEqual(['Авто (Низкое)', 'Низкое', 'Очень высокое']);
    expect(select.value).toBe('low');
    change(select, 'xhigh');
    expect(oneffort).toHaveBeenLastCalledWith('xhigh');
    change(select, '');
    expect(oneffort).toHaveBeenLastCalledWith('');
  });

  it('extra agent options render a select and save', () => {
    const onoption = vi.fn();
    mount({ onoption, value: { driver: 'acp:codex', model: 'gpt-5' }, agentOptions: { speed: 'fast' } });
    const select = nativeOf('#cmp-opt-speed');
    expect(select.value).toBe('fast');
    change(select, '');
    expect(onoption).toHaveBeenLastCalledWith('speed', '');
  });

  it('falls back to low/medium/high for API drivers and hides effort for ACP without efforts', () => {
    mount();
    expect([...nativeOf('#cmp-effort').options].map((o) => o.value)).toEqual(['', 'low', 'medium', 'high']);
    handle.update({ value: { driver: 'acp:other', model: 'm1' } });
    flushSync();
    expect(host.querySelector('#cmp-effort')).toBeNull();
  });

  it('Esc closes, «Все настройки модели…» and connect work', () => {
    const onclose = vi.fn();
    const onmore = vi.fn();
    const onconnect = vi.fn();
    mount({ onclose, onmore, onconnect });
    host.querySelector('.cmp-more').click();
    expect(onmore).toHaveBeenCalled();
    host.querySelector('.mp-add').click();
    expect(onconnect).toHaveBeenCalled();
  });

  it('Escape calls onclose with focus back', () => {
    const onclose = vi.fn();
    mount({ onclose });
    host.querySelector('.mp').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    flushSync();
    expect(onclose).toHaveBeenCalledWith(true);
    expect(host.querySelector('.mp-pop')).toBeNull();
  });
});
