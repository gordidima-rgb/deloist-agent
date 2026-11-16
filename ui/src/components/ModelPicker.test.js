import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import { mountComponent } from '../lib/mount.svelte.js';
import ModelPicker from './ModelPicker.svelte';

const plan = { driver: 'deloist', defaultModel: 'deepseek-flash', models: [{ id: 'deepseek-flash', name: 'DeepSeek Flash' }, { id: 'gpt-6.1-terra', name: 'GPT-6.1 Terra' }] };
const providers = [
  { id: 'codex', kind: 'acp', name: 'ChatGPT', installed: '1', status: 'connected', enabled: true, driver: 'acp:codex', models: [{ id: 'gpt-5', name: 'GPT-5' }, { id: 'gpt-5-mini', name: 'GPT-5 mini' }], hiddenModels: ['gpt-5-mini'] },
];

let handle;
let host;
function mount(props) {
  host = document.createElement('div');
  document.body.append(host);
  handle = mountComponent(ModelPicker, host, { providers, plan, settings: {}, value: { driver: '', model: '' }, ...props });
  flushSync();
}
afterEach(() => {
  handle?.destroy();
  host?.remove();
});

const items = () =>
  [...host.querySelectorAll('.mp-item')].map((b) => {
    const copy = b.cloneNode(true);
    copy.querySelectorAll('.mp-logo').forEach((l) => l.remove());
    return copy.textContent.replace(/\s+/g, ' ').trim();
  });
const rail = (name) => {
  host.querySelector('.mp-rail-btn[aria-label="' + name + '"]').click();
  flushSync();
};
const open = () => {
  host.querySelector('.mp-row').click();
  flushSync();
};

describe('ModelPicker', () => {
  it('shows «как в настройках» by default and lists models after a click', () => {
    mount();
    expect(host.querySelector('.mp-row').textContent).toContain('как в настройках');
    expect(host.querySelector('.mp-pop')).toBeNull();
    open();
    expect(items()).toContain('DeepSeek Flash Подписка');
    rail('ChatGPT');
    const list = items();
    expect(list).toContain('GPT-5');
    expect(list.join('|')).not.toContain('GPT-5 mini');
  });

  it('search narrows the list', () => {
    mount();
    open();
    const input = host.querySelector('.mp-search input');
    input.value = 'terra';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
    expect(items()).toEqual(['GPT-6.1 Terra Подписка']);
  });

  it('picking a model reports driver + model and closes the popover', () => {
    const onselect = vi.fn();
    mount({ onselect });
    open();
    rail('ChatGPT');
    [...host.querySelectorAll('.mp-item')].find((b) => b.textContent.includes('GPT-5')).click();
    flushSync();
    expect(onselect).toHaveBeenCalledWith({ driver: 'acp:codex', model: 'gpt-5' });
    expect(host.querySelector('.mp-pop')).toBeNull();
  });

  it('«как в настройках» row clears both fields; hidden when the default makes no sense', () => {
    const onselect = vi.fn();
    mount({ onselect, value: { driver: 'acp:codex', model: 'gpt-5' } });
    expect(host.querySelector('.mp-row').textContent).toContain('ChatGPT/GPT-5');
    open();
    host.querySelector('.mp-item').click();
    expect(onselect).toHaveBeenCalledWith({ driver: '', model: '' });
    handle.update({ allowDefault: false });
    open();
    expect(items().some((t) => t === 'как в настройках')).toBe(false);
  });

  it('checkmark sits on the current model; plus opens the add-provider flow', () => {
    const onconnect = vi.fn();
    mount({ onconnect, value: { driver: 'deloist', model: 'gpt-6.1-terra' } });
    open();
    const checked = [...host.querySelectorAll('.mp-item[aria-checked="true"]')];
    expect(checked).toHaveLength(1);
    expect(checked[0].textContent).toContain('GPT-6.1 Terra');
    host.querySelector('.mp-add').click();
    expect(onconnect).toHaveBeenCalled();
  });
});

describe('ModelPicker look', () => {
  const many = {
    id: 'big',
    kind: 'key',
    name: 'BigCloud',
    status: 'connected',
    enabled: true,
    driver: 'openai:big',
    models: Array.from({ length: 11 }, (_, i) => ({ id: `m${i}`, name: `Model ${i}` })),
  };
  const search = (text) => {
    const input = host.querySelector('.mp-search input');
    input.value = text;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
  };

  it('renders an svg logo for a known provider, in the rail and in the closed state', () => {
    mount({ value: { driver: 'deloist', model: 'gpt-6.1-terra' } });
    expect(host.querySelector('.mp-row .mp-logo svg')).not.toBeNull();
    open();
    expect(host.querySelector('.mp-rail-btn[aria-label="ChatGPT"] svg')).not.toBeNull();
    expect(host.querySelector('.mp-item .mp-logo svg')).not.toBeNull();
  });

  it('rail switches provider: header shows its name and count, the dot slides to it', () => {
    mount();
    open();
    const dot = () => host.querySelector('.mp-rail-dot').style.transform;
    expect(host.querySelector('.mp-title').textContent).toBe('Подписка Deloist');
    expect(dot()).toBe('translateY(0px)');
    rail('ChatGPT');
    expect(host.querySelector('.mp-title').textContent).toBe('ChatGPT');
    expect(host.querySelector('.mp-count').textContent).toBe('1');
    expect(dot()).toBe('translateY(36px)');
    expect(host.querySelector('.mp-rail-btn[aria-label="ChatGPT"]').getAttribute('aria-pressed')).toBe('true');
  });

  it('search stays in the active provider; with no match there it goes global, grouped by provider', () => {
    mount();
    open();
    rail('ChatGPT');
    search('gpt-5');
    expect(items()).toEqual(['GPT-5']);
    expect(host.querySelector('.mp-group')).toBeNull();
    search('terra');
    expect(items()).toEqual(['GPT-6.1 Terra Подписка']);
    expect(host.querySelector('.mp-group').textContent).toContain('Подписка Deloist');
    expect(host.querySelector('.mp-rail-dot').classList.contains('off')).toBe(true);
    search('');
    expect(host.querySelector('.mp-title').textContent).toBe('ChatGPT');
    expect(host.querySelector('.mp-group')).toBeNull();
  });

  it('keeps one rail button per Gemini account (pickerOnly entries), each with its own models', () => {
    const acc = (id, email, models) => ({ id, kind: 'acp', name: 'Gemini · ' + email, pickerOnly: true, status: 'connected', enabled: true, driver: 'acp:' + id, models });
    const gem = [
      acc('antigravity', 'a@x.io', [{ id: 'g-pro', name: 'Gemini Pro' }]),
      acc('antigravity@a2', 'b@x.io', [{ id: 'g-flash', name: 'Gemini Flash' }, { id: 'g-lite', name: 'Gemini Lite' }]),
    ];
    const onselect = vi.fn();
    mount({ providers: gem, plan: null, onselect });
    open();
    const labels = [...host.querySelectorAll('.mp-rail-btn:not(.mp-add)')].map((b) => b.getAttribute('aria-label'));
    expect(labels).toEqual(['Gemini · a@x.io', 'Gemini · b@x.io']);
    expect(items().filter((t) => t.includes('Gemini'))).toEqual(['Gemini Pro']);
    rail('Gemini · b@x.io');
    expect(host.querySelector('.mp-count').textContent).toBe('2');
    expect(host.querySelector('.mp-rail-dot').style.transform).toBe('translateY(36px)');
    expect(items()).toEqual(expect.arrayContaining(['Gemini Flash', 'Gemini Lite']));
    [...host.querySelectorAll('.mp-item')].find((b) => b.textContent.includes('Gemini Lite')).click();
    expect(onselect).toHaveBeenCalledWith({ driver: 'acp:antigravity@a2', model: 'g-lite' });
  });

  it('collapses models beyond the first eight under «Ещё модели» and expands on click', () => {
    mount({ providers: [many], plan: null });
    open();
    rail('BigCloud');
    expect(items()).not.toContain('Model 9');
    const more = host.querySelector('.mp-more');
    expect(more.textContent).toContain('Ещё модели · 3');
    more.click();
    flushSync();
    expect(items()).toContain('Model 9');
  });

  it('auto-expands when the search matches inside it, and when the current value is there', () => {
    mount({ providers: [many], plan: null });
    open();
    rail('BigCloud');
    search('model 10');
    expect(items()).toContain('Model 10');
    handle.destroy();
    host.remove();
    mount({ providers: [many], plan: null, value: { driver: 'openai:big', model: 'm10' } });
    open();
    expect(items()).toContain('Model 10');
  });

  it('«Подключить ИИ» in the rail and the end row call onconnect', () => {
    const onconnect = vi.fn();
    mount({ onconnect, providers: [], plan: null });
    open();
    host.querySelector('.mp-add').click();
    flushSync();
    expect(onconnect).toHaveBeenCalledTimes(1);
    open();
    host.querySelector('.mp-connect').click();
    expect(onconnect).toHaveBeenCalledTimes(2);
  });
});
